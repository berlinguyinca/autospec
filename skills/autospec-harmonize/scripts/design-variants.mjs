#!/usr/bin/env node
// skills/autospec-harmonize/scripts/design-variants.mjs
//
// Stage 3 — Variants: given a baseline variant, emit an array of design
// variants for requested axes. Baseline is always index 0.
//
// CLI: node design-variants.mjs --baseline <file>
//        --axes minimal,high-contrast,dense,bold[,vendor-blend]
//        [--vendor-file <f>]   (bypasses fetch-vendor.sh in tests)
// Stdout: JSON array of variants conforming to
//         schemas/autospec-harmonize-variant.schema.json

import fs from 'node:fs';
import path from 'node:path';
import { execSync, spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

import { hexToRgb, rgbToHsl, hexToHsl, isChromatic, wcagLuminance, contrastRatio, pickBackground, minForegroundContrast } from './lib/color.mjs';

const __dirname = path.dirname(fileURLToPath(import.meta.url));

// ---------------------------------------------------------------------------
// Hex utilities
// ---------------------------------------------------------------------------

/** [r,g,b] in [0,255] → "#rrggbb" */
function rgbToHex(r, g, b) {
  return '#' + [r, g, b].map(v => Math.round(Math.max(0, Math.min(255, v)))
    .toString(16).padStart(2, '0')).join('');
}

/** HSL [h∈[0,360), s∈[0,1], l∈[0,1]] → "#rrggbb" */
function hslToHex(h, s, l) {
  // Standard HSL→RGB conversion
  const c = (1 - Math.abs(2 * l - 1)) * s;
  const x = c * (1 - Math.abs((h / 60) % 2 - 1));
  const m = l - c / 2;
  let r = 0, g = 0, b = 0;
  if (h < 60)       { r = c; g = x; b = 0; }
  else if (h < 120) { r = x; g = c; b = 0; }
  else if (h < 180) { r = 0; g = c; b = x; }
  else if (h < 240) { r = 0; g = x; b = c; }
  else if (h < 300) { r = x; g = 0; b = c; }
  else              { r = c; g = 0; b = x; }
  return rgbToHex((r + m) * 255, (g + m) * 255, (b + m) * 255);
}

/** Deep-clone a JSON-serializable object. */
function clone(obj) {
  return JSON.parse(JSON.stringify(obj));
}

// ---------------------------------------------------------------------------
// CLI argument parsing
// ---------------------------------------------------------------------------

function parseArgs(argv) {
  const args = argv.slice(2);
  const result = { baseline: null, axes: [], vendorFile: null };
  for (let i = 0; i < args.length; i++) {
    if (args[i] === '--baseline' && args[i + 1]) {
      result.baseline = args[++i];
    } else if (args[i] === '--axes' && args[i + 1] !== undefined) {
      const raw = args[++i].trim();
      result.axes = raw ? raw.split(',').map(a => a.trim()).filter(Boolean) : [];
    } else if (args[i] === '--vendor-file' && args[i + 1]) {
      result.vendorFile = args[++i];
    }
  }
  return result;
}

// ---------------------------------------------------------------------------
// Axis transforms
// ---------------------------------------------------------------------------

/** minimal: drop shadows, desaturate accent role. */
function makeMinimal(baseline) {
  const v = clone(baseline);
  v.id    = 'minimal';
  v.label = 'Minimal — reduced decoration';
  v.axis  = 'minimal';

  // Drop shadows
  v.tokens.shadows = [];

  // Desaturate accent palette entry
  v.tokens.palette = v.tokens.palette.map(entry => {
    if (entry.role !== 'accent') return entry;
    const [h, , l] = hexToHsl(entry.hex);
    // Fully desaturate (s→0), keep lightness
    return { ...entry, hex: hslToHex(h, 0, l) };
  });

  // Desaturating the accent changes its luminance, so recompute the contrast
  // annotation from THIS variant's palette — never inherit the baseline's stale
  // wcag_min_ratio (Phase 5.5 audit #1147 finding F1).
  v.wcag_min_ratio = minForegroundContrast(v.tokens.palette);
  v.design_md = '# Minimal Variant\n\nShadows removed; accent desaturated for a clean, decoration-free aesthetic.';
  return v;
}

/** high-contrast: push dark colors darker, light colors lighter; recompute wcag_min_ratio. */
function makeHighContrast(baseline) {
  const v = clone(baseline);
  v.id    = 'high-contrast';
  v.label = 'High-Contrast — WCAG-AA+ accessibility';
  v.axis  = 'high-contrast';

  // Contrast is foreground-vs-background. Push the background further toward
  // its own extreme, and push every foreground color AWAY from the background
  // luminance (light bg → darken foregrounds; dark bg → lighten them). This
  // raises each foreground-vs-bg contrast monotonically.
  const bg = pickBackground(v.tokens.palette);
  const bgLight = bg ? wcagLuminance(bg.hex) >= 0.5 : true;
  v.tokens.palette = v.tokens.palette.map(entry => {
    const [h, s, l] = hexToHsl(entry.hex);
    let newL;
    if (entry === bg) {
      newL = bgLight ? Math.min(1, l + 0.15) : Math.max(0, l - 0.15);
    } else {
      newL = bgLight ? Math.max(0, l - 0.20) : Math.min(1, l + 0.20);
    }
    return { ...entry, hex: hslToHex(h, s, newL) };
  });

  v.wcag_min_ratio = minForegroundContrast(v.tokens.palette);
  v.design_md = '# High-Contrast Variant\n\nPalette colors pushed apart for WCAG-AA+ contrast.';
  return v;
}

/** dense: multiply spacing & radii px by 0.75. */
function makeDense(baseline) {
  const v = clone(baseline);
  v.id    = 'dense';
  v.label = 'Dense — compact spacing';
  v.axis  = 'dense';

  v.tokens.spacing = v.tokens.spacing.map(s => ({ ...s, px: s.px * 0.75 }));
  v.tokens.radii   = v.tokens.radii.map(r => ({ ...r, px: r.px * 0.75 }));

  v.design_md = '# Dense Variant\n\nSpacing and radii reduced to 75% for compact layouts.';
  return v;
}

/** bold: heavier font weights + bump type_scale by ~2px. */
function makeBold(baseline) {
  const v = clone(baseline);
  v.id    = 'bold';
  v.label = 'Bold — strong typographic presence';
  v.axis  = 'bold';

  v.tokens.type_scale = v.tokens.type_scale.map(t => {
    const bumped = { ...t };
    if (typeof bumped.px === 'number') bumped.px = bumped.px + 2;
    // Add/increase font weight
    bumped.weight = typeof bumped.weight === 'number'
      ? Math.min(900, bumped.weight + 100)
      : 700;
    return bumped;
  });

  v.design_md = '# Bold Variant\n\nType scale bumped up 2px per step; weights increased for strong typographic presence.';
  return v;
}

/** vendor-blend: channel-wise 50% lerp between baseline hex and vendor hex. */
function blendHex(baseHex, vendorHex) {
  const [br, bg, bb] = hexToRgb(baseHex);
  const [vr, vg, vb] = hexToRgb(vendorHex);
  // Midpoint — strictly between when channels differ
  const r = Math.round((br + vr) / 2);
  const g = Math.round((bg + vg) / 2);
  const b = Math.round((bb + vb) / 2);
  return rgbToHex(r, g, b);
}

function makeVendorBlend(baseline, vendorPalette) {
  const v = clone(baseline);
  v.id     = 'vendor-blend';
  v.label  = 'Vendor-Blend — 50% palette merge with vendor brand';
  v.axis   = 'vendor-blend';
  v.vendor = 'vendor';

  // Build a map from role → vendor hex (prefer role match, else index match)
  const vendorByRole = {};
  for (const entry of vendorPalette) {
    if (entry.role) vendorByRole[entry.role] = entry.hex;
  }

  v.tokens.palette = v.tokens.palette.map((entry, idx) => {
    // Find vendor counterpart: by role first, then by index
    const vendorHex = entry.role && vendorByRole[entry.role]
      ? vendorByRole[entry.role]
      : vendorPalette[idx]?.hex ?? null;

    if (!vendorHex) return entry;
    return { ...entry, hex: blendHex(entry.hex, vendorHex) };
  });

  v.design_md = '# Vendor-Blend Variant\n\nPalette is a 50% channel-wise blend between the baseline and vendor brand colors.';
  return v;
}

// ---------------------------------------------------------------------------
// Vendor fetch
// ---------------------------------------------------------------------------

function fetchVendor(vendorFile, scriptDir) {
  // Test mode: vendor file provided directly
  if (vendorFile) {
    if (!fs.existsSync(vendorFile)) return null;
    try {
      const data = JSON.parse(fs.readFileSync(vendorFile, 'utf8'));
      return data.palette ?? null;
    } catch {
      return null;
    }
  }

  // Normal mode: call fetch-vendor.sh
  const fetchScript = path.join(scriptDir, 'fetch-vendor.sh');
  if (!fs.existsSync(fetchScript)) return null;

  const tmpOut = `/tmp/autospec-vendor-${Date.now()}.json`;
  try {
    const result = spawnSync(
      'bash',
      [fetchScript, '--vendor', 'default', '--out', tmpOut],
      { stdio: ['ignore', 'pipe', 'pipe'], timeout: 30000 }
    );
    if (result.status !== 0) return null;
    if (!fs.existsSync(tmpOut)) return null;
    const data = JSON.parse(fs.readFileSync(tmpOut, 'utf8'));
    fs.unlinkSync(tmpOut);
    return data.palette ?? null;
  } catch {
    if (fs.existsSync(tmpOut)) {
      try { fs.unlinkSync(tmpOut); } catch { /* ignore */ }
    }
    return null;
  }
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

/** Directional axes that are a pure function of the baseline (no I/O). */
const SIMPLE_AXES = {
  minimal: makeMinimal,
  'high-contrast': makeHighContrast,
  dense: makeDense,
  bold: makeBold,
};

/** Read + parse the baseline variant file; exit 1 on missing/invalid input. */
function loadBaseline(file) {
  if (!fs.existsSync(file)) {
    process.stderr.write(`Error: baseline file not found: ${file}\n`);
    process.exit(1);
  }
  try {
    return JSON.parse(fs.readFileSync(file, 'utf8'));
  } catch (e) {
    process.stderr.write(`Error: could not parse baseline JSON: ${e.message}\n`);
    process.exit(1);
  }
}

/** Build the variant array (baseline at index 0) for the requested axes. */
function buildVariants(baseline, axes, vendorFile) {
  const variants = [baseline];
  for (const axis of axes) {
    if (SIMPLE_AXES[axis]) {
      variants.push(SIMPLE_AXES[axis](baseline));
    } else if (axis === 'vendor-blend') {
      const vendorPalette = fetchVendor(vendorFile, __dirname);
      if (!vendorPalette) {
        // Drop only vendor-blend; other axes survive.
        process.stderr.write('code_health:harmonize_vendor_fetch_failed\n');
        continue;
      }
      variants.push(makeVendorBlend(baseline, vendorPalette));
    } else {
      process.stderr.write(`Warning: unknown axis "${axis}" — skipped\n`);
    }
  }
  return variants;
}

/** Self-validate variants against the schema using ajv CLI if available.
 *  Exits 1 with an error to stderr if any variant is invalid.
 *  Skips silently when ajv is not on PATH (no hard dependency).
 */
function selfValidateVariants(variants, schemaPath) {
  // Check if ajv is available — skip silently if not
  const ajvCheck = spawnSync('ajv', ['--version'], { stdio: 'pipe' });
  if (ajvCheck.status !== 0 || ajvCheck.error) return; // ajv not available — skip

  for (let i = 0; i < variants.length; i++) {
    const tmpFile = `/tmp/autospec-variants-validate-${Date.now()}-${i}.json`;
    try {
      fs.writeFileSync(tmpFile, JSON.stringify(variants[i]));
      const result = spawnSync(
        'ajv',
        ['validate', '-s', schemaPath, '--spec=draft2020', '-d', tmpFile],
        { stdio: 'pipe' }
      );
      try { fs.unlinkSync(tmpFile); } catch { /* ignore */ }
      if (result.status !== 0) {
        const stderr = result.stderr ? result.stderr.toString() : '';
        process.stderr.write(`Error: variant[${i}] (id="${variants[i].id}") failed schema validation:\n${stderr}\n`);
        process.exit(1);
      }
    } catch (e) {
      try { fs.unlinkSync(tmpFile); } catch { /* ignore */ }
      process.stderr.write(`Error: schema validation error for variant[${i}]: ${e.message}\n`);
      process.exit(1);
    }
  }
}

function main() {
  const args = parseArgs(process.argv);
  if (!args.baseline) {
    process.stderr.write('Usage: node design-variants.mjs --baseline <file> --axes <axes> [--vendor-file <f>]\n');
    process.exit(1);
  }
  const baseline = loadBaseline(args.baseline);
  // Recompute baseline wcag_min_ratio with the same foreground-vs-bg metric the
  // variants use, so comparisons are apples-to-apples regardless of what value
  // the upstream baseline carried.
  baseline.wcag_min_ratio = minForegroundContrast(baseline.tokens.palette);
  const variants = buildVariants(baseline, args.axes, args.vendorFile);

  // F5: self-validate each variant against the schema (skips when ajv absent)
  const schemaPath = path.resolve(__dirname, '../../../schemas/autospec-harmonize-variant.schema.json');
  selfValidateVariants(variants, schemaPath);

  process.stdout.write(JSON.stringify(variants, null, 2) + '\n');
}

main();
