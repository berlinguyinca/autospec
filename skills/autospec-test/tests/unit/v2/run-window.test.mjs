/**
 * run-window.test.mjs — Unit tests for Metric G window-contract runner.
 *
 * Uses real Playwright against a tiny inline HTTP server serving static HTML.
 * No mocks — requests are intercepted via page.route() inside the recorder.
 *
 * Run: node --test skills/autospec-test/tests/unit/v2/run-window.test.mjs
 */

import { describe, it, before, after } from 'node:test';
import assert from 'node:assert/strict';
import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium } from '/opt/homebrew/lib/node_modules/playwright/index.mjs';
import { attachRecorder } from '../../../scripts/window-contract/request-recorder.mjs';
import { resolve as resolveDateExpr } from '../../../scripts/window-contract/date-math.mjs';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const FIXTURE_DIR = path.join(__dirname, '../../fixtures/v2/window-contract');

// ── Inline test HTTP server ────────────────────────────────────────────────────

let server;
let baseUrl;

/**
 * Serve the mismatch fixture HTML plus a fake API endpoint.
 * The fixture HTML fetches /api/household/timeline?from=...&to=...
 * The server responds with 200 JSON to let the browser complete the fetch.
 */
before(async () => {
  server = http.createServer((req, res) => {
    const url = new URL(req.url, 'http://localhost');

    if (url.pathname === '/api/household/timeline') {
      res.writeHead(200, { 'Content-Type': 'application/json' });
      res.end(JSON.stringify({ events: [] }));
      return;
    }

    if (url.pathname === '/' || url.pathname === '/index.html') {
      const html = fs.readFileSync(path.join(FIXTURE_DIR, 'index.html'), 'utf8');
      res.writeHead(200, { 'Content-Type': 'text/html' });
      res.end(html);
      return;
    }

    res.writeHead(404);
    res.end('not found');
  });

  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  baseUrl = `http://127.0.0.1:${server.address().port}`;
});

after(async () => {
  await new Promise(resolve => server.close(resolve));
});

// ── Helpers ────────────────────────────────────────────────────────────────────

/**
 * Build a minimal contract for a single window-contract entry.
 */
function makeContract(overrides = {}) {
  return {
    e2e: {
      invariants_v2: {
        enabled: true,
        window_contracts: [
          {
            id: 'dashboard-streak-window',
            ui_display: {
              route: '/',
              widget: '[data-testid="streak-widget"]',
              window_days_attr: 'data-window-days',
            },
            api_query: {
              method: 'GET',
              path_pattern: '^/api/household/timeline$',
              window_params: {
                from: {
                  type: 'iso_date',
                  must_be: 'today - $N days',
                  tolerance_days: overrides.from_tolerance ?? 1,
                },
                to: {
                  type: 'iso_date',
                  must_be: 'today',
                  tolerance_days: overrides.to_tolerance ?? 1,
                },
              },
            },
            mismatch_action: 'hard_fail',
            ...overrides.contract_overrides,
          },
        ],
      },
    },
  };
}

/**
 * Run the core window-contract check logic against a live Playwright page.
 * Mirrors what run-window.mjs does but in-process so we can assert results.
 */
async function runWindowCheck({ page, contract, todayISO, requestWaitMs = 5000 }) {
  const wc = contract.e2e.invariants_v2.window_contracts[0];
  const { ui_display, api_query } = wc;
  const toleranceDays = wc.tolerance_days ?? 1;
  const today = new Date(todayISO + 'T00:00:00Z');
  const dateCtx = { today };

  const recorder = attachRecorder(page, api_query.path_pattern);
  await page.goto(baseUrl + ui_display.route, { waitUntil: 'domcontentloaded' });
  await page.locator(ui_display.widget).waitFor({ state: 'visible', timeout: 10_000 });

  const raw = await page.locator(ui_display.widget).getAttribute(ui_display.window_days_attr);
  const N = parseInt(raw, 10);

  // Wait for requests (short poll loop)
  const deadline = Date.now() + requestWaitMs;
  while (Date.now() < deadline && recorder.requests.length === 0) {
    await new Promise(r => setTimeout(r, 50));
  }

  assert.ok(recorder.requests.length > 0, 'Expected at least one recorded request');

  const firstReq = recorder.requests[0];
  const violations = [];

  for (const [paramName, paramSpec] of Object.entries(api_query.window_params)) {
    const perParamTolerance = paramSpec.tolerance_days ?? toleranceDays;
    const exprWithN = paramSpec.must_be.replace('$N', String(N));
    const expected = resolveDateExpr(exprWithN, dateCtx);
    const observed = firstReq.params[paramName];

    if (!observed) {
      violations.push({ param: paramName, expected, observed: null, reason: 'missing' });
      continue;
    }

    const msA = new Date(expected + 'T00:00:00Z').getTime();
    const msB = new Date(observed + 'T00:00:00Z').getTime();
    const diff = Math.abs(Math.round((msA - msB) / 86_400_000));
    if (diff > perParamTolerance) {
      violations.push({ param: paramName, expected, observed, diff_days: diff });
    }
  }

  return { N, violations, requests: recorder.requests };
}

// ── Tests ──────────────────────────────────────────────────────────────────────

describe('Metric G — window-contract runner', () => {
  let browser;
  let page;

  before(async () => {
    browser = await chromium.launch({ headless: true });
  });
  after(async () => {
    await browser.close();
  });

  // Fresh page per test to isolate recorders
  async function freshPage() {
    if (page) await page.close().catch(() => {});
    page = await browser.newPage();
    return page;
  }

  // ── Mismatch case ────────────────────────────────────────────────────────

  it('detects mismatch: UI=7d widget but API fetches 3d window', async () => {
    const p = await freshPage();
    // Today is the date the fixture uses as "today" when computing "today - 3d"
    // We pin today so the test is deterministic regardless of real clock
    const realToday = new Date();
    const pad = n => String(n).padStart(2, '0');
    const todayISO = `${realToday.getUTCFullYear()}-${pad(realToday.getUTCMonth()+1)}-${pad(realToday.getUTCDate())}`;

    const contract = makeContract({ from_tolerance: 1 });
    const result = await runWindowCheck({ page: p, contract, todayISO });

    // Widget declares N=7; fixture fetches from=today-3d → diff=4 > tolerance=1 → violation
    assert.equal(result.N, 7);
    const fromViolation = result.violations.find(v => v.param === 'from');
    assert.ok(fromViolation, `Expected a 'from' violation; got: ${JSON.stringify(result.violations)}`);
    assert.ok(fromViolation.diff_days >= 3,
      `Expected diff_days >= 3, got ${fromViolation.diff_days}`);
    assert.ok(fromViolation.expected, 'violation should have expected field');
    assert.ok(fromViolation.observed, 'violation should have observed field');
  });

  // ── Tolerance boundary: +1 day passes ────────────────────────────────────

  it('tolerance_days=4: mismatch of 4 days passes (boundary)', async () => {
    const p = await freshPage();
    const realToday = new Date();
    const pad = n => String(n).padStart(2, '0');
    const todayISO = `${realToday.getUTCFullYear()}-${pad(realToday.getUTCMonth()+1)}-${pad(realToday.getUTCDate())}`;

    // With tolerance_days=4 the 4-day difference (today-3d vs today-7d) just passes
    const contract = makeContract({ from_tolerance: 4 });
    const result = await runWindowCheck({ page: p, contract, todayISO });

    assert.equal(result.N, 7);
    const fromViolation = result.violations.find(v => v.param === 'from');
    assert.equal(fromViolation, undefined, 'Should be no violation with tolerance=4');
  });

  // ── Tolerance boundary: tolerance=3 passes, tolerance=2 fails ────────────

  it('tolerance_days=3: mismatch of 4 days fails (just outside boundary)', async () => {
    const p = await freshPage();
    const realToday = new Date();
    const pad = n => String(n).padStart(2, '0');
    const todayISO = `${realToday.getUTCFullYear()}-${pad(realToday.getUTCMonth()+1)}-${pad(realToday.getUTCDate())}`;

    // tolerance=3 < diff=4 → fail
    const contract = makeContract({ from_tolerance: 3 });
    const result = await runWindowCheck({ page: p, contract, todayISO });

    const fromViolation = result.violations.find(v => v.param === 'from');
    assert.ok(fromViolation, 'Should have violation with tolerance=3 and diff=4');
  });

  // ── attachRecorder idempotency ────────────────────────────────────────────

  it('attachRecorder is idempotent — double attach returns same recorder', async () => {
    const p = await freshPage();
    const r1 = attachRecorder(p, '^/api/test$');
    const r2 = attachRecorder(p, '^/api/test$');
    assert.strictEqual(r1, r2, 'Same recorder object expected on double attach');
  });

  // ── attachRecorder: only matching paths recorded ──────────────────────────

  it('attachRecorder captures only matching path pattern', async () => {
    const p = await freshPage();
    const recorder = attachRecorder(p, '^/api/household/timeline$');

    // Navigate to the fixture — it fetches /api/household/timeline
    await p.goto(baseUrl + '/', { waitUntil: 'domcontentloaded' });

    const deadline = Date.now() + 5000;
    while (Date.now() < deadline && recorder.requests.length === 0) {
      await new Promise(r => setTimeout(r, 50));
    }

    assert.ok(recorder.requests.length >= 1, 'Expected recorded request for /api/household/timeline');
    for (const req of recorder.requests) {
      assert.match(req.path, /^\/api\/household\/timeline$/);
    }
  });

  // ── Gate JSON shape emitted by run-window.mjs ─────────────────────────────

  it('run-window.mjs emits correct gate JSON shape on stdout', async () => {
    // Smoke: call run-window.mjs as a subprocess, pipe in minimal contract
    const { execFileSync } = await import('node:child_process');

    const contract = {
      e2e: {
        invariants_v2: {
          enabled: false,
          window_contracts: [],
        },
      },
    };

    let stdout;
    try {
      stdout = execFileSync('node', [
        'skills/autospec-test/scripts/window-contract/run-window.mjs',
      ], {
        input: JSON.stringify({ contract, base_url: baseUrl }),
        cwd: path.resolve(__dirname, '../../../../..'),
        timeout: 15_000,
        encoding: 'utf8',
      });
    } catch (e) {
      throw new Error(`run-window.mjs subprocess failed: ${e.stderr || e.message}`);
    }

    const result = JSON.parse(stdout);
    assert.equal(result.metric, 'G');
    assert.equal(typeof result.passed, 'boolean');
    assert.ok(Array.isArray(result.contracts));
    assert.equal(typeof result.summary, 'object');
    assert.ok('total' in result.summary);
    assert.ok('passed_count' in result.summary);
    assert.ok('failed_count' in result.summary);
    assert.ok('violation_count' in result.summary);
  });
});
