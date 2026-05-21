// tests/unit/error-signature.test.mjs
// node --test  (Node.js built-in test runner)
// Tests for scripts/error-signature.mjs

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const SCRIPTS_DIR = path.resolve(__dirname, '../../scripts');

const { normalize, signature } = await import(`file://${SCRIPTS_DIR}/error-signature.mjs`);

// ── normalize() tests ─────────────────────────────────────────────────────────

test('normalize: strips line:col numbers', () => {
    const result = normalize('Error at src/foo.ts:123:45');
    assert.ok(!result.includes('123'), `Should strip line number: ${result}`);
    assert.ok(!result.includes('45'), `Should strip col number: ${result}`);
});

test('normalize: strips "line N" patterns', () => {
    const result = normalize('Failed at line 42 in test');
    assert.ok(!result.includes('42'), `Should strip line number: ${result}`);
    assert.ok(result.includes('line L'), `Should replace with L: ${result}`);
});

test('normalize: strips browser tags [chromium] [firefox] [webkit]', () => {
    const r1 = normalize('Test failed [chromium] in dashboard');
    const r2 = normalize('Test failed [firefox] in dashboard');
    const r3 = normalize('Test failed [webkit] in dashboard');
    assert.ok(!r1.includes('[chromium]'));
    assert.ok(!r2.includes('[firefox]'));
    assert.ok(!r3.includes('[webkit]'));
    // All should produce same normalized text
    assert.equal(r1, r2);
    assert.equal(r2, r3);
});

test('normalize: strips worker tags', () => {
    // Both [worker1] and [worker2] bracket tags should normalize to [BROWSER]
    // And bare workerN words should normalize to WORKER
    const r1 = normalize('timeout [worker1] failed');
    const r2 = normalize('timeout [worker2] failed');
    assert.equal(r1, r2);
    assert.ok(!r1.includes('worker1'), `Should strip worker1: ${r1}`);
});

test('normalize: strips ISO-8601 timestamps', () => {
    const result = normalize('Timeout 2026-05-21T20:30:00Z exceeded');
    assert.ok(!result.includes('2026-05-21'), `Should strip timestamp: ${result}`);
    assert.ok(result.includes('TIMESTAMP'), `Should replace with TIMESTAMP: ${result}`);
});

test('normalize: strips UUIDs', () => {
    const result = normalize('Session id: 550e8400-e29b-41d4-a716-446655440000 done');
    assert.ok(!result.includes('550e8400'), `Should strip UUID: ${result}`);
    assert.ok(result.includes('UUID'), `Should replace with UUID: ${result}`);
});

test('normalize: normalizes whitespace', () => {
    const result = normalize('Error:   too   many   spaces');
    assert.ok(!result.includes('  '), `Should normalize spaces: ${result}`);
});

test('normalize: empty string returns empty', () => {
    assert.equal(normalize(''), '');
});

test('normalize: null-like input returns empty', () => {
    assert.equal(normalize(null), '');
    assert.equal(normalize(undefined), '');
});

// ── signature() tests — same root cause → same hash ───────────────────────────

test('signature: same error different line numbers → same hash', () => {
    const err1 = 'expect(received).toBe(expected) at src/calc.ts:10:5';
    const err2 = 'expect(received).toBe(expected) at src/calc.ts:20:3';
    assert.equal(signature(err1), signature(err2));
});

test('signature: same error different browsers → same hash', () => {
    const err1 = 'Test failed [chromium] - element not found';
    const err2 = 'Test failed [firefox] - element not found';
    assert.equal(signature(err1), signature(err2));
});

test('signature: same error different timestamps → same hash', () => {
    // Timestamps are stripped before hashing — same error at different times = same signature
    const err1 = 'Timeout after 2026-05-21T10:00:00Z exceeded limit';
    const err2 = 'Timeout after 2026-05-21T11:30:00Z exceeded limit';
    assert.equal(signature(err1), signature(err2));
});

test('signature: different errors → different hashes', () => {
    const err1 = 'expect(42).toBe(43)';
    const err2 = 'expect("hello").toBe("world")';
    assert.notEqual(signature(err1), signature(err2));
});

test('signature: returns 64-char hex string', () => {
    const sig = signature('some error text');
    assert.equal(typeof sig, 'string');
    assert.equal(sig.length, 64);
    assert.ok(/^[0-9a-f]{64}$/.test(sig), `Not valid hex: ${sig}`);
});

test('signature: empty input produces consistent hash', () => {
    assert.equal(signature(''), signature(''));
    assert.equal(signature(''), signature('   '));  // whitespace normalizes to empty
});

test('signature: idempotent — same input always same output', () => {
    const text = 'Error: Cannot find module "calculator"\n  at require (line 5:3)\n[chromium]';
    const sig1 = signature(text);
    const sig2 = signature(text);
    assert.equal(sig1, sig2);
});
