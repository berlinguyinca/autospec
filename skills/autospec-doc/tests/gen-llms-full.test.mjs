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

import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const SCRIPTS_DIR = path.resolve(__dirname, '../scripts');

const { generateLlmsFull, writeLlmsFull, fillManifest } =
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
