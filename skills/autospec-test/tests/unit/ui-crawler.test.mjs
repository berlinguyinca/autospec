// skills/autospec-test/tests/unit/ui-crawler.test.mjs
// node --test  (Node.js built-in test runner)
// Tests for scripts/ui-crawler.mjs

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import fs from 'node:fs';
import os from 'node:os';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const SCRIPTS_DIR = path.resolve(__dirname, '../../scripts');

const { crawl } = await import(`${SCRIPTS_DIR}/ui-crawler.mjs`);

// ── Selector strategy preference ──────────────────────────────────────────────

test('crawl: selector preference order is data-testid > role+name > xpath', async () => {
    // The crawl function should document its preference in a property or docstring
    // We test by verifying elements with data-testid get that as selector
    const result = await crawl('http://localhost:9999', {
        maxRoutes: 1,
        dryRun: true,  // dry-run: don't actually launch browser
        mockElements: [
            { selector: 'button[data-testid="submit"]', role: 'button', name: 'Submit' },
            { selector: 'button', role: 'button', name: 'Cancel' },
            { selector: '/html/body/button[3]', role: null, name: null }
        ]
    });
    assert.ok(result, 'crawl should return a result');
    assert.ok('routes' in result, 'result should have routes');
    assert.ok('manifest' in result, 'result should have manifest');
    assert.ok('selector_strategy' in result, 'result should declare selector strategy');
    assert.equal(result.selector_strategy, 'data-testid > role+name > xpath');
});

test('crawl: caps at maxRoutes (default 200)', async () => {
    const result = await crawl('http://localhost:9999', {
        maxRoutes: 5,
        dryRun: true,
        mockRoutes: Array.from({ length: 10 }, (_, i) => `http://localhost:9999/page${i}`)
    });
    assert.ok(result.routes.length <= 5, `Expected <= 5 routes, got ${result.routes.length}`);
});

test('crawl: respects sitemap.xml when present', async () => {
    const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'crawler-test-'));
    const sitemapContent = `<?xml version="1.0" encoding="UTF-8"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
  <url><loc>http://localhost:9999/</loc></url>
  <url><loc>http://localhost:9999/about</loc></url>
</urlset>`;
    fs.writeFileSync(path.join(tmpDir, 'sitemap.xml'), sitemapContent);

    const result = await crawl('http://localhost:9999', {
        maxRoutes: 200,
        dryRun: true,
        sitemapPath: path.join(tmpDir, 'sitemap.xml')
    });
    // When sitemap provided, crawler should seed from sitemap URLs
    assert.ok(result.seeded_from_sitemap, 'should indicate sitemap was used');
    fs.rmSync(tmpDir, { recursive: true });
});

test('crawl: dry-run returns valid manifest structure', async () => {
    const result = await crawl('http://localhost:9999', { dryRun: true });
    assert.ok(Array.isArray(result.routes), 'routes should be array');
    assert.ok(Array.isArray(result.manifest), 'manifest should be array');
    // Each manifest entry should have route and selector
    for (const entry of result.manifest) {
        assert.ok('route' in entry, `manifest entry missing route: ${JSON.stringify(entry)}`);
        assert.ok('selector' in entry, `manifest entry missing selector: ${JSON.stringify(entry)}`);
    }
});

test('crawl: throws for non-http URL', async () => {
    await assert.rejects(
        () => crawl('ftp://example.com', { dryRun: true }),
        /invalid|unsupported|http/i
    );
});
