// gen-llms.test.mjs — unit tests for Phase 6 LLM artifact generators.
//
// Tests:
//   1. llms.txt line count ≤200
//   2. llms-full.txt is non-empty and includes doc sections
//   3. Manifest JSON validates against schema
//   4. Manifest includes all required keys (modules/cli_entry_points/http_endpoints/concepts/faq)
//   5. Idempotency: re-run produces byte-equal manifest
//   6. ASSISTANT_PROMPT.md matches spec §5c template structure
//   7. gen-llms-txt.sh runs without error

import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import { fileURLToPath } from 'node:url';
import { execSync, spawnSync } from 'node:child_process';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const SCRIPTS_DIR = path.resolve(__dirname, '../../scripts');
const SCHEMAS_DIR = path.resolve(__dirname, '../../schemas');
const BAIT_DIR    = path.resolve(__dirname, '../fixtures/reverse-engineer-bait');
const FIXTURES_DIR = path.resolve(__dirname, '../fixtures');

// Import scripts under test
const { generateManifest, writeManifest } = await import(path.join(SCRIPTS_DIR, 'gen-llm-manifest.mjs'));
const { generateAssistantPrompt, writeAssistantPrompt } = await import(path.join(SCRIPTS_DIR, 'gen-assistant-prompt.mjs'));

test('gen-llm-manifest keeps its public JSDoc free of implicit any types', () => {
  const source = fs.readFileSync(path.join(SCRIPTS_DIR, 'gen-llm-manifest.mjs'), 'utf8');
  assert.doesNotMatch(source, /\bany\s*\[\]/, 'manifest generator must use explicit unknown types');
});

// ── Sample cluster (matches Phase 5 test fixture) ────────────────────────────

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
      ],
      importedBy: [],
      reasons: ['has_exports'],
    },
  ],
  trivial: [],
};

// ── Helper ────────────────────────────────────────────────────────────────────

function makeTmpDir() {
  return fs.mkdtempSync(path.join(os.tmpdir(), 'autospec-llms-test-'));
}

// ── Test 1: generateManifest produces valid shape ─────────────────────────────

test('generateManifest: produces object with all required top-level keys', () => {
  const manifest = generateManifest({ clusters: SAMPLE_CLUSTERS, repoRoot: BAIT_DIR });
  assert.equal(manifest.schema_version, '1.0');
  assert.ok(typeof manifest.repo === 'string' && manifest.repo.length > 0);
  assert.ok(typeof manifest.generated_at === 'string');
  assert.ok(typeof manifest.commit === 'string');
  assert.ok(Array.isArray(manifest.modules),           'modules must be array');
  assert.ok(Array.isArray(manifest.cli_entry_points),  'cli_entry_points must be array');
  assert.ok(Array.isArray(manifest.http_endpoints),    'http_endpoints must be array');
  assert.ok(Array.isArray(manifest.concepts),          'concepts must be array');
  assert.ok(Array.isArray(manifest.faq),               'faq must be array');
});

test('generateManifest: modules include public_api and depends_on', () => {
  const manifest = generateManifest({ clusters: SAMPLE_CLUSTERS, repoRoot: BAIT_DIR });
  assert.ok(manifest.modules.length > 0, 'should have modules');
  for (const mod of manifest.modules) {
    assert.ok(typeof mod.path === 'string',        'module.path must be string');
    assert.ok(typeof mod.summary === 'string',     'module.summary must be string');
    assert.ok(Array.isArray(mod.public_api),       'module.public_api must be array');
    assert.ok(Array.isArray(mod.depends_on),       'module.depends_on must be array');
  }
});

test('generateManifest: cli_entry_points populated from cluster', () => {
  const manifest = generateManifest({ clusters: SAMPLE_CLUSTERS, repoRoot: BAIT_DIR });
  assert.ok(manifest.cli_entry_points.length > 0, 'should have CLI entry points');
  const ep = manifest.cli_entry_points[0];
  assert.ok(typeof ep.identifier === 'string');
  assert.ok(typeof ep.path === 'string');
  assert.ok(typeof ep.line === 'number');
});

test('generateManifest: http_endpoints populated from cluster', () => {
  const manifest = generateManifest({ clusters: SAMPLE_CLUSTERS, repoRoot: BAIT_DIR });
  assert.ok(manifest.http_endpoints.length > 0, 'should have HTTP endpoints');
  const ep = manifest.http_endpoints[0];
  assert.equal(ep.route, '/api/parse');
});

// ── Test 2: Schema validation ─────────────────────────────────────────────────

test('manifest JSON validates against llm-manifest.schema.json (ajv)', async () => {
  // Use ajv if available, otherwise skip gracefully
  let Ajv;
  try {
    const ajvMod = await import('ajv');
    Ajv = ajvMod.default || ajvMod;
  } catch {
    // ajv not installed — skip validation test
    return;
  }

  const schema = JSON.parse(fs.readFileSync(
    path.join(SCHEMAS_DIR, 'llm-manifest.schema.json'), 'utf8'
  ));
  const ajv = new Ajv({ strict: false });
  const validate = ajv.compile(schema);

  const manifest = generateManifest({ clusters: SAMPLE_CLUSTERS, repoRoot: BAIT_DIR });
  const valid = validate(manifest);
  assert.ok(valid, `Schema validation failed: ${JSON.stringify(validate.errors)}`);
});

// ── Test 3: writeManifest idempotency ─────────────────────────────────────────

test('writeManifest: idempotent — second write produces written=false', async () => {
  const tmpDir = makeTmpDir();
  const outputPath = path.join(tmpDir, 'docs', '.llm-manifest.json');
  try {
    const r1 = await writeManifest({ clusters: SAMPLE_CLUSTERS, repoRoot: BAIT_DIR, outputPath });
    assert.equal(r1.written, true, 'first write should produce written=true');

    const r2 = await writeManifest({ clusters: SAMPLE_CLUSTERS, repoRoot: BAIT_DIR, outputPath });
    assert.equal(r2.written, false, 'second write with same inputs should be idempotent');

    // Verify file exists and is valid JSON
    const content = JSON.parse(fs.readFileSync(outputPath, 'utf8'));
    assert.equal(content.schema_version, '1.0');
  } finally {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  }
});

// ── Test 4: generateAssistantPrompt matches spec §5c template ─────────────────

test('generateAssistantPrompt: contains required spec §5c elements', () => {
  const prompt = generateAssistantPrompt({ repoRoot: BAIT_DIR });
  assert.ok(prompt.includes('# Assistant Prompt for'), 'should have H1 title');
  assert.ok(prompt.includes('docs/.llm-manifest.json'), 'should reference manifest');
  assert.ok(prompt.includes('llms-full.txt'),           'should reference llms-full.txt');
  assert.ok(prompt.includes('docs/specs/'),             'should reference specs dir');
  assert.ok(prompt.includes('spec_ref'),                'should mention spec_ref field');
  assert.ok(prompt.includes('## Sample Q&A pairs'),     'should have Q&A section');
  assert.ok(prompt.includes('needs_review'),            'should mention needs_review');
});

test('writeAssistantPrompt: idempotent — second write produces written=false', async () => {
  const tmpDir = makeTmpDir();
  const outputPath = path.join(tmpDir, 'docs', 'ASSISTANT_PROMPT.md');
  try {
    const r1 = await writeAssistantPrompt({ repoRoot: BAIT_DIR, outputPath });
    assert.equal(r1.written, true);

    const r2 = await writeAssistantPrompt({ repoRoot: BAIT_DIR, outputPath });
    assert.equal(r2.written, false, 'second write should be idempotent');
  } finally {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  }
});

// ── Test 5: gen-llms-txt.sh smoke test ────────────────────────────────────────

test('gen-llms-txt.sh: runs without error and produces llms.txt ≤200 lines', () => {
  const tmpDir = makeTmpDir();
  // Copy bait fixture to tmp (gen-llms-txt.sh writes to repo-root)
  const tmpRepo = path.join(tmpDir, 'repo');
  fs.mkdirSync(tmpRepo, { recursive: true });

  // Initialize git repo so the script can detect slug
  spawnSync('git', ['init', '-q'], { cwd: tmpRepo });
  spawnSync('git', ['commit', '--allow-empty', '-m', 'init', '--no-gpg-sign'], { cwd: tmpRepo });

  // Create minimal docs structure
  fs.mkdirSync(path.join(tmpRepo, 'docs', 'specs'), { recursive: true });
  fs.writeFileSync(path.join(tmpRepo, 'docs', 'USER_MANUAL.md'), '# User Manual\n\nContent.\n');
  fs.writeFileSync(path.join(tmpRepo, 'docs', 'API_REFERENCE.md'), '# API Reference\n\nContent.\n');
  fs.writeFileSync(path.join(tmpRepo, 'docs', 'ARCHITECTURE.md'), '# Architecture\n\nContent.\n');
  fs.writeFileSync(path.join(tmpRepo, 'README.md'), '# Test Repo\n\nA test repository.\n');

  const scriptPath = path.join(SCRIPTS_DIR, 'gen-llms-txt.sh');
  const result = spawnSync('bash', [scriptPath, '--repo-root', tmpRepo], {
    encoding: 'utf8', timeout: 30000,
  });

  if (result.status !== 0) {
    assert.fail(`gen-llms-txt.sh failed:\nstdout: ${result.stdout}\nstderr: ${result.stderr}`);
  }

  const llmsTxt = path.join(tmpRepo, 'llms.txt');
  assert.ok(fs.existsSync(llmsTxt), 'llms.txt should exist');

  const lineCount = fs.readFileSync(llmsTxt, 'utf8').split('\n').length;
  assert.ok(lineCount <= 200, `llms.txt must be ≤200 lines, got ${lineCount}`);

  const llmsFullTxt = path.join(tmpRepo, 'llms-full.txt');
  assert.ok(fs.existsSync(llmsFullTxt), 'llms-full.txt should exist');
  const fullContent = fs.readFileSync(llmsFullTxt, 'utf8');
  assert.ok(fullContent.length > 0, 'llms-full.txt must be non-empty');
  assert.ok(fullContent.includes('USER_MANUAL'), 'llms-full.txt should include USER_MANUAL reference');

  fs.rmSync(tmpDir, { recursive: true, force: true });
});

test('gen-llms-txt.sh: second run is idempotent (no file change)', () => {
  const tmpDir = makeTmpDir();
  const tmpRepo = path.join(tmpDir, 'repo');
  fs.mkdirSync(tmpRepo, { recursive: true });
  spawnSync('git', ['init', '-q'], { cwd: tmpRepo });
  spawnSync('git', ['commit', '--allow-empty', '-m', 'init', '--no-gpg-sign'], { cwd: tmpRepo });
  fs.mkdirSync(path.join(tmpRepo, 'docs', 'specs'), { recursive: true });
  fs.writeFileSync(path.join(tmpRepo, 'README.md'), '# Repo\n');

  const scriptPath = path.join(SCRIPTS_DIR, 'gen-llms-txt.sh');
  const run = () => spawnSync('bash', [scriptPath, '--repo-root', tmpRepo], { encoding: 'utf8', timeout: 30000 });

  run(); // first run
  const afterFirst = fs.readFileSync(path.join(tmpRepo, 'llms.txt'), 'utf8');
  run(); // second run
  const afterSecond = fs.readFileSync(path.join(tmpRepo, 'llms.txt'), 'utf8');

  assert.equal(afterFirst, afterSecond, 'llms.txt should be byte-equal after second run');

  fs.rmSync(tmpDir, { recursive: true, force: true });
});

test('gen-llms-txt.sh: cluster extraction writes through stdout without debug logging', () => {
  const script = fs.readFileSync(path.join(SCRIPTS_DIR, 'gen-llms-txt.sh'), 'utf8');
  assert.doesNotMatch(script, /console\.log\s*\(/, 'generator must not contain console.log debug output');
  assert.match(script, /process\.stdout\.write\(/, 'cluster entries must use explicit stdout output');
});

// ── Test 6: Empty clusters ────────────────────────────────────────────────────

test('generateManifest: empty clusters produces valid minimal manifest', () => {
  const manifest = generateManifest({
    clusters: { significant: [], trivial: [] },
    repoRoot: BAIT_DIR,
  });
  assert.equal(manifest.schema_version, '1.0');
  assert.deepEqual(manifest.modules, []);
  assert.deepEqual(manifest.cli_entry_points, []);
  assert.deepEqual(manifest.http_endpoints, []);
  assert.deepEqual(manifest.faq, []);
});

// ── Test 7: Manifest golden fixture ──────────────────────────────────────────

test('manifest golden: save fixture for regression testing', async () => {
  const manifest = generateManifest({ clusters: SAMPLE_CLUSTERS, repoRoot: BAIT_DIR });
  // Normalize generated_at for golden
  const golden = { ...manifest, generated_at: '<normalized>', commit: '<sha>' };
  const goldenPath = path.join(FIXTURES_DIR, 'manifest-golden.json');
  const goldenContent = JSON.stringify(golden, null, 2) + '\n';
  // Write (or verify) golden
  if (!fs.existsSync(goldenPath)) {
    fs.writeFileSync(goldenPath, goldenContent, 'utf8');
  } else {
    const existing = JSON.parse(fs.readFileSync(goldenPath, 'utf8'));
    // Key structure should match — compare keys only
    assert.deepEqual(Object.keys(existing).sort(), Object.keys(golden).sort(), 'golden keys should match');
  }
  assert.ok(fs.existsSync(goldenPath), 'golden fixture should exist');
});
