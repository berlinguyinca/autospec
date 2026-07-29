#!/usr/bin/env node
// doc-scaffold.mjs — deterministic, project-grounded feature-inventory scaffolder.
//
// The audience-doc generator renders per-audience prose from a feature inventory
// (.autospec/doc-features.json). For a NEW project that inventory is hand-authored
// from scratch. This module bootstraps a SKELETON inventory grounded in the repo's
// own shape — with NO project-specific hardcoding — so the authoring pass starts
// from a real list of features instead of a blank page.
//
// Candidate-derivation strategy (project-transparent, priority order):
//   1. Existing topic docs — non-index `*.md` under docs/<topic>/ subdirectories
//      that are NOT the generated audience dirs (docs/user|developer|admin|general).
//      This is the strongest "what are the features" signal.
//   2. Else source module dirs — immediate children of apps/, packages/, services/,
//      cmd/, modules/, crates/, or (fallback) first-level dirs under src/.
//   3. Else a single `overview` feature.
//
// Pure: scaffoldFeatures() never writes. writeScaffold() is the only writer and
// NEVER clobbers an existing fixture.
//
// Exports:
//   scaffoldFeatures({ repoRoot, maxFeatures=16, sourceDirs }) → { features, source }
//   writeScaffold({ repoRoot, audiences, maxFeatures }) → { path, written, features, source }

import fs from 'node:fs';
import path from 'node:path';

import { DEFAULT_AUDIENCES } from './doc-config.mjs';

// ── Constants ─────────────────────────────────────────────────────────────────

// Generated audience doc roots — excluded as topic-doc sources (we derive features
// from AUTHORED topic docs, never from the docs we generate).
const AUDIENCE_DIR_NAMES = new Set(['user', 'developer', 'admin', 'general']);

// Doc basenames that are navigation/boilerplate, not a feature.
const NON_FEATURE_DOC_BASENAMES = new Set(['index', 'overview', 'readme']);

// Source roots whose immediate children are features (priority order).
const DEFAULT_SOURCE_DIRS = ['apps', 'packages', 'services', 'cmd', 'modules', 'crates'];

// Directories we never treat as a feature / never walk into. Mirrors the exclusion
// spirit of doc-coverage.mjs (build artifacts, vcs, package caches, vendored trees).
const EXCLUDED_DIR_NAMES = new Set([
  'node_modules', '.git', 'target', 'build', 'dist', 'out', '.autospec',
  'venv', '.venv', 'env', '.env', 'virtualenv', 'site-packages', '__pycache__',
  '.tox', '.mypy_cache', '.pytest_cache', '.ruff_cache', 'eggs', '.eggs',
  '.gradle', '.m2',
  'vendor', 'bower_components', 'Pods', '.cargo', '.pnpm-store', '.yarn',
  '.next', '.nuxt', '.svelte-kit', 'coverage', '.nyc_output',
  '.idea', '.vscode', '.terraform',
]);

// ── Slug / title helpers ────────────────────────────────────────────────────────

// Kebab-case a name into a filesystem-safe slug: lowercase, alnum runs joined by
// single hyphens, no `..`, no `/`, no leading slash, no NUL.
function kebabSlug(name) {
  const slug = String(name)
    .replace(/\0/g, '')
    .replace(/[_\s]+/g, '-')          // underscores / whitespace → hyphen
    .replace(/([a-z0-9])([A-Z])/g, '$1-$2') // camelCase → kebab
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')      // any other char → hyphen
    .replace(/-+/g, '-')
    .replace(/^-+|-+$/g, '');
  return slug;
}

// Humanize a basename into a Title Case title.
function humanize(name) {
  const words = String(name)
    .replace(/[_\-]+/g, ' ')
    .replace(/([a-z0-9])([A-Z])/g, '$1 $2')
    .trim()
    .split(/\s+/)
    .filter(Boolean);
  return words.map(w => w.charAt(0).toUpperCase() + w.slice(1)).join(' ');
}

// ── Filesystem probes ────────────────────────────────────────────────────────────

function listDirs(abs) {
  let entries;
  try { entries = fs.readdirSync(abs, { withFileTypes: true }); } catch { return []; }
  return entries
    .filter(e => e.isDirectory() && !e.name.startsWith('.') && !EXCLUDED_DIR_NAMES.has(e.name))
    .map(e => e.name);
}

function listMarkdown(abs) {
  let entries;
  try { entries = fs.readdirSync(abs, { withFileTypes: true }); } catch { return []; }
  return entries.filter(e => e.isFile() && /\.md$/i.test(e.name)).map(e => e.name);
}

// ── Candidate derivation ────────────────────────────────────────────────────────

// 1. Topic docs: docs/<topic>/*.md for every topic dir that is NOT a generated
//    audience dir. Returns Array<{ slug, title, dir }> (dir = relative topic dir).
function deriveFromTopicDocs(repoRoot) {
  const docsRoot = path.join(repoRoot, 'docs');
  const topicDirs = listDirs(docsRoot).filter(d => !AUDIENCE_DIR_NAMES.has(d));
  const bySlug = new Map();
  for (const topic of topicDirs.sort()) {
    const topicAbs = path.join(docsRoot, topic);
    for (const file of listMarkdown(topicAbs).sort()) {
      const base = path.basename(file, path.extname(file));
      if (NON_FEATURE_DOC_BASENAMES.has(base.toLowerCase())) continue;
      const slug = kebabSlug(base);
      if (!slug || bySlug.has(slug)) continue;
      bySlug.set(slug, {
        slug,
        title: humanize(base),
        glob: `docs/${topic}/${file}`,
      });
    }
  }
  return [...bySlug.values()];
}

// 2. Source module dirs: immediate children of common source roots, else first-
//    level dirs under src/. Returns Array<{ slug, title, glob }>.
function deriveFromSourceDirs(repoRoot, sourceDirs) {
  const roots = (Array.isArray(sourceDirs) && sourceDirs.length) ? sourceDirs : DEFAULT_SOURCE_DIRS;
  const bySlug = new Map();
  for (const root of roots) {
    const rootAbs = path.join(repoRoot, root);
    for (const child of listDirs(rootAbs).sort()) {
      const slug = kebabSlug(child);
      if (!slug || bySlug.has(slug)) continue;
      bySlug.set(slug, { slug, title: humanize(child), glob: `${root}/${child}/**` });
    }
  }
  if (bySlug.size > 0) return [...bySlug.values()];

  // Fallback: first-level dirs under src/.
  const srcAbs = path.join(repoRoot, 'src');
  for (const child of listDirs(srcAbs).sort()) {
    const slug = kebabSlug(child);
    if (!slug || bySlug.has(slug)) continue;
    bySlug.set(slug, { slug, title: humanize(child), glob: `src/${child}/**` });
  }
  return [...bySlug.values()];
}

// ── Skeleton shape ───────────────────────────────────────────────────────────────

// Produce one EMPTY-prose skeleton feature. The authoring pass fills the blanks;
// we never invent prose. code_entry_points is grounded on a derived glob when
// available, else the slug glob.
function skeletonFeature({ slug, title, glob }) {
  const entry = glob || `**/${slug}*`;
  return {
    slug,
    title: title || humanize(slug),
    summary: '',
    spec_sections: { user: [], developer: [], admin: [], general: [], default: [] },
    code_entry_points: [entry],
    data_model: '',
    invariants: '',
    errors: '',
    config_reference: { admin: '', developer: '' },
    algorithm: { developer: '', general: '' },
    config_profiles: { admin: [], developer: [] },
    settings: [],
    implementation_snippets: [],
    rationale: '',
    depends_on: [],
  };
}

// ── Public: scaffoldFeatures ─────────────────────────────────────────────────────

/**
 * scaffoldFeatures — derive a project-grounded skeleton feature inventory.
 *
 * @param {{ repoRoot: string, maxFeatures?: number, sourceDirs?: string[] }} opts
 * @returns {{ features: object[], source: string }}
 */
export function scaffoldFeatures({ repoRoot, maxFeatures = 16, sourceDirs } = {}) {
  if (!repoRoot) return { features: [skeletonFeature({ slug: 'overview', title: 'Overview' })], source: 'overview fallback (no repoRoot)' };

  let candidates = deriveFromTopicDocs(repoRoot);
  let source = 'topic docs (docs/<topic>/*.md)';

  if (candidates.length === 0) {
    candidates = deriveFromSourceDirs(repoRoot, sourceDirs);
    source = 'source module dirs (apps/packages/services/…)';
  }

  if (candidates.length === 0) {
    candidates = [{ slug: 'overview', title: 'Overview', glob: '**' }];
    source = 'overview fallback (no topic docs or source dirs found)';
  }

  // Stable sort by slug, then cap at maxFeatures (first N by name sort).
  candidates.sort((a, b) => (a.slug < b.slug ? -1 : a.slug > b.slug ? 1 : 0));
  const total = candidates.length;
  if (total > maxFeatures) {
    candidates = candidates.slice(0, maxFeatures);
    source += ` — truncated to ${maxFeatures} of ${total}`;
  }

  return { features: candidates.map(skeletonFeature), source };
}

// ── Public: writeScaffold ────────────────────────────────────────────────────────

/**
 * writeScaffold — seed .autospec/doc-features.json if absent; never clobber.
 *
 * @param {{ repoRoot: string, audiences?: object[], maxFeatures?: number }} opts
 * @returns {{ path: string, written: boolean, features?: object[], source?: string }}
 */
export function writeScaffold({ repoRoot, audiences, maxFeatures = 16 } = {}) {
  const target = path.join(repoRoot, '.autospec', 'doc-features.json');

  // Never overwrite an authored / human fixture.
  if (fs.existsSync(target)) {
    return { path: target, written: false };
  }

  const { features, source } = scaffoldFeatures({ repoRoot, maxFeatures });
  const auds = (Array.isArray(audiences) && audiences.length)
    ? audiences
    : DEFAULT_AUDIENCES.map(a => ({ ...a }));

  fs.mkdirSync(path.dirname(target), { recursive: true });
  fs.writeFileSync(target, JSON.stringify({ audiences: auds, features }, null, 2) + '\n', 'utf8');

  return { path: target, written: true, features, source };
}
