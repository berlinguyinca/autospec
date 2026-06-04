// skills/autospec-test/tests/stage2a-integration.test.mjs
// node --test  (Node.js built-in test runner)
//
// End-to-end Stage 2A integration test (issue #1000, spec §10).
//
// Boots the authoring-fixture as a REAL node http server, exercises it over REAL
// HTTP, then drives the merged Stage 2A producer modules (imported unchanged — no
// mocks, no re-implementation) and asserts each spec-§10 outcome:
//
//   1. clusterRoutes clusters the 3 fixture routes.
//   2. centralizeHelpers creates helpers idempotently (second run skips).
//   3. authored-spec lint catches an invented testid (PW_SELECTOR_UNVERIFIED via
//      the selector-evidence resolver) AND the strict-mode trap (PW_STRICT_MODE_RISK).
//   4. reset-endpoint-gen generates a guard-env-gated reset route for the no-reset
//      fixture (AUTOSPEC_TEST_STACK guard; AUTOSPEC-GENERATED test-only header).
//   5. loop-classifier classifies the delete bug as product_bug (assertion NOT weakened).
//   6. coverageReport emits correct {total, covered, pct} to e2e/.autospec/coverage.json.

import { test, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import fs from 'node:fs';
import os from 'node:os';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const SCRIPTS_DIR = path.resolve(__dirname, '../scripts');
const FIXTURE_DIR = path.resolve(__dirname, 'fixtures/authoring-fixture');

// ── Real producer modules (imported unchanged — no forks, no mocks) ─────────────
const { clusterRoutes, centralizeHelpers, coverageReport } =
    await import(`file://${SCRIPTS_DIR}/stage2a-orchestrator.mjs`);
const { lintSpec } =
    await import(`file://${SCRIPTS_DIR}/lint-playwright-author.mjs`);
const { buildEvidence, writeManifest } =
    await import(`file://${SCRIPTS_DIR}/selector-evidence.mjs`);
const { generateResetRoute } =
    await import(`file://${SCRIPTS_DIR}/reset-endpoint-gen.mjs`);
const { classify } =
    await import(`file://${SCRIPTS_DIR}/loop-classifier.mjs`);
const { startServer, ROUTES } =
    await import(`file://${FIXTURE_DIR}/server.mjs`);

// ── Live fixture (real process, real HTTP) ──────────────────────────────────────
let fixture; // { server, baseUrl, port }
let workRoot; // temp repo root for generated artifacts

before(async () => {
    fixture = await startServer();
    workRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'stage2a-int-'));
});

after(async () => {
    if (fixture?.server) await new Promise((r) => fixture.server.close(r));
    if (workRoot) fs.rmSync(workRoot, { recursive: true, force: true });
});

async function http(method, p) {
    const res = await fetch(`${fixture.baseUrl}${p}`, { method });
    const body = await res.text();
    return { status: res.status, body };
}

// ── 0. The fixture really serves the 3 routes over HTTP ─────────────────────────
test('fixture serves 3 routes with embedded traps over real HTTP', async () => {
    const products = await http('GET', '/products');
    const orders = await http('GET', '/orders');
    const account = await http('GET', '/account');

    assert.equal(products.status, 200);
    assert.equal(orders.status, 200);
    assert.equal(account.status, 200);

    // trap1: products heading has NO data-testid
    assert.match(products.body, /<h1>Products<\/h1>/);
    assert.doesNotMatch(products.body, /data-testid=["']products-heading["']/);

    // trap2: "Orders" text appears in BOTH the nav rail and the page heading
    const ordersHits = (orders.body.match(/>Orders</g) || []).length;
    assert.ok(ordersHits >= 2, `expected "Orders" text in nav + heading, got ${ordersHits}`);

    // no reset endpoint exists on the fixture
    const reset = await http('POST', '/api/test/reset');
    assert.equal(reset.status, 404);
});

// ── 1. clusterRoutes clusters the 3 routes ──────────────────────────────────────
test('clusterRoutes clusters the 3 fixture routes (one cluster per segment)', () => {
    const clusters = clusterRoutes(ROUTES, { fanout_max: 4 });
    const names = clusters.map((c) => c.name).sort();
    assert.deepEqual(names, ['account', 'orders', 'products']);
    const assigned = clusters.flatMap((c) => c.routes).sort();
    assert.deepEqual(assigned, [...ROUTES].sort());
});

// ── 2. centralizeHelpers is idempotent ──────────────────────────────────────────
test('centralizeHelpers creates helpers once, skips on re-run (idempotent)', () => {
    const helpersDir = path.join(workRoot, 'e2e', 'helpers');
    const files = {
        'api.mjs':
            "export async function getProducts(baseUrl) {\n" +
            "  const r = await fetch(`${baseUrl}/api/products`);\n" +
            "  return (await r.json()).rows;\n}\n",
    };

    const first = centralizeHelpers(helpersDir, { files });
    assert.deepEqual(first.created, ['api.mjs']);
    assert.deepEqual(first.conflicts, []);

    const second = centralizeHelpers(helpersDir, { files });
    assert.deepEqual(second.created, []);
    assert.deepEqual(second.skipped, ['api.mjs']);
    assert.deepEqual(second.conflicts, []);
});

// ── 3. Lint catches the invented testid + the strict-mode trap ──────────────────
test('lint catches invented testid (PW_SELECTOR_UNVERIFIED) and strict-mode trap (PW_STRICT_MODE_RISK)', async () => {
    const specDir = path.join(workRoot, 'e2e', 'specs');
    fs.mkdirSync(specDir, { recursive: true });
    const specPath = path.join(specDir, 'products.spec.mjs');

    // An author who invents data-testid="products-heading" (NOT in app source —
    // trap1) and uses an unscoped getByText('Orders') (trap2).
    const spec =
        "import { test, expect } from '@playwright/test';\n" +
        "test('products', async ({ page }) => {\n" +
        "  await page.goto('/products');\n" +
        "  await expect(page.getByTestId('products-heading')).toBeVisible();\n" +
        "  await page.getByText('Orders').click();\n" +
        "});\n";
    fs.writeFileSync(specPath, spec, 'utf8');

    const appSrcGlobs = [path.join(FIXTURE_DIR, 'src')];
    const result = await lintSpec(specPath, {
        appSrcGlobs,
        assignedFile: specPath,
        resolveSelector: true,
        repoRoot: FIXTURE_DIR,
    });

    assert.equal(result.ok, false, 'spec with traps must hard-fail lint');
    const rules = result.findings.map((f) => f.rule);
    assert.ok(
        rules.includes('PW_SELECTOR_UNVERIFIED'),
        `expected PW_SELECTOR_UNVERIFIED for invented testid, got ${rules.join(',')}`
    );
    assert.ok(
        rules.includes('PW_STRICT_MODE_RISK'),
        `expected PW_STRICT_MODE_RISK for unscoped getByText, got ${rules.join(',')}`
    );

    // A clean spec (verified testid + scoped/exact text) must pass lint.
    const cleanPath = path.join(specDir, 'account.spec.mjs');
    const cleanSpec =
        "import { test, expect } from '@playwright/test';\n" +
        "test('account', async ({ page }) => {\n" +
        "  await page.goto('/account');\n" +
        "  await expect(page.getByTestId('account-heading')).toBeVisible();\n" +
        "  const apiHelper = {};\n" +
        "  await page.getByTestId('save-account').click();\n" +
        "});\n";
    fs.writeFileSync(cleanPath, cleanSpec, 'utf8');
    const cleanResult = await lintSpec(cleanPath, {
        appSrcGlobs,
        assignedFile: cleanPath,
        resolveSelector: true,
        repoRoot: FIXTURE_DIR,
    });
    assert.equal(cleanResult.ok, true,
        `clean spec must pass; findings=${JSON.stringify(cleanResult.findings)}`);

    // The selector-evidence manifest records the verified vs unverified split.
    const evidence = buildEvidence(cleanPath, appSrcGlobs, FIXTURE_DIR);
    assert.ok(Object.values(evidence).every((v) => v !== null),
        `clean spec selectors must all resolve; evidence=${JSON.stringify(evidence)}`);
    const manifestPath = writeManifest({ [cleanPath]: evidence }, workRoot);
    assert.ok(fs.existsSync(manifestPath));
});

// ── 4. reset-endpoint-gen generates a guard-env-gated reset route ───────────────
test('reset-endpoint-gen generates a guard-env-gated reset route for the no-reset fixture', async () => {
    // Copy the fixture into the temp repo root so generation writes there.
    const repoRoot = path.join(workRoot, 'app');
    fs.mkdirSync(repoRoot, { recursive: true });
    fs.copyFileSync(
        path.join(FIXTURE_DIR, 'package.json'),
        path.join(repoRoot, 'package.json')
    );

    const result = await generateResetRoute(repoRoot, {
        guardEnv: 'AUTOSPEC_TEST_STACK',
        baseUrl: fixture.baseUrl, // localhost — passes preflightHostname
    });

    assert.equal(result.generated, true);
    assert.equal(result.guardEnv, 'AUTOSPEC_TEST_STACK');
    assert.ok(fs.existsSync(result.filePath), 'reset route file must be written');

    const content = fs.readFileSync(result.filePath, 'utf8');
    // AUTOSPEC-GENERATED test-only header (shared contract).
    assert.match(content, /AUTOSPEC-GENERATED test-only/);
    // Guard-env gating: the route is inert unless AUTOSPEC_TEST_STACK is set.
    assert.match(content, /AUTOSPEC_TEST_STACK/);
});

// ── 5. loop-classifier classifies the delete bug as product_bug ─────────────────
test('loop-classifier classifies the delete-persists bug as product_bug (assertion unweakened)', async () => {
    // Drive the real product bug over HTTP: DELETE returns 200 but the row persists.
    const before = JSON.parse((await http('GET', '/api/products')).body).rows;
    const del = await http('DELETE', '/api/products/2');
    assert.equal(del.status, 200, 'delete acknowledges success (200)');
    const afterRows = JSON.parse((await http('GET', '/api/products')).body).rows;

    // Real persistence violation: the row is still present despite the 200.
    assert.equal(afterRows.length, before.length, 'row persists despite 200 (the bug)');
    assert.ok(afterRows.some((r) => r.id === 2), 'deleted row id=2 still present');

    // A faithful test asserts the row is gone; that assertion fails with an
    // "expected ... but received" shape — which the classifier must read as a
    // product_bug (fix the app), NOT weaken the assertion.
    const gateJson = {
        stage: 'e2e',
        passed: false,
        reason: 'tests_red',
        test_run_summary: {
            stderr_tail:
                'Error: expected product row 2 to be removed but received ' +
                'a list still containing { id: 2, name: "Gadget" } after DELETE returned 200',
            stdout_tail: '',
        },
    };
    const verdict = classify({ gate_json: gateJson });
    assert.equal(verdict.classification, 'product_bug',
        `delete bug must classify product_bug, got ${verdict.classification}`);
    // product_bug points at product code (src/lib/pkg), never at test dirs —
    // proving the assertion is fixed in the app, not weakened.
    assert.ok(
        verdict.suggested_files.some((f) => /^(src|lib|pkg)\b/.test(f) || f.startsWith('src/')),
        `product_bug must target product code, got ${verdict.suggested_files.join(',')}`
    );
    assert.ok(
        !verdict.suggested_files.some((f) => /test/i.test(f)),
        'product_bug must NOT route to test dirs (assertion stays unweakened)'
    );
});

// ── 6. coverageReport emits correct {total, covered, pct} ───────────────────────
test('coverageReport emits correct {total, covered, pct} to e2e/.autospec/coverage.json', () => {
    // The crawler manifest is the denominator authority (3 fixture routes).
    const crawlerManifest = { routes: [...ROUTES] };
    // Authored specs covered 2 of the 3 routes (products + account).
    const covered = ['/products', '/account'];

    const report = coverageReport(crawlerManifest, covered, workRoot);
    assert.equal(report.total, 3);
    assert.equal(report.covered, 2);
    assert.equal(report.pct, 67); // round(2/3 * 100)

    const outPath = path.join(workRoot, 'e2e', '.autospec', 'coverage.json');
    const onDisk = JSON.parse(fs.readFileSync(outPath, 'utf8'));
    assert.deepEqual(onDisk, { total: 3, covered: 2, pct: 67 });

    // A lying author cannot inflate past the crawler denominator.
    const inflated = coverageReport(crawlerManifest, ['/products', '/orders', '/account', '/ghost'], workRoot);
    assert.equal(inflated.covered, 3);
    assert.equal(inflated.pct, 100);
});
