// tests/unit/loop-classifier.env-blocker.test.mjs
// node --test  (Node.js built-in test runner)
// Tests for env_blocker triage class added to scripts/loop-classifier.mjs
// per issue #998 and shared contracts from playwright-authoring design §§6,9.

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const SCRIPTS_DIR = path.resolve(__dirname, '../../scripts');

const { classify, ENV_BLOCKER_SIGNALS } = await import(`file://${SCRIPTS_DIR}/loop-classifier.mjs`);

// ── Helpers ────────────────────────────────────────────────────────────────────

function makeE2EGate(opts = {}) {
    return {
        passed: opts.passed ?? false,
        stage: 'e2e',
        reason: opts.reason || 'tests_red',
        metrics: {
            e2e: {
                passed: false,
                ui_element_coverage: { passed: true, missing: [] },
                behavior_categories: { passed: true, missing: [] },
            }
        },
        test_run_summary: {
            exit_code: 1,
            stdout_tail: opts.stdout || '',
            stderr_tail: opts.stderr || '',
        }
    };
}

function makeUnitGate(opts = {}) {
    return {
        passed: opts.passed ?? false,
        stage: 'unit',
        reason: opts.reason || 'tests_red',
        metrics: {
            unit: {
                passed: false,
                lines: 80,
                branches: 75,
                functions: 85,
                missing_function_tests: opts.missingFns || [],
            }
        },
        test_run_summary: {
            exit_code: 1,
            stdout_tail: opts.stdout || '',
            stderr_tail: opts.stderr || '',
        }
    };
}

// ── AC: CORS-error log fixture classifies as env_blocker ──────────────────────

test('classify: CORS error in stderr → env_blocker', () => {
    const gate = makeE2EGate({
        stderr: 'Access to XMLHttpRequest blocked by CORS policy: No Access-Control-Allow-Origin',
    });
    const result = classify({ gate_json: gate });
    assert.equal(result.classification, 'env_blocker',
        `Expected env_blocker but got ${result.classification}`);
    assert.ok(result.suggested_files.some(f => /config|env|infra/i.test(f)),
        'env_blocker should suggest config/env/infra files');
});

test('classify: CORS error in stdout → env_blocker', () => {
    const gate = makeE2EGate({
        stdout: 'Error: CORS preflight failed for https://api.example.com/login',
    });
    const result = classify({ gate_json: gate });
    assert.equal(result.classification, 'env_blocker');
});

// ── AC: rate-limit signals classify as env_blocker ────────────────────────────

test('classify: rate limit in stderr → env_blocker', () => {
    const gate = makeE2EGate({
        stderr: 'Request failed with status 429 Too Many Requests; rate limit exceeded',
    });
    const result = classify({ gate_json: gate });
    assert.equal(result.classification, 'env_blocker');
});

test('classify: rate-limit hyphenated in stdout → env_blocker', () => {
    const gate = makeE2EGate({
        stdout: 'Retrying after rate-limit backoff from auth service',
    });
    const result = classify({ gate_json: gate });
    assert.equal(result.classification, 'env_blocker');
});

// ── AC: auth-bootstrap 401 classifies as env_blocker ─────────────────────────

test('classify: auth bootstrap 401 on login → env_blocker', () => {
    const gate = makeE2EGate({
        stderr: 'POST /login 401 Unauthorized — auth bootstrap failed, seed user not initialized',
    });
    const result = classify({ gate_json: gate });
    assert.equal(result.classification, 'env_blocker');
});

test('classify: auth bootstrap phrase in stdout → env_blocker', () => {
    const gate = makeE2EGate({
        stdout: 'auth-bootstrap: no seed credentials found; test environment not ready',
    });
    const result = classify({ gate_json: gate });
    assert.equal(result.classification, 'env_blocker');
});

// ── AC: missing migration/reset classifies as env_blocker ────────────────────

test('classify: no migration found in stderr → env_blocker', () => {
    const gate = makeE2EGate({
        stderr: 'no migration found for users table; run db:migrate first',
    });
    const result = classify({ gate_json: gate });
    assert.equal(result.classification, 'env_blocker');
});

test('classify: missing reset endpoint → env_blocker', () => {
    const gate = makeE2EGate({
        stderr: 'GET /api/test/reset 404 Not Found — reset endpoint missing or not registered',
    });
    const result = classify({ gate_json: gate });
    assert.equal(result.classification, 'env_blocker');
});

test('classify: crawler-unreachable → env_blocker (§9 fail-closed)', () => {
    const gate = makeE2EGate({
        stderr: 'crawler-unreachable: could not connect to http://localhost:3000; check env setup',
    });
    const result = classify({ gate_json: gate });
    assert.equal(result.classification, 'env_blocker');
});

// ── AC: 500 with persisted-row-missing → product_bug, NOT env_blocker ─────────

test('classify: delete-200-but-row-persists → product_bug, not env_blocker', () => {
    // A product_bug: delete returned 200 but row still present in DB
    const gate = makeE2EGate({
        stderr: 'Expected row to be deleted but received existing row; assertion failed: Expected 0 but received 1',
    });
    const result = classify({ gate_json: gate });
    assert.equal(result.classification, 'product_bug',
        `Expected product_bug but got ${result.classification}`);
    assert.notEqual(result.classification, 'env_blocker',
        'A product assertion failure must not be misclassified as env_blocker');
});

test('classify: 500 error with data truly absent → product_bug, not env_blocker', () => {
    const gate = makeE2EGate({
        stderr: 'GET /api/items 500 Internal Server Error; Expected items array but received empty',
    });
    const result = classify({ gate_json: gate });
    // product_bug signals (Expected ... but received) take priority over 5xx env
    assert.equal(result.classification, 'product_bug',
        `Expected product_bug but got ${result.classification}`);
});

// ── AC: env_blocker does not bypass assertion-shift anti-loosen guard ─────────

test('classify: env_blocker classification does not appear in assertion-shift path', async () => {
    // The loop-classifier and assertion-shift-classifier are separate guards.
    // Verifying loop-classifier env_blocker result shape doesn't include assertion-related fields.
    const gate = makeE2EGate({
        stderr: 'CORS policy blocked request to /api/auth',
    });
    const result = classify({ gate_json: gate });
    assert.equal(result.classification, 'env_blocker');
    // env_blocker result must NOT include loosening_files or shifting_unjustified_files
    // (those are assertion-shift-classifier's domain — env_blocker has no bypass)
    assert.ok(!('loosening_files' in result),
        'env_blocker result must not carry loosening_files (assertion-shift domain)');
    assert.ok(!('shifting_unjustified_files' in result),
        'env_blocker result must not carry shifting_unjustified_files');
});

test('classify: env_blocker path routes to environment-fix, never assertion-loosen', () => {
    const gate = makeE2EGate({
        stderr: 'auth-bootstrap: 401 login failed; test stack not initialized',
    });
    const result = classify({ gate_json: gate });
    assert.equal(result.classification, 'env_blocker');
    // suggested_files must point to env/config/infra, NOT test files
    const hasTestPath = result.suggested_files.some(f =>
        /\btest[s]?\b|\bspec\b|\bplaywrigh/i.test(f)
    );
    assert.ok(!hasTestPath,
        `env_blocker suggested_files should not point to test dirs; got: ${JSON.stringify(result.suggested_files)}`);
});

// ── Existing classes still classify unchanged ──────────────────────────────────

test('existing: selector_brittle signal still classifies correctly', () => {
    const gate = makeE2EGate({
        stderr: 'waiting for selector .submit-btn timeout exceeded',
    });
    const result = classify({ gate_json: gate });
    // selector_brittle or missing_test — not env_blocker
    assert.notEqual(result.classification, 'env_blocker',
        'selector timeout must not be misrouted to env_blocker');
    assert.ok(
        result.classification === 'selector_brittle' || result.classification === 'missing_test',
        `Expected selector_brittle or missing_test, got ${result.classification}`
    );
});

test('existing: product_bug signal still classifies correctly', () => {
    const gate = makeUnitGate({
        stderr: 'Expected 42 but received 43',
    });
    const result = classify({ gate_json: gate });
    assert.equal(result.classification, 'product_bug');
});

test('existing: missing_unit_test still classifies correctly', () => {
    const gate = makeUnitGate({
        missingFns: ['calculateTotal', 'applyDiscount'],
    });
    const result = classify({ gate_json: gate });
    assert.equal(result.classification, 'missing_unit_test');
});

// ── ENV_BLOCKER_SIGNALS export ─────────────────────────────────────────────────

test('ENV_BLOCKER_SIGNALS is exported and is an array of RegExps', () => {
    assert.ok(Array.isArray(ENV_BLOCKER_SIGNALS),
        'ENV_BLOCKER_SIGNALS must be exported as an array');
    assert.ok(ENV_BLOCKER_SIGNALS.length >= 4,
        'ENV_BLOCKER_SIGNALS must include at least 4 signal patterns');
    for (const sig of ENV_BLOCKER_SIGNALS) {
        assert.ok(sig instanceof RegExp,
            `Each signal must be a RegExp; got ${typeof sig}`);
    }
});

test('ENV_BLOCKER_SIGNALS covers CORS', () => {
    const hasCors = ENV_BLOCKER_SIGNALS.some(r => r.test('CORS policy blocked'));
    assert.ok(hasCors, 'ENV_BLOCKER_SIGNALS must match CORS');
});

test('ENV_BLOCKER_SIGNALS covers rate-limit', () => {
    const hasRateLimit = ENV_BLOCKER_SIGNALS.some(r => r.test('rate-limit exceeded'));
    assert.ok(hasRateLimit, 'ENV_BLOCKER_SIGNALS must match rate-limit');
});

test('ENV_BLOCKER_SIGNALS covers auth-bootstrap', () => {
    const hasAuth = ENV_BLOCKER_SIGNALS.some(r => r.test('auth-bootstrap failed'));
    assert.ok(hasAuth, 'ENV_BLOCKER_SIGNALS must match auth-bootstrap');
});

test('ENV_BLOCKER_SIGNALS covers missing migration', () => {
    const hasMigration = ENV_BLOCKER_SIGNALS.some(r => r.test('no migration found'));
    assert.ok(hasMigration, 'ENV_BLOCKER_SIGNALS must match missing migration');
});

// ── Output schema for env_blocker ─────────────────────────────────────────────

test('classify: env_blocker result has all required fields', () => {
    const gate = makeE2EGate({ stderr: 'CORS blocked' });
    const result = classify({ gate_json: gate });
    assert.equal(result.classification, 'env_blocker');
    assert.ok('target_failures' in result, 'missing target_failures');
    assert.ok('suggested_files' in result, 'missing suggested_files');
    assert.ok('estimated_minutes' in result, 'missing estimated_minutes');
    assert.ok('priority' in result, 'missing priority');
    assert.ok(Array.isArray(result.target_failures));
    assert.ok(Array.isArray(result.suggested_files));
    assert.ok(typeof result.priority === 'number');
    assert.ok(result.priority > 0, 'env_blocker must have non-zero priority');
});

// ── env_blocker priority fits between product_bug and selector_brittle ─────────

test('classify: env_blocker priority is below product_bug', () => {
    // env_blocker should not outrank product_bug
    const productBugGate = makeUnitGate({
        stderr: 'Expected 1 but received 2',  // product_bug signal
        stdout: 'CORS policy blocked',         // also has env_blocker
    });
    const result = classify({ gate_json: productBugGate });
    // product_bug must win
    assert.equal(result.classification, 'product_bug',
        'product_bug must outrank env_blocker');
});
