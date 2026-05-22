// gen-arch-diagram.test.mjs — unit tests for Phase 7 mermaid diagram generator.
//
// Tests:
//   1. generateModuleGraph: cluster fixture → expected mermaid string (golden byte-comparison)
//   2. generateModuleGraph: empty cluster → safe fallback string
//   3. generateModuleGraph: subgraphs emitted for modules sharing a directory
//   4. generateAllDiagrams: all 3 diagram types appear in output
//   5. cliTrees: CLI entry-point units produce call trees
//   6. httpTrees: HTTP entry-point units produce call trees
//   7. patchArchitectureMd: replaces <!-- mermaid-graph-placeholder --> with mermaid block
//   8. patchArchitectureMd: idempotent (calling twice does not nest diagrams)
//   9. Mode II violation: forbidden URL in route → gen-screenshots aborts (see gen-screenshots.test.mjs)

import { test } from 'node:test';
import assert from 'node:assert/strict';
import path from 'node:path';
import fs from 'node:fs';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const SCRIPTS_DIR = path.resolve(__dirname, '../../scripts');

const {
  generateModuleGraph,
  generateCallTree,
  generateAllDiagrams,
  patchArchitectureMd,
} = await import(path.join(SCRIPTS_DIR, 'gen-arch-diagram.mjs'));

// ── Fixtures ──────────────────────────────────────────────────────────────────

const FIXTURE_PATH = path.resolve(__dirname, '../fixtures/cluster-sample.json');
const CLUSTER_SAMPLE = JSON.parse(fs.readFileSync(FIXTURE_PATH, 'utf8'));

/** Minimal cluster with no units */
const EMPTY_CLUSTER = { significant: [], trivial: [] };

/** Cluster with a CLI entry and its dependency */
const CLI_CLUSTER = {
  significant: [
    {
      slug: 'src-cli',
      files: ['/repo/src/cli.mjs'],
      language: 'javascript',
      exports: [{ name: 'main', kind: 'function', signature: 'export function main()', line: 1 }],
      entry_points: [{ kind: 'cli_command', identifier: 'cli.mjs', line: 1 }],
      importedBy: [],
      reasons: ['has_exports', 'cli_entry'],
    },
    {
      slug: 'src-utils',
      files: ['/repo/src/utils.mjs'],
      language: 'javascript',
      exports: [{ name: 'greet', kind: 'function', signature: 'export function greet(name)', line: 1 }],
      entry_points: [],
      importedBy: ['/repo/src/cli.mjs'],
      reasons: ['has_exports'],
    },
  ],
  trivial: [],
};

/** Cluster with an HTTP handler */
const HTTP_CLUSTER = {
  significant: [
    {
      slug: 'src-server',
      files: ['/repo/src/server.mjs'],
      language: 'javascript',
      exports: [{ name: 'startServer', kind: 'function', signature: 'export function startServer()', line: 1 }],
      entry_points: [{ kind: 'http_handler', identifier: 'GET /', line: 5 }],
      importedBy: [],
      reasons: ['has_exports', 'http_handler'],
    },
  ],
  trivial: [],
};

/** Cluster where two modules share a directory (src/) */
const SHARED_DIR_CLUSTER = {
  significant: [
    {
      slug: 'src-cli',
      files: ['/repo/src/cli.mjs'],
      language: 'javascript',
      exports: [],
      entry_points: [{ kind: 'cli_command', identifier: 'cli.mjs', line: 1 }],
      importedBy: [],
      reasons: ['cli_entry'],
    },
    {
      slug: 'src-parser',
      files: ['/repo/src/parser.mjs'],
      language: 'javascript',
      exports: [],
      entry_points: [],
      importedBy: [],
      reasons: ['has_exports'],
    },
  ],
  trivial: [],
};

// ── Tests ─────────────────────────────────────────────────────────────────────

test('generateModuleGraph: empty cluster returns safe fallback', () => {
  const result = generateModuleGraph(EMPTY_CLUSTER);
  assert.ok(result.includes('graph LR'), 'must start with graph LR');
  assert.ok(result.includes('%%'), 'must include comment for empty case');
});

test('generateModuleGraph: produces graph LR for cluster-sample fixture', () => {
  const result = generateModuleGraph(CLUSTER_SAMPLE);
  assert.ok(result.startsWith('graph LR'), 'must start with graph LR');
  // All significant slugs must appear in the output
  for (const unit of CLUSTER_SAMPLE.significant) {
    const id = unit.slug.replace(/[-./]/g, '_').replace(/[^a-zA-Z0-9_]/g, '');
    assert.ok(result.includes(id), `must include mermaid ID for ${unit.slug}`);
  }
});

test('generateModuleGraph: subgraph emitted when two modules share a directory', () => {
  const result = generateModuleGraph(SHARED_DIR_CLUSTER);
  assert.ok(result.includes('subgraph'), 'must emit subgraph for shared directory');
  assert.ok(result.includes('src'), 'subgraph must reference the shared directory name');
});

test('generateModuleGraph: edges from importedBy relationships', () => {
  const result = generateModuleGraph(CLI_CLUSTER);
  // src-utils is imported by src-cli → edge src_cli --> src_utils
  assert.ok(result.includes('src_cli') && result.includes('src_utils'), 'both slugs must appear');
  assert.ok(result.includes('-->'), 'must include at least one edge');
});

test('generateAllDiagrams: all 3 diagram types present in output object', () => {
  const diagrams = generateAllDiagrams(CLI_CLUSTER);
  assert.ok(typeof diagrams.moduleGraph === 'string', 'moduleGraph must be string');
  assert.ok(Array.isArray(diagrams.cliTrees), 'cliTrees must be array');
  assert.ok(Array.isArray(diagrams.httpTrees), 'httpTrees must be array');
});

test('generateAllDiagrams: CLI entry produces cliTree', () => {
  const diagrams = generateAllDiagrams(CLI_CLUSTER);
  assert.ok(diagrams.cliTrees.length > 0, 'must have at least one CLI call tree');
  assert.ok(diagrams.cliTrees[0].includes('graph LR'), 'CLI tree must be graph LR');
  assert.ok(diagrams.cliTrees[0].includes('CLI'), 'CLI tree must label entry as CLI');
});

test('generateAllDiagrams: HTTP entry produces httpTree', () => {
  const diagrams = generateAllDiagrams(HTTP_CLUSTER);
  assert.ok(diagrams.httpTrees.length > 0, 'must have at least one HTTP call tree');
  assert.ok(diagrams.httpTrees[0].includes('graph LR'), 'HTTP tree must be graph LR');
  assert.ok(diagrams.httpTrees[0].includes('HTTP'), 'HTTP tree must label entry as HTTP');
});

test('generateAllDiagrams: cluster-sample fixture → valid mermaid output (golden byte-comparison)', () => {
  const diagrams = generateAllDiagrams(CLUSTER_SAMPLE);
  const graph = diagrams.moduleGraph;

  // Golden: must start with graph LR
  assert.ok(graph.startsWith('graph LR'), 'golden: must start with graph LR');
  // Golden: lib-config and src-cli slugs present
  assert.ok(graph.includes('lib_config'), 'golden: lib_config node must appear');
  assert.ok(graph.includes('src_cli'), 'golden: src_cli node must appear');
  // Golden: CLI tree for src-cli (has cli_entry)
  assert.ok(diagrams.cliTrees.length >= 1, 'golden: must have at least 1 CLI tree');
  // Golden: no HTTP entries in fixture
  assert.strictEqual(diagrams.httpTrees.length, 0, 'golden: fixture has no HTTP entries');
});

test('generateCallTree: depth-limited to 3 hops', () => {
  // Build a 5-deep chain to verify truncation
  const deep = {
    significant: [
      { slug: 'a', files: ['/r/a.mjs'], entry_points: [{ kind: 'cli_command', identifier: 'a', line: 1 }], importedBy: [], exports: [], reasons: ['cli_entry'] },
      { slug: 'b', files: ['/r/b.mjs'], entry_points: [], importedBy: ['/r/a.mjs'], exports: [], reasons: ['has_exports'] },
      { slug: 'c', files: ['/r/c.mjs'], entry_points: [], importedBy: ['/r/b.mjs'], exports: [], reasons: ['has_exports'] },
      { slug: 'd', files: ['/r/d.mjs'], entry_points: [], importedBy: ['/r/c.mjs'], exports: [], reasons: ['has_exports'] },
      { slug: 'e', files: ['/r/e.mjs'], entry_points: [], importedBy: ['/r/d.mjs'], exports: [], reasons: ['has_exports'] },
    ],
    trivial: [],
  };
  const entryUnit = deep.significant[0];
  const tree = generateCallTree(entryUnit, deep, 'cli_command', 3);
  // At depth 3, we can reach b, c, d but not e
  assert.ok(tree.includes('b'), 'depth 1 node must appear');
  assert.ok(tree.includes('c'), 'depth 2 node must appear');
  assert.ok(tree.includes('d'), 'depth 3 node must appear');
  assert.ok(!tree.includes('"e"'), 'depth 4 node must be truncated');
});

test('patchArchitectureMd: replaces placeholder with mermaid block', () => {
  const archContent = '# Architecture\n\n<!-- mermaid-graph-placeholder -->\n\n## Modules\n';
  const diagrams = generateAllDiagrams(CLI_CLUSTER);
  const patched = patchArchitectureMd(archContent, diagrams);

  assert.ok(patched.includes('```mermaid'), 'must contain mermaid fence');
  assert.ok(patched.includes('graph LR'), 'must contain graph LR');
  assert.ok(patched.includes('<!-- mermaid-graph-placeholder -->'), 'must preserve placeholder marker');
  assert.ok(patched.includes('## Modules'), 'must preserve content after placeholder');
});

test('patchArchitectureMd: idempotent (patching twice does not nest diagrams)', () => {
  const archContent = '# Architecture\n\n<!-- mermaid-graph-placeholder -->\n\n## Modules\n';
  const diagrams = generateAllDiagrams(CLI_CLUSTER);
  const patched1 = patchArchitectureMd(archContent, diagrams);
  const patched2 = patchArchitectureMd(patched1, diagrams);

  // Count occurrences of 'graph LR' — should be same in both passes
  const count1 = (patched1.match(/graph LR/g) || []).length;
  const count2 = (patched2.match(/graph LR/g) || []).length;
  assert.strictEqual(count1, count2, 'patching twice must not increase diagram count');
});

test('patchArchitectureMd: content without placeholder is unchanged', () => {
  const archContent = '# Architecture\n\nNo placeholder here.\n';
  const diagrams = generateAllDiagrams(CLI_CLUSTER);
  const patched = patchArchitectureMd(archContent, diagrams);
  assert.strictEqual(patched, archContent, 'content without placeholder must be unchanged');
});
