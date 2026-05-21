// every-row-has-required-actions.test.mjs
import { test, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import { chromium } from '/opt/homebrew/lib/node_modules/playwright/index.mjs';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const KINDS_DIR = path.resolve(__dirname, '../../../../scripts/invariants/kinds');
const FIXTURES_DIR = path.resolve(__dirname, '../../../fixtures/v2/kinds/every-row-has-required-actions');

const { id, signature, run } = await import(`${KINDS_DIR}/every-row-has-required-actions.mjs`);

let browser, page;
before(async () => { browser = await chromium.launch({ headless: true }); page = await browser.newPage(); });
after(async () => { await browser.close(); });

test('every-row-has-required-actions: id is correct', () => {
  assert.equal(id, 'every_row_has_required_actions');
});

test('every-row-has-required-actions: signature has required params', () => {
  assert.ok(signature.params.row);
  assert.ok(signature.params.required_actions);
  assert.ok(signature.required.includes('row'));
  assert.ok(signature.required.includes('required_actions'));
});

test('every-row-has-required-actions: pass fixture returns passed=true', async () => {
  await page.goto(`file://${FIXTURES_DIR}/pass.html`);
  const result = await run(page, {
    row: '[data-testid^="task-row-"]',
    required_actions: ['button:text("edit")', 'button:text("delete")'],
  }, { baseUrl: 'file://', route: '/pass' });

  assert.equal(result.passed, true, `violations: ${JSON.stringify(result.violations)}`);
  assert.equal(result.violations.length, 0);
  assert.ok(result.count_observed >= 1);
});

test('every-row-has-required-actions: fail fixture returns passed=false (missing delete)', async () => {
  await page.goto(`file://${FIXTURES_DIR}/fail.html`);
  const result = await run(page, {
    row: '[data-testid^="task-row-"]',
    required_actions: ['button:text("edit")', 'button:text("delete")'],
  }, { baseUrl: 'file://', route: '/fail' });

  assert.equal(result.passed, false, 'Expected passed=false when delete button is missing');
  assert.ok(result.violations.length > 0);
  assert.ok(result.violations.some(v => v.reason.includes('delete')));
});

test('every-row-has-required-actions: KindResult has correct shape', async () => {
  await page.goto(`file://${FIXTURES_DIR}/pass.html`);
  const result = await run(page, {
    row: '[data-testid^="task-row-"]',
    required_actions: ['button:text("edit")'],
  }, { baseUrl: 'file://', route: '/pass' });
  assert.ok('passed' in result);
  assert.ok('violations' in result);
  assert.ok('count_observed' in result);
});
