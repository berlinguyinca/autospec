// every-visible-x-has-accessible-name.test.mjs
import { test, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import { chromium } from '/opt/homebrew/lib/node_modules/playwright/index.mjs';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const KINDS_DIR = path.resolve(__dirname, '../../../../scripts/invariants/kinds');
const FIXTURES_DIR = path.resolve(__dirname, '../../../fixtures/v2/kinds/every-visible-x-has-accessible-name');

const { id, signature, run } = await import(`${KINDS_DIR}/every-visible-x-has-accessible-name.mjs`);

let browser, page;
before(async () => { browser = await chromium.launch({ headless: true }); page = await browser.newPage(); });
after(async () => { await browser.close(); });

test('every-visible-x-has-accessible-name: id is correct', () => {
  assert.equal(id, 'every_visible_X_has_accessible_name');
});

test('every-visible-x-has-accessible-name: signature has params object', () => {
  assert.ok(signature.params, 'has params');
  assert.deepEqual(signature.required, [], 'required is empty (all params optional)');
});

test('every-visible-x-has-accessible-name: pass fixture returns passed=true', async () => {
  await page.goto(`file://${FIXTURES_DIR}/pass.html`);
  const result = await run(page, {}, { baseUrl: 'file://', route: '/pass' });
  assert.equal(result.passed, true, `violations: ${JSON.stringify(result.violations)}`);
  assert.equal(result.violations.length, 0);
  assert.ok(result.count_observed >= 1);
});

test('every-visible-x-has-accessible-name: fail fixture returns passed=false (empty button)', async () => {
  await page.goto(`file://${FIXTURES_DIR}/fail.html`);
  const result = await run(page, {}, { baseUrl: 'file://', route: '/fail' });
  assert.equal(result.passed, false, 'Expected passed=false for elements with no accessible name');
  assert.ok(result.violations.length > 0);
});

test('every-visible-x-has-accessible-name: KindResult has correct shape', async () => {
  await page.goto(`file://${FIXTURES_DIR}/pass.html`);
  const result = await run(page, {}, { baseUrl: 'file://', route: '/pass' });
  assert.ok('passed' in result);
  assert.ok('violations' in result);
  assert.ok('count_observed' in result);
  assert.equal(typeof result.count_observed, 'number');
});
