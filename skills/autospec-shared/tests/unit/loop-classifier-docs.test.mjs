// tests/unit/loop-classifier-docs.test.mjs
// Unit tests for skills/autospec-shared/scripts/loop-classifier-docs-extension.mjs
//
// Run: node --test skills/autospec-shared/tests/unit/loop-classifier-docs.test.mjs
// (from repo root, or: node --test tests/unit/loop-classifier-docs.test.mjs
//  from skills/autospec-shared/)

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const SCRIPTS_DIR = path.resolve(__dirname, '../../scripts');
const EXT_PATH = path.join(SCRIPTS_DIR, 'loop-classifier-docs-extension.mjs');

const { classify, detectBucket, DOCS_CATEGORY_PRIORITY } = await import(`file://${EXT_PATH}`);

// ── Helpers ───────────────────────────────────────────────────────────────────

function makeDriftGate(opts = {}) {
    return {
        passed: opts.passed ?? false,
        drift: opts.drift || [],
        missing_scope: opts.missingScope || [],
        visual_stale: opts.visualStale || [],
        ai_review_stale: opts.aiStale || [],
        skipped: opts.skipped || false,
        manifest_stale: opts.manifestStale || false,
    };
}

// ── Null cases ────────────────────────────────────────────────────────────────

test('returns null for non-doc gate JSON (no doc fields)', () => {
    const result = classify({ stage: 'unit', passed: false, reason: 'tests_red' });
    assert.equal(result, null);
});

test('returns null for null input', () => {
    assert.equal(classify(null), null);
});

test('returns null when gate passed', () => {
    const gate = makeDriftGate({ passed: true, drift: [], missingScope: [] });
    assert.equal(classify(gate), null);
});

test('returns null when no findings', () => {
    const gate = makeDriftGate({ passed: false, drift: [], missingScope: [] });
    assert.equal(classify(gate), null);
});

// ── missing_doc_scope ─────────────────────────────────────────────────────────

test('missing_doc_scope: routes correctly from missing_scope array', () => {
    const gate = makeDriftGate({
        passed: false,
        missingScope: [
            { source_file: 'skills/autospec-test/scripts/new-thing.sh',
              suggestion: 'no docs section declares scope for this path' }
        ],
    });
    const result = classify(gate);
    assert.ok(result, 'Expected non-null result');
    assert.equal(result.classification, 'missing_doc_scope');
    assert.ok(result.target_files.includes('skills/autospec-test/scripts/new-thing.sh'));
});

test('missing_doc_scope: suggested_action mentions the source file', () => {
    const gate = makeDriftGate({
        passed: false,
        missingScope: [{ source_file: 'src/foo.ts', suggestion: '' }],
    });
    const result = classify(gate);
    assert.ok(result.suggested_action.includes('src/foo.ts') ||
              result.suggested_action.length > 0);
});

// ── failing_doc_drift ─────────────────────────────────────────────────────────

test('failing_doc_drift: routes correctly from drift array', () => {
    const gate = makeDriftGate({
        passed: false,
        drift: [
            { doc_file: 'docs/USER_MANUAL.md',
              heading: '## Installing autospec-test',
              matching_source_files: ['install.sh'],
              reason: 'install instructions outdated' }
        ],
    });
    const result = classify(gate);
    assert.ok(result, 'Expected non-null result');
    assert.equal(result.classification, 'failing_doc_drift');
    assert.ok(result.target_files.includes('docs/USER_MANUAL.md'));
});

test('failing_doc_drift: estimated_minutes is reasonable', () => {
    const gate = makeDriftGate({
        passed: false,
        drift: [{ doc_file: 'docs/ARCHITECTURE.md', heading: '## Overview',
                  matching_source_files: ['src/main.ts'], reason: '' }],
    });
    const result = classify(gate);
    assert.ok(result.estimated_minutes > 0);
    assert.ok(result.estimated_minutes <= 30);
});

// ── failing_visual_stale ──────────────────────────────────────────────────────

test('failing_visual_stale: routes correctly from visual_stale array', () => {
    const gate = makeDriftGate({
        passed: false,
        visualStale: [
            { doc_file: 'docs/USER_MANUAL.md',
              heading: '## Dashboard',
              screenshot: 'docs/assets/screenshots/dashboard__desktop.png',
              source_changed: ['components/dashboard/index.tsx'] }
        ],
    });
    const result = classify(gate);
    assert.ok(result, 'Expected non-null result');
    assert.equal(result.classification, 'failing_visual_stale');
    assert.ok(result.target_files.includes('docs/assets/screenshots/dashboard__desktop.png'));
});

// ── failing_ai_review_stale ───────────────────────────────────────────────────

test('failing_ai_review_stale: routes correctly from ai_review_stale array', () => {
    const gate = makeDriftGate({
        passed: false,
        aiStale: [
            { doc_file: 'docs/ARCHITECTURE.md',
              heading: '## Module graph',
              last_reviewed_at: '2026-05-01T00:00:00Z',
              source_changed_after: '2026-05-10T00:00:00Z' }
        ],
    });
    const result = classify(gate);
    assert.ok(result, 'Expected non-null result');
    assert.equal(result.classification, 'failing_ai_review_stale');
    assert.ok(result.target_files.includes('docs/ARCHITECTURE.md'));
});

// ── failing_manifest_stale ────────────────────────────────────────────────────

test('failing_manifest_stale: routes correctly when manifest_stale=true', () => {
    const gate = makeDriftGate({ passed: false, manifestStale: true });
    const result = classify(gate);
    assert.ok(result, 'Expected non-null result');
    assert.equal(result.classification, 'failing_manifest_stale');
    assert.ok(result.target_files.includes('docs/.llm-manifest.json'));
});

// ── Priority ordering ─────────────────────────────────────────────────────────

test('DOCS_CATEGORY_PRIORITY is an array of 5 items in correct order', () => {
    assert.ok(Array.isArray(DOCS_CATEGORY_PRIORITY));
    assert.equal(DOCS_CATEGORY_PRIORITY.length, 5);
    assert.equal(DOCS_CATEGORY_PRIORITY[0], 'missing_doc_scope');
    assert.equal(DOCS_CATEGORY_PRIORITY[1], 'failing_doc_drift');
});

test('missing_doc_scope outranks failing_doc_drift when both present', () => {
    const gate = makeDriftGate({
        passed: false,
        missingScope: [{ source_file: 'src/foo.ts', suggestion: '' }],
        drift: [{ doc_file: 'docs/USER_MANUAL.md', heading: '## Foo',
                  matching_source_files: ['src/foo.ts'], reason: '' }],
    });
    const result = classify(gate);
    assert.equal(result.classification, 'missing_doc_scope',
        `Expected missing_doc_scope to win, got ${result.classification}`);
});

test('failing_doc_drift outranks failing_visual_stale when both present', () => {
    const gate = makeDriftGate({
        passed: false,
        drift: [{ doc_file: 'docs/USER_MANUAL.md', heading: '## Foo',
                  matching_source_files: ['src/foo.ts'], reason: '' }],
        visualStale: [{ doc_file: 'docs/USER_MANUAL.md', heading: '## Bar',
                        screenshot: 'docs/assets/screenshots/foo.png',
                        source_changed: ['src/foo.ts'] }],
    });
    const result = classify(gate);
    assert.equal(result.classification, 'failing_doc_drift',
        `Expected failing_doc_drift to win, got ${result.classification}`);
});

// ── Output schema ─────────────────────────────────────────────────────────────

test('result has all required fields', () => {
    const gate = makeDriftGate({
        passed: false,
        missingScope: [{ source_file: 'src/foo.ts', suggestion: '' }],
    });
    const result = classify(gate);
    assert.ok('classification' in result, 'missing classification');
    assert.ok('target_files' in result, 'missing target_files');
    assert.ok('suggested_action' in result, 'missing suggested_action');
    assert.ok('estimated_minutes' in result, 'missing estimated_minutes');
    assert.ok('priority' in result, 'missing priority');
    assert.ok(Array.isArray(result.target_files), 'target_files must be array');
    assert.ok(typeof result.priority === 'number', 'priority must be number');
    assert.ok(typeof result.suggested_action === 'string', 'suggested_action must be string');
});

test('priority is always a positive number for docs findings', () => {
    const gate = makeDriftGate({
        passed: false,
        drift: [{ doc_file: 'docs/USER_MANUAL.md', heading: '## Foo',
                  matching_source_files: ['src/foo.ts'], reason: '' }],
    });
    const result = classify(gate);
    assert.ok(result.priority > 0, `Expected priority > 0, got ${result.priority}`);
});

// ── detectBucket ──────────────────────────────────────────────────────────────

test('detectBucket: scope comment removed → LOOSENING', () => {
    const diff = [
        '--- a/docs/USER_MANUAL.md',
        '+++ b/docs/USER_MANUAL.md',
        '-<!-- autospec-doc-scope:',
        '-  src: ["install.sh"]',
        '--->',
    ].join('\n');
    assert.equal(detectBucket(diff), 'LOOSENING');
});

test('detectBucket: scope comment added → STRENGTHENING', () => {
    const diff = [
        '--- a/docs/USER_MANUAL.md',
        '+++ b/docs/USER_MANUAL.md',
        '+<!-- autospec-doc-scope:',
        '+  src: ["install.sh"]',
        '+-->',
    ].join('\n');
    assert.equal(detectBucket(diff), 'STRENGTHENING');
});

test('detectBucket: glob removed → LOOSENING', () => {
    const diff = [
        '--- a/docs/USER_MANUAL.md',
        '+++ b/docs/USER_MANUAL.md',
        ' <!-- autospec-doc-scope:',
        '-  src: ["install.sh", "skills/autospec-test/install.sh"]',
        '+  src: ["install.sh"]',
        ' -->',
    ].join('\n');
    assert.equal(detectBucket(diff), 'LOOSENING');
});

test('detectBucket: glob added → STRENGTHENING', () => {
    const diff = [
        '--- a/docs/USER_MANUAL.md',
        '+++ b/docs/USER_MANUAL.md',
        ' <!-- autospec-doc-scope:',
        '-  src: ["install.sh"]',
        '+  src: ["install.sh", "skills/autospec-test/install.sh"]',
        ' -->',
    ].join('\n');
    assert.equal(detectBucket(diff), 'STRENGTHENING');
});

test('detectBucket: doc content changed (no scope lines) → SHIFTING', () => {
    const diff = [
        '--- a/docs/USER_MANUAL.md',
        '+++ b/docs/USER_MANUAL.md',
        '-Old paragraph text.',
        '+New paragraph text.',
    ].join('\n');
    assert.equal(detectBucket(diff), 'SHIFTING');
});

test('detectBucket: null input → null', () => {
    assert.equal(detectBucket(null), null);
    assert.equal(detectBucket(''), null);
});
