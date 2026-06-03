// doc-config.test.mjs — unit tests for skills/autospec-doc/scripts/doc-config.mjs (issue #917)
//
// Tests:
//   1. style.palette defaults to 'light-blue' when absent
//   2. examples.verify defaults to true when absent
//   3. examples.sandbox defaults to 'worktree' when absent
//   4. default audiences seeded only when audiences list is empty
//   5. existing audience entries preserved verbatim (name/path/focus/require_scope)
//   6. existing audience entries preserved verbatim (id-keyed legacy entries)
//   7. all four default audience names are user/developer/admin/general
//   8. default audience paths match spec §D2 exactly
//   9. default audience focus strings match spec §D2 exactly
//  10. folder-contract constants exported and match §D2 exactly
//  11. loadConfig returns structured object with audiences/style/examples
//  12. explicit style/examples values are honoured over defaults

import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const CONFIG_MOD = path.resolve(__dirname, '../scripts/doc-config.mjs');

const { loadConfig, FOLDER_CONTRACT, DEFAULT_AUDIENCES } = await import(CONFIG_MOD);

// ── Helpers ───────────────────────────────────────────────────────────────────

function makeTmpDir() {
  return fs.mkdtempSync(path.join(os.tmpdir(), 'doc-config-test-'));
}

function writeYml(dir, content) {
  fs.mkdirSync(path.join(dir, '.autospec'), { recursive: true });
  fs.writeFileSync(path.join(dir, '.autospec', 'autospec.yml'), content, 'utf8');
}

function configPath(dir) {
  return path.join(dir, '.autospec', 'autospec.yml');
}

// ── Tests ─────────────────────────────────────────────────────────────────────

test('style.palette defaults to light-blue when absent', () => {
  const dir = makeTmpDir();
  writeYml(dir, 'documentation:\n  audiences:\n    - name: user\n      path: docs/user\n      focus: "tasks"\n      require_scope: true\n');
  const cfg = loadConfig(configPath(dir));
  assert.strictEqual(cfg.style.palette, 'light-blue');
  fs.rmSync(dir, { recursive: true, force: true });
});

test('examples.verify defaults to true when absent', () => {
  const dir = makeTmpDir();
  writeYml(dir, 'documentation:\n  audiences:\n    - name: user\n      path: docs/user\n      focus: "tasks"\n      require_scope: true\n');
  const cfg = loadConfig(configPath(dir));
  assert.strictEqual(cfg.examples.verify, true);
  fs.rmSync(dir, { recursive: true, force: true });
});

test('examples.sandbox defaults to worktree when absent', () => {
  const dir = makeTmpDir();
  writeYml(dir, 'documentation:\n  audiences:\n    - name: user\n      path: docs/user\n      focus: "tasks"\n      require_scope: true\n');
  const cfg = loadConfig(configPath(dir));
  assert.strictEqual(cfg.examples.sandbox, 'worktree');
  fs.rmSync(dir, { recursive: true, force: true });
});

test('default audiences seeded only when audiences list is empty', () => {
  const dir = makeTmpDir();
  writeYml(dir, 'documentation:\n  audiences: []\n');
  const cfg = loadConfig(configPath(dir));
  assert.strictEqual(cfg.audiences.length, 4);
  assert.deepEqual(cfg.audiences.map(a => a.name), ['user', 'developer', 'admin', 'general']);
  fs.rmSync(dir, { recursive: true, force: true });
});

test('default audiences NOT seeded when audiences list is non-empty', () => {
  const dir = makeTmpDir();
  writeYml(dir, 'documentation:\n  audiences:\n    - name: user\n      path: docs/user\n      focus: "tasks"\n      require_scope: true\n');
  const cfg = loadConfig(configPath(dir));
  assert.strictEqual(cfg.audiences.length, 1);
  assert.strictEqual(cfg.audiences[0].name, 'user');
  fs.rmSync(dir, { recursive: true, force: true });
});

test('default audiences seeded when audiences key is absent entirely', () => {
  const dir = makeTmpDir();
  writeYml(dir, 'documentation:\n  style:\n    palette: light-blue\n');
  const cfg = loadConfig(configPath(dir));
  assert.strictEqual(cfg.audiences.length, 4);
  fs.rmSync(dir, { recursive: true, force: true });
});

test('existing audience name/path/focus/require_scope preserved verbatim', () => {
  const dir = makeTmpDir();
  writeYml(dir, [
    'documentation:',
    '  audiences:',
    '    - name: user',
    '      path: docs/user',
    '      focus: "tasks, workflows, how to use features"',
    '      require_scope: true',
    '    - name: developer',
    '      path: docs/developer',
    '      focus: "architecture, APIs, extending"',
    '      require_scope: false',
    '',
  ].join('\n'));
  const cfg = loadConfig(configPath(dir));
  assert.strictEqual(cfg.audiences[0].name, 'user');
  assert.strictEqual(cfg.audiences[0].path, 'docs/user');
  assert.strictEqual(cfg.audiences[0].focus, 'tasks, workflows, how to use features');
  assert.strictEqual(cfg.audiences[0].require_scope, true);
  assert.strictEqual(cfg.audiences[1].name, 'developer');
  assert.strictEqual(cfg.audiences[1].require_scope, false);
  fs.rmSync(dir, { recursive: true, force: true });
});

test('legacy id-keyed audiences preserved verbatim (not overwritten)', () => {
  const dir = makeTmpDir();
  writeYml(dir, [
    'documentation:',
    '  audiences:',
    '    - id: users',
    '      label: End users',
    '      path: docs/USER_MANUAL.md',
    '      focus: "Installation and workflows"',
    '      require_scope: true',
    '',
  ].join('\n'));
  const cfg = loadConfig(configPath(dir));
  // One entry — not replaced with defaults (list is non-empty).
  assert.strictEqual(cfg.audiences.length, 1);
  assert.strictEqual(cfg.audiences[0].id, 'users');
  assert.strictEqual(cfg.audiences[0].path, 'docs/USER_MANUAL.md');
  fs.rmSync(dir, { recursive: true, force: true });
});

test('all four default audience names are user/developer/admin/general', () => {
  assert.deepEqual(DEFAULT_AUDIENCES.map(a => a.name), ['user', 'developer', 'admin', 'general']);
});

test('default audience paths match spec §D2 exactly', () => {
  const paths = DEFAULT_AUDIENCES.reduce((acc, a) => { acc[a.name] = a.path; return acc; }, {});
  assert.strictEqual(paths.user,      'docs/user');
  assert.strictEqual(paths.developer, 'docs/developer');
  assert.strictEqual(paths.admin,     'docs/admin');
  assert.strictEqual(paths.general,   'docs/general');
});

test('default audience focus strings match spec §D2 exactly', () => {
  const focus = DEFAULT_AUDIENCES.reduce((acc, a) => { acc[a.name] = a.focus; return acc; }, {});
  assert.strictEqual(focus.user,      'tasks, workflows, how to use features');
  assert.strictEqual(focus.developer, 'architecture, APIs, extending');
  assert.strictEqual(focus.admin,     'install, configure, operate, troubleshoot');
  assert.strictEqual(focus.general,   'what it is, why it matters, plain language');
});

test('folder-contract constants exported and match spec §D2', () => {
  assert.ok(Array.isArray(FOLDER_CONTRACT.baseFiles), 'FOLDER_CONTRACT.baseFiles must be an array');
  assert.ok(FOLDER_CONTRACT.baseFiles.includes('index.md'));
  assert.ok(FOLDER_CONTRACT.baseFiles.includes('getting-started.md'));
  // Tutorial and feature patterns
  assert.ok(typeof FOLDER_CONTRACT.tutorialPattern === 'string');
  assert.ok(FOLDER_CONTRACT.tutorialPattern.includes('tutorials/'));
  assert.ok(typeof FOLDER_CONTRACT.featurePattern === 'string');
  assert.ok(FOLDER_CONTRACT.featurePattern.includes('features/'));
  // Developer-specific extras
  assert.ok(Array.isArray(FOLDER_CONTRACT.developerExtras));
  assert.ok(FOLDER_CONTRACT.developerExtras.includes('architecture/'));
  assert.ok(FOLDER_CONTRACT.developerExtras.includes('api/'));
  // Admin link
  assert.strictEqual(FOLDER_CONTRACT.adminRunbooksLink, 'docs/runbooks/');
  // Shared assets
  assert.ok(FOLDER_CONTRACT.sharedAssets.includes('docs/assets/screenshots'));
  assert.ok(FOLDER_CONTRACT.sharedAssets.includes('docs/assets/diagrams'));
  assert.ok(FOLDER_CONTRACT.sharedAssets.includes('docs/assets/transcripts'));
});

test('loadConfig returns structured object with audiences/style/examples', () => {
  const dir = makeTmpDir();
  writeYml(dir, 'documentation:\n  audiences: []\n');
  const cfg = loadConfig(configPath(dir));
  assert.ok(typeof cfg === 'object');
  assert.ok(Array.isArray(cfg.audiences));
  assert.ok(typeof cfg.style === 'object');
  assert.ok(typeof cfg.examples === 'object');
  fs.rmSync(dir, { recursive: true, force: true });
});

test('inline-mapping audience entries (init output form) parse to all four defaults', () => {
  // The init scaffolder writes audiences as inline flow mappings:
  //   - {name: user, path: docs/user, focus: "..."}
  // loadConfig must parse these back to the canonical four with exact strings.
  const dir = makeTmpDir();
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
    '',
  ].join('\n'));
  const cfg = loadConfig(configPath(dir));
  assert.deepEqual(cfg.audiences.map(a => a.name), ['user', 'developer', 'admin', 'general']);
  assert.strictEqual(cfg.audiences[0].path, 'docs/user');
  assert.strictEqual(cfg.audiences[1].focus, 'architecture, APIs, extending');
  assert.strictEqual(cfg.style.palette, 'light-blue');
  assert.strictEqual(cfg.examples.verify, true);
  assert.strictEqual(cfg.examples.sandbox, 'worktree');
  fs.rmSync(dir, { recursive: true, force: true });
});

test('explicit style.palette and examples values are honoured over defaults', () => {
  const dir = makeTmpDir();
  writeYml(dir, [
    'documentation:',
    '  audiences: []',
    '  style:',
    '    palette: dark-blue',
    '  examples:',
    '    verify: false',
    '    sandbox: docker',
    '',
  ].join('\n'));
  const cfg = loadConfig(configPath(dir));
  assert.strictEqual(cfg.style.palette, 'dark-blue');
  assert.strictEqual(cfg.examples.verify, false);
  assert.strictEqual(cfg.examples.sandbox, 'docker');
  fs.rmSync(dir, { recursive: true, force: true });
});
