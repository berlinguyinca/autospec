#!/usr/bin/env node
// verify-examples.mjs — docs-as-tests example engine (issue #919, spec §D3).
//
// The quality invariant: **if it's in the docs, it ran.**
//
// 1. Scan generated pages for fenced ```bash / ```console blocks tagged with the
//    `<!-- example -->` comment (and tutorial step sequences, which are just a
//    series of tagged blocks).
// 2. Execute each example in an isolated sandbox (a fresh worktree off
//    origin/main; network-restricted where feasible; per-example timeout,
//    default 60s).
// 3. Embed the real captured output in an adjacent ```output block and stamp a
//    `<!-- example-verified: <head-sha> <ISO-date> -->` marker.
// 4. A failing example FAILS generation — verifyExamples reports it and the CLI
//    exits non-zero, blocking the doc PR exactly like a failing test. (Code PRs
//    are never blocked by doc generation — this script runs only in the doc
//    pipeline.)
//
// NO mocks in production: the default executor really shells out. Tests inject a
// fake `exec` (or set AUTOSPEC_EXAMPLE_FAKE) so the suite never spawns a worktree
// and never stalls — only the engine contract is exercised there.
//
// Reuse: the worktree sandbox mirrors the fresh-worktree-off-origin/main pattern
// used throughout autospec (e.g. the Phase 4 implementer prompt and
// gen-screenshots.mjs's replay path). Web walkthroughs / CLI casts replay through
// gen-screenshots.mjs's Playwright/asciinema path; this engine governs the
// fenced-command examples and delegates richer replay to that existing script.
//
// Exports:
//   parseExamples(content)            -> Example[]
//   verifyExamples(opts)              -> { content, failed, verified }
//   stampMarker(sha, iso)             -> string
//   makeWorktreeExecutor(opts)        -> exec fn (the real, shelling executor)
//   DEFAULT_TIMEOUT_MS, EXAMPLE_TAG, OUTPUT_LANG, MARKER_RE
//
// Example: { lang, command, blockStart, blockEnd, tagStart }
//   byte/char offsets into the original content string.

import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { spawnSync, execSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

// ── Pinned contract constants ─────────────────────────────────────────────────

export const EXAMPLE_TAG = '<!-- example -->';
export const DEFAULT_TIMEOUT_MS = 60_000; // 60s per example (spec §D3 step 2)
export const OUTPUT_LANG = 'output';

// Marker format is pinned verbatim by the #916-#925 shared contract:
//   <!-- example-verified: <head-sha> <ISO-date> -->
const MARKER_RE = /<!-- example-verified: \S+ \S+ -->/g;
export { MARKER_RE };

const EXAMPLE_LANGS = new Set(['bash', 'console', 'sh', 'shell']);

export function stampMarker(sha, iso) {
  return `<!-- example-verified: ${sha} ${iso} -->`;
}

// ── Parse ─────────────────────────────────────────────────────────────────────

// Find every `<!-- example -->`-tagged fenced bash/console block. A tutorial
// "step sequence" is simply several such tagged blocks in a row, so collecting
// each tagged block individually covers both single examples and step lists.
//
// Returns examples in document order with char offsets so the caller can splice
// in an adjacent output block deterministically.
export function parseExamples(content) {
  const examples = [];
  const lines = content.split('\n');

  // Precompute the char offset at the start of each line.
  const lineOffsets = [];
  let off = 0;
  for (const ln of lines) {
    lineOffsets.push(off);
    off += ln.length + 1; // +1 for the '\n'
  }

  const tagRe = /^\s*<!--\s*example\s*-->\s*$/;
  const fenceOpenRe = /^\s*```([A-Za-z0-9_-]*)\s*$/;
  const fenceCloseRe = /^\s*```\s*$/;

  for (let i = 0; i < lines.length; i++) {
    if (!tagRe.test(lines[i])) continue;
    // Next non-blank line must open a fence; allow one optional blank line.
    let j = i + 1;
    while (j < lines.length && lines[j].trim() === '') j++;
    if (j >= lines.length) continue;
    const openM = fenceOpenRe.exec(lines[j]);
    if (!openM) continue;
    const lang = (openM[1] || '').toLowerCase();
    if (!EXAMPLE_LANGS.has(lang)) continue;

    // Find the closing fence.
    let k = j + 1;
    while (k < lines.length && !fenceCloseRe.test(lines[k])) k++;
    if (k >= lines.length) continue; // unterminated fence — skip defensively

    const command = lines.slice(j + 1, k).join('\n');
    examples.push({
      lang: lang === 'sh' || lang === 'shell' ? 'bash' : lang,
      command,
      tagStart: lineOffsets[i],
      blockStart: lineOffsets[j],
      // char offset just past the closing-fence line (incl. its newline if any)
      blockEnd: k + 1 < lines.length ? lineOffsets[k + 1] : content.length,
    });
    i = k; // continue scanning after this block
  }
  return examples;
}

// ── Output / marker splicing ──────────────────────────────────────────────────

// Build the replacement text that follows a verified example: a fresh marker and
// an output block. We rewrite the whole region after the example fence so re-runs
// replace (never duplicate) any prior marker/output.
function renderVerifiedTail(sha, iso, output) {
  const body = output.replace(/\s+$/, ''); // trim trailing whitespace/newlines
  return `\n${stampMarker(sha, iso)}\n\n\`\`\`${OUTPUT_LANG}\n${body}\n\`\`\`\n`;
}

// After an example's closing fence there may already be a verified marker and/or
// an output block from a prior run. Consume them so we can replace cleanly.
function consumeExistingTail(content, fromOffset) {
  let rest = content.slice(fromOffset);
  // Leading blank lines.
  const tail = /^(\s*)/.exec(rest);
  let consumed = tail ? tail[1].length : 0;
  rest = content.slice(fromOffset + consumed);
  // Optional existing marker line.
  const mk = /^<!-- example-verified: \S+ \S+ -->[ \t]*\n?/.exec(rest);
  if (mk) {
    consumed += mk[0].length;
    rest = content.slice(fromOffset + consumed);
    const blanks = /^(\s*)/.exec(rest);
    if (blanks) { consumed += blanks[1].length; rest = content.slice(fromOffset + consumed); }
  }
  // Optional existing ```output block.
  const ob = /^```output[ \t]*\n[\s\S]*?\n```[ \t]*\n?/.exec(rest);
  if (ob) consumed += ob[0].length;
  return consumed;
}

// ── Verify ────────────────────────────────────────────────────────────────────

// opts:
//   content   string                       page markdown
//   headSha   string                       HEAD sha for the marker
//   isoDate   string                       ISO date for the marker
//   exec      async ({command,lang,timeoutMs,sandbox}) => {stdout,stderr,code}
//             (default: makeWorktreeExecutor() — the real shelling executor)
//   timeoutMs number  (default DEFAULT_TIMEOUT_MS)
//   sandbox   string  (default 'worktree')
//
// Returns { content, failed: Failure[], verified: number }.
//   Failure: { command, lang, code, stderr }
export async function verifyExamples(opts) {
  const {
    content,
    headSha,
    isoDate,
    exec = makeWorktreeExecutor(),
    timeoutMs = DEFAULT_TIMEOUT_MS,
    sandbox = 'worktree',
  } = opts;

  const examples = parseExamples(content);
  const failed = [];
  let verified = 0;

  // Run examples sequentially in a single shared sandbox (each in a clean
  // sub-shell). Splice from the end so earlier offsets stay valid.
  const results = [];
  for (const ex of examples) {
    const r = await exec({ command: ex.command, lang: ex.lang, timeoutMs, sandbox });
    results.push({ ex, r });
    if (r.code !== 0) {
      failed.push({ command: ex.command, lang: ex.lang, code: r.code, stderr: r.stderr || '' });
    } else {
      verified++;
    }
  }

  // Rewrite content: replace each example's trailing region with a fresh marker
  // + output block. Process in reverse document order to keep offsets valid.
  let out = content;
  for (let n = results.length - 1; n >= 0; n--) {
    const { ex, r } = results[n];
    if (r.code !== 0) continue; // failing example: do not stamp (it blocks)
    const consumed = consumeExistingTail(out, ex.blockEnd);
    const tail = renderVerifiedTail(headSha, isoDate, r.stdout || '');
    out = out.slice(0, ex.blockEnd) + tail + out.slice(ex.blockEnd + consumed);
  }

  return { content: out, failed, verified };
}

// ── Real executor (fresh worktree off origin/main) ────────────────────────────

// makeWorktreeExecutor creates ONE fresh worktree off origin/main and runs every
// example inside it in a clean sub-shell, network-restricted where feasible, with
// a per-example timeout. The worktree is torn down on `dispose()`.
//
// AUTOSPEC_EXAMPLE_FAKE short-circuits real execution for hermetic tests:
//   'pass' → every example returns {stdout:'<command echoed>', code:0}
//   'fail' → every example returns {code:1}
// This lets the CLI test exercise the full file-rewrite path without a worktree.
export function makeWorktreeExecutor(execOpts = {}) {
  const fake = execOpts.fake ?? process.env.AUTOSPEC_EXAMPLE_FAKE ?? '';
  let worktreeDir = null;
  let worktreeRepoRoot = null;

  async function ensureWorktree() {
    if (fake) return null;
    if (worktreeDir) return worktreeDir;
    const repoRoot = execOpts.repoRoot
      || execSync('git rev-parse --show-toplevel', { encoding: 'utf8' }).trim();
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'autospec-example-wt-'));
    fs.rmSync(dir, { recursive: true, force: true });
    execSync('git fetch origin --quiet', { cwd: repoRoot, stdio: 'ignore' });
    execSync(
      `git worktree add --detach ${JSON.stringify(dir)} origin/main`,
      { cwd: repoRoot, stdio: 'ignore' },
    );
    worktreeDir = dir;
    worktreeRepoRoot = repoRoot;
    return worktreeDir;
  }

  const exec = async ({ command, timeoutMs }) => {
    if (fake === 'pass') return { stdout: `${command}\n`, stderr: '', code: 0 };
    if (fake === 'fail') return { stdout: '', stderr: 'forced failure', code: 1 };
    if (fake) return { stdout: '', stderr: `unknown fake mode: ${fake}`, code: 2 };

    const dir = await ensureWorktree();
    // Network restriction: unset proxy/credential env where feasible. True
    // network namespacing is platform-specific; we degrade gracefully to a
    // scrubbed-env best effort (documented limitation).
    const childEnv = { ...process.env };
    delete childEnv.http_proxy; delete childEnv.https_proxy;
    delete childEnv.HTTP_PROXY; delete childEnv.HTTPS_PROXY;
    childEnv.GIT_TERMINAL_PROMPT = '0';

    const res = spawnSync('bash', ['-eo', 'pipefail', '-c', command], {
      cwd: dir,
      timeout: timeoutMs,
      env: childEnv,
      encoding: 'utf8',
      maxBuffer: 8 * 1024 * 1024,
    });
    if (res.error && res.error.code === 'ETIMEDOUT') {
      return { stdout: res.stdout || '', stderr: `timed out after ${timeoutMs}ms`, code: 124 };
    }
    return {
      stdout: res.stdout || '',
      stderr: res.stderr || '',
      code: res.status == null ? 1 : res.status,
    };
  };

  exec.dispose = () => {
    if (worktreeDir && worktreeRepoRoot) {
      try {
        execSync(`git worktree remove --force ${JSON.stringify(worktreeDir)}`,
          { cwd: worktreeRepoRoot, stdio: 'ignore' });
      } catch { /* best-effort cleanup */ }
      worktreeDir = null;
      worktreeRepoRoot = null;
    }
  };
  return exec;
}

// ── CLI ───────────────────────────────────────────────────────────────────────

async function main(argv) {
  const files = argv.filter((a) => !a.startsWith('-'));
  if (files.length === 0) {
    process.stderr.write('usage: verify-examples.mjs <page.md> [<page.md> ...]\n');
    return 2;
  }

  let headSha = 'unknown';
  let isoDate = new Date().toISOString().slice(0, 10);
  try {
    headSha = execSync('git rev-parse HEAD', { encoding: 'utf8' }).trim();
  } catch { /* not a git repo / detached — leave 'unknown' */ }

  const exec = makeWorktreeExecutor();
  let anyFailed = false;
  try {
    for (const file of files) {
      const content = fs.readFileSync(file, 'utf8');
      const res = await verifyExamples({ content, headSha, isoDate, exec });
      if (res.failed.length > 0) {
        anyFailed = true;
        for (const f of res.failed) {
          process.stderr.write(
            `FAIL example in ${file} (exit ${f.code}):\n  $ ${f.command}\n  ${f.stderr}\n`,
          );
        }
        // Do not rewrite a file with a failing example — generation is blocked.
        continue;
      }
      if (res.content !== content) fs.writeFileSync(file, res.content, 'utf8');
      process.stdout.write(`OK ${file}: ${res.verified} example(s) verified\n`);
    }
  } finally {
    exec.dispose();
  }
  return anyFailed ? 1 : 0;
}

// Run as CLI when invoked directly.
if (import.meta.url === `file://${process.argv[1]}`) {
  main(process.argv.slice(2)).then((code) => process.exit(code));
}
