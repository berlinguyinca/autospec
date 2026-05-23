#!/usr/bin/env node
// packages/cli/bin/autospec.js — @autospec/cli top-level dispatcher
// Routes subcommands to scripts/<cmd>.sh in the same package directory.
// Keep under 120 lines; no framework dependencies.

import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';
import { readFileSync } from 'node:fs';

const __dirname = dirname(fileURLToPath(import.meta.url));
const SCRIPTS_DIR = join(__dirname, '..', 'scripts');

// Read version from package.json
function getVersion() {
  const pkgPath = join(__dirname, '..', 'package.json');
  const pkg = JSON.parse(readFileSync(pkgPath, 'utf8'));
  return pkg.version;
}

function usage() {
  console.log(`autospec v${getVersion()}

Usage: autospec <subcommand> [options]

Subcommands:
  init        Bootstrap a target repo (.autospec/test.yml + initial scopes)
  install     Install autospec skills and scripts into your harness
  status      List installed skills, versions, and cache-hit-rate
  upgrade     Fetch latest autospec and reinstall skills
  uninstall   Remove autospec skills from harness paths (preserves ~/.autospec/)
  --version   Print version
  --help      Show this help

Examples:
  autospec init
  autospec install
  autospec install --dry-run
  autospec status
  autospec upgrade --dry-run
  autospec uninstall --yes
`);
}

function runScript(name, args) {
  const scriptPath = join(SCRIPTS_DIR, `${name}.sh`);
  try {
    execFileSync('bash', [scriptPath, ...args], {
      stdio: 'inherit',
      env: { ...process.env },
    });
  } catch (err) {
    process.exit(err.status ?? 1);
  }
}

const [, , cmd, ...rest] = process.argv;

switch (cmd) {
  case 'init':
    runScript('init', rest);
    break;
  case 'install':
    runScript('install', rest);
    break;
  case 'status':
    runScript('status', rest);
    break;
  case 'upgrade':
    runScript('upgrade', rest);
    break;
  case 'uninstall':
    runScript('uninstall', rest);
    break;
  case '--version':
  case '-v':
    console.log(getVersion());
    break;
  case '--help':
  case '-h':
  case undefined:
    usage();
    break;
  default:
    console.error(`autospec: unknown subcommand '${cmd}'`);
    console.error("Run 'autospec --help' for usage.");
    process.exit(1);
}
