/**
 * run-symmetry.mjs — Metric I: Data-source Contract Symmetry Runner
 *
 * Reads a contract + base_url from stdin JSON and for each declared
 * contract_symmetry entry:
 *   1. Extracts (task_id, date) tuples from the UI via ui-extractor
 *   2. For each tuple, interpolates the api_target.path_template
 *   3. Fetches the API endpoint via page.request
 *   4. Asserts must_contain (JSONPath) and must_be_editable (JSONPath boolean)
 *   5. Emits gate JSON
 *
 * Input (stdin JSON):
 *   {
 *     contract: { e2e: { invariants_v2: { contract_symmetry: [...] } } },
 *     base_url: string
 *   }
 *
 * Output (stdout JSON):
 *   {
 *     metric: "I",
 *     passed: boolean,
 *     contracts: [{ id, passed, tuples_checked, violations[] }],
 *     violations: [...all violations across contracts...],
 *     summary: { total, passed_count, failed_count, violation_count }
 *   }
 *
 * Exit codes: 0 = pass, 1 = fail (violations), 2 = fatal error
 */

import { chromium } from '/opt/homebrew/lib/node_modules/playwright/index.mjs';
import { extract } from './ui-extractor.mjs';
import { interpolate } from './interpolator.mjs';
import { assertContains, assertBoolean } from './jsonpath-verifier.mjs';

export function accessScopeViolation(route, guard, declaredScope, documented = '') {
  const scope = String(declaredScope || '').toLowerCase();
  const guardText = String(guard || '').toLowerCase();
  if (!scope || !guardText) return null;
  const adminOnly = /admin[-_ ]?(only|guard|middleware)|requireadmin|isadmin/.test(guardText);
  const adminScope = /admin[-_ ]?only|administrators?/.test(scope);
  if (adminOnly && !adminScope && !/acceptable|documented|intentional/i.test(documented)) {
    return { type: 'access-scope-mismatch', route, guard, declared_scope: declaredScope };
  }
  if (/^\/admin(?:\/|$)/i.test(route) && !adminScope && !/acceptable|documented|intentional/i.test(documented)) {
    return { type: 'access-scope-mismatch', route, guard, declared_scope: declaredScope };
  }
  return null;
}

async function run() {
  let input;
  try {
    const chunks = [];
    for await (const chunk of process.stdin) chunks.push(chunk);
    input = JSON.parse(Buffer.concat(chunks).toString('utf8'));
  } catch (e) {
    process.stderr.write(`[run-symmetry] fatal: failed to parse stdin JSON: ${e.message}\n`);
    process.exit(2);
  }

  const { contract, base_url: baseUrl } = input;
  if (!contract || !baseUrl) {
    process.stderr.write('[run-symmetry] fatal: stdin must have { contract, base_url }\n');
    process.exit(2);
  }

  const invariantsV2 = contract?.e2e?.invariants_v2;
  if (!invariantsV2?.enabled) {
    const result = {
      metric: 'I',
      passed: true,
      contracts: [],
      violations: [],
      summary: { total: 0, passed_count: 0, failed_count: 0, violation_count: 0 },
    };
    process.stdout.write(JSON.stringify(result, null, 2) + '\n');
    process.exit(0);
  }

  const symmetryContracts = invariantsV2.contract_symmetry || [];

  const browser = await chromium.launch({ headless: true });
  const page = await browser.newPage();
  const contractResults = [];
  const allViolations = [];

  try {
    for (const cs of symmetryContracts) {
      const { id, ui_source, api_target } = cs;
      const violations = [];

      // Step 1: extract tuples from UI
      let tuples;
      try {
        const route = baseUrl.replace(/\/$/, '') + ui_source.route;
        tuples = await extract(page, route, ui_source);
      } catch (e) {
        violations.push({ contract_id: id, phase: 'ui_extract', reason: e.message });
        contractResults.push({ id, passed: false, tuples_checked: 0, violations });
        allViolations.push(...violations);
        continue;
      }

      const declaredScope = cs.access_scope || cs.spec_access_scope || cs.user_access_scope;
      for (const tuple of tuples) {
        const mismatch = accessScopeViolation(ui_source.route, tuple.guard, declaredScope, cs.access_scope_note || cs.scope_documentation);
        if (mismatch) violations.push({ contract_id: id, phase: 'access_scope', ...mismatch });
      }

      // Step 2-4: for each tuple, interpolate URL, fetch, assert
      for (const tuple of tuples) {
        let apiUrl;
        try {
          const path = interpolate(api_target.path_template, tuple);
          apiUrl = baseUrl.replace(/\/$/, '') + path;
        } catch (e) {
          violations.push({ contract_id: id, tuple, phase: 'interpolate', reason: e.message });
          continue;
        }

        let body;
        try {
          const method = (api_target.method || 'GET').toLowerCase();
          const response = await page.request[method](apiUrl, { timeout: 10_000 });
          body = await response.json();
        } catch (e) {
          violations.push({ contract_id: id, tuple, api_url: apiUrl, phase: 'api_fetch', reason: e.message });
          continue;
        }

        // must_contain assertion
        if (api_target.must_contain) {
          try {
            const pathExpr = interpolate(api_target.must_contain, tuple);
            assertContains(body, pathExpr, tuple);
          } catch (e) {
            violations.push({
              contract_id: id,
              tuple,
              api_url: apiUrl,
              phase: 'must_contain',
              reason: e.message,
            });
          }
        }

        // must_be_editable assertion
        if (api_target.must_be_editable) {
          try {
            const pathExpr = interpolate(api_target.must_be_editable, tuple);
            assertBoolean(body, pathExpr, tuple);
          } catch (e) {
            violations.push({
              contract_id: id,
              tuple,
              api_url: apiUrl,
              phase: 'must_be_editable',
              reason: e.message,
            });
          }
        }
      }

      const contractPassed = violations.length === 0;
      contractResults.push({
        id,
        passed: contractPassed,
        tuples_checked: tuples.length,
        violations,
      });
      allViolations.push(...violations);
    }
  } finally {
    await browser.close();
  }

  const passed_count = contractResults.filter(r => r.passed).length;
  const failed_count = contractResults.filter(r => !r.passed).length;

  const output = {
    metric: 'I',
    passed: allViolations.length === 0,
    contracts: contractResults,
    violations: allViolations,
    summary: {
      total: contractResults.length,
      passed_count,
      failed_count,
      violation_count: allViolations.length,
    },
  };

  process.stdout.write(JSON.stringify(output, null, 2) + '\n');
  process.exit(output.passed ? 0 : 1);
}

run().catch(e => {
  process.stderr.write(`[run-symmetry] fatal: ${e.message}\n${e.stack}\n`);
  process.exit(2);
});
