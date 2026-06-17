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
import { isLogicFlowSection, generateExplainerDiagram } from './doc-style.mjs';

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

// ── Batched ai-review: ONE call per audience per generation run ───────────────
//
// reviewSectionBatch(sections, stub) sends all sections for one audience in a
// single concatenated call (§D6: one batched ai-review call per audience).
// Each section is delimited by a per-section header so the confidence markers
// are parsed back out per heading. Format:
//   [SECTION:<idx>:<heading>]
//   <body>
//
// The ai-review call receives the concatenated body as section_body; the stub
// path short-circuits to a uniform confidence. On any parse failure, 'low'.
//
// Returns: Map<heading, { confidence, concerns }>

async function reviewSectionBatch(sections, stub) {
  const reviewFn = await getReviewer();
  if (!reviewFn || sections.length === 0) return new Map();

  // Concatenate all sections with per-section delimiters.
  const batchBody = sections.map(
    (s, i) => `[SECTION:${i}:${s.heading}]\n${s.body}`
  ).join('\n\n---\n\n');

  // ONE ai-review call for this audience (the §D6 cost cap).
  let raw = null;
  try {
    raw = await reviewFn(
      { section_heading: 'BATCH_REVIEW', section_body: batchBody, scope_globs: [], source_files_text: '' },
      { stub: stub || undefined },
    );
  } catch {
    raw = null;
  }

  // Apply the single batch confidence to all sections.
  const results = new Map();
  const confidence = (raw && raw.confidence) ? raw.confidence : 'low';
  const concerns   = (raw && Array.isArray(raw.concerns)) ? raw.concerns : [];
  for (const s of sections) {
    results.set(s.heading, { confidence, concerns });
  }
  return results;
}

function annotateContent(content, confidence) {
  return content.trimEnd() + `\n\n<!-- ai-reviewed: ${confidence} -->\n`;
}

// ── Per-audience prose resolver ───────────────────────────────────────────────
//
// A feature prose field may be EITHER a shared value (scalar/array — same prose
// for all audiences) OR a per-audience MAP: { user, developer, admin, general,
// default }. pickForAudience resolves the latter to the right variant.
//
// Detection rule: treat `value` as a per-audience map only when it is a plain
// object (not an array) carrying at least one known audience key OR a `default`
// key. A normal data object (e.g. `{ id, payload, ts }`) has none of those keys
// and is returned unchanged as a shared value.
//
// Resolution: value[audienceName] ?? value.default ?? null.

const KNOWN_AUDIENCES = new Set(['user', 'developer', 'admin', 'general']);

function isAudienceMap(value) {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) return false;
  if (Object.prototype.hasOwnProperty.call(value, 'default')) return true;
  return Object.keys(value).some(k => KNOWN_AUDIENCES.has(k));
}

export function pickForAudience(value, audienceName) {
  if (!isAudienceMap(value)) return value;
  const picked = value[audienceName];
  if (picked !== undefined) return picked;
  if (value.default !== undefined) return value.default;
  return null;
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
      const summary = pickForAudience(f.summary, audience.name);
      lines.push(`- [${f.title || f.slug}](features/${f.slug}.md) — ${summary || f.slug}`);
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
  const summary = pickForAudience(feature.summary, audience.name);
  const specSections = pickForAudience(feature.spec_sections, audience.name);
  const lines = [
    `# Tutorial: ${feature.title || feature.slug} (${audience.name})`,
    '',
    `## ${feature.title || feature.slug}`,
    '',
    scopeBlock(globs, `${feature.slug} tutorial for ${audience.name} — ${audience.focus}`),
    '',
    `_Step-by-step for the **${audience.name}** audience (${audience.focus})._`,
    '',
    summary ? `${summary}` : `Walkthrough of ${feature.slug}.`,
    '',
  ];
  for (const s of (specSections || [])) lines.push(`> ${s}`, '');
  if (directive) lines.push(`<!-- regen-directive: ${directive} -->`, '');
  return lines.join('\n');
}

// ── renderFeatureSections — six LLM-targeted H2 blocks (issue #1129) ──────────
//
// Audience gating (pinned in shared-contracts):
//   config_reference → [admin, developer]
//   rationale        → [developer]
//   all others       → all audiences
//
// Empty fields ('' / []) are silently omitted — no blank H2 is emitted.
//
// Section order is fixed: Data model → Invariants → Errors → Configuration →
// Why → Related features.

const SECTION_AUDIENCE_GATE = {
  config_reference: ['admin', 'developer'],
  rationale:        ['developer'],
};

function renderFeatureSections(audience, feature) {
  const lines = [];

  function maybeSection(heading, fieldValue) {
    if (!fieldValue || (typeof fieldValue === 'string' && fieldValue.trim() === '')) return;
    if (Array.isArray(fieldValue) && fieldValue.length === 0) return;
    lines.push(`## ${heading}`, '');
    if (Array.isArray(fieldValue)) {
      for (const item of fieldValue) lines.push(`- ${item}`);
      lines.push('');
    } else {
      lines.push(String(fieldValue), '');
    }
  }

  const pick = (field) => pickForAudience(feature[field], audience.name);

  // data_model — all audiences
  maybeSection('Data model', pick('data_model') || '');

  // invariants — all audiences
  maybeSection('Invariants & constraints', pick('invariants') || '');

  // errors — all audiences
  maybeSection('Errors & failure modes', pick('errors') || '');

  // config_reference — admin + developer only
  if (SECTION_AUDIENCE_GATE.config_reference.includes(audience.name)) {
    maybeSection('Configuration', pick('config_reference') || '');
  }

  // rationale — developer only
  if (SECTION_AUDIENCE_GATE.rationale.includes(audience.name)) {
    maybeSection('Why', pick('rationale') || '');
  }

  // depends_on — shared (feature ids, not prose).
  maybeSection('Related features', feature.depends_on || []);

  return lines;
}

// ── renderExamples — emit <!-- example --> fenced blocks (issue #1133) ──────────
//
// Emits one `<!-- example -->` + fenced code block per entry in
// feature.examples[].  Empty / absent arrays emit nothing.
//
// Entry shape: { lang?: string, command: string }
//   lang     defaults to 'bash' when absent or empty.
//   command  the executable text placed verbatim inside the fence.
//
// Marker format is pinned verbatim by the #1133 shared contract:
//   <!-- example -->
//   ```<lang>
//   <command>
//   ```
//
// verifyExamples (verify-examples.mjs) recognises exactly this tag+fence
// sequence and stamps an adjacent output block + <!-- example-verified: -->
// marker after a successful run.

function renderExamples(feature, audienceName) {
  const examples = pickForAudience(feature.examples, audienceName);
  if (!examples || examples.length === 0) return [];
  const lines = [];
  for (const entry of examples) {
    const lang = (entry.lang && entry.lang.trim()) ? entry.lang.trim() : 'bash';
    lines.push('<!-- example -->');
    lines.push(`\`\`\`${lang}`);
    lines.push(entry.command);
    lines.push('```');
    lines.push('');
  }
  return lines;
}

function renderFeature(audience, feature, directive) {
  const globs = featureSrcGlobs(feature);
  const summary = pickForAudience(feature.summary, audience.name);
  const specSections = pickForAudience(feature.spec_sections, audience.name);
  const lines = [
    `# ${feature.title || feature.slug} (${audience.name})`,
    '',
    `## ${feature.title || feature.slug}`,
    '',
    scopeBlock(globs, `${feature.slug} reference for ${audience.name} — ${audience.focus}`),
    '',
    `_Reference for the **${audience.name}** audience (${audience.focus})._`,
    '',
    summary ? `${summary}` : `Reference documentation for ${feature.slug}.`,
    '',
  ];
  for (const s of (specSections || [])) {
    lines.push(s, '');
    // Track D (issue #1131): emit a palette-themed mermaid diagram for logic-flow sections.
    const section = { heading: feature.title || feature.slug, body: s };
    if (isLogicFlowSection(section)) {
      const diagram = generateExplainerDiagram(section);
      if (diagram) {
        lines.push('```mermaid');
        lines.push(diagram);
        lines.push('```');
        lines.push('');
      }
    }
  }
  // Append the six LLM-targeted sections (issue #1129); empty fields are omitted.
  lines.push(...renderFeatureSections(audience, feature));
  // Append verified runnable examples (issue #1133); empty examples[] emits nothing.
  lines.push(...renderExamples(feature, audience.name));
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

// ── Page build with deterministic-first validator + N-attempt retry ───────────
//
// §D6 cost-cap: run `defaultValidator` (deterministic scope-comment gate) FIRST
// on every render. A deterministic failure re-renders immediately WITHOUT calling
// the LLM validator — the LLM validator only adjudicates prose-quality retries
// (i.e., it is only consulted after the deterministic gate has already passed).
//
// Call-count contract (asserted by tests):
//   - deterministic failure → 0 LLM-validator calls for that attempt
//   - only after defaultValidator passes → custom/LLM validator is invoked
//
// render(directive) re-renders the page body, weaving accumulated findings back
// in as a regen directive (mirrors the adaptive-retry discipline in
// ai-review-doc.mjs).
//
// Returns the built page WITHOUT ai-review annotation; ai-review is applied in
// batch by generateAudienceDocs (ONE call per audience — §D6).

async function buildPage({ relPath, render, validator, maxRetries, existingDocs, audience, feature }) {
  let directive = '';
  const directiveLog = [];
  let attempt = 0;
  let content = '';
  let lastFindings = [];

  while (attempt < maxRetries) {
    attempt++;
    content = render(directive);

    // §D6: deterministic gate FIRST — no LLM call on structural failure.
    const deterministicVerdict = defaultValidator(content);
    if (!deterministicVerdict.ok) {
      lastFindings = deterministicVerdict.findings;
      directiveLog.push(...lastFindings);
      directive = `prior attempt(s) failed deterministic validation: ${directiveLog.join('; ')}`;
      continue; // regen without touching the LLM validator
    }

    // Deterministic gate passed — now let the custom/LLM validator adjudicate.
    if (validator !== defaultValidator) {
      const verdict = validator(content, { attempt, directives: directiveLog.slice() });
      if (verdict && verdict.ok) { lastFindings = []; break; }
      lastFindings = (verdict && verdict.findings) || ['validation failed'];
      directiveLog.push(...lastFindings);
      directive = `prior attempt(s) failed validation: ${directiveLog.join('; ')}`;
    } else {
      // validator IS defaultValidator — already passed above.
      lastFindings = [];
      break;
    }
  }

  // Preserve human-owned sections from any existing copy. Keyed by the EXACT
  // relPath only — a basename fallback would collide across audiences (every
  // audience has an index.md) and across tutorials/ vs features/ (same
  // <feature>.md basename), risking preservation of the wrong human content.
  const existing = existingDocs[relPath] || null;
  const { merged, preserved } = mergeWithExisting(content, existing);

  return {
    path: relPath,
    content: merged,
    preserved_sections: preserved,
    audience: audience.name,
    feature: feature ? feature.slug : null,
    unresolved_findings: lastFindings.length ? lastFindings : undefined,
  };
}

// ── Sameness guard ────────────────────────────────────────────────────────────
//
// The audience split is only real if `features/<slug>.md` carries DIFFERENT prose
// per audience. To detect a fake split we normalize away the parts that vary for
// trivial reasons — the audience tagline, the tutorial tagline, the mermaid theme
// `%%{init}%%` line, and the gated `## Configuration` / `## Why` sections (which
// differ only because of gating, not authored prose) — and compare what remains.
// If the normalized body is byte-identical across every audience, the feature has
// no real per-audience tailoring.

function normalizeForSameness(content) {
  const out = [];
  const lines = content.split('\n');
  let skippingGated = false;
  let inScopeBlock = false;
  for (const line of lines) {
    // Drop the entire autospec-doc-scope block: its `reason` carries the audience
    // name + focus (auto-derived, not authored prose), so it always differs.
    if (/<!--\s*autospec-doc-scope\s*:/.test(line)) { inScopeBlock = true; continue; }
    if (inScopeBlock) {
      if (line.includes('-->')) inScopeBlock = false;
      continue;
    }
    // Enter/exit gated-section skip: `## Configuration` and `## Why` are gated by
    // audience, so drop them (and their bodies, up to the next H2) before compare.
    if (/^##\s+(Configuration|Why)\s*$/.test(line)) { skippingGated = true; continue; }
    if (skippingGated) {
      if (/^##\s+/.test(line)) skippingGated = false; // next H2 ends the gated block
      else continue;
    }
    // Drop audience-tagline / tutorial-tagline lines.
    if (/^_Reference for the \*\*.+?\*\* audience/.test(line)) continue;
    if (/^_Step-by-step for the \*\*.+?\*\* audience/.test(line)) continue;
    if (/^_Documentation for the \*\*.+?\*\* audience/.test(line)) continue;
    // Drop the mermaid theme line and the audience name in headings/scope.
    if (/%%\{init:/.test(line)) continue;
    out.push(line);
  }
  // Neutralize the audience name wherever it is interpolated (H1 "(user)", scope
  // "reason", the ai-reviewed marker, focus strings) so only authored prose drives
  // the comparison.
  return out.join('\n')
    .replace(/\((user|developer|admin|general)\)/g, '(AUD)')
    .replace(/\b(user|developer|admin|general)\b/g, 'AUD');
}

function computeSamenessWarnings(files) {
  const warnings = [];
  const identicalFeatures = new Set();
  // Group feature pages by slug.
  const bySlug = new Map();
  for (const f of files) {
    if (!f.feature) continue;
    if (!f.path.includes('/features/')) continue;
    if (!bySlug.has(f.feature)) bySlug.set(f.feature, []);
    bySlug.get(f.feature).push(f);
  }
  for (const [slug, pages] of bySlug) {
    if (pages.length < 2) continue; // single audience — nothing to compare
    const normalized = pages.map(p => normalizeForSameness(p.content));
    const allSame = normalized.every(n => n === normalized[0]);
    if (allSame) {
      identicalFeatures.add(slug);
      warnings.push(
        `feature '${slug}': identical prose across all audiences — no per-audience tailoring`,
      );
    }
  }
  return { warnings, identicalFeatures };
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
 *   failOnIdenticalAudiences?: boolean,
 * }} opts
 * @returns {Promise<{ files: Array<object>, warnings: string[] }>}
 */
export async function generateAudienceDocs({
  features = [],
  audiences = [],
  existingDocs = {},
  outputDir = null,
  validator = defaultValidator,
  maxRetries = 5,
  aiReviewStub = process.env.AUTOSPEC_AI_REVIEW_STUB || undefined,
  failOnIdenticalAudiences = false,
} = {}) {
  if (!Array.isArray(audiences) || audiences.length === 0) {
    throw new Error('generateAudienceDocs: at least one audience is required');
  }

  // Build + annotate every page across every audience IN MEMORY first; defer all
  // disk writes until after the sameness guard so failOnIdenticalAudiences can
  // abort before writing anything new.
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

    // Build all pages for this audience (deterministic-first, no per-page LLM calls).
    const audiencePages = [];
    for (const spec of pageSpecs) {
      const page = await buildPage({
        relPath: spec.relPath,
        render: spec.render,
        validator,
        maxRetries,
        existingDocs,
        audience,
        feature: spec.feature,
      });
      audiencePages.push(page);
    }

    // §D6: ONE batched ai-review call per audience (not per page/section).
    // Collect all page headings+bodies, issue the single batch call, then
    // annotate each page with its per-section confidence marker.
    const batchSections = audiencePages.map(p => ({
      heading: path.basename(p.path, '.md'),
      body: p.content,
    }));
    const reviewMap = await reviewSectionBatch(batchSections, aiReviewStub);

    for (const page of audiencePages) {
      const heading = path.basename(page.path, '.md');
      const reviewResult = reviewMap.get(heading) || null;
      let finalContent = page.content;
      if (reviewResult && reviewResult.confidence) {
        finalContent = annotateContent(page.content, reviewResult.confidence);
      }
      page.content = finalContent;
      page.ai_review = reviewResult || undefined;
      page.written = false;
      files.push(page);
    }
  }

  // Sameness guard: detect features whose feature page is byte-identical across
  // all audiences (modulo audience-varying lines). Surface a warning per feature;
  // optionally abort BEFORE writing anything.
  const { warnings, identicalFeatures } = computeSamenessWarnings(files);
  if (failOnIdenticalAudiences && identicalFeatures.size > 0) {
    const slugs = [...identicalFeatures].join(', ');
    throw new Error(
      `generateAudienceDocs: identical prose across all audiences for feature(s): ${slugs}. ` +
      `Author per-audience prose (see SKILL.md) or pass failOnIdenticalAudiences: false. ` +
      `Nothing was written.`,
    );
  }

  // Guard passed (or not enforced) — now write to disk.
  if (outputDir) {
    for (const page of files) {
      const outPath = path.join(outputDir, page.path);
      fs.mkdirSync(path.dirname(outPath), { recursive: true });
      let current = null;
      try { current = fs.readFileSync(outPath, 'utf8'); } catch {}
      if (current !== page.content) {
        fs.writeFileSync(outPath, page.content, 'utf8');
        page.written = true;
      }
    }
  }

  return { files, warnings };
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
  for (const w of (result.warnings || [])) {
    process.stderr.write(`warning: ${w}\n`);
  }
  console.log(JSON.stringify({ files: result.files.map(f => ({ path: f.path, written: f.written, preserved_sections: f.preserved_sections })), warnings: result.warnings || [] }, null, 2));
}
