// every-foldout-opens-all-nested.test.mjs
import { test, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import { chromium } from '/opt/homebrew/lib/node_modules/playwright/index.mjs';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const KINDS_DIR = path.resolve(__dirname, '../../../../scripts/invariants/kinds');
const FIXTURES_DIR = path.resolve(__dirname, '../../../fixtures/v2/kinds/every-foldout-opens-all-nested');

const { id, signature, run } = await import(`${KINDS_DIR}/every-foldout-opens-all-nested.mjs`);

let browser, page;
before(async () => { browser = await chromium.launch({ headless: true }); page = await browser.newPage(); });
after(async () => { await browser.close(); });

test('every-foldout-opens-all-nested: id is correct', () => {
  assert.equal(id, 'every_foldout_opens_all_nested');
});

test('every-foldout-opens-all-nested: signature has required params', () => {
  assert.ok(signature.params.foldout);
  assert.ok(signature.params.nested_must_be_visible_after_open);
  assert.ok(signature.required.includes('foldout'));
  assert.ok(signature.required.includes('nested_must_be_visible_after_open'));
});

test('every-foldout-opens-all-nested: pass fixture returns passed=true', async () => {
  await page.goto(`file://${FIXTURES_DIR}/pass.html`);
  const result = await run(page, {
    foldout: '[data-testid^="foldout-day-"]',
    nested_must_be_visible_after_open: '[data-testid^="nested-row-"]',
  }, { baseUrl: 'file://', route: '/pass' });

  assert.equal(result.passed, true, `violations: ${JSON.stringify(result.violations)}`);
  assert.equal(result.violations.length, 0);
  assert.ok(result.count_observed >= 1);
});

test('every-foldout-opens-all-nested: fail fixture returns passed=false', async () => {
  await page.goto(`file://${FIXTURES_DIR}/fail.html`);
  const result = await run(page, {
    foldout: '[data-testid^="foldout-day-"]',
    nested_must_be_visible_after_open: '[data-testid^="nested-row-"]',
  }, { baseUrl: 'file://', route: '/fail' });

  assert.equal(result.passed, false, 'Expected passed=false for foldout with no nested rows');
  assert.ok(result.violations.length > 0);
});

test('every-foldout-opens-all-nested: KindResult has correct shape', async () => {
  await page.goto(`file://${FIXTURES_DIR}/pass.html`);
  const result = await run(page, {
    foldout: '[data-testid^="foldout-day-"]',
    nested_must_be_visible_after_open: '[data-testid^="nested-row-"]',
  }, { baseUrl: 'file://', route: '/pass' });
  assert.ok('passed' in result);
  assert.ok('violations' in result);
  assert.ok('count_observed' in result);
  assert.equal(typeof result.passed, 'boolean');
  assert.ok(Array.isArray(result.violations));
});
