// every-visible-x-is-y.test.mjs — TDD tests for every_visible_X_is_Y kind
// Uses real Playwright (chromium headless) against file:// fixture URLs.
// Run with: node --test skills/autospec-test/tests/unit/v2/kinds/

import { test, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import { chromium } from '/opt/homebrew/lib/node_modules/playwright/index.mjs';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const KINDS_DIR = path.resolve(__dirname, '../../../../scripts/invariants/kinds');
const FIXTURES_DIR = path.resolve(__dirname, '../../../fixtures/v2/kinds/every-visible-x-is-y');

const { id, signature, run } = await import(`${KINDS_DIR}/every-visible-x-is-y.mjs`);

let browser, page;

before(async () => {
  browser = await chromium.launch({ headless: true });
  page = await browser.newPage();
});

after(async () => {
  await browser.close();
});

// ── Module contract ────────────────────────────────────────────────────────────

test('every-visible-x-is-y: id is correct', () => {
  assert.equal(id, 'every_visible_X_is_Y');
});

test('every-visible-x-is-y: signature has required params', () => {
  assert.ok(signature.params.visible, 'has visible param');
  assert.ok(signature.params.action, 'has action param');
  assert.deepEqual(signature.required, ['visible', 'action']);
});

test('every-visible-x-is-y: run is async function', () => {
  assert.equal(typeof run, 'function');
  assert.equal(run.constructor.name, 'AsyncFunction');
});

// ── Pass fixture ──────────────────────────────────────────────────────────────

test('every-visible-x-is-y: pass fixture returns passed=true', async () => {
  const passUrl = `file://${FIXTURES_DIR}/pass.html`;
  await page.goto(passUrl);

  const result = await run(page, {
    visible: '[data-testid^="done-item-row-"]',
    action: 'button[aria-label="edit"]',
    verifies_open: '[data-testid="done-item-edit-dialog"]',
    verifies_close: '#dialog button',
    require_count_at_least: 1,
  }, { baseUrl: 'file://', route: '/pass' });

  assert.equal(result.passed, true, `Expected passed=true, violations: ${JSON.stringify(result.violations)}`);
  assert.equal(result.violations.length, 0);
  assert.ok(result.count_observed >= 1);
});

// ── Fail fixture ──────────────────────────────────────────────────────────────

test('every-visible-x-is-y: fail fixture returns passed=false (no action buttons)', async () => {
  const failUrl = `file://${FIXTURES_DIR}/fail.html`;
  await page.goto(failUrl);

  const result = await run(page, {
    visible: '[data-testid^="done-item-row-"]',
    action: 'button[aria-label="edit"]',
    require_count_at_least: 1,
  }, { baseUrl: 'file://', route: '/fail' });

  assert.equal(result.passed, false, 'Expected passed=false when action buttons are missing');
  assert.ok(result.violations.length > 0);
});

// ── require_count_at_least ────────────────────────────────────────────────────

test('every-visible-x-is-y: fails when require_count_at_least not met', async () => {
  const passUrl = `file://${FIXTURES_DIR}/pass.html`;
  await page.goto(passUrl);

  const result = await run(page, {
    visible: '[data-testid^="done-item-row-"]',
    action: 'button[aria-label="edit"]',
    require_count_at_least: 100, // impossible
  }, { baseUrl: 'file://', route: '/pass' });

  assert.equal(result.passed, false);
  assert.ok(result.violations.some(v => v.reason.includes('require_count_at_least')));
});

// ── KindResult shape ──────────────────────────────────────────────────────────

test('every-visible-x-is-y: KindResult has correct shape', async () => {
  const passUrl = `file://${FIXTURES_DIR}/pass.html`;
  await page.goto(passUrl);

  const result = await run(page, {
    visible: '[data-testid^="done-item-row-"]',
    action: 'button[aria-label="edit"]',
  }, { baseUrl: 'file://', route: '/pass' });

  assert.ok('passed' in result, 'has passed');
  assert.ok('violations' in result, 'has violations');
  assert.ok('count_observed' in result, 'has count_observed');
  assert.equal(typeof result.passed, 'boolean');
  assert.ok(Array.isArray(result.violations));
  assert.equal(typeof result.count_observed, 'number');
});
