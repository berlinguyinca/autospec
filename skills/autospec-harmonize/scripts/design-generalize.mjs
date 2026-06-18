#!/usr/bin/env node
// skills/autospec-harmonize/scripts/design-generalize.mjs
//
// Stage 2 — Generalize: collapse a token profile into ONE id:"baseline" design
// variant conforming to schemas/autospec-harmonize-variant.schema.json.
//
// CLI: node design-generalize.mjs --tokens <profile.json>
// Stdout: one variant JSON with id:"baseline", axis:"baseline"
//
// LLM path (Tier-A):
//   Attempted only when AUTOSPEC_HARMONIZE_LLM_STUB is NOT set AND
//   AUTOSPEC_HARMONIZE_LLM_CMD is configured (points to a CLI that accepts
//   a prompt on stdin and returns JSON on stdout). If the LLM output fails
//   schema validation twice, the deterministic path runs instead.
//
// Deterministic path (always runs when AUTOSPEC_HARMONIZE_LLM_STUB=1 OR
//   no LLM is configured OR LLM output fails validation twice):
//   1. Cluster palette hexes by hue proximity (~25° HSL window).
//   2. Pick the highest-count hex per cluster as the cluster representative.
//   3. Assign roles: lightest luminance→bg, darkest luminance→text,
//      highest-count chromatic cluster→primary, next chromatic→accent, rest unnamed.
//   4. Compute median representative sets for type_scale, spacing, radii.
//   5. Pass through up to 3 representative shadows.
//   6. Build a short design_md summary.

import fs from 'node:fs';
import path from 'node:path';
import { execSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));

// ---------------------------------------------------------------------------
// CLI arg parsing
// ---------------------------------------------------------------------------
function parseArgs(argv) {
  const args = argv.slice(2);
  const result = { tokens: null };
  for (let i = 0; i < args.length; i++) {
    if (args[i] === '--tokens' && args[i + 1]) {
      result.tokens = args[++i];
    }
  }
  return result;
}

// ---------------------------------------------------------------------------
// Hex ↔ HSL conversion
// ---------------------------------------------------------------------------

/** Parse "#rrggbb" → [r, g, b] in [0,255]. */
function hexToRgb(hex) {
  const h = hex.replace('#', '');
  return [
    parseInt(h.slice(0, 2), 16),
    parseInt(h.slice(2, 4), 16),
    parseInt(h.slice(4, 6), 16),
  ];
}

/** [r,g,b] in [0,255] → [h,s,l] where h∈[0,360), s∈[0,1], l∈[0,1]. */
function rgbToHsl(r, g, b) {
  r /= 255; g /= 255; b /= 255;
  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  const l = (max + min) / 2;
  if (max === min) return [0, 0, l]; // achromatic
  const d = max - min;
  const s = l > 0.5 ? d / (2 - max - min) : d / (max + min);
  let h;
  switch (max) {
    case r: h = ((g - b) / d + (g < b ? 6 : 0)) / 6; break;
    case g: h = ((b - r) / d + 2) / 6; break;
    default: h = ((r - g) / d + 4) / 6; break;
  }
  return [h * 360, s, l];
}

/** Return [h, s, l] for a "#rrggbb" hex string. */
function hexToHsl(hex) {
  return rgbToHsl(...hexToRgb(hex));
}

/** Luminance proxy: l component from HSL (0=dark, 1=light). */
function luminance(hex) {
  return hexToHsl(hex)[2];
}

/** Is this hex "chromatic" (saturation >= 0.15)? */
function isChromatic(hex) {
  const [, s] = hexToHsl(hex);
  return s >= 0.15;
}

/** Circular hue distance in degrees. */
function hueDist(a, b) {
  const d = Math.abs(a - b) % 360;
  return d > 180 ? 360 - d : d;
}

// ---------------------------------------------------------------------------
// Palette clustering (hue-based, ~25° window)
// ---------------------------------------------------------------------------

/** Cluster palette entries by hue proximity (~25° HSL window).
 *  Achromatic colors each get their own cluster.
 *  Returns array of cluster arrays (each is array of {hex, count}).
 */
function clusterByHue(palette, hueThreshold = 25) {
  const clusters = [];

  for (const entry of palette) {
    const [h, s] = hexToHsl(entry.hex);
    const chromatic = s >= 0.15;

    if (!chromatic) {
      // Achromatic → own cluster
      clusters.push([entry]);
      continue;
    }

    // Find existing chromatic cluster within threshold
    let assigned = false;
    for (const cluster of clusters) {
      const repEntry = cluster[0];
      const [repH, repS] = hexToHsl(repEntry.hex);
      if (repS < 0.15) continue; // skip achromatic clusters
      if (hueDist(h, repH) <= hueThreshold) {
        cluster.push(entry);
        assigned = true;
        break;
      }
    }
    if (!assigned) clusters.push([entry]);
  }

  return clusters;
}

/** Pick the highest-count entry from a cluster; break ties by hex asc. */
function clusterRep(cluster) {
  return cluster.reduce((best, e) => {
    const bc = best.count ?? 0;
    const ec = e.count ?? 0;
    if (ec > bc) return e;
    if (ec === bc && e.hex < best.hex) return e;
    return best;
  });
}

// ---------------------------------------------------------------------------
// Role assignment
// ---------------------------------------------------------------------------

/**
 * Given cluster representatives, assign roles:
 * - EXACTLY ONE gets role "primary" (the highest-count chromatic rep).
 * - Lightest overall → "bg"
 * - Darkest overall → "text"
 * - Next-most-frequent chromatic → "accent"
 * - Others get no role.
 *
 * Returns array of {hex, role?, count?} objects (no extra keys).
 */
function assignRoles(reps) {
  if (reps.length === 0) return [];

  const withLum = reps.map(r => ({ ...r, _lum: luminance(r.hex) }));

  // Sort chromatic reps by count desc to find primary/accent
  const chromatics = withLum
    .filter(r => isChromatic(r.hex))
    .sort((a, b) => (b.count ?? 0) - (a.count ?? 0) || a.hex.localeCompare(b.hex));

  // Lightest → bg, Darkest → text (by luminance)
  const sorted = [...withLum].sort((a, b) => b._lum - a._lum);
  const lightestHex = sorted[0]?.hex;
  const darkestHex = sorted[sorted.length - 1]?.hex;

  // Assign
  const primaryHex = chromatics[0]?.hex ?? null;
  const accentHex  = chromatics[1]?.hex ?? null;

  const result = [];
  for (const r of reps) {
    const item = { hex: r.hex };
    if (r.count !== undefined) item.count = r.count;

    if (r.hex === primaryHex) {
      item.role = 'primary';
    } else if (r.hex === lightestHex && r.hex !== primaryHex) {
      item.role = 'bg';
    } else if (r.hex === darkestHex && r.hex !== primaryHex) {
      item.role = 'text';
    } else if (r.hex === accentHex) {
      item.role = 'accent';
    }
    // else: no role

    result.push(item);
  }

  // Safety: if no chromatic hex found (all achromatic), assign primary to
  // the most-frequent entry so there's always exactly one.
  const hasPrimary = result.some(r => r.role === 'primary');
  if (!hasPrimary && result.length > 0) {
    const mostFreq = [...reps].sort((a, b) => (b.count ?? 0) - (a.count ?? 0))[0];
    for (const item of result) {
      if (item.hex === mostFreq.hex) {
        item.role = 'primary';
        break;
      }
    }
  }

  return result;
}

// ---------------------------------------------------------------------------
// Numeric scale helpers
// ---------------------------------------------------------------------------

/** Compute median of a numeric array. */
function median(values) {
  if (values.length === 0) return null;
  const s = [...values].sort((a, b) => a - b);
  const mid = Math.floor(s.length / 2);
  return s.length % 2 === 0 ? (s[mid - 1] + s[mid]) / 2 : s[mid];
}

/**
 * Reduce a sorted array of {px} objects to a representative subset using
 * a simple dedup + median-anchored selection. Returns sorted {px} objects.
 * If ≤5 entries, return all. Else: min, 25th-pct, median, 75th-pct, max.
 */
function representativeScale(pxItems) {
  if (!pxItems || pxItems.length === 0) return [];
  const vals = pxItems.map(i => i.px).sort((a, b) => a - b);
  const deduped = [...new Set(vals)];
  if (deduped.length <= 5) return deduped.map(px => ({ px }));

  const pick = (arr, fraction) => arr[Math.round(fraction * (arr.length - 1))];
  const selected = new Set([
    pick(deduped, 0),
    pick(deduped, 0.25),
    pick(deduped, 0.5),
    pick(deduped, 0.75),
    pick(deduped, 1),
  ]);
  return [...selected].sort((a, b) => a - b).map(px => ({ px }));
}

// ---------------------------------------------------------------------------
// Deterministic collapse
// ---------------------------------------------------------------------------

function deterministicCollapse(profile) {
  const rawPalette = profile.palette ?? [];

  // 1. Cluster by hue
  const clusters = clusterByHue(rawPalette);

  // 2. Pick one rep per cluster
  const reps = clusters.map(clusterRep);

  // 3. Assign roles (guarantees exactly one "primary")
  const palette = assignRoles(reps);

  // 4. Representative scales
  const type_scale = representativeScale(profile.type_scale ?? []);
  const spacing    = representativeScale(profile.spacing ?? []);
  const radii      = representativeScale(profile.radii ?? []);

  // 5. Shadows: pass through up to 3 representative entries
  const shadows = (profile.shadows ?? []).slice(0, 3);

  // 6. Build design_md
  const primaryEntry = palette.find(p => p.role === 'primary');
  const paletteCount = palette.length;
  const typeCount    = type_scale.length;
  const spacingCount = spacing.length;

  const design_md = [
    '# Baseline Design System',
    '',
    'This baseline collapses the discovered token profile into a single coherent',
    'design system with minimal visual change — consistency first.',
    '',
    '## Palette',
    `${paletteCount} canonical color${paletteCount !== 1 ? 's' : ''} derived from hue clustering.`,
    primaryEntry ? `Primary: \`${primaryEntry.hex}\`` : '',
    '',
    '## Typography',
    `${typeCount} representative font size${typeCount !== 1 ? 's' : ''}.`,
    '',
    '## Spacing',
    `${spacingCount} spacing step${spacingCount !== 1 ? 's' : ''}.`,
    '',
    '## Radii',
    `${radii.length} border-radius value${radii.length !== 1 ? 's' : ''}.`,
    '',
    '## Shadows',
    `${shadows.length} shadow definition${shadows.length !== 1 ? 's' : ''}.`,
  ].filter(l => l !== null).join('\n');

  return {
    id: 'baseline',
    label: 'Baseline — faithful consolidation',
    axis: 'baseline',
    tokens: { palette, type_scale, spacing, radii, shadows },
    design_md,
  };
}

// ---------------------------------------------------------------------------
// Schema validation (inline AJV via Node)
// ---------------------------------------------------------------------------

/** Locate the repo root relative to this script. */
function repoRoot() {
  // scripts/ → autospec-harmonize/ → skills/ → repo root
  return path.resolve(__dirname, '../../..');
}

/**
 * Validate `obj` against the variant schema using ajv CLI.
 * Returns true if valid, false otherwise.
 */
function validateVariant(obj, schemaPath) {
  try {
    const tmpFile = `/tmp/autospec-gen-validate-${Date.now()}.json`;
    fs.writeFileSync(tmpFile, JSON.stringify(obj));
    execSync(`ajv validate -s "${schemaPath}" --spec=draft2020 -d "${tmpFile}"`, {
      stdio: 'pipe',
    });
    fs.unlinkSync(tmpFile);
    return true;
  } catch {
    return false;
  }
}

// ---------------------------------------------------------------------------
// LLM path (Tier-A)
// ---------------------------------------------------------------------------

/**
 * Attempt LLM synthesis. Returns a validated variant object or null.
 * Gated behind: AUTOSPEC_HARMONIZE_LLM_STUB not set AND
 *               AUTOSPEC_HARMONIZE_LLM_CMD is set.
 */
function tryLlmPath(profile, schemaPath) {
  const llmCmd = process.env.AUTOSPEC_HARMONIZE_LLM_CMD;
  if (!llmCmd) return null;

  const prompt = [
    'You are a design-system assistant. Given the token profile below, produce',
    'ONE baseline design variant as JSON matching this schema: ' + schemaPath,
    'Requirements: id must be "baseline", axis must be "baseline",',
    'exactly one palette entry must have role "primary".',
    'Respond with ONLY valid JSON, no markdown fences.',
    '',
    'TOKEN PROFILE:',
    JSON.stringify(profile, null, 2),
  ].join('\n');

  for (let attempt = 0; attempt < 2; attempt++) {
    try {
      const raw = execSync(llmCmd, {
        input: prompt,
        stdio: ['pipe', 'pipe', 'pipe'],
        encoding: 'utf8',
        timeout: 60000,
      });
      const obj = JSON.parse(raw.trim());
      if (validateVariant(obj, schemaPath)) return obj;
    } catch {
      // fall through to retry / deterministic
    }
  }
  return null;
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------
function main() {
  const args = parseArgs(process.argv);

  if (!args.tokens) {
    process.stderr.write('Usage: node design-generalize.mjs --tokens <profile.json>\n');
    process.exit(1);
  }

  if (!fs.existsSync(args.tokens)) {
    process.stderr.write(`Error: token profile not found: ${args.tokens}\n`);
    process.exit(1);
  }

  let profile;
  try {
    profile = JSON.parse(fs.readFileSync(args.tokens, 'utf8'));
  } catch (e) {
    process.stderr.write(`Error: could not parse profile JSON: ${e.message}\n`);
    process.exit(1);
  }

  const schemaPath = path.join(repoRoot(), 'schemas', 'autospec-harmonize-variant.schema.json');
  const useStub = process.env.AUTOSPEC_HARMONIZE_LLM_STUB === '1';

  let variant = null;

  // LLM path: only when not stubbed and LLM_CMD configured
  if (!useStub) {
    variant = tryLlmPath(profile, schemaPath);
    if (variant) {
      process.stderr.write('design-generalize: used LLM path\n');
    }
  }

  // Deterministic fallback (always used when stub=1 or LLM unavailable/failed)
  if (!variant) {
    if (!useStub) {
      process.stderr.write('design-generalize: using deterministic fallback\n');
    }
    variant = deterministicCollapse(profile);
  }

  process.stdout.write(JSON.stringify(variant, null, 2) + '\n');
}

main();
