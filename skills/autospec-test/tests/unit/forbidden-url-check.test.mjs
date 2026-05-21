// skills/autospec-test/tests/unit/forbidden-url-check.test.mjs
// node --test  (Node.js built-in test runner)
// Tests for scripts/forbidden-url-check.mjs

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const SCRIPTS_DIR = path.resolve(__dirname, '../../scripts');

// Dynamically import the checker module
const { check } = await import(`${SCRIPTS_DIR}/forbidden-url-check.mjs`);

// ── URL field coverage per spec §5a Layer A ────────────────────────────────

test('check: no violation for safe localhost URL', async () => {
    const config = { baseURL: 'http://localhost:3000', use: { baseURL: 'http://localhost:3000' } };
    const patterns = ['^https?://prod\\.example\\.com'];
    const result = check(config, patterns);
    assert.equal(result.violations.length, 0);
});

test('check: detects violation in top-level baseURL', async () => {
    const config = { baseURL: 'https://prod.example.com' };
    const patterns = ['^https?://prod\\.example\\.com'];
    const result = check(config, patterns);
    assert.ok(result.violations.length > 0);
    assert.equal(result.violations[0].field, 'baseURL');
});

test('check: detects violation in use.baseURL', async () => {
    const config = { use: { baseURL: 'https://prod.example.com' } };
    const patterns = ['^https?://prod\\.example\\.com'];
    const result = check(config, patterns);
    assert.ok(result.violations.length > 0);
    const fields = result.violations.map(v => v.field);
    assert.ok(fields.includes('use.baseURL'), `Expected use.baseURL in ${JSON.stringify(fields)}`);
});

test('check: detects violation in webServer.url', async () => {
    const config = { webServer: { url: 'https://prod.example.com/health' } };
    const patterns = ['^https?://prod\\.example\\.com'];
    const result = check(config, patterns);
    assert.ok(result.violations.length > 0);
    const fields = result.violations.map(v => v.field);
    assert.ok(fields.includes('webServer.url'), `Expected webServer.url in ${JSON.stringify(fields)}`);
});

test('check: detects violation in projects[].use.baseURL', async () => {
    const config = {
        projects: [
            { name: 'chromium', use: { baseURL: 'https://prod.example.com' } }
        ]
    };
    const patterns = ['^https?://prod\\.example\\.com'];
    const result = check(config, patterns);
    assert.ok(result.violations.length > 0);
});

test('check: handles nested objects in use block without false positives', async () => {
    // Regression from PR #331 finding #4: nested viewport/permissions/headers
    const config = {
        use: {
            viewport: { width: 1280, height: 720 },
            permissions: ['clipboard-read'],
            baseURL: 'http://localhost:3000',
            extraHTTPHeaders: { 'X-Custom': 'value' }
        }
    };
    const patterns = ['^https?://prod\\.example\\.com'];
    const result = check(config, patterns);
    assert.equal(result.violations.length, 0);
});

test('check: multiple patterns — first match wins', async () => {
    const config = { baseURL: 'https://app.acme.com/api' };
    const patterns = ['^https?://app\\.acme\\.com', '^https?://.*\\.prod\\.acme\\.internal'];
    const result = check(config, patterns);
    assert.ok(result.violations.length > 0);
    assert.equal(result.violations[0].pattern, '^https?://app\\.acme\\.com');
});

test('check: empty patterns array returns no violations', async () => {
    const config = { baseURL: 'https://prod.example.com' };
    const result = check(config, []);
    assert.equal(result.violations.length, 0);
});

test('check: violations include field, value, and pattern', async () => {
    const config = { baseURL: 'https://prod.example.com' };
    const patterns = ['^https?://prod\\.example\\.com'];
    const result = check(config, patterns);
    assert.ok(result.violations.length > 0);
    const v = result.violations[0];
    assert.ok('field' in v, 'violation missing field');
    assert.ok('value' in v, 'violation missing value');
    assert.ok('pattern' in v, 'violation missing pattern');
});

test('check: no config returns empty violations', async () => {
    const result = check({}, ['^https?://prod\\.example\\.com']);
    assert.equal(result.violations.length, 0);
});

test('check: webServer as array (multiple servers) checks all', async () => {
    const config = {
        webServer: [
            { url: 'http://localhost:3000/health' },
            { url: 'https://prod.example.com/health' }
        ]
    };
    const patterns = ['^https?://prod\\.example\\.com'];
    const result = check(config, patterns);
    assert.ok(result.violations.length > 0);
});
