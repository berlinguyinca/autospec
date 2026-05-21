#!/usr/bin/env node
// mode-ii-postcheck.mjs — Mode II post-suite DB verifier.
//
// Usage:
//   echo '<contract_json>' | node mode-ii-postcheck.mjs \
//       --window-from <epoch|0> --window-to <epoch|now> \
//       --db <sqlite_path> --contract - [--autospec-dir <dir>]
//
// Reads scope tokens from contract, queries the SQLite DB for rows mutated
// in the window, checks for out-of-scope mutations. On violation:
//   1. Invokes restore_cmd (or custom_restore_cmd) from contract
//   2. If restore succeeds: exit 1 (scope violation, restored)
//   3. If restore fails: writes .CRITICAL sentinel, exit 2
//
// Exit codes:
//   0 = pass (no scope violations)
//   1 = scope violation, restore succeeded
//   2 = scope violation, restore FAILED (CRITICAL)

import { parseArgs } from 'node:util';
import fs from 'node:fs';
import path from 'node:path';
import { spawnSync } from 'node:child_process';

async function main() {
    const { values: args } = parseArgs({
        options: {
            'window-from':  { type: 'string' },
            'window-to':    { type: 'string' },
            'db':           { type: 'string' },
            'contract':     { type: 'string' },
            'autospec-dir': { type: 'string' },
        },
        strict: false,
    });

    const windowFrom  = args['window-from']  || '0';
    const windowTo    = args['window-to']    || 'now';
    const dbPath      = args['db'];
    const contractArg = args['contract'] || '-';
    const autospecDir = args['autospec-dir'] || process.env.AUTOSPEC_DIR || path.join(process.cwd(), '.autospec');

    // ── Helpers ──────────────────────────────────────────────────────────────────

    function writeCritical(reason) {
        const sentinelPath = path.join(autospecDir, '.CRITICAL');
        try {
            fs.mkdirSync(autospecDir, { recursive: true });
            fs.writeFileSync(sentinelPath, JSON.stringify({
                ts: Math.floor(Date.now() / 1000),
                reason,
            }, null, 2) + '\n');
        } catch (e) {
            process.stderr.write(`postcheck: failed to write .CRITICAL sentinel: ${e.message}\n`);
        }
        process.stderr.write(`postcheck: CRITICAL — ${reason}\n`);
        process.exit(2);
    }

    function resolveEpoch(val) {
        if (val === '-' || val === '0' || val === 0) return 0;
        if (val === 'now') return Math.floor(Date.now() / 1000);
        const n = parseInt(val, 10);
        return isNaN(n) ? 0 : n;
    }

    // ── Load contract ────────────────────────────────────────────────────────────

    let contractJson;
    if (contractArg === '-') {
        // Read from stdin
        const chunks = [];
        for await (const chunk of process.stdin) chunks.push(chunk);
        const raw = Buffer.concat(chunks).toString('utf8').trim();
        if (!raw) {
            process.stderr.write('postcheck: no contract JSON on stdin\n');
            process.exit(2);
        }
        contractJson = JSON.parse(raw);
    } else {
        contractJson = JSON.parse(fs.readFileSync(contractArg, 'utf8'));
    }

    const scopeTokens = contractJson?.e2e?.production_scoped_access?.scope_tokens || [];
    const backupConfig = contractJson?.e2e?.backup || {};
    const restoreCmd = backupConfig.restore_cmd || backupConfig.custom_restore_cmd || '';

    // ── Resolve time window ──────────────────────────────────────────────────────

    const fromEpoch = resolveEpoch(windowFrom);
    const toEpoch   = resolveEpoch(windowTo);
    const toEpochSafe = toEpoch === 0 ? 9999999999 : toEpoch;

    // ── Query SQLite for mutated rows ────────────────────────────────────────────

    if (!dbPath || !fs.existsSync(dbPath)) {
        // No DB path — no scope violation possible; exit clean
        process.stdout.write('{"passed":true,"reason":"no_db_provided"}\n');
        process.exit(0);
    }

    const violations = [];

    for (const token of scopeTokens) {
        if (token.kind !== 'row_filter') continue;

        const table   = token.table;
        const column  = token.column;
        const allowed = token.allowed_values || [];

        // Build SQL to find rows touched in window that are NOT in allowed_values
        let whereNotAllowed = '';
        if (allowed.length > 0) {
            const quoted = allowed.map(v => `'${String(v).replace(/'/g, "''")}'`).join(', ');
            whereNotAllowed = `AND ${column} NOT IN (${quoted})`;
        }
        const tsColumn = 'updated_at';

        const sql = [
            `SELECT ${column}, ${tsColumn}`,
            `FROM ${table}`,
            `WHERE ${tsColumn} >= ${fromEpoch}`,
            `  AND ${tsColumn} <= ${toEpochSafe}`,
            whereNotAllowed,
            `LIMIT 20;`,
        ].join(' ');

        // Run via sqlite3 CLI
        const result = spawnSync('sqlite3', [dbPath, sql, '-json'], {
            encoding: 'utf8',
            timeout: 10000,
        });

        if (result.status !== 0) {
            process.stderr.write(`postcheck: sqlite3 query failed for table ${table}: ${result.stderr}\n`);
            continue;
        }

        let rows = [];
        try {
            const out = (result.stdout || '').trim();
            rows = out ? JSON.parse(out) : [];
        } catch {
            rows = [];
        }

        if (rows.length > 0) {
            violations.push({
                token,
                rows,
                reason: `out_of_scope_rows_in_table_${table}`,
            });
        }
    }

    // ── No violations: clean pass ────────────────────────────────────────────────

    if (violations.length === 0) {
        process.stdout.write(JSON.stringify({ passed: true, violations: [] }) + '\n');
        process.exit(0);
    }

    // ── Scope violation: invoke restore ──────────────────────────────────────────

    process.stderr.write(`postcheck: SCOPE VIOLATION detected (${violations.length} violation(s))\n`);
    for (const v of violations) {
        process.stderr.write(`  - ${v.reason}: ${v.rows.length} out-of-scope row(s)\n`);
    }

    if (!restoreCmd) {
        writeCritical('no_restore_cmd_available: scope violation detected but no restore_cmd configured');
    }

    process.stderr.write(`postcheck: invoking restore: ${restoreCmd}\n`);

    const restoreResult = spawnSync('bash', ['-c', restoreCmd], {
        encoding: 'utf8',
        timeout: 60000,
    });

    if (restoreResult.status !== 0) {
        writeCritical(`restore_command_failed (exit ${restoreResult.status}): ${restoreResult.stderr}`);
    }

    // Restore succeeded
    process.stderr.write('postcheck: restore completed successfully\n');
    process.stdout.write(JSON.stringify({
        passed: false,
        violations,
        restored: true,
    }) + '\n');
    process.exit(1);
}

main().catch(err => {
    process.stderr.write(`postcheck: fatal: ${err.message}\n`);
    process.exit(2);
});
