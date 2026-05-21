#!/usr/bin/env node
// quarantine.mjs — Mode II consecutive-violation tracker and quarantine enforcer.
//
// Usage:
//   node quarantine.mjs --record-violation  [--autospec-dir <dir>]
//   node quarantine.mjs --record-success    [--autospec-dir <dir>]
//
// State file: <autospec-dir>/scoped-prod-violations.json
//   { consecutive_violations: N, total_violations: N, last_violation_ts: epoch }
//
// Quarantine trigger: 2 consecutive violations → patches test.yml:
//   mode: scoped_production  →  mode: scoped_production_quarantined
//
// This is the ONE exception to loop-immutability for .autospec/test.yml.
// Only the quarantine bit is written, never by the loop itself.
//
// Exit codes: 0=success, 1=failure

import { parseArgs } from 'node:util';
import fs from 'node:fs';
import path from 'node:path';

const { values: args } = parseArgs({
    options: {
        'record-violation': { type: 'boolean' },
        'record-success':   { type: 'boolean' },
        'autospec-dir':     { type: 'string' },
    },
    strict: false,
});

const autospecDir = args['autospec-dir'] || process.env.AUTOSPEC_DIR || path.join(process.cwd(), '.autospec');
const VIOLATIONS_FILE = path.join(autospecDir, 'scoped-prod-violations.json');
const TEST_YML = path.join(autospecDir, 'test.yml');
const QUARANTINE_THRESHOLD = 2;

// ── Load current state ────────────────────────────────────────────────────────

function loadState() {
    if (fs.existsSync(VIOLATIONS_FILE)) {
        try {
            return JSON.parse(fs.readFileSync(VIOLATIONS_FILE, 'utf8'));
        } catch {
            // Corrupt state — start fresh
        }
    }
    return { consecutive_violations: 0, total_violations: 0, last_violation_ts: 0 };
}

function saveState(state) {
    fs.mkdirSync(autospecDir, { recursive: true });
    fs.writeFileSync(VIOLATIONS_FILE, JSON.stringify(state, null, 2) + '\n');
}

// ── Quarantine: patch test.yml mode field ─────────────────────────────────────

function applyQuarantine() {
    if (!fs.existsSync(TEST_YML)) {
        process.stderr.write(`quarantine: WARNING: ${TEST_YML} not found; cannot set quarantine mode\n`);
        return;
    }

    let yml = fs.readFileSync(TEST_YML, 'utf8');

    // Already quarantined — idempotent
    if (yml.includes('scoped_production_quarantined')) {
        process.stderr.write('quarantine: already in quarantined state (idempotent)\n');
        return;
    }

    // Replace mode: scoped_production with mode: scoped_production_quarantined
    // Only replaces the exact mode line, not any other occurrences
    const updated = yml.replace(
        /^(mode:\s*)scoped_production(\s*)$/m,
        '$1scoped_production_quarantined$2'
    );

    if (updated === yml) {
        process.stderr.write('quarantine: WARNING: could not find "mode: scoped_production" in test.yml to patch\n');
        return;
    }

    fs.writeFileSync(TEST_YML, updated);
    process.stderr.write('quarantine: test.yml patched → mode: scoped_production_quarantined\n');
    process.stdout.write('quarantine: QUARANTINED after 2 consecutive scope violations. Manual re-ack required.\n');
}

// ── Main ──────────────────────────────────────────────────────────────────────

if (args['record-violation']) {
    const state = loadState();
    state.consecutive_violations = (state.consecutive_violations || 0) + 1;
    state.total_violations = (state.total_violations || 0) + 1;
    state.last_violation_ts = Math.floor(Date.now() / 1000);
    saveState(state);

    process.stdout.write(`quarantine: violation recorded (consecutive=${state.consecutive_violations}, total=${state.total_violations})\n`);

    if (state.consecutive_violations >= QUARANTINE_THRESHOLD) {
        applyQuarantine();
    }

    process.exit(0);
} else if (args['record-success']) {
    const state = loadState();
    state.consecutive_violations = 0;
    state.last_success_ts = Math.floor(Date.now() / 1000);
    saveState(state);

    process.stdout.write(`quarantine: success recorded; consecutive violations reset to 0 (total=${state.total_violations})\n`);
    process.exit(0);
} else {
    process.stderr.write('quarantine: usage: quarantine.mjs --record-violation | --record-success [--autospec-dir <dir>]\n');
    process.exit(1);
}
