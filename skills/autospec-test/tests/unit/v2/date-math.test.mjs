/**
 * date-math.test.mjs — Unit tests for the date-math expression resolver.
 *
 * Run: node --test skills/autospec-test/tests/unit/v2/date-math.test.mjs
 */

import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { resolve } from '../../../scripts/window-contract/date-math.mjs';

describe('resolve()', () => {
  const TODAY = new Date('2026-05-21T00:00:00Z');
  const ctx = { today: TODAY };

  // ── Table from plan §4.2 ─────────────────────────────────────────────────

  it('resolves "today" to the ctx.today date', () => {
    assert.equal(resolve('today', ctx), '2026-05-21');
  });

  it('resolves "today - 7 days" correctly', () => {
    assert.equal(resolve('today - 7 days', ctx), '2026-05-14');
  });

  it('resolves an ISO literal unchanged', () => {
    assert.equal(resolve('2026-05-01', {}), '2026-05-01');
  });

  it('resolves "today + 3 days" correctly', () => {
    assert.equal(resolve('today + 3 days', ctx), '2026-05-24');
  });

  it('throws on unparseable expression', () => {
    assert.throws(
      () => resolve('tomorrow', {}),
      /unparseable/i,
    );
  });

  // ── Additional edge cases ────────────────────────────────────────────────

  it('resolves "today - 1 days" correctly', () => {
    assert.equal(resolve('today - 1 days', ctx), '2026-05-20');
  });

  it('resolves "today + 0 days" as today', () => {
    assert.equal(resolve('today + 0 days', ctx), '2026-05-21');
  });

  it('resolves "today - 0 days" as today', () => {
    assert.equal(resolve('today - 0 days', ctx), '2026-05-21');
  });

  it('resolves singular "today - 1 day" (without s)', () => {
    assert.equal(resolve('today - 1 day', ctx), '2026-05-20');
  });

  it('resolves singular "today + 1 day" (without s)', () => {
    assert.equal(resolve('today + 1 day', ctx), '2026-05-22');
  });

  it('handles month rollover correctly (today - 21 days)', () => {
    assert.equal(resolve('today - 21 days', ctx), '2026-04-30');
  });

  it('uses real Date.now() when ctx.today is omitted', () => {
    const result = resolve('today', {});
    assert.match(result, /^\d{4}-\d{2}-\d{2}$/);
  });

  it('throws on empty string', () => {
    assert.throws(() => resolve('', {}), /unparseable/i);
  });

  it('throws on numeric-only input', () => {
    assert.throws(() => resolve('7', {}), /unparseable/i);
  });

  it('resolves ISO literal with ctx.today present (ctx ignored for ISO)', () => {
    assert.equal(resolve('2025-12-31', ctx), '2025-12-31');
  });
});
