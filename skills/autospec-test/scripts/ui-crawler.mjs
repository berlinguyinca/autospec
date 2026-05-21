#!/usr/bin/env node
// ui-crawler.mjs — headless UI element crawler producing route+element manifest.
//
// Usage: node ui-crawler.mjs <base_url_or_static_dir> [<max_routes>]
//
// In static mode (directory arg): parses HTML files directly.
// In URL mode: uses Playwright (must be installed in target repo) for BFS crawl.
//
// Output JSON (stdout):
//   {
//     "routes": ["/", "/about"],
//     "elements": [
//       { "route": "/", "selector": "[data-testid=submit-btn]", "strategy": "data-testid" }
//     ],
//     "routes_found": 2,
//     "elements_found": 5
//   }
//
// Selector strategy preference (per spec §4 Metric B):
//   data-testid > role+name > xpath
//
// Element types: button, a, input, select, textarea,
//   [role=button], [contenteditable], [tabindex], [onclick]
//
// Cap: ≤200 routes (configurable via MAX_ROUTES env or arg).
//
// Exit codes: 0=ok, 1=fatal

import { existsSync, readFileSync, readdirSync, statSync } from 'fs';
import { join, resolve, extname } from 'path';

const [,, target, maxRoutesArg] = process.argv;

if (!target) {
  process.stderr.write('Usage: ui-crawler.mjs <base_url_or_static_dir> [<max_routes>]\n');
  process.exit(1);
}

const MAX_ROUTES = parseInt(maxRoutesArg || process.env.MAX_ROUTES || '200', 10);

// ── Selector strategy ─────────────────────────────────────────────────────────
function selectorStrategy(attrs) {
  if (attrs['data-testid']) return { selector: `[data-testid="${attrs['data-testid']}"]`, strategy: 'data-testid' };
  if (attrs.role && attrs['aria-label']) return { selector: `[role="${attrs.role}"][aria-label="${attrs['aria-label']}"]`, strategy: 'role+name' };
  if (attrs.role) return { selector: `[role="${attrs.role}"]`, strategy: 'role+name' };
  if (attrs.id) return { selector: `#${attrs.id}`, strategy: 'id' };
  if (attrs.type) return { selector: `input[type="${attrs.type}"]`, strategy: 'attribute' };
  return null; // xpath would be used but we skip for static analysis
}

// ── Simple HTML attribute parser ──────────────────────────────────────────────
function parseAttrs(attrStr) {
  const attrs = {};
  const re = /([\w-]+)\s*=\s*(?:"([^"]*)"|'([^']*)'|(\S+))/g;
  let m;
  while ((m = re.exec(attrStr)) !== null) {
    attrs[m[1]] = m[2] ?? m[3] ?? m[4] ?? '';
  }
  return attrs;
}

// ── Static HTML file crawler ──────────────────────────────────────────────────
function crawlStaticDir(dir) {
  const routes = [];
  const elements = [];
  const visited = new Set();

  function processHTML(filePath, route) {
    if (visited.has(route)) return;
    visited.add(route);
    routes.push(route);

    let html;
    try {
      html = readFileSync(filePath, 'utf8');
    } catch (_) { return; }

    // Find interactive elements: button, a, input, select, textarea
    const elementRe = /<(button|a|input|select|textarea)(\s[^>]*)?\/?>/gi;
    // Also: elements with role, contenteditable, tabindex, onclick
    const roleRe = /<(\w+)(\s[^>]*(?:role|contenteditable|tabindex|onclick)[^>]*)>/gi;

    let em;
    while ((em = elementRe.exec(html)) !== null) {
      const tag = em[1].toLowerCase();
      const attrStr = em[2] || '';
      const attrs = parseAttrs(attrStr);

      const sel = selectorStrategy(attrs);
      if (sel) {
        elements.push({ route, tag, ...sel });
      } else {
        // Fallback: use tag + index as xpath-style
        elements.push({ route, tag, selector: tag, strategy: 'tag' });
      }
    }
    while ((em = roleRe.exec(html)) !== null) {
      const tag = em[1].toLowerCase();
      if (['button','a','input','select','textarea'].includes(tag)) continue; // already handled
      const attrStr = em[2] || '';
      const attrs = parseAttrs(attrStr);
      const sel = selectorStrategy(attrs);
      if (sel) {
        elements.push({ route, tag, ...sel });
      }
    }

    // Extract <a href> links for BFS
    const linkRe = /<a\s[^>]*href=["']([^"']+)["']/gi;
    let lm;
    while ((lm = linkRe.exec(html)) !== null && routes.length < MAX_ROUTES) {
      const href = lm[1];
      // Only follow local paths
      if (href.startsWith('/') || (!href.startsWith('http') && !href.startsWith('mailto'))) {
        const nextRoute = href.startsWith('/') ? href : `/${href}`;
        if (!visited.has(nextRoute)) {
          // Try to find a matching file
          const candidates = [
            join(dir, nextRoute.replace(/^\//, ''), 'index.html'),
            join(dir, nextRoute.replace(/^\//, '') + '.html'),
          ];
          for (const c of candidates) {
            if (existsSync(c)) {
              processHTML(c, nextRoute);
              break;
            }
          }
        }
      }
    }
  }

  // Check for sitemap.xml
  const sitemapPath = join(dir, 'sitemap.xml');
  if (existsSync(sitemapPath)) {
    const sitemap = readFileSync(sitemapPath, 'utf8');
    const locRe = /<loc>([^<]+)<\/loc>/g;
    let sm;
    while ((sm = locRe.exec(sitemap)) !== null && routes.length < MAX_ROUTES) {
      const url = sm[1].trim();
      // Extract path from URL
      try {
        const parsed = new URL(url);
        const routePath = parsed.pathname;
        const candidates = [
          join(dir, routePath.replace(/^\//, ''), 'index.html'),
          join(dir, routePath.replace(/^\//, '') + '.html'),
          join(dir, 'index.html'),
        ];
        for (const c of candidates) {
          if (existsSync(c)) {
            processHTML(c, routePath);
            break;
          }
        }
      } catch (_) {}
    }
  }

  // Start BFS from index.html
  const indexPath = join(dir, 'index.html');
  if (existsSync(indexPath)) {
    processHTML(indexPath, '/');
  } else {
    // Walk all HTML files
    const htmlFiles = [];
    function walkForHtml(d) {
      try {
        for (const entry of readdirSync(d)) {
          const full = join(d, entry);
          const st = statSync(full);
          if (st.isDirectory()) walkForHtml(full);
          else if (entry.endsWith('.html')) htmlFiles.push(full);
        }
      } catch (_) {}
    }
    walkForHtml(dir);
    for (const f of htmlFiles.slice(0, MAX_ROUTES)) {
      const route = '/' + f.replace(dir + '/', '').replace(/\/index\.html$/, '').replace(/\.html$/, '');
      processHTML(f, route);
    }
  }

  return { routes, elements };
}

// ── URL mode (requires Playwright in target) ──────────────────────────────────
async function crawlURL(baseURL) {
  // Try to use Playwright if available
  try {
    const { chromium } = await import('playwright');
    const browser = await chromium.launch({ headless: true });
    const context = await browser.newContext();
    const page = await context.newPage();

    const visited = new Set();
    const queue = [baseURL];
    const routes = [];
    const elements = [];

    while (queue.length > 0 && routes.length < MAX_ROUTES) {
      const url = queue.shift();
      if (visited.has(url)) continue;
      visited.add(url);

      try {
        await page.goto(url, { waitUntil: 'domcontentloaded', timeout: 10000 });
        const path = new URL(url).pathname;
        routes.push(path);

        // Collect interactive elements
        const elems = await page.evaluate(() => {
          const SELECTORS = 'button, a, input, select, textarea, [role=button], [contenteditable], [tabindex], [onclick]';
          return Array.from(document.querySelectorAll(SELECTORS)).map(el => ({
            tag: el.tagName.toLowerCase(),
            testId: el.getAttribute('data-testid'),
            role: el.getAttribute('role'),
            ariaLabel: el.getAttribute('aria-label'),
            id: el.id,
            type: el.getAttribute('type'),
          }));
        });

        for (const el of elems) {
          let selector, strategy;
          if (el.testId) {
            selector = `[data-testid="${el.testId}"]`;
            strategy = 'data-testid';
          } else if (el.role && el.ariaLabel) {
            selector = `[role="${el.role}"][aria-label="${el.ariaLabel}"]`;
            strategy = 'role+name';
          } else if (el.role) {
            selector = `[role="${el.role}"]`;
            strategy = 'role+name';
          } else if (el.id) {
            selector = `#${el.id}`;
            strategy = 'id';
          } else {
            continue; // skip elements with no stable selector
          }
          elements.push({ route: path, tag: el.tag, selector, strategy });
        }

        // Collect links for BFS
        const links = await page.evaluate((base) =>
          Array.from(document.querySelectorAll('a[href]'))
            .map(a => a.href)
            .filter(h => h.startsWith(base)),
          baseURL
        );
        for (const link of links) {
          if (!visited.has(link)) queue.push(link);
        }
      } catch (e) {
        process.stderr.write(`ui-crawler: WARN: failed to crawl ${url}: ${e.message}\n`);
      }
    }

    await browser.close();
    return { routes, elements };
  } catch (e) {
    process.stderr.write(`ui-crawler: WARN: Playwright not available (${e.message}); static analysis only\n`);
    return { routes: [], elements: [] };
  }
}

// ── Main ──────────────────────────────────────────────────────────────────────
let result;

const absTarget = resolve(target);
if (existsSync(absTarget) && statSync(absTarget).isDirectory()) {
  // Static dir mode
  const { routes, elements } = crawlStaticDir(absTarget);
  result = {
    routes,
    elements,
    routes_found: routes.length,
    elements_found: elements.length,
    mode: 'static',
  };
  process.stdout.write(JSON.stringify(result, null, 2) + '\n');
  process.exit(0);
} else {
  // URL mode
  crawlURL(target)
    .then(({ routes, elements }) => {
      result = {
        routes,
        elements,
        routes_found: routes.length,
        elements_found: elements.length,
        mode: 'playwright',
      };
      process.stdout.write(JSON.stringify(result, null, 2) + '\n');
      process.exit(0);
    })
    .catch(e => {
      process.stderr.write(`ui-crawler: fatal: ${e.message}\n`);
      process.exit(1);
    });
}
