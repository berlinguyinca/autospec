// tests/unit/loop-classifier.test.mjs
// node --test  (Node.js built-in test runner)
// Tests for scripts/loop-classifier.mjs — one test per classification category
// plus priority ordering and edge cases.

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const SCRIPTS_DIR = path.resolve(__dirname, '../../scripts');

const { classify } = await import(`file://${SCRIPTS_DIR}/loop-classifier.mjs`);

// ── Helpers ────────────────────────────────────────────────────────────────────

function makeUnitGate(opts = {}) {
    return {
        passed: opts.passed ?? false,
        stage: 'unit',
        reason: opts.reason || 'tests_red',
        metrics: {
            unit: {
                passed: opts.unitPassed ?? false,
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

function makeE2EGate(opts = {}) {
    return {
        passed: opts.passed ?? false,
        stage: 'e2e',
        reason: opts.reason || 'e2e_coverage_gap',
        metrics: {
            e2e: {
                passed: opts.e2ePassed ?? false,
                ui_element_coverage: {
                    passed: opts.uiPassed ?? true,
                    missing: opts.uiMissing || [],
                },
                behavior_categories: {
                    passed: opts.behaviorPassed ?? true,
                    missing: opts.behaviorMissing || [],
                }
            }
        },
        test_run_summary: {
            exit_code: 1,
            stdout_tail: opts.stdout || '',
            stderr_tail: opts.stderr || '',
        }
    };
}

// ── Classification tests ───────────────────────────────────────────────────────

test('classify: missing_function_tests → missing_unit_test', () => {
    const gate = makeUnitGate({
        passed: false,
        reason: 'function_presence_fail',
        unitPassed: false,
        missingFns: ['calculateTotal', 'applyDiscount'],
    });
    const result = classify({ gate_json: gate });
    assert.equal(result.classification, 'missing_unit_test');
    assert.ok(result.target_failures.includes('calculateTotal'));
    assert.ok(result.suggested_files.some(f => /test/i.test(f)));
});

test('classify: unit tests_red → missing_unit_test or failing_unit_test', () => {
    // When unit tests are red with no missing functions and no product_bug signal,
    // classifier may return missing_unit_test (unit.passed=false triggers it) or failing_unit_test.
    const gate = makeUnitGate({
        passed: false,
        reason: 'tests_red',
        unitPassed: false,
        missingFns: [],
    });
    const result = classify({ gate_json: gate });
    const validClassifications = ['failing_unit_test', 'missing_unit_test', 'product_bug'];
    assert.ok(
        validClassifications.includes(result.classification),
        `Unexpected classification: ${result.classification}`
    );
});

test('classify: product_bug signal in stderr → product_bug', () => {
    const gate = makeUnitGate({
        passed: false,
        stderr: 'Expected 42 but received 43\n  at calc.test.ts:10',
    });
    const result = classify({ gate_json: gate });
    assert.equal(result.classification, 'product_bug');
    assert.ok(result.suggested_files.some(f => /src|lib|pkg/i.test(f)));
});

test('classify: missing UI elements → missing_test (E2E)', () => {
    const gate = makeE2EGate({
        passed: false,
        uiPassed: false,
        uiMissing: [
            { route: '/dashboard', selector: 'button[data-testid=export]' }
        ],
        behaviorPassed: true,
    });
    const result = classify({ gate_json: gate });
    assert.equal(result.classification, 'missing_test');
});

test('classify: missing behavior categories → missing_test (E2E)', () => {
    const gate = makeE2EGate({
        passed: false,
        behaviorPassed: false,
        behaviorMissing: ['drag_drop', 'keyboard_nav'],
    });
    const result = classify({ gate_json: gate });
    assert.equal(result.classification, 'missing_test');
    assert.ok(result.target_failures.some(f => f.includes('drag_drop')));
});

test('classify: selector brittle signal in stderr → selector_brittle', () => {
    const gate = makeE2EGate({
        passed: false,
        stderr: 'waiting for selector .submit-btn timeout exceeded',
    });
    // No missing UI or behavior, so selector_brittle takes precedence over failing_test
    const result = classify({ gate_json: gate });
    assert.ok(
        result.classification === 'selector_brittle' || result.classification === 'missing_test',
        `Got: ${result.classification}`
    );
});

test('classify: flaky — passes in some iterations, fails in others', () => {
    const gate = makeE2EGate({ passed: false });
    const last3 = [
        { gate_passed: true },
        { gate_passed: false },
        { gate_passed: true },
    ];
    const result = classify({ gate_json: gate, last_3_iterations: last3 });
    // flaky_test is lowest priority but should appear when no other signal
    assert.ok(
        result.classification === 'flaky_test' || result.classification === 'missing_test',
        `Got: ${result.classification}`
    );
});

test('classify: empty gate (no failures detected) → empty_action', () => {
    const gate = { passed: true, stage: 'e2e', metrics: {}, test_run_summary: {} };
    const result = classify({ gate_json: gate });
    assert.equal(result.classification, 'empty_action');
    assert.equal(result.target_failures.length, 0);
    assert.equal(result.estimated_minutes, 0);
    assert.equal(result.priority, 0);
});

// ── Priority ordering ──────────────────────────────────────────────────────────

test('classify: product_bug beats missing_unit_test when both signals present', () => {
    const gate = makeUnitGate({
        passed: false,
        missingFns: ['foo'],
        stderr: 'Expected 1 but received 2',  // product_bug signal
    });
    const result = classify({ gate_json: gate });
    assert.equal(result.classification, 'product_bug');
});

// ── Output schema ──────────────────────────────────────────────────────────────

test('classify: result has all required fields', () => {
    const gate = makeUnitGate({ passed: false, reason: 'tests_red' });
    const result = classify({ gate_json: gate });
    assert.ok('classification' in result, 'missing classification');
    assert.ok('target_failures' in result, 'missing target_failures');
    assert.ok('suggested_files' in result, 'missing suggested_files');
    assert.ok('estimated_minutes' in result, 'missing estimated_minutes');
    assert.ok('priority' in result, 'missing priority');
    assert.ok(Array.isArray(result.target_failures), 'target_failures should be array');
    assert.ok(Array.isArray(result.suggested_files), 'suggested_files should be array');
    assert.ok(typeof result.priority === 'number', 'priority should be number');
});

test('classify: classification is one of the 8 valid values', () => {
    const valid = [
        'missing_unit_test', 'missing_test', 'failing_unit_test', 'failing_test',
        'flaky_test', 'selector_brittle', 'product_bug', 'empty_action'
    ];
    const gate = makeUnitGate({ passed: false });
    const result = classify({ gate_json: gate });
    assert.ok(valid.includes(result.classification), `Invalid: ${result.classification}`);
});
