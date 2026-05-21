#!/usr/bin/env node
// tests/unit/assertion-shift/run-parameterized.mjs
// Parameterized test runner for the assertion-shift classifier fixture corpus.
// Run via: node --test skills/autospec-test/tests/unit/assertion-shift/
// Or directly: node skills/autospec-test/tests/unit/assertion-shift/run-parameterized.mjs

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = path.resolve(__dirname, '../../../../..');
const SCRIPTS_DIR = path.join(REPO_ROOT, 'skills/autospec-test/scripts');

// Import classifier and fixtures
const { classify } = await import(`file://${SCRIPTS_DIR}/assertion-shift-classifier.mjs`);
const { FIXTURES } = await import(`file://${__dirname}/fixtures.mjs`);

// ── Parameterized tests ────────────────────────────────────────────────────────

for (const fixture of FIXTURES) {
    test(`[${fixture.id}] ${fixture.description}`, async () => {
        const result = await classify({
            repoRoot: REPO_ROOT,
            baseRef: 'HEAD~1',
            headRef: 'HEAD',
            // Use pre-computed diff to avoid git dependency in unit tests
            diffText: fixture.diff,
            commitMessages: fixture.commitMessages,
            nonTestFilesChanged: fixture.nonTestFilesChanged,
        });

        const { gate } = result;

        // Check gate pass/fail
        assert.equal(
            gate.passed,
            fixture.expected.gate_passed,
            `[${fixture.id}] gate.passed expected ${fixture.expected.gate_passed}, got ${gate.passed}. reason=${gate.reason}, verdicts=${JSON.stringify(result.verdicts)}`
        );

        // Check reason code when gate fails
        if (!fixture.expected.gate_passed && fixture.expected.reason) {
            assert.equal(
                gate.reason,
                fixture.expected.reason,
                `[${fixture.id}] gate.reason expected '${fixture.expected.reason}', got '${gate.reason}'`
            );
        }

        // Check at least one verdict has the expected bucket (when specified)
        if (fixture.expected.any_bucket !== null && fixture.expected.any_bucket !== undefined) {
            const hasBucket = result.verdicts.some(v => v.bucket === fixture.expected.any_bucket);
            assert.ok(
                hasBucket,
                `[${fixture.id}] Expected at least one verdict with bucket='${fixture.expected.any_bucket}'. Got: ${JSON.stringify(result.verdicts.map(v => v.bucket))}`
            );
        } else if (fixture.expected.any_bucket === null) {
            // Expect zero verdicts
            assert.equal(
                result.verdicts.length,
                0,
                `[${fixture.id}] Expected zero verdicts but got: ${JSON.stringify(result.verdicts)}`
            );
        }
    });
}

// ── Additional edge case tests ─────────────────────────────────────────────────

test('classify: pure non-test file changes produce no verdicts', async () => {
    const result = await classify({
        repoRoot: REPO_ROOT,
        baseRef: 'HEAD~1',
        headRef: 'HEAD',
        diffText: `diff --git a/src/utils.js b/src/utils.js
@@ -5,7 +5,7 @@
-  return 42;
+  return 43;
`,
        commitMessages: 'fix: update return value\n',
        nonTestFilesChanged: ['src/utils.js'],
    });
    assert.equal(result.verdicts.length, 0, 'Non-test files should produce no verdicts');
    assert.equal(result.gate.passed, true);
});

test('classify: empty diff produces no verdicts and passes gate', async () => {
    const result = await classify({
        repoRoot: REPO_ROOT,
        baseRef: 'HEAD~1',
        headRef: 'HEAD',
        diffText: '',
        commitMessages: '',
        nonTestFilesChanged: [],
    });
    assert.equal(result.verdicts.length, 0);
    assert.equal(result.gate.passed, true);
});

test('classify: LOOSENING + SHIFTING together → loosening_and_unjustified_shift', async () => {
    const result = await classify({
        repoRoot: REPO_ROOT,
        baseRef: 'HEAD~1',
        headRef: 'HEAD',
        diffText: `diff --git a/src/__tests__/calc.test.js b/src/__tests__/calc.test.js
@@ -5,8 +5,8 @@ test('add', () => {
-  expect(result).toStrictEqual({a: 1});
+  expect(result).toEqual({a: 1});
-  expect(count).toBe(10);
+  expect(count).toBe(11);
`,
        commitMessages: 'fix: update\n',
        nonTestFilesChanged: [],
    });
    assert.equal(result.gate.passed, false);
    // Should have both loosening and shifting verdicts
    const hasLoosening = result.verdicts.some(v => v.bucket === 'LOOSENING');
    const hasShifting = result.verdicts.some(v => v.bucket === 'SHIFTING');
    assert.ok(hasLoosening || hasShifting, 'Should have LOOSENING or SHIFTING verdicts');
});

test('classify: STRENGTHENING-only changes pass gate', async () => {
    const result = await classify({
        repoRoot: REPO_ROOT,
        baseRef: 'HEAD~1',
        headRef: 'HEAD',
        diffText: `diff --git a/src/__tests__/calc.test.js b/src/__tests__/calc.test.js
@@ -5,6 +5,7 @@ test('add', () => {
+  expect(result).toStrictEqual({a: 1, b: 2});
`,
        commitMessages: 'test: add stricter check\n',
        nonTestFilesChanged: [],
    });
    assert.equal(result.gate.passed, true);
    assert.equal(result.gate.loosening_files.length, 0);
    assert.equal(result.gate.shifting_unjustified_files.length, 0);
});

test('classify: gate result includes has_justification and has_co_edit fields', async () => {
    const result = await classify({
        repoRoot: REPO_ROOT,
        baseRef: 'HEAD~1',
        headRef: 'HEAD',
        diffText: '',
        commitMessages: 'fix: update\nJUSTIFICATION: reason here\n',
        nonTestFilesChanged: ['src/app.js'],
    });
    assert.ok('has_justification' in result.gate, 'gate should have has_justification');
    assert.ok('has_co_edit' in result.gate, 'gate should have has_co_edit');
    assert.equal(result.gate.has_justification, true);
    assert.equal(result.gate.has_co_edit, true);
});

test('classify: verdicts have required schema fields', async () => {
    const result = await classify({
        repoRoot: REPO_ROOT,
        baseRef: 'HEAD~1',
        headRef: 'HEAD',
        diffText: `diff --git a/src/__tests__/calc.test.js b/src/__tests__/calc.test.js
@@ -5,7 +5,7 @@ test('add', () => {
-  expect(result).toBe(42);
+  expect(result).toBe(43);
`,
        commitMessages: 'fix: update\n',
        nonTestFilesChanged: [],
    });
    for (const v of result.verdicts) {
        assert.ok('file' in v, 'verdict missing file');
        assert.ok('line' in v, 'verdict missing line');
        assert.ok('bucket' in v, 'verdict missing bucket');
        assert.ok('framework' in v, 'verdict missing framework');
        assert.ok('detail' in v, 'verdict missing detail');
        assert.ok(['LOOSENING', 'SHIFTING', 'STRENGTHENING'].includes(v.bucket),
            `invalid bucket: ${v.bucket}`);
    }
});
