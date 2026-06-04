// skills/autospec-test/tests/unit/lint-playwright-author.test.mjs
// node --test  (Node.js built-in test runner)
// Tests for scripts/lint-playwright-author.mjs
// Covers: lintSpec(), runWithRetry(), all 6 RULE_IDs (PW_MOCK_BANNED through PW_NO_PERSISTENCE_ASSERT)

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import fs from 'node:fs';
import os from 'node:os';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const SCRIPTS_DIR = path.resolve(__dirname, '../../scripts');

const { lintSpec, runWithRetry } = await import(`file://${SCRIPTS_DIR}/lint-playwright-author.mjs`);

// ── Helpers ────────────────────────────────────────────────────────────────────

function tmpSpec(content) {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'lint-pw-test-'));
    const specPath = path.join(dir, 'my-spec.spec.ts');
    fs.writeFileSync(specPath, content, 'utf8');
    return { dir, specPath };
}

function tmpSrc(dir, filename, content) {
    const srcPath = path.join(dir, filename);
    fs.writeFileSync(srcPath, content, 'utf8');
    return srcPath;
}

function cleanup(dir) {
    fs.rmSync(dir, { recursive: true, force: true });
}

const CLEAN_SPEC = `
import { test, expect } from '@playwright/test';

test('renders dashboard', async ({ page }) => {
    await page.goto('/dashboard');
    const heading = page.getByRole('heading', { name: 'Dashboard', exact: true });
    await expect(heading).toBeVisible();
});
`.trim();

// ── lintSpec: clean spec returns ok=true ──────────────────────────────────────

test('lintSpec: clean spec with no violations returns ok=true', async () => {
    const { dir, specPath } = tmpSpec(CLEAN_SPEC);
    try {
        const result = await lintSpec(specPath, { appSrcGlobs: [], assignedFile: specPath });
        assert.equal(result.ok, true);
        assert.equal(result.findings.length, 0);
    } finally {
        cleanup(dir);
    }
});

// ── PW_MOCK_BANNED ─────────────────────────────────────────────────────────────

test('PW_MOCK_BANNED: detects msw import', async () => {
    const { dir, specPath } = tmpSpec(`
import { setupServer } from 'msw/node';
import { test } from '@playwright/test';
test('foo', async ({ page }) => { await page.goto('/'); });
`);
    try {
        const result = await lintSpec(specPath, { appSrcGlobs: [], assignedFile: specPath });
        assert.equal(result.ok, false);
        const rules = result.findings.map(f => f.rule);
        assert.ok(rules.includes('PW_MOCK_BANNED'), `Expected PW_MOCK_BANNED, got ${JSON.stringify(rules)}`);
    } finally {
        cleanup(dir);
    }
});

test('PW_MOCK_BANNED: detects nock require', async () => {
    const { dir, specPath } = tmpSpec(`
const nock = require('nock');
import { test } from '@playwright/test';
test('foo', async ({ page }) => { await page.goto('/'); });
`);
    try {
        const result = await lintSpec(specPath, { appSrcGlobs: [], assignedFile: specPath });
        assert.equal(result.ok, false);
        const rules = result.findings.map(f => f.rule);
        assert.ok(rules.includes('PW_MOCK_BANNED'), `Expected PW_MOCK_BANNED`);
    } finally {
        cleanup(dir);
    }
});

test('PW_MOCK_BANNED: detects page.route(', async () => {
    const { dir, specPath } = tmpSpec(`
import { test } from '@playwright/test';
test('foo', async ({ page }) => {
    await page.route('/api/**', route => route.fulfill({ body: '{}' }));
    await page.goto('/');
});
`);
    try {
        const result = await lintSpec(specPath, { appSrcGlobs: [], assignedFile: specPath });
        assert.equal(result.ok, false);
        const rules = result.findings.map(f => f.rule);
        assert.ok(rules.includes('PW_MOCK_BANNED'), `Expected PW_MOCK_BANNED`);
    } finally {
        cleanup(dir);
    }
});

test('PW_MOCK_BANNED: detects route.fulfill(', async () => {
    const { dir, specPath } = tmpSpec(`
import { test } from '@playwright/test';
test('foo', async ({ page }) => {
    await someRoute.fulfill({ status: 200 });
    await page.goto('/');
});
`);
    try {
        const result = await lintSpec(specPath, { appSrcGlobs: [], assignedFile: specPath });
        assert.equal(result.ok, false);
        const rules = result.findings.map(f => f.rule);
        assert.ok(rules.includes('PW_MOCK_BANNED'), `Expected PW_MOCK_BANNED`);
    } finally {
        cleanup(dir);
    }
});

test('PW_MOCK_BANNED: detects route.abort(', async () => {
    const { dir, specPath } = tmpSpec(`
import { test } from '@playwright/test';
test('foo', async ({ page }) => {
    await someRoute.abort();
    await page.goto('/');
});
`);
    try {
        const result = await lintSpec(specPath, { appSrcGlobs: [], assignedFile: specPath });
        assert.equal(result.ok, false);
        const rules = result.findings.map(f => f.rule);
        assert.ok(rules.includes('PW_MOCK_BANNED'), `Expected PW_MOCK_BANNED`);
    } finally {
        cleanup(dir);
    }
});

test('PW_MOCK_BANNED: detects string-concat evasion of mock import', async () => {
    // AST check: import from dynamic string built from 'm' + 'sw' etc.
    const { dir, specPath } = tmpSpec(`
import { test } from '@playwright/test';
const lib = 'ms' + 'w';
const mockModule = require(lib);
test('foo', async ({ page }) => { await page.goto('/'); });
`);
    try {
        const result = await lintSpec(specPath, { appSrcGlobs: [], assignedFile: specPath });
        assert.equal(result.ok, false);
        const rules = result.findings.map(f => f.rule);
        assert.ok(rules.includes('PW_MOCK_BANNED'), `Expected PW_MOCK_BANNED for concat evasion`);
    } finally {
        cleanup(dir);
    }
});

test('PW_MOCK_BANNED: detects sinon usage', async () => {
    const { dir, specPath } = tmpSpec(`
import sinon from 'sinon';
import { test } from '@playwright/test';
test('foo', async ({ page }) => { sinon.stub(); await page.goto('/'); });
`);
    try {
        const result = await lintSpec(specPath, { appSrcGlobs: [], assignedFile: specPath });
        assert.equal(result.ok, false);
        const rules = result.findings.map(f => f.rule);
        assert.ok(rules.includes('PW_MOCK_BANNED'), `Expected PW_MOCK_BANNED`);
    } finally {
        cleanup(dir);
    }
});

// ── PW_STRICT_MODE_RISK ────────────────────────────────────────────────────────

test('PW_STRICT_MODE_RISK: unscoped page.getByText without exact:true', async () => {
    const { dir, specPath } = tmpSpec(`
import { test, expect } from '@playwright/test';
test('foo', async ({ page }) => {
    await page.goto('/');
    const el = page.getByText('Submit');
    await expect(el).toBeVisible();
});
`);
    try {
        const result = await lintSpec(specPath, { appSrcGlobs: [], assignedFile: specPath });
        assert.equal(result.ok, false);
        const rules = result.findings.map(f => f.rule);
        assert.ok(rules.includes('PW_STRICT_MODE_RISK'), `Expected PW_STRICT_MODE_RISK`);
    } finally {
        cleanup(dir);
    }
});

test('PW_STRICT_MODE_RISK: page.getByText with exact:true is ok', async () => {
    const { dir, specPath } = tmpSpec(`
import { test, expect } from '@playwright/test';
test('foo', async ({ page }) => {
    await page.goto('/');
    const el = page.getByText('Submit', { exact: true });
    await expect(el).toBeVisible();
});
`);
    try {
        const result = await lintSpec(specPath, { appSrcGlobs: [], assignedFile: specPath });
        const strictRisk = result.findings.filter(f => f.rule === 'PW_STRICT_MODE_RISK');
        assert.equal(strictRisk.length, 0, `Should not flag exact:true`);
    } finally {
        cleanup(dir);
    }
});

test('PW_STRICT_MODE_RISK: unscoped page.getByRole without exact:true', async () => {
    const { dir, specPath } = tmpSpec(`
import { test, expect } from '@playwright/test';
test('foo', async ({ page }) => {
    await page.goto('/');
    const btn = page.getByRole('button', { name: 'Submit' });
    await btn.click();
});
`);
    try {
        const result = await lintSpec(specPath, { appSrcGlobs: [], assignedFile: specPath });
        const strictRisk = result.findings.filter(f => f.rule === 'PW_STRICT_MODE_RISK');
        assert.ok(strictRisk.length > 0, `Expected PW_STRICT_MODE_RISK for getByRole without exact`);
    } finally {
        cleanup(dir);
    }
});

test('PW_STRICT_MODE_RISK: page.getByRole with exact:true is ok', async () => {
    const { dir, specPath } = tmpSpec(`
import { test, expect } from '@playwright/test';
test('foo', async ({ page }) => {
    await page.goto('/');
    const btn = page.getByRole('button', { name: 'Submit', exact: true });
    await btn.click();
});
`);
    try {
        const result = await lintSpec(specPath, { appSrcGlobs: [], assignedFile: specPath });
        const strictRisk = result.findings.filter(f => f.rule === 'PW_STRICT_MODE_RISK');
        assert.equal(strictRisk.length, 0, `Should not flag exact:true`);
    } finally {
        cleanup(dir);
    }
});

// ── PW_SEED_BYPASS ─────────────────────────────────────────────────────────────

test('PW_SEED_BYPASS: detects prisma. in spec', async () => {
    const { dir, specPath } = tmpSpec(`
import { test } from '@playwright/test';
import { prisma } from '../lib/db';
test('foo', async ({ page }) => {
    await prisma.user.create({ data: { name: 'test' } });
    await page.goto('/');
});
`);
    try {
        const result = await lintSpec(specPath, { appSrcGlobs: [], assignedFile: specPath });
        assert.equal(result.ok, false);
        const rules = result.findings.map(f => f.rule);
        assert.ok(rules.includes('PW_SEED_BYPASS'), `Expected PW_SEED_BYPASS`);
    } finally {
        cleanup(dir);
    }
});

test('PW_SEED_BYPASS: detects knex( in spec', async () => {
    const { dir, specPath } = tmpSpec(`
import { test } from '@playwright/test';
test('foo', async ({ page }) => {
    await knex('users').insert({ name: 'test' });
    await page.goto('/');
});
`);
    try {
        const result = await lintSpec(specPath, { appSrcGlobs: [], assignedFile: specPath });
        assert.equal(result.ok, false);
        const rules = result.findings.map(f => f.rule);
        assert.ok(rules.includes('PW_SEED_BYPASS'), `Expected PW_SEED_BYPASS`);
    } finally {
        cleanup(dir);
    }
});

test('PW_SEED_BYPASS: detects raw SQL INSERT in spec', async () => {
    const { dir, specPath } = tmpSpec(`
import { test } from '@playwright/test';
test('foo', async ({ page }) => {
    await db.query("INSERT INTO users (name) VALUES ('test')");
    await page.goto('/');
});
`);
    try {
        const result = await lintSpec(specPath, { appSrcGlobs: [], assignedFile: specPath });
        assert.equal(result.ok, false);
        const rules = result.findings.map(f => f.rule);
        assert.ok(rules.includes('PW_SEED_BYPASS'), `Expected PW_SEED_BYPASS`);
    } finally {
        cleanup(dir);
    }
});

// ── PW_SHARED_FILE_EDIT ────────────────────────────────────────────────────────

test('PW_SHARED_FILE_EDIT: specPath not matching assignedFile triggers violation', async () => {
    const { dir, specPath } = tmpSpec(CLEAN_SPEC);
    const otherSpec = path.join(path.dirname(specPath), 'other.spec.ts');
    fs.writeFileSync(otherSpec, CLEAN_SPEC);
    try {
        // specPath is the file being linted; assignedFile is the expected file
        const result = await lintSpec(specPath, {
            appSrcGlobs: [],
            assignedFile: otherSpec // different file
        });
        assert.equal(result.ok, false);
        const rules = result.findings.map(f => f.rule);
        assert.ok(rules.includes('PW_SHARED_FILE_EDIT'), `Expected PW_SHARED_FILE_EDIT`);
    } finally {
        cleanup(dir);
    }
});

test('PW_SHARED_FILE_EDIT: specPath matching assignedFile is ok', async () => {
    const { dir, specPath } = tmpSpec(CLEAN_SPEC);
    try {
        const result = await lintSpec(specPath, {
            appSrcGlobs: [],
            assignedFile: specPath
        });
        const sharedEdit = result.findings.filter(f => f.rule === 'PW_SHARED_FILE_EDIT');
        assert.equal(sharedEdit.length, 0, `Should not flag matching assignedFile`);
    } finally {
        cleanup(dir);
    }
});

// ── PW_NO_PERSISTENCE_ASSERT ──────────────────────────────────────────────────

test('PW_NO_PERSISTENCE_ASSERT: click+submit without API assert emits warn finding', async () => {
    const { dir, specPath } = tmpSpec(`
import { test, expect } from '@playwright/test';
test('creates item', async ({ page }) => {
    await page.goto('/items/new');
    await page.getByRole('textbox', { name: 'Title', exact: true }).fill('My Item');
    await page.getByRole('button', { name: 'Submit', exact: true }).click();
    await expect(page.getByRole('heading', { name: 'Success', exact: true })).toBeVisible();
});
`);
    try {
        const result = await lintSpec(specPath, { appSrcGlobs: [], assignedFile: specPath });
        const warnFindings = result.findings.filter(f => f.rule === 'PW_NO_PERSISTENCE_ASSERT');
        assert.ok(warnFindings.length > 0, `Expected PW_NO_PERSISTENCE_ASSERT warn`);
        // warn does not set ok=false
        const hardFails = result.findings.filter(f => f.rule !== 'PW_NO_PERSISTENCE_ASSERT');
        if (hardFails.length === 0) {
            assert.equal(result.ok, true, `warn-only should not fail`);
        }
    } finally {
        cleanup(dir);
    }
});

// ── Finding shape ──────────────────────────────────────────────────────────────

test('lintSpec: findings have rule, msg, line shape', async () => {
    const { dir, specPath } = tmpSpec(`
import { setupServer } from 'msw/node';
import { test } from '@playwright/test';
test('foo', async ({ page }) => { await page.goto('/'); });
`);
    try {
        const result = await lintSpec(specPath, { appSrcGlobs: [], assignedFile: specPath });
        assert.ok(result.findings.length > 0);
        for (const f of result.findings) {
            assert.ok('rule' in f, `finding missing rule: ${JSON.stringify(f)}`);
            assert.ok('msg' in f, `finding missing msg: ${JSON.stringify(f)}`);
            assert.ok('line' in f, `finding missing line: ${JSON.stringify(f)}`);
            assert.equal(typeof f.rule, 'string');
            assert.equal(typeof f.msg, 'string');
            assert.equal(typeof f.line, 'number');
        }
    } finally {
        cleanup(dir);
    }
});

// ── runWithRetry ───────────────────────────────────────────────────────────────

test('runWithRetry: succeeds on first try returns result', async () => {
    const specPath = '/tmp/fake.spec.ts';
    let callCount = 0;
    const authorFn = async (_specPath, _directives) => {
        callCount++;
        return 'success';
    };
    const result = await runWithRetry(authorFn, specPath);
    assert.equal(result, 'success');
    assert.equal(callCount, 1);
});

test('runWithRetry: retries up to MAX=5 then rejects', async () => {
    const specPath = '/tmp/fake.spec.ts';
    let callCount = 0;
    const authorFn = async (_specPath, _directives) => {
        callCount++;
        throw new Error(`attempt ${callCount} failed`);
    };
    await assert.rejects(
        async () => await runWithRetry(authorFn, specPath, 5),
        (err) => {
            assert.ok(err.message.includes('5') || err.message.includes('failed') || err.message.includes('MAX'),
                `Unexpected error: ${err.message}`);
            return true;
        }
    );
    assert.equal(callCount, 5);
});

test('runWithRetry: succeeds on second try after first failure', async () => {
    let callCount = 0;
    const authorFn = async (_specPath, directives) => {
        callCount++;
        if (callCount === 1) throw new Error('first attempt failed');
        return `ok on attempt ${callCount}`;
    };
    const result = await runWithRetry(authorFn, '/tmp/fake.spec.ts', 5);
    assert.equal(result, 'ok on attempt 2');
    assert.equal(callCount, 2);
});

test('runWithRetry: passes directives from previous failure to next attempt', async () => {
    const receivedDirectives = [];
    let callCount = 0;
    const authorFn = async (_specPath, directives) => {
        callCount++;
        receivedDirectives.push(directives);
        if (callCount < 3) throw new Error(`PW_MOCK_BANNED: found page.route on line 5`);
        return 'success';
    };
    await runWithRetry(authorFn, '/tmp/fake.spec.ts', 5);
    assert.equal(callCount, 3);
    // Second and third calls should have directives from the previous failure
    assert.ok(receivedDirectives[1] && receivedDirectives[1].length > 0,
        `Expected directives on retry 2, got: ${JSON.stringify(receivedDirectives[1])}`);
});

test('runWithRetry: default MAX is 5', async () => {
    let callCount = 0;
    const authorFn = async () => { callCount++; throw new Error('always fail'); };
    await assert.rejects(async () => await runWithRetry(authorFn, '/tmp/fake.spec.ts'));
    assert.equal(callCount, 5);
});

// ── PW_SELECTOR_UNVERIFIED: comment-greenwash regression (Phase 5.5 audit #1001) ─
//
// End-to-end seam test: with resolveSelector:true the lint delegates to
// selector-evidence.mjs's resolver. A selector that exists ONLY in an app-source
// comment must NOT count as verified — the lint must hard-fail PW_SELECTOR_UNVERIFIED.

test('lintSpec: selector present only in an app-source comment hard-fails PW_SELECTOR_UNVERIFIED', async () => {
    const { dir, specPath } = tmpSpec(`
import { test, expect } from '@playwright/test';
test('ghost', async ({ page }) => {
    await page.getByTestId('ghost-btn').click();
    await expect(page.getByRole('status')).toContainText('ok');
});
`.trim());
    try {
        // App source mentions the selector ONLY inside a comment (greenwash attempt).
        tmpSrc(dir, 'App.tsx', `
export function App() {
    // data-testid="ghost-btn" is described here but never rendered
    return <button data-testid="real-btn">Go</button>;
}
`);
        const result = await lintSpec(specPath, {
            appSrcGlobs: [path.join(dir, '*.tsx')],
            assignedFile: specPath,
            resolveSelector: true,
            repoRoot: dir,
        });
        assert.equal(result.ok, false, 'comment-only selector must NOT pass lint');
        assert.ok(result.findings.some(f => f.rule === 'PW_SELECTOR_UNVERIFIED'),
            `expected PW_SELECTOR_UNVERIFIED, got ${JSON.stringify(result.findings)}`);
    } finally {
        cleanup(dir);
    }
});

test('lintSpec: selector in real (non-comment) markup passes PW_SELECTOR_UNVERIFIED', async () => {
    const { dir, specPath } = tmpSpec(`
import { test, expect } from '@playwright/test';
test('real', async ({ page }) => {
    await page.getByTestId('real-btn').click();
});
`.trim());
    try {
        tmpSrc(dir, 'App.tsx', `
export function App() {
    return <button data-testid="real-btn">Go</button>;
}
`);
        const result = await lintSpec(specPath, {
            appSrcGlobs: [path.join(dir, '*.tsx')],
            assignedFile: specPath,
            resolveSelector: true,
            repoRoot: dir,
        });
        assert.ok(!result.findings.some(f => f.rule === 'PW_SELECTOR_UNVERIFIED'),
            `real-btn should be verified, got ${JSON.stringify(result.findings)}`);
    } finally {
        cleanup(dir);
    }
});
