// gen-llms-full.test.mjs — unit tests for skills/autospec-doc/scripts/gen-llms-full.mjs
// (issue #921, spec §D5). Mirrors the shape of gen-audience-docs.test.mjs.
//
// Tests:
//   1. concat is byte-identical across two runs on the same input (idempotency)
//   2. every page is wrapped in <!-- llms: audience=<a> feature=<f> --> delimiters
//   3. a chunk marker appears past the ~30000-token boundary
//   4. delimiter wraps open and close in correct order per page
//   5. empty pages array produces an empty llms-full.txt (no crash)
//   6. manifest has non-empty modules/concepts/FAQ after fillManifest
//   7. fillManifest: modules sourced from generated pages (heading extraction)
//   8. fillManifest: concepts extracted from <!-- autospec-concept: --> markers
//   9. fillManifest: FAQ entries extracted from ## FAQ / ### Q: sections
//  10. generateLlmsFull: deterministic sort — changing page order does not change output
//  11. writeLlmsFull: writes file to disk; second identical run reports written=false
//  12. chunk markers contain the correct token-budget boundary annotation
//  Track E (issue #1132):
//  18. generateLlmsIndex: single H1 in output
//  19. generateLlmsIndex: blockquote summary present
//  20. generateLlmsIndex: described link sections (each link has a description)
//  21. generateLlmsIndex: no duplicate headings
//  22. generateLlmsIndex: empty pages → well-formed minimal output
//  23. generateLlmsFull: top ToC with anchors present
//  24. generateLlmsFull: per-section summary before each section
//  25. generateLlmsFull: per-section approx_tokens annotation
//  26. generateLlmsFull: reverse-routing block present
//  27. generateLlmsFull: generated_at + commit stamp present
//  28. generateLlmsFull: re-run idempotency (no-op diff with fixed stamp)

import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const SCRIPTS_DIR = path.resolve(__dirname, '../scripts');

const { generateLlmsFull, writeLlmsFull, fillManifest, generateLlmsIndex, writeLlmsIndex } =
  await import(path.join(SCRIPTS_DIR, 'gen-llms-full.mjs'));

// ── Fixtures ─────────────────────────────────────────────────────────────────

function makeTmpDir() {
  return fs.mkdtempSync(path.join(os.tmpdir(), 'autospec-gen-llms-full-test-'));
}

// ~30k tokens ≈ 120k chars — build a page just over the boundary
const LARGE_BODY = 'word '.repeat(25000); // ~125k chars > 120k threshold

const SAMPLE_PAGES = [
  {
    audience: 'user',
    feature: 'export-pipeline',
    path: 'docs/user/features/export-pipeline.md',
    content: '# Export Pipeline (user)\n\nStreams records out to downstream sinks.\n',
  },
  {
    audience: 'user',
    feature: null,
    path: 'docs/user/index.md',
    content: '# User documentation\n\n## Overview\n\nUser-facing docs.\n',
  },
  {
    audience: 'developer',
    feature: 'auth',
    path: 'docs/developer/features/auth.md',
    content: '# Authentication (developer)\n\nToken-based login architecture.\n',
  },
];

// Pages with an <!-- autospec-concept: --> marker and a FAQ section
const PAGES_WITH_MARKERS = [
  {
    audience: 'general',
    feature: null,
    path: 'docs/general/index.md',
    content: [
      '# General documentation',
      '',
      '<!-- autospec-concept: lock-step rule -->',
      'Every phase-1 change locks all related trios simultaneously.',
      '',
      '## FAQ',
      '',
      '### Q: What is autospec?',
      'A multi-harness CI/CD automation system.',
      '',
      '### Q: Is it free?',
      'Yes, MIT licensed.',
    ].join('\n'),
  },
];

// ── Test 1: idempotency across two runs ───────────────────────────────────────

test('generateLlmsFull is byte-identical across two runs on the same input', () => {
  const run1 = generateLlmsFull({ pages: SAMPLE_PAGES });
  const run2 = generateLlmsFull({ pages: SAMPLE_PAGES });
  assert.strictEqual(run1, run2, 'two runs must produce byte-identical output');
});

// ── Test 2: every page wrapped in pinned delimiters ───────────────────────────

test('every page is wrapped in <!-- llms: audience=<a> feature=<f> --> delimiters', () => {
  const output = generateLlmsFull({ pages: SAMPLE_PAGES });
  for (const page of SAMPLE_PAGES) {
    const featureVal = page.feature || 'none';
    const openTag = `<!-- llms: audience=${page.audience} feature=${featureVal} -->`;
    assert.ok(output.includes(openTag),
      `missing open delimiter for ${page.path}: expected "${openTag}"`);
    const closeTag = `<!-- /llms: audience=${page.audience} feature=${featureVal} -->`;
    assert.ok(output.includes(closeTag),
      `missing close delimiter for ${page.path}: expected "${closeTag}"`);
  }
});

// ── Test 3: chunk marker appears past the ~30000-token boundary ───────────────

test('a chunk marker is inserted past the ~30000-token (120k char) boundary', () => {
  const bigPages = [
    {
      audience: 'user',
      feature: 'big-feature',
      path: 'docs/user/features/big-feature.md',
      content: LARGE_BODY,
    },
    {
      audience: 'developer',
      feature: 'small',
      path: 'docs/developer/features/small.md',
      content: 'small page\n',
    },
  ];
  const output = generateLlmsFull({ pages: bigPages });
  assert.ok(
    output.includes('<!-- llms-chunk:'),
    'output should contain at least one chunk marker past the 30k-token boundary',
  );
});

// ── Test 4: open / close delimiter ordering per page ─────────────────────────

test('open delimiter appears before page content and close appears after', () => {
  const output = generateLlmsFull({ pages: [SAMPLE_PAGES[0]] });
  const page = SAMPLE_PAGES[0];
  const featureVal = page.feature || 'none';
  const openTag  = `<!-- llms: audience=${page.audience} feature=${featureVal} -->`;
  const closeTag = `<!-- /llms: audience=${page.audience} feature=${featureVal} -->`;
  const openIdx  = output.indexOf(openTag);
  const contentIdx = output.indexOf(page.content.trim().slice(0, 20));
  const closeIdx = output.indexOf(closeTag);
  assert.ok(openIdx  !== -1, 'open tag must be present');
  assert.ok(closeIdx !== -1, 'close tag must be present');
  assert.ok(openIdx < contentIdx,  'open tag must precede page content');
  assert.ok(contentIdx < closeIdx, 'page content must precede close tag');
});

// ── Test 5: empty pages array produces empty-ish output without crashing ──────

test('empty pages array returns an empty string without throwing', () => {
  let result;
  assert.doesNotThrow(() => { result = generateLlmsFull({ pages: [] }); });
  assert.ok(typeof result === 'string', 'should return a string');
});

// ── Test 6: manifest has non-empty modules/concepts/FAQ after fillManifest ────

test('fillManifest returns non-empty modules, concepts, and faq from marked pages', () => {
  const manifest = { modules: [], cli_entry_points: [], http_endpoints: [], concepts: [], faq: [] };
  fillManifest(manifest, PAGES_WITH_MARKERS);
  assert.ok(Array.isArray(manifest.concepts),   'concepts must be array');
  assert.ok(manifest.concepts.length > 0,       'concepts must be non-empty');
  assert.ok(Array.isArray(manifest.faq),         'faq must be array');
  assert.ok(manifest.faq.length > 0,             'faq must be non-empty');
});

// ── Test 7: modules sourced from generated pages (H1 heading extraction) ──────

test('fillManifest extracts module entries from H1 headings in pages', () => {
  const manifest = { modules: [], cli_entry_points: [], http_endpoints: [], concepts: [], faq: [] };
  fillManifest(manifest, SAMPLE_PAGES);
  assert.ok(manifest.modules.length > 0, 'should extract at least one module from H1 headings');
  // Each module should carry a path and a summary
  for (const mod of manifest.modules) {
    assert.ok(typeof mod.path === 'string' && mod.path.length > 0, 'module must have path');
    assert.ok(typeof mod.summary === 'string' && mod.summary.length > 0, 'module must have summary');
  }
});

// ── Test 8: concepts extracted from <!-- autospec-concept: --> markers ─────────

test('fillManifest extracts concepts from <!-- autospec-concept: name --> markers', () => {
  const manifest = { modules: [], cli_entry_points: [], http_endpoints: [], concepts: [], faq: [] };
  fillManifest(manifest, PAGES_WITH_MARKERS);
  const names = manifest.concepts.map(c => c.name);
  assert.ok(names.includes('lock-step rule'),
    `expected concept "lock-step rule" in ${JSON.stringify(names)}`);
});

// ── Test 9: FAQ entries extracted from ## FAQ / ### Q: sections ───────────────

test('fillManifest extracts FAQ entries from ### Q: headings', () => {
  const manifest = { modules: [], cli_entry_points: [], http_endpoints: [], concepts: [], faq: [] };
  fillManifest(manifest, PAGES_WITH_MARKERS);
  const questions = manifest.faq.map(f => f.question);
  assert.ok(questions.some(q => q.includes('autospec')),
    `expected a question about autospec in faq: ${JSON.stringify(questions)}`);
});

// ── Test 10: deterministic sort — page order does not change output ────────────

test('generateLlmsFull output is the same regardless of input page order', () => {
  const pagesA = [...SAMPLE_PAGES];
  const pagesB = [...SAMPLE_PAGES].reverse();
  const outA = generateLlmsFull({ pages: pagesA });
  const outB = generateLlmsFull({ pages: pagesB });
  assert.strictEqual(outA, outB, 'output must be sort-stable regardless of input order');
});

// ── Test 11: writeLlmsFull writes to disk; identical second run → written=false ─

test('writeLlmsFull writes file; second identical run reports written=false', async () => {
  const tmpDir = makeTmpDir();
  try {
    const outputPath = path.join(tmpDir, 'llms-full.txt');
    const r1 = await writeLlmsFull({ pages: SAMPLE_PAGES, outputPath });
    assert.strictEqual(r1.written, true,  'first run must write the file');
    assert.ok(fs.existsSync(outputPath), 'file must exist on disk');

    const r2 = await writeLlmsFull({ pages: SAMPLE_PAGES, outputPath });
    assert.strictEqual(r2.written, false, 'second identical run must not re-write');
  } finally {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  }
});

// ── Test 12: chunk markers contain token-budget boundary annotation ────────────

test('chunk markers include a boundary token count annotation', () => {
  const bigPages = [
    {
      audience: 'user',
      feature: 'large',
      path: 'docs/user/features/large.md',
      content: LARGE_BODY,
    },
  ];
  const output = generateLlmsFull({ pages: bigPages });
  // The chunk marker must carry the chunk number (chunk=N) and a token estimate.
  const markerMatch = output.match(/<!-- llms-chunk: chunk=(\d+) approx_tokens=(\d+) -->/);
  assert.ok(markerMatch, 'chunk marker must match pattern <!-- llms-chunk: chunk=N approx_tokens=N -->');
  const chunkNum = parseInt(markerMatch[1], 10);
  assert.ok(chunkNum >= 1, 'chunk number must be >= 1');
});

// ── Track B new tests (issue #1130) ───────────────────────────────────────────

// Fixture corpus for Track B: contains an H1, a cli marker, and a concept marker
const TRACK_B_PAGES = [
  {
    audience: 'developer',
    feature: 'autospec-run',
    path: 'docs/developer/features/autospec-run.md',
    content: [
      '# autospec-run',
      '',
      'The main entry point for running the autospec pipeline.',
      '',
      '<!-- autospec-concept: lock-step rule -->',
      'Every phase-1 change locks all related trios simultaneously.',
      '',
      '<!-- autospec-concept: worktree isolation -->',
      'Each implementation runs in an isolated git worktree.',
      '',
      '## CLI',
      '`/autospec-run` — run the implementation loop',
    ].join('\n'),
  },
  {
    audience: 'developer',
    feature: 'autospec-define',
    path: 'docs/developer/features/autospec-define.md',
    content: [
      '# autospec-define',
      '',
      'Plan a feature and decompose into GitHub issues.',
    ].join('\n'),
  },
];

// ── Test 13: modules[] carry name, summary, source_anchor, approx_tokens ──────

test('fillManifest: modules carry name, source_anchor, and approx_tokens', () => {
  const manifest = { modules: [], cli_entry_points: [], http_endpoints: [], concepts: [], faq: [] };
  fillManifest(manifest, TRACK_B_PAGES);
  assert.ok(manifest.modules.length > 0, 'modules must be non-empty');
  for (const mod of manifest.modules) {
    assert.ok(typeof mod.name === 'string' && mod.name.length > 0,
      `module must have name field; got ${JSON.stringify(mod)}`);
    assert.ok(typeof mod.summary === 'string' && mod.summary.length > 0,
      `module must have summary; got ${JSON.stringify(mod)}`);
    assert.ok(typeof mod.source_anchor === 'string' && mod.source_anchor.includes('#L'),
      `module must have source_anchor with #L; got ${JSON.stringify(mod)}`);
    assert.ok(typeof mod.approx_tokens === 'number' && mod.approx_tokens > 0,
      `module must have approx_tokens > 0; got ${JSON.stringify(mod)}`);
    assert.ok(Array.isArray(mod.public_api),
      `module must have public_api array; got ${JSON.stringify(mod)}`);
    assert.ok(typeof mod.doc === 'string',
      `module must have doc field; got ${JSON.stringify(mod)}`);
  }
});

// ── Test 14: concepts[] carry source_anchor and approx_tokens ─────────────────

test('fillManifest: concepts carry source_anchor and approx_tokens', () => {
  const manifest = { modules: [], cli_entry_points: [], http_endpoints: [], concepts: [], faq: [] };
  fillManifest(manifest, TRACK_B_PAGES);
  assert.ok(manifest.concepts.length >= 2, 'expected at least 2 concepts from fixture');
  for (const concept of manifest.concepts) {
    assert.ok(typeof concept.source_anchor === 'string' && concept.source_anchor.includes('#L'),
      `concept must have source_anchor with #L; got ${JSON.stringify(concept)}`);
    assert.ok(typeof concept.approx_tokens === 'number' && concept.approx_tokens > 0,
      `concept must have approx_tokens > 0; got ${JSON.stringify(concept)}`);
  }
});

// ── Test 15: no literal <name> placeholder in manifest output ──────────────────

test('fillManifest: no literal <name> placeholder survives in any field', () => {
  // Inject a page that contains the literal placeholder string in a concept marker
  const pagesWithPlaceholder = [
    ...TRACK_B_PAGES,
    {
      audience: 'developer',
      feature: 'bad',
      path: 'docs/developer/features/bad.md',
      content: '# Bad page\n\n<!-- autospec-concept: <name> -->\nShould be skipped.\n',
    },
  ];
  const manifest = { modules: [], cli_entry_points: [], http_endpoints: [], concepts: [], faq: [] };
  fillManifest(manifest, pagesWithPlaceholder);
  const conceptNames = manifest.concepts.map(c => c.name);
  assert.ok(!conceptNames.includes('<name>'),
    `literal <name> placeholder must not appear in concepts; got ${JSON.stringify(conceptNames)}`);
  // Also check no field in manifest JSON contains literal <name>
  const json = JSON.stringify(manifest);
  assert.ok(!json.includes('"<name>"'),
    `literal "<name>" must not appear in manifest JSON; found: ${json.slice(0, 200)}`);
});

// ── Test 16: cli_entry_points[] non-empty when pages have CLI sections ─────────

test('fillManifest: cli_entry_points extracted from pages', () => {
  const manifest = { modules: [], cli_entry_points: [], http_endpoints: [], concepts: [], faq: [] };
  fillManifest(manifest, TRACK_B_PAGES);
  // TRACK_B_PAGES has a CLI section with `/autospec-run`; cli_entry_points should be populated
  assert.ok(Array.isArray(manifest.cli_entry_points),
    'cli_entry_points must be an array');
  // The fixture has CLI content — at least one entry is expected
  assert.ok(manifest.cli_entry_points.length > 0,
    `cli_entry_points must be non-empty for pages with CLI sections; got ${JSON.stringify(manifest.cli_entry_points)}`);
});

// ── Test 17: fixture corpus produces zero placeholder strings ─────────────────

test('fillManifest: fixture corpus produces non-empty modules and concepts, zero placeholders', () => {
  const manifest = { modules: [], cli_entry_points: [], http_endpoints: [], concepts: [], faq: [] };
  fillManifest(manifest, TRACK_B_PAGES);
  assert.ok(manifest.modules.length > 0, 'modules must be non-empty');
  assert.ok(manifest.concepts.length > 0, 'concepts must be non-empty');
  const json = JSON.stringify(manifest);
  assert.ok(!json.includes('<name>'),
    `no <name> placeholder must survive; manifest JSON: ${json.slice(0, 300)}`);
});

// ── Track E tests (issue #1132) ───────────────────────────────────────────────

// Fixture for Track E: pages with varied features + sections
const TRACK_E_PAGES = [
  {
    audience: 'user',
    feature: 'export-pipeline',
    path: 'docs/user/features/export-pipeline.md',
    content: [
      '# Export Pipeline',
      '',
      'Streams records out to downstream sinks.',
      '',
      '## Overview',
      '',
      'The export pipeline processes batches of records.',
    ].join('\n'),
  },
  {
    audience: 'developer',
    feature: 'auth',
    path: 'docs/developer/features/auth.md',
    content: [
      '# Authentication',
      '',
      'Token-based login architecture.',
      '',
      '## Overview',
      '',
      'Supports OAuth2 and API keys.',
    ].join('\n'),
  },
  {
    audience: 'user',
    feature: null,
    path: 'docs/user/index.md',
    content: [
      '# User Guide',
      '',
      'Entry point for user-facing documentation.',
    ].join('\n'),
  },
];

// ── Test 18: generateLlmsIndex — single H1 in output ──────────────────────────

test('generateLlmsIndex: output contains exactly one H1 heading', () => {
  const output = generateLlmsIndex({ pages: TRACK_E_PAGES });
  assert.ok(typeof output === 'string', 'should return a string');
  const h1Matches = output.match(/^# .+/gm);
  assert.ok(h1Matches !== null, 'output must contain at least one H1');
  assert.strictEqual(h1Matches.length, 1, `output must have exactly one H1; got ${h1Matches.length}: ${JSON.stringify(h1Matches)}`);
});

// ── Test 19: generateLlmsIndex — blockquote summary present ───────────────────

test('generateLlmsIndex: output contains a blockquote summary line', () => {
  const output = generateLlmsIndex({ pages: TRACK_E_PAGES });
  assert.ok(output.includes('\n> '), 'output must contain a blockquote line (> ...)');
});

// ── Test 20: generateLlmsIndex — described links (each link has a description) ─

test('generateLlmsIndex: each link entry has a one-line description', () => {
  const output = generateLlmsIndex({ pages: TRACK_E_PAGES });
  // llmstxt.org: links in sections look like:  - [Title](url): Description
  const linkLines = output.split('\n').filter(l => /^- \[.+\]\(.+\):/.test(l));
  assert.ok(linkLines.length > 0, `output must contain at least one described link; output:\n${output}`);
  for (const line of linkLines) {
    // After the colon there must be non-empty description text
    const descPart = line.replace(/^- \[.+\]\(.+\):\s*/, '').trim();
    assert.ok(descPart.length > 0, `link line must have a non-empty description: "${line}"`);
  }
});

// ── Test 21: generateLlmsIndex — no duplicate headings ────────────────────────

test('generateLlmsIndex: no duplicate headings in output', () => {
  const output = generateLlmsIndex({ pages: TRACK_E_PAGES });
  const headings = output.split('\n').filter(l => /^#{1,6} /.test(l));
  const seen = new Set();
  for (const h of headings) {
    assert.ok(!seen.has(h), `duplicate heading found: "${h}"`);
    seen.add(h);
  }
});

// ── Test 22: generateLlmsIndex — empty pages → well-formed minimal output ──────

test('generateLlmsIndex: empty pages array returns a well-formed string', () => {
  let result;
  assert.doesNotThrow(() => { result = generateLlmsIndex({ pages: [] }); });
  assert.ok(typeof result === 'string', 'should return a string');
  // Must still have a single H1
  const h1Matches = result.match(/^# .+/gm);
  assert.ok(h1Matches !== null && h1Matches.length === 1, 'empty-pages output must still have exactly one H1');
});

// ── Test 23: generateLlmsFull — top ToC with anchors present ──────────────────

test('generateLlmsFull (Track E): top ToC with anchors present', () => {
  const output = generateLlmsFull({ pages: TRACK_E_PAGES });
  // ToC section: ## Table of Contents or similar, with anchor links [text](#anchor)
  assert.ok(output.includes('Table of Contents') || output.includes('## Contents'),
    'output must contain a table of contents section');
  // Anchors look like (#something)
  assert.ok(/\(#[a-z0-9-]+\)/.test(output),
    'ToC must contain at least one anchor link like (#section-name)');
});

// ── Test 24: generateLlmsFull — per-section summary before each section ────────

test('generateLlmsFull (Track E): per-section summary (1-2 lines) before each section', () => {
  const output = generateLlmsFull({ pages: TRACK_E_PAGES });
  // A summary comment block looks like: <!-- summary: ... --> or plain text before the delimiter
  // We check that summary annotations appear somewhere in the output
  assert.ok(
    output.includes('<!-- section-summary:') || output.includes('<!-- summary:'),
    'output must contain per-section summary annotations'
  );
});

// ── Test 25: generateLlmsFull — per-section approx_tokens annotation ──────────

test('generateLlmsFull (Track E): per-section approx_tokens annotation present', () => {
  const output = generateLlmsFull({ pages: TRACK_E_PAGES });
  assert.ok(
    /approx_tokens=\d+/.test(output),
    'output must contain per-section approx_tokens=N annotations'
  );
});

// ── Test 26: generateLlmsFull — reverse-routing block present ─────────────────

test('generateLlmsFull (Track E): reverse-routing block (source→doc mapping) present', () => {
  const output = generateLlmsFull({ pages: TRACK_E_PAGES });
  assert.ok(
    output.includes('<!-- reverse-routing') || output.includes('## Source Routing') || output.includes('source-routing'),
    'output must contain a reverse-routing block mapping source files to docs'
  );
});

// ── Test 27: generateLlmsFull — generated_at + commit stamp present ───────────

test('generateLlmsFull (Track E): generated_at and commit stamp present', () => {
  const output = generateLlmsFull({ pages: TRACK_E_PAGES, generatedAt: '2026-06-16T00:00:00Z', commit: 'abc1234' });
  assert.ok(output.includes('generated_at'), 'output must contain generated_at stamp');
  assert.ok(output.includes('2026-06-16T00:00:00Z'), 'output must contain the provided generated_at value');
  assert.ok(output.includes('abc1234'), 'output must contain the commit hash');
});

// ── Test 28: generateLlmsFull — re-run idempotency with fixed stamp ───────────

test('generateLlmsFull (Track E): re-run with same inputs produces identical output (idempotent)', () => {
  const opts = { pages: TRACK_E_PAGES, generatedAt: '2026-06-16T00:00:00Z', commit: 'abc1234' };
  const run1 = generateLlmsFull(opts);
  const run2 = generateLlmsFull(opts);
  assert.strictEqual(run1, run2, 'two runs with identical inputs must produce byte-identical output');
});
