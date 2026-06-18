// doc-coverage.test.mjs — unit tests for skills/autospec-doc/scripts/doc-coverage.mjs
//
// The doc-coverage module is a pure, deterministic, project-transparent
// "answerability" audit: it MINES the project's own vocabulary (SCREAMING_SNAKE
// enum constants + dotted config keys) and checks whether the generated audience
// docs cover those terms. No project-specific hardcoding — everything is derived
// from the repo under audit.
//
// Tests:
//   extractDomainTerms
//     1. finds INVALID_TARGET / CONFIRMED with correct freq & distinct-file count
//     2. drops stoplisted ALLCAPS noise (API) and sub-threshold terms
//     3. excludes terms found ONLY under docs/<audience>/ (generated docs)
//     4. excludes build/vendor dirs (node_modules, target, …)
//     5. extracts dotted config keys (depth ≥ 2) from YAML + .properties
//   scoreCoverage
//     6. marks a term covered when a page mentions it (case-insensitive)
//     7. coveragePct math + missing list ordering
//     8. empty terms → 100% / 0 total
//     9. config-key partial match (last 2 segments) counts as covered
//  auditCoverage
//    10. extract + score convenience wrapper

import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const COV_MOD = path.resolve(__dirname, '../scripts/doc-coverage.mjs');

const { extractDomainTerms, scoreCoverage, auditCoverage } = await import(COV_MOD);

// ── Fixture builder ─────────────────────────────────────────────────────────────
//
// Builds a throwaway repo tree:
//   src/Annotation.scala   — INVALID_TARGET (x3 across this + Target.scala), CONFIRMED, API noise
//   src/Target.scala       — INVALID_TARGET, RAW_ONLY_HERE (single file → sub-threshold on files)
//   src/Once.scala         — RARE_TERM (freq 1 → sub-threshold on freq)
//   application.yml        — nested dotted config keys
//   service.properties     — a.b.c=… config key
//   docs/user/features/x.md — DOCONLY_TERM (must be excluded: lives under generated docs)
//   node_modules/junk.js    — VENDOR_TERM (must be excluded: build/vendor dir)

function makeFixture() {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'doc-coverage-test-'));
  const w = (rel, content) => {
    const full = path.join(root, rel);
    fs.mkdirSync(path.dirname(full), { recursive: true });
    fs.writeFileSync(full, content, 'utf8');
  };

  w('src/Annotation.scala', [
    'object Annotation {',
    '  val a = AnnotationTargetType.INVALID_TARGET',
    '  val b = INVALID_TARGET',
    '  val c = CONFIRMED_HIT',
    '  val d = CONFIRMED_HIT',
    '  val e = CONFIRMED_HIT',
    '  // talks to the API and more API noise API',
    '}',
  ].join('\n'));

  w('src/Target.scala', [
    'object Target {',
    '  val x = INVALID_TARGET',
    '  val y = RAW_ONLY_HERE',
    '  val z = RAW_ONLY_HERE',
    '  val w = RAW_ONLY_HERE',
    '}',
  ].join('\n'));

  w('src/Once.scala', 'object Once { val r = RARE_TERM }\n');

  w('application.yml', [
    'wcmc:',
    '  workflow:',
    '    targets:',
    '      invalidate:',
    '        name: invalidate-targets',
    '        enabled: true',
    '',
  ].join('\n'));

  w('service.properties', 'wcmc.service.port=8080\nlonely=1\n');

  // Generated doc — its terms must NOT be mined as a source.
  w('docs/user/features/x.md', 'This file mentions DOCONLY_TERM DOCONLY_TERM DOCONLY_TERM.\n');

  // Vendor/build dir — must be excluded.
  w('node_modules/junk.js', 'const a = VENDOR_TERM; const b = VENDOR_TERM; const c = VENDOR_TERM;\n');

  return root;
}

function cleanup(root) {
  fs.rmSync(root, { recursive: true, force: true });
}

// ── extractDomainTerms ──────────────────────────────────────────────────────────

test('extractDomainTerms finds INVALID_TARGET and CONFIRMED with correct freq/files', () => {
  const root = makeFixture();
  const { terms } = extractDomainTerms({ repoRoot: root, minFreq: 3, minFiles: 2 });
  const byTerm = new Map(terms.map(t => [t.term, t]));

  const inv = byTerm.get('INVALID_TARGET');
  assert.ok(inv, 'INVALID_TARGET should be extracted');
  assert.strictEqual(inv.kind, 'enum');
  assert.strictEqual(inv.freq, 3, 'INVALID_TARGET appears 3 times total');
  assert.strictEqual(inv.files, 2, 'INVALID_TARGET appears in 2 distinct files');
  assert.ok(Array.isArray(inv.sampleFiles) && inv.sampleFiles.length >= 1);

  // CONFIRMED_HIT is freq 3 but only 1 file → dropped at minFiles=2 …
  assert.ok(!byTerm.has('CONFIRMED_HIT'), 'CONFIRMED_HIT (1 file) dropped at minFiles=2');
  cleanup(root);
});

test('extractDomainTerms with minFiles=1 surfaces CONFIRMED_HIT (freq 3, 1 file)', () => {
  const root = makeFixture();
  const { terms } = extractDomainTerms({ repoRoot: root, minFreq: 3, minFiles: 1 });
  const byTerm = new Map(terms.map(t => [t.term, t]));
  const conf = byTerm.get('CONFIRMED_HIT');
  assert.ok(conf, 'CONFIRMED_HIT should be extracted at minFiles=1');
  assert.strictEqual(conf.freq, 3);
  assert.strictEqual(conf.files, 1);
  cleanup(root);
});

test('extractDomainTerms drops stoplisted noise and sub-threshold terms', () => {
  const root = makeFixture();
  const { terms } = extractDomainTerms({ repoRoot: root, minFreq: 3, minFiles: 2 });
  const names = new Set(terms.map(t => t.term));

  // API is single-segment ALLCAPS in the stoplist AND too short / not snake — dropped.
  assert.ok(!names.has('API'), 'API must be dropped (stoplist)');
  // RARE_TERM has freq 1 → below minFreq.
  assert.ok(!names.has('RARE_TERM'), 'RARE_TERM below minFreq must be dropped');
  cleanup(root);
});

test('extractDomainTerms drops framework/stdlib enum constants by default', () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'doc-coverage-fw-'));
  const w = (rel, c) => { const f = path.join(root, rel); fs.mkdirSync(path.dirname(f), { recursive: true }); fs.writeFileSync(f, c, 'utf8'); };
  // Two files each so they pass minFiles=2; mix framework noise with a domain enum.
  const noise = 'HALF_EVEN SCOPE_PROTOTYPE REQUIRES_NEW UTF_8 APPLICATION_JSON ROUNDING_MODE ISO_ZONED_DATE_TIME';
  w('a/A.scala', `object A { val n = "${noise}"; val d = INVALID_TARGET }`);
  w('b/B.scala', `object B { val n = "${noise}"; val d = INVALID_TARGET }`);
  const { terms } = extractDomainTerms({ repoRoot: root, minFreq: 2, minFiles: 2 });
  const names = new Set(terms.map(t => t.term));
  for (const fw of ['HALF_EVEN', 'SCOPE_PROTOTYPE', 'REQUIRES_NEW', 'UTF_8', 'APPLICATION_JSON', 'ROUNDING_MODE', 'ISO_ZONED_DATE_TIME']) {
    assert.ok(!names.has(fw), `${fw} (framework/stdlib) must be stoplisted`);
  }
  assert.ok(names.has('INVALID_TARGET'), 'real domain enum must survive');
  fs.rmSync(root, { recursive: true, force: true });
});

test('extractDomainTerms with minFiles=1 still drops freq-1 terms', () => {
  const root = makeFixture();
  const { terms } = extractDomainTerms({ repoRoot: root, minFreq: 3, minFiles: 1 });
  const byTerm = new Map(terms.map(t => [t.term, t]));
  // RAW_ONLY_HERE: freq 3 in 1 file → now passes both thresholds.
  const raw = byTerm.get('RAW_ONLY_HERE');
  assert.ok(raw, 'RAW_ONLY_HERE should pass with minFiles=1');
  assert.strictEqual(raw.files, 1);
  // RARE_TERM still freq 1 → dropped.
  assert.ok(!byTerm.has('RARE_TERM'));
  cleanup(root);
});

test('extractDomainTerms excludes terms found only under docs/<audience>/', () => {
  const root = makeFixture();
  const { terms } = extractDomainTerms({ repoRoot: root, minFreq: 3, minFiles: 1 });
  const names = new Set(terms.map(t => t.term));
  assert.ok(!names.has('DOCONLY_TERM'), 'generated-doc-only terms must be excluded');
  cleanup(root);
});

test('extractDomainTerms excludes vendor/build dirs (node_modules)', () => {
  const root = makeFixture();
  const { terms } = extractDomainTerms({ repoRoot: root, minFreq: 3, minFiles: 1 });
  const names = new Set(terms.map(t => t.term));
  assert.ok(!names.has('VENDOR_TERM'), 'node_modules terms must be excluded');
  cleanup(root);
});

test('extractDomainTerms extracts dotted config keys (depth >= 2) from yaml + properties', () => {
  const root = makeFixture();
  const { terms } = extractDomainTerms({ repoRoot: root, minFreq: 1, minFiles: 1 });
  const configTerms = terms.filter(t => t.kind === 'config').map(t => t.term);
  // YAML nesting reconstructed into dotted path.
  assert.ok(configTerms.includes('wcmc.workflow.targets.invalidate.name'),
    `expected reconstructed yaml key, got: ${configTerms.join(', ')}`);
  // .properties left-hand side.
  assert.ok(configTerms.includes('wcmc.service.port'),
    `expected properties key, got: ${configTerms.join(', ')}`);
  // depth-1 keys dropped.
  assert.ok(!configTerms.includes('lonely'), 'depth-1 keys must be dropped');
  cleanup(root);
});

test('extractDomainTerms excludes vendored python venv / site-packages, keeps real source', () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'doc-coverage-venv-'));
  const w = (rel, content) => {
    const full = path.join(root, rel);
    fs.mkdirSync(path.dirname(full), { recursive: true });
    fs.writeFileSync(full, content, 'utf8');
  };
  // Vendored Python virtualenv tree — must be pruned at the dir level.
  w('proj/venv/lib/python3.13/site-packages/foo.py',
    'TYPE_CHECKING = 1\nx = TYPE_CHECKING\ny = TYPE_CHECKING\n');
  // Real project source — INVALID_TARGET must survive.
  w('proj/src/Foo.scala', [
    'object Foo {',
    '  val a = INVALID_TARGET',
    '  val b = INVALID_TARGET',
    '  val c = INVALID_TARGET',
    '}',
  ].join('\n'));
  const { terms } = extractDomainTerms({ repoRoot: root, minFreq: 3, minFiles: 1 });
  const names = new Set(terms.map(t => t.term));
  assert.ok(!names.has('TYPE_CHECKING'), 'venv/site-packages terms must be excluded');
  assert.ok(names.has('INVALID_TARGET'), 'real project source term must survive');
  fs.rmSync(root, { recursive: true, force: true });
});

test('extractDomainTerms skips hashed/content-addressed JS bundles', () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'doc-coverage-hash-'));
  const w = (rel, content) => {
    const full = path.join(root, rel);
    fs.mkdirSync(path.dirname(full), { recursive: true });
    fs.writeFileSync(full, content, 'utf8');
  };
  w('app.deadbeef12.js', 'const a = SOME_ENUM; const b = SOME_ENUM; const c = SOME_ENUM;\n');
  const { terms } = extractDomainTerms({ repoRoot: root, minFreq: 3, minFiles: 1 });
  const names = new Set(terms.map(t => t.term));
  assert.ok(!names.has('SOME_ENUM'), 'hashed-bundle terms must be skipped');
  fs.rmSync(root, { recursive: true, force: true });
});

test('extractDomainTerms honours excludeDirs (custom dir name pruned)', () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'doc-coverage-exdir-'));
  const w = (rel, content) => {
    const full = path.join(root, rel);
    fs.mkdirSync(path.dirname(full), { recursive: true });
    fs.writeFileSync(full, content, 'utf8');
  };
  w('thirdparty/lib.scala', 'val a = THIRD_PARTY; val b = THIRD_PARTY; val c = THIRD_PARTY\n');
  w('src/Main.scala', 'val a = KEEP_THIS; val b = KEEP_THIS; val c = KEEP_THIS\n');

  const without = extractDomainTerms({ repoRoot: root, minFreq: 3, minFiles: 1 });
  assert.ok(new Set(without.terms.map(t => t.term)).has('THIRD_PARTY'),
    'baseline: THIRD_PARTY present without excludeDirs');

  const { terms } = extractDomainTerms({
    repoRoot: root, minFreq: 3, minFiles: 1, excludeDirs: ['thirdparty'],
  });
  const names = new Set(terms.map(t => t.term));
  assert.ok(!names.has('THIRD_PARTY'), 'excludeDirs must prune the custom dir');
  assert.ok(names.has('KEEP_THIS'), 'other source still mined');
  fs.rmSync(root, { recursive: true, force: true });
});

test('extractDomainTerms honours excludeGlobs (per-file path glob pruned)', () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'doc-coverage-exglob-'));
  const w = (rel, content) => {
    const full = path.join(root, rel);
    fs.mkdirSync(path.dirname(full), { recursive: true });
    fs.writeFileSync(full, content, 'utf8');
  };
  w('fixtures/sample.scala', 'val a = FIXTURE_ENUM; val b = FIXTURE_ENUM; val c = FIXTURE_ENUM\n');
  w('src/Main.scala', 'val a = REAL_ENUM; val b = REAL_ENUM; val c = REAL_ENUM\n');

  const { terms } = extractDomainTerms({
    repoRoot: root, minFreq: 3, minFiles: 1, excludeGlobs: ['fixtures/**'],
  });
  const names = new Set(terms.map(t => t.term));
  assert.ok(!names.has('FIXTURE_ENUM'), 'excludeGlobs must prune matching files');
  assert.ok(names.has('REAL_ENUM'), 'non-matching source still mined');
  fs.rmSync(root, { recursive: true, force: true });
});

test('extractDomainTerms excludes build-generated git.properties by basename', () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'doc-coverage-gitprops-'));
  const w = (rel, content) => {
    const full = path.join(root, rel);
    fs.mkdirSync(path.dirname(full), { recursive: true });
    fs.writeFileSync(full, content, 'utf8');
  };
  // git.properties lives at a non-excluded path but is build-generated noise.
  w('classes/git.properties', 'git.commit.id=abc\ngit.branch=main\ngit.build.time=now\n');
  w('src/app.properties', 'myapp.feature.flag=true\n');
  const { terms } = extractDomainTerms({ repoRoot: root, minFreq: 1, minFiles: 1 });
  const cfgKeys = terms.filter(t => t.kind === 'config').map(t => t.term);
  assert.ok(!cfgKeys.some(k => k.startsWith('git.')),
    `git.properties keys must be excluded, got: ${cfgKeys.join(', ')}`);
  assert.ok(cfgKeys.includes('myapp.feature.flag'), 'real project key must survive');
  fs.rmSync(root, { recursive: true, force: true });
});

test('extractDomainTerms drops enum-stoplist builtin MAX_VALUE while INVALID_TARGET survives', () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'doc-coverage-builtin-'));
  const w = (rel, content) => {
    const full = path.join(root, rel);
    fs.mkdirSync(path.dirname(full), { recursive: true });
    fs.writeFileSync(full, content, 'utf8');
  };
  w('src/A.scala', [
    'object A {',
    '  val a = Long.MAX_VALUE',
    '  val b = Long.MAX_VALUE',
    '  val c = Long.MAX_VALUE',
    '  val d = INVALID_TARGET',
    '  val e = INVALID_TARGET',
    '  val f = INVALID_TARGET',
    '}',
  ].join('\n'));
  const { terms } = extractDomainTerms({ repoRoot: root, minFreq: 3, minFiles: 1 });
  const names = new Set(terms.map(t => t.term));
  assert.ok(!names.has('MAX_VALUE'), 'MAX_VALUE language builtin must be dropped');
  assert.ok(names.has('INVALID_TARGET'), 'real domain enum must survive');
  fs.rmSync(root, { recursive: true, force: true });
});

test('extractDomainTerms drops framework config prefix (spring.*) but keeps project namespace', () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'doc-coverage-cfgprefix-'));
  const w = (rel, content) => {
    const full = path.join(root, rel);
    fs.mkdirSync(path.dirname(full), { recursive: true });
    fs.writeFileSync(full, content, 'utf8');
  };
  w('app.properties', [
    'spring.datasource.url=jdbc:h2:mem',
    'myapp.feature.flag=true',
  ].join('\n'));
  const { terms } = extractDomainTerms({ repoRoot: root, minFreq: 1, minFiles: 1 });
  const cfgKeys = new Set(terms.filter(t => t.kind === 'config').map(t => t.term));
  assert.ok(!cfgKeys.has('spring.datasource.url'), 'spring.* must be dropped by prefix stoplist');
  assert.ok(cfgKeys.has('myapp.feature.flag'), 'project-namespaced key must survive');
  fs.rmSync(root, { recursive: true, force: true });
});

test('extractDomainTerms honours caller-added configPrefixStoplist (merged with defaults)', () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'doc-coverage-cfgadd-'));
  const w = (rel, content) => {
    const full = path.join(root, rel);
    fs.mkdirSync(path.dirname(full), { recursive: true });
    fs.writeFileSync(full, content, 'utf8');
  };
  w('app.properties', [
    'spring.datasource.url=jdbc:h2:mem',   // default-dropped
    'acme.internal.thing=1',               // caller-dropped
    'myapp.feature.flag=true',             // kept
  ].join('\n'));
  const { terms } = extractDomainTerms({
    repoRoot: root, minFreq: 1, minFiles: 1, configPrefixStoplist: ['acme'],
  });
  const cfgKeys = new Set(terms.filter(t => t.kind === 'config').map(t => t.term));
  assert.ok(!cfgKeys.has('spring.datasource.url'), 'default prefix still applied');
  assert.ok(!cfgKeys.has('acme.internal.thing'), 'caller-added prefix dropped');
  assert.ok(cfgKeys.has('myapp.feature.flag'), 'project key survives');
  fs.rmSync(root, { recursive: true, force: true });
});

test('extractDomainTerms useDefaultConfigPrefixStoplist:false keeps spring.*', () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'doc-coverage-cfgoptout-'));
  const w = (rel, content) => {
    const full = path.join(root, rel);
    fs.mkdirSync(path.dirname(full), { recursive: true });
    fs.writeFileSync(full, content, 'utf8');
  };
  w('app.properties', 'spring.datasource.url=jdbc:h2:mem\n');
  const { terms } = extractDomainTerms({
    repoRoot: root, minFreq: 1, minFiles: 1, useDefaultConfigPrefixStoplist: false,
  });
  const cfgKeys = new Set(terms.filter(t => t.kind === 'config').map(t => t.term));
  assert.ok(cfgKeys.has('spring.datasource.url'),
    'opting out of default prefix stoplist must keep spring.*');
  fs.rmSync(root, { recursive: true, force: true });
});

test('extractDomainTerms honours caller-added excludeFiles basenames', () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'doc-coverage-exfiles-'));
  const w = (rel, content) => {
    const full = path.join(root, rel);
    fs.mkdirSync(path.dirname(full), { recursive: true });
    fs.writeFileSync(full, content, 'utf8');
  };
  w('generated.properties', 'gen.key.one=1\n');
  w('real.properties', 'myapp.feature.flag=true\n');
  const { terms } = extractDomainTerms({
    repoRoot: root, minFreq: 1, minFiles: 1, excludeFiles: ['generated.properties'],
  });
  const cfgKeys = new Set(terms.filter(t => t.kind === 'config').map(t => t.term));
  assert.ok(!cfgKeys.has('gen.key.one'), 'excludeFiles basename must be pruned');
  assert.ok(cfgKeys.has('myapp.feature.flag'), 'other config still mined');
  fs.rmSync(root, { recursive: true, force: true });
});

// ── scoreCoverage ────────────────────────────────────────────────────────────────

test('scoreCoverage marks a term covered when a page mentions it (case-insensitive)', () => {
  const terms = [
    { term: 'INVALID_TARGET', kind: 'enum', freq: 5, files: 3, sampleFiles: ['src/A.scala'] },
    { term: 'CONFIRMED', kind: 'enum', freq: 4, files: 2, sampleFiles: ['src/B.scala'] },
  ];
  const pages = [
    { path: 'docs/user/x.md', content: 'A compound can be marked invalid_target by the system.' },
  ];
  const res = scoreCoverage({ terms, pages });
  assert.strictEqual(res.total, 2);
  assert.strictEqual(res.covered, 1);
  assert.strictEqual(res.missing.length, 1);
  assert.strictEqual(res.missing[0].term, 'CONFIRMED');
  assert.strictEqual(res.coveragePct, 50);
});

test('scoreCoverage missing list is sorted by salience desc and preserves metadata', () => {
  const terms = [
    { term: 'LOW_SIGNAL', kind: 'enum', freq: 3, files: 2, sampleFiles: ['a'] },
    { term: 'HIGH_SIGNAL', kind: 'enum', freq: 99, files: 9, sampleFiles: ['b'] },
  ];
  const res = scoreCoverage({ terms, pages: [] });
  assert.strictEqual(res.covered, 0);
  assert.strictEqual(res.coveragePct, 0);
  assert.strictEqual(res.missing[0].term, 'HIGH_SIGNAL', 'highest salience first');
  assert.strictEqual(res.missing[0].freq, 99);
  assert.deepEqual(res.missing[1].sampleFiles, ['a']);
});

test('scoreCoverage empty terms → 100% covered and 0 total', () => {
  const res = scoreCoverage({ terms: [], pages: [{ path: 'p', content: 'x' }] });
  assert.strictEqual(res.total, 0);
  assert.strictEqual(res.covered, 0);
  assert.strictEqual(res.coveragePct, 100);
  assert.deepEqual(res.missing, []);
});

test('scoreCoverage config-key partial match (last 2 segments) counts as covered', () => {
  const terms = [
    { term: 'wcmc.workflow.targets.invalidate.name', kind: 'config', freq: 1, files: 1, sampleFiles: ['application.yml'] },
  ];
  // Page mentions only the last two segments, not the full dotted key.
  const pages = [{ path: 'docs/admin/config.md', content: 'Set invalidate.name to control the target.' }];
  const res = scoreCoverage({ terms, pages });
  assert.strictEqual(res.covered, 1, 'last-2-segment match should count');
  assert.strictEqual(res.missing.length, 0);
});

test('scoreCoverage config prefix-alias match: same trailing path, different root prefix', () => {
  // A key documented under one root prefix covers the prefix-aliased duplicate
  // (last-3 segments shared), even though the suffix is preceded by a dot in the
  // doc. A key with a DIFFERENT trailing path is NOT covered.
  const terms = [
    { term: 'wcmc.workflow.algorithm.preprocessing.deconvolution.msdial4.massSliceWidth', kind: 'config', freq: 5, files: 5, sampleFiles: ['a.yml'] },
    { term: 'wcmc.workflow.algorithm.preprocessing.deconvolution.msdial.massSliceWidth', kind: 'config', freq: 5, files: 5, sampleFiles: ['b.yml'] },
  ];
  // Doc documents only the `lcms` root prefix, msdial4 variant.
  const pages = [{ path: 'docs/admin/peak.md', content: 'Tune `wcmc.workflow.lcms.preprocessing.deconvolution.msdial4.massSliceWidth` (default 0.1).' }];
  const res = scoreCoverage({ terms, pages });
  const missing = new Set(res.missing.map(m => m.term));
  assert.ok(!missing.has('wcmc.workflow.algorithm.preprocessing.deconvolution.msdial4.massSliceWidth'), 'algorithm-prefix alias (same last-3) must be covered');
  assert.ok(missing.has('wcmc.workflow.algorithm.preprocessing.deconvolution.msdial.massSliceWidth'), 'msdial v3 (different last-3) must NOT be covered by the msdial4 key');
});

test('scoreCoverage enum match is whole-token (does not match substring of a longer word)', () => {
  const terms = [{ term: 'DONE', kind: 'enum', freq: 5, files: 3, sampleFiles: ['a'] }];
  const pages = [{ path: 'p', content: 'This is abandoned and undone work.' }];
  const res = scoreCoverage({ terms, pages });
  assert.strictEqual(res.covered, 0, 'DONE must not match inside "undone"/"abandoned"');
});

// ── auditCoverage ────────────────────────────────────────────────────────────────

test('auditCoverage extracts then scores in one call', () => {
  const root = makeFixture();
  const pages = [
    { path: 'docs/user/features/x.md', content: 'Compounds become INVALID_TARGET or CONFIRMED_HIT.' },
  ];
  const res = auditCoverage({ repoRoot: root, pages, options: { minFreq: 3, minFiles: 1 } });
  assert.ok(res.total >= 2);
  assert.ok(Array.isArray(res.terms));
  assert.ok(res.covered >= 2, 'both enum terms mentioned in the page are covered');
  assert.ok(typeof res.coveragePct === 'number');
  cleanup(root);
});
