#!/usr/bin/env node
// skills/autospec-harmonize/scripts/gen-migration-spec.mjs
//
// Stage 5 — Migration spec: render a dated migration design spec from a
// chosen variant + inventory, with one `- [ ]` checkbox per diverging
// page/component + inconsistency category, a `## UX findings` section
// lifted from inventory.md, and print the `/autospec-define` handoff.
//
// CLI:
//   node gen-migration-spec.mjs \
//     --variant <id> \
//     --tokens <profile-or-variants.json> \
//     --inventory <inventory.md> \
//     --date <YYYY-MM-DD> \
//     --slug <slug> \
//     --out <dir>
//
// Writes: <out>/docs/specs/<date>-harmonize-<slug>-design.md
// Stdout: /autospec-define <path>

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

// ---------------------------------------------------------------------------
// CLI arg parsing
// ---------------------------------------------------------------------------

function parseArgs(argv) {
  const args = argv.slice(2);
  const result = {
    variant: null,
    tokens: null,
    inventory: null,
    date: null,
    slug: null,
    out: null,
  };
  for (let i = 0; i < args.length; i++) {
    switch (args[i]) {
      case '--variant':   result.variant   = args[++i]; break;
      case '--tokens':    result.tokens    = args[++i]; break;
      case '--inventory': result.inventory = args[++i]; break;
      case '--date':      result.date      = args[++i]; break;
      case '--slug':      result.slug      = args[++i]; break;
      case '--out':       result.out       = args[++i]; break;
    }
  }
  return result;
}

function validateArgs(args) {
  const required = ['variant', 'tokens', 'inventory', 'date', 'slug', 'out'];
  for (const k of required) {
    if (!args[k]) {
      process.stderr.write(
        `Error: --${k} is required\n` +
        'Usage: node gen-migration-spec.mjs --variant <id> --tokens <file> ' +
        '--inventory <file> --date <YYYY-MM-DD> --slug <slug> --out <dir>\n'
      );
      process.exit(1);
    }
  }
}

// ---------------------------------------------------------------------------
// Load variant from variants array JSON or single-variant JSON
// ---------------------------------------------------------------------------

function loadVariant(tokensFile, variantId) {
  if (!fs.existsSync(tokensFile)) {
    process.stderr.write(`Error: tokens file not found: ${tokensFile}\n`);
    process.exit(1);
  }
  let data;
  try {
    data = JSON.parse(fs.readFileSync(tokensFile, 'utf8'));
  } catch (e) {
    process.stderr.write(`Error: could not parse tokens JSON: ${e.message}\n`);
    process.exit(1);
  }
  // Support both array-of-variants and single variant object
  if (Array.isArray(data)) {
    const found = data.find(v => v.id === variantId);
    if (!found) {
      process.stderr.write(
        `Error: variant "${variantId}" not found in ${tokensFile}\n` +
        `Available: ${data.map(v => v.id).join(', ')}\n`
      );
      process.exit(1);
    }
    return found;
  }
  // Single variant — validate id matches
  if (data.id && data.id !== variantId) {
    process.stderr.write(
      `Warning: tokens file variant id "${data.id}" does not match requested "${variantId}"\n`
    );
  }
  return data;
}

// ---------------------------------------------------------------------------
// Parse inventory.md — extract components and inconsistencies
// ---------------------------------------------------------------------------

function parseInventory(inventoryFile) {
  if (!fs.existsSync(inventoryFile)) {
    process.stderr.write(`Error: inventory file not found: ${inventoryFile}\n`);
    process.exit(1);
  }
  const text = fs.readFileSync(inventoryFile, 'utf8');
  const lines = text.split('\n');

  const components = [];
  const inconsistencies = [];
  const uxFindings = [];

  let section = null;
  for (const line of lines) {
    if (/^## Inconsistencies/i.test(line)) { section = 'inconsistencies'; continue; }
    if (/^## Components/i.test(line))      { section = 'components';      continue; }
    if (/^## /i.test(line))                { section = null;              continue; }

    if (section === 'inconsistencies') {
      // Lines like: - **category**: detail
      const m = line.match(/^-\s+\*\*([^*]+)\*\*:\s+(.+)/);
      if (m) {
        inconsistencies.push({ category: m[1].trim(), detail: m[2].trim() });
        uxFindings.push(`- **${m[1].trim()}**: ${m[2].trim()}`);
      } else if (/^-\s+\S/.test(line)) {
        // Plain bullet inconsistency line
        uxFindings.push(line);
        inconsistencies.push({ category: line.replace(/^-\s+/, '').trim(), detail: '' });
      }
    }

    if (section === 'components') {
      const m = line.match(/^-\s+(\S+)/);
      if (m) {
        components.push(m[1].trim());
      }
    }
  }

  return { components, inconsistencies, uxFindings };
}

// ---------------------------------------------------------------------------
// Build spec markdown — broken into focused section helpers
// ---------------------------------------------------------------------------

function buildHeader(args, variant) {
  return [
    `# Harmonize Migration Spec — ${args.slug}`,
    '',
    `**Date**: ${args.date}`,
    `**Variant**: ${variant.id} — ${variant.label || variant.id}`,
    `**Slug**: ${args.slug}`,
    '',
    '## Summary',
    '',
    `This migration spec captures the per-component and per-inconsistency ` +
    `acceptance tasks required to harmonize the design system using the ` +
    `\`${variant.id}\` variant. Each checkbox below maps to one diverging ` +
    `group that must be resolved before the system can be considered unified.`,
    '',
  ];
}

function buildChecklist(variant, components, inconsistencies) {
  const lines = ['## Acceptance checklist', '', '### Component groups', ''];
  for (const comp of components) {
    lines.push(`- [ ] Migrate \`${comp}\` to ${variant.id} variant tokens`);
  }
  lines.push('', '### Inconsistency categories', '');
  for (const inc of inconsistencies) {
    const label = inc.detail
      ? `Resolve ${inc.category}: ${inc.detail}`
      : `Resolve ${inc.category}`;
    lines.push(`- [ ] ${label}`);
  }
  lines.push('');
  return lines;
}

function buildUxFindings(uxFindings) {
  const lines = ['## UX findings', ''];
  if (uxFindings.length > 0) {
    lines.push(...uxFindings);
  } else {
    lines.push('No UX drift findings recorded in inventory.');
  }
  lines.push('');
  return lines;
}

function buildTokenTable(variant) {
  const tok = variant.tokens || {};
  return [
    '## Variant tokens',
    '',
    '| Category | Count |',
    '|---|---|',
    `| palette | ${(tok.palette || []).length} |`,
    `| type_scale | ${(tok.type_scale || []).length} |`,
    `| spacing | ${(tok.spacing || []).length} |`,
    `| radii | ${(tok.radii || []).length} |`,
    `| shadows | ${(tok.shadows || []).length} |`,
    '',
    '## Next steps',
    '',
    '1. Run `/autospec-define` with this spec to decompose into implementation issues.',
    '2. Implementers pick up each checkbox group as an atomic PR.',
    '3. Re-run `design-discover.sh` after each merge to confirm drift count decreases.',
    '',
  ];
}

function buildSpec(args, variant, components, inconsistencies, uxFindings) {
  return [
    ...buildHeader(args, variant),
    ...buildChecklist(variant, components, inconsistencies),
    ...buildUxFindings(uxFindings),
    ...buildTokenTable(variant),
  ].join('\n');
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

function main() {
  const args = parseArgs(process.argv);
  validateArgs(args);

  const variant = loadVariant(args.tokens, args.variant);
  const { components, inconsistencies, uxFindings } = parseInventory(args.inventory);

  const specContent = buildSpec(args, variant, components, inconsistencies, uxFindings);

  // Write spec to <out>/docs/specs/<date>-harmonize-<slug>-design.md
  const specDir = path.join(args.out, 'docs', 'specs');
  fs.mkdirSync(specDir, { recursive: true });
  const specFilename = `${args.date}-harmonize-${args.slug}-design.md`;
  const specPath = path.join(specDir, specFilename);
  fs.writeFileSync(specPath, specContent, 'utf8');

  // Print handoff to stdout
  process.stdout.write(`/autospec-define ${specPath}\n`);
}

main();
