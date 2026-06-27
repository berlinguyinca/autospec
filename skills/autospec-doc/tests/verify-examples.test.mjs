// verify-examples.test.mjs — unit tests for skills/autospec-doc/scripts/verify-examples.mjs (issue #919, §D3)
//
// The docs-as-tests engine: every fenced `bash`/`console` block tagged
// `<!-- example -->` is executed in an isolated sandbox; real output is embedded
// in an adjacent ```output block; a `<!-- example-verified: <sha> <iso> -->`
// marker is stamped. A failing example fails generation (non-zero exit).
//
// "NO mocks — examples really execute" holds in PRODUCTION: the default executor
// shells out for real. Tests inject a fake executor (`opts.exec`) so the suite
// never spawns worktrees or stalls — the engine's contract is exercised, the
// real subprocess is not. This is the same stub-injection pattern #918 uses for
// the AI reviewer.
//
// Coverage:
//   1. parseExamples finds <!-- example --> bash/console blocks (and ignores untagged)
//   2. a passing example embeds an adjacent ```output block with captured stdout
//   3. a passing example is stamped with an <!-- example-verified: <sha> <iso> --> marker
//   4. marker SHA + ISO date match the head sha / iso passed in
//   5. a re-run replaces a previous ```output block (idempotent, no duplication)
//   6. a FAILING example fixture makes verifyExamples reject / report a failure
//   7. the per-example timeout defaults to 60 seconds and is passed to the executor
//   8. the sandbox uses a fresh worktree off origin/main (executor sees worktree opts)
//   9. tutorial step sequences are collected as examples
//  10. running the CLI on a failing-example fixture file exits non-zero

import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const MOD = path.resolve(__dirname, '../scripts/verify-examples.mjs');

const {
  parseExamples,
  verifyExamples,
  DEFAULT_TIMEOUT_MS,
  EXAMPLE_TAG,
  stampMarker,
  parseImplementationSnippets,
  verifyImplementationSnippets,
} = await import(MOD);

// ── Helpers ─────────────────────────────────────────────────────────────────

function makeTmpDir() {
  return fs.mkdtempSync(path.join(os.tmpdir(), 'verify-examples-test-'));
}

// A fake executor: records every call, returns canned results keyed by command.
// Shape mirrors the real executor contract:
//   exec({ command, lang, timeoutMs, sandbox }) -> { stdout, stderr, code }
function makeExec(responder) {
  const calls = [];
  const exec = async (ctx) => {
    calls.push(ctx);
    const r = responder ? responder(ctx) : {};
    return {
      stdout: r.stdout ?? '', stderr: r.stderr ?? '', code: r.code ?? 0,
      ...(r.sandboxSha !== undefined ? { sandboxSha: r.sandboxSha } : {}),
    };
  };
  exec.calls = calls;
  return exec;
}

function passingExec(stdout = 'hello\n') {
  return makeExec(() => ({ stdout, code: 0 }));
}

const PASS_PAGE = [
  '# Tutorial',
  '',
  'Run the greeter:',
  '',
  '<!-- example -->',
  '```bash',
  'echo hello',
  '```',
  '',
  'Done.',
  '',
].join('\n');

const UNTAGGED_PAGE = [
  '# No example here',
  '',
  '```bash',
  'echo not-an-example',
  '```',
  '',
].join('\n');

const FAILING_PAGE = [
  '# Failing tutorial',
  '',
  '<!-- example -->',
  '```bash',
  'exit 7',
  '```',
  '',
].join('\n');

// ── Tests ───────────────────────────────────────────────────────────────────

test('parseExamples finds <!-- example --> tagged bash blocks and ignores untagged', () => {
  const tagged = parseExamples(PASS_PAGE);
  assert.strictEqual(tagged.length, 1);
  assert.strictEqual(tagged[0].lang, 'bash');
  assert.strictEqual(tagged[0].command.trim(), 'echo hello');

  const none = parseExamples(UNTAGGED_PAGE);
  assert.strictEqual(none.length, 0, 'untagged fenced blocks are not examples');
});

test('parseExamples recognizes console-tagged examples too', () => {
  const page = '<!-- example -->\n```console\n$ ls\n```\n';
  const ex = parseExamples(page);
  assert.strictEqual(ex.length, 1);
  assert.strictEqual(ex[0].lang, 'console');
});

test('EXAMPLE_TAG is the pinned <!-- example --> marker', () => {
  assert.match(EXAMPLE_TAG, /<!--\s*example\s*-->/);
});

test('DEFAULT_TIMEOUT_MS is 60 seconds', () => {
  assert.strictEqual(DEFAULT_TIMEOUT_MS, 60_000);
});

test('passing example embeds an adjacent ```output block with captured stdout', async () => {
  const exec = passingExec('hello\n');
  const res = await verifyExamples({
    content: PASS_PAGE,
    headSha: 'deadbeef',
    isoDate: '2026-06-03',
    exec,
  });
  assert.strictEqual(res.failed.length, 0);
  assert.match(res.content, /```output\nhello\n```/);
});

test('passing example is stamped with example-verified marker carrying sha + iso', async () => {
  const exec = passingExec('hi\n');
  const res = await verifyExamples({
    content: PASS_PAGE,
    headSha: 'abc123',
    isoDate: '2026-06-03',
    exec,
  });
  assert.match(res.content, /<!-- example-verified: abc123 2026-06-03 -->/);
});

test('re-run replaces the prior output block and marker (idempotent, no duplication)', async () => {
  const exec1 = passingExec('first\n');
  const r1 = await verifyExamples({
    content: PASS_PAGE, headSha: 'sha1', isoDate: '2026-06-01', exec: exec1,
  });
  const exec2 = passingExec('second\n');
  const r2 = await verifyExamples({
    content: r1.content, headSha: 'sha2', isoDate: '2026-06-02', exec: exec2,
  });
  // Output is refreshed, not appended.
  const outputBlocks = (r2.content.match(/```output/g) || []).length;
  assert.strictEqual(outputBlocks, 1, 'exactly one output block after re-run');
  assert.match(r2.content, /```output\nsecond\n```/);
  assert.doesNotMatch(r2.content, /first/);
  // Marker is refreshed, not duplicated.
  const markers = (r2.content.match(/example-verified:/g) || []).length;
  assert.strictEqual(markers, 1, 'exactly one verified marker after re-run');
  assert.match(r2.content, /<!-- example-verified: sha2 2026-06-02 -->/);
});

test('a failing example is reported as a failure (generation blocks)', async () => {
  const exec = makeExec(() => ({ stdout: '', stderr: 'boom', code: 7 }));
  const res = await verifyExamples({
    content: FAILING_PAGE, headSha: 'x', isoDate: '2026-06-03', exec,
  });
  assert.strictEqual(res.failed.length, 1, 'one failing example');
  assert.strictEqual(res.failed[0].code, 7);
  assert.match(res.failed[0].command, /exit 7/);
});

test('the executor receives the default 60s timeout and a worktree sandbox spec', async () => {
  const exec = passingExec();
  await verifyExamples({
    content: PASS_PAGE, headSha: 'x', isoDate: '2026-06-03', exec,
  });
  assert.strictEqual(exec.calls.length, 1);
  assert.strictEqual(exec.calls[0].timeoutMs, DEFAULT_TIMEOUT_MS);
  assert.strictEqual(exec.calls[0].sandbox, 'worktree');
});

test('an explicit timeoutMs overrides the default and is forwarded to the executor', async () => {
  const exec = passingExec();
  await verifyExamples({
    content: PASS_PAGE, headSha: 'x', isoDate: '2026-06-03', exec, timeoutMs: 5000,
  });
  assert.strictEqual(exec.calls[0].timeoutMs, 5000);
});

test('tutorial step sequences are collected as examples', () => {
  const page = [
    '## Steps',
    '',
    '<!-- example -->',
    '```bash',
    'step-one',
    '```',
    '',
    '<!-- example -->',
    '```bash',
    'step-two',
    '```',
    '',
  ].join('\n');
  const ex = parseExamples(page);
  assert.strictEqual(ex.length, 2);
  assert.deepEqual(ex.map(e => e.command.trim()), ['step-one', 'step-two']);
});

test('console block strips $ prompts and ignores illustrative output lines', () => {
  const page = [
    '<!-- example -->',
    '```console',
    '$ echo one',
    'sample output that must NOT execute',
    '$ echo two',
    '```',
    '',
  ].join('\n');
  const ex = parseExamples(page);
  assert.strictEqual(ex.length, 1);
  // Only the two prompted commands survive; the output line is dropped.
  assert.strictEqual(ex[0].command, 'echo one\necho two');
});

test('console block with no prompts runs verbatim', () => {
  const page = '<!-- example -->\n```console\nplain command\n```\n';
  const ex = parseExamples(page);
  assert.strictEqual(ex[0].command, 'plain command');
});

test('output containing triple backticks uses a longer fence (no markdown corruption)', async () => {
  const exec = passingExec('here is ```code``` inside\n');
  const res = await verifyExamples({
    content: PASS_PAGE, headSha: 'x', isoDate: '2026-06-03', exec,
  });
  // A 3-backtick fence would be closed early by the embedded ```; engine must
  // pick a 4+ backtick fence.
  assert.match(res.content, /````output\n[\s\S]*```code```[\s\S]*\n````/);
  // Re-running must still cleanly replace the long-fenced block (idempotent).
  const exec2 = passingExec('plain\n');
  const res2 = await verifyExamples({
    content: res.content, headSha: 'y', isoDate: '2026-06-04', exec: exec2,
  });
  const outputBlocks = (res2.content.match(/`{3,}output/g) || []).length;
  assert.strictEqual(outputBlocks, 1, 'one output block after re-run over a long fence');
  assert.doesNotMatch(res2.content, /code/);
});

test('verifyExamples stamps the executor-reported sandbox sha when no headSha given', async () => {
  const exec = makeExec(() => ({ stdout: 'ok\n', code: 0, sandboxSha: 'sandbox123' }));
  const res = await verifyExamples({
    content: PASS_PAGE, isoDate: '2026-06-03', exec,
  });
  assert.match(res.content, /<!-- example-verified: sandbox123 2026-06-03 -->/);
});

test('stampMarker produces the exact pinned marker format', () => {
  const m = stampMarker('feedface', '2026-06-03');
  assert.strictEqual(m, '<!-- example-verified: feedface 2026-06-03 -->');
});

// ── CLI smoke (subprocess of node itself, not the example sandbox) ────────────
// The CLI is run with --no-sandbox-exec and an injected fake-pass/fail mode so
// it never spawns a real worktree. A failing-example fixture must exit non-zero.

test('CLI exits non-zero on a failing-example fixture (deterministic, no real sandbox)', () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'verify-ex-cli-'));
  const page = path.join(dir, 'fail.md');
  fs.writeFileSync(page, FAILING_PAGE, 'utf8');
  let exitCode = 0;
  try {
    // AUTOSPEC_EXAMPLE_FAKE=fail makes the built-in executor return a non-zero
    // code WITHOUT spawning a worktree — keeps the suite hermetic.
    execFileSync(process.execPath, [MOD, page], {
      env: { ...process.env, AUTOSPEC_EXAMPLE_FAKE: 'fail' },
      stdio: 'pipe',
    });
  } catch (e) {
    exitCode = e.status ?? 1;
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
  assert.notStrictEqual(exitCode, 0, 'failing example must fail generation');
});

test('CLI exits 0 and rewrites the file on a passing-example fixture (fake-pass)', () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'verify-ex-cli-'));
  const page = path.join(dir, 'pass.md');
  fs.writeFileSync(page, PASS_PAGE, 'utf8');
  let exitCode = 0;
  try {
    execFileSync(process.execPath, [MOD, page], {
      env: { ...process.env, AUTOSPEC_EXAMPLE_FAKE: 'pass' },
      stdio: 'pipe',
    });
  } catch (e) {
    exitCode = e.status ?? 1;
  }
  const out = fs.readFileSync(page, 'utf8');
  fs.rmSync(dir, { recursive: true, force: true });
  assert.strictEqual(exitCode, 0, 'passing example must succeed');
  assert.match(out, /```output/);
  assert.match(out, /example-verified:/);
});


test('parseImplementationSnippets finds source-grounded fenced code fragments', () => {
  const page = [
    '# Implementation',
    '<!-- implementation-snippet: src/algo.js:2-3 -->',
    '```js',
    'const threshold = 0.8;',
    'return threshold;',
    '```',
    '',
  ].join('\n');
  const snippets = parseImplementationSnippets(page);
  assert.strictEqual(snippets.length, 1);
  assert.strictEqual(snippets[0].sourcePath, 'src/algo.js');
  assert.strictEqual(snippets[0].startLine, 2);
  assert.strictEqual(snippets[0].endLine, 3);
  assert.strictEqual(snippets[0].code, 'const threshold = 0.8;\nreturn threshold;');
});

test('verifyImplementationSnippets verifies rendered fragments against real source lines', () => {
  const root = makeTmpDir();
  try {
    fs.mkdirSync(path.join(root, 'src'), { recursive: true });
    fs.writeFileSync(path.join(root, 'src/algo.js'), [
      'function pick() {',
      '  const threshold = 0.8;',
      '  return threshold;',
      '}',
      '',
    ].join('\n'), 'utf8');
    const page = [
      '<!-- implementation-snippet: src/algo.js:2-3 -->',
      '```js',
      '  const threshold = 0.8;',
      '  return threshold;',
      '```',
      '',
    ].join('\n');
    const result = verifyImplementationSnippets({ content: page, repoRoot: root });
    assert.strictEqual(result.failed.length, 0);
    assert.strictEqual(result.verified, 1);
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test('verifyImplementationSnippets rejects invented or stale code fragments', () => {
  const root = makeTmpDir();
  try {
    fs.mkdirSync(path.join(root, 'src'), { recursive: true });
    fs.writeFileSync(path.join(root, 'src/algo.js'), 'const real = 1;\n', 'utf8');
    const page = [
      '<!-- implementation-snippet: src/algo.js:1-1 -->',
      '```js',
      'const invented = 2;',
      '```',
      '',
    ].join('\n');
    const result = verifyImplementationSnippets({ content: page, repoRoot: root });
    assert.strictEqual(result.verified, 0);
    assert.strictEqual(result.failed.length, 1);
    assert.match(result.failed[0].reason, /does not match source/);
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

// ── Track C: gen-audience-docs examples[] emission + llms-full.txt survival ───
// Issue #1133: gen-audience-docs.mjs must emit <!-- example --> fenced blocks
// from feature.examples[]; the output block and verified marker must survive
// generateLlmsFull concatenation unaltered.

import { generateLlmsFull } from '../scripts/gen-llms-full.mjs';
// generateAudienceDocs is the entry point for Track A+C together
const { generateAudienceDocs } = await import(
  path.resolve(__dirname, '../scripts/gen-audience-docs.mjs')
);
// EXAMPLE_TAG is already destructured from MOD above; no re-import needed.

// Minimal audience fixture for Track C tests
const TRACK_C_AUDIENCE = [
  { name: 'developer', path: 'docs/developer', focus: 'APIs, architecture, extending' },
];

// Feature with an examples[] array — the new Track C field
const FEATURE_WITH_EXAMPLES = {
  slug: 'greet',
  title: 'Greeter',
  summary: 'Emits a greeting.',
  spec_sections: [],
  code_entry_points: [],
  examples: [
    { lang: 'bash', command: 'echo hello-world' },
    { lang: 'bash', command: 'echo second-example' },
  ],
};

// Feature with no examples[] — backward compat must be unaffected
const FEATURE_NO_EXAMPLES = {
  slug: 'plain',
  title: 'Plain feature',
  summary: 'No examples.',
  spec_sections: [],
  code_entry_points: [],
};

test('Track C: gen-audience-docs emits <!-- example --> blocks for each feature.examples[] entry', async () => {
  const result = await generateAudienceDocs({
    features: [FEATURE_WITH_EXAMPLES],
    audiences: TRACK_C_AUDIENCE,
  });
  // The feature page should contain EXAMPLE_TAG for each entry
  const featurePage = result.files.find(f => f.path.includes('features/greet.md'));
  assert.ok(featurePage, 'feature page greet.md must be generated');
  const tagCount = (featurePage.content.match(/<!-- example -->/g) || []).length;
  assert.strictEqual(tagCount, 2, 'one <!-- example --> tag per examples[] entry');
  // Each command should appear in a fenced block after the tag
  assert.match(featurePage.content, /<!-- example -->\n```bash\necho hello-world\n```/);
  assert.match(featurePage.content, /<!-- example -->\n```bash\necho second-example\n```/);
});

test('Track C: feature with no examples[] emits no <!-- example --> blocks (backward compat)', async () => {
  const result = await generateAudienceDocs({
    features: [FEATURE_NO_EXAMPLES],
    audiences: TRACK_C_AUDIENCE,
  });
  const featurePage = result.files.find(f => f.path.includes('features/plain.md'));
  assert.ok(featurePage, 'feature page plain.md must be generated');
  assert.doesNotMatch(featurePage.content, /<!-- example -->/,
    'no example tag when examples[] is absent');
});

test('Track C: output block and example-verified marker survive generateLlmsFull concatenation', () => {
  // Build a page that already has a verified example (simulate post-verify-examples state)
  const verifiedPage = [
    '# Greeter (developer)',
    '',
    '<!-- example -->',
    '```bash',
    'echo hello-world',
    '```',
    '<!-- example-verified: deadbeef 2026-06-16 -->',
    '',
    '```output',
    'hello-world',
    '```',
    '',
  ].join('\n');

  const llmsFull = generateLlmsFull({
    pages: [{ audience: 'developer', feature: 'greet', path: 'docs/developer/features/greet.md', content: verifiedPage }],
  });

  // The marker must survive intact
  assert.match(llmsFull, /<!-- example-verified: deadbeef 2026-06-16 -->/,
    'example-verified marker must survive llms-full concat');
  // The output block must survive intact
  assert.match(llmsFull, /```output\nhello-world\n```/,
    'output fence block must survive llms-full concat');
  // The example tag must survive intact
  assert.match(llmsFull, /<!-- example -->/,
    '<!-- example --> tag must survive llms-full concat');
});

test('Track C: end-to-end — example in gen-audience-docs page + verify + llms-full survival', async () => {
  // 1. Generate the docs page with examples
  const result = await generateAudienceDocs({
    features: [FEATURE_WITH_EXAMPLES],
    audiences: TRACK_C_AUDIENCE,
  });
  const featurePage = result.files.find(f => f.path.includes('features/greet.md'));
  assert.ok(featurePage, 'feature page must exist');

  // 2. Run verifyExamples with a fake executor (no real sandbox needed)
  const fakeExec = async ({ command }) => ({
    stdout: `${command}-output\n`,
    stderr: '',
    code: 0,
  });
  const verified = await verifyExamples({
    content: featurePage.content,
    headSha: 'cafe1234',
    isoDate: '2026-06-16',
    exec: fakeExec,
  });
  assert.strictEqual(verified.failed.length, 0, 'no failures from fake executor');
  assert.ok(verified.verified >= 1, 'at least one example verified');
  // Marker must be in the stamped page
  assert.match(verified.content, /<!-- example-verified: cafe1234 2026-06-16 -->/,
    'marker must appear in verified page content');
  // Output block must be in the stamped page
  assert.match(verified.content, /```output/, 'output block must appear in verified page');

  // 3. Concatenate via generateLlmsFull and confirm marker/output survive
  const llmsFull = generateLlmsFull({
    pages: [{ audience: 'developer', feature: 'greet', path: featurePage.path, content: verified.content }],
  });
  assert.match(llmsFull, /<!-- example-verified: cafe1234 2026-06-16 -->/,
    'marker must survive into llms-full.txt');
  assert.match(llmsFull, /```output/,
    'output block must survive into llms-full.txt');
});
