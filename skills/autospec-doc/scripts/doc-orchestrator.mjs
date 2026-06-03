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

// ── Config detection ──────────────────────────────────────────────────────────
// The real config loader lands in #917. For the scaffold we only need to answer
// one question: does a `documentation:` block exist in `.autospec/autospec.yml`?
// Non-`init` subcommands require it; `init` is what creates it.
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

// ── Handlers (no-op stubs; filled by downstream issues) ───────────────────────

function handleIncremental(_opts) {
  console.log('[autospec-doc] incremental: plan scopes affected since last generation (stub — #918/#919).');
  return 0;
}

function handleFull(_opts) {
  console.log('[autospec-doc] full: regenerate every audience + run completeness audit (stub — #918/#923).');
  return 0;
}

function handleAudit(_opts) {
  console.log('[autospec-doc] audit: read-only completeness/drift report (stub — #919/#923).');
  return 0;
}

function handleAudience(opts) {
  if (!opts.audience) {
    console.error('[autospec-doc] --audience requires a <name> argument.');
    return 2;
  }
  console.log(`[autospec-doc] audience: regenerate "${opts.audience}" only (stub — #918).`);
  return 0;
}

function handleInit(_opts) {
  // init is the bootstrap: it scans the repo and scaffolds the documentation:
  // config block + starter doc scopes. The real scaffolding logic lands in #917
  // (config schema + folder contract). The scaffold prints the plan.
  console.log('[autospec-doc] init: scaffold plan');
  console.log('  - scan the repo for features, entry points, and existing docs/');
  console.log('  - write a `documentation:` block to .autospec/autospec.yml (default audiences: user, developer, admin, general)');
  console.log('  - create starter doc scopes under docs/<audience>/ per the folder contract');
  console.log('  (scaffolding logic filled in by #917)');
  return 0;
}

// ── Argv parsing ──────────────────────────────────────────────────────────────

function parseArgs(argv) {
  // Returns { subcommand, audience } where subcommand is one of:
  //   incremental | full | audit | audience | init | usage
  const opts = { subcommand: 'incremental', audience: null };
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    switch (arg) {
      case 'init':
        opts.subcommand = 'init';
        break;
      case '--full':
        opts.subcommand = 'full';
        break;
      case '--audit':
        opts.subcommand = 'audit';
        break;
      case '--audience':
        opts.subcommand = 'audience';
        opts.audience = argv[i + 1] || null;
        i++; // consume the value
        break;
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
