#!/usr/bin/env node
// doc-coverage.mjs — project-transparent answerability / domain-term coverage audit.
//
// Problem: autospec-doc generates per-audience docs from a feature fixture. If the
// fixture omits a domain concept, the generated docs silently can't answer questions
// about it (e.g. an `INVALID_TARGET` enum value or a `StateOfSample` lifecycle that
// users ask about but no page mentions).
//
// This module MINES the project's OWN vocabulary — with NO project-specific
// hardcoding — and checks whether the generated docs cover it:
//
//   - enum   — SCREAMING_SNAKE_CASE identifiers (≥2 segments): enum constants,
//              case objects, state names users ask about.
//   - config — dotted lowercase config keys (depth ≥2) from YAML / .properties / .conf.
//
// Pure, deterministic, no network. Exports:
//   extractDomainTerms({ repoRoot, sourceGlobs, configGlobs, minFreq, minFiles, stoplist, maxTerms, excludeDirs, excludeGlobs, excludeFiles, configPrefixStoplist, useDefaultConfigPrefixStoplist })
//   scoreCoverage({ terms, pages })
//   auditCoverage({ repoRoot, pages, options })

import fs from 'node:fs';
import path from 'node:path';

// ── Defaults ──────────────────────────────────────────────────────────────────

// Common code extensions for enum mining.
const DEFAULT_SOURCE_GLOBS = [
  '**/*.{scala,java,kt,ts,tsx,js,jsx,py,go,rs,rb,cs,swift}',
];

// Common config files for dotted-key mining.
const DEFAULT_CONFIG_GLOBS = [
  '**/application*.{yml,yaml}',
  '**/*.properties',
  '**/*.conf',
];

// Directories we never walk into: build artifacts, vcs, package caches, our own
// state dir, and the generated audience doc trees (we must NOT mine the docs we
// are auditing as a source of terms).
const EXCLUDED_DIR_NAMES = new Set([
  // Existing build / vcs / state dirs.
  'node_modules', '.git', 'target', 'build', 'dist', 'out', '.autospec',
  // Python virtualenvs / vendored interpreters / caches.
  'venv', '.venv', 'env', '.env', 'virtualenv', 'site-packages', '__pycache__',
  '.tox', '.mypy_cache', '.pytest_cache', '.ruff_cache', 'eggs', '.eggs',
  // JVM / build-tool caches.
  '.gradle', '.m2',
  // Generic vendored / package dirs.
  'vendor', 'bower_components', 'Pods', '.cargo', '.pnpm-store', '.yarn',
  // Framework build/output dirs.
  '.next', '.nuxt', '.svelte-kit', 'coverage', '.nyc_output',
  // IDE / infra.
  '.idea', '.vscode', '.terraform',
]);

// Generated audience doc roots (relative path prefixes) — excluded as term sources.
const EXCLUDED_DOC_PREFIXES = [
  'docs/user', 'docs/developer', 'docs/admin', 'docs/general',
];

// Built-in stoplist of generic ALLCAPS noise. Matched against the WHOLE term, so
// `INVALID_TARGET` survives while a bare `API` is dropped.
const DEFAULT_STOPLIST = [
  'TODO', 'FIXME', 'NOTE', 'XXX', 'HACK',
  'HTTP', 'HTTPS', 'JSON', 'XML', 'HTML', 'YAML', 'CSV', 'TSV',
  'URL', 'URI', 'UUID', 'UTF', 'ASCII', 'API', 'SDK', 'CLI',
  'CPU', 'GPU', 'RAM', 'OK', 'GET', 'PUT', 'POST', 'HEAD', 'PATCH',
  'ID', 'IDS', 'MD5', 'SHA', 'SPDX', 'TRUE', 'FALSE', 'NULL',
  'AND', 'OR', 'NOT', 'FOR', 'THE', 'MIT', 'BSD', 'GPL',
  'EOF', 'OS', 'IO', 'DB', 'SQL', 'NaN',
  // Language / stdlib numeric & sentinel builtins (framework-generic). These are
  // multi-segment SCREAMING_SNAKE so they pass the enum regex, but they are pure
  // language noise (e.g. Long.MAX_VALUE, Number.MAX_SAFE_INTEGER). Whole-term
  // matched, so real domain enums like INVALID_TARGET are unaffected.
  'MAX_VALUE', 'MIN_VALUE', 'POSITIVE_INFINITY', 'NEGATIVE_INFINITY',
  'MAX_SAFE_INTEGER', 'MIN_SAFE_INTEGER', 'NOT_FOUND', 'NO_ERROR',
  // Framework / stdlib enum CONSTANTS (multi-segment, so they pass the enum
  // regex, but they are library vocabulary — not the project's domain). These
  // recur in any JVM/Spring/Jackson codebase. Whole-term matched, so real
  // domain enums are unaffected; projects can extend via `coverage.stoplist`.
  // java.math.RoundingMode
  'HALF_EVEN', 'HALF_UP', 'HALF_DOWN', 'ROUNDING_MODE', 'UNNECESSARY',
  // java.nio.charset.StandardCharsets
  'UTF_8', 'UTF_16', 'UTF_16BE', 'UTF_16LE', 'US_ASCII', 'ISO_8859_1',
  // Spring bean scopes / tx propagation / ordering
  'SCOPE_PROTOTYPE', 'SCOPE_SINGLETON', 'REQUIRES_NEW', 'PROPAGATION_REQUIRES_NEW',
  'PROPAGATION_REQUIRED', 'LOWEST_PRECEDENCE', 'HIGHEST_PRECEDENCE', 'ASSIGNABLE_TYPE',
  // Jackson (de)serialization features
  'FAIL_ON_UNKNOWN_PROPERTIES', 'FAIL_ON_EMPTY_BEANS', 'WRITE_DATES_AS_TIMESTAMPS',
  // Spring/HTTP media types
  'APPLICATION_JSON', 'APPLICATION_XML', 'TEXT_PLAIN', 'MULTIPART_FORM_DATA',
  // java.time formatters / SQL / NIO options
  'ISO_ZONED_DATE_TIME', 'ISO_LOCAL_DATE_TIME', 'ISO_INSTANT', 'ISO_DATE_TIME',
  'ISO_LOCAL_DATE', 'CURRENT_TIMESTAMP', 'CURRENT_DATE', 'REPLACE_EXISTING', 'COPY_ATTRIBUTES',
  // HTTP statuses / AWS regions / misc stdlib + library constants.
  'BAD_GATEWAY', 'NOT_FOUND', 'BAD_REQUEST', 'BUFFER_SIZE', 'CASE_INSENSITIVE',
  'NO_COMPRESSION', 'OBJECT_MAPPER',
  // javax.xml.stream.XMLStreamConstants
  'START_ELEMENT', 'END_ELEMENT', 'START_DOCUMENT', 'END_DOCUMENT', 'CHARACTERS',
];

// Build-generated info files replicated per module: pure noise (git.properties is
// emitted by git-commit-id plugins into every JVM module, etc.). Skipped by
// basename, alongside the vendor-file regex. Overridable/extensible via the
// `excludeFiles` option.
const EXCLUDED_FILE_NAMES = new Set([
  'git.properties', 'build-info.properties', 'pom.properties',
  'MANIFEST.MF', 'git.json',
]);

// Generic framework / infra FIRST-segment config-key prefixes. A `config` term
// whose first dotted segment (case-insensitive) is in this set is framework noise
// (spring.datasource.url, git.commit.id, …) and dropped — while the project's own
// namespaced keys (wcmc.workflow.*, myapp.*) survive. Framework-generic, NOT
// project-specific. Extensible/overridable via the `configPrefixStoplist` option.
const CONFIG_PREFIX_STOPLIST = [
  'spring', 'git', 'management', 'server', 'logging', 'springdoc', 'springfox',
  'eureka', 'feign', 'ribbon', 'hystrix', 'kafka', 'redis', 'rabbitmq',
  'hibernate', 'flyway', 'liquibase', 'jackson', 'quarkus', 'micronaut',
  'camel', 'aws', 'azure', 'gcp', 'java', 'sun', 'user', 'os', 'line', 'file',
  'path', 'maven', 'gradle', 'log4j', 'logback', 'slf4j', 'tomcat', 'jetty',
  'netty', 'grpc', 'otel', 'opentelemetry', 'sentry', 'datadog',
  // datasource proxy / SQL logging decorators (datasource-proxy, p6spy).
  'decorator', 'p6spy',
];

// Regex: SCREAMING_SNAKE_CASE with at least two segments.
const ENUM_RE = /\b[A-Z][A-Z0-9]*(?:_[A-Z0-9]+)+\b/g;

// Obvious vendored / minified / content-addressed files to skip:
//   - *.min.js / *.min.css / *-min.js / *-min.css   (minified)
//   - *.bundle.js / *.bundle.css                     (bundled)
//   - name.<hex8+>.js|css|mjs                         (hashed/content-addressed)
//   - *.map                                          (source maps)
//   - *.lock / *-lock.json                           (lockfiles)
const VENDOR_FILE_RE = /(?:\.min\.(?:js|css)|-min\.(?:js|css)|\.bundle\.(?:js|css)|\.[0-9a-f]{8,}\.(?:js|css|mjs)|\.map|\.lock|-lock\.json)$/i;

// ── Glob → RegExp (small, dependency-free) ──────────────────────────────────────
//
// Supports the subset used by the default globs: `**`, `*`, and `{a,b,c}` brace
// alternation. Matching is done against POSIX-style relative paths.
function globToRegExp(glob) {
  let re = '';
  for (let i = 0; i < glob.length; i++) {
    const c = glob[i];
    if (c === '*') {
      if (glob[i + 1] === '*') {
        // `**` → any path segment(s) including '/'
        re += '.*';
        i++;
        if (glob[i + 1] === '/') i++; // swallow the slash after **
      } else {
        re += '[^/]*';
      }
    } else if (c === '{') {
      const end = glob.indexOf('}', i);
      if (end === -1) { re += '\\{'; continue; }
      const alts = glob.slice(i + 1, end).split(',').map(a =>
        a.replace(/[.+^${}()|[\]\\]/g, '\\$&'));
      re += '(?:' + alts.join('|') + ')';
      i = end;
    } else if (c === '.') {
      re += '\\.';
    } else if ('+^$()|[]\\'.includes(c)) {
      re += '\\' + c;
    } else {
      re += c;
    }
  }
  return new RegExp('^' + re + '$');
}

function compileGlobs(globs) {
  return globs.map(globToRegExp);
}

function matchesAny(relPath, regexps) {
  return regexps.some(re => re.test(relPath));
}

// ── File walker ─────────────────────────────────────────────────────────────────
//
// Walk the tree from repoRoot, yielding POSIX-relative paths, honoring the
// directory exclusions. Generated-doc-prefix and vendor filtering happen at the
// caller (per-file) so they can be precise about full relative paths.
// `extraExcludedDirs` (a Set of additional dir names) is merged with the
// built-in EXCLUDED_DIR_NAMES so projects can prune custom vendored trees.
function walkFiles(repoRoot, extraExcludedDirs = null) {
  const out = [];
  const stack = [repoRoot];
  while (stack.length) {
    const dir = stack.pop();
    let entries;
    try { entries = fs.readdirSync(dir, { withFileTypes: true }); } catch { continue; }
    for (const ent of entries) {
      const full = path.join(dir, ent.name);
      if (ent.isDirectory()) {
        if (EXCLUDED_DIR_NAMES.has(ent.name)) continue;
        if (extraExcludedDirs && extraExcludedDirs.has(ent.name)) continue;
        stack.push(full);
      } else if (ent.isFile()) {
        out.push(full);
      }
    }
  }
  return out;
}

function toRel(repoRoot, full) {
  return path.relative(repoRoot, full).split(path.sep).join('/');
}

function isExcludedDocPath(relPath) {
  return EXCLUDED_DOC_PREFIXES.some(p => relPath === p || relPath.startsWith(p + '/'));
}

// ── Config-key extraction ───────────────────────────────────────────────────────

// YAML: reconstruct dotted paths from indentation nesting. Only leaf keys
// (a key whose value is a scalar) produce a dotted path. Depth ≥2 kept.
function extractYamlKeys(content) {
  const keys = [];
  const lines = content.split('\n');
  // Stack of { indent, key }.
  const stack = [];
  for (const rawLine of lines) {
    const line = rawLine.replace(/\t/g, '  ');
    if (line.trim() === '' || line.trim().startsWith('#')) continue;
    if (line.trim().startsWith('- ')) continue; // skip sequence items
    const m = line.match(/^(\s*)([A-Za-z0-9_.-]+)\s*:(.*)$/);
    if (!m) continue;
    const indent = m[1].length;
    const key = m[2];
    const rest = m[3].trim();

    // Pop stack entries at >= current indent.
    while (stack.length && stack[stack.length - 1].indent >= indent) stack.pop();
    stack.push({ indent, key });

    const hasScalarValue = rest !== '' && !rest.startsWith('#');
    if (hasScalarValue) {
      const dotted = stack.map(s => s.key).join('.');
      keys.push(dotted);
    }
  }
  return keys;
}

// .properties / .conf: take the left side of `a.b.c=value` (or `a.b.c: value`).
function extractPropertiesKeys(content) {
  const keys = [];
  for (const rawLine of content.split('\n')) {
    const line = rawLine.trim();
    if (line === '' || line.startsWith('#') || line.startsWith('!') || line.startsWith('//')) continue;
    const m = line.match(/^([A-Za-z0-9_.-]+)\s*[:=]/);
    if (!m) continue;
    keys.push(m[1]);
  }
  return keys;
}

function configKeysForFile(relPath, content) {
  if (/\.(ya?ml)$/i.test(relPath)) return extractYamlKeys(content);
  return extractPropertiesKeys(content);
}

// Keep only dotted keys with depth ≥2.
function isDeepKey(key) {
  return key.split('.').filter(Boolean).length >= 2;
}

// ── Term accumulation ───────────────────────────────────────────────────────────

function makeAccumulator() {
  // term → { kind, freq, files:Set, sampleFiles:[] }
  return new Map();
}

function record(acc, term, kind, relPath) {
  let entry = acc.get(term);
  if (!entry) {
    entry = { kind, freq: 0, files: new Set(), sampleFiles: [] };
    acc.set(term, entry);
  }
  entry.freq += 1;
  if (!entry.files.has(relPath)) {
    entry.files.add(relPath);
    if (entry.sampleFiles.length < 5) entry.sampleFiles.push(relPath);
  }
}

// Salience: freq weighted by distinct-file spread.
function salience(t) {
  return t.freq * Math.log((t.files || (t.fileSet ? t.fileSet.size : 0)) + 1);
}

/**
 * extractDomainTerms — mine enum constants and config keys from the project.
 *
 * @returns {{ terms: Array<{term,kind,freq,files,sampleFiles}> }}
 */
export function extractDomainTerms({
  repoRoot,
  sourceGlobs = DEFAULT_SOURCE_GLOBS,
  configGlobs = DEFAULT_CONFIG_GLOBS,
  minFreq = 3,
  minFiles = 2,
  stoplist = [],
  maxTerms = 200,
  excludeDirs = [],
  excludeGlobs = [],
  excludeFiles = [],
  configPrefixStoplist = [],
  useDefaultConfigPrefixStoplist = true,
} = {}) {
  if (!repoRoot) return { terms: [] };

  const sourceRes = compileGlobs(sourceGlobs);
  const configRes = compileGlobs(configGlobs);
  const stop = new Set([...DEFAULT_STOPLIST, ...stoplist].map(s => String(s)));

  // Extra directory names (merged with the built-in EXCLUDED_DIR_NAMES) and
  // extra per-file path globs (matched like VENDOR_FILE_RE) for project overrides.
  const extraExcludedDirs = new Set(
    (Array.isArray(excludeDirs) ? excludeDirs : []).map(s => String(s)));
  const excludeGlobRes = compileGlobs(
    (Array.isArray(excludeGlobs) ? excludeGlobs : []).map(s => String(s)));

  // Excluded basenames: built-in build-generated info files PLUS caller additions.
  const excludedFileNames = new Set([
    ...EXCLUDED_FILE_NAMES,
    ...(Array.isArray(excludeFiles) ? excludeFiles : []).map(s => String(s)),
  ]);

  // Config-key first-segment stoplist: defaults (unless opted out) MERGED with
  // caller additions. Lower-cased for case-insensitive first-segment matching.
  const configPrefixStop = new Set([
    ...(useDefaultConfigPrefixStoplist ? CONFIG_PREFIX_STOPLIST : []),
    ...(Array.isArray(configPrefixStoplist) ? configPrefixStoplist : []).map(s => String(s)),
  ].map(s => s.toLowerCase()));

  const acc = makeAccumulator();
  const files = walkFiles(repoRoot, extraExcludedDirs);

  for (const full of files) {
    const relPath = toRel(repoRoot, full);
    if (isExcludedDocPath(relPath)) continue;       // never mine generated docs
    if (VENDOR_FILE_RE.test(relPath)) continue;     // skip min/bundle files
    if (excludedFileNames.has(path.basename(relPath))) continue; // build-generated info files
    if (excludeGlobRes.length && matchesAny(relPath, excludeGlobRes)) continue; // project override

    const isSource = matchesAny(relPath, sourceRes);
    const isConfig = matchesAny(relPath, configRes);
    if (!isSource && !isConfig) continue;

    let content;
    try { content = fs.readFileSync(full, 'utf8'); } catch { continue; }

    if (isSource) {
      let m;
      ENUM_RE.lastIndex = 0;
      while ((m = ENUM_RE.exec(content)) !== null) {
        record(acc, m[0], 'enum', relPath);
      }
    }
    if (isConfig) {
      for (const key of configKeysForFile(relPath, content)) {
        if (isDeepKey(key)) record(acc, key, 'config', relPath);
      }
    }
  }

  // Materialize, apply stoplist + thresholds.
  const terms = [];
  for (const [term, entry] of acc) {
    if (term.length < 4) continue;
    if (entry.kind === 'enum') {
      // Whole-term stoplist match (drops bare API; keeps INVALID_TARGET).
      if (stop.has(term)) continue;
      // Defensive: the regex requires ≥2 segments; drop any single-segment item.
      if (!term.includes('_')) continue;
    }
    if (entry.kind === 'config') {
      // Drop framework/infra keys by FIRST dotted segment (case-insensitive),
      // keeping the project's own namespaced keys (wcmc.workflow.*, myapp.*).
      const firstSeg = String(term).split('.')[0].toLowerCase();
      if (configPrefixStop.has(firstSeg)) continue;
    }
    const fileCount = entry.files.size;
    if (entry.freq < minFreq) continue;
    if (fileCount < minFiles) continue;
    terms.push({
      term,
      kind: entry.kind,
      freq: entry.freq,
      files: fileCount,
      sampleFiles: entry.sampleFiles.slice(0, 5),
    });
  }

  terms.sort((a, b) => {
    const sa = a.freq * Math.log(a.files + 1);
    const sb = b.freq * Math.log(b.files + 1);
    if (sb !== sa) return sb - sa;
    if (b.freq !== a.freq) return b.freq - a.freq;
    if (b.files !== a.files) return b.files - a.files;
    return a.term < b.term ? -1 : a.term > b.term ? 1 : 0;
  });

  return { terms: terms.slice(0, maxTerms) };
}

// ── Coverage scoring ────────────────────────────────────────────────────────────

function escapeRe(s) {
  return s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

// A term is covered if it appears in any page content via a whole-token,
// case-insensitive match. For config keys, the full dotted key, a key sharing
// the same trailing path (last 3 segments — covers prefix-aliased duplicates
// like `…lcms.X` vs `…algorithm.X`), or the standalone last-2-segment leaf all
// suffice.
function termCovered(term, haystack) {
  if (term.kind === 'config') {
    const segs = String(term.term).split('.').filter(Boolean);
    // candidate := { text, suffix }. suffix=true relaxes the leading lookbehind
    // to allow a preceding dot, so a documented key with a different ROOT prefix
    // but the same trailing segments still counts as covering this term.
    const candidates = [{ text: term.term, suffix: false }];
    if (segs.length >= 3) candidates.push({ text: segs.slice(-3).join('.'), suffix: true });
    if (segs.length >= 2) candidates.push({ text: segs.slice(-2).join('.'), suffix: false });
    return candidates.some(({ text, suffix }) => {
      const behind = suffix ? '(?<!\\w)' : '(?<![\\w.])';
      const re = new RegExp(behind + escapeRe(text) + '(?![\\w.])', 'i');
      return re.test(haystack);
    });
  }
  // enum — whole-token match (word boundaries, case-insensitive).
  const re = new RegExp('\\b' + escapeRe(term.term) + '\\b', 'i');
  return re.test(haystack);
}

/**
 * scoreCoverage — score how many mined terms the generated pages cover.
 *
 * @returns {{ total, covered, missing, coveragePct }}
 */
export function scoreCoverage({ terms = [], pages = [] } = {}) {
  const haystack = (pages || []).map(p => (p && p.content) || '').join('\n');
  const total = terms.length;
  const missing = [];
  let covered = 0;

  for (const t of terms) {
    if (termCovered(t, haystack)) covered += 1;
    else missing.push(t);
  }

  missing.sort((a, b) => {
    const sa = a.freq * Math.log((a.files || 0) + 1);
    const sb = b.freq * Math.log((b.files || 0) + 1);
    if (sb !== sa) return sb - sa;
    if (b.freq !== a.freq) return b.freq - a.freq;
    return (b.files || 0) - (a.files || 0);
  });

  const coveragePct = total === 0 ? 100 : Math.round((covered / total) * 100);
  return { total, covered, missing, coveragePct };
}

/**
 * auditCoverage — extract then score in one call.
 *
 * @returns {{ total, covered, missing, coveragePct, terms }}
 */
export function auditCoverage({ repoRoot, pages = [], options = {} } = {}) {
  const { terms } = extractDomainTerms({
    repoRoot,
    sourceGlobs: options.sourceGlobs || options.source_globs,
    configGlobs: options.configGlobs || options.config_globs,
    minFreq: options.minFreq != null ? options.minFreq : options.min_freq,
    minFiles: options.minFiles != null ? options.minFiles : options.min_files,
    stoplist: options.stoplist || [],
    maxTerms: options.maxTerms != null ? options.maxTerms : options.max_terms,
    excludeDirs: options.excludeDirs || options.exclude_dirs || [],
    excludeGlobs: options.excludeGlobs || options.exclude_globs || [],
    excludeFiles: options.excludeFiles || options.exclude_files || [],
    configPrefixStoplist: options.configPrefixStoplist || options.config_prefix_stoplist || [],
    useDefaultConfigPrefixStoplist:
      (options.useDefaultConfigPrefixStoplist != null) ? options.useDefaultConfigPrefixStoplist
      : (options.use_default_config_prefix_stoplist != null) ? options.use_default_config_prefix_stoplist
      : true,
  });
  const score = scoreCoverage({ terms, pages });
  return { ...score, terms };
}
