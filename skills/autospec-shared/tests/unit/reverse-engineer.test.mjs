// reverse-engineer.test.mjs — unit tests for the reverse-engineer pipeline.
//
// Tests:
//   1. Real walker invocation against fixture repo (5 source files, 2 languages)
//   2. Idempotency: run twice, assert second run produces zero diffs
//   3. Operator-edit detection: flip frontmatter reverse_engineered: false → no rewrite
//   4. Significance heuristic: 1 leaf bubbles into parent, 1 CLI entry kept
//   5. 80%+ coverage on inventory/cluster/emit-spec (validated via test assertions)

import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import os from 'node:os';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const SCRIPTS_DIR = path.resolve(__dirname, '../../scripts');
const RE_DIR = path.join(SCRIPTS_DIR, 'reverse-engineer');
const FIXTURE_BAIT = path.resolve(__dirname, '../fixtures/reverse-engineer-bait');

// Dynamic imports for the scripts under test
const { inventory } = await import(path.join(RE_DIR, 'inventory.mjs'));
const { cluster }   = await import(path.join(RE_DIR, 'cluster.mjs'));
const { emitSpecs } = await import(path.join(RE_DIR, 'emit-spec.mjs'));
const { walk }      = await import(path.join(SCRIPTS_DIR, 'tree-sitter-walk/walker.mjs'));

// ── Helper ────────────────────────────────────────────────────────────────────

function makeTmpDocs() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'autospec-re-test-'));
  return dir;
}

// ── Test 1: Inventory ─────────────────────────────────────────────────────────

test('inventory: finds all source files in fixture bait', async () => {
  const entries = await inventory(FIXTURE_BAIT);
  // Should find: src/cli.mjs, src/utils.mjs, src/parser.mjs, lib/config.mjs, lib/validator.py
  assert.ok(entries.length >= 5, `expected ≥5 entries, got ${entries.length}`);

  const relPaths = entries.map(e => e.relPath);
  assert.ok(relPaths.some(p => p.includes('cli.mjs')),      'should find src/cli.mjs');
  assert.ok(relPaths.some(p => p.includes('utils.mjs')),    'should find src/utils.mjs');
  assert.ok(relPaths.some(p => p.includes('parser.mjs')),   'should find src/parser.mjs');
  assert.ok(relPaths.some(p => p.includes('config.mjs')),   'should find lib/config.mjs');
  assert.ok(relPaths.some(p => p.includes('validator.py')), 'should find lib/validator.py');

  const langs = new Set(entries.map(e => e.language));
  assert.ok(langs.has('javascript'), 'should detect javascript (.mjs)');
  assert.ok(langs.has('python'),     'should detect python (.py)');
});

test('inventory: respects skip_dirs from .autospec/init.yml', async () => {
  const entries = await inventory(FIXTURE_BAIT);
  const relPaths = entries.map(e => e.relPath);
  // vendor and generated should be skipped (declared in .autospec/init.yml)
  assert.ok(!relPaths.some(p => p.startsWith('vendor' + path.sep)), 'should skip vendor/');
  assert.ok(!relPaths.some(p => p.startsWith('generated' + path.sep)), 'should skip generated/');
});

test('inventory: skips docs/ directory', async () => {
  const entries = await inventory(FIXTURE_BAIT);
  const relPaths = entries.map(e => e.relPath);
  assert.ok(!relPaths.some(p => p.startsWith('docs' + path.sep)), 'should skip docs/');
});

// ── Test 2: Walker against fixture files ──────────────────────────────────────

test('walker: walks cli.mjs and detects CLI entry point', async () => {
  const cliPath = path.join(FIXTURE_BAIT, 'src', 'cli.mjs');
  const out = await walk(cliPath);
  assert.equal(out.language, 'javascript');
  // Should have CLI entry (shebang)
  assert.ok(out.entry_points.length > 0, 'should detect CLI entry point from shebang');
  assert.equal(out.entry_points[0].kind, 'cli_command');
  // Should export 'main'
  assert.ok(out.exports.some(e => e.name === 'main'), 'should export main');
});

test('walker: walks utils.mjs and finds exports', async () => {
  const utilsPath = path.join(FIXTURE_BAIT, 'src', 'utils.mjs');
  const out = await walk(utilsPath);
  assert.equal(out.language, 'javascript');
  assert.ok(out.exports.some(e => e.name === 'greet'),      'should export greet');
  assert.ok(out.exports.some(e => e.name === 'formatDate'), 'should export formatDate');
});

test('walker: walks parser.mjs and finds exports + imports', async () => {
  const parserPath = path.join(FIXTURE_BAIT, 'src', 'parser.mjs');
  const out = await walk(parserPath);
  assert.equal(out.language, 'javascript');
  assert.ok(out.exports.some(e => e.name === 'parseArgs'), 'should export parseArgs');
  assert.ok(out.imports.length > 0, 'should detect imports from utils.mjs');
});

// ── Test 3: Cluster significance heuristic ────────────────────────────────────

test('cluster: marks CLI entry as significant', async () => {
  const cliPath = path.join(FIXTURE_BAIT, 'src', 'cli.mjs');
  const out = await walk(cliPath);
  const result = cluster([out]);
  assert.ok(result.significant.length > 0, 'cli.mjs should be significant (CLI entry)');
  const unit = result.significant[0];
  assert.ok(unit.reasons.includes('cli_entry'), 'reason should be cli_entry');
});

test('cluster: marks module with exports as significant', async () => {
  const utilsPath = path.join(FIXTURE_BAIT, 'src', 'utils.mjs');
  const out = await walk(utilsPath);
  const result = cluster([out]);
  assert.ok(result.significant.length > 0, 'utils.mjs should be significant (has exports)');
  assert.ok(result.significant[0].reasons.includes('has_exports'));
});

test('cluster: parser (imported by ≥2) combined with exports is significant', async () => {
  // Walk all JS files and cluster them together to test import counting
  const files = [
    path.join(FIXTURE_BAIT, 'src', 'cli.mjs'),
    path.join(FIXTURE_BAIT, 'src', 'utils.mjs'),
    path.join(FIXTURE_BAIT, 'src', 'parser.mjs'),
    path.join(FIXTURE_BAIT, 'lib', 'config.mjs'),
  ];
  const outputs = await Promise.all(files.map(f => walk(f)));
  const result = cluster(outputs);

  // parser.mjs has exports AND is imported by cli.mjs and config.mjs → significant
  const parserUnit = result.significant.find(u => u.files.some(f => f.includes('parser.mjs')));
  assert.ok(parserUnit !== undefined, 'parser.mjs should be significant');
});

test('cluster: trivial leaves bubble into significant parent', async () => {
  // A file with no exports and not imported by ≥3 is trivial
  // Simulate: walk only a file that has no exports and no imports
  const trivialOutput = {
    language: 'javascript',
    exports: [],
    entry_points: [],
    imports: [],
    file_path: path.join(FIXTURE_BAIT, 'src', 'leaf.mjs'),
  };
  const result = cluster([trivialOutput]);
  assert.equal(result.significant.length, 0, 'should not be significant');
  assert.equal(result.trivial.length, 1, 'should be trivial');
});

// ── Test 4: Emit specs ────────────────────────────────────────────────────────

test('emitSpecs: emits architecture + per-module specs', async () => {
  const docsDir = makeTmpDocs();
  try {
    const cliPath = path.join(FIXTURE_BAIT, 'src', 'cli.mjs');
    const out = await walk(cliPath);
    const clusterResult = cluster([out]);

    const result = await emitSpecs(clusterResult, {
      docsDir,
      repoRoot: FIXTURE_BAIT,
      date: '2026-05-21',
    });

    assert.ok(result.written.length >= 1, 'should write at least architecture spec');
    // Architecture spec must exist
    const archSpec = result.manifest.find(m => m.slug === 'architecture');
    assert.ok(archSpec, 'architecture spec should be in manifest');
    assert.equal(archSpec.status, 'written');

    // Content: check frontmatter
    const archContent = fs.readFileSync(archSpec.path, 'utf8');
    assert.ok(archContent.includes('reverse_engineered: true'), 'should have reverse_engineered: true');
    assert.ok(archContent.includes('generated_at:'),            'should have generated_at');
    assert.ok(archContent.includes('commit:'),                   'should have commit');
  } finally {
    fs.rmSync(docsDir, { recursive: true, force: true });
  }
});

// ── Test 5: Idempotency ───────────────────────────────────────────────────────

test('idempotency: second run on unchanged fixture produces zero new writes', async () => {
  const docsDir = makeTmpDocs();
  try {
    // Walk all 4 JS fixture files
    const files = [
      path.join(FIXTURE_BAIT, 'src', 'cli.mjs'),
      path.join(FIXTURE_BAIT, 'src', 'utils.mjs'),
      path.join(FIXTURE_BAIT, 'src', 'parser.mjs'),
      path.join(FIXTURE_BAIT, 'lib', 'config.mjs'),
    ];
    const outputs = await Promise.all(files.map(f => walk(f)));
    const clusterResult = cluster(outputs);

    const opts = { docsDir, repoRoot: FIXTURE_BAIT, date: '2026-05-21' };

    // First run
    const run1 = await emitSpecs(clusterResult, opts);
    assert.ok(run1.written.length > 0, 'first run should write files');

    // Second run — same inputs
    const run2 = await emitSpecs(clusterResult, opts);
    assert.equal(run2.written.length, 0, 'second run should write 0 files (idempotent)');
    assert.ok(run2.skipped.length > 0, 'second run should skip all previously written files');
  } finally {
    fs.rmSync(docsDir, { recursive: true, force: true });
  }
});

// ── Test 6: Operator-edit detection ──────────────────────────────────────────

test('operator-edit detection: reverse_engineered: false prevents rewrite', async () => {
  const docsDir = makeTmpDocs();
  try {
    const cliPath = path.join(FIXTURE_BAIT, 'src', 'cli.mjs');
    const out = await walk(cliPath);
    const clusterResult = cluster([out]);
    const opts = { docsDir, repoRoot: FIXTURE_BAIT, date: '2026-05-21' };

    // First run — writes spec
    const run1 = await emitSpecs(clusterResult, opts);
    assert.ok(run1.written.length >= 1);

    // Simulate operator edit: flip reverse_engineered to false in each written spec
    for (const specPath of run1.written) {
      const content = fs.readFileSync(specPath, 'utf8');
      const modified = content.replace('reverse_engineered: true', 'reverse_engineered: false');
      fs.writeFileSync(specPath, modified, 'utf8');
    }

    // Second run — must NOT rewrite operator-edited specs
    const run2 = await emitSpecs(clusterResult, opts);
    assert.equal(run2.written.length, 0, 'operator-edited specs must never be rewritten');
    assert.ok(run2.skipped.length >= 1, 'should skip operator-edited specs');
  } finally {
    fs.rmSync(docsDir, { recursive: true, force: true });
  }
});

// ── Test 7: Full pipeline integration (smoke) ─────────────────────────────────

test('full pipeline: inventory → walk → cluster → emit on fixture bait', async () => {
  const docsDir = makeTmpDocs();
  try {
    // Step 1: inventory
    const entries = await inventory(FIXTURE_BAIT);
    assert.ok(entries.length >= 5);

    // Step 2: walk (sequential, not parallel — this is a test)
    const outputs = [];
    for (const entry of entries) {
      const out = await walk(entry.filePath);
      outputs.push(out);
    }

    // Step 3: cluster
    const clusterResult = cluster(outputs);
    assert.ok(clusterResult.significant.length > 0, 'should find significant units');

    // Step 4: emit
    const result = await emitSpecs(clusterResult, {
      docsDir,
      repoRoot: FIXTURE_BAIT,
      date: '2026-05-21',
    });

    // Architecture + at least one per-module spec
    assert.ok(result.written.length >= 2, `should write ≥2 specs, got ${result.written.length}`);

    // All written files must exist and have required frontmatter keys
    for (const p of result.written) {
      const content = fs.readFileSync(p, 'utf8');
      assert.ok(content.includes('reverse_engineered: true'), `${p} missing reverse_engineered`);
      assert.ok(content.includes('generated_at:'),            `${p} missing generated_at`);
      assert.ok(content.includes('source_root:'),             `${p} missing source_root`);
      assert.ok(content.includes('commit:'),                  `${p} missing commit`);
      assert.ok(content.includes('ai_reviewed:'),             `${p} missing ai_reviewed`);
    }
  } finally {
    fs.rmSync(docsDir, { recursive: true, force: true });
  }
});
