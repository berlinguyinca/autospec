#!/usr/bin/env node
// behavior-taxonomy-check.mjs — Metric D: behavior taxonomy coverage checker.
//
// Usage: node behavior-taxonomy-check.mjs <test_results_dir> <contract_json_file>
//
// Reads Playwright trace/result data from test_results_dir.
// Maps interaction primitives to declared behavior categories per spec §4.
//
// Output JSON (stdout):
//   {
//     "passed": true,
//     "missing": [],
//     "passing": ["sort","scroll","upload",...],
//     "categories_checked": 9
//   }
//
// Exit codes: 0=passed, 1=fatal, 2=missing categories found

import { readFileSync, existsSync, readdirSync, statSync } from 'fs';
import { join } from 'path';

const ALL_CATEGORIES = ['sort', 'scroll', 'upload', 'download', 'filter', 'paginate', 'bulk_select', 'keyboard_nav', 'drag_drop'];

// Primitive → category mapping per spec §4 Metric D
// Each category requires ≥1 trace entry with a matching primitive.
const CATEGORY_PRIMITIVES = {
  sort:         ['click.*column.*header', 'sort', 'columnheader'],
  scroll:       ['scroll', 'wheel', 'touchmove'],
  upload:       ['upload', 'setInputFiles', 'file.*input'],
  download:     ['download', 'waitForDownload'],
  filter:       ['filter', 'search', 'input.*filter', 'select.*filter'],
  paginate:     ['paginate', 'page.*next', 'page.*prev', 'pagination'],
  bulk_select:  ['checkbox.*all', 'select.*all', 'bulk.*select', 'check.*all'],
  keyboard_nav: ['keyboard', 'press.*tab', 'press.*arrow', 'press.*enter', 'keydown'],
  drag_drop:    ['drag', 'drop', 'dragover', 'dragstart'],
};

const [,, testResultsDir, contractArg] = process.argv;

if (!testResultsDir || !contractArg) {
  process.stderr.write('Usage: behavior-taxonomy-check.mjs <test_results_dir> <contract_json_file>\n');
  process.exit(1);
}

let contract;
try {
  contract = JSON.parse(readFileSync(contractArg, 'utf8'));
} catch (e) {
  process.stderr.write(`behavior-taxonomy-check: fatal: cannot parse contract: ${e.message}\n`);
  process.exit(1);
}

// Get declared categories from contract (defaults to all if not specified)
const declaredCategories = contract?.e2e?.coverage_thresholds?.behavior_categories || ALL_CATEGORIES;

// ── Collect trace content ─────────────────────────────────────────────────────
function collectTraceContent(dir) {
  const lines = [];
  if (!existsSync(dir)) return lines;

  function walk(d) {
    try {
      for (const entry of readdirSync(d)) {
        const full = join(d, entry);
        const st = statSync(full);
        if (st.isDirectory()) {
          walk(full);
        } else if (entry.endsWith('.json') || entry.endsWith('.txt') || entry.endsWith('.log')) {
          try {
            lines.push(readFileSync(full, 'utf8').toLowerCase());
          } catch (_) {}
        }
      }
    } catch (_) {}
  }
  walk(dir);
  return lines;
}

const traceLines = collectTraceContent(testResultsDir).join('\n');

// Also look for annotation-style markers: category:sort, @category(scroll), etc.
const annotationPattern = /(?:category[:\s(]+)(['"]?)(\w+)\1/gi;
const foundAnnotations = new Set();
let m;
while ((m = annotationPattern.exec(traceLines)) !== null) {
  foundAnnotations.add(m[2].toLowerCase());
}

// ── Check each declared category ──────────────────────────────────────────────
const passing = [];
const missing = [];

for (const category of declaredCategories) {
  const primitives = CATEGORY_PRIMITIVES[category] || [category];

  // Check annotations first
  if (foundAnnotations.has(category)) {
    passing.push(category);
    continue;
  }

  // Check trace primitives
  let found = false;
  for (const primitive of primitives) {
    try {
      const regex = new RegExp(primitive, 'i');
      if (regex.test(traceLines)) {
        found = true;
        break;
      }
    } catch (_) {}
  }

  if (found) {
    passing.push(category);
  } else {
    missing.push(category);
  }
}

const result = {
  passed: missing.length === 0,
  missing,
  passing,
  categories_checked: declaredCategories.length,
};

process.stdout.write(JSON.stringify(result, null, 2) + '\n');
process.exit(missing.length > 0 ? 2 : 0);
