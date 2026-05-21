#!/usr/bin/env node
// forbidden-url-check.mjs — Layer A forbidden-URL preflight check.
//
// Usage: node forbidden-url-check.mjs <config_json_file> <contract_json_file>
//   OR:  echo '<config_json>' | node forbidden-url-check.mjs - <contract_json_file>
//
// Checks every URL-shaped value in the resolved Playwright config against
// forbidden_url_patterns from the contract. Any match → exit 2 (refuse-to-run).
//
// Output JSON (stdout):
//   {
//     "passed": true,
//     "violations": [],
//     "checked_urls": [{ "field": "baseURL", "value": "http://..." }]
//   }
//
// Exit codes:
//   0 = no violations
//   1 = fatal error
//   2 = violations found (refuse-to-run)

import { readFileSync, existsSync } from 'fs';

const [,, configArg, contractArg] = process.argv;

if (!configArg || !contractArg) {
  process.stderr.write('Usage: forbidden-url-check.mjs <config_json_or_-> <contract_json_file>\n');
  process.exit(1);
}

function readJSON(arg) {
  if (arg === '-') {
    return JSON.parse(readFileSync('/dev/stdin', 'utf8'));
  }
  if (!existsSync(arg)) {
    process.stderr.write(`forbidden-url-check: fatal: file not found: ${arg}\n`);
    process.exit(1);
  }
  return JSON.parse(readFileSync(arg, 'utf8'));
}

let config, contract;
try {
  config = readJSON(configArg);
} catch (e) {
  process.stderr.write(`forbidden-url-check: fatal: cannot parse config: ${e.message}\n`);
  process.exit(1);
}
try {
  contract = readJSON(contractArg);
} catch (e) {
  process.stderr.write(`forbidden-url-check: fatal: cannot parse contract: ${e.message}\n`);
  process.exit(1);
}

// ── Extract URL-shaped fields from Playwright config ─────────────────────────
// Per spec §5a: every URL-shaped value in effective config must be checked.
const URL_FIELDS = [
  { field: 'baseURL',      value: config.baseURL },
  { field: 'useBaseURL',   value: config.useBaseURL },
  { field: 'webServerURL', value: config.webServerURL },
];

// Also check E2E_BASE_URL, PLAYWRIGHT_BASE_URL, BASE_URL from env
const ENV_URL_FIELDS = [
  { field: 'E2E_BASE_URL',       value: process.env.E2E_BASE_URL },
  { field: 'PLAYWRIGHT_BASE_URL', value: process.env.PLAYWRIGHT_BASE_URL },
  { field: 'BASE_URL',           value: process.env.BASE_URL },
];

const checkedURLs = [
  ...URL_FIELDS.filter(f => f.value),
  ...ENV_URL_FIELDS.filter(f => f.value),
];

// ── Get forbidden patterns from contract ─────────────────────────────────────
const forbiddenPatterns = (contract?.e2e?.forbidden_url_patterns) || [];
const intentionallyEmpty = contract?.e2e?.forbidden_url_patterns_intentionally_empty === true;

// ── Check each URL against each pattern ──────────────────────────────────────
const violations = [];

for (const { field, value } of checkedURLs) {
  for (const pattern of forbiddenPatterns) {
    let regex;
    try {
      regex = new RegExp(pattern);
    } catch (e) {
      process.stderr.write(`forbidden-url-check: WARN: invalid regex pattern: ${pattern}\n`);
      continue;
    }
    if (regex.test(value)) {
      violations.push({ field, value, pattern });
    }
  }
}

const result = {
  passed: violations.length === 0,
  violations,
  checked_urls: checkedURLs,
  patterns_count: forbiddenPatterns.length,
  intentionally_empty: intentionallyEmpty,
};

process.stdout.write(JSON.stringify(result, null, 2) + '\n');
process.exit(violations.length > 0 ? 2 : 0);
