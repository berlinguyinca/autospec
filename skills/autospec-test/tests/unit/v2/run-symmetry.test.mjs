/**
 * run-symmetry.test.mjs — Unit tests for Metric I data-source contract symmetry.
 *
 * Uses real Playwright + inline HTTP server. No mocks.
 * Server serves:
 *   GET /          → HTML with 3 streak-task rows (t-1, t-2, t-3; date=2026-05-14)
 *   GET /api/events → JSON with events for t-1 and t-2 only (t-3 missing)
 *   GET /api/events?task_id=t-1 → event with editable=true
 *   GET /api/events?task_id=t-2 → event with editable=false  (not-editable violation)
 *   GET /api/events?task_id=t-3 → empty events array (missing violation)
 *
 * Run: node --test skills/autospec-test/tests/unit/v2/run-symmetry.test.mjs
 */

import { describe, it, before, after } from 'node:test';
import assert from 'node:assert/strict';
import http from 'node:http';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright';
import { interpolate } from '../../../scripts/contract-symmetry/interpolator.mjs';
import { assertContains, assertBoolean } from '../../../scripts/contract-symmetry/jsonpath-verifier.mjs';
import { extract } from '../../../scripts/contract-symmetry/ui-extractor.mjs';
import { accessScopeViolation } from '../../../scripts/contract-symmetry/run-symmetry.mjs';

const __dirname = path.dirname(fileURLToPath(import.meta.url));

describe('access-scope contract symmetry', () => {
  it('flags an all-users route guarded by admin middleware', () => {
    const finding = accessScopeViolation('/admin/cluster-state', 'adminOnly', 'all users');
    assert.equal(finding.type, 'access-scope-mismatch');
    assert.equal(finding.route, '/admin/cluster-state');
    assert.equal(finding.guard, 'adminOnly');
    assert.equal(finding.declared_scope, 'all users');
  });

  it('accepts an admin-only route with an admin guard', () => {
    assert.equal(accessScopeViolation('/admin/settings', 'adminOnly', 'admin-only'), null);
  });

  it('flags undocumented admin paths for non-admin scopes', () => {
    const finding = accessScopeViolation('/admin/report', 'sessionGuard', 'all users');
    assert.equal(finding.type, 'access-scope-mismatch');
  });
});

// ── Inline fixture server ──────────────────────────────────────────────────────

const FIXTURE_HTML = `<!DOCTYPE html>
<html><body>
  <div data-testid="streak-task-1" data-task-id="t-1" data-date="2026-05-14">Task 1</div>
  <div data-testid="streak-task-2" data-task-id="t-2" data-date="2026-05-14">Task 2</div>
  <div data-testid="streak-task-3" data-task-id="t-3" data-date="2026-05-14">Task 3</div>
</body></html>`;

// API data: t-1 editable, t-2 not editable, t-3 missing
const API_DATA = {
  't-1': { events: [{ task_id: 't-1', date: '2026-05-14', editable: true }] },
  't-2': { events: [{ task_id: 't-2', date: '2026-05-14', editable: false }] },
  't-3': { events: [] },
};

let server;
let baseUrl;

before(async () => {
  server = http.createServer((req, res) => {
    const url = new URL(req.url, 'http://localhost');

    if (url.pathname === '/') {
      res.writeHead(200, { 'Content-Type': 'text/html' });
      res.end(FIXTURE_HTML);
      return;
    }

    if (url.pathname === '/api/events') {
      const taskId = url.searchParams.get('task_id');
      const data = taskId ? (API_DATA[taskId] || { events: [] }) : { events: [] };
      res.writeHead(200, { 'Content-Type': 'application/json' });
      res.end(JSON.stringify(data));
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

// ── interpolator tests ─────────────────────────────────────────────────────────

describe('interpolator', () => {
  it('substitutes ${key} placeholders', () => {
    assert.equal(
      interpolate('/api/timeline?from=${date}&to=${date}', { date: '2026-05-14' }),
      '/api/timeline?from=2026-05-14&to=2026-05-14',
    );
  });

  it('substitutes ${url:key} with URL-encoding', () => {
    assert.equal(
      interpolate('/api?q=${url:q}', { q: 'a b' }),
      '/api?q=a%20b',
    );
  });

  it('handles multiple different keys', () => {
    assert.equal(
      interpolate('/api?task=${task_id}&date=${date}', { task_id: 't-1', date: '2026-05-14' }),
      '/api?task=t-1&date=2026-05-14',
    );
  });

  it('throws when key is undefined — error includes key name', () => {
    assert.throws(
      () => interpolate('/api?x=${missing_key}', { other: 'value' }),
      (err) => {
        assert.match(err.message, /missing_key/);
        return true;
      },
    );
  });

  it('plain ${date} substitution without URL encoding', () => {
    assert.equal(interpolate('${date}', { date: '2026-05-14' }), '2026-05-14');
  });
});

// ── assertContains / assertBoolean tests ───────────────────────────────────────

describe('jsonpath-verifier', () => {
  const body = { events: [{ task_id: 't-1', editable: true }, { task_id: 't-2', editable: false }] };

  it('assertContains: matches existing task_id', () => {
    assert.doesNotThrow(() => assertContains(body, '$.events[?(@.task_id=="t-1")]'));
  });

  it('assertContains: throws on no match — error includes api_response_summary truncated to 500 bytes', () => {
    assert.throws(
      () => assertContains(body, '$.events[?(@.task_id=="t-999")]'),
      (err) => {
        assert.match(err.message, /assertContains/);
        assert.match(err.message, /api_response_summary/);
        return true;
      },
    );
  });

  it('assertBoolean: passes when JSONPath resolves to true', () => {
    assert.doesNotThrow(() => assertBoolean(body, '$.events[?(@.task_id=="t-1")].editable'));
  });

  it('assertBoolean: throws when value is false', () => {
    assert.throws(
      () => assertBoolean(body, '$.events[?(@.task_id=="t-2")].editable'),
      /assertBoolean/,
    );
  });

  it('assertBoolean: throws on no match', () => {
    assert.throws(
      () => assertBoolean(body, '$.events[?(@.task_id=="t-999")].editable'),
      /assertBoolean/,
    );
  });

  it('assertContains: error includes api_response_summary truncated at 500 bytes', () => {
    // Build a large body to trigger truncation
    const largeBody = { events: Array.from({ length: 100 }, (_, i) => ({ task_id: `t-${i}`, editable: true })) };
    assert.throws(
      () => assertContains(largeBody, '$.events[?(@.task_id=="t-missing")]'),
      (err) => {
        // Summary should be present and ≤ ~510 chars (500 + ellipsis)
        const summaryMatch = err.message.match(/api_response_summary: (.+)/s);
        assert.ok(summaryMatch, 'error should include api_response_summary');
        assert.ok(summaryMatch[1].length <= 510, 'summary should be truncated');
        return true;
      },
    );
  });
});

// ── ui-extractor tests ──────────────────────────────────────────────────────────

describe('ui-extractor', () => {
  let browser;
  let page;

  before(async () => {
    browser = await chromium.launch({ headless: true });
    page = await browser.newPage();
  });
  after(async () => { await browser.close(); });

  it('extracts 3 tuples from fixture HTML', async () => {
    const ui_source = {
      route: '/',
      extract: '[data-testid^="streak-task-"]',
      per_match: { task_id: 'data-task-id', date: 'data-date' },
    };
    const tuples = await extract(page, baseUrl + '/', ui_source);
    assert.equal(tuples.length, 3);
    assert.deepEqual(tuples[0], { task_id: 't-1', date: '2026-05-14' });
    assert.deepEqual(tuples[1], { task_id: 't-2', date: '2026-05-14' });
    assert.deepEqual(tuples[2], { task_id: 't-3', date: '2026-05-14' });
  });
});

// ── run-symmetry in-process tests ──────────────────────────────────────────────

describe('Metric I — run-symmetry (in-process)', () => {
  let browser;
  let page;

  before(async () => {
    browser = await chromium.launch({ headless: true });
    page = await browser.newPage();
  });
  after(async () => { await browser.close(); });

  const UI_SOURCE = {
    route: '/',
    extract: '[data-testid^="streak-task-"]',
    per_match: { task_id: 'data-task-id', date: 'data-date' },
  };

  const API_TARGET = {
    method: 'GET',
    path_template: '/api/events?task_id=${task_id}&date=${date}',
    must_contain: '$.events[?(@.task_id=="${task_id}")]',
    must_be_editable: '$.events[?(@.task_id=="${task_id}")].editable',
  };

  async function runCheck(contractOverrides = {}) {
    const contract = {
      e2e: {
        invariants_v2: {
          enabled: true,
          contract_symmetry: [{
            id: 'streak-task-must-be-editable',
            ui_source: UI_SOURCE,
            api_target: { ...API_TARGET, ...contractOverrides },
            mismatch_action: 'hard_fail',
          }],
        },
      },
    };

    const p = await browser.newPage();
    const symmetryContracts = contract.e2e.invariants_v2.contract_symmetry;
    const allViolations = [];
    const contractResults = [];

    for (const cs of symmetryContracts) {
      const violations = [];
      const route = baseUrl.replace(/\/$/, '') + cs.ui_source.route;
      const tuples = await extract(p, route, cs.ui_source);

      for (const tuple of tuples) {
        const path = interpolate(cs.api_target.path_template, tuple);
        const apiUrl = baseUrl.replace(/\/$/, '') + path;
        const response = await p.request.get(apiUrl);
        const body = await response.json();

        if (cs.api_target.must_contain) {
          try {
            const expr = interpolate(cs.api_target.must_contain, tuple);
            assertContains(body, expr, tuple);
          } catch (e) {
            violations.push({ contract_id: cs.id, tuple, phase: 'must_contain', reason: e.message });
          }
        }
        if (cs.api_target.must_be_editable) {
          try {
            const expr = interpolate(cs.api_target.must_be_editable, tuple);
            assertBoolean(body, expr, tuple);
          } catch (e) {
            violations.push({ contract_id: cs.id, tuple, phase: 'must_be_editable', reason: e.message });
          }
        }
      }

      contractResults.push({ id: cs.id, passed: violations.length === 0, tuples_checked: tuples.length, violations });
      allViolations.push(...violations);
    }
    await p.close();
    return { passed: allViolations.length === 0, contracts: contractResults, violations: allViolations };
  }

  it('happy path: t-1 event exists and is editable → 0 violations', async () => {
    // Override to check only t-1 by using a selector that matches only first element
    const contractSingle = {
      e2e: {
        invariants_v2: {
          enabled: true,
          contract_symmetry: [{
            id: 'single-task',
            ui_source: {
              route: '/',
              extract: '[data-testid="streak-task-1"]',
              per_match: { task_id: 'data-task-id', date: 'data-date' },
            },
            api_target: API_TARGET,
            mismatch_action: 'hard_fail',
          }],
        },
      },
    };

    const p = await browser.newPage();
    const cs = contractSingle.e2e.invariants_v2.contract_symmetry[0];
    const route = baseUrl + cs.ui_source.route;
    const tuples = await extract(p, route, cs.ui_source);
    const violations = [];

    for (const tuple of tuples) {
      const path = interpolate(cs.api_target.path_template, tuple);
      const apiUrl = baseUrl + path;
      const response = await p.request.get(apiUrl);
      const body = await response.json();
      try { assertContains(body, interpolate(cs.api_target.must_contain, tuple), tuple); } catch(e) { violations.push(e.message); }
      try { assertBoolean(body, interpolate(cs.api_target.must_be_editable, tuple), tuple); } catch(e) { violations.push(e.message); }
    }
    await p.close();

    assert.equal(violations.length, 0, `Expected 0 violations, got: ${JSON.stringify(violations)}`);
  });

  it('missing event (t-3): 1 violation with tuple.task_id field present', async () => {
    const result = await runCheck();
    const t3Violations = result.violations.filter(v => v.tuple?.task_id === 't-3');
    assert.ok(t3Violations.length >= 1, `Expected violation for t-3, got: ${JSON.stringify(result.violations)}`);
    assert.equal(t3Violations[0].tuple.task_id, 't-3', 'violation should carry tuple.task_id');
  });

  it('not-editable (t-2): 1 must_be_editable violation', async () => {
    const result = await runCheck();
    const t2Violations = result.violations.filter(v => v.tuple?.task_id === 't-2' && v.phase === 'must_be_editable');
    assert.ok(t2Violations.length >= 1, `Expected must_be_editable violation for t-2, got: ${JSON.stringify(result.violations)}`);
  });

  it('missing + not-editable: at least 2 violations total', async () => {
    const result = await runCheck();
    // t-3 missing (must_contain fails) + t-2 not editable (must_be_editable fails)
    assert.ok(result.violations.length >= 2,
      `Expected >=2 violations, got: ${JSON.stringify(result.violations)}`);
  });

  it('gate JSON shape: metric=I, passed, contracts[], violations[]', async () => {
    const { execFileSync } = await import('node:child_process');
    const contract = {
      e2e: { invariants_v2: { enabled: false, contract_symmetry: [] } },
    };
    let stdout;
    try {
      stdout = execFileSync('node', ['skills/autospec-test/scripts/contract-symmetry/run-symmetry.mjs'], {
        input: JSON.stringify({ contract, base_url: baseUrl }),
        cwd: path.resolve(__dirname, '../../../../..'),
        timeout: 15_000, encoding: 'utf8',
      });
    } catch (e) {
      throw new Error(`run-symmetry.mjs subprocess failed: ${e.stderr || e.message}`);
    }
    const result = JSON.parse(stdout);
    assert.equal(result.metric, 'I');
    assert.equal(typeof result.passed, 'boolean');
    assert.ok(Array.isArray(result.contracts));
    assert.ok(Array.isArray(result.violations));
    assert.ok('violation_count' in result.summary);
  });

  // Regression pin: a contract whose ui_source.extract selector matches
  // zero elements must NEVER report passed:true. Before this fix,
  // `contractPassed = violations.length === 0` was trivially true for a
  // tuples.length === 0 result — a check that examined nothing silently
  // reported success, the same fail-open shape this codebase has shipped
  // repeatedly (a metric skipped-but-marked-passed, a `// true` jq
  // coercion of a real failure, an app harness reporting a process
  // "started" that never started). This exercises the REAL
  // run-symmetry.mjs subprocess (not the in-process reimplementation
  // above) against a live fixture server, with a selector guaranteed not
  // to match anything on the page.
  it('zero tuples extracted: contract reports passed:false, never a vacuous pass', async () => {
    const { execFileSync } = await import('node:child_process');
    const contract = {
      e2e: {
        invariants_v2: {
          enabled: true,
          contract_symmetry: [{
            id: 'nothing-matches',
            ui_source: {
              route: '/',
              extract: '[data-testid^="does-not-exist-on-this-page-"]',
              per_match: { task_id: 'data-task-id' },
            },
            api_target: { method: 'GET', path_template: '/api/${task_id}', must_contain: '$.x' },
            mismatch_action: 'hard_fail',
          }],
        },
      },
    };
    let stdout;
    try {
      stdout = execFileSync('node', ['skills/autospec-test/scripts/contract-symmetry/run-symmetry.mjs'], {
        input: JSON.stringify({ contract, base_url: baseUrl }),
        cwd: path.resolve(__dirname, '../../../../..'),
        timeout: 15_000, encoding: 'utf8',
      });
      assert.fail('expected run-symmetry.mjs to exit non-zero (a failing verdict), but it exited 0');
    } catch (e) {
      // execFileSync throws on non-zero exit; the real verdict JSON is on
      // its stdout (e.stdout), same as a normal failing run.
      stdout = e.stdout;
    }
    const result = JSON.parse(stdout);
    assert.equal(result.passed, false, `zero-tuple contract must not report passed:true; got: ${stdout}`);
    assert.equal(result.contracts[0].tuples_checked, 0);
    assert.equal(result.contracts[0].passed, false);
    assert.ok(
      result.contracts[0].violations.some(v => v.phase === 'ui_extract' && /matched 0 elements/.test(v.reason)),
      `expected a ui_extract violation explaining zero elements matched; got: ${JSON.stringify(result.contracts[0].violations)}`,
    );
  });
});
