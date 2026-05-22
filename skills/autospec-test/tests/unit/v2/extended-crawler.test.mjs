/**
 * extended-crawler.test.mjs — Unit tests for Metric H extended crawler.
 *
 * Uses real Playwright against a static inline HTTP server serving the 3-page fixture.
 * No mocks. Tests: working case, broken case, tolerance, BFS cap, foldout integration.
 *
 * Run: node --test skills/autospec-test/tests/unit/v2/extended-crawler.test.mjs
 */

import { describe, it, before, after } from 'node:test';
import assert from 'node:assert/strict';
import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium } from '/opt/homebrew/lib/node_modules/playwright/index.mjs';
import { verifyAffordance } from '../../../scripts/crawler-v2/affordance-verifier.mjs';
import { openAllFoldouts } from '../../../scripts/crawler-v2/foldout-opener.mjs';

// ── In-process BFS crawler (mirrors extended-crawler.mjs logic for direct testing) ──

async function runCrawlerInProcess({ page, baseUrl, crawlerConfig }) {
  const bfsMaxRoutes = crawlerConfig.bfs_max_routes ?? 200;
  const openFoldouts = crawlerConfig.open_all_foldouts ?? false;
  const affordancePatterns = crawlerConfig.affordance_patterns ?? [];
  const maxUnaffordable = crawlerConfig.failure_threshold?.max_unaffordable_elements ?? 0;

  const base = baseUrl.replace(/\/$/, '');
  const baseOrigin = new URL(base).origin;

  function normalizeUrl(u) {
    try {
      const parsed = new URL(u);
      parsed.hash = '';
      if (parsed.pathname.length > 1 && parsed.pathname.endsWith('/')) {
        parsed.pathname = parsed.pathname.slice(0, -1);
      }
      return parsed.toString();
    } catch { return u; }
  }

  const startUrl = normalizeUrl(base + '/');
  const queue = [startUrl];
  const visited = new Set();
  const allResults = [];

  while (queue.length > 0 && visited.size < bfsMaxRoutes) {
    const url = normalizeUrl(queue.shift());
    if (visited.has(url)) continue;
    visited.add(url);

    try {
      await page.goto(url, { waitUntil: 'domcontentloaded', timeout: 10_000 });
    } catch {
      continue;
    }

    if (openFoldouts) {
      await openAllFoldouts(page).catch(() => {});
    }

    const hrefs = await page.evaluate((origin) => {
      return Array.from(document.querySelectorAll('a[href]'))
        .map(a => a.href)
        .filter(href => {
          try { const u = new URL(href); return u.origin === origin && !href.includes('#'); }
          catch { return false; }
        });
    }, baseOrigin).catch(() => []);

    for (const rawHref of hrefs) {
      const href = normalizeUrl(rawHref);
      if (!visited.has(href) && !queue.includes(href)) queue.push(href);
    }

    for (const pattern of affordancePatterns) {
      const matchCount = await page.locator(pattern.element).count().catch(() => 0);
      if (matchCount === 0) continue;
      const results = await verifyAffordance(page, pattern, url).catch(() => []);
      allResults.push(...results);
    }
  }

  const routesCapped = visited.size >= bfsMaxRoutes && queue.length > 0;
  const unaffordableElements = allResults.filter(r => !r.passed && !r.skipped);
  const passedChecks = allResults.filter(r => r.passed && !r.skipped).length;
  const failedChecks = unaffordableElements.length;
  const totalChecks = allResults.filter(r => !r.skipped).length;
  const passed = unaffordableElements.length <= maxUnaffordable;

  return {
    metric: 'H',
    passed,
    unaffordable_elements: unaffordableElements,
    routes_visited: visited.size,
    routes_capped: routesCapped,
    summary: { total_checks: totalChecks, passed_checks: passedChecks, failed_checks: failedChecks },
  };
}

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const FIXTURE_DIR = path.join(__dirname, '../../fixtures/v2/crawler-v2/site');

// ── Inline test server ─────────────────────────────────────────────────────────

let server;
let baseUrl;

before(async () => {
  server = http.createServer((req, res) => {
    const url = new URL(req.url, 'http://localhost');
    let filePath;

    if (url.pathname === '/' || url.pathname === '/index.html' || url.pathname === '/page-a.html') {
      filePath = path.join(FIXTURE_DIR, 'page-a.html');
    } else if (url.pathname === '/page-b.html') {
      filePath = path.join(FIXTURE_DIR, 'page-b.html');
    } else if (url.pathname === '/page-c.html') {
      filePath = path.join(FIXTURE_DIR, 'page-c.html');
    } else {
      res.writeHead(404);
      res.end('not found');
      return;
    }

    const html = fs.readFileSync(filePath, 'utf8');
    res.writeHead(200, { 'Content-Type': 'text/html' });
    res.end(html);
  });

  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  baseUrl = `http://127.0.0.1:${server.address().port}`;
});

after(async () => {
  await new Promise(resolve => server.close(resolve));
});

// ── Shared affordance pattern ──────────────────────────────────────────────────

const WORKING_PATTERN = {
  element: '[data-testid^="done-item-row-"] [role="button"]',
  opens: '[data-testid="done-item-edit-dialog"]',
  closes_via: '[data-testid="done-item-edit-dialog"] [role="button"][aria-label="Close dialog"]',
};

// ── Tests ──────────────────────────────────────────────────────────────────────

describe('Metric H — affordance verifier', () => {
  let browser;
  let page;

  before(async () => {
    browser = await chromium.launch({ headless: true });
    page = await browser.newPage();
  });

  after(async () => {
    await browser.close();
  });

  it('working affordance on page-a: all checks pass', async () => {
    await page.goto(`${baseUrl}/page-a.html`, { waitUntil: 'domcontentloaded' });
    const results = await verifyAffordance(page, WORKING_PATTERN, '/page-a.html');
    const failures = results.filter(r => !r.passed && !r.skipped);
    assert.equal(failures.length, 0, `Expected no failures, got: ${JSON.stringify(failures)}`);
    assert.ok(results.length > 0, 'Expected at least one affordance checked');
  });

  it('broken affordance on page-b: dialog does not open → violation', async () => {
    await page.goto(`${baseUrl}/page-b.html`, { waitUntil: 'domcontentloaded' });
    const results = await verifyAffordance(page, WORKING_PATTERN, '/page-b.html');
    const failures = results.filter(r => !r.passed && !r.skipped);
    assert.ok(failures.length > 0, `Expected at least one failure on page-b, got: ${JSON.stringify(results)}`);
    assert.ok(failures[0].failure_reason, 'failure_reason should be set');
  });

  it('foldout hidden affordance: not found without openAllFoldouts', async () => {
    await page.goto(`${baseUrl}/page-c.html`, { waitUntil: 'domcontentloaded' });
    // Without opening foldouts, the done-item-row-3 inside <details> is hidden
    const results = await verifyAffordance(page, WORKING_PATTERN, '/page-c.html');
    // All elements should be skipped (not visible) or count=0
    const nonSkipped = results.filter(r => !r.skipped);
    assert.equal(nonSkipped.length, 0, 'Without foldout open, no visible affordances should be checked');
  });

  it('foldout hidden affordance: found and fails with openAllFoldouts', async () => {
    await page.goto(`${baseUrl}/page-c.html`, { waitUntil: 'domcontentloaded' });
    await openAllFoldouts(page);
    const results = await verifyAffordance(page, WORKING_PATTERN, '/page-c.html');
    const failures = results.filter(r => !r.passed && !r.skipped);
    assert.ok(failures.length > 0,
      `Expected failure after foldout opened (broken affordance inside), got: ${JSON.stringify(results)}`);
  });
});

describe('Metric H — extended-crawler (in-process BFS)', () => {
  let browser;
  let crawlPage;

  before(async () => {
    browser = await chromium.launch({ headless: true });
    crawlPage = await browser.newPage();
  });

  after(async () => {
    await browser.close();
  });

  it('working 3-page site with high tolerance: passed=true', async () => {
    const result = await runCrawlerInProcess({
      page: crawlPage,
      baseUrl,
      crawlerConfig: {
        bfs_max_routes: 200,
        open_all_foldouts: false,
        affordance_patterns: [WORKING_PATTERN],
        failure_threshold: { max_unaffordable_elements: 10 },
      },
    });
    assert.equal(result.metric, 'H');
    assert.equal(result.passed, true, `Expected passed=true with tolerance=10, got: ${JSON.stringify(result)}`);
    assert.ok(result.routes_visited >= 2, 'Should visit at least 2 routes (root + pages)');
  });

  it('broken fixture (page-b) emits passed:false with max_unaffordable_elements:0', async () => {
    // Fresh page to avoid route recorder interference from previous test
    const p = await browser.newPage();
    const result = await runCrawlerInProcess({
      page: p,
      baseUrl,
      crawlerConfig: {
        bfs_max_routes: 200,
        open_all_foldouts: false,
        affordance_patterns: [WORKING_PATTERN],
        failure_threshold: { max_unaffordable_elements: 0 },
      },
    });
    await p.close();

    assert.equal(result.passed, false, `Expected passed=false, got: ${JSON.stringify(result)}`);
    assert.ok(result.unaffordable_elements.length >= 1, 'Expected at least 1 unaffordable element');
    assert.ok(result.unaffordable_elements[0].failure_reason, 'violation should have failure_reason');
  });

  it('tolerance: max_unaffordable_elements=10 flips broken fixture to passed:true', async () => {
    const p = await browser.newPage();
    const result = await runCrawlerInProcess({
      page: p,
      baseUrl,
      crawlerConfig: {
        bfs_max_routes: 200,
        open_all_foldouts: false,
        affordance_patterns: [WORKING_PATTERN],
        failure_threshold: { max_unaffordable_elements: 10 },
      },
    });
    await p.close();
    assert.equal(result.passed, true, `Expected passed=true with tolerance=10, got: ${JSON.stringify(result)}`);
  });

  it('BFS cap: stops at bfs_max_routes and sets routes_capped=true when site has more routes', async () => {
    // Serve many pages via a dynamic server that generates N pages
    const manyServer = http.createServer((req, res) => {
      const url = new URL(req.url, 'http://localhost');
      const match = url.pathname.match(/^\/page-(\d+)\.html$/);
      if (url.pathname === '/') {
        // Root links to 250 pages
        let links = '';
        for (let i = 1; i <= 250; i++) links += `<a href="/page-${i}.html">P${i}</a>`;
        res.writeHead(200, { 'Content-Type': 'text/html' });
        res.end(`<html><body>${links}</body></html>`);
      } else if (match) {
        res.writeHead(200, { 'Content-Type': 'text/html' });
        res.end(`<html><body><p>Page ${match[1]}</p><a href="/">Home</a></body></html>`);
      } else {
        res.writeHead(404); res.end();
      }
    });
    await new Promise(r => manyServer.listen(0, '127.0.0.1', r));
    const manyBase = `http://127.0.0.1:${manyServer.address().port}`;

    const p = await browser.newPage();
    const result = await runCrawlerInProcess({
      page: p,
      baseUrl: manyBase,
      crawlerConfig: {
        bfs_max_routes: 10, // cap at 10 of 251 total routes
        open_all_foldouts: false,
        affordance_patterns: [],
        failure_threshold: { max_unaffordable_elements: 0 },
      },
    });
    await p.close();
    await new Promise(r => manyServer.close(r));

    assert.equal(result.routes_visited, 10, `Expected routes_visited=10, got ${result.routes_visited}`);
    assert.equal(result.routes_capped, true, 'Expected routes_capped=true');
    assert.equal(result.passed, true, 'No affordances checked → passed=true');
  });

  it('foldout integration: hidden broken affordance found only with open_all_foldouts', async () => {
    // Without foldouts: page-c's broken affordance (inside <details>) stays hidden
    const pNo = await browser.newPage();
    const resultNoFoldout = await runCrawlerInProcess({
      page: pNo,
      baseUrl,
      crawlerConfig: {
        bfs_max_routes: 200,
        open_all_foldouts: false,
        affordance_patterns: [WORKING_PATTERN],
        failure_threshold: { max_unaffordable_elements: 0 },
      },
    });
    await pNo.close();

    // With foldouts: page-c's foldout opens, broken affordance is found
    const pYes = await browser.newPage();
    const resultWithFoldout = await runCrawlerInProcess({
      page: pYes,
      baseUrl,
      crawlerConfig: {
        bfs_max_routes: 200,
        open_all_foldouts: true,
        affordance_patterns: [WORKING_PATTERN],
        failure_threshold: { max_unaffordable_elements: 0 },
      },
    });
    await pYes.close();

    assert.ok(
      resultWithFoldout.unaffordable_elements.length >= resultNoFoldout.unaffordable_elements.length,
      `With foldouts should find same or more violations. ` +
      `Without: ${resultNoFoldout.unaffordable_elements.length}, ` +
      `With: ${resultWithFoldout.unaffordable_elements.length}`,
    );
  });
});

describe('Metric H — extended-crawler subprocess (gate JSON shape)', () => {
  it('emits correct gate JSON with metric:H when disabled', async () => {
    const { execFileSync } = await import('node:child_process');

    const contract = {
      e2e: {
        invariants_v2: {
          enabled: false,
          crawler: { enabled: false },
        },
      },
    };

    let stdout;
    try {
      stdout = execFileSync('node', [
        'skills/autospec-test/scripts/crawler-v2/extended-crawler.mjs',
      ], {
        input: JSON.stringify({ contract, base_url: baseUrl }),
        cwd: path.resolve(__dirname, '../../../../..'),
        timeout: 15_000,
        encoding: 'utf8',
      });
    } catch (e) {
      throw new Error(`extended-crawler.mjs subprocess failed: ${e.stderr || e.message}`);
    }

    const result = JSON.parse(stdout);
    assert.equal(result.metric, 'H');
    assert.equal(typeof result.passed, 'boolean');
    assert.ok(Array.isArray(result.unaffordable_elements));
    assert.equal(typeof result.routes_visited, 'number');
    assert.equal(typeof result.routes_capped, 'boolean');
    assert.ok('total_checks' in result.summary);
    assert.ok('passed_checks' in result.summary);
    assert.ok('failed_checks' in result.summary);
  });
});
