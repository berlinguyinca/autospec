/**
 * extended-crawler.mjs — Metric H: Extended BFS Crawler
 *
 * Reads a contract + base_url from stdin JSON and performs:
 *   1. BFS over in-domain a[href] links, capped at bfs_max_routes (default 200)
 *   2. Opens all foldouts on each page when crawler.open_all_foldouts=true
 *   3. Verifies every declared affordance_pattern on each visited page
 *   4. Emits gate JSON on stdout
 *
 * Input (stdin JSON):
 *   {
 *     contract: { e2e: { invariants_v2: { crawler: { ... } } } },
 *     base_url: string
 *   }
 *
 * Output (stdout JSON):
 *   {
 *     metric: "H",
 *     passed: boolean,
 *     unaffordable_elements: [{ route, element_index, failure_reason }],
 *     routes_visited: number,
 *     routes_capped: boolean,
 *     summary: { total_checks, passed_checks, failed_checks }
 *   }
 *
 * Exit codes: 0 = pass, 1 = fail, 2 = fatal error
 */

import { chromium } from '/opt/homebrew/lib/node_modules/playwright/index.mjs';
import { openAllFoldouts } from './foldout-opener.mjs';
import { verifyAffordance } from './affordance-verifier.mjs';

async function run() {
  let input;
  try {
    const chunks = [];
    for await (const chunk of process.stdin) chunks.push(chunk);
    input = JSON.parse(Buffer.concat(chunks).toString('utf8'));
  } catch (e) {
    process.stderr.write(`[extended-crawler] fatal: failed to parse stdin JSON: ${e.message}\n`);
    process.exit(2);
  }

  const { contract, base_url: baseUrl } = input;
  if (!contract || !baseUrl) {
    process.stderr.write('[extended-crawler] fatal: stdin must have { contract, base_url }\n');
    process.exit(2);
  }

  const invariantsV2 = contract?.e2e?.invariants_v2;
  if (!invariantsV2?.enabled || !invariantsV2?.crawler?.enabled) {
    const result = {
      metric: 'H',
      passed: true,
      unaffordable_elements: [],
      routes_visited: 0,
      routes_capped: false,
      summary: { total_checks: 0, passed_checks: 0, failed_checks: 0 },
    };
    process.stdout.write(JSON.stringify(result, null, 2) + '\n');
    process.exit(0);
  }

  const crawlerConfig = invariantsV2.crawler;
  const bfsMaxRoutes = crawlerConfig.bfs_max_routes ?? 200;
  const openFoldouts = crawlerConfig.open_all_foldouts ?? false;
  const affordancePatterns = crawlerConfig.affordance_patterns ?? [];
  const maxUnaffordable = crawlerConfig.failure_threshold?.max_unaffordable_elements ?? 0;

  // Normalise base URL (strip trailing slash)
  const base = baseUrl.replace(/\/$/, '');
  let baseOrigin;
  try {
    baseOrigin = new URL(base).origin;
  } catch {
    process.stderr.write(`[extended-crawler] fatal: invalid base_url: ${base}\n`);
    process.exit(2);
  }

  /** Normalise URL: strip trailing slash (except bare origin), strip hash */
  function normalizeUrl(u) {
    try {
      const parsed = new URL(u);
      parsed.hash = '';
      // Keep pathname as-is but strip trailing slash if it has a path
      if (parsed.pathname.length > 1 && parsed.pathname.endsWith('/')) {
        parsed.pathname = parsed.pathname.slice(0, -1);
      }
      return parsed.toString();
    } catch {
      return u;
    }
  }

  const browser = await chromium.launch({ headless: true });
  const page = await browser.newPage();

  const startUrl = normalizeUrl(base + '/');
  const queue = [startUrl];
  const visited = new Set();
  const allResults = [];

  try {
    while (queue.length > 0 && visited.size < bfsMaxRoutes) {
      const url = normalizeUrl(queue.shift());
      if (visited.has(url)) continue;
      visited.add(url);

      // Navigate to the URL
      try {
        await page.goto(url, { waitUntil: 'domcontentloaded', timeout: 15_000 });
      } catch (e) {
        process.stderr.write(`[extended-crawler] warn: goto ${url} failed: ${e.message}\n`);
        continue;
      }

      // Open foldouts if configured
      if (openFoldouts) {
        await openAllFoldouts(page).catch(e => {
          process.stderr.write(`[extended-crawler] warn: openAllFoldouts failed on ${url}: ${e.message}\n`);
        });
      }

      // Discover in-domain hrefs
      const hrefs = await page.evaluate((origin) => {
        const links = Array.from(document.querySelectorAll('a[href]'));
        return links
          .map(a => a.href)
          .filter(href => {
            try {
              const u = new URL(href);
              return u.origin === origin && !href.includes('#');
            } catch {
              return false;
            }
          });
      }, baseOrigin).catch(() => []);

      for (const rawHref of hrefs) {
        const href = normalizeUrl(rawHref);
        if (!visited.has(href) && !queue.includes(href)) {
          queue.push(href);
        }
      }

      // Verify affordance patterns on this page
      const route = url;
      for (const pattern of affordancePatterns) {
        // Quick check: does the element exist on this page?
        const matchCount = await page.locator(pattern.element).count().catch(() => 0);
        if (matchCount === 0) continue;

        const results = await verifyAffordance(page, pattern, route).catch(e => {
          process.stderr.write(`[extended-crawler] warn: verifyAffordance failed on ${url}: ${e.message}\n`);
          return [];
        });
        allResults.push(...results);
      }
    }
  } finally {
    await browser.close();
  }

  const routesCapped = visited.size >= bfsMaxRoutes && queue.length > 0;
  const unaffordableElements = allResults.filter(r => !r.passed && !r.skipped);
  const passedChecks = allResults.filter(r => r.passed && !r.skipped).length;
  const failedChecks = unaffordableElements.length;
  const totalChecks = allResults.filter(r => !r.skipped).length;

  const passed = unaffordableElements.length <= maxUnaffordable;

  const output = {
    metric: 'H',
    passed,
    unaffordable_elements: unaffordableElements.map(r => ({
      route: r.route,
      element_index: r.element_index,
      failure_reason: r.failure_reason,
    })),
    routes_visited: visited.size,
    routes_capped: routesCapped,
    summary: {
      total_checks: totalChecks,
      passed_checks: passedChecks,
      failed_checks: failedChecks,
    },
  };

  process.stdout.write(JSON.stringify(output, null, 2) + '\n');
  process.exit(output.passed ? 0 : 1);
}

run().catch(e => {
  process.stderr.write(`[extended-crawler] fatal: ${e.message}\n${e.stack}\n`);
  process.exit(2);
});
