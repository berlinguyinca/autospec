// skills/autospec-test/tests/unit/stage2a-orchestrator.test.mjs
// node --test  (Node.js built-in test runner)
// Tests for scripts/stage2a-orchestrator.mjs (#996)
// Covers: clusterRoutes, centralizeHelpers, parseGate, coverageReport
// Plus the #1003 carry-over: selector-evidence resolver wired into the lint path
// so PW_SELECTOR_UNVERIFIED actually fires (invented data-testid FAILS, source-backed PASSES).

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import fs from 'node:fs';
import os from 'node:os';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const SCRIPTS_DIR = path.resolve(__dirname, '../../scripts');

const { clusterRoutes, centralizeHelpers, parseGate, coverageReport } =
    await import(`file://${SCRIPTS_DIR}/stage2a-orchestrator.mjs`);
const { lintSpec } = await import(`file://${SCRIPTS_DIR}/lint-playwright-author.mjs`);

// ── Helpers ────────────────────────────────────────────────────────────────────

function tmpDir() {
    return fs.mkdtempSync(path.join(os.tmpdir(), 'stage2a-test-'));
}

function writeFile(dir, relPath, content) {
    const full = path.join(dir, relPath);
    fs.mkdirSync(path.dirname(full), { recursive: true });
    fs.writeFileSync(full, content, 'utf8');
    return full;
}

function cleanup(dir) {
    fs.rmSync(dir, { recursive: true, force: true });
}

// ── clusterRoutes ────────────────────────────────────────────────────────────────

test('clusterRoutes: auto-clusters by first path segment', () => {
    const routes = ['/users/1', '/users/2', '/orders/9', '/orders/10', '/'];
    const clusters = clusterRoutes(routes, { fanout_max: 4, route_clusters: 'auto' });
    // grouped by first segment: users, orders, root
    const names = clusters.map(c => c.name).sort();
    assert.deepEqual(names, ['orders', 'root', 'users']);
    const users = clusters.find(c => c.name === 'users');
    assert.deepEqual(users.routes.sort(), ['/users/1', '/users/2']);
    const root = clusters.find(c => c.name === 'root');
    assert.deepEqual(root.routes, ['/']);
});

test('clusterRoutes: caps cluster count at fanout_max by merging overflow', () => {
    const routes = ['/a/1', '/b/1', '/c/1', '/d/1', '/e/1', '/f/1'];
    const clusters = clusterRoutes(routes, { fanout_max: 3, route_clusters: 'auto' });
    assert.ok(clusters.length <= 3, `expected <=3 clusters, got ${clusters.length}`);
    // every route is still assigned exactly once
    const all = clusters.flatMap(c => c.routes).sort();
    assert.deepEqual(all, routes.slice().sort());
});

test('clusterRoutes: honors explicit override (array of {name, routes})', () => {
    const routes = ['/users/1', '/orders/1'];
    const explicit = [
        { name: 'people', routes: ['/users/1'] },
        { name: 'commerce', routes: ['/orders/1'] },
    ];
    const clusters = clusterRoutes(routes, { fanout_max: 4, route_clusters: explicit });
    assert.deepEqual(clusters.map(c => c.name).sort(), ['commerce', 'people']);
});

test('clusterRoutes: fanout_max default is 4 when omitted', () => {
    const routes = ['/a/1', '/b/1', '/c/1', '/d/1', '/e/1'];
    const clusters = clusterRoutes(routes, {});
    assert.ok(clusters.length <= 4, `expected default cap 4, got ${clusters.length}`);
});

// ── centralizeHelpers ────────────────────────────────────────────────────────────

test('centralizeHelpers: creates helpers_dir files, idempotent', () => {
    const dir = tmpDir();
    try {
        const helpersDir = path.join(dir, 'e2e', 'helpers');
        const files = { 'api.ts': 'export const api = 1;\n', 'reset.ts': 'export const reset = 2;\n' };
        const r1 = centralizeHelpers(helpersDir, { files });
        assert.equal(r1.created.length, 2);
        assert.ok(fs.existsSync(path.join(helpersDir, 'api.ts')));
        assert.equal(fs.readFileSync(path.join(helpersDir, 'api.ts'), 'utf8'), 'export const api = 1;\n');

        // Idempotent: second run with same content creates nothing new
        const r2 = centralizeHelpers(helpersDir, { files });
        assert.equal(r2.created.length, 0);
        assert.equal(r2.skipped.length, 2);
    } finally {
        cleanup(dir);
    }
});

test('centralizeHelpers: single-writer — does not overwrite divergent existing file', () => {
    const dir = tmpDir();
    try {
        const helpersDir = path.join(dir, 'e2e', 'helpers');
        centralizeHelpers(helpersDir, { files: { 'api.ts': 'v1\n' } });
        // author/operator changed the file; orchestrator must not clobber it silently
        const r = centralizeHelpers(helpersDir, { files: { 'api.ts': 'v2\n' } });
        assert.equal(fs.readFileSync(path.join(helpersDir, 'api.ts'), 'utf8'), 'v1\n');
        assert.equal(r.created.length, 0);
        assert.ok(r.conflicts.includes('api.ts'));
    } finally {
        cleanup(dir);
    }
});

// ── parseGate ────────────────────────────────────────────────────────────────────

test('parseGate: returns playwright --list invocation shape', () => {
    const gate = parseGate('e2e/specs', { playwrightBin: 'npx playwright' });
    assert.equal(gate.cmd, 'npx');
    assert.deepEqual(gate.args.slice(0, 3), ['playwright', 'test', '--list']);
    assert.ok(gate.args.includes('e2e/specs'));
});

test('parseGate: default bin is npx playwright', () => {
    const gate = parseGate('e2e/specs', {});
    assert.equal(gate.cmd, 'npx');
    assert.ok(gate.args.includes('--list'));
});

// ── coverageReport ───────────────────────────────────────────────────────────────

test('coverageReport: computes {total,covered,pct} from crawler manifest denominator', () => {
    const dir = tmpDir();
    try {
        const crawlerManifest = {
            routes: ['/a', '/b', '/c', '/d'],
            manifest: [{ route: '/a', selector: 'x' }],
        };
        const report = coverageReport(crawlerManifest, ['/a', '/b'], dir);
        assert.equal(report.total, 4);
        assert.equal(report.covered, 2);
        assert.equal(report.pct, 50);
        // written to e2e/.autospec/coverage.json
        const written = JSON.parse(fs.readFileSync(path.join(dir, 'e2e', '.autospec', 'coverage.json'), 'utf8'));
        assert.deepEqual(written, { total: 4, covered: 2, pct: 50 });
    } finally {
        cleanup(dir);
    }
});

test('coverageReport: pct uses crawler denominator, not author self-report; covered capped to known routes', () => {
    const dir = tmpDir();
    try {
        const crawlerManifest = { routes: ['/a', '/b'], manifest: [] };
        // author claims 5 covered, but only 2 routes exist; covered must not exceed total
        const report = coverageReport(crawlerManifest, ['/a', '/b', '/ghost', '/phantom', '/x'], dir);
        assert.equal(report.total, 2);
        assert.equal(report.covered, 2);
        assert.equal(report.pct, 100);
    } finally {
        cleanup(dir);
    }
});

test('coverageReport: total=0 yields pct 0 (no divide-by-zero)', () => {
    const dir = tmpDir();
    try {
        const report = coverageReport({ routes: [], manifest: [] }, [], dir);
        assert.equal(report.total, 0);
        assert.equal(report.covered, 0);
        assert.equal(report.pct, 0);
    } finally {
        cleanup(dir);
    }
});

// ── #1003 carry-over: selector-evidence resolver wired into lint ────────────────
// lint-playwright-author.mjs's appSrcGlobs path was a naive substring check.
// #996 wires selector-evidence.mjs's resolveSelector so PW_SELECTOR_UNVERIFIED
// fires on an invented data-testid and passes on a source-backed one.

test('carry-over: invented data-testid FAILS lint via resolver', async () => {
    const dir = tmpDir();
    try {
        writeFile(dir, 'src/Form.tsx',
            'export const Form = () => <button data-testid="real-submit">Go</button>;\n');
        const specPath = writeFile(dir, 'e2e/specs/form.spec.ts', `
import { test, expect } from '@playwright/test';
test('submit', async ({ page }) => {
  await page.getByTestId('totally-invented-id').click({ exact: true });
});
`);
        const res = await lintSpec(specPath, {
            appSrcGlobs: [path.join(dir, 'src')],
            assignedFile: specPath,
            resolveSelector: true,
            repoRoot: dir,
        });
        assert.ok(!res.ok, 'expected lint to FAIL on invented selector');
        assert.ok(res.findings.some(f => f.rule === 'PW_SELECTOR_UNVERIFIED'),
            `expected PW_SELECTOR_UNVERIFIED, got ${JSON.stringify(res.findings)}`);
    } finally {
        cleanup(dir);
    }
});

test('carry-over: source-backed data-testid PASSES lint via resolver', async () => {
    const dir = tmpDir();
    try {
        writeFile(dir, 'src/Form.tsx',
            'export const Form = () => <button data-testid="real-submit">Go</button>;\n');
        const specPath = writeFile(dir, 'e2e/specs/form.spec.ts', `
import { test, expect } from '@playwright/test';
test('submit', async ({ page, request }) => {
  await page.getByTestId('real-submit').click();
  const r = await request.get('/api/state');
});
`);
        const res = await lintSpec(specPath, {
            appSrcGlobs: [path.join(dir, 'src')],
            assignedFile: specPath,
            resolveSelector: true,
            repoRoot: dir,
        });
        assert.ok(res.ok, `expected lint to PASS, got ${JSON.stringify(res.findings)}`);
        assert.ok(!res.findings.some(f => f.rule === 'PW_SELECTOR_UNVERIFIED'),
            'no PW_SELECTOR_UNVERIFIED expected for source-backed selector');
    } finally {
        cleanup(dir);
    }
});
