#!/usr/bin/env node
// skills/autospec-harmonize/scripts/extract-runtime.mjs
//
// Runtime computed-CSS token-profile extractor for autospec-harmonize Stage 1.
//
// CLI: node extract-runtime.mjs --url <url> [--pages a,b]
// Stdout: JSON token profile conforming to
//         schemas/autospec-harmonize-token-profile.schema.json (source:"runtime")
//
// Lazily imports Playwright, crawls the route(s), reads getComputedStyle over
// every element, converts rgb()->hex, and aggregates into the same token-profile
// shape as extract-source.mjs. Screenshots each route to
// .autospec/design/before/<route>.png. When Playwright is missing or the URL is
// unreachable it prints `code_health:harmonize_runtime_unavailable` and exits 3.

import fs from 'node:fs';
import path from 'node:path';

const RUNTIME_UNAVAILABLE = 'code_health:harmonize_runtime_unavailable';

// ---------------------------------------------------------------------------
// CLI arg parsing
// ---------------------------------------------------------------------------
function parseArgs(argv) {
  const args = argv.slice(2);
  const result = { url: null, pages: null };
  for (let i = 0; i < args.length; i++) {
    if (args[i] === '--url' && args[i + 1]) result.url = args[++i];
    else if (args[i] === '--pages' && args[i + 1]) result.pages = args[++i];
  }
  return result;
}

/** Print a code_health line and exit 3 (the runtime-unavailable degradation path). */
function failUnavailable(detail) {
  process.stderr.write(`${RUNTIME_UNAVAILABLE}: ${detail}\n`);
  process.exit(3);
}

// ---------------------------------------------------------------------------
// Conversion + aggregation helpers (mirror extract-source.mjs output shape)
// ---------------------------------------------------------------------------

/** "rgb(26, 115, 232)" / "rgba(26,115,232,0.5)" -> "#1a73e8" (null if transparent/unparseable). */
function rgbToHex(value) {
  const v = (value || '').trim();
  const m = /rgba?\(\s*(\d+)\s*,\s*(\d+)\s*,\s*(\d+)/i.exec(v);
  if (!m) return null;
  const alpha = /rgba\(\s*\d+\s*,\s*\d+\s*,\s*\d+\s*,\s*([\d.]+)\s*\)/i.exec(v);
  if (alpha && Number(alpha[1]) === 0) return null; // fully transparent
  const [r, g, b] = [m[1], m[2], m[3]].map(Number);
  return '#' + [r, g, b].map(n => n.toString(16).padStart(2, '0')).join('').toLowerCase();
}

/** "16px" -> 16 (null otherwise). */
function pxNum(value) {
  const m = /^(\d+(?:\.\d+)?)px$/.exec((value || '').trim());
  return m ? Number(m[1]) : null;
}

function detectInconsistencies(palette, typeScale, buttons) {
  const result = [];
  if (palette.length > 3) {
    result.push({ category: 'palette', detail: `${palette.length} colors found; consider consolidating to a smaller set` });
  }
  if (typeScale.length > 4) {
    result.push({ category: 'type_scale', detail: `${typeScale.length} distinct font sizes found` });
  }
  if (buttons.length > 1) {
    result.push({ category: 'components', detail: `${buttons.length} button treatments found` });
  }
  return result;
}

/** Reduce per-element computed-style records to the token-profile sub-shapes. */
function aggregate(records) {
  const paletteCounts = new Map();
  const type = new Set();
  const spacing = new Set();
  const radii = new Set();
  const shadows = new Set();
  const buttons = [];

  for (const rec of records) {
    for (const colorStr of [rec.color, rec.backgroundColor]) {
      const hex = rgbToHex(colorStr);
      if (hex) paletteCounts.set(hex, (paletteCounts.get(hex) ?? 0) + 1);
    }
    const fz = pxNum(rec.fontSize);
    if (fz != null) type.add(fz);
    for (const p of rec.paddings || []) {
      const v = pxNum(p);
      if (v != null && v > 0) spacing.add(v);
    }
    for (const rd of rec.radii || []) {
      const v = pxNum(rd);
      if (v != null && v > 0) radii.add(v);
    }
    if (rec.boxShadow && rec.boxShadow !== 'none') shadows.add(rec.boxShadow.trim());
    if (rec.isButton) {
      buttons.push({
        selector: rec.selector || 'button',
        rules: `color:${rec.color}; background:${rec.backgroundColor}; radius:${(rec.radii || [])[0] || ''}`.slice(0, 200),
      });
    }
  }

  const palette = Array.from(paletteCounts.entries())
    .sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))
    .map(([hex, count]) => ({ hex, count }));

  return {
    palette,
    type_scale: Array.from(type).sort((a, b) => a - b).map(px => ({ px })),
    spacing: Array.from(spacing).sort((a, b) => a - b).map(px => ({ px })),
    radii: Array.from(radii).sort((a, b) => a - b).map(px => ({ px })),
    shadows: Array.from(shadows).map(value => ({ value })),
    buttons,
  };
}

// ---------------------------------------------------------------------------
// Page reader — runs in the browser context via $$eval.
// ---------------------------------------------------------------------------
function readComputedStyles(els) {
  return els.slice(0, 5000).map(el => {
    const cs = getComputedStyle(el);
    const tag = el.tagName.toLowerCase();
    const cls = (el.className && typeof el.className === 'string') ? el.className : '';
    const isButton = tag === 'button' || /\b(btn|button|cta)\b/i.test(cls);
    return {
      color: cs.color,
      backgroundColor: cs.backgroundColor,
      fontSize: cs.fontSize,
      paddings: [cs.paddingTop, cs.paddingRight, cs.paddingBottom, cs.paddingLeft],
      radii: [cs.borderTopLeftRadius, cs.borderTopRightRadius, cs.borderBottomRightRadius, cs.borderBottomLeftRadius],
      boxShadow: cs.boxShadow,
      isButton,
      selector: isButton ? (tag + (cls ? '.' + cls.trim().split(/\s+/).join('.') : '')) : null,
    };
  });
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------
async function main() {
  const args = parseArgs(process.argv);

  if (!args.url) {
    process.stderr.write('Usage: node extract-runtime.mjs --url <url> [--pages a,b]\n');
    process.exit(1);
  }

  let playwright;
  try {
    playwright = await import('playwright');
  } catch {
    failUnavailable('playwright not installed (run: npm i -D playwright && npx playwright install chromium)');
  }
  const chromium = playwright.chromium || (playwright.default && playwright.default.chromium);
  if (!chromium) failUnavailable('playwright chromium export unavailable');

  let browser = null;
  try {
    browser = await chromium.launch();
  } catch (e) {
    failUnavailable(`browser launch failed: ${e && e.message ? e.message : e}`);
  }
  if (!browser) failUnavailable('browser launch returned null');

  const routes = args.pages
    ? args.pages.split(',').map(s => s.trim()).filter(Boolean)
    : [''];

  const beforeDir = path.join('.autospec', 'design', 'before');
  fs.mkdirSync(beforeDir, { recursive: true });

  const records = [];
  try {
    for (const route of routes) {
      const page = await browser.newPage();
      const target = route ? new URL(route, args.url).toString() : args.url;
      await page.goto(target, { timeout: 8000, waitUntil: 'load' });
      const recs = await page.$$eval('*', readComputedStyles);
      records.push(...recs);
      const safe = (route || 'index').replace(/[^a-zA-Z0-9_-]+/g, '_').replace(/^_+|_+$/g, '') || 'index';
      try { await page.screenshot({ path: path.join(beforeDir, `${safe}.png`), fullPage: true }); } catch { /* screenshot is best-effort */ }
      await page.close();
    }
  } catch (e) {
    try { await browser.close(); } catch { /* ignore */ }
    failUnavailable(`navigation failed: ${e && e.message ? e.message : e}`);
  }
  try { await browser.close(); } catch { /* ignore */ }

  const agg = aggregate(records);
  const inconsistencies = detectInconsistencies(agg.palette, agg.type_scale, agg.buttons);

  const profile = {
    source: 'runtime',
    palette: agg.palette,
    type_scale: agg.type_scale,
    spacing: agg.spacing,
    radii: agg.radii,
    shadows: agg.shadows,
    components: { button: agg.buttons },
    inconsistencies,
  };

  process.stdout.write(JSON.stringify(profile, null, 2) + '\n');
}

main().catch(e => {
  process.stderr.write(`${RUNTIME_UNAVAILABLE}: ${e && e.message ? e.message : e}\n`);
  process.exit(3);
});
