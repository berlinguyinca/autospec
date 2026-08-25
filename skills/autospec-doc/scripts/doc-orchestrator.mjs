#!/usr/bin/env node
// doc-orchestrator.mjs — subcommand router for the /autospec-doc skill.
//
// This is the scaffold ROUTER STUB (issue #916). It parses the subcommand
// contract and dispatches to named no-op handlers. The generator logic
// (gen-audience-docs, verify-examples, doc-style, gen-llms-full) is filled in
// by downstream issues #917-#921; here every handler is a planning no-op that
// prints what it WOULD do.
//
// Subcommand contract (spec §D1):
//   /autospec-doc            incremental — scope docs affected since last gen
//   --full                   regenerate everything + completeness audit
//   --audit                  read-only completeness/drift report
//   --audience <name>        regenerate one audience only
//   init                     scaffold the documentation: config + starter scopes
//
// Exit codes:
//   0  handler dispatched
//   2  usage error, OR a non-`init` subcommand was invoked with no
//      `documentation:` config present (config is required to know what to
//      generate; `init` is the bootstrap that CREATES that config).

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { execSync } from 'node:child_process';

import { loadConfig, resolveFeatures, resolveCoverageOptions, DEFAULT_AUDIENCES, FOLDER_CONTRACT } from './doc-config.mjs';
import { scaffoldFeatures, writeScaffold } from './doc-scaffold.mjs';
import { writeLlmsFull, fillManifest } from './gen-llms-full.mjs';
import { generateAudienceDocs } from './gen-audience-docs.mjs';
import { auditCoverage } from './doc-coverage.mjs';

const __dirname = path.dirname(fileURLToPath(import.meta.url));

// ── Project root (generation output target) ───────────────────────────────────
// `__dirname` is inside the AUTOSPEC INSTALL repo — wrong for generation output.
// Generation must be rooted at the PROJECT root: the directory that CONTAINS the
// resolved CONFIG_PATH's `.autospec/` dir. For `<proj>/.autospec/autospec.yml`
// that is `<proj>` = dirname(dirname(resolve(CONFIG_PATH))). Defaults to cwd.
function projectRoot() {
  const resolved = path.resolve(CONFIG_PATH);
  const root = path.dirname(path.dirname(resolved));
  return root || process.cwd();
}

// ── Config detection ──────────────────────────────────────────────────────────
// The config loader (doc-config.mjs, #917) parses the `documentation:` block.
// Non-`init` subcommands require the block to exist; `init` is what creates it.
const CONFIG_PATH = process.env.AUTOSPEC_DOC_CONFIG
  || path.join('.autospec', 'autospec.yml');

function hasDocumentationConfig(configPath) {
  try {
    const raw = fs.readFileSync(configPath, 'utf8');
    // Match a top-level `documentation:` key (start of line, no indentation).
    return /^documentation:\s*$/m.test(raw) || /^documentation:\s*\S/m.test(raw);
  } catch {
    return false;
  }
}

// ── Config-backed handlers ────────────────────────────────────────────────────
// The generator logic (scope-diff, AI-review, verify) is filled in by downstream
// issues #918-#923. Here each non-init handler loads the real config via
// doc-config.mjs (#917) so the audience list it reports is the resolved one
// (declared entries verbatim, or the four seeded defaults).

function collectAudiencePages(cfg, repoRoot) {
  // Discover all generated .md pages from configured audience paths.
  // Returns Array<{ audience, feature, path, content }>.
  const pages = [];
  for (const aud of cfg.audiences) {
    const audName = aud.name || aud.id || 'unknown';
    const audPath = aud.path;
    if (!audPath) continue;
    const dir = path.resolve(repoRoot, audPath);
    if (!fs.existsSync(dir)) continue;
    let stat;
    try { stat = fs.statSync(dir); } catch { continue; }
    if (stat.isFile() && audPath.replace(/\/+$/, '').endsWith('.md')) {
      // Single-file audience (issue #2968): the configured path is the document
      // itself, not a directory to walk. Mirrors the renderer's `.md` suffix
      // contract — a non-.md file keeps the folder (misconfiguration) behavior.
      const relPath = path.relative(repoRoot, dir).replace(/\\/g, '/');
      let content;
      try { content = fs.readFileSync(dir, 'utf8'); } catch { continue; }
      const featureMatch = relPath.match(/\/(?:tutorials|features)\/([^/]+)\.md$/);
      const feature = featureMatch ? featureMatch[1] : null;
      pages.push({ audience: audName, feature, path: relPath, content });
      continue;
    }
    const walk = (d) => {
      let entries;
      try { entries = fs.readdirSync(d, { withFileTypes: true }); } catch { return; }
      for (const ent of entries) {
        const full = path.join(d, ent.name);
        if (ent.isDirectory()) { walk(full); continue; }
        if (!ent.name.endsWith('.md')) continue;
        const relPath = path.relative(repoRoot, full).replace(/\\/g, '/');
        let content;
        try { content = fs.readFileSync(full, 'utf8'); } catch { continue; }
        const featureMatch = relPath.match(/\/(?:tutorials|features)\/([^/]+)\.md$/);
        const feature = featureMatch ? featureMatch[1] : null;
        pages.push({ audience: audName, feature, path: relPath, content });
      }
    };
    walk(dir);
  }
  return pages;
}

async function regenerateLlmsFull(cfg, repoRoot) {
  const pages = collectAudiencePages(cfg, repoRoot);
  const outputPath = path.join(repoRoot, 'llms-full.txt');
  const result = await writeLlmsFull({ pages, outputPath });
  process.stderr.write(`[autospec-doc] llms-full.txt ${result.written ? 'written' : 'unchanged'} (${pages.length} pages)\n`);

  // Fill manifest if present.
  const manifestPath = path.join(repoRoot, '.llm-manifest.json');
  if (fs.existsSync(manifestPath)) {
    try {
      const manifest = JSON.parse(fs.readFileSync(manifestPath, 'utf8'));
      if (manifest && typeof manifest === 'object') {
        if (!Array.isArray(manifest.modules))          manifest.modules = [];
        if (!Array.isArray(manifest.concepts))         manifest.concepts = [];
        if (!Array.isArray(manifest.faq))              manifest.faq = [];
        if (!Array.isArray(manifest.cli_entry_points)) manifest.cli_entry_points = [];
        if (!Array.isArray(manifest.http_endpoints))   manifest.http_endpoints = [];
        fillManifest(manifest, pages);
        const newContent = JSON.stringify(manifest, null, 2) + '\n';
        const oldContent = fs.readFileSync(manifestPath, 'utf8');
        if (oldContent !== newContent) {
          fs.writeFileSync(manifestPath, newContent, 'utf8');
          process.stderr.write(`[autospec-doc] .llm-manifest.json updated\n`);
        }
      }
    } catch { /* manifest read/parse failure is non-fatal */ }
  }
}

// ── Incremental scope-set computation (§D6) ───────────────────────────────────
//
// §D6: the default (bare) subcommand is INCREMENTAL — it computes the set of
// changed scopes from `check-doc-drift.sh --working-tree` output, then
// regenerates only those scopes. Full fan-out stays under `--full`.
// Zero changed scopes → fast no-op (one log line, no generation work).

const CHECK_DRIFT_SH = path.resolve(__dirname, '../../../scripts/check-doc-drift.sh');

function reportWarnings(warnings) {
  for (const warning of warnings || []) {
    process.stderr.write(`[autospec-doc] warning: ${warning}\n`);
  }
}

async function regenerateLlmsFullSafely(cfg, repoRoot) {
  try {
    await regenerateLlmsFull(cfg, repoRoot);
  } catch (error) {
    process.stderr.write(`[autospec-doc] llms-full regen error: ${error.message}\n`);
  }
}

/**
 * Run check-doc-drift.sh --working-tree and parse the JSON gate output.
 * Returns the changed-scope set as an array of scope identifiers.
 * Returns [] when the script is unavailable or exits cleanly with no drift.
 */
function computeChangedScopes() {
  const script = process.env.AUTOSPEC_CHECK_DRIFT_SH || CHECK_DRIFT_SH;
  if (!fs.existsSync(script)) {
    process.stderr.write(`[autospec-doc] check-doc-drift.sh not found at ${script}; treating as zero changed scopes\n`);
    return [];
  }
  let raw = '';
  let exitCode = 0;
  try {
    raw = execSync(`bash ${JSON.stringify(script)} --working-tree`, {
      encoding: 'utf8',
      stdio: ['pipe', 'pipe', 'pipe'],
    });
  } catch (err) {
    exitCode = (err && err.status) || 1;
    raw = (err && err.stdout) || '';
  }
  // Exit 0 = clean (no drift); exit 1 = drift detected; exit 2 = missing-scope / error.
  // Parse stdout as JSON gate result (spec §3b); extract changed scope identifiers.
  let gate = null;
  try { gate = JSON.parse(raw); } catch { /* non-JSON output is fine — treat as no scopes */ }
  if (!gate || exitCode === 0) return []; // clean → nothing changed
  const scopes = [];
  if (gate && Array.isArray(gate.changed_scopes)) scopes.push(...gate.changed_scopes);
  else if (gate && gate.scope) scopes.push(gate.scope);
  return scopes;
}

async function handleIncremental(_opts) {
  const cfg = loadConfig(CONFIG_PATH);
  const names = cfg.audiences.map(a => a.name || a.id).join(', ');
  const projRoot = projectRoot();

  // §D6: compute changed-scope set from check-doc-drift.sh --working-tree.
  const changedScopes = computeChangedScopes();

  if (changedScopes.length === 0) {
    // Zero changed scopes → fast no-op.
    console.log(`[autospec-doc] incremental: no changed scopes detected — nothing to regenerate`);
    return 0;
  }

  console.log(`[autospec-doc] incremental: ${changedScopes.length} changed scope(s) detected for audiences [${names}]; regenerating affected scopes: ${changedScopes.join(', ')}`);
  // Regenerate llms-full.txt from any already-generated pages (cheap concat).
  await regenerateLlmsFullSafely(cfg, projRoot);
  return 0;
}

// ── Deterministic completeness audit (read-only) ──────────────────────────────
//
// Computes, from an in-memory generation pass + the on-disk page tree:
//   - features that have no feature page for one or more audiences (missing)
//   - generated pages that are empty / near-empty on disk
//   - audiences with zero generated pages on disk
//   - "identical prose across all audiences" warnings (from the generator)
// Returns a structured report object; never writes.
async function runAudit(cfg, projRoot, features) {
  const audiences = cfg.audiences;
  let warnings = [];
  try {
    const result = await generateAudienceDocs({
      features,
      audiences,
      outputDir: null,
      failOnIdenticalAudiences: false,
    });
    warnings = result.warnings || [];
  } catch (e) {
    warnings = [`generation error during audit: ${e.message}`];
  }

  const pages = collectAudiencePages(cfg, projRoot);
  const audienceNames = audiences.map(a => a.name || a.id);

  // Audiences with zero pages on disk.
  const pagesByAudience = new Map();
  for (const p of pages) {
    if (!pagesByAudience.has(p.audience)) pagesByAudience.set(p.audience, []);
    pagesByAudience.get(p.audience).push(p);
  }
  const emptyAudiences = audienceNames.filter(n => !(pagesByAudience.get(n) || []).length);

  // Missing feature pages: each feature should have docs/<aud>/features/<slug>.md.
  const missing = [];
  for (const aud of audiences) {
    const audName = aud.name || aud.id;
    if (!aud.path) continue;
    // Single-file audience (issue #2968): features fold into the one composed
    // document, so per-feature folder pages are not expected.
    if (aud.path.replace(/\/+$/, '').endsWith('.md')) continue;
    for (const f of features) {
      if (!f || !f.slug) continue;
      const rel = `${aud.path.replace(/\/+$/, '')}/features/${f.slug}.md`;
      const onDisk = path.resolve(projRoot, rel);
      if (!fs.existsSync(onDisk)) missing.push(`${audName}: ${rel}`);
    }
  }

  // Empty / near-empty generated pages on disk (< 40 non-whitespace chars).
  const nearEmpty = [];
  for (const p of pages) {
    const stripped = (p.content || '').replace(/\s+/g, '');
    if (stripped.length < 40) nearEmpty.push(p.path);
  }

  return {
    audiences: audienceNames,
    features: features.length,
    pagesOnDisk: pages.length,
    emptyAudiences,
    missing,
    nearEmpty,
    sameness: warnings,
  };
}

function printAuditReport(report) {
  console.log(
    `[autospec-doc] audit: ${report.audiences.length} audience(s), ${report.features} feature(s), `
    + `${report.pagesOnDisk} page(s) on disk — `
    + `${report.missing.length} missing page(s), ${report.nearEmpty.length} near-empty, `
    + `${report.emptyAudiences.length} audience(s) with zero pages, ${report.sameness.length} sameness warning(s)`,
  );
  for (const m of report.missing)        console.log(`  missing: ${m}`);
  for (const n of report.nearEmpty)      console.log(`  near-empty: ${n}`);
  for (const a of report.emptyAudiences) console.log(`  zero-pages: ${a}`);
  for (const w of report.sameness)       console.log(`  sameness: ${w}`);
}

// ── Answerability / domain-term coverage (doc-coverage.mjs) ────────────────────
//
// Mines the project's OWN vocabulary (SCREAMING_SNAKE enum constants + dotted
// config keys) and checks whether the generated audience docs cover it. Advisory
// only — never throws, never writes. Returns the coverage result (or null when
// disabled / on error).
function runCoverage(cfg, projRoot) {
  const opts = resolveCoverageOptions(cfg);
  if (!opts.enabled) return null;
  try {
    const pages = collectAudiencePages(cfg, projRoot);
    const res = auditCoverage({
      repoRoot: projRoot,
      pages,
      options: {
        minFreq:     opts.minFreq,
        minFiles:    opts.minFiles,
        sourceGlobs: opts.sourceGlobs || undefined,
        configGlobs: opts.configGlobs || undefined,
        stoplist:    opts.stoplist || [],
        excludeDirs: opts.excludeDirs || [],
        excludeGlobs: opts.excludeGlobs || [],
        excludeFiles: opts.excludeFiles || [],
        configPrefixStoplist: opts.configPrefixStoplist || [],
        useDefaultConfigPrefixStoplist: opts.useDefaultConfigPrefixStoplist,
      },
    });
    return { ...res, maxReport: opts.maxReport };
  } catch (e) {
    process.stderr.write(`[autospec-doc] coverage audit error: ${e.message}\n`);
    return null;
  }
}

// Full multi-line coverage section for --audit (and --full's audit pass).
function printCoverageReport(res) {
  if (!res) return;
  console.log(`  domain coverage: ${res.covered}/${res.total} terms (${res.coveragePct}%)`);
  const limit = res.maxReport != null ? res.maxReport : 15;
  for (const t of res.missing.slice(0, limit)) {
    const sample = (t.sampleFiles && t.sampleFiles[0]) ? ` e.g. ${t.sampleFiles[0]}` : '';
    console.log(`  missing ${t.kind}: ${t.term} (freq ${t.freq}, ${t.files} files)${sample}`);
  }
}

async function handleFull(_opts) {
  const cfg = loadConfig(CONFIG_PATH);
  const projRoot = projectRoot();
  const features = resolveFeatures(cfg, projRoot);
  const names = cfg.audiences.map(a => a.name || a.id).join(', ');

  const { files, warnings } = await generateAudienceDocs({
    features,
    audiences: cfg.audiences,
    outputDir: projRoot,
    failOnIdenticalAudiences: false,
  });
  const written = files.filter(f => f.written).length;

  console.log(
    `[autospec-doc] full: regenerated every audience [${names}] — `
    + `${written} page(s) written, ${features.length} feature(s), ${cfg.audiences.length} audience(s).`,
  );
  reportWarnings(warnings);

  // Regenerate llms-full.txt from the freshly written pages.
  await regenerateLlmsFullSafely(cfg, projRoot);

  // Run the deterministic completeness audit and print its summary.
  const report = await runAudit(cfg, projRoot, features);
  printAuditReport(report);

  // Advisory answerability/domain-term coverage summary (one line; never throws).
  const cov = runCoverage(cfg, projRoot);
  if (cov) {
    console.log(
      `[autospec-doc] domain coverage: ${cov.coveragePct}% (${cov.covered}/${cov.total}); `
      + `${cov.missing.length} high-signal terms missing`,
    );
  }
  return 0;
}

async function handleAudit(_opts) {
  // READ-ONLY: writes nothing.
  const cfg = loadConfig(CONFIG_PATH);
  const projRoot = projectRoot();
  const features = resolveFeatures(cfg, projRoot);
  const report = await runAudit(cfg, projRoot, features);
  printAuditReport(report);

  // Advisory answerability/domain-term coverage section (read-only; never writes).
  const cov = runCoverage(cfg, projRoot);
  printCoverageReport(cov);
  return 0;
}

async function handleAudience(opts) {
  if (!opts.audience) {
    console.error('[autospec-doc] --audience requires a <name> argument.');
    return 2;
  }
  const cfg = loadConfig(CONFIG_PATH);
  // Resolve against the configured audiences first, then fall back to the four
  // canonical defaults (user/developer/admin/general are always valid targets
  // even when the config declares a narrower explicit list).
  let match = cfg.audiences.find(a => (a.name || a.id) === opts.audience);
  if (!match) match = DEFAULT_AUDIENCES.find(a => a.name === opts.audience);
  if (!match) {
    const known = [...new Set([
      ...cfg.audiences.map(a => a.name || a.id),
      ...DEFAULT_AUDIENCES.map(a => a.name),
    ])].join(', ');
    console.error(
      `[autospec-doc] audience: "${opts.audience}" is not a known audience `
      + `(known: ${known || 'none'}).`,
    );
    return 1;
  }

  const projRoot = projectRoot();
  const features = resolveFeatures(cfg, projRoot);

  const { files, warnings } = await generateAudienceDocs({
    features,
    audiences: [match],
    outputDir: projRoot,
    failOnIdenticalAudiences: false,
  });
  const written = files.filter(f => f.written).length;

  console.log(
    `[autospec-doc] audience: regenerated "${opts.audience}" only — `
    + `${written} page(s) written, ${features.length} feature(s).`,
  );
  reportWarnings(warnings);

  await regenerateLlmsFullSafely(cfg, projRoot);
  return 0;
}

// ── init scaffolding (#917) ───────────────────────────────────────────────────
//
// init is the bootstrap. It writes a `documentation:` block to
// .autospec/autospec.yml (seeding the four default audiences) and creates the
// per-audience starter folder contract under docs/<audience>/. It is idempotent:
// an existing `documentation:` block is left untouched, and existing doc files
// are never overwritten. With --dry-run it prints the plan and writes nothing.

// Render the canonical `documentation:` YAML block from the default audiences
// and the style/examples defaults. Kept in lock-step with spec §D2 and
// doc-config.mjs's defaults.
function renderDocumentationBlock() {
  const lines = ['documentation:', '  audiences:'];
  for (const a of DEFAULT_AUDIENCES) {
    lines.push(`    - {name: ${a.name}, path: ${a.path}, focus: "${a.focus}"}`);
  }
  lines.push('  style:');
  lines.push('    palette: light-blue');
  lines.push('  examples:');
  lines.push('    verify: true');
  lines.push('    sandbox: worktree');
  return lines.join('\n') + '\n';
}

// Per-audience starter files per the folder contract (spec §D2): index.md +
// getting-started.md for every audience; developer also gets architecture/ and
// api/ directory placeholders.
function plannedScopeFiles() {
  const files = [];
  for (const a of DEFAULT_AUDIENCES) {
    for (const base of FOLDER_CONTRACT.baseFiles) {
      files.push(path.join(a.path, base));
    }
    if (a.name === 'developer') {
      for (const extra of FOLDER_CONTRACT.developerExtras) {
        // Directory placeholder — keep the tree present with a .gitkeep.
        files.push(path.join(a.path, extra, '.gitkeep'));
      }
    }
  }
  return files;
}

function starterContent(audience, relfile) {
  if (path.basename(relfile) === '.gitkeep') return '';
  const title = path.basename(relfile, '.md');
  return `<!-- autospec-doc-scope: audience=${audience.name} generated: false -->\n`
    + `# ${audience.name} — ${title}\n\n`
    + `_Starter scope for the **${audience.name}** audience (${audience.focus})._\n\n`
    + `Run \`/autospec-doc --audience ${audience.name}\` to generate this content.\n`;
}

function handleInit(opts) {
  // The "init: scaffold plan" line is part of the stable output contract.
  console.log('[autospec-doc] init: scaffold plan');

  const configExists = hasDocumentationConfig(CONFIG_PATH);
  const scopeFiles = plannedScopeFiles();

  if (configExists) {
    console.log(`  - documentation: block already present in ${CONFIG_PATH} (left untouched)`);
  } else {
    console.log(`  - write a \`documentation:\` block to ${CONFIG_PATH} (default audiences: user, developer, admin, general)`);
  }
  console.log('  - create starter doc scopes under docs/<audience>/ per the folder contract:');
  for (const f of scopeFiles) console.log(`      ${f}`);

  // The feature inventory itself is hand-authored today; init seeds a project-
  // grounded SKELETON so the authoring pass starts from a real feature list.
  const repoRoot = path.dirname(path.dirname(path.resolve(CONFIG_PATH)));
  const featuresPath = path.join(repoRoot, '.autospec', 'doc-features.json');
  const featuresExist = fs.existsSync(featuresPath);

  if (opts.dryRun) {
    if (featuresExist) {
      console.log('  - .autospec/doc-features.json already present (left untouched)');
    } else {
      const { features, source } = scaffoldFeatures({ repoRoot });
      console.log(`  - scaffold ${features.length} feature skeleton(s) into .autospec/doc-features.json (from ${source})`);
    }
    console.log('  (--dry-run: no files written)');
    return 0;
  }

  // 1. Write/extend the config block.
  if (!configExists) {
    const dir = path.dirname(CONFIG_PATH);
    if (dir && dir !== '.') fs.mkdirSync(dir, { recursive: true });
    const block = renderDocumentationBlock();
    if (fs.existsSync(CONFIG_PATH)) {
      const existing = fs.readFileSync(CONFIG_PATH, 'utf8');
      const sep = existing.endsWith('\n') || existing === '' ? '' : '\n';
      fs.writeFileSync(CONFIG_PATH, existing + sep + block, 'utf8');
    } else {
      fs.writeFileSync(CONFIG_PATH, block, 'utf8');
    }
    console.log(`  ✓ wrote documentation: block to ${CONFIG_PATH}`);
  }

  // 2. Create starter scopes (never overwrite human-owned files).
  const byPath = new Map(DEFAULT_AUDIENCES.map(a => [a.path, a]));
  let created = 0;
  for (const rel of scopeFiles) {
    if (fs.existsSync(rel)) continue;
    fs.mkdirSync(path.dirname(rel), { recursive: true });
    // Resolve which audience owns this file by longest matching path prefix.
    let owner = null;
    for (const [p, a] of byPath) {
      if (rel === p || rel.startsWith(p + path.sep)) { owner = a; break; }
    }
    fs.writeFileSync(rel, owner ? starterContent(owner, rel) : '', 'utf8');
    created++;
  }
  console.log(`  ✓ created ${created} starter doc file(s)`);

  // 3. Seed a project-grounded skeleton feature inventory (never clobber an
  //    existing/authored fixture).
  if (featuresExist) {
    console.log('  - .autospec/doc-features.json already present (left untouched)');
  } else {
    const audiences = DEFAULT_AUDIENCES.map(a => ({ name: a.name, path: a.path, focus: a.focus }));
    const res = writeScaffold({ repoRoot, audiences });
    if (res.written) {
      console.log(`  ✓ scaffolded ${res.features.length} feature(s) into .autospec/doc-features.json (from ${res.source})`);
      console.log('  Next: author per-audience prose for each feature in .autospec/doc-features.json,');
      console.log('        then run `/autospec-doc --full` and `/autospec-doc --audit` to find coverage gaps.');
    }
  }
  return 0;
}

// ── Argv parsing ──────────────────────────────────────────────────────────────

function parseArgs(argv) {
  // Returns { subcommand, audience } where subcommand is one of:
  //   incremental | full | audit | audience | init | usage
  const opts = { subcommand: 'incremental', audience: null, dryRun: false };
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    switch (arg) {
      case 'init':
        opts.subcommand = 'init';
        break;
      case '--dry-run':
        opts.dryRun = true;
        break;
      case '--full':
        opts.subcommand = 'full';
        break;
      case '--audit':
        opts.subcommand = 'audit';
        break;
      case '--audience': {
        const next = argv[i + 1];
        // The value must be a real name, not another flag or a missing arg.
        // `--audience --full` or a trailing `--audience` is a usage error.
        if (next === undefined || next.startsWith('-')) {
          console.error('[autospec-doc] --audience requires a <name> argument.');
          opts.subcommand = 'usage';
          return opts;
        }
        opts.subcommand = 'audience';
        opts.audience = next;
        i++; // consume the value
        break;
      }
      case '-h':
      case '--help':
        opts.subcommand = 'usage';
        break;
      default:
        console.error(`[autospec-doc] unknown argument: ${arg}`);
        opts.subcommand = 'usage';
        return opts;
    }
  }
  return opts;
}

function usage() {
  console.log('Usage: doc-orchestrator.mjs [--full | --audit | --audience <name> | init]');
  console.log('  (no subcommand)     incremental regeneration of affected scopes');
  console.log('  --full              regenerate everything + completeness audit');
  console.log('  --audit             read-only completeness/drift report');
  console.log('  --audience <name>   regenerate one audience only');
  console.log('  init                scaffold documentation: config + starter scopes');
  console.log('  --dry-run           (with init) print the scaffold plan; write nothing');
}

// ── Main ──────────────────────────────────────────────────────────────────────

async function main() {
  const opts = parseArgs(process.argv.slice(2));

  if (opts.subcommand === 'usage') {
    usage();
    return 2;
  }

  // Every subcommand except `init` requires a `documentation:` config: without
  // it there is nothing to generate. `init` is the bootstrap that creates it.
  if (opts.subcommand !== 'init' && !hasDocumentationConfig(CONFIG_PATH)) {
    console.error(
      `[autospec-doc] no \`documentation:\` config found at ${CONFIG_PATH}. `
      + 'Run `/autospec-doc init` first to scaffold it.',
    );
    return 2;
  }

  switch (opts.subcommand) {
    case 'init':        return handleInit(opts);
    case 'full':        return await handleFull(opts);
    case 'audit':       return await handleAudit(opts);
    case 'audience':    return await handleAudience(opts);
    case 'incremental': return await handleIncremental(opts);
    default:
      usage();
      return 2;
  }
}

main().then(code => process.exit(code)).catch(err => {
  process.stderr.write(`[autospec-doc] fatal: ${err && err.stack ? err.stack : err}\n`);
  process.exit(1);
});
