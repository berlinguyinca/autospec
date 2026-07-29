#!/usr/bin/env node
// Flat entry-point shim for the autospec-doc orchestrator.
//
// Installed as ${AUTOSPEC_SCRIPTS_DIR}/doc-orchestrator.mjs (the path skill
// surfaces invoke by convention). The real orchestrator and its ES-module
// closure (doc-config, doc-scaffold, gen-llms-full, gen-audience-docs,
// doc-coverage, doc-style) live in the two-level subtree
// ${AUTOSPEC_SCRIPTS_DIR}/../skills/autospec-doc/scripts/ — they MUST live there
// so gen-audience-docs.mjs's path.resolve(__dirname, '../../autospec-shared/scripts')
// resolves at runtime. A flat copy of the orchestrator can never satisfy that
// closure, so the flat entry delegates here, preserving argv, stdio, and exit code.
import { spawnSync } from 'node:child_process';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const real = path.resolve(here, '..', 'skills', 'autospec-doc', 'scripts', 'doc-orchestrator.mjs');
const result = spawnSync(process.execPath, [real, ...process.argv.slice(2)], { stdio: 'inherit' });
process.exit(result.status ?? 1);
