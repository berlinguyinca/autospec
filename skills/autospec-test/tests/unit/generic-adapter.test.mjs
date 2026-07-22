import test from 'node:test';
import assert from 'node:assert/strict';
import { bucket } from '../../scripts/adapters/generic.mjs';

const diff = (removed, added, startLine = 10) => ({
  hunks: [{
    startLine,
    removed: removed.map((content, index) => ({ content, line: startLine + index })),
    added: added.map((content, index) => ({ content, line: startLine + index })),
  }],
});

test('classifies assertion removal and addition with the file framework', () => {
  const verdicts = bucket(
    diff(['expect(value).toBe(1)'], ['expect(value).toBe(2)']),
    'example.test.ts',
  );

  assert.equal(verdicts.length, 1);
  assert.equal(verdicts[0].bucket, 'SHIFTING');
  assert.equal(verdicts[0].framework, 'playwright');
  assert.equal(verdicts[0].line, 10);
});

test('prioritizes coverage changes over assertion matching', () => {
  const verdicts = bucket(
    diff(["test('keeps coverage', () => { expect(value).toBe(1); })"], ["test.skip('keeps coverage', () => { expect(value).toBe(1); })"]),
    'example.js',
  );

  assert.equal(verdicts[0].bucket, 'LOOSENING');
});

test('classifies unpaired assertions and tolerance changes', () => {
  const removed = bucket(diff(['assert value == 1'], []), 'test_example.py');
  const tolerance = bucket(
    diff(['expect(value).toBeCloseTo(1, 2)'], ['expect(value).toBeCloseTo(1, 4)']),
    'example.js',
  );

  assert.equal(removed[0].bucket, 'LOOSENING');
  assert.equal(tolerance[0].bucket, 'LOOSENING');
});
