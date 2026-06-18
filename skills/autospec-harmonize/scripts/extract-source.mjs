#!/usr/bin/env node
// skills/autospec-harmonize/scripts/extract-source.mjs
//
// Static CSS/SCSS token-profile extractor for autospec-harmonize Stage 1.
//
// CLI: node extract-source.mjs --root <dir>
// Stdout: JSON token profile conforming to
//         schemas/autospec-harmonize-token-profile.schema.json
//
// Walks --root recursively (skips node_modules/.git), concats *.css/*.scss,
// regex-collects design tokens, dedupes, and emits the schema-required shape.

import fs from 'node:fs';
import path from 'node:path';

// ---------------------------------------------------------------------------
// CLI arg parsing
// ---------------------------------------------------------------------------
function parseArgs(argv) {
  const args = argv.slice(2);
  const result = { root: null };
  for (let i = 0; i < args.length; i++) {
    if (args[i] === '--root' && args[i + 1]) {
      result.root = args[++i];
    }
  }
  return result;
}

// ---------------------------------------------------------------------------
// File walking
// ---------------------------------------------------------------------------
const SKIP_DIRS = new Set(['node_modules', '.git', '.svn', '.hg']);

function walkSync(dir) {
  const files = [];
  let entries;
  try {
    entries = fs.readdirSync(dir, { withFileTypes: true });
  } catch {
    return files;
  }
  for (const entry of entries) {
    if (entry.isDirectory()) {
      if (!SKIP_DIRS.has(entry.name)) {
        files.push(...walkSync(path.join(dir, entry.name)));
      }
    } else if (/\.(css|scss|html|htm)$/.test(entry.name)) {
      files.push(path.join(dir, entry.name));
    }
  }
  return files;
}

/**
 * Given the text of an HTML file, extract the contents of all
 * <style>...</style> blocks and concatenate them.
 */
function extractStyleBlocks(html) {
  const blocks = [];
  const re = /<style[^>]*>([\s\S]*?)<\/style>/gi;
  let m;
  while ((m = re.exec(html)) !== null) {
    blocks.push(m[1]);
  }
  return blocks.join('\n');
}

function readAllCSS(root) {
  const files = walkSync(root);
  return files.map(f => {
    let text;
    try { text = fs.readFileSync(f, 'utf8'); } catch { return ''; }
    // For HTML/HTM files, extract only the <style> block contents so that
    // the CSS token regexes don't match stray hex-like strings in markup.
    if (/\.html?$/.test(f)) {
      return extractStyleBlocks(text);
    }
    return text;
  }).join('\n');
}

// ---------------------------------------------------------------------------
// Token extraction helpers
// ---------------------------------------------------------------------------

/** Extract all 6-digit hex colors, lowercased, deduped with occurrence count. */
function extractPalette(css) {
  const counts = new Map();
  const re = /#([0-9a-fA-F]{6})\b/g;
  let m;
  while ((m = re.exec(css)) !== null) {
    const hex = '#' + m[1].toLowerCase();
    counts.set(hex, (counts.get(hex) ?? 0) + 1);
  }
  // Sort by count desc, then hex asc
  return Array.from(counts.entries())
    .sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))
    .map(([hex, count]) => ({ hex, count }));
}

/** Extract font-size px values → sorted unique array of {px} objects. */
function extractTypeScale(css) {
  const vals = new Set();
  const re = /font-size\s*:\s*(\d+(?:\.\d+)?)px/gi;
  let m;
  while ((m = re.exec(css)) !== null) vals.add(Number(m[1]));
  return Array.from(vals).sort((a, b) => a - b).map(px => ({ px }));
}

/** Extract padding|gap|margin px values → sorted unique {px} objects. */
function extractSpacing(css) {
  const vals = new Set();
  const re = /(?:padding|gap|margin)\s*:\s*(\d+(?:\.\d+)?)px/gi;
  let m;
  while ((m = re.exec(css)) !== null) vals.add(Number(m[1]));
  return Array.from(vals).sort((a, b) => a - b).map(px => ({ px }));
}

/** Extract border-radius px values → sorted unique {px} objects. */
function extractRadii(css) {
  const vals = new Set();
  // border-radius may be a multi-value shorthand (e.g. "4px 8px 12px 16px");
  // capture every px corner value, matching the runtime extractor's per-corner
  // read (Phase 5.5 audit #1147 finding F4).
  const re = /border-radius\s*:\s*([^;}{]+)/gi;
  let m;
  while ((m = re.exec(css)) !== null) {
    const pxRe = /(\d+(?:\.\d+)?)px/g;
    let p;
    while ((p = pxRe.exec(m[1])) !== null) vals.add(Number(p[1]));
  }
  return Array.from(vals).sort((a, b) => a - b).map(px => ({ px }));
}

/** Extract box-shadow values → deduped {value} objects. */
function extractShadows(css) {
  const seen = new Set();
  const re = /box-shadow\s*:\s*([^;}{]+)/gi;
  let m;
  while ((m = re.exec(css)) !== null) {
    const val = m[1].trim();
    if (val) seen.add(val);
  }
  return Array.from(seen).map(value => ({ value }));
}

/**
 * Collect button-family selectors (.btn*, .button*, .cta*) and their
 * rule bodies (first 200 chars each) into an array of descriptor strings.
 */
function extractButtonComponents(css) {
  const entries = [];
  const re = /(\.[a-zA-Z][a-zA-Z0-9_-]*)\s*\{([^}]*)\}/gs;
  let m;
  while ((m = re.exec(css)) !== null) {
    const sel = m[1];
    if (/\.(btn|button|cta)/i.test(sel)) {
      entries.push({ selector: sel, rules: m[2].trim().slice(0, 200) });
    }
  }
  return entries;
}

// ---------------------------------------------------------------------------
// Inconsistency detection
// ---------------------------------------------------------------------------
function detectInconsistencies(palette, typeScale, buttons) {
  const result = [];

  if (palette.length > 3) {
    result.push({
      category: 'palette',
      detail: `${palette.length} colors found; consider consolidating to a smaller set`,
    });
  }

  if (typeScale.length > 4) {
    result.push({
      category: 'type_scale',
      detail: `${typeScale.length} distinct font sizes found`,
    });
  }

  if (buttons.length > 1) {
    result.push({
      category: 'components',
      detail: `${buttons.length} button treatments found (.btn/.button/.cta selectors)`,
    });
  }

  return result;
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------
function main() {
  const args = parseArgs(process.argv);

  if (!args.root) {
    process.stderr.write('Usage: node extract-source.mjs --root <dir>\n');
    process.exit(1);
  }

  if (!fs.existsSync(args.root)) {
    process.stderr.write(`Error: root directory not found: ${args.root}\n`);
    process.exit(1);
  }

  const css = readAllCSS(args.root);

  if (!css.trim()) {
    process.stderr.write('Warning: no CSS/SCSS source found under ' + args.root + '\n');
  }

  const palette        = extractPalette(css);
  const typeScale      = extractTypeScale(css);
  const spacing        = extractSpacing(css);
  const radii          = extractRadii(css);
  const shadows        = extractShadows(css);
  const buttons        = extractButtonComponents(css);
  const inconsistencies = detectInconsistencies(palette, typeScale, buttons);

  const profile = {
    source: 'source',
    palette,
    type_scale: typeScale,
    spacing,
    radii,
    shadows,
    components: { button: buttons },
    inconsistencies,
  };

  process.stdout.write(JSON.stringify(profile, null, 2) + '\n');
}

main();
