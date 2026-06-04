// doc-auto-regenerate.test.mjs — unit tests for resolveAutoRegenerate + auto_regenerate
// config key in skills/autospec-doc/scripts/doc-config.mjs (issue #953)
//
// Tests:
//   1.  default: no opt-in → generate=false, reason='default-off'
//   2.  config opt-in alone → generate=true, reason='config'
//   3.  withDocsFlag opt-in alone → generate=true, reason='flag'
//   4.  docs: generate body line alone → generate=true, reason='generate-line'
//   5.  docs: skip beats docs: generate → generate=false, reason='skip-line'
//   6.  docs: skip beats config opt-in → generate=false, reason='skip-line'
//   7.  docs: skip beats withDocsFlag → generate=false, reason='skip-line'
//   8.  docs: generate beats config opt-in (both present) → reason='generate-line'
//   9.  docs: generate beats withDocsFlag (both present) → reason='generate-line'
//  10.  docs: generate is case-insensitive → generate=true
//  11.  docs: skip is case-insensitive → generate=false
//  12.  docs: generate on its own line (no leading spaces) → matches
//  13.  partial match does NOT trigger (e.g. "docs: generate more") → default-off
//  14.  loadConfig returns documentation.auto_regenerate=false by default (no yml)
//  15.  loadConfig reads documentation.auto_regenerate=true from yml
//  16.  loadConfig reads documentation.auto_regenerate=false explicitly from yml
//  17.  loadConfig includes documentation key alongside audiences/style/examples
//  18.  init round-trip: auto_regenerate key survives write+read

import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const CONFIG_MOD = path.resolve(__dirname, '../scripts/doc-config.mjs');

const { loadConfig, resolveAutoRegenerate } = await import(CONFIG_MOD);

// ── Helpers ───────────────────────────────────────────────────────────────────

function makeTmpDir() {
  return fs.mkdtempSync(path.join(os.tmpdir(), 'doc-auto-regen-test-'));
}

function writeYml(dir, content) {
  fs.mkdirSync(path.join(dir, '.autospec'), { recursive: true });
  fs.writeFileSync(path.join(dir, '.autospec', 'autospec.yml'), content, 'utf8');
}

function configPath(dir) {
  return path.join(dir, '.autospec', 'autospec.yml');
}

// ── resolveAutoRegenerate tests ───────────────────────────────────────────────

test('default: no opt-in returns generate=false, reason=default-off', () => {
  const result = resolveAutoRegenerate({ config: {}, issueBody: '', withDocsFlag: false });
  assert.strictEqual(result.generate, false);
  assert.strictEqual(result.reason, 'default-off');
});

test('config opt-in alone returns generate=true, reason=config', () => {
  const config = { documentation: { auto_regenerate: true } };
  const result = resolveAutoRegenerate({ config, issueBody: '', withDocsFlag: false });
  assert.strictEqual(result.generate, true);
  assert.strictEqual(result.reason, 'config');
});

test('withDocsFlag=true alone returns generate=true, reason=flag', () => {
  const result = resolveAutoRegenerate({ config: {}, issueBody: '', withDocsFlag: true });
  assert.strictEqual(result.generate, true);
  assert.strictEqual(result.reason, 'flag');
});

test('docs: generate body line alone returns generate=true, reason=generate-line', () => {
  const result = resolveAutoRegenerate({ config: {}, issueBody: 'Some text\ndocs: generate\nMore text', withDocsFlag: false });
  assert.strictEqual(result.generate, true);
  assert.strictEqual(result.reason, 'generate-line');
});

test('docs: skip beats docs: generate → generate=false, reason=skip-line', () => {
  const result = resolveAutoRegenerate({
    config: {},
    issueBody: 'docs: generate\ndocs: skip',
    withDocsFlag: false,
  });
  assert.strictEqual(result.generate, false);
  assert.strictEqual(result.reason, 'skip-line');
});

test('docs: skip beats config opt-in → generate=false, reason=skip-line', () => {
  const config = { documentation: { auto_regenerate: true } };
  const result = resolveAutoRegenerate({ config, issueBody: 'docs: skip', withDocsFlag: false });
  assert.strictEqual(result.generate, false);
  assert.strictEqual(result.reason, 'skip-line');
});

test('docs: skip beats withDocsFlag → generate=false, reason=skip-line', () => {
  const result = resolveAutoRegenerate({ config: {}, issueBody: 'docs: skip', withDocsFlag: true });
  assert.strictEqual(result.generate, false);
  assert.strictEqual(result.reason, 'skip-line');
});

test('docs: generate beats config opt-in when both present → reason=generate-line', () => {
  const config = { documentation: { auto_regenerate: true } };
  const result = resolveAutoRegenerate({ config, issueBody: 'docs: generate', withDocsFlag: false });
  assert.strictEqual(result.generate, true);
  assert.strictEqual(result.reason, 'generate-line');
});

test('docs: generate beats withDocsFlag when both present → reason=generate-line', () => {
  const result = resolveAutoRegenerate({ config: {}, issueBody: 'docs: generate', withDocsFlag: true });
  assert.strictEqual(result.generate, true);
  assert.strictEqual(result.reason, 'generate-line');
});

test('docs: generate is case-insensitive → generate=true', () => {
  const result = resolveAutoRegenerate({ config: {}, issueBody: 'DOCS: GENERATE', withDocsFlag: false });
  assert.strictEqual(result.generate, true);
  assert.strictEqual(result.reason, 'generate-line');
});

test('docs: skip is case-insensitive → generate=false', () => {
  const result = resolveAutoRegenerate({ config: {}, issueBody: 'DOCS: SKIP', withDocsFlag: false });
  assert.strictEqual(result.generate, false);
  assert.strictEqual(result.reason, 'skip-line');
});

test('docs: generate on its own line (no leading spaces, trailing newlines) matches', () => {
  // Spec pattern: ^docs:\s*generate\s*$ — no leading-space allowance; line must start with "docs:".
  const body = '\n\ndocs: generate\n\n';
  const result = resolveAutoRegenerate({ config: {}, issueBody: body, withDocsFlag: false });
  assert.strictEqual(result.generate, true);
  assert.strictEqual(result.reason, 'generate-line');
});

test('partial match "docs: generate more" does NOT trigger → default-off', () => {
  const result = resolveAutoRegenerate({ config: {}, issueBody: 'docs: generate more stuff', withDocsFlag: false });
  assert.strictEqual(result.generate, false);
  assert.strictEqual(result.reason, 'default-off');
});

// ── loadConfig documentation.auto_regenerate tests ───────────────────────────

test('loadConfig returns documentation.auto_regenerate=false by default (missing file)', () => {
  const dir = makeTmpDir();
  // No yml written — file missing → defaults.
  const cfg = loadConfig(configPath(dir));
  assert.strictEqual(cfg.documentation.auto_regenerate, false);
  fs.rmSync(dir, { recursive: true, force: true });
});

test('loadConfig reads documentation.auto_regenerate=true from yml', () => {
  const dir = makeTmpDir();
  writeYml(dir, [
    'documentation:',
    '  documentation:',
    '    auto_regenerate: true',
    '',
  ].join('\n'));
  const cfg = loadConfig(configPath(dir));
  assert.strictEqual(cfg.documentation.auto_regenerate, true);
  fs.rmSync(dir, { recursive: true, force: true });
});

test('loadConfig reads documentation.auto_regenerate=false explicitly from yml', () => {
  const dir = makeTmpDir();
  writeYml(dir, [
    'documentation:',
    '  documentation:',
    '    auto_regenerate: false',
    '',
  ].join('\n'));
  const cfg = loadConfig(configPath(dir));
  assert.strictEqual(cfg.documentation.auto_regenerate, false);
  fs.rmSync(dir, { recursive: true, force: true });
});

test('loadConfig returns documentation key alongside audiences/style/examples', () => {
  const dir = makeTmpDir();
  writeYml(dir, 'documentation:\n  audiences: []\n');
  const cfg = loadConfig(configPath(dir));
  assert.ok(typeof cfg.documentation === 'object', 'documentation key must be present');
  assert.ok('auto_regenerate' in cfg.documentation, 'auto_regenerate must be present');
  assert.ok(Array.isArray(cfg.audiences), 'audiences must be present');
  assert.ok(typeof cfg.style === 'object', 'style must be present');
  assert.ok(typeof cfg.examples === 'object', 'examples must be present');
  fs.rmSync(dir, { recursive: true, force: true });
});

test('init round-trip: auto_regenerate key survives write+read', () => {
  const dir = makeTmpDir();
  // Simulate init output — writes the key explicitly with a comment.
  writeYml(dir, [
    'documentation:',
    '  audiences:',
    '    - {name: user, path: docs/user, focus: "tasks, workflows, how to use features"}',
    '    - {name: developer, path: docs/developer, focus: "architecture, APIs, extending"}',
    '    - {name: admin, path: docs/admin, focus: "install, configure, operate, troubleshoot"}',
    '    - {name: general, path: docs/general, focus: "what it is, why it matters, plain language"}',
    '  style:',
    '    palette: light-blue',
    '  examples:',
    '    verify: true',
    '    sandbox: worktree',
    '  # Set to true or add "docs: generate" to a per-issue body to opt in.',
    '  documentation:',
    '    auto_regenerate: false',
    '',
  ].join('\n'));
  const cfg = loadConfig(configPath(dir));
  assert.strictEqual(cfg.documentation.auto_regenerate, false);
  assert.deepEqual(cfg.audiences.map(a => a.name), ['user', 'developer', 'admin', 'general']);
  assert.strictEqual(cfg.style.palette, 'light-blue');
  assert.strictEqual(cfg.examples.verify, true);
  fs.rmSync(dir, { recursive: true, force: true });
});
