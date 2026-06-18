#!/usr/bin/env node
// skills/autospec-harmonize/scripts/design-preview.mjs
//
// Stage 4 — Preview: emit a self-contained HTML gallery from a variants JSON array.
// Screenshots each variant card with Playwright when available; falls back to
// color swatches when AUTOSPEC_HARMONIZE_NO_PLAYWRIGHT=1 or Playwright is absent.
//
// CLI: node design-preview.mjs --variants <arr.json> --out <dir>
// Output: <dir>/preview/index.html  (+ <dir>/preview/<id>.png when Playwright runs)

import fs from 'node:fs';
import path from 'node:path';

const NO_PLAYWRIGHT = process.env.AUTOSPEC_HARMONIZE_NO_PLAYWRIGHT === '1';
const FALLBACK_SIGNAL = 'code_health:harmonize_preview_no_render';

// ---------------------------------------------------------------------------
// CLI arg parsing
// ---------------------------------------------------------------------------
function parseArgs(argv) {
  const args = argv.slice(2);
  const result = { variants: null, out: null };
  for (let i = 0; i < args.length; i++) {
    if (args[i] === '--variants' && args[i + 1]) result.variants = args[++i];
    else if (args[i] === '--out' && args[i + 1]) result.out = args[++i];
  }
  return result;
}

// ---------------------------------------------------------------------------
// HTML helpers
// ---------------------------------------------------------------------------

/** Escape text so it is safe inside HTML attribute values and text nodes. */
function htmlEscape(str) {
  return String(str)
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;');
}

/** Build a CSS custom-properties string from a variant's tokens. */
function tokensToCSS(tokens) {
  const props = [];
  (tokens.palette || []).forEach((entry, i) => {
    const name = entry.role ? `--color-${entry.role}` : `--color-${i}`;
    props.push(`${name}: ${entry.hex}`);
  });
  (tokens.type_scale || []).forEach((t, i) => {
    props.push(`--font-size-${i}: ${t.px}px`);
  });
  (tokens.spacing || []).forEach((s, i) => {
    props.push(`--space-${i}: ${s.px}px`);
  });
  (tokens.radii || []).forEach((r, i) => {
    props.push(`--radius-${i}: ${r.px}px`);
  });
  return props.join('; ');
}

/** Render palette swatches as inline HTML divs (fallback path). */
function renderSwatches(palette) {
  if (!palette || !palette.length) return '';
  const swatches = palette.map(entry => {
    const label = htmlEscape(entry.role || entry.hex);
    const hex = htmlEscape(entry.hex);
    return `<div class="swatch" style="background:${hex};` +
      `width:40px;height:40px;display:inline-block;border-radius:4px;` +
      `margin:2px;" title="${label}"></div>`;
  }).join('\n        ');
  return `<div class="swatches">\n        ${swatches}\n      </div>`;
}

/** Build one <section> for a variant (swatch path). */
function renderVariantSection(variant, imgTag) {
  const cssVars = tokensToCSS(variant.tokens || {});
  const wcag = typeof variant.wcag_min_ratio === 'number'
    ? variant.wcag_min_ratio.toFixed(2)
    : 'n/a';
  const swatchHtml = imgTag || renderSwatches((variant.tokens || {}).palette);
  return `  <section data-variant="${htmlEscape(variant.id)}" style="${htmlEscape(cssVars)}">
    <h2>${htmlEscape(variant.label || variant.id)}</h2>
    <p class="contrast">WCAG ${htmlEscape(wcag)}</p>
    ${swatchHtml}
    <details>
      <summary>Design notes</summary>
      <pre>${htmlEscape(variant.design_md || '')}</pre>
    </details>
  </section>`;
}

/** Wrap sections in a minimal self-contained HTML document. */
function buildHtml(sections) {
  return `<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Design Variant Gallery</title>
  <style>
    body { font-family: system-ui, sans-serif; margin: 2rem; background: #f8f8f8; }
    section { background: #fff; border-radius: 8px; padding: 1.5rem;
              margin-bottom: 1.5rem; box-shadow: 0 1px 4px rgba(0,0,0,.1); }
    h2 { margin: 0 0 .5rem; }
    .contrast { font-size: .85rem; color: #555; margin: .25rem 0 .75rem; }
    .swatch { border: 1px solid #ccc; }
    img { max-width: 100%; border-radius: 4px; margin-top: .5rem; }
    pre { white-space: pre-wrap; font-size: .8rem; }
  </style>
</head>
<body>
  <h1>Design Variant Gallery</h1>
${sections.join('\n')}
</body>
</html>
`;
}

// ---------------------------------------------------------------------------
// Playwright screenshot path
// ---------------------------------------------------------------------------

/** Write a minimal card HTML file for a single variant and screenshot it. */
async function screenshotVariant(chromium, variant, previewDir) {
  const cssVars = tokensToCSS(variant.tokens || {});
  const cardHtml = `<!DOCTYPE html><html><head><meta charset="utf-8">
<style>body{margin:0;padding:1rem;font-family:system-ui,sans-serif;${cssVars}}
.swatch{width:40px;height:40px;display:inline-block;border-radius:4px;margin:2px;border:1px solid #ccc}</style>
</head><body>
<h2>${htmlEscape(variant.label || variant.id)}</h2>
${renderSwatches((variant.tokens || {}).palette)}
</body></html>`;

  const tmpFile = path.join(previewDir, `_card_${variant.id}.html`);
  const pngFile = path.join(previewDir, `${variant.id}.png`);
  fs.writeFileSync(tmpFile, cardHtml, 'utf8');

  let browser = null;
  try {
    browser = await chromium.launch();
    const page = await browser.newPage();
    await page.goto(`file://${tmpFile}`);
    await page.screenshot({ path: pngFile, fullPage: true });
  } finally {
    if (browser) { try { await browser.close(); } catch { /* best-effort */ } }
    try { fs.unlinkSync(tmpFile); } catch { /* best-effort */ }
  }
  return pngFile;
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

async function main() {
  const args = parseArgs(process.argv);

  if (!args.variants || !args.out) {
    process.stderr.write(
      'Usage: node design-preview.mjs --variants <arr.json> --out <dir>\n'
    );
    process.exit(1);
  }

  if (!fs.existsSync(args.variants)) {
    process.stderr.write(`Error: variants file not found: ${args.variants}\n`);
    process.exit(1);
  }

  let variants;
  try {
    variants = JSON.parse(fs.readFileSync(args.variants, 'utf8'));
  } catch (e) {
    process.stderr.write(`Error: could not parse variants JSON: ${e.message}\n`);
    process.exit(1);
  }

  if (!Array.isArray(variants) || variants.length === 0) {
    process.stderr.write('Error: variants must be a non-empty JSON array\n');
    process.exit(1);
  }

  const previewDir = path.join(args.out, 'preview');
  fs.mkdirSync(previewDir, { recursive: true });

  // Determine whether Playwright is available
  let chromium = null;
  if (!NO_PLAYWRIGHT) {
    try {
      const pw = await import('playwright');
      chromium = pw.chromium || (pw.default && pw.default.chromium) || null;
    } catch {
      chromium = null;
    }
  }

  const useFallback = NO_PLAYWRIGHT || !chromium;
  if (useFallback) {
    process.stderr.write(`${FALLBACK_SIGNAL}\n`);
  }

  // Build one section per variant
  const sections = [];
  for (const variant of variants) {
    let imgTag = '';
    if (!useFallback && chromium) {
      try {
        const pngPath = await screenshotVariant(chromium, variant, previewDir);
        const relPath = path.relative(previewDir, pngPath);
        imgTag = `<img src="${htmlEscape(relPath)}" alt="${htmlEscape(variant.id)} screenshot">`;
      } catch {
        imgTag = ''; // fall through to swatches on screenshot failure
      }
    }
    sections.push(renderVariantSection(variant, imgTag));
  }

  const html = buildHtml(sections);
  const outFile = path.join(previewDir, 'index.html');
  fs.writeFileSync(outFile, html, 'utf8');

  process.stdout.write(`Preview written to ${outFile}\n`);
}

main().catch(e => {
  process.stderr.write(`Error: ${e && e.message ? e.message : e}\n`);
  process.exit(1);
});
