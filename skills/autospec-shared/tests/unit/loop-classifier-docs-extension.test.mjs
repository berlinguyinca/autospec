// tests/unit/loop-classifier-docs-extension.test.mjs
// Unit tests for the `regenerate` self-heal action added to
// skills/autospec-shared/scripts/loop-classifier-docs-extension.mjs (issue #922,
// spec §D6 row 1).
//
// Run: node --test skills/autospec-shared/tests/unit/loop-classifier-docs-extension.test.mjs
//
// The classifier's pre-existing per-category routing/priority behavior is
// covered by loop-classifier-docs.test.mjs; this file isolates the `regenerate`
// action contract: which signals emit it, that it carries the affected scope
// list, and that it never appears for a passing/empty gate.

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const SCRIPTS_DIR = path.resolve(__dirname, '../../scripts');
const EXT_PATH = path.join(SCRIPTS_DIR, 'loop-classifier-docs-extension.mjs');

const { classify, REGENERATE_ACTION, REGENERATE_SIGNALS } =
    await import(`file://${EXT_PATH}`);

// ── Helpers ───────────────────────────────────────────────────────────────────

function makeDriftGate(opts = {}) {
    return {
        passed: opts.passed ?? false,
        drift: opts.drift || [],
        missing_scope: opts.missingScope || [],
        visual_stale: opts.visualStale || [],
        ai_review_stale: opts.aiStale || [],
        example_stale: opts.exampleStale || [],
        skipped: opts.skipped || false,
        manifest_stale: opts.manifestStale || false,
    };
}

// ── Pinned action name + signal set ───────────────────────────────────────────

test('REGENERATE_ACTION is the pinned literal "regenerate"', () => {
    assert.equal(REGENERATE_ACTION, 'regenerate');
});

test('REGENERATE_SIGNALS is exactly drift, missing_scope, example_stale', () => {
    assert.ok(Array.isArray(REGENERATE_SIGNALS));
    assert.deepEqual(
        [...REGENERATE_SIGNALS].sort(),
        ['drift', 'example_stale', 'missing_scope'],
    );
});

// ── drift → regenerate ────────────────────────────────────────────────────────

test('drift signal emits action=regenerate carrying affected scopes', () => {
    const gate = makeDriftGate({
        drift: [
            { doc_file: 'docs/developer/architecture/core.md',
              heading: '## Core', matching_source_files: ['src/core.ts'] },
        ],
    });
    const result = classify(gate);
    assert.ok(result, 'expected a candidate');
    assert.equal(result.action, 'regenerate');
    assert.ok(Array.isArray(result.scopes));
    assert.ok(result.scopes.includes('docs/developer/architecture/core.md'),
        `expected drift doc_file in scopes, got ${JSON.stringify(result.scopes)}`);
});

// ── missing_scope → regenerate ────────────────────────────────────────────────

test('missing_scope signal emits action=regenerate carrying affected scopes', () => {
    const gate = makeDriftGate({
        missingScope: [{ source_file: 'src/new-thing.ts', suggestion: '' }],
    });
    const result = classify(gate);
    assert.ok(result, 'expected a candidate');
    assert.equal(result.action, 'regenerate');
    assert.ok(result.scopes.includes('src/new-thing.ts'),
        `expected missing source_file in scopes, got ${JSON.stringify(result.scopes)}`);
});

// ── example_stale → regenerate ────────────────────────────────────────────────

test('example_stale signal routes to a candidate with action=regenerate', () => {
    const gate = makeDriftGate({
        exampleStale: [
            { doc_file: 'docs/user/tutorials/login.md',
              heading: '## Login', verified_sha: 'abc123' },
        ],
    });
    const result = classify(gate);
    assert.ok(result, 'expected a candidate for example_stale');
    assert.equal(result.action, 'regenerate');
    assert.ok(result.scopes.includes('docs/user/tutorials/login.md'),
        `expected example_stale doc_file in scopes, got ${JSON.stringify(result.scopes)}`);
});

test('example_stale candidate has the standard schema fields', () => {
    const gate = makeDriftGate({
        exampleStale: [{ doc_file: 'docs/user/features/x.md', heading: '## X' }],
    });
    const result = classify(gate);
    assert.ok('classification' in result);
    assert.ok('target_files' in result);
    assert.ok(Array.isArray(result.target_files));
    assert.ok('suggested_action' in result);
    assert.ok('priority' in result);
    assert.ok(typeof result.priority === 'number');
});

// ── scopes are de-duplicated and only the affected ones ───────────────────────

test('regenerate scopes are de-duplicated across multiple findings', () => {
    const gate = makeDriftGate({
        drift: [
            { doc_file: 'docs/a.md', heading: '## A', matching_source_files: ['x'] },
            { doc_file: 'docs/a.md', heading: '## B', matching_source_files: ['y'] },
        ],
    });
    const result = classify(gate);
    assert.equal(result.action, 'regenerate');
    const occurrences = result.scopes.filter((s) => s === 'docs/a.md').length;
    assert.equal(occurrences, 1, 'docs/a.md should appear once');
});

// ── no regenerate when gate passes / empty ────────────────────────────────────

test('passing gate yields no candidate (no regenerate)', () => {
    assert.equal(classify(makeDriftGate({ passed: true })), null);
});

test('empty gate yields no candidate (no regenerate)', () => {
    assert.equal(classify(makeDriftGate({})), null);
});

// ── regenerate signals outrank non-regenerate ones ────────────────────────────

test('a regenerate signal is selected when present alongside lower-priority ones', () => {
    const gate = makeDriftGate({
        drift: [{ doc_file: 'docs/a.md', heading: '## A',
                  matching_source_files: ['x'] }],
        aiStale: [{ doc_file: 'docs/b.md', heading: '## B' }],
    });
    const result = classify(gate);
    assert.equal(result.action, 'regenerate',
        `expected the drift (regenerate) candidate to win, got ${result.classification}`);
});
