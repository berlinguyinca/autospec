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

// ── Public API ────────────────────────────────────────────────────────────────

/**
 * Generate a deterministic, byte-stable llms-full.txt string from page objects.
 *
 * @param {{
 *   pages: Array<{ audience: string, feature: string|null, path: string, content: string }>
 * }} opts
 * @returns {string}
 */
export function generateLlmsFull({ pages = [] } = {}) {
  if (!pages || pages.length === 0) return '';

  // Sort deterministically by path so output is stable regardless of input order.
  const sorted = [...pages].sort((a, b) => a.path < b.path ? -1 : a.path > b.path ? 1 : 0);

  const parts = [];
  let accChars = 0;
  let chunkNum = 1;

  for (const page of sorted) {
    const open  = openDelimiter(page.audience, page.feature);
    const close = closeDelimiter(page.audience, page.feature);
    const body  = (page.content || '').trimEnd();

    // Emit chunk marker BEFORE this page if we would cross the budget boundary.
    // We check accChars > 0 to avoid a spurious marker at the very start.
    if (accChars > 0 && (accChars + open.length + body.length) > TOKEN_BUDGET_CHARS * chunkNum) {
      const marker = chunkMarker(chunkNum, accChars);
      parts.push(marker);
      accChars += marker.length + 1;
      chunkNum++;
    }

    const block = [open, body, close, ''].join('\n');
    parts.push(block);
    accChars += block.length;

    // Also emit a marker AFTER this page if the page itself was large enough to
    // cross the next budget boundary (handles single-page inputs > TOKEN_BUDGET_CHARS).
    while (accChars > TOKEN_BUDGET_CHARS * chunkNum) {
      const marker = chunkMarker(chunkNum, accChars);
      parts.push(marker);
      accChars += marker.length + 1;
      chunkNum++;
    }
  }

  return parts.join('\n');
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
 *   modules          — one entry per page; name = H1 text; summary = first
 *                      non-heading paragraph line; public_api = backtick
 *                      command/function tokens on lines inside ## CLI / ## API
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
export function fillManifest(manifest, pages = []) {
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

      // summary = first non-empty, non-heading line after the H1
      let summary = name;
      let pastH1 = !h1Line; // if no H1, start scanning from line 0
      for (const ln of lines) {
        if (!pastH1) { if (/^#\s+/.test(ln)) { pastH1 = true; } continue; }
        const t = ln.trim();
        if (t && !/^#+\s/.test(t) && !t.startsWith('<!--')) {
          summary = t;
          break;
        }
      }

      // public_api = backtick tokens from ## CLI / ## API sub-sections
      const public_api = [];
      let inApiSection = false;
      for (const ln of lines) {
        if (/^##\s+(CLI|API|Public API|Commands)/i.test(ln)) {
          inApiSection = true;
          continue;
        }
        if (/^##\s+/.test(ln)) { inApiSection = false; continue; }
        if (!inApiSection) continue;
        // extract all `token` spans
        const ticks = [...ln.matchAll(/`([^`]+)`/g)].map(m2 => m2[1]);
        for (const tok of ticks) {
          if (!public_api.includes(tok)) public_api.push(tok);
        }
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
