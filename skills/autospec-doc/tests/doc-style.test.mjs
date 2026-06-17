// doc-style.test.mjs — unit tests for skills/autospec-doc/scripts/doc-style.mjs (issue #920)
//
// Tests:
//   1. PALETTE export has exactly 6 keys matching spec §D4 names
//   2. Each palette value matches the pinned §D4 hex exactly
//   3. mermaidInit() emits all 6 palette hex values
//   4. mermaidInit() emits %%{init ...}%% wrapper
//   5. mermaidInit() emits theme:base
//   6. mermaidInit() output is valid %%{init}%% JSON (parseable)
//   7. resolvePalette('light-blue') returns the light-blue PALETTE object
//   8. resolvePalette with unknown preset returns null
//   9. mermaidInitForPreset('light-blue') emits the same block as mermaidInit()
//  10. gen-arch-diagram.mjs --style flag prepends the init block before graph content
//  11. isLogicFlowSection: ordered-step section returns true
//  12. isLogicFlowSection: plain prose section returns false
//  13. isLogicFlowSection: decision-rule section returns true
//  14. isLogicFlowSection: state-machine section returns true
//  15. generateExplainerDiagram: returns a themed flowchart string for a logic section
//  16. generateExplainerDiagram: returns null for a non-logic section
//  17. single-source guard: no hex from PALETTE appears in any file outside doc-style.mjs

import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { execFileSync, spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const SCRIPTS_DIR = path.resolve(__dirname, '../scripts');
const REPO_ROOT = path.resolve(__dirname, '../../../..');

const { PALETTE, mermaidInit, resolvePalette, mermaidInitForPreset, isLogicFlowSection, generateExplainerDiagram, truncateLabel } =
  await import(path.join(SCRIPTS_DIR, 'doc-style.mjs'));

// ── Spec §D4 pinned values ────────────────────────────────────────────────────

const EXPECTED_HEXES = {
  background: '#E3F2FD',
  primary:    '#90CAF9',
  secondary:  '#BBDEFB',
  accent:     '#1E88E5',
  line:       '#64B5F6',
  text:       '#0D2C45',
};

// ── Tests 1-2: PALETTE export ─────────────────────────────────────────────────

test('PALETTE export has exactly 6 keys matching spec §D4 names', () => {
  const keys = Object.keys(PALETTE).sort();
  const expected = Object.keys(EXPECTED_HEXES).sort();
  assert.deepEqual(keys, expected);
});

test('each palette value matches the pinned §D4 hex exactly', () => {
  for (const [key, hex] of Object.entries(EXPECTED_HEXES)) {
    assert.strictEqual(
      PALETTE[key],
      hex,
      `PALETTE.${key} must be ${hex} (got ${PALETTE[key]})`
    );
  }
});

// ── Tests 3-6: mermaidInit() ──────────────────────────────────────────────────

test('mermaidInit() emits all 6 palette hex values', () => {
  const block = mermaidInit();
  for (const [key, hex] of Object.entries(EXPECTED_HEXES)) {
    assert.ok(
      block.includes(hex),
      `mermaidInit() output must contain ${hex} (for ${key})`
    );
  }
});

test('mermaidInit() emits %%{init ...}%% wrapper', () => {
  const block = mermaidInit();
  assert.ok(block.startsWith('%%{init:'), `mermaidInit() must start with %%{init: — got: ${block.slice(0, 30)}`);
  assert.ok(block.trimEnd().endsWith('}%%'), `mermaidInit() must end with }%% — got: ${block.slice(-20)}`);
});

test('mermaidInit() emits theme:base', () => {
  const block = mermaidInit();
  assert.ok(block.includes("'theme':'base'") || block.includes('"theme":"base"') || block.includes("'theme': 'base'"),
    `mermaidInit() must include theme:base`);
});

test('mermaidInit() output is valid %%{init}%% JSON (parseable)', () => {
  const block = mermaidInit();
  // Strip %%{init: ... }%%
  const inner = block.replace(/^%%\{init:\s*/, '').replace(/\s*\}%%\s*$/, '');
  let parsed;
  try {
    parsed = JSON.parse(inner);
  } catch (e) {
    assert.fail(`mermaidInit() inner JSON is not parseable: ${e.message}\nBlock: ${block}`);
  }
  assert.ok(parsed && typeof parsed === 'object', 'parsed inner must be an object');
  assert.ok('themeVariables' in parsed, 'parsed must have themeVariables key');
});

// ── Tests 7-9: resolvePalette + mermaidInitForPreset ─────────────────────────

test('resolvePalette(\'light-blue\') returns the light-blue PALETTE object', () => {
  const pal = resolvePalette('light-blue');
  assert.deepEqual(pal, PALETTE);
});

test('resolvePalette with unknown preset returns null', () => {
  const pal = resolvePalette('does-not-exist');
  assert.strictEqual(pal, null);
});

test('mermaidInitForPreset(\'light-blue\') emits the same block as mermaidInit()', () => {
  const a = mermaidInit();
  const b = mermaidInitForPreset('light-blue');
  assert.strictEqual(b, a);
});

// ── Test 10: gen-arch-diagram.mjs --style flag ────────────────────────────────

test('gen-arch-diagram.mjs --style flag prepends the init block before graph content', () => {
  const GEN_ARCH = path.resolve(__dirname, '../../autospec-shared/scripts/gen-arch-diagram.mjs');
  const clusterFixture = path.resolve(__dirname, '../../autospec-shared/tests/fixtures/cluster-sample.json');
  if (!fs.existsSync(GEN_ARCH)) {
    // Skip if gen-arch-diagram not present (shouldn't happen)
    return;
  }
  if (!fs.existsSync(clusterFixture)) {
    return;
  }
  const result = spawnSync(process.execPath, [GEN_ARCH, '--cluster', clusterFixture, '--style'], {
    encoding: 'utf8',
    timeout: 10000,
  });
  assert.strictEqual(result.status, 0, `gen-arch-diagram.mjs --style exited ${result.status}: ${result.stderr}`);
  const out = result.stdout;
  assert.ok(out.includes('%%{init:'), `--style output must contain mermaid init block, got: ${out.slice(0, 200)}`);
  assert.ok(out.includes(EXPECTED_HEXES.primary), `--style output must contain primary hex ${EXPECTED_HEXES.primary}`);
});

// ── Tests 11-14: isLogicFlowSection ──────────────────────────────────────────

test('isLogicFlowSection: ordered-step section returns true', () => {
  const section = {
    heading: 'Processing Pipeline',
    body: '1. Validate input\n2. Transform data\n3. Write output\n4. Confirm success',
  };
  assert.strictEqual(isLogicFlowSection(section), true);
});

test('isLogicFlowSection: plain prose section returns false', () => {
  const section = {
    heading: 'Overview',
    body: 'This is a general overview of the system. It describes the background context.',
  };
  assert.strictEqual(isLogicFlowSection(section), false);
});

test('isLogicFlowSection: decision-rule section returns true', () => {
  const section = {
    heading: 'Routing Logic',
    body: 'If the request is authenticated, route to /api. Otherwise, redirect to /login.',
  };
  assert.strictEqual(isLogicFlowSection(section), true);
});

test('isLogicFlowSection: state-machine section returns true', () => {
  const section = {
    heading: 'Issue States',
    body: 'States: open → in-progress → review → closed. Transitions are guarded by labels.',
  };
  assert.strictEqual(isLogicFlowSection(section), true);
});

// ── Tests 15-16: generateExplainerDiagram ────────────────────────────────────

test('generateExplainerDiagram: returns a themed flowchart string for a logic section', () => {
  const section = {
    heading: 'Processing Steps',
    body: '1. Read config\n2. Parse input\n3. Generate output\n4. Write file',
  };
  const diagram = generateExplainerDiagram(section);
  assert.ok(typeof diagram === 'string' && diagram.length > 0,
    'generateExplainerDiagram must return a non-empty string for a logic section');
  assert.ok(diagram.includes('%%{init:'), `diagram must include mermaid init block, got: ${diagram.slice(0, 200)}`);
  assert.ok(diagram.includes('flowchart') || diagram.includes('graph') || diagram.includes('sequenceDiagram'),
    `diagram must include a mermaid diagram type, got: ${diagram.slice(0, 200)}`);
});

test('generateExplainerDiagram: returns null for a non-logic section', () => {
  const section = {
    heading: 'Background',
    body: 'This section provides background context about the project history.',
  };
  const diagram = generateExplainerDiagram(section);
  assert.strictEqual(diagram, null);
});

// ── Test 17: single-source guard ─────────────────────────────────────────────

test('single-source guard: no hex from PALETTE appears in any file outside doc-style.mjs and its test', () => {
  // Directories to scan (narrow to .mjs files to keep this fast)
  const hexValues = Object.values(PALETTE);
  const violations = [];

  function scanDir(dir) {
    if (!fs.existsSync(dir)) return;
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      const fullPath = path.join(dir, entry.name);
      if (entry.isDirectory()) {
        // Skip node_modules, .git, test fixtures
        if (['node_modules', '.git', 'fixtures'].includes(entry.name)) continue;
        scanDir(fullPath);
      } else if (entry.isFile() && (entry.name.endsWith('.mjs') || entry.name.endsWith('.sh'))) {
        // NOTE: .md files (specs, docs) are allowed to document hex values;
        // only source code (.mjs, .sh) must not hardcode them.
        // Skip doc-style.mjs itself and its test
        if (entry.name === 'doc-style.mjs') continue;
        if (entry.name === 'doc-style.test.mjs') continue;
        let content;
        try { content = fs.readFileSync(fullPath, 'utf8'); } catch { continue; }
        for (const hex of hexValues) {
          if (content.includes(hex)) {
            violations.push(`${fullPath}: contains palette hex ${hex}`);
          }
        }
      }
    }
  }

  // Scan the full repo (skills/ + scripts/ + docs/)
  for (const topDir of ['skills', 'scripts', 'docs']) {
    scanDir(path.join(REPO_ROOT, topDir));
  }

  assert.deepEqual(violations, [], `Single-source violation: palette hexes found outside doc-style.mjs:\n${violations.join('\n')}`);
});

// ── Tests 18+: diagram-generation bug fixes ──────────────────────────────────

test('truncateLabel: short string is returned unchanged (no ellipsis)', () => {
  const s = 'discovering the runner';
  assert.strictEqual(truncateLabel(s, 80), s);
  assert.ok(!truncateLabel(s, 80).includes('…'));
});

test('truncateLabel: never produces a mid-word cut and appends … only when truncated', () => {
  const s = 'discovering the runner with the appropriate configuration';
  const out = truncateLabel(s, 20);
  assert.ok(out.length <= s.length);
  assert.ok(out.endsWith('…'), `expected ellipsis, got: ${out}`);
  // The portion before the ellipsis must be a prefix of whole words from the input.
  const kept = out.slice(0, -1);
  assert.ok(s.startsWith(kept), `kept portion must be a prefix of the input, got: ${kept}`);
  // No partial word: the char in the source immediately after `kept` is whitespace
  // (i.e. we cut on a word boundary), unless we kept the whole string.
  if (kept.length < s.length) {
    assert.match(s[kept.length], /\s/, `cut must land on a word boundary, got next char: '${s[kept.length]}'`);
  }
});

test('truncateLabel: hard-cuts a first word that alone exceeds max', () => {
  const s = 'antidisestablishmentarianism';
  const out = truncateLabel(s, 10);
  assert.ok(out.endsWith('…'));
  assert.strictEqual(out, s.slice(0, 10) + '…');
});

test('flowchart step label longer than max is truncated at a word boundary', () => {
  const longStep = 'Validate the incoming request payload against the published schema and reject anything malformed immediately';
  const section = {
    heading: 'Processing Steps',
    body: `1. ${longStep}\n2. Transform data\n3. Write output`,
  };
  const diagram = generateExplainerDiagram(section);
  assert.ok(diagram.includes('flowchart'));
  // The truncated label must appear with an ellipsis and not contain the final partial token.
  const m = diagram.match(/S1\["([^"]*)"\]/);
  assert.ok(m, `expected S1 node label, got: ${diagram}`);
  const label = m[1];
  assert.ok(label.endsWith('…'), `label must end with ellipsis, got: ${label}`);
  // The input's final word "immediately" must not be partially present.
  assert.ok(!/immediatel\b/.test(label), `label must not contain a mid-word cut, got: ${label}`);
  assert.ok(longStep.startsWith(label.slice(0, -1)), 'kept portion must be a word-prefix of the step');
});

test('generateExplainerDiagram: returns null for logic-ish prose with no steps, arrows, or if/otherwise pair', () => {
  const section = {
    heading: 'Pipeline Overview',
    body: 'The pipeline processes records through a series of rules and routes them according to the configured workflow. This describes the general flow at a high level.',
  };
  const diagram = generateExplainerDiagram(section);
  assert.strictEqual(diagram, null, `expected null, got: ${diagram}`);
});

test('generateExplainerDiagram: does NOT emit the old fabricated decision placeholder', () => {
  const section = {
    heading: 'Routing Process',
    body: 'When a request arrives the process applies a rule to decide the route through the pipeline. The branch is chosen by the guard.',
  };
  const diagram = generateExplainerDiagram(section);
  // Must not produce the old placeholder fabricated-decision output.
  if (diagram !== null) {
    assert.ok(!diagram.includes('Condition?'), `must not emit Condition? placeholder, got: ${diagram}`);
    assert.ok(!diagram.includes('True branch'), `must not emit True branch placeholder, got: ${diagram}`);
    assert.ok(!diagram.includes('False branch'), `must not emit False branch placeholder, got: ${diagram}`);
  } else {
    assert.strictEqual(diagram, null);
  }
});

test('generateExplainerDiagram: real if/otherwise section emits decision flowchart with real branch labels', () => {
  const section = {
    heading: 'Routing Logic',
    body: 'If the request is authenticated route to dashboard, otherwise redirect to login.',
  };
  const diagram = generateExplainerDiagram(section);
  assert.ok(typeof diagram === 'string', `expected a diagram, got: ${diagram}`);
  assert.ok(diagram.includes('flowchart'));
  // Branch labels must contain the real extracted text, not the placeholders.
  assert.ok(diagram.includes('the request is authenticated route to dashboard') ||
            diagram.includes('the request is authenticated'),
    `Yes branch must contain real text, got: ${diagram}`);
  assert.ok(diagram.includes('redirect to login'), `No branch must contain real text, got: ${diagram}`);
  assert.ok(!diagram.includes('True branch'), `must not contain True branch, got: ${diagram}`);
  assert.ok(!diagram.includes('False branch'), `must not contain False branch, got: ${diagram}`);
});

test('buildStateDiagram path: clean A -> B -> C section yields a full state diagram with [*] start/end', () => {
  const section = {
    heading: 'Issue States',
    body: 'States: open -> in-progress -> review -> closed. Transitions are guarded by labels.',
  };
  const diagram = generateExplainerDiagram(section);
  assert.ok(typeof diagram === 'string', `expected a diagram, got: ${diagram}`);
  assert.ok(diagram.includes('stateDiagram-v2'));
  for (const state of ['open', 'in_progress', 'review', 'closed']) {
    assert.ok(diagram.includes(state), `diagram must include state ${state}, got: ${diagram}`);
  }
  assert.ok(diagram.includes('[*] -->'), 'must include a start transition');
  assert.ok(diagram.includes('--> [*]'), 'must include an end transition');
});

test('buildStateDiagram path: stray -> inside prose with long tokens returns null', () => {
  const section = {
    heading: 'Background',
    body: 'The system takes the raw measurement data and transforms it -> producing a cleaned-up normalized result that downstream consumers can rely on for analysis.',
  };
  const diagram = generateExplainerDiagram(section);
  assert.strictEqual(diagram, null, `expected null for stray-arrow prose, got: ${diagram}`);
});
