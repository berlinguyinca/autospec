#!/usr/bin/env node
// gen-llms-full.mjs — deterministic LLM-ingest concatenator (issue #921, spec §D5).
//
// Exports:
//   generateLlmsFull({ pages }) → string
//     Concatenates every generated page wrapped in pinned delimiters with
//     chunk markers every ~30k tokens (~120k chars). Deterministic: pages are
//     sorted by path before concatenation so output is byte-stable regardless
//     of input order.
//
//   fillManifest(manifest, pages) → void
//     Fills modules, concepts, and faq fields of an existing manifest object
//     from page content (H1 headings → modules; <!-- autospec-concept: -->
//     markers → concepts; ### Q: headings → faq). Non-destructive: existing
//     entries are preserved; duplicates (by name/path) are skipped.
//
//   writeLlmsFull({ pages, outputPath }) → Promise<{ written: boolean, path: string }>
//     Writes llms-full.txt; skips write if byte-equal to existing file (idempotent).
//
// Delimiter format (PINNED — spec §D5 shared-contracts):
//   <!-- llms: audience=<a> feature=<f> -->
//   ...page content...
//   <!-- /llms: audience=<a> feature=<f> -->
//
// Chunk marker format:
//   <!-- llms-chunk: chunk=<N> approx_tokens=<T> -->
//
// Token budget: ~30000 tokens ≈ 120000 chars (4 chars/token heuristic).
// A chunk marker is emitted whenever accumulated content crosses a
// TOKEN_BUDGET_CHARS boundary.

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));

// ── Constants ─────────────────────────────────────────────────────────────────

/** ~30k tokens × 4 chars/token heuristic */
const TOKEN_BUDGET_CHARS = 120_000;

// ── Internal helpers ──────────────────────────────────────────────────────────

/**
 * Build the pinned open delimiter for a page.
 * feature=none when the page has no feature association.
 */
function openDelimiter(audience, feature) {
  return `<!-- llms: audience=${audience} feature=${feature || 'none'} -->`;
}

/**
 * Build the pinned close delimiter for a page.
 */
function closeDelimiter(audience, feature) {
  return `<!-- /llms: audience=${audience} feature=${feature || 'none'} -->`;
}

/**
 * Build a chunk marker at the given chunk number and accumulated char count.
 * approx_tokens is a 4-chars/token estimate.
 */
function chunkMarker(chunkNum, charCount) {
  const approxTokens = Math.round(charCount / 4);
  return `<!-- llms-chunk: chunk=${chunkNum} approx_tokens=${approxTokens} -->`;
}

// ── Internal helpers (Track E) ────────────────────────────────────────────────

/**
 * Derive a URL-safe anchor slug from a heading string.
 * Mirrors GitHub-flavored Markdown anchor rules: lowercase, spaces→hyphens,
 * strip non-alphanumeric-hyphen chars.
 */
function slugify(text) {
  return text.toLowerCase().replace(/[^\w\s-]/g, '').replace(/\s+/g, '-').replace(/-+/g, '-');
}

/**
 * Extract the first H1 text from a page's content, falling back to the
 * path's basename.
 */
function pageTitle(page) {
  const m = (page.content || '').match(/^#\s+(.+)/m);
  return m ? m[1].trim() : path.basename(page.path, '.md');
}

/**
 * Extract the first real prose line of a page: skip the H1, any multi-line HTML
 * comment block (e.g. the autospec-doc-scope comment whose interior `src:` line
 * must NOT be mistaken for prose), other headings, and the italic audience
 * boilerplate line (`_Reference for the … audience._`). Returns null if none.
 */
function firstProseLine(content) {
  const lines = (content || '').split('\n');
  let pastH1 = false;
  let inComment = false;
  for (const ln of lines) {
    const t = ln.trim();
    if (inComment) { if (t.includes('-->')) inComment = false; continue; }
    if (t.startsWith('<!--')) { if (!t.includes('-->')) inComment = true; continue; }
    if (!pastH1) { if (/^#\s+/.test(t)) pastH1 = true; continue; }
    if (!t) continue;
    if (/^#+\s/.test(t)) continue;          // headings
    if (/^_.*_$/.test(t)) continue;         // italic audience boilerplate
    return t;
  }
  return null;
}

/**
 * Extract a one-line summary for a page: the first real prose line (see
 * firstProseLine), falling back to the H1 title.
 */
function pageSummary(page) {
  return firstProseLine(page.content) || pageTitle(page);
}

/**
 * Parse the `src: [...]` list out of a page's first autospec-doc-scope comment.
 * These are the feature's code_entry_points — used for source→doc reverse
 * routing and real public_api extraction. Returns [] when no scope/src present.
 */
function parseScopeSrc(content) {
  const block = (content || '').match(/<!--\s*autospec-doc-scope:[\s\S]*?-->/);
  if (!block) return [];
  const srcM = block[0].match(/src:\s*\[([^\]]*)\]/);
  if (!srcM) return [];
  return [...srcM[1].matchAll(/"([^"]+)"|'([^']+)'/g)].map(m => m[1] || m[2]);
}

/**
 * Best-effort extraction of the public symbol surface of a source file: ESM
 * exports (function/const/let/var/class + `export { … }`) and module-level
 * Python def/class. Returns [] when the file is absent, a glob, or unreadable.
 */
function extractExports(absPath) {
  let src;
  try { src = fs.readFileSync(absPath, 'utf8'); } catch { return []; }
  const out = [];
  // Accept only valid identifiers — guards against matching `export { … }` that
  // appears inside this file's own JSDoc/comments or string literals.
  const push = (n) => { const s = (n || '').trim(); if (/^[A-Za-z_$][\w$]*$/.test(s) && !out.includes(s)) out.push(s); };
  for (const m of src.matchAll(/export\s+(?:async\s+)?function\s+([A-Za-z0-9_$]+)/g)) push(m[1]);
  for (const m of src.matchAll(/export\s+(?:const|let|var|class)\s+([A-Za-z0-9_$]+)/g)) push(m[1]);
  for (const m of src.matchAll(/export\s*\{([^}]+)\}/g)) {
    for (const part of m[1].split(',')) push(part.split(/\s+as\s+/)[0]);
  }
  for (const m of src.matchAll(/^(?:def|class)\s+([A-Za-z0-9_]+)/gm)) push(m[1]);
  return out;
}

/**
 * Group sorted pages by their audience, returning
 * Array<{ audience: string, pages: page[] }> in deterministic order.
 */
function groupByAudience(sorted) {
  const map = new Map();
  for (const page of sorted) {
    if (!map.has(page.audience)) map.set(page.audience, []);
    map.get(page.audience).push(page);
  }
  return [...map.entries()].map(([audience, pages]) => ({ audience, pages }));
}

// ── Public API ────────────────────────────────────────────────────────────────

/**
 * Generate a deterministic llms.txt string conforming to llmstxt.org:
 *   - single H1 title
 *   - blockquote one-line summary
 *   - one H2 section per audience with described links
 *
 * @param {{
 *   pages: Array<{ audience: string, feature: string|null, path: string, content: string }>,
 *   title?: string,
 *   summary?: string,
 * }} opts
 * @returns {string}
 */
export function generateLlmsIndex({ pages = [], title = 'autospec', summary = 'Multi-harness LLM-driven CI/CD skill family for autonomous feature decomposition, implementation, and review.' } = {}) {
  // Sort deterministically by path.
  const sorted = [...pages].sort((a, b) => a.path < b.path ? -1 : a.path > b.path ? 1 : 0);
  const groups = groupByAudience(sorted);

  const parts = [];

  // Single H1
  parts.push(`# ${title}`);
  parts.push('');

  // Blockquote summary
  parts.push(`> ${summary}`);
  parts.push('');

  if (groups.length === 0) {
    // No pages — emit a placeholder note section
    parts.push('## Documentation');
    parts.push('');
    parts.push('No documentation pages have been generated yet.');
    parts.push('');
  } else {
    // One H2 section per audience, with described links
    for (const { audience, pages: grpPages } of groups) {
      const sectionTitle = audience.charAt(0).toUpperCase() + audience.slice(1);
      parts.push(`## ${sectionTitle} Documentation`);
      parts.push('');
      for (const pg of grpPages) {
        const t = pageTitle(pg);
        const desc = pageSummary(pg);
        parts.push(`- [${t}](${pg.path}): ${desc}`);
      }
      parts.push('');
    }
  }

  return parts.join('\n');
}

/**
 * Write llms.txt to outputPath. Idempotent: skips write if byte-equal.
 *
 * @param {{
 *   pages: Array<object>,
 *   outputPath: string,
 *   title?: string,
 *   summary?: string,
 * }} opts
 * @returns {Promise<{ written: boolean, path: string }>}
 */
export async function writeLlmsIndex({ pages, outputPath, title, summary }) {
  const content = generateLlmsIndex({ pages, title, summary });

  fs.mkdirSync(path.dirname(outputPath), { recursive: true });

  let existing = null;
  try { existing = fs.readFileSync(outputPath, 'utf8'); } catch {}

  const written = existing !== content;
  if (written) fs.writeFileSync(outputPath, content, 'utf8');

  return { written, path: outputPath };
}

/**
 * Generate a deterministic, byte-stable llms-full.txt string from page objects.
 * Track E additions: top ToC with anchors, per-section summary + approx_tokens,
 * reverse-routing block, generated_at + commit freshness stamp.
 *
 * @param {{
 *   pages: Array<{ audience: string, feature: string|null, path: string, content: string }>,
 *   generatedAt?: string,   // ISO timestamp; omit to use current time (non-idempotent)
 *   commit?: string,        // git commit hash; omit to skip
 * }} opts
 * @returns {string}
 */
export function generateLlmsFull({ pages = [], generatedAt, commit } = {}) {
  if (!pages || pages.length === 0) return '';

  // Sort deterministically by path so output is stable regardless of input order.
  const sorted = [...pages].sort((a, b) => a.path < b.path ? -1 : a.path > b.path ? 1 : 0);

  // ── Track E: build ToC entries ──────────────────────────────────────────────
  // Each page gets an anchor derived from its path slug.
  const tocEntries = sorted.map(page => {
    const title = pageTitle(page);
    const anchor = slugify(page.path.replace(/[/\\]/g, '-').replace(/\.md$/, ''));
    return { page, title, anchor };
  });

  const headerParts = [];

  // Top-level freshness stamp (generated_at + commit) — only emitted when
  // generatedAt is explicitly provided so that callers without a fixed stamp
  // stay byte-stable across calls.
  if (generatedAt) {
    headerParts.push(`<!-- generated_at=${generatedAt}${commit ? ` commit=${commit}` : ''} -->`);
    headerParts.push('');
  }

  // Table of Contents
  headerParts.push('## Table of Contents');
  headerParts.push('');
  for (const { title, anchor } of tocEntries) {
    headerParts.push(`- [${title}](#${anchor})`);
  }
  headerParts.push('');

  // Reverse-routing block: source_file → doc paths. Built from each page's
  // autospec-doc-scope `src:` list so an LLM can answer "where are the docs for
  // src/foo.mjs?". Pages without a scope contribute nothing (no identity noise).
  const routeMap = new Map();
  for (const page of sorted) {
    for (const src of parseScopeSrc(page.content)) {
      if (!routeMap.has(src)) routeMap.set(src, new Set());
      routeMap.get(src).add(page.path);
    }
  }
  headerParts.push('<!-- reverse-routing');
  for (const src of [...routeMap.keys()].sort()) {
    const docs = [...routeMap.get(src)].sort().join(', ');
    headerParts.push(`  ${src} -> ${docs}`);
  }
  headerParts.push('-->');
  headerParts.push('');

  // ── Page blocks ─────────────────────────────────────────────────────────────
  const pageParts = [];
  let accChars = headerParts.join('\n').length;
  let chunkNum = 1;

  for (const { page, anchor } of tocEntries) {
    const open  = openDelimiter(page.audience, page.feature);
    const close = closeDelimiter(page.audience, page.feature);
    const body  = (page.content || '').trimEnd();
    const tokens = approxTokens(body);

    // Per-section summary + approx_tokens annotation (Track E)
    const summary = pageSummary(page);
    const sectionHeader = `<!-- section-summary: ${summary} approx_tokens=${tokens} id=${anchor} -->`;

    // Emit chunk marker BEFORE this page if we would cross the budget boundary.
    if (accChars > 0 && (accChars + open.length + body.length) > TOKEN_BUDGET_CHARS * chunkNum) {
      const marker = chunkMarker(chunkNum, accChars);
      pageParts.push(marker);
      accChars += marker.length + 1;
      chunkNum++;
    }

    const block = [sectionHeader, open, body, close, ''].join('\n');
    pageParts.push(block);
    accChars += block.length;

    // Also emit a marker AFTER this page if the page itself was large enough to
    // cross the next budget boundary (handles single-page inputs > TOKEN_BUDGET_CHARS).
    while (accChars > TOKEN_BUDGET_CHARS * chunkNum) {
      const marker = chunkMarker(chunkNum, accChars);
      pageParts.push(marker);
      accChars += marker.length + 1;
      chunkNum++;
    }
  }

  return headerParts.join('\n') + pageParts.join('\n');
}

/**
 * Approximate token count for a string using the 4-chars/token heuristic.
 * @param {string} text
 * @returns {number}
 */
function approxTokens(text) {
  return Math.max(1, Math.round((text || '').length / 4));
}

/**
 * Fill modules, concepts, cli_entry_points, and faq in a manifest object from
 * page content.  Non-destructive: existing entries are kept; duplicates skipped.
 *
 * Extraction rules:
 *   modules          — one entry per page; name = H1 text; summary = first real
 *                      prose line (the scope comment + audience boilerplate are
 *                      skipped); public_api = real exported symbols read from the
 *                      page's code_entry_points (scope `src:`) under repoRoot,
 *                      supplemented by backtick tokens inside ## CLI / ## API
 *                      sections; doc = page.path; source_anchor = "<path>#L1";
 *                      approx_tokens = chars/4 heuristic on full page content.
 *   cli_entry_points — slash-command tokens (`/word-word`) found anywhere in
 *                      the page, deduplicated.
 *   concepts         — <!-- autospec-concept: <name> --> markers; next
 *                      non-empty line = definition; source_anchor = line ref;
 *                      approx_tokens = definition char/4; literal "<name>"
 *                      placeholder is ALWAYS skipped.
 *   faq              — ### Q: <question> / next non-empty line = answer.
 *
 * @param {object} manifest - object with modules, cli_entry_points, concepts,
 *                            faq arrays (mutated in place)
 * @param {Array<{ path: string, content: string }>} pages
 * @returns {void}
 */
export function fillManifest(manifest, pages = [], { repoRoot = process.cwd() } = {}) {
  if (!Array.isArray(manifest.modules))         manifest.modules = [];
  if (!Array.isArray(manifest.cli_entry_points)) manifest.cli_entry_points = [];
  if (!Array.isArray(manifest.concepts))         manifest.concepts = [];
  if (!Array.isArray(manifest.faq))              manifest.faq = [];

  const existingModPaths     = new Set(manifest.modules.map(m => m.path || m.doc));
  const existingConceptNames = new Set(manifest.concepts.map(c => c.name));
  const existingFaqQs        = new Set(manifest.faq.map(f => f.question));
  const existingCliCmds      = new Set(manifest.cli_entry_points.map(e =>
    typeof e === 'string' ? e : e.command));

  for (const page of pages) {
    const content = page.content || '';
    const lines   = content.split('\n');

    // ── modules: one entry per page keyed by path ────────────────────────────
    if (!existingModPaths.has(page.path)) {
      // name = first H1 text (strip leading #s)
      const h1Line  = lines.find(l => /^#\s+/.test(l));
      const name    = h1Line ? h1Line.replace(/^#+\s+/, '').trim()
                             : path.basename(page.path, '.md');

      // summary = first real prose line (skips the scope comment + boilerplate)
      const summary = firstProseLine(content) || name;

      // public_api = real exported symbols from the page's code_entry_points,
      // supplemented by backtick tokens in any ## CLI / ## API sub-section.
      const public_api = [];
      const addApi = (n) => { const s = (n || '').trim(); if (s && !public_api.includes(s)) public_api.push(s); };
      for (const src of parseScopeSrc(content)) {
        if (/[*?[\]]/.test(src)) continue;   // skip globs — only read literal paths
        for (const sym of extractExports(path.resolve(repoRoot, src))) addApi(sym);
      }
      let inApiSection = false;
      for (const ln of lines) {
        if (/^##\s+(CLI|API|Public API|Commands)/i.test(ln)) { inApiSection = true; continue; }
        if (/^##\s+/.test(ln)) { inApiSection = false; continue; }
        if (!inApiSection) continue;
        for (const m2 of ln.matchAll(/`([^`]+)`/g)) addApi(m2[1]);
      }

      manifest.modules.push({
        name,
        summary,
        public_api,
        doc:           page.path,
        path:          page.path,
        source_anchor: `${page.path}#L1`,
        approx_tokens: approxTokens(content),
      });
      existingModPaths.add(page.path);
    }

    // ── cli_entry_points: /slash-command tokens anywhere in page ─────────────
    const slashCmds = [...content.matchAll(/`(\/[\w-]+(?:\s+[\w-]+)?)`/g)]
      .map(m2 => m2[1]);
    for (const cmd of slashCmds) {
      if (!existingCliCmds.has(cmd)) {
        manifest.cli_entry_points.push(cmd);
        existingCliCmds.add(cmd);
      }
    }

    // ── concepts: <!-- autospec-concept: <name> --> ──────────────────────────
    for (let i = 0; i < lines.length; i++) {
      const m = lines[i].match(/<!--\s*autospec-concept:\s*(.+?)\s*-->/);
      if (!m) continue;
      const name = m[1].trim();
      // Skip the literal template placeholder "<name>"
      if (name === '<name>') continue;
      if (existingConceptNames.has(name)) continue;
      // Definition is the next non-empty line after the marker.
      let definition = '';
      for (let j = i + 1; j < lines.length; j++) {
        const def = lines[j].trim();
        if (def) { definition = def; break; }
      }
      manifest.concepts.push({
        name,
        definition,
        source_anchor: `${page.path}#L${i + 1}`,
        approx_tokens: approxTokens(definition),
      });
      existingConceptNames.add(name);
    }

    // ── faq: ### Q: <question> headings ──────────────────────────────────────
    for (let i = 0; i < lines.length; i++) {
      const qm = lines[i].match(/^###\s+Q:\s+(.+)/);
      if (!qm) continue;
      const question = qm[1].trim();
      if (existingFaqQs.has(question)) continue;
      // Answer is the next non-empty line.
      let answer = '';
      for (let j = i + 1; j < lines.length; j++) {
        const ans = lines[j].trim();
        // Stop at the next heading.
        if (/^#{1,6}\s/.test(lines[j])) break;
        if (ans) { answer = ans; break; }
      }
      manifest.faq.push({ question, answer });
      existingFaqQs.add(question);
    }
  }
}

/**
 * Write llms-full.txt to outputPath. Idempotent: skips write if byte-equal.
 *
 * @param {{
 *   pages: Array<object>,
 *   outputPath: string,
 * }} opts
 * @returns {Promise<{ written: boolean, path: string }>}
 */
export async function writeLlmsFull({ pages, outputPath }) {
  const content = generateLlmsFull({ pages });

  fs.mkdirSync(path.dirname(outputPath), { recursive: true });

  let existing = null;
  try { existing = fs.readFileSync(outputPath, 'utf8'); } catch {}

  const written = existing !== content;
  if (written) fs.writeFileSync(outputPath, content, 'utf8');

  return { written, path: outputPath };
}

// ── CLI ───────────────────────────────────────────────────────────────────────

function realResolve(p) {
  try { return fs.realpathSync(path.resolve(p)); } catch { return path.resolve(p); }
}
const isMain = process.argv[1] &&
  realResolve(process.argv[1]) === realResolve(fileURLToPath(import.meta.url));

if (isMain) {
  const args = process.argv.slice(2);
  let repoRoot  = process.cwd();
  let outputPath = null;
  let manifestPath = null;
  for (let i = 0; i < args.length; i++) {
    if (args[i] === '--repo-root'     && args[i + 1]) repoRoot     = args[++i];
    if (args[i] === '--output'        && args[i + 1]) outputPath   = args[++i];
    if (args[i] === '--manifest-path' && args[i + 1]) manifestPath = args[++i];
  }

  outputPath   = outputPath   || path.join(repoRoot, 'llms-full.txt');
  manifestPath = manifestPath || path.join(repoRoot, '.llm-manifest.json');

  // Discover generated pages from docs/<audience>/ trees.
  // Pages are read from the four default audience paths; non-existent dirs skip silently.
  const audiencePaths = ['docs/user', 'docs/developer', 'docs/admin', 'docs/general'];
  const pages = [];

  for (const audPath of audiencePaths) {
    const dir = path.join(repoRoot, audPath);
    if (!fs.existsSync(dir)) continue;
    const audienceName = path.basename(audPath);
    const walk = (d) => {
      let entries;
      try { entries = fs.readdirSync(d, { withFileTypes: true }); } catch { return; }
      for (const ent of entries) {
        const full = path.join(d, ent.name);
        if (ent.isDirectory()) { walk(full); continue; }
        if (!ent.name.endsWith('.md')) continue;
        const relPath = path.relative(repoRoot, full).replace(/\\/g, '/');
        const content = fs.readFileSync(full, 'utf8');
        // Derive feature from path segment (tutorials/<f>.md or features/<f>.md)
        const featureMatch = relPath.match(/\/(?:tutorials|features)\/([^/]+)\.md$/);
        const feature = featureMatch ? featureMatch[1] : null;
        pages.push({ audience: audienceName, feature, path: relPath, content });
      }
    };
    walk(dir);
  }

  const result = await writeLlmsFull({ pages, outputPath });
  process.stderr.write(`[gen-llms-full] ${result.written ? 'written' : 'unchanged'}: ${result.path} (${pages.length} pages)\n`);

  // Fill manifest if it exists.
  if (fs.existsSync(manifestPath)) {
    let manifest;
    try { manifest = JSON.parse(fs.readFileSync(manifestPath, 'utf8')); } catch { manifest = null; }
    if (manifest && typeof manifest === 'object') {
      if (!Array.isArray(manifest.modules))       manifest.modules = [];
      if (!Array.isArray(manifest.concepts))      manifest.concepts = [];
      if (!Array.isArray(manifest.faq))           manifest.faq = [];
      if (!Array.isArray(manifest.cli_entry_points)) manifest.cli_entry_points = [];
      if (!Array.isArray(manifest.http_endpoints))   manifest.http_endpoints = [];
      fillManifest(manifest, pages);
      const newContent = JSON.stringify(manifest, null, 2) + '\n';
      const oldContent = fs.readFileSync(manifestPath, 'utf8');
      if (oldContent !== newContent) {
        fs.writeFileSync(manifestPath, newContent, 'utf8');
        process.stderr.write(`[gen-llms-full] manifest updated: ${manifestPath}\n`);
      } else {
        process.stderr.write(`[gen-llms-full] manifest unchanged: ${manifestPath}\n`);
      }
    }
  }
}
