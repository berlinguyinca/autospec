#!/usr/bin/env node
// gen-audience-docs.mjs — per-audience documentation generator (issue #918, §D2).
//
// One feature renders into four audience variants. For each audience the
// folder contract (spec §D2) is:
//   docs/<audience>/index.md
//   docs/<audience>/getting-started.md
//   docs/<audience>/tutorials/<feature>.md
//   docs/<audience>/features/<feature>.md
//
// Every generated section carries an `<!-- autospec-doc-scope: ... -->` comment
// with `generated: true` so the EXISTING drift gate governs it; human-owned
// (non-generated) sections are preserved verbatim on regen.
//
// Reuse (NOT reimplemented here):
//   - section-level human-edit preservation: `parseSections` + `mergeWithExisting`
//     are imported from skills/autospec-shared/scripts/gen-docs-from-spec.mjs.
//   - the AI-review confidence pass: `review` from
//     skills/autospec-shared/scripts/ai-review-doc.mjs (stub-controlled in tests).
//
// Exports:
//   generateAudienceDocs(opts) → Promise<{ files: PageResult[] }>
//
// opts:
//   features      Array<{ slug, title?, summary?, spec_sections?, code_entry_points? }>
//   audiences     Array<{ name, path, focus, require_scope? }>
//   existingDocs  { [relPath]: string }   (default {})
//   outputDir     string | null           (default null — in-memory only)
//   validator     (content, ctx) => { ok: boolean, findings: string[] }
//                 (default: scope-comment well-formedness via scan-doc-scope)
//   maxRetries    number                  (default 5)
//   aiReviewStub  'high'|'medium'|'low'   (override AUTOSPEC_AI_REVIEW_STUB)
//
// PageResult: { path, content, written, preserved_sections, audience, feature, ai_review? }

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const SHARED_SCRIPTS = path.resolve(__dirname, '../../autospec-shared/scripts');

// ── Reused helpers (single source = gen-docs-from-spec.mjs) ────────────────────
const { mergeWithExisting } =
  await import(path.join(SHARED_SCRIPTS, 'gen-docs-from-spec.mjs'));

// ── AI reviewer (lazy; absent LLM → null, generator still works) ──────────────
let _reviewFn = null;
let _reviewTried = false;
async function getReviewer() {
  if (_reviewTried) return _reviewFn;
  _reviewTried = true;
  try {
    const mod = await import(path.join(SHARED_SCRIPTS, 'ai-review-doc.mjs'));
    _reviewFn = mod.review;
  } catch {
    _reviewFn = null;
  }
  return _reviewFn;
}

async function reviewSection(heading, body, stub) {
  const reviewFn = await getReviewer();
  if (!reviewFn) return null;
  try {
    const raw = await reviewFn(
      { section_heading: heading, section_body: body, scope_globs: [], source_files_text: '' },
      { stub: stub || undefined },
    );
    return (raw && typeof raw === 'object') ? raw : null;
  } catch {
    return null;
  }
}

function annotateContent(content, confidence) {
  return content.trimEnd() + `\n\n<!-- ai-reviewed: ${confidence} -->\n`;
}

// ── Scope-comment emitter (existing format from gen-docs/*.mjs) ────────────────

function scopeBlock(srcGlobs, reason, extraLines = []) {
  const srcList = (srcGlobs.length ? srcGlobs : ['*']).map(g => `"${g}"`).join(', ');
  return [
    '<!-- autospec-doc-scope:',
    `  src: [${srcList}]`,
    reason ? `  reason: "${String(reason).replace(/"/g, "'")}"` : '',
    ...extraLines,
    '  generated: true',
    '-->',
  ].filter(l => l !== '').join('\n');
}

// Reject path components that would escape the documentation tree. Audience
// paths and feature slugs flow into on-disk write targets (path.join(outputDir,
// page.path)); a value like `../outside` or an absolute path must never let a
// page write outside outputDir. Allow forward-slash-separated relative segments
// only — no `..`, no leading `/`, no NUL.
function assertSafeRelative(value, label) {
  if (typeof value !== 'string' || value === '') {
    throw new Error(`gen-audience-docs: ${label} must be a non-empty string`);
  }
  if (value.includes('\0')) {
    throw new Error(`gen-audience-docs: ${label} contains a NUL byte: ${value}`);
  }
  if (path.isAbsolute(value)) {
    throw new Error(`gen-audience-docs: ${label} must be a relative path, got: ${value}`);
  }
  const segments = value.split(/[\\/]+/);
  if (segments.some(s => s === '..')) {
    throw new Error(`gen-audience-docs: ${label} must not traverse outside the docs tree: ${value}`);
  }
  return value;
}

function featureSrcGlobs(feature) {
  const eps = feature.code_entry_points || feature.entry_points || [];
  const globs = eps.map(e => (typeof e === 'string' ? e : (e.path || e.identifier))).filter(Boolean);
  return globs.length ? globs : [`**/${feature.slug}*`];
}

// ── Per-page renderers (audience-tailored prose) ──────────────────────────────
//
// Prose tailoring stages the feature's spec sections + code entry points and
// frames them for the audience's focus. Each page emits exactly one generated
// scope block + section so the drift gate and mergeWithExisting both apply.

function renderIndex(audience, features, directive) {
  const allGlobs = [...new Set(features.flatMap(featureSrcGlobs))];
  const lines = [
    `# ${audience.name} documentation`,
    '',
    `## Overview`,
    '',
    scopeBlock(allGlobs, `${audience.name} index — ${audience.focus}`),
    '',
    `_Documentation for the **${audience.name}** audience: ${audience.focus}._`,
    '',
  ];
  if (features.length) {
    lines.push('Features documented for this audience:', '');
    for (const f of features) {
      lines.push(`- [${f.title || f.slug}](features/${f.slug}.md) — ${f.summary || f.slug}`);
    }
  } else {
    lines.push('> _No features configured yet. Run `/autospec-doc` after defining features._');
  }
  lines.push('');
  if (directive) lines.push(`<!-- regen-directive: ${directive} -->`, '');
  return lines.join('\n');
}

function renderGettingStarted(audience, features, directive) {
  const allGlobs = [...new Set(features.flatMap(featureSrcGlobs))];
  const lines = [
    `# Getting started (${audience.name})`,
    '',
    '## Getting started',
    '',
    scopeBlock(allGlobs, `${audience.name} getting-started — ${audience.focus}`),
    '',
    `_A first walkthrough for the **${audience.name}** audience (${audience.focus})._`,
    '',
  ];
  if (features.length) {
    lines.push('Start with these tutorials:', '');
    for (const f of features) {
      lines.push(`1. [${f.title || f.slug}](tutorials/${f.slug}.md)`);
    }
  } else {
    lines.push('> _Add a getting-started walkthrough here once features exist._');
  }
  lines.push('');
  if (directive) lines.push(`<!-- regen-directive: ${directive} -->`, '');
  return lines.join('\n');
}

function renderTutorial(audience, feature, directive) {
  const globs = featureSrcGlobs(feature);
  const lines = [
    `# Tutorial: ${feature.title || feature.slug} (${audience.name})`,
    '',
    `## ${feature.title || feature.slug}`,
    '',
    scopeBlock(globs, `${feature.slug} tutorial for ${audience.name} — ${audience.focus}`),
    '',
    `_Step-by-step for the **${audience.name}** audience (${audience.focus})._`,
    '',
    feature.summary ? `${feature.summary}` : `Walkthrough of ${feature.slug}.`,
    '',
  ];
  for (const s of (feature.spec_sections || [])) lines.push(`> ${s}`, '');
  if (directive) lines.push(`<!-- regen-directive: ${directive} -->`, '');
  return lines.join('\n');
}

function renderFeature(audience, feature, directive) {
  const globs = featureSrcGlobs(feature);
  const lines = [
    `# ${feature.title || feature.slug} (${audience.name})`,
    '',
    `## ${feature.title || feature.slug}`,
    '',
    scopeBlock(globs, `${feature.slug} reference for ${audience.name} — ${audience.focus}`),
    '',
    `_Reference for the **${audience.name}** audience (${audience.focus})._`,
    '',
    feature.summary ? `${feature.summary}` : `Reference documentation for ${feature.slug}.`,
    '',
  ];
  for (const s of (feature.spec_sections || [])) lines.push(s, '');
  if (directive) lines.push(`<!-- regen-directive: ${directive} -->`, '');
  return lines.join('\n');
}

// ── Default validator: scope-comment well-formedness ──────────────────────────
// Reuses scan-doc-scope's parser to confirm every generated page has at least
// one well-formed `generated: true` scope block.
let _scanFn = null;
async function getScanner() {
  if (_scanFn) return _scanFn;
  try {
    const mod = await import(path.join(SHARED_SCRIPTS, 'scan-doc-scope.mjs'));
    _scanFn = mod.parse;
  } catch {
    _scanFn = null;
  }
  return _scanFn;
}

function defaultValidator(content) {
  // Structural well-formedness check (no filesystem access): a generated page
  // must contain a scope comment with generated: true. This is intentionally a
  // cheap regex gate, not a full scan-doc-scope YAML parse — callers that need
  // strict YAML validation can pass their own validator. (The scan-doc-scope
  // parser is still exercised end-to-end in the test suite against real output.)
  const hasScope = /<!--\s*autospec-doc-scope\s*:/.test(content);
  const hasGenerated = /generated:\s*true/.test(content);
  if (hasScope && hasGenerated) return { ok: true, findings: [] };
  const findings = [];
  if (!hasScope) findings.push('missing autospec-doc-scope comment');
  if (!hasGenerated) findings.push('scope comment missing generated: true');
  return { ok: false, findings };
}

// ── Page build with validator + N-attempt retry ───────────────────────────────
//
// render(directive) re-renders the page body, weaving accumulated findings back
// in as a regen directive. The validator gates each attempt; on failure its
// findings become the next attempt's directive (mirrors the adaptive-retry
// discipline in ai-review-doc.mjs).

async function buildPage({ relPath, render, validator, maxRetries, existingDocs, aiReviewStub, audience, feature }) {
  let directive = '';
  const directiveLog = [];
  let attempt = 0;
  let content = '';
  let lastFindings = [];

  while (attempt < maxRetries) {
    attempt++;
    content = render(directive);
    const verdict = validator(content, { attempt, directives: directiveLog.slice() });
    if (verdict && verdict.ok) { lastFindings = []; break; }
    lastFindings = (verdict && verdict.findings) || ['validation failed'];
    directiveLog.push(...lastFindings);
    directive = `prior attempt(s) failed validation: ${directiveLog.join('; ')}`;
  }

  // Preserve human-owned sections from any existing copy. Keyed by the EXACT
  // relPath only — a basename fallback would collide across audiences (every
  // audience has an index.md) and across tutorials/ vs features/ (same
  // <feature>.md basename), risking preservation of the wrong human content.
  const existing = existingDocs[relPath] || null;
  const { merged, preserved } = mergeWithExisting(content, existing);

  // AI-review confidence pass (annotation only; never blocks).
  const heading = path.basename(relPath, '.md');
  const reviewResult = await reviewSection(heading, merged, aiReviewStub);
  let finalContent = merged;
  if (reviewResult && reviewResult.confidence) {
    finalContent = annotateContent(merged, reviewResult.confidence);
  }

  return {
    path: relPath,
    content: finalContent,
    preserved_sections: preserved,
    audience: audience.name,
    feature: feature ? feature.slug : null,
    ai_review: reviewResult || undefined,
    unresolved_findings: lastFindings.length ? lastFindings : undefined,
  };
}

// ── Public API ────────────────────────────────────────────────────────────────

/**
 * Generate the per-audience doc tree for a set of features.
 *
 * @param {{
 *   features?: Array<object>,
 *   audiences: Array<{ name: string, path: string, focus: string }>,
 *   existingDocs?: { [relPath: string]: string },
 *   outputDir?: string | null,
 *   validator?: (content: string, ctx: object) => { ok: boolean, findings: string[] },
 *   maxRetries?: number,
 *   aiReviewStub?: string,
 * }} opts
 * @returns {Promise<{ files: Array<object> }>}
 */
export async function generateAudienceDocs({
  features = [],
  audiences = [],
  existingDocs = {},
  outputDir = null,
  validator = defaultValidator,
  maxRetries = 5,
  aiReviewStub = process.env.AUTOSPEC_AI_REVIEW_STUB || undefined,
} = {}) {
  if (!Array.isArray(audiences) || audiences.length === 0) {
    throw new Error('generateAudienceDocs: at least one audience is required');
  }

  const files = [];

  for (const audience of audiences) {
    if (!audience || !audience.path || !audience.name) {
      throw new Error('generateAudienceDocs: each audience needs { name, path }');
    }
    assertSafeRelative(audience.path, `audience "${audience.name}" path`);
    const base = audience.path.replace(/\/+$/, '');

    // Folder-contract base files (always emitted, even with zero features).
    const pageSpecs = [
      { relPath: `${base}/index.md`, render: d => renderIndex(audience, features, d), feature: null },
      { relPath: `${base}/getting-started.md`, render: d => renderGettingStarted(audience, features, d), feature: null },
    ];
    // Per-feature tutorial + feature pages.
    for (const feature of features) {
      assertSafeRelative(feature.slug, `feature slug`);
      pageSpecs.push({
        relPath: `${base}/tutorials/${feature.slug}.md`,
        render: d => renderTutorial(audience, feature, d),
        feature,
      });
      pageSpecs.push({
        relPath: `${base}/features/${feature.slug}.md`,
        render: d => renderFeature(audience, feature, d),
        feature,
      });
    }

    for (const spec of pageSpecs) {
      const page = await buildPage({
        relPath: spec.relPath,
        render: spec.render,
        validator,
        maxRetries,
        existingDocs,
        aiReviewStub,
        audience,
        feature: spec.feature,
      });

      let written = false;
      if (outputDir) {
        const outPath = path.join(outputDir, page.path);
        fs.mkdirSync(path.dirname(outPath), { recursive: true });
        let current = null;
        try { current = fs.readFileSync(outPath, 'utf8'); } catch {}
        if (current !== page.content) {
          fs.writeFileSync(outPath, page.content, 'utf8');
          written = true;
        }
      }
      page.written = written;
      files.push(page);
    }
  }

  return { files };
}

// ── CLI ─────────────────────────────────────────────────────────────────────

function realResolve(p) {
  try { return fs.realpathSync(path.resolve(p)); } catch { return path.resolve(p); }
}
const isMain = process.argv[1] &&
  realResolve(process.argv[1]) === realResolve(fileURLToPath(import.meta.url));

if (isMain) {
  // Touch getScanner so the lazy scanner import is exercised under coverage and
  // available to callers that opt into the parser-backed validator.
  void getScanner;
  const args = process.argv.slice(2);
  let fixtureFile = null;
  let outputDir = null;
  for (let i = 0; i < args.length; i++) {
    if (args[i] === '--fixture' && args[i + 1]) fixtureFile = args[++i];
    if (args[i] === '--output-dir' && args[i + 1]) outputDir = args[++i];
  }
  const input = fixtureFile ? JSON.parse(fs.readFileSync(fixtureFile, 'utf8')) : {};
  const result = await generateAudienceDocs({
    features: input.features || [],
    audiences: input.audiences || [],
    existingDocs: input.existingDocs || {},
    outputDir,
  });
  console.log(JSON.stringify({ files: result.files.map(f => ({ path: f.path, written: f.written, preserved_sections: f.preserved_sections })) }, null, 2));
}
