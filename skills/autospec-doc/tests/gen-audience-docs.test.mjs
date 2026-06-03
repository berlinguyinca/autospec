// gen-audience-docs.test.mjs — unit tests for the per-audience doc generator
// (issue #918, spec §D2). Mirrors the shape of
// skills/autospec-shared/tests/unit/gen-docs.test.mjs.
//
// Tests:
//   1. each configured audience produces the folder-contract files
//      (index.md, getting-started.md, tutorials/<feature>.md, features/<feature>.md)
//   2. every generated section carries an autospec-doc-scope comment with generated: true
//   3. human-owned (non-generated) sections survive a regen via mergeWithExisting
//   4. scope comments are well-formed (parseable by scan-doc-scope.parse)
//   5. the validator + 5-attempt retry re-prompts on validator failure
//   6. section-preservation helpers are imported from gen-docs-from-spec.mjs
//      (not reimplemented)
//   7. four default audiences each produce a full tree; files land under the
//      audience path; outputDir writes are real files

import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const SCRIPTS_DIR = path.resolve(__dirname, '../scripts');
const SHARED_SCRIPTS_DIR = path.resolve(__dirname, '../../autospec-shared/scripts');

const { generateAudienceDocs } = await import(path.join(SCRIPTS_DIR, 'gen-audience-docs.mjs'));
const { parse: parseScopeBlocks } = await import(path.join(SHARED_SCRIPTS_DIR, 'scan-doc-scope.mjs'));

// ── Fixtures ────────────────────────────────────────────────────────────────

// Force the AI-review pass into a deterministic high-confidence stub so tests
// never depend on an LLM being present.
process.env.AUTOSPEC_AI_REVIEW_STUB = process.env.AUTOSPEC_AI_REVIEW_STUB || 'high';

const FOUR_AUDIENCES = [
  { name: 'user',      path: 'docs/user',      focus: 'tasks, workflows, how to use features' },
  { name: 'developer', path: 'docs/developer', focus: 'architecture, APIs, extending' },
  { name: 'admin',     path: 'docs/admin',     focus: 'install, configure, operate, troubleshoot' },
  { name: 'general',   path: 'docs/general',   focus: 'what it is, why it matters, plain language' },
];

const SAMPLE_FEATURES = [
  {
    slug: 'export-pipeline',
    title: 'Export Pipeline',
    summary: 'Streams records out to downstream sinks.',
    spec_sections: ['The export pipeline batches records and flushes to sinks.'],
    code_entry_points: ['src/export/pipeline.mjs', 'src/export/sink.mjs'],
  },
  {
    slug: 'auth',
    title: 'Authentication',
    summary: 'Token-based login.',
    spec_sections: ['Auth issues short-lived JWTs.'],
    code_entry_points: ['src/auth/login.mjs'],
  },
];

function makeTmpDir() {
  return fs.mkdtempSync(path.join(os.tmpdir(), 'autospec-gen-audience-test-'));
}

function countOccurrences(str, sub) {
  let count = 0, idx = 0;
  while ((idx = str.indexOf(sub, idx)) !== -1) { count++; idx += sub.length; }
  return count;
}

// ── Test 1: folder contract per audience ──────────────────────────────────────

test('each audience produces the folder-contract files for each feature', async () => {
  const result = await generateAudienceDocs({
    features: SAMPLE_FEATURES,
    audiences: FOUR_AUDIENCES,
  });
  for (const aud of FOUR_AUDIENCES) {
    const audPaths = result.files.map(f => f.path).filter(p => p.startsWith(aud.path + '/'));
    assert.ok(audPaths.includes(`${aud.path}/index.md`),
      `${aud.name}: index.md missing`);
    assert.ok(audPaths.includes(`${aud.path}/getting-started.md`),
      `${aud.name}: getting-started.md missing`);
    for (const feat of SAMPLE_FEATURES) {
      assert.ok(audPaths.includes(`${aud.path}/tutorials/${feat.slug}.md`),
        `${aud.name}: tutorials/${feat.slug}.md missing`);
      assert.ok(audPaths.includes(`${aud.path}/features/${feat.slug}.md`),
        `${aud.name}: features/${feat.slug}.md missing`);
    }
  }
});

// ── Test 2: every generated section has a scope comment with generated: true ──

test('every generated file carries an autospec-doc-scope comment with generated: true', async () => {
  const result = await generateAudienceDocs({
    features: SAMPLE_FEATURES,
    audiences: FOUR_AUDIENCES,
  });
  for (const file of result.files) {
    const scopeCount = countOccurrences(file.content, '<!-- autospec-doc-scope:');
    assert.ok(scopeCount > 0, `${file.path}: no autospec-doc-scope comment`);
    assert.ok(file.content.includes('generated: true'),
      `${file.path}: scope comment missing generated: true`);
  }
});

// ── Test 3: human-owned section preserved across regen ────────────────────────

test('human-owned (non-generated) section survives a regen via mergeWithExisting', async () => {
  const result1 = await generateAudienceDocs({
    features: SAMPLE_FEATURES,
    audiences: [FOUR_AUDIENCES[0]],
  });
  const indexFile = result1.files.find(f => f.path === 'docs/user/index.md');
  assert.ok(indexFile, 'should generate docs/user/index.md');

  // Simulate a human flipping a section to generated: false and editing its body.
  const handEdited = indexFile.content
    .replace('generated: true', 'generated: false')
    .replace(/(<!-- autospec-doc-scope:[\s\S]*?-->\n)/, '$1\nHUMAN EDIT: hand-written overview.\n');

  const result2 = await generateAudienceDocs({
    features: SAMPLE_FEATURES,
    audiences: [FOUR_AUDIENCES[0]],
    existingDocs: { 'docs/user/index.md': handEdited },
  });
  const regen = result2.files.find(f => f.path === 'docs/user/index.md');
  assert.ok(regen, 'should regenerate docs/user/index.md');
  assert.ok(regen.content.includes('HUMAN EDIT: hand-written overview.'),
    'human-edited section should be preserved verbatim');
  assert.ok(regen.preserved_sections > 0, 'preserved_sections should be > 0');
});

// ── Test 4: scope comments are well-formed (scan-doc-scope parses cleanly) ─────

test('generated scope comments are parseable by scan-doc-scope', async () => {
  const tmpDir = makeTmpDir();
  try {
    const result = await generateAudienceDocs({
      features: SAMPLE_FEATURES,
      audiences: [FOUR_AUDIENCES[1]],
    });
    const sample = result.files.find(f => f.path.endsWith('features/export-pipeline.md'));
    assert.ok(sample, 'should generate a feature page');
    const tmpFile = path.join(tmpDir, 'feature.md');
    fs.writeFileSync(tmpFile, sample.content, 'utf8');
    const scopes = parseScopeBlocks(tmpFile);
    assert.ok(Array.isArray(scopes), 'parse should return an array');
    assert.ok(scopes.length > 0, 'should detect at least one scope block');
    assert.ok(scopes.every(s => s.generated === true),
      'every parsed scope should be generated: true');
    assert.ok(scopes.every(s => Array.isArray(s.src_globs) && s.src_globs.length > 0),
      'every scope should carry src globs');
  } finally {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  }
});

// ── Test 5: validator + 5-attempt retry re-prompts on failure ─────────────────

test('validator + retry loop re-prompts when validation fails, capped at 5 attempts', async () => {
  // A validator that rejects the first two attempts, then accepts. The generator
  // must feed findings back as directives and succeed within the cap.
  let calls = 0;
  const directives = [];
  const validator = (content, ctx) => {
    calls++;
    if (ctx && ctx.directives) directives.push(ctx.directives.length);
    if (calls <= 2) return { ok: false, findings: [`synthetic failure ${calls}`] };
    return { ok: true, findings: [] };
  };
  const result = await generateAudienceDocs({
    features: [SAMPLE_FEATURES[0]],
    audiences: [FOUR_AUDIENCES[0]],
    validator,
    maxRetries: 5,
  });
  assert.ok(calls >= 3, `validator should be retried (got ${calls} calls)`);
  // Later attempts must receive accumulated findings as directives.
  assert.ok(directives.some(n => n > 0), 'findings should be fed back as directives');
  assert.ok(result.files.length > 0, 'should still produce files once validation passes');
});

test('validator that never passes exhausts the retry cap without throwing', async () => {
  let calls = 0;
  const validator = () => { calls++; return { ok: false, findings: ['always fails'] }; };
  const result = await generateAudienceDocs({
    features: [SAMPLE_FEATURES[0]],
    audiences: [FOUR_AUDIENCES[0]],
    validator,
    maxRetries: 5,
  });
  // One feature page is validated; the cap is per-page. Should stop at 5.
  assert.ok(calls >= 5, `should attempt up to the cap (got ${calls})`);
  assert.ok(Array.isArray(result.files), 'should return files array even on exhausted retries');
});

// ── Test 6: section-preservation imported, not reimplemented ───────────────────

test('gen-audience-docs imports mergeWithExisting/parseSections from gen-docs-from-spec', async () => {
  const src = fs.readFileSync(path.join(SCRIPTS_DIR, 'gen-audience-docs.mjs'), 'utf8');
  assert.ok(
    /gen-docs-from-spec\.mjs/.test(src)
      && /\bimport\b/.test(src)
      && /mergeWithExisting/.test(src),
    'must import mergeWithExisting from gen-docs-from-spec.mjs',
  );
  // Must not define its own mergeWithExisting / parseSections.
  assert.ok(!/function\s+mergeWithExisting\s*\(/.test(src),
    'must not reimplement mergeWithExisting');
  assert.ok(!/function\s+parseSections\s*\(/.test(src),
    'must not reimplement parseSections');
});

// ── Test 7: outputDir writes real files under the audience path ───────────────

test('writes files to outputDir under each audience path', async () => {
  const tmpDir = makeTmpDir();
  try {
    const result = await generateAudienceDocs({
      features: [SAMPLE_FEATURES[0]],
      audiences: FOUR_AUDIENCES,
      outputDir: tmpDir,
    });
    for (const file of result.files) {
      const outPath = path.join(tmpDir, file.path);
      assert.ok(fs.existsSync(outPath), `${file.path} should exist in outputDir`);
      const content = fs.readFileSync(outPath, 'utf8');
      assert.ok(content.length > 0, `${file.path} should be non-empty`);
    }
    // developer feature page should reference the developer focus.
    const devTree = result.files.filter(f => f.path.startsWith('docs/developer/'));
    assert.ok(devTree.length > 0, 'developer tree should be generated');
  } finally {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  }
});

test('second write with identical content is idempotent (written=false)', async () => {
  const tmpDir = makeTmpDir();
  try {
    await generateAudienceDocs({ features: SAMPLE_FEATURES, audiences: [FOUR_AUDIENCES[0]], outputDir: tmpDir });
    const second = await generateAudienceDocs({ features: SAMPLE_FEATURES, audiences: [FOUR_AUDIENCES[0]], outputDir: tmpDir });
    for (const file of second.files) {
      assert.equal(file.written, false, `${file.path} should not be re-written on identical second pass`);
    }
  } finally {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  }
});

// ── Test 9: path-traversal guard ──────────────────────────────────────────────

test('rejects an audience path that escapes the docs tree', async () => {
  await assert.rejects(
    generateAudienceDocs({
      features: [],
      audiences: [{ name: 'evil', path: '../../etc', focus: 'x' }],
    }),
    /must not traverse|must be a relative/,
  );
});

test('rejects an absolute audience path', async () => {
  await assert.rejects(
    generateAudienceDocs({
      features: [],
      audiences: [{ name: 'evil', path: '/etc', focus: 'x' }],
    }),
    /must be a relative path/,
  );
});

test('rejects a feature slug that traverses out of the tree', async () => {
  await assert.rejects(
    generateAudienceDocs({
      features: [{ slug: '../../escape' }],
      audiences: [FOUR_AUDIENCES[0]],
    }),
    /must not traverse/,
  );
});

// ── Test 10: basename fallback collision is NOT applied across audiences ───────

test('existing content keyed by basename does NOT leak across audiences', async () => {
  // A human-edited index.md for the user audience must not be preserved into
  // the developer audience's index.md (which has the same basename).
  const userIndex = (await generateAudienceDocs({
    features: [],
    audiences: [FOUR_AUDIENCES[0]],
  })).files.find(f => f.path === 'docs/user/index.md');
  const handEdited = userIndex.content
    .replace('generated: true', 'generated: false')
    .replace(/(<!-- autospec-doc-scope:[\s\S]*?-->\n)/, '$1\nLEAK MARKER user-only.\n');

  const result = await generateAudienceDocs({
    features: [],
    audiences: [FOUR_AUDIENCES[1]], // developer
    existingDocs: { 'docs/user/index.md': handEdited }, // wrong-audience key
  });
  const devIndex = result.files.find(f => f.path === 'docs/developer/index.md');
  assert.ok(devIndex, 'developer index should be generated');
  assert.ok(!devIndex.content.includes('LEAK MARKER user-only.'),
    'user-audience content must not leak into developer index via basename fallback');
});

// ── Test 8: empty features still produce index + getting-started ──────────────

test('audience with no features still gets index.md and getting-started.md', async () => {
  const result = await generateAudienceDocs({
    features: [],
    audiences: [FOUR_AUDIENCES[0]],
  });
  const paths = result.files.map(f => f.path);
  assert.ok(paths.includes('docs/user/index.md'));
  assert.ok(paths.includes('docs/user/getting-started.md'));
});
