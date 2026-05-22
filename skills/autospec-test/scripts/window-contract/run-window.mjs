/**
 * run-window.mjs — Metric G: Window-Contract Symmetry Runner
 *
 * Reads a contract + base_url from stdin JSON, for each declared window_contract:
 *   1. Attaches a request recorder for the declared api_query.path_pattern
 *   2. Navigates to ui_display.route
 *   3. Reads N from ui_display.window_days_attr on the widget element
 *   4. Waits up to 30s for ≥1 matching recorded request
 *   5. For each declared window_param, resolves the expected date and compares
 *      to the observed query-param value; a difference > tolerance_days is a violation
 *   6. Emits gate JSON on stdout
 *
 * Input (stdin JSON):
 *   {
 *     contract: { e2e: { invariants_v2: { window_contracts: [...] } } },
 *     base_url: string,
 *     today?: string,   // optional ISO override for deterministic tests (YYYY-MM-DD)
 *   }
 *
 * Output (stdout JSON):
 *   {
 *     metric: "G",
 *     passed: boolean,
 *     contracts: [{ id, passed, N, violations[], requests_seen }],
 *     summary: { total, passed_count, failed_count, violation_count }
 *   }
 *
 * Exit codes: 0 = pass, 1 = fail (violations found), 2 = fatal/config error
 */

import { chromium } from '/opt/homebrew/lib/node_modules/playwright/index.mjs';
import { resolve as resolveDateExpr } from './date-math.mjs';
import { attachRecorder } from './request-recorder.mjs';

const REQUEST_WAIT_MS = parseInt(process.env.AUTOSPEC_WINDOW_REQUEST_WAIT_MS ?? '30000', 10);
const POLL_INTERVAL_MS = 100;

// ── Helpers ───────────────────────────────────────────────────────────────────

/**
 * Wait until recorder has ≥1 request, or timeout.
 * @param {{ requests: object[] }} recorder
 * @param {number} timeoutMs
 * @returns {Promise<boolean>}
 */
async function waitForRequest(recorder, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (recorder.requests.length > 0) return true;
    await new Promise(r => setTimeout(r, POLL_INTERVAL_MS));
  }
  return false;
}

/**
 * Compute the absolute difference in calendar days between two ISO date strings.
 * @param {string} a YYYY-MM-DD
 * @param {string} b YYYY-MM-DD
 * @returns {number}
 */
function diffDays(a, b) {
  const msA = new Date(a + 'T00:00:00Z').getTime();
  const msB = new Date(b + 'T00:00:00Z').getTime();
  return Math.abs(Math.round((msA - msB) / 86_400_000));
}

// ── Main ──────────────────────────────────────────────────────────────────────

async function run() {
  // Read stdin
  let input;
  try {
    const chunks = [];
    for await (const chunk of process.stdin) chunks.push(chunk);
    input = JSON.parse(Buffer.concat(chunks).toString('utf8'));
  } catch (e) {
    process.stderr.write(`[run-window] fatal: failed to parse stdin JSON: ${e.message}\n`);
    process.exit(2);
  }

  const { contract, base_url: baseUrl, today: todayOverride } = input;

  if (!contract || !baseUrl) {
    process.stderr.write('[run-window] fatal: stdin must have { contract, base_url }\n');
    process.exit(2);
  }

  const invariantsV2 = contract?.e2e?.invariants_v2;
  if (!invariantsV2?.enabled) {
    const result = {
      metric: 'G',
      passed: true,
      contracts: [],
      summary: { total: 0, passed_count: 0, failed_count: 0, violation_count: 0 },
    };
    process.stdout.write(JSON.stringify(result, null, 2) + '\n');
    process.exit(0);
  }

  const windowContracts = invariantsV2.window_contracts || [];

  // Build today Date for date-math
  const today = todayOverride
    ? new Date(todayOverride + 'T00:00:00Z')
    : new Date();
  const dateCtx = { today };

  const browser = await chromium.launch({ headless: true });
  const page = await browser.newPage();

  const contractResults = [];

  try {
    for (const wc of windowContracts) {
      const { id, ui_display, api_query } = wc;
      const toleranceDays = wc.tolerance_days ?? 1; // contract-level default
      const pathPattern = api_query.path_pattern;
      const recorder = attachRecorder(page, pathPattern);

      // Navigate
      const url = baseUrl.replace(/\/$/, '') + ui_display.route;
      try {
        await page.goto(url, { waitUntil: 'domcontentloaded' });
      } catch (e) {
        contractResults.push({
          id,
          passed: false,
          N: null,
          violations: [{ param: '_navigation', reason: `goto failed: ${e.message}` }],
          requests_seen: 0,
        });
        continue;
      }

      // Wait for widget
      try {
        await page.locator(ui_display.widget).waitFor({ state: 'visible', timeout: 10_000 });
      } catch {
        contractResults.push({
          id,
          passed: false,
          N: null,
          violations: [{ param: '_widget', reason: `widget "${ui_display.widget}" not visible within 10s` }],
          requests_seen: 0,
        });
        continue;
      }

      // Read N from DOM attribute
      let N;
      try {
        const raw = await page.locator(ui_display.widget).getAttribute(ui_display.window_days_attr);
        N = parseInt(raw, 10);
        if (isNaN(N)) throw new Error(`attribute "${ui_display.window_days_attr}" is not a number: ${raw}`);
      } catch (e) {
        contractResults.push({
          id,
          passed: false,
          N: null,
          violations: [{ param: '_window_days_attr', reason: e.message }],
          requests_seen: 0,
        });
        continue;
      }

      // Wait for at least one recorded request
      const got = await waitForRequest(recorder, REQUEST_WAIT_MS);
      if (!got) {
        contractResults.push({
          id,
          passed: false,
          N,
          violations: [{ param: '_requests', reason: `no request matching "${pathPattern}" captured within ${REQUEST_WAIT_MS}ms` }],
          requests_seen: 0,
        });
        continue;
      }

      // Take the first matching request for param comparison
      const firstReq = recorder.requests[0];
      const violations = [];

      const windowParams = api_query.window_params || {};
      for (const [paramName, paramSpec] of Object.entries(windowParams)) {
        const perParamTolerance = paramSpec.tolerance_days ?? toleranceDays;

        // Resolve expected value: substitute $N in the must_be expression
        const exprWithN = paramSpec.must_be.replace('$N', String(N));
        let expected;
        try {
          expected = resolveDateExpr(exprWithN, dateCtx);
        } catch (e) {
          violations.push({
            param: paramName,
            reason: `could not resolve expected expression "${exprWithN}": ${e.message}`,
          });
          continue;
        }

        // Observed value from the captured request
        const observed = firstReq.params[paramName];
        if (!observed) {
          violations.push({
            param: paramName,
            expected,
            observed: null,
            reason: `param "${paramName}" missing from captured request`,
          });
          continue;
        }

        // Validate observed is a valid ISO date
        if (!/^\d{4}-\d{2}-\d{2}$/.test(observed)) {
          violations.push({
            param: paramName,
            expected,
            observed,
            reason: `param "${paramName}" is not an ISO date: ${observed}`,
          });
          continue;
        }

        const diff = diffDays(expected, observed);
        if (diff > perParamTolerance) {
          violations.push({
            param: paramName,
            expected,
            observed,
            diff_days: diff,
            tolerance_days: perParamTolerance,
            reason: `expected ${paramName}=${expected}, got ${observed} (diff=${diff}d > tolerance=${perParamTolerance}d)`,
          });
        }
      }

      contractResults.push({
        id,
        passed: violations.length === 0,
        N,
        violations,
        requests_seen: recorder.requests.length,
      });
    }
  } finally {
    await browser.close();
  }

  const passed_count = contractResults.filter(r => r.passed).length;
  const failed_count = contractResults.filter(r => !r.passed).length;
  const violation_count = contractResults.reduce((s, r) => s + r.violations.length, 0);

  const output = {
    metric: 'G',
    passed: failed_count === 0,
    contracts: contractResults,
    summary: {
      total: contractResults.length,
      passed_count,
      failed_count,
      violation_count,
    },
  };

  process.stdout.write(JSON.stringify(output, null, 2) + '\n');
  process.exit(output.passed ? 0 : 1);
}

run().catch((e) => {
  process.stderr.write(`[run-window] fatal: ${e.message}\n${e.stack}\n`);
  process.exit(2);
});
