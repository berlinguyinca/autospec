// gen-docs.test.mjs — unit tests for doc generator pipeline (Phase 5).
//
// Tests:
//   1. Empty existing-docs case yields full output for each generator
//   2. Existing docs with 1 hand-edited section preserved across regenerate
//   3. Every generated section contains a <!-- autospec-doc-scope: ... --> block with src:
//   4. architecture.mjs output contains exactly 1 <!-- mermaid-graph-placeholder -->
//   5. All 4 files emit valid markdown parseable by scan-doc-scope.mjs
//   6. Orchestrator dispatches to all 3 generators

import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const SCRIPTS_DIR = path.resolve(__dirname, '../../scripts');
const GEN_DOCS_DIR = path.join(SCRIPTS_DIR, 'gen-docs');

// Import generators under test
const { generate: generateUserManual }   = await import(path.join(GEN_DOCS_DIR, 'user-manual.mjs'));
const { generate: generateApiReference } = await import(path.join(GEN_DOCS_DIR, 'api-reference.mjs'));
const { generate: generateArchitecture } = await import(path.join(GEN_DOCS_DIR, 'architecture.mjs'));
const { generateDocs }                   = await import(path.join(SCRIPTS_DIR, 'gen-docs-from-spec.mjs'));
const { parse: parseScopeBlocks }        = await import(path.join(SCRIPTS_DIR, 'scan-doc-scope.mjs'));

test('gen-docs CLI writes JSON through stdout without console logging', () => {
  const source = fs.readFileSync(path.join(SCRIPTS_DIR, 'gen-docs-from-spec.mjs'), 'utf8');
  assert.doesNotMatch(source, /console\.(log|error)\s*\(/, 'CLI output must use explicit streams');
  assert.match(source, /process\.stdout\.write\(/, 'CLI output should be written to stdout');
});

// ── Fixture data ──────────────────────────────────────────────────────────────

const BAIT_DIR = path.resolve(__dirname, '../fixtures/reverse-engineer-bait');

/** A minimal cluster output with CLI entry, exports, and HTTP routes */
const SAMPLE_CLUSTERS = {
  significant: [
    {
      slug: 'src-cli',
      files: [path.join(BAIT_DIR, 'src/cli.mjs')],
      language: 'javascript',
      exports: [
        { name: 'main', kind: 'function', signature: 'export function main()', line: 10 },
      ],
      entry_points: [
        { kind: 'cli_command', identifier: 'cli.mjs', line: 1 },
      ],
      importedBy: [],
      reasons: ['has_exports', 'cli_entry'],
    },
    {
      slug: 'src-utils',
      files: [path.join(BAIT_DIR, 'src/utils.mjs')],
      language: 'javascript',
      exports: [
        { name: 'greet',      kind: 'function', signature: 'export function greet(name)', line: 7 },
        { name: 'formatDate', kind: 'function', signature: 'export function formatDate(date)', line: 15 },
      ],
      entry_points: [],
      importedBy: [
        path.join(BAIT_DIR, 'src/cli.mjs'),
        path.join(BAIT_DIR, 'src/parser.mjs'),
        path.join(BAIT_DIR, 'lib/config.mjs'),
      ],
      reasons: ['has_exports', 'imported_by_3+'],
    },
    {
      slug: 'src-parser',
      files: [path.join(BAIT_DIR, 'src/parser.mjs')],
      language: 'javascript',
      exports: [
        { name: 'parseArgs', kind: 'function', signature: 'export function parseArgs(argv)', line: 10 },
      ],
      entry_points: [
        { kind: 'http_route', identifier: '/api/parse', line: 20 },
        { kind: 'http_route', identifier: '/api/status', line: 25 },
      ],
      importedBy: [
        path.join(BAIT_DIR, 'src/cli.mjs'),
        path.join(BAIT_DIR, 'lib/config.mjs'),
      ],
      reasons: ['has_exports'],
    },
  ],
  trivial: [
    {
      filePath: path.join(BAIT_DIR, 'lib/validator.py'),
      bubbledInto: path.join(BAIT_DIR, 'src/utils.mjs'),
    },
  ],
};

const EMPTY_CLUSTERS = { significant: [], trivial: [] };

// ── Helper ────────────────────────────────────────────────────────────────────

function makeTmpDir() {
  return fs.mkdtempSync(path.join(os.tmpdir(), 'autospec-gen-docs-test-'));
}

/**
 * Count occurrences of a substring in a string.
 */
function countOccurrences(str, sub) {
  let count = 0, idx = 0;
  while ((idx = str.indexOf(sub, idx)) !== -1) { count++; idx += sub.length; }
  return count;
}

// ── Test 1: Empty clusters — each generator produces output ──────────────────

test('user-manual: empty clusters produces at least a Getting Started section', () => {
  const result = generateUserManual({ clusters: EMPTY_CLUSTERS, specs: [], existingDocs: {} });
  assert.ok(result.content.length > 0, 'should produce non-empty content');
  assert.ok(result.content.includes('# User Manual'), 'should have H1 title');
  assert.ok(result.section_anchors.length > 0, 'should have at least one anchor');
});

test('api-reference: empty clusters produces a stub doc', () => {
  const result = generateApiReference({ clusters: EMPTY_CLUSTERS, specs: [], existingDocs: {} });
  assert.ok(result.content.includes('# API Reference'), 'should have H1 title');
  assert.equal(result.path, 'docs/API_REFERENCE.md');
});

test('architecture: empty clusters produces a stub doc with mermaid placeholder', () => {
  const result = generateArchitecture({ clusters: EMPTY_CLUSTERS, specs: [], existingDocs: {} });
  assert.ok(result.content.includes('# Architecture'), 'should have H1 title');
  assert.ok(result.content.includes('<!-- mermaid-graph-placeholder -->'), 'should have mermaid placeholder');
});

// ── Test 2: Full clusters — generators produce rich output ────────────────────

test('user-manual: CLI entry points get their own sections', () => {
  const result = generateUserManual({ clusters: SAMPLE_CLUSTERS, specs: [], existingDocs: {} });
  assert.ok(result.content.includes('## CLI Commands'), 'should have CLI Commands section');
  assert.ok(result.content.includes('cli.mjs'), 'should mention the CLI entry');
});

test('user-manual: HTTP route groups get their own sections', () => {
  const result = generateUserManual({ clusters: SAMPLE_CLUSTERS, specs: [], existingDocs: {} });
  assert.ok(result.content.includes('## HTTP API'), 'should have HTTP API section');
  assert.ok(result.content.includes('/api'), 'should mention the /api route group');
});

test('api-reference: per-module sections with exports', () => {
  const result = generateApiReference({ clusters: SAMPLE_CLUSTERS, specs: [], existingDocs: {} });
  assert.ok(result.content.includes('## Module: `src-utils`'), 'should have src-utils module');
  assert.ok(result.content.includes('### `greet`'), 'should have greet export');
  assert.ok(result.content.includes('### `formatDate`'), 'should have formatDate export');
  assert.ok(result.content.includes('### `parseArgs`'), 'should have parseArgs export');
});

test('architecture: per-cluster paragraphs present', () => {
  const result = generateArchitecture({ clusters: SAMPLE_CLUSTERS, specs: [], existingDocs: {} });
  assert.ok(result.content.includes('### `src-cli`'), 'should have src-cli section');
  assert.ok(result.content.includes('### `src-utils`'), 'should have src-utils section');
  assert.ok(result.content.includes('### `src-parser`'), 'should have src-parser section');
});

test('architecture: trivial files section present', () => {
  const result = generateArchitecture({ clusters: SAMPLE_CLUSTERS, specs: [], existingDocs: {} });
  assert.ok(result.content.includes('## Supporting Files'), 'should have Supporting Files section');
  assert.ok(result.content.includes('validator.py'), 'should mention trivial validator.py');
});

// ── Test 3: Every generated section has autospec-doc-scope block ─────────────

test('user-manual: all sections include autospec-doc-scope with src:', () => {
  const result = generateUserManual({ clusters: SAMPLE_CLUSTERS, specs: [], existingDocs: {} });
  const scopeCount = countOccurrences(result.content, '<!-- autospec-doc-scope:');
  const srcCount   = countOccurrences(result.content, 'src: [');
  assert.ok(scopeCount > 0, 'should have at least 1 scope block');
  assert.equal(scopeCount, srcCount, 'every scope block should have a src: entry');
});

test('api-reference: all sections include autospec-doc-scope with src:', () => {
  const result = generateApiReference({ clusters: SAMPLE_CLUSTERS, specs: [], existingDocs: {} });
  const scopeCount = countOccurrences(result.content, '<!-- autospec-doc-scope:');
  const srcCount   = countOccurrences(result.content, 'src: [');
  assert.ok(scopeCount > 0, 'should have at least 1 scope block');
  assert.equal(scopeCount, srcCount, 'every scope block should have a src: entry');
});

test('architecture: all sections include autospec-doc-scope with src:', () => {
  const result = generateArchitecture({ clusters: SAMPLE_CLUSTERS, specs: [], existingDocs: {} });
  const scopeCount = countOccurrences(result.content, '<!-- autospec-doc-scope:');
  const srcCount   = countOccurrences(result.content, 'src: [');
  assert.ok(scopeCount > 0, 'should have at least 1 scope block');
  assert.equal(scopeCount, srcCount, 'every scope block should have a src: entry');
});

// ── Test 4: Exactly 1 mermaid-graph-placeholder in architecture ───────────────

test('architecture: exactly 1 mermaid-graph-placeholder in output', () => {
  const result = generateArchitecture({ clusters: SAMPLE_CLUSTERS, specs: [], existingDocs: {} });
  const count = countOccurrences(result.content, '<!-- mermaid-graph-placeholder -->');
  assert.equal(count, 1, `expected exactly 1 mermaid placeholder, got ${count}`);
});

// ── Test 5: scan-doc-scope parses generated docs without errors ───────────────

test('scan-doc-scope: parses user-manual output cleanly', async () => {
  const tmpDir = makeTmpDir();
  try {
    const result = generateUserManual({ clusters: SAMPLE_CLUSTERS, specs: [], existingDocs: {} });
    const tmpFile = path.join(tmpDir, 'USER_MANUAL.md');
    fs.writeFileSync(tmpFile, result.content, 'utf8');
    // parse() should return an array (even if empty — no throwing)
    const scopes = await parseScopeBlocks(tmpFile);
    assert.ok(Array.isArray(scopes), 'parse should return an array');
    assert.ok(scopes.length > 0, 'should detect at least 1 scope block in user-manual');
  } finally {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  }
});

test('scan-doc-scope: parses architecture output cleanly', async () => {
  const tmpDir = makeTmpDir();
  try {
    const result = generateArchitecture({ clusters: SAMPLE_CLUSTERS, specs: [], existingDocs: {} });
    const tmpFile = path.join(tmpDir, 'ARCHITECTURE.md');
    fs.writeFileSync(tmpFile, result.content, 'utf8');
    const scopes = await parseScopeBlocks(tmpFile);
    assert.ok(Array.isArray(scopes), 'parse should return an array');
    assert.ok(scopes.length > 0, 'should detect at least 1 scope block in architecture');
  } finally {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  }
});

// ── Test 6: Human-edit preservation ──────────────────────────────────────────

test('orchestrator: hand-edited section (no generated:true) is preserved verbatim', async () => {
  // Generate initial docs
  const result1 = await generateDocs({ clusters: SAMPLE_CLUSTERS });
  const archFile = result1.files.find(f => f.path === 'docs/ARCHITECTURE.md');
  assert.ok(archFile, 'should generate ARCHITECTURE.md');

  // Simulate a human editing the Module Graph section: remove generated: true
  const originalContent = archFile.content;
  const handEdited = originalContent.replace(
    '  generated: true\n-->',
    '  generated: false\n-->'
  ).replace(
    '> *Mermaid diagram will be generated in Phase 7 (`gen-arch-diagram.mjs`).*',
    '> *HUMAN EDIT: This is a hand-crafted description.*'
  );

  // Regenerate with hand-edited content as existing
  const result2 = await generateDocs({
    clusters: SAMPLE_CLUSTERS,
    existingDocs: { 'docs/ARCHITECTURE.md': handEdited },
  });

  const regenArch = result2.files.find(f => f.path === 'docs/ARCHITECTURE.md');
  assert.ok(regenArch, 'should regenerate ARCHITECTURE.md');
  assert.ok(
    regenArch.content.includes('HUMAN EDIT: This is a hand-crafted description.'),
    'human-edited section should be preserved verbatim'
  );
  assert.ok(regenArch.preserved_sections > 0, 'preserved_sections should be > 0');
});

// ── Test 7: Orchestrator dispatches to all 3 generators ──────────────────────

test('orchestrator: generateDocs returns all 3 doc files', async () => {
  const result = await generateDocs({ clusters: SAMPLE_CLUSTERS });
  assert.equal(result.files.length, 3, 'should return exactly 3 files');
  const paths = result.files.map(f => f.path);
  assert.ok(paths.includes('docs/USER_MANUAL.md'),   'should include USER_MANUAL.md');
  assert.ok(paths.includes('docs/API_REFERENCE.md'), 'should include API_REFERENCE.md');
  assert.ok(paths.includes('docs/ARCHITECTURE.md'),  'should include ARCHITECTURE.md');
});

test('orchestrator: writes files to outputDir when provided', async () => {
  const tmpDir = makeTmpDir();
  try {
    const result = await generateDocs({ clusters: SAMPLE_CLUSTERS, outputDir: tmpDir });
    for (const file of result.files) {
      const outPath = path.join(tmpDir, file.path);
      assert.ok(fs.existsSync(outPath), `${file.path} should exist in outputDir`);
      const content = fs.readFileSync(outPath, 'utf8');
      assert.ok(content.length > 0, `${file.path} should have non-empty content`);
    }
  } finally {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  }
});

test('orchestrator: second write with same content is idempotent (written=false)', async () => {
  const tmpDir = makeTmpDir();
  try {
    const result1 = await generateDocs({ clusters: SAMPLE_CLUSTERS, outputDir: tmpDir });
    const result2 = await generateDocs({ clusters: SAMPLE_CLUSTERS, outputDir: tmpDir });
    // All files should show written=false on second pass (content unchanged)
    for (const file of result2.files) {
      assert.equal(file.written, false, `${file.path} should not be re-written on second pass`);
    }
  } finally {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  }
});

// ── Test 8: Path and content correctness ──────────────────────────────────────

test('all generators: return correct path constants', () => {
  const um = generateUserManual({ clusters: EMPTY_CLUSTERS });
  const ar = generateApiReference({ clusters: EMPTY_CLUSTERS });
  const ac = generateArchitecture({ clusters: EMPTY_CLUSTERS });
  assert.equal(um.path, 'docs/USER_MANUAL.md');
  assert.equal(ar.path, 'docs/API_REFERENCE.md');
  assert.equal(ac.path, 'docs/ARCHITECTURE.md');
});

test('all generators: section_anchors is an array', () => {
  const um = generateUserManual({ clusters: SAMPLE_CLUSTERS });
  const ar = generateApiReference({ clusters: SAMPLE_CLUSTERS });
  const ac = generateArchitecture({ clusters: SAMPLE_CLUSTERS });
  assert.ok(Array.isArray(um.section_anchors));
  assert.ok(Array.isArray(ar.section_anchors));
  assert.ok(Array.isArray(ac.section_anchors));
});
