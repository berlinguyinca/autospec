#!/usr/bin/env node
// scripts/ui-crawler.mjs
// Headless Playwright BFS crawler producing a route+element manifest.
//
// Selector preference: data-testid > role+name > xpath (per spec §4 Metric B)
//
// Export: crawl(baseURL, options) async function
// CLI: node ui-crawler.mjs --base-url <url> [--max-routes N] [--output <file>]

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const INTERACTIVE_SELECTORS = [
    '[role=button]',
    'button',
    'a[href]',
    'input',
    'select',
    'textarea',
    '[contenteditable]',
    '[tabindex]',
    '[onclick]',
];

/**
 * Parse a sitemap.xml file and return the list of URLs.
 * @param {string} sitemapPath
 * @returns {string[]}
 */
function parseSitemap(sitemapPath) {
    const content = fs.readFileSync(sitemapPath, 'utf8');
    const urls = [];
    const locPattern = /<loc>\s*(.*?)\s*<\/loc>/gi;
    let match;
    while ((match = locPattern.exec(content)) !== null) {
        urls.push(match[1]);
    }
    return urls;
}

/**
 * Derive the best stable selector for an element descriptor.
 * Preference: data-testid > role+name > xpath
 *
 * @param {{selector: string, role: string|null, name: string|null}} el
 * @returns {string}
 */
function bestSelector(el) {
    if (el.selector && el.selector.includes('data-testid')) {
        return el.selector;
    }
    if (el.role && el.name) {
        return `[role=${el.role}][name="${el.name}"]`;
    }
    return el.selector || 'unknown';
}

/**
 * Crawl a web application and produce a route+element manifest.
 *
 * In dry-run mode (options.dryRun === true), no browser is launched.
 * Instead, mockElements and mockRoutes from options are used to build
 * a synthetic result, enabling unit tests without Playwright.
 *
 * @param {string} baseURL
 * @param {object} [options]
 * @param {number} [options.maxRoutes=200]
 * @param {boolean} [options.dryRun=false]
 * @param {string} [options.sitemapPath] - path to sitemap.xml
 * @param {Array} [options.mockElements] - elements to use in dry-run
 * @param {string[]} [options.mockRoutes] - routes to use in dry-run
 * @returns {Promise<{routes: string[], manifest: {route: string, selector: string}[], selector_strategy: string, seeded_from_sitemap: boolean}>}
 */
export async function crawl(baseURL, options = {}) {
    const maxRoutes = options.maxRoutes ?? 200;
    const dryRun = options.dryRun === true;

    // Validate URL scheme
    if (!baseURL.startsWith('http://') && !baseURL.startsWith('https://')) {
        throw new Error(`invalid or unsupported URL scheme: ${baseURL}`);
    }

    let seededFromSitemap = false;

    // Dry-run mode: return synthetic result without launching browser
    if (dryRun) {
        const mockRoutes = (options.mockRoutes ?? [baseURL]).slice(0, maxRoutes);
        const mockElements = options.mockElements ?? [];

        // If sitemapPath provided, seed from sitemap
        let routes = mockRoutes;
        if (options.sitemapPath && fs.existsSync(options.sitemapPath)) {
            const sitemapURLs = parseSitemap(options.sitemapPath).slice(0, maxRoutes);
            routes = sitemapURLs.length > 0 ? sitemapURLs.slice(0, maxRoutes) : mockRoutes;
            seededFromSitemap = true;
        }

        const manifest = mockElements.map(el => ({
            route: baseURL,
            selector: bestSelector(el),
        }));

        return {
            routes,
            manifest,
            selector_strategy: 'data-testid > role+name > xpath',
            seeded_from_sitemap: seededFromSitemap,
        };
    }

    // Real crawl using Playwright
    let chromium;
    try {
        ({ chromium } = await import('playwright'));
    } catch {
        throw new Error('Playwright not installed; run: npm install playwright');
    }

    const visited = new Set();
    const queue = [];
    const manifest = [];

    // Seed from sitemap if available
    if (options.sitemapPath && fs.existsSync(options.sitemapPath)) {
        const sitemapURLs = parseSitemap(options.sitemapPath);
        for (const u of sitemapURLs) {
            queue.push(u);
        }
        seededFromSitemap = true;
    }

    if (queue.length === 0) {
        queue.push(baseURL);
    }

    const browser = await chromium.launch({ headless: true });
    const context = await browser.newContext();
    const page = await context.newPage();

    try {
        while (queue.length > 0 && visited.size < maxRoutes) {
            const url = queue.shift();
            if (visited.has(url)) continue;
            visited.add(url);

            try {
                await page.goto(url, { waitUntil: 'domcontentloaded', timeout: 15000 });
            } catch {
                continue;
            }

            // Collect interactive elements
            for (const sel of INTERACTIVE_SELECTORS) {
                const elements = await page.locator(sel).all();
                for (const el of elements) {
                    let chosenSelector = sel;

                    // Prefer data-testid
                    const testId = await el.getAttribute('data-testid').catch(() => null);
                    if (testId) {
                        chosenSelector = `[data-testid="${testId}"]`;
                    } else {
                        // Try role+name
                        const role = await el.getAttribute('role').catch(() => null);
                        const ariaLabel = await el.getAttribute('aria-label').catch(() => null);
                        const text = await el.innerText().catch(() => null);
                        const name = ariaLabel || (text && text.trim().slice(0, 50)) || null;
                        if (role && name) {
                            chosenSelector = `[role=${role}]` + (name ? `[aria-label="${name}"]` : '');
                        }
                        // else fallback to CSS selector
                    }

                    manifest.push({ route: url, selector: chosenSelector });
                }
            }

            // BFS: discover links on same origin
            const links = await page.locator('a[href]').all();
            const origin = new URL(baseURL).origin;
            for (const link of links) {
                const href = await link.getAttribute('href').catch(() => null);
                if (!href) continue;
                try {
                    const abs = new URL(href, url).href;
                    if (abs.startsWith(origin) && !visited.has(abs)) {
                        queue.push(abs);
                    }
                } catch {
                    // ignore invalid URLs
                }
            }
        }
    } finally {
        await browser.close();
    }

    return {
        routes: Array.from(visited),
        manifest,
        selector_strategy: 'data-testid > role+name > xpath',
        seeded_from_sitemap: seededFromSitemap,
    };
}

// CLI entrypoint
const __filename = fileURLToPath(import.meta.url);
if (process.argv[1] && fs.realpathSync(path.resolve(process.argv[1])) === fs.realpathSync(path.resolve(__filename))) {
    const args = process.argv.slice(2);
    let baseURL = null;
    let maxRoutes = 200;
    let outputFile = null;
    let sitemapPath = null;

    for (let i = 0; i < args.length; i++) {
        if (args[i] === '--base-url') baseURL = args[i + 1];
        if (args[i] === '--max-routes') maxRoutes = parseInt(args[i + 1], 10);
        if (args[i] === '--output') outputFile = args[i + 1];
        if (args[i] === '--sitemap') sitemapPath = args[i + 1];
    }

    if (!baseURL) {
        process.stderr.write('Usage: ui-crawler.mjs --base-url <url> [--max-routes N] [--output <file>] [--sitemap <path>]\n');
        process.exit(1);
    }

    try {
        const result = await crawl(baseURL, { maxRoutes, sitemapPath });
        const json = JSON.stringify(result, null, 2) + '\n';
        if (outputFile) {
            fs.writeFileSync(outputFile, json, 'utf8');
        } else {
            process.stdout.write(json);
        }
        process.exit(0);
    } catch (err) {
        process.stderr.write(`ui-crawler: error: ${err.message}\n`);
        process.exit(1);
    }
}
