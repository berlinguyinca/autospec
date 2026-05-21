#!/usr/bin/env node
/**
 * run-structural.mjs — Metric F: Structural Invariants Runner
 *
 * Reads a contract + base_url from stdin JSON, executes each declared invariant
 * against each apply_on_routes entry using the kind catalog from phase 2, and
 * emits the canonical Stage 2.5 gate JSON shape on stdout.
 *
 * Input (stdin JSON):
 *   {
 *     contract: { e2e: { invariants_v2: { invariants: [...], crawler?: { open_all_foldouts? } } } },
 *     base_url: string,         // e.g. "http://localhost:3000" or "file:///path/to/fixture"
 *     route_list?: string[],    // optional override (defaults to apply_on_routes per invariant)
 *     custom_kinds_dir?: string // optional path to .autospec/invariant-kinds/ in target repo
 *   }
 *
 * Output (stdout JSON):
 *   {
 *     metric: "F",
 *     passed: boolean,
 *     invariants: [{ id, kind, route, passed, violations[], count_observed }],
 *     summary: { total, passed_count, failed_count, violation_count }
 *   }
 *
 * Exit codes: 0 = pass, 1 = fail (invariant violations), 2 = fatal error
 */

import { fileURLToPath } from 'node:url';
import path from 'node:path';
import fs from 'node:fs';
import { chromium } from '/opt/homebrew/lib/node_modules/playwright/index.mjs';
import { openAllFoldouts } from '../crawler-v2/foldout-opener.mjs';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const KINDS_DIR = path.join(__dirname, 'kinds');

// ── Kind catalog loading ───────────────────────────────────────────────────────

async function loadKindCatalog(customKindsDir) {
  const catalog = new Map();

  // Load built-in kinds
  const builtinFiles = fs.readdirSync(KINDS_DIR).filter(f => f.endsWith('.mjs'));
  for (const file of builtinFiles) {
    try {
      const mod = await import(path.join(KINDS_DIR, file));
      if (mod.id && typeof mod.run === 'function') {
        catalog.set(mod.id, mod);
      }
    } catch (e) {
      process.stderr.write(`[run-structural] warn: failed to load built-in kind ${file}: ${e.message}\n`);
    }
  }

  // Load custom kinds if directory provided
  if (customKindsDir && fs.existsSync(customKindsDir)) {
    const customFiles = fs.readdirSync(customKindsDir).filter(f => f.endsWith('.mjs'));
    for (const file of customFiles) {
      try {
        const mod = await import(path.join(customKindsDir, file));
        if (mod.id && typeof mod.run === 'function') {
          catalog.set(mod.id, mod); // custom kinds can override built-ins
        } else {
          process.stderr.write(`[run-structural] warn: custom kind ${file} missing id or run export\n`);
        }
      } catch (e) {
        process.stderr.write(`[run-structural] warn: failed to load custom kind ${file}: ${e.message}\n`);
      }
    }
  }

  return catalog;
}

// ── Main runner ────────────────────────────────────────────────────────────────

async function run() {
  // Read stdin
  let input;
  try {
    const chunks = [];
    for await (const chunk of process.stdin) chunks.push(chunk);
    input = JSON.parse(Buffer.concat(chunks).toString('utf8'));
  } catch (e) {
    process.stderr.write(`[run-structural] fatal: failed to parse stdin JSON: ${e.message}\n`);
    process.exit(2);
  }

  const { contract, base_url: baseUrl, custom_kinds_dir: customKindsDir } = input;

  if (!contract || !baseUrl) {
    process.stderr.write('[run-structural] fatal: stdin must have { contract, base_url }\n');
    process.exit(2);
  }

  const invariantsV2 = contract?.e2e?.invariants_v2;
  if (!invariantsV2?.enabled) {
    const result = {
      metric: 'F',
      passed: true,
      invariants: [],
      summary: { total: 0, passed_count: 0, failed_count: 0, violation_count: 0 },
    };
    process.stdout.write(JSON.stringify(result, null, 2) + '\n');
    process.exit(0);
  }

  const invariants = invariantsV2.invariants || [];
  const openFoldouts = invariantsV2.crawler?.open_all_foldouts ?? false;

  const catalog = await loadKindCatalog(customKindsDir);
  const browser = await chromium.launch({ headless: true });
  const page = await browser.newPage();

  const results = [];

  try {
    for (const invariant of invariants) {
      const kindMod = catalog.get(invariant.kind);
      if (!kindMod) {
        process.stderr.write(`[run-structural] warn: unknown kind "${invariant.kind}" — skipping invariant "${invariant.id}"\n`);
        continue;
      }

      const routes = invariant.apply_on_routes || [];
      for (const route of routes) {
        const url = baseUrl.replace(/\/$/, '') + route;
        await page.goto(url, { waitUntil: 'domcontentloaded' }).catch((e) => {
          process.stderr.write(`[run-structural] warn: goto ${url} failed: ${e.message}\n`);
        });

        if (openFoldouts) {
          await openAllFoldouts(page);
        }

        let kindResult;
        try {
          kindResult = await kindMod.run(page, invariant, { baseUrl, route });
        } catch (e) {
          kindResult = {
            passed: false,
            violations: [{ index: -1, selector: invariant.kind, reason: `run() threw: ${e.message}` }],
            count_observed: 0,
          };
        }

        results.push({
          id: invariant.id,
          kind: invariant.kind,
          route,
          passed: kindResult.passed,
          violations: kindResult.violations,
          count_observed: kindResult.count_observed,
        });
      }
    }
  } finally {
    await browser.close();
  }

  const passed_count = results.filter(r => r.passed).length;
  const failed_count = results.filter(r => !r.passed).length;
  const violation_count = results.reduce((sum, r) => sum + (r.violations?.length || 0), 0);

  const output = {
    metric: 'F',
    passed: failed_count === 0,
    invariants: results,
    summary: {
      total: results.length,
      passed_count,
      failed_count,
      violation_count,
    },
  };

  process.stdout.write(JSON.stringify(output, null, 2) + '\n');
  process.exit(output.passed ? 0 : 1);
}

run().catch((e) => {
  process.stderr.write(`[run-structural] fatal: ${e.message}\n${e.stack}\n`);
  process.exit(2);
});
