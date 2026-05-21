// every-modal-returns-to-body-scroll.test.mjs
import { test, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import { chromium } from '/opt/homebrew/lib/node_modules/playwright/index.mjs';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const KINDS_DIR = path.resolve(__dirname, '../../../../scripts/invariants/kinds');
const FIXTURES_DIR = path.resolve(__dirname, '../../../fixtures/v2/kinds/every-modal-returns-to-body-scroll');

const { id, signature, run } = await import(`${KINDS_DIR}/every-modal-returns-to-body-scroll.mjs`);

let browser, page;
before(async () => { browser = await chromium.launch({ headless: true }); page = await browser.newPage(); });
after(async () => { await browser.close(); });

test('every-modal-returns-to-body-scroll: id is correct', () => {
  assert.equal(id, 'every_modal_returns_to_body_scroll');
});

test('every-modal-returns-to-body-scroll: signature has required params', () => {
  assert.ok(signature.params.modal_open);
  assert.ok(signature.params.modal_selector);
  assert.ok(signature.params.modal_close);
  assert.ok(signature.required.includes('modal_open'));
  assert.ok(signature.required.includes('modal_selector'));
  assert.ok(signature.required.includes('modal_close'));
});

test('every-modal-returns-to-body-scroll: pass fixture returns passed=true', async () => {
  await page.goto(`file://${FIXTURES_DIR}/pass.html`);
  const result = await run(page, {
    modal_open: '#open-modal',
    modal_selector: '#modal',
    modal_close: '#close-modal',
  }, { baseUrl: 'file://', route: '/pass' });

  assert.equal(result.passed, true, `violations: ${JSON.stringify(result.violations)}`);
  assert.equal(result.violations.length, 0);
});

test('every-modal-returns-to-body-scroll: fail fixture returns passed=false (overflow not restored)', async () => {
  await page.goto(`file://${FIXTURES_DIR}/fail.html`);
  const result = await run(page, {
    modal_open: '#open-modal',
    modal_selector: '#modal',
    modal_close: '#close-modal',
  }, { baseUrl: 'file://', route: '/fail' });

  assert.equal(result.passed, false, 'Expected passed=false when body overflow is not restored');
  assert.ok(result.violations.some(v => v.reason.includes('overflow')));
});

test('every-modal-returns-to-body-scroll: KindResult has correct shape', async () => {
  await page.goto(`file://${FIXTURES_DIR}/pass.html`);
  const result = await run(page, {
    modal_open: '#open-modal',
    modal_selector: '#modal',
    modal_close: '#close-modal',
  }, { baseUrl: 'file://', route: '/pass' });
  assert.ok('passed' in result);
  assert.ok('violations' in result);
  assert.ok('count_observed' in result);
  assert.equal(typeof result.passed, 'boolean');
  assert.ok(Array.isArray(result.violations));
});
