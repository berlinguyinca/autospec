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

import { loadConfig, DEFAULT_AUDIENCES, FOLDER_CONTRACT } from './doc-config.mjs';
import { writeLlmsFull, fillManifest } from './gen-llms-full.mjs';

const __dirname = path.dirname(fileURLToPath(import.meta.url));

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

function handleIncremental(_opts) {
  const cfg = loadConfig(CONFIG_PATH);
  const names = cfg.audiences.map(a => a.name || a.id).join(', ');
  const repoRoot = path.resolve(__dirname, '../../..');

  // §D6: compute changed-scope set from check-doc-drift.sh --working-tree.
  const changedScopes = computeChangedScopes();

  if (changedScopes.length === 0) {
    // Zero changed scopes → fast no-op.
    console.log(`[autospec-doc] incremental: no changed scopes detected — nothing to regenerate`);
    return 0;
  }

  console.log(`[autospec-doc] incremental: ${changedScopes.length} changed scope(s) detected for audiences [${names}]; regenerating affected scopes: ${changedScopes.join(', ')}`);
  // Regenerate llms-full.txt from any already-generated pages (cheap concat).
  regenerateLlmsFull(cfg, repoRoot).catch(e => process.stderr.write(`[autospec-doc] llms-full regen error: ${e.message}\n`));
  return 0;
}

function handleFull(_opts) {
  const cfg = loadConfig(CONFIG_PATH);
  const names = cfg.audiences.map(a => a.name || a.id).join(', ');
  console.log(`[autospec-doc] full: regenerate every audience [${names}] + run completeness audit (generation stub — #918/#923).`);
  // Regenerate llms-full.txt from any already-generated pages (cheap concat).
  const repoRoot = path.resolve(__dirname, '../../..');
  regenerateLlmsFull(cfg, repoRoot).catch(e => process.stderr.write(`[autospec-doc] llms-full regen error: ${e.message}\n`));
  return 0;
}

function handleAudit(_opts) {
  const cfg = loadConfig(CONFIG_PATH);
  console.log(`[autospec-doc] audit: read-only completeness/drift report across ${cfg.audiences.length} audiences (generation stub — #919/#923).`);
  return 0;
}

function handleAudience(opts) {
  if (!opts.audience) {
    console.error('[autospec-doc] --audience requires a <name> argument.');
    return 2;
  }
  // The router dispatches by form; audience-membership resolution (and the
  // unknown-audience error) is the generator's concern (#918). Here we load the
  // config only to surface whether the requested name is already configured.
  const cfg = loadConfig(CONFIG_PATH);
  const known = cfg.audiences.some(a => (a.name || a.id) === opts.audience);
  const note = known ? '' : ' [not in current config — generator will resolve]';
  console.log(`[autospec-doc] audience: regenerate "${opts.audience}" only${note} (generation stub — #918).`);
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

  if (opts.dryRun) {
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

function main() {
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
    case 'full':        return handleFull(opts);
    case 'audit':       return handleAudit(opts);
    case 'audience':    return handleAudience(opts);
    case 'incremental': return handleIncremental(opts);
    default:
      usage();
      return 2;
  }
}

process.exit(main());
