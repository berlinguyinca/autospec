// doc-scaffold.test.mjs — unit tests for skills/autospec-doc/scripts/doc-scaffold.mjs
//
// The scaffolder bootstraps a project-grounded SKELETON feature inventory on ANY
// repo (no project-specific hardcoding). It is pure (scaffoldFeatures) plus one
// guarded writer (writeScaffold) that never clobbers an existing fixture.
//
// Tests:
//   1. derives features from topic docs (docs/apps/*.md + docs/algorithms/*.md),
//      excluding generated audience dirs and index/overview/README, kebab-casing slugs
//   2. falls back to source module dirs (apps/* / packages/*) when no topic docs,
//      skipping build/vendor dirs
//   3. caps at maxFeatures and notes truncation in `source`
//   4. single `overview` fallback when nothing is found
//   5. skeleton shape is valid (per-audience maps present, empty prose, safe slugs)
//   6. writeScaffold writes when absent; does NOT overwrite when present

import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const SCAFFOLD_MOD = path.resolve(__dirname, '../scripts/doc-scaffold.mjs');

const { scaffoldFeatures, writeScaffold } = await import(SCAFFOLD_MOD);

// ── Helpers ─────────────────────────────────────────────────────────────────────

function makeTmpDir() {
  return fs.mkdtempSync(path.join(os.tmpdir(), 'doc-scaffold-test-'));
}

function write(root, rel, content = '') {
  const full = path.join(root, rel);
  fs.mkdirSync(path.dirname(full), { recursive: true });
  fs.writeFileSync(full, content, 'utf8');
}

function slugs(features) {
  return features.map(f => f.slug).sort();
}

// ── 1. Topic-doc derivation ──────────────────────────────────────────────────────

test('derives features from topic docs, excluding audience dirs and index/overview/README', () => {
  const root = makeTmpDir();
  // Topic docs (the strongest "what are the features" signal).
  write(root, 'docs/apps/Export Pipeline.md', '# Export');
  write(root, 'docs/apps/ingest.md', '# Ingest');
  write(root, 'docs/algorithms/peak-detection.md', '# Peaks');
  write(root, 'docs/apps/index.md', '# index');       // excluded
  write(root, 'docs/apps/README.md', '# readme');      // excluded
  write(root, 'docs/algorithms/overview.md', '# ov');  // excluded
  // Generated audience dirs must NOT be treated as topic dirs.
  write(root, 'docs/user/features/export-pipeline.md', '# gen');
  write(root, 'docs/developer/index.md', '# gen');

  const { features, source } = scaffoldFeatures({ repoRoot: root });

  assert.deepEqual(slugs(features), ['export-pipeline', 'ingest', 'peak-detection']);
  assert.match(source, /doc/i);
  // Titles humanized from basenames.
  const byslug = Object.fromEntries(features.map(f => [f.slug, f]));
  assert.equal(byslug['export-pipeline'].title, 'Export Pipeline');
  assert.equal(byslug['peak-detection'].title, 'Peak Detection');

  fs.rmSync(root, { recursive: true, force: true });
});

test('de-dupes topic-doc features by slug', () => {
  const root = makeTmpDir();
  write(root, 'docs/apps/export.md', '# a');
  write(root, 'docs/algorithms/Export.md', '# b'); // same slug
  const { features } = scaffoldFeatures({ repoRoot: root });
  assert.deepEqual(slugs(features), ['export']);
  fs.rmSync(root, { recursive: true, force: true });
});

// ── 2. Source-module-dir fallback ────────────────────────────────────────────────

test('falls back to source module dirs (apps/*, packages/*), skipping build/vendor dirs', () => {
  const root = makeTmpDir();
  write(root, 'apps/web/main.js', '');
  write(root, 'apps/worker/main.js', '');
  write(root, 'packages/core/index.js', '');
  // Build / vendor dirs must be skipped.
  write(root, 'apps/node_modules/dep/index.js', '');
  write(root, 'packages/dist/bundle.js', '');

  const { features, source } = scaffoldFeatures({ repoRoot: root });
  assert.deepEqual(slugs(features), ['core', 'web', 'worker']);
  assert.match(source, /dir/i);
  fs.rmSync(root, { recursive: true, force: true });
});

test('falls back to first-level src/ dirs when no monorepo roots exist', () => {
  const root = makeTmpDir();
  write(root, 'src/auth/index.js', '');
  write(root, 'src/billing/index.js', '');
  const { features } = scaffoldFeatures({ repoRoot: root });
  assert.deepEqual(slugs(features), ['auth', 'billing']);
  fs.rmSync(root, { recursive: true, force: true });
});

// ── 3. maxFeatures cap ────────────────────────────────────────────────────────────

test('caps at maxFeatures (first N by name sort) and notes truncation', () => {
  const root = makeTmpDir();
  for (const n of ['delta', 'alpha', 'charlie', 'bravo', 'echo']) {
    write(root, `docs/apps/${n}.md`, '# x');
  }
  const { features, source } = scaffoldFeatures({ repoRoot: root, maxFeatures: 3 });
  assert.equal(features.length, 3);
  assert.deepEqual(slugs(features), ['alpha', 'bravo', 'charlie']);
  assert.match(source, /trunc|cap|5/i);
  fs.rmSync(root, { recursive: true, force: true });
});

// ── 4. overview fallback ──────────────────────────────────────────────────────────

test('single overview feature when nothing is found', () => {
  const root = makeTmpDir();
  const { features, source } = scaffoldFeatures({ repoRoot: root });
  assert.equal(features.length, 1);
  assert.equal(features[0].slug, 'overview');
  assert.match(source, /overview|fallback/i);
  fs.rmSync(root, { recursive: true, force: true });
});

// ── 5. Skeleton shape ─────────────────────────────────────────────────────────────

test('skeleton shape is valid: per-audience maps present, empty prose, safe slugs', () => {
  const root = makeTmpDir();
  write(root, 'docs/apps/export pipeline.md', '# x');
  const { features } = scaffoldFeatures({ repoRoot: root });
  const f = features[0];

  assert.equal(f.slug, 'export-pipeline');
  // Safe slug.
  assert.doesNotMatch(f.slug, /[/\\\0]/);
  assert.ok(!f.slug.startsWith('/'));
  assert.ok(!f.slug.includes('..'));

  assert.equal(f.summary, '');
  assert.deepEqual(f.spec_sections, { user: [], developer: [], admin: [], general: [], default: [] });
  assert.ok(Array.isArray(f.code_entry_points) && f.code_entry_points.length >= 1);
  assert.equal(f.data_model, '');
  assert.equal(f.invariants, '');
  assert.equal(f.errors, '');
  assert.deepEqual(f.config_reference, { admin: '', developer: '' });
  assert.equal(f.rationale, '');
  assert.deepEqual(f.depends_on, []);
  assert.ok(typeof f.title === 'string' && f.title.length > 0);

  fs.rmSync(root, { recursive: true, force: true });
});

test('code_entry_points are grounded on a real path when derivable', () => {
  const root = makeTmpDir();
  write(root, 'apps/web/main.js', '');
  const { features } = scaffoldFeatures({ repoRoot: root });
  const f = features.find(x => x.slug === 'web');
  assert.ok(f.code_entry_points.some(g => g.includes('apps/web')));
  fs.rmSync(root, { recursive: true, force: true });
});

// ── 6. writeScaffold ──────────────────────────────────────────────────────────────

test('writeScaffold writes the fixture when absent', () => {
  const root = makeTmpDir();
  write(root, 'docs/apps/export.md', '# x');
  const res = writeScaffold({ repoRoot: root });
  assert.equal(res.written, true);
  assert.ok(res.path.endsWith(path.join('.autospec', 'doc-features.json')));
  const parsed = JSON.parse(fs.readFileSync(res.path, 'utf8'));
  assert.ok(Array.isArray(parsed.features) && parsed.features.length === 1);
  assert.ok(Array.isArray(parsed.audiences) && parsed.audiences.length >= 4);
  assert.equal(parsed.features[0].slug, 'export');
  fs.rmSync(root, { recursive: true, force: true });
});

test('writeScaffold does NOT overwrite an existing fixture', () => {
  const root = makeTmpDir();
  write(root, 'docs/apps/export.md', '# x');
  const existing = '{"features":[{"slug":"hand-authored"}]}';
  write(root, '.autospec/doc-features.json', existing);
  const res = writeScaffold({ repoRoot: root });
  assert.equal(res.written, false);
  assert.equal(fs.readFileSync(path.join(root, '.autospec', 'doc-features.json'), 'utf8'), existing);
  fs.rmSync(root, { recursive: true, force: true });
});

test('writeScaffold honours a passed audiences list', () => {
  const root = makeTmpDir();
  const audiences = [{ name: 'user', path: 'docs/user', focus: 'tasks' }];
  const res = writeScaffold({ repoRoot: root, audiences });
  const parsed = JSON.parse(fs.readFileSync(res.path, 'utf8'));
  assert.deepEqual(parsed.audiences, audiences);
  fs.rmSync(root, { recursive: true, force: true });
});
