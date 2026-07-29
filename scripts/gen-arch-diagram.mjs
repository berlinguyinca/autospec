#!/usr/bin/env node
// gen-arch-diagram.mjs — generate mermaid architecture diagrams from cluster output.
//
// Input: cluster JSON (output of tree-sitter-walk / reverse-engineer pipeline)
// Output: mermaid syntax strings embedded into ARCHITECTURE.md
//
// Replaces <!-- mermaid-graph-placeholder --> markers inserted by Phase 5 architecture.mjs.
//
// Three diagram types:
//   1. Top-level module graph: graph LR with directory clusters via mermaid subgraph
//   2. Per-CLI-entry call tree (depth 3)
//   3. Per-HTTP-entry call tree (depth 3)
//
// CLI:
//   node gen-arch-diagram.mjs --cluster <file>              # print mermaid to stdout
//   node gen-arch-diagram.mjs --cluster <file> --arch <md>  # patch ARCHITECTURE.md in place

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __filename = fileURLToPath(import.meta.url);

// ── Style / palette integration ───────────────────────────────────────────────
// doc-style.mjs is the single source of the light-blue palette.
// When --style is passed on the CLI, mermaidInit() is prepended to every
// generated diagram so renderers apply the correct theme variables.
let _mermaidInitFn = null;

async function getMermaidInit() {
  if (_mermaidInitFn) return _mermaidInitFn;
  try {
    const stylePath = path.resolve(path.dirname(__filename), '../../autospec-doc/scripts/doc-style.mjs');
    const mod = await import(stylePath);
    _mermaidInitFn = mod.mermaidInit;
  } catch {
    _mermaidInitFn = null;
  }
  return _mermaidInitFn;
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/**
 * Sanitize a slug/identifier for mermaid node IDs.
 * Replaces hyphens and dots with underscores; keeps alphanumeric + underscore.
 */
function mermaidId(slug) {
  return slug.replace(/[-./]/g, '_').replace(/[^a-zA-Z0-9_]/g, '');
}

/**
 * Extract the top-level directory segment from a file path.
 * e.g. "/tmp/wt/skills/src/cli.mjs" relative to a base, or just the first path segment.
 */
function dirGroup(filePath) {
  const parts = filePath.replace(/\\/g, '/').split('/').filter(Boolean);
  // Use the second-to-last segment (parent dir of file)
  if (parts.length >= 2) return parts[parts.length - 2];
  return parts[0] || 'root';
}

// ── Diagram generators ───────────────────────────────────────────────────────

/**
 * Generate the top-level module graph as mermaid `graph LR` with subgraph clusters.
 *
 * @param {{ significant: object[], trivial: object[] }} clusters
 * @returns {string} mermaid block (without fences)
 */
export function generateModuleGraph(clusters) {
  const units = clusters.significant || [];
  if (units.length === 0) return 'graph LR\n  %% no significant modules detected';

  // Group by parent directory
  const groups = new Map(); // dir → [slug, ...]
  for (const unit of units) {
    const file = (unit.files || [])[0] || '';
    const dir = dirGroup(file);
    if (!groups.has(dir)) groups.set(dir, []);
    groups.get(dir).push(unit.slug);
  }

  // Build importedBy edges: importedBy[slug] = [importer_slugs...]
  const fileToSlug = new Map();
  for (const unit of units) {
    for (const f of unit.files || []) {
      fileToSlug.set(f, unit.slug);
    }
  }

  const edges = []; // { from: slug, to: slug }
  for (const unit of units) {
    for (const importer of unit.importedBy || []) {
      const importerSlug = fileToSlug.get(importer);
      if (importerSlug && importerSlug !== unit.slug) {
        // importer depends on unit
        edges.push({ from: importerSlug, to: unit.slug });
      }
    }
  }

  const lines = ['graph LR'];

  // Subgraphs per directory
  for (const [dir, slugs] of groups) {
    if (slugs.length > 1) {
      lines.push(`  subgraph ${dir}`);
      for (const slug of slugs) {
        lines.push(`    ${mermaidId(slug)}["${slug}"]`);
      }
      lines.push('  end');
    } else {
      lines.push(`  ${mermaidId(slugs[0])}["${slugs[0]}"]`);
    }
  }

  // Edges
  const seen = new Set();
  for (const { from, to } of edges) {
    const key = `${from}-->${to}`;
    if (seen.has(key)) continue;
    seen.add(key);
    lines.push(`  ${mermaidId(from)} --> ${mermaidId(to)}`);
  }

  return lines.join('\n');
}

/**
 * Generate a call tree for a single entry point unit, depth-limited.
 *
 * @param {object} entryUnit - the cluster unit with an entry_point
 * @param {{ significant: object[] }} clusters
 * @param {'cli_command'|'http_handler'} kind
 * @param {number} maxDepth
 * @returns {string} mermaid block (without fences)
 */
export function generateCallTree(entryUnit, clusters, kind, maxDepth = 3) {
  const units = clusters.significant || [];
  const fileToSlug = new Map();
  for (const u of units) {
    for (const f of u.files || []) fileToSlug.set(f, u.slug);
  }

  // Build slug → imported slugs (what this slug calls)
  const importMap = new Map(); // slug → Set<slug>
  for (const u of units) {
    const calledSlugs = new Set();
    for (const other of units) {
      for (const importer of other.importedBy || []) {
        if ((u.files || []).includes(importer)) {
          calledSlugs.add(other.slug);
        }
      }
    }
    importMap.set(u.slug, calledSlugs);
  }

  const entry = entryUnit.slug;
  const entryKindLabel = kind === 'cli_command' ? 'CLI' : 'HTTP';
  const lines = [`graph LR`];
  lines.push(`  ${mermaidId(entry)}["${entry} [${entryKindLabel}]"]`);

  // BFS up to maxDepth
  const visited = new Set([entry]);
  const queue = [{ slug: entry, depth: 0 }];
  const edges = [];

  while (queue.length > 0) {
    const { slug, depth } = queue.shift();
    if (depth >= maxDepth) continue;
    for (const called of (importMap.get(slug) || [])) {
      if (!visited.has(called)) {
        visited.add(called);
        queue.push({ slug: called, depth: depth + 1 });
      }
      edges.push({ from: slug, to: called });
    }
  }

  for (const called of visited) {
    if (called !== entry) lines.push(`  ${mermaidId(called)}["${called}"]`);
  }

  const seen = new Set();
  for (const { from, to } of edges) {
    const key = `${from}-->${to}`;
    if (seen.has(key)) continue;
    seen.add(key);
    lines.push(`  ${mermaidId(from)} --> ${mermaidId(to)}`);
  }

  return lines.join('\n');
}

/**
 * Generate all three diagram types from cluster data.
 *
 * @param {{ significant: object[], trivial: object[] }} clusters
 * @returns {{ moduleGraph: string, cliTrees: string[], httpTrees: string[] }}
 */
export function generateAllDiagrams(clusters) {
  const units = clusters.significant || [];

  const moduleGraph = generateModuleGraph(clusters);

  const cliTrees = units
    .filter(u => (u.entry_points || []).some(ep => ep.kind === 'cli_command'))
    .map(u => generateCallTree(u, clusters, 'cli_command'));

  const httpTrees = units
    .filter(u => (u.entry_points || []).some(ep => ep.kind === 'http_handler'))
    .map(u => generateCallTree(u, clusters, 'http_handler'));

  return { moduleGraph, cliTrees, httpTrees };
}

/**
 * Wrap a mermaid string in a fenced code block.
 */
function mermaidFence(content) {
  return '```mermaid\n' + content + '\n```';
}

/**
 * Patch an ARCHITECTURE.md string, replacing <!-- mermaid-graph-placeholder -->
 * with the generated diagrams. Idempotent: replaces existing mermaid blocks too.
 *
 * @param {string} archContent - current ARCHITECTURE.md text
 * @param {{ moduleGraph: string, cliTrees: string[], httpTrees: string[] }} diagrams
 * @returns {string} updated ARCHITECTURE.md text
 */
export function patchArchitectureMd(archContent, diagrams) {
  const { moduleGraph, cliTrees, httpTrees } = diagrams;

  if (!archContent.includes('mermaid-graph-placeholder')) return archContent;

  const parts = [];
  parts.push(mermaidFence(moduleGraph));

  if (cliTrees.length > 0) {
    parts.push('\n### CLI entry-point call trees\n');
    for (const tree of cliTrees) {
      parts.push(mermaidFence(tree));
    }
  }

  if (httpTrees.length > 0) {
    parts.push('\n### HTTP entry-point call trees\n');
    for (const tree of httpTrees) {
      parts.push(mermaidFence(tree));
    }
  }

  const newBlock = parts.join('\n');
  const MARKER = '<!-- mermaid-graph-placeholder -->';

  // Split on the marker, then for each occurrence replace everything between
  // the marker and the next top-level heading (## or end-of-string) with newBlock.
  // This is idempotent: running twice yields the same output.
  const markerRe = /<!--\s*mermaid-graph-placeholder\s*-->/g;
  let result = '';
  let lastIndex = 0;
  let match;

  while ((match = markerRe.exec(archContent)) !== null) {
    // Append content up to and including the marker
    result += archContent.slice(lastIndex, match.index) + MARKER + '\n';

    // Find where the previously-generated block ends: scan forward from after the marker
    // until we hit the next top-level section heading (^## ) or end of string.
    // We consume any content that was previously inserted (mermaid fences, ### headings).
    const afterMarker = archContent.slice(match.index + match[0].length);
    // Skip the previously-generated block: lines that are part of generated output
    // (```mermaid fences, ### lines, blank lines, graph lines) before the next ## section.
    const nextSectionMatch = afterMarker.match(/\n(?=## )/);
    let skipEnd = 0;
    if (nextSectionMatch && nextSectionMatch.index !== undefined) {
      // Only skip up to the next ## heading
      skipEnd = nextSectionMatch.index + 1; // +1 to include the \n before ##
    }
    // If there's generated content between marker and next section, skip it
    const betweenContent = afterMarker.slice(0, skipEnd);
    const hasGeneratedContent = betweenContent.includes('```mermaid');
    if (hasGeneratedContent) {
      result += newBlock + '\n';
      lastIndex = match.index + match[0].length + skipEnd;
    } else {
      result += newBlock + '\n';
      lastIndex = match.index + match[0].length;
    }
  }

  result += archContent.slice(lastIndex);
  return result;
}

// ── CLI entrypoint ───────────────────────────────────────────────────────────

if (process.argv[1] && fs.realpathSync(path.resolve(process.argv[1])) === fs.realpathSync(path.resolve(__filename))) {
  const args = process.argv.slice(2);
  let clusterFile = null;
  let archFile = null;

  let useStyle = false;

  for (let i = 0; i < args.length; i++) {
    if (args[i] === '--cluster') clusterFile = args[i + 1];
    if (args[i] === '--arch')    archFile    = args[i + 1];
    if (args[i] === '--style')   useStyle    = true;
  }

  if (!clusterFile) {
    process.stderr.write('Usage: gen-arch-diagram.mjs --cluster <file> [--arch <architecture.md>] [--style]\n');
    process.exit(1);
  }

  let clusters;
  try {
    clusters = JSON.parse(fs.readFileSync(clusterFile, 'utf8'));
  } catch (err) {
    process.stderr.write(`gen-arch-diagram: failed to parse cluster file: ${err.message}\n`);
    process.exit(1);
  }

  // Resolve the mermaid init block when --style is requested.
  let initBlock = '';
  if (useStyle) {
    const initFn = await getMermaidInit();
    if (initFn) {
      initBlock = initFn() + '\n';
    } else {
      process.stderr.write('gen-arch-diagram: --style requested but doc-style.mjs could not be loaded; continuing without theme\n');
    }
  }

  /**
   * Wrap a mermaid string with the optional init block and fences.
   * @param {string} content
   * @returns {string}
   */
  function styledFence(content) {
    return '```mermaid\n' + initBlock + content + '\n```';
  }

  const diagrams = generateAllDiagrams(clusters);

  if (archFile) {
    let existing = '';
    if (fs.existsSync(archFile)) existing = fs.readFileSync(archFile, 'utf8');
    // When --style is active, patch diagrams to include the init block.
    let patchedDiagrams = diagrams;
    if (initBlock) {
      patchedDiagrams = {
        moduleGraph: initBlock.trimEnd() + '\n' + diagrams.moduleGraph,
        cliTrees:  diagrams.cliTrees.map(t  => initBlock.trimEnd() + '\n' + t),
        httpTrees: diagrams.httpTrees.map(t => initBlock.trimEnd() + '\n' + t),
      };
    }
    const patched = patchArchitectureMd(existing, patchedDiagrams);
    fs.writeFileSync(archFile, patched, 'utf8');
    process.stdout.write(`gen-arch-diagram: patched ${archFile}\n`);
  } else {
    // Print all diagrams to stdout
    const { moduleGraph, cliTrees, httpTrees } = diagrams;
    process.stdout.write('## Module Graph\n\n');
    process.stdout.write(styledFence(moduleGraph) + '\n');
    if (cliTrees.length > 0) {
      process.stdout.write('\n## CLI Entry-point Call Trees\n\n');
      for (const t of cliTrees) process.stdout.write(styledFence(t) + '\n\n');
    }
    if (httpTrees.length > 0) {
      process.stdout.write('\n## HTTP Entry-point Call Trees\n\n');
      for (const t of httpTrees) process.stdout.write(styledFence(t) + '\n\n');
    }
  }

  process.exit(0);
}
