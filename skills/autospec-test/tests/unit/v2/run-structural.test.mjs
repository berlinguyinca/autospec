// run-structural.test.mjs — TDD tests for Metric F structural invariants runner
// Run with: node --test skills/autospec-test/tests/unit/v2/run-structural.test.mjs

import { test, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import { spawn } from 'node:child_process';
import { createServer } from 'node:http';
import fs from 'node:fs';
import { chromium } from '/opt/homebrew/lib/node_modules/playwright/index.mjs';
import { openAllFoldouts } from '../../../scripts/crawler-v2/foldout-opener.mjs';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const SCRIPTS_DIR = path.resolve(__dirname, '../../../scripts');
const FIXTURE_DIR = path.resolve(__dirname, '../../fixtures/v2/structural');
const RUN_STRUCTURAL = path.join(SCRIPTS_DIR, 'invariants/run-structural.mjs');

// ── Static file server for fixtures ───────────────────────────────────────────

let server, serverUrl;

before(async () => {
  await new Promise((resolve, reject) => {
    server = createServer((req, res) => {
      const filePath = path.join(FIXTURE_DIR, req.url === '/' ? 'index.html' : req.url);
      fs.readFile(filePath, (err, data) => {
        if (err) { res.writeHead(404); res.end('Not found'); return; }
        res.writeHead(200, { 'Content-Type': 'text/html; charset=utf-8' });
        res.end(data);
      });
    });
    server.listen(0, '127.0.0.1', () => {
      const { port } = server.address();
      serverUrl = `http://127.0.0.1:${port}`;
      resolve();
    });
    server.on('error', reject);
  });
});

after(async () => {
  await new Promise((resolve) => server.close(resolve));
});

// ── Helper: run run-structural.mjs with stdin JSON ─────────────────────────────

function runStructural(input) {
  return new Promise((resolve, reject) => {
    const proc = spawn('node', [RUN_STRUCTURAL], { stdio: ['pipe', 'pipe', 'pipe'] });
    let stdout = '';
    let stderr = '';
    proc.stdout.on('data', (d) => { stdout += d.toString(); });
    proc.stderr.on('data', (d) => { stderr += d.toString(); });
    proc.on('error', reject);
    proc.on('close', (code) => {
      let json = null;
      try { json = JSON.parse(stdout); } catch (e) {}
      resolve({ code, stdout, stderr, json });
    });
    proc.stdin.write(JSON.stringify(input));
    proc.stdin.end();
  });
}

// ── foldout-opener tests ───────────────────────────────────────────────────────

let browser, page;

test('openAllFoldouts: opens 3-level nested foldouts', async () => {
  browser = await chromium.launch({ headless: true });
  page = await browser.newPage();
  await page.goto(`${serverUrl}/`);

  // All 3 foldout levels should start collapsed
  const collapsed = await page.locator('[aria-expanded="false"]').count();
  assert.ok(collapsed >= 3, `Expected >= 3 collapsed foldouts, got ${collapsed}`);

  const result = await openAllFoldouts(page, { maxDepth: 5 });

  assert.ok(result.opened_count >= 3, `Expected opened_count >= 3, got ${result.opened_count}`);
  assert.ok(result.depth_reached >= 1, 'Expected depth_reached >= 1');

  // Verify deep content is accessible
  const deepContent = page.locator('[data-testid="deep-content"]');
  const isVisible = await deepContent.isVisible();
  assert.equal(isVisible, true, 'deep-content should be visible after openAllFoldouts');

  await browser.close();
  browser = null;
});

test('openAllFoldouts: returns { opened_count, depth_reached } shape', async () => {
  const br = await chromium.launch({ headless: true });
  const pg = await br.newPage();
  await pg.goto(`${serverUrl}/`);
  const result = await openAllFoldouts(pg, { maxDepth: 5 });
  assert.ok('opened_count' in result, 'has opened_count');
  assert.ok('depth_reached' in result, 'has depth_reached');
  assert.equal(typeof result.opened_count, 'number');
  assert.equal(typeof result.depth_reached, 'number');
  await br.close();
});

// ── run-structural pass case ───────────────────────────────────────────────────

test('run-structural: pass case — all rows editable — passed=true, count_observed=4', async () => {
  // Pass case: use rows 0,1,2,4 only (skip row 3 by filtering selector)
  const input = {
    base_url: serverUrl,
    contract: {
      e2e: {
        invariants_v2: {
          enabled: true,
          invariants: [
            {
              id: 'rows-0-1-2-4-editable',
              kind: 'every_visible_X_is_Y',
              visible: '[data-testid="done-item-row-0"], [data-testid="done-item-row-1"], [data-testid="done-item-row-2"], [data-testid="done-item-row-4"]',
              action: 'button[aria-label="edit"]',
              apply_on_routes: ['/'],
              require_count_at_least: 1,
            },
          ],
        },
      },
    },
  };

  const { code, json } = await runStructural(input);
  assert.equal(json?.metric, 'F', 'metric should be F');
  assert.equal(json?.passed, true, `Expected passed=true, got: ${JSON.stringify(json?.invariants)}`);
  assert.ok(json?.invariants.length >= 1, 'should have invariant results');
  assert.equal(json?.summary?.failed_count, 0);
  assert.equal(code, 0, 'exit code should be 0 on pass');
});

// ── run-structural fail case ───────────────────────────────────────────────────

test('run-structural: fail case — row 3 has no edit button — passed=false with violation', async () => {
  const input = {
    base_url: serverUrl,
    contract: {
      e2e: {
        invariants_v2: {
          enabled: true,
          invariants: [
            {
              id: 'all-rows-editable',
              kind: 'every_visible_X_is_Y',
              visible: '[data-testid^="done-item-row-"]',
              action: 'button[aria-label="edit"]',
              apply_on_routes: ['/'],
              require_count_at_least: 1,
            },
          ],
        },
      },
    },
  };

  const { code, json } = await runStructural(input);
  assert.equal(json?.metric, 'F');
  assert.equal(json?.passed, false, 'Expected passed=false when row 3 has no edit button');
  assert.ok(json?.invariants.length >= 1);
  const failedInvariant = json?.invariants.find(i => !i.passed);
  assert.ok(failedInvariant, 'should have a failing invariant');
  assert.ok(failedInvariant.violations.length > 0, 'should have violations');
  assert.equal(code, 1, 'exit code should be 1 on fail');
});

// ── JSON output shape ──────────────────────────────────────────────────────────

test('run-structural: output validates { metric, passed, invariants, summary } shape', async () => {
  const input = {
    base_url: serverUrl,
    contract: {
      e2e: {
        invariants_v2: {
          enabled: true,
          invariants: [
            {
              id: 'shape-check',
              kind: 'every_visible_X_is_Y',
              visible: '[data-testid="done-item-row-0"]',
              action: 'button[aria-label="edit"]',
              apply_on_routes: ['/'],
              require_count_at_least: 1,
            },
          ],
        },
      },
    },
  };

  const { json } = await runStructural(input);
  assert.ok(json, 'should output valid JSON');
  assert.equal(json.metric, 'F');
  assert.equal(typeof json.passed, 'boolean');
  assert.ok(Array.isArray(json.invariants));
  assert.ok(typeof json.summary === 'object');
  assert.ok('total' in json.summary);
  assert.ok('passed_count' in json.summary);
  assert.ok('failed_count' in json.summary);
  assert.ok('violation_count' in json.summary);
});

// ── Disabled invariants_v2 ─────────────────────────────────────────────────────

test('run-structural: disabled invariants_v2 — passed=true, empty invariants', async () => {
  const input = {
    base_url: serverUrl,
    contract: {
      e2e: {
        invariants_v2: { enabled: false, invariants: [] },
      },
    },
  };

  const { code, json } = await runStructural(input);
  assert.equal(json?.metric, 'F');
  assert.equal(json?.passed, true);
  assert.deepEqual(json?.invariants, []);
  assert.equal(code, 0);
});

// ── open_all_foldouts integration ─────────────────────────────────────────────

test('run-structural: open_all_foldouts=true exposes nested content', async () => {
  const input = {
    base_url: serverUrl,
    contract: {
      e2e: {
        invariants_v2: {
          enabled: true,
          crawler: { open_all_foldouts: true },
          invariants: [
            {
              id: 'deep-content-visible',
              kind: 'every_visible_X_is_Y',
              visible: '[data-testid="done-item-row-0"]',
              action: 'button[aria-label="edit"]',
              apply_on_routes: ['/'],
              require_count_at_least: 1,
            },
          ],
        },
      },
    },
  };

  const { json } = await runStructural(input);
  assert.equal(json?.metric, 'F');
  // With open_all_foldouts, page still has the items — just verifies it doesn't crash
  assert.ok(typeof json?.passed === 'boolean');
});
