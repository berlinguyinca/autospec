// tests/unit/mode-ii/runtime-intercept.test.mjs
// node --test
// Tests mode-ii-runtime-intercept.mjs: scope-token check on mutating requests.

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import fs from 'node:fs';
import os from 'node:os';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const SCRIPTS_DIR = path.resolve(__dirname, '../../../scripts');

// Import the interceptor module for unit testing
const INTERCEPT_MODULE = `file://${path.join(SCRIPTS_DIR, 'mode-ii-runtime-intercept.mjs')}`;

function makeTmpDir() {
    return fs.mkdtempSync(path.join(os.tmpdir(), 'autospec-intercept-'));
}

let interceptModule;

// Lazy-load so that import errors surface per-test
async function getModule() {
    if (!interceptModule) {
        interceptModule = await import(INTERCEPT_MODULE);
    }
    return interceptModule;
}

// ── Scope-pass: request carrying valid scope token ────────────────────────────

test('runtime-intercept: allows mutating request with matching scope token', async () => {
    const mod = await getModule();
    const scopeTokens = [
        {
            kind: 'route_filter',
            methods: ['POST', 'PUT', 'PATCH', 'DELETE'],
            allowed_path_patterns: ['^/api/families/test-family-7a3f9c(/.*)?$'],
            action_on_violation: 'hard_fail',
        },
    ];

    const result = mod.checkRequest({
        method: 'POST',
        url: '/api/families/test-family-7a3f9c/update',
        scopeTokens,
    });

    assert.strictEqual(result.allowed, true, `expected allowed, got: ${JSON.stringify(result)}`);
    assert.strictEqual(result.violation, false);
});

// ── Scope-violation: mutating request lacking scope token ─────────────────────

test('runtime-intercept: blocks mutating request without matching scope token', async () => {
    const mod = await getModule();
    const scopeTokens = [
        {
            kind: 'route_filter',
            methods: ['POST', 'PUT', 'PATCH', 'DELETE'],
            allowed_path_patterns: ['^/api/families/test-family-7a3f9c(/.*)?$'],
            action_on_violation: 'hard_fail',
        },
    ];

    const result = mod.checkRequest({
        method: 'POST',
        url: '/api/families/other-family-id/update',
        scopeTokens,
    });

    assert.strictEqual(result.allowed, false, `expected blocked, got: ${JSON.stringify(result)}`);
    assert.strictEqual(result.violation, true);
});

// ── Read-only request is always allowed ───────────────────────────────────────

test('runtime-intercept: allows GET request regardless of scope', async () => {
    const mod = await getModule();
    const scopeTokens = [
        {
            kind: 'route_filter',
            methods: ['POST', 'PUT', 'PATCH', 'DELETE'],
            allowed_path_patterns: ['^/api/families/test-family-7a3f9c(/.*)?$'],
            action_on_violation: 'hard_fail',
        },
    ];

    const result = mod.checkRequest({
        method: 'GET',
        url: '/api/families/any-family',
        scopeTokens,
    });

    assert.strictEqual(result.allowed, true, 'GET should always be allowed');
    assert.strictEqual(result.violation, false);
});

// ── Sentinel file written on violation ────────────────────────────────────────

test('runtime-intercept: writes .scope-violation sentinel on block', async () => {
    const tmpDir = makeTmpDir();
    try {
        const mod = await getModule();
        const scopeTokens = [
            {
                kind: 'route_filter',
                methods: ['POST'],
                allowed_path_patterns: ['^/api/safe-path$'],
                action_on_violation: 'hard_fail',
            },
        ];

        const result = mod.checkRequest({
            method: 'POST',
            url: '/api/unsafe-path',
            scopeTokens,
            autospecDir: tmpDir,
        });

        assert.strictEqual(result.violation, true);
        const sentinelPath = path.join(tmpDir, '.scope-violation');
        assert.ok(fs.existsSync(sentinelPath), '.scope-violation sentinel should be written on violation');
    } finally {
        fs.rmSync(tmpDir, { recursive: true, force: true });
    }
});

// ── No scope tokens: all mutating requests blocked ────────────────────────────

test('runtime-intercept: blocks all mutating requests when scopeTokens is empty', async () => {
    const mod = await getModule();

    const result = mod.checkRequest({
        method: 'DELETE',
        url: '/api/anything',
        scopeTokens: [],
    });

    assert.strictEqual(result.allowed, false, 'empty scope tokens should block all mutations');
    assert.strictEqual(result.violation, true);
});
