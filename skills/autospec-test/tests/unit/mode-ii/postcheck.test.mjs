// tests/unit/mode-ii/postcheck.test.mjs
// node --test
// Tests mode-ii-postcheck.mjs: scope violation detection, restore-on-violation,
// CRITICAL sentinel on restore failure. Uses real SQLite fixture. No mocks.

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const SCRIPTS_DIR = path.resolve(__dirname, '../../../scripts');
const POSTCHECK = path.join(SCRIPTS_DIR, 'mode-ii-postcheck.mjs');

function makeTmpDir() {
    return fs.mkdtempSync(path.join(os.tmpdir(), 'autospec-postcheck-'));
}

function runPostcheck(args, contractJson, env = {}) {
    // Write contract to temp file to avoid stdin complexity
    const tmpFile = path.join(os.tmpdir(), `autospec-contract-${Date.now()}.json`);
    fs.writeFileSync(tmpFile, JSON.stringify(contractJson));
    try {
        // Replace '--contract', '-' with '--contract', tmpFile
        const resolvedArgs = args.map(a => a === '-' ? tmpFile : a);
        const result = spawnSync('node', [POSTCHECK, ...resolvedArgs], {
            encoding: 'utf8',
            env: { ...process.env, ...env },
            timeout: 15000,
        });
        return {
            exitCode: result.status,
            stdout: result.stdout || '',
            stderr: result.stderr || '',
        };
    } finally {
        fs.rmSync(tmpFile, { force: true });
    }
}

function hasSqlite3() {
    const r = spawnSync('sqlite3', ['--version'], { encoding: 'utf8' });
    return r.status === 0;
}

function createSqliteDb(dbPath, rows) {
    // rows: array of { id, updated_at }
    const insertRows = rows.map(r =>
        `INSERT INTO families VALUES ('${r.id}', 'Test', ${r.updated_at});`
    ).join('\n');

    const sql = `
        CREATE TABLE families (id TEXT, name TEXT, updated_at INTEGER);
        ${insertRows}
    `;
    const result = spawnSync('sqlite3', [dbPath, sql], { encoding: 'utf8' });
    return result.status === 0;
}

// ── Scope-pass: no out-of-scope mutations ──────────────────────────────────────

test('postcheck: passes when no out-of-scope rows exist', () => {
    if (!hasSqlite3()) return; // skip if sqlite3 not available

    const tmpDir = makeTmpDir();
    try {
        const dbPath = path.join(tmpDir, 'test.db');
        const now = Math.floor(Date.now() / 1000);

        const ok = createSqliteDb(dbPath, [
            { id: 'test-family-7a3f9c', updated_at: now },
        ]);
        if (!ok) return; // sqlite3 create failed

        const contract = {
            e2e: {
                production_scoped_access: {
                    scope_tokens: [
                        { kind: 'row_filter', table: 'families', column: 'id', allowed_values: ['test-family-7a3f9c'], out_of_scope_action: 'hard_fail' },
                    ],
                },
                backup: {
                    driver: 'custom',
                    custom_restore_cmd: 'true',
                },
            },
        };

        const r = runPostcheck(
            ['--window-from', '0', '--window-to', 'now', '--db', dbPath, '--contract', '-'],
            contract,
            { AUTOSPEC_DIR: tmpDir }
        );
        assert.strictEqual(r.exitCode, 0, `postcheck failed unexpectedly: ${r.stderr} stdout: ${r.stdout}`);
        const json = JSON.parse(r.stdout);
        assert.strictEqual(json.passed, true);
    } finally {
        fs.rmSync(tmpDir, { recursive: true, force: true });
    }
});

// ── Scope-violation: out-of-scope row triggers restore ─────────────────────────

test('postcheck: triggers restore on out-of-scope mutation', () => {
    if (!hasSqlite3()) return;

    const tmpDir = makeTmpDir();
    try {
        const dbPath = path.join(tmpDir, 'test.db');
        const bakPath = path.join(tmpDir, 'test.db.bak');
        const restoreLog = path.join(tmpDir, 'restore.log');
        const now = Math.floor(Date.now() / 1000);

        const ok = createSqliteDb(dbPath, [
            { id: 'test-family-7a3f9c', updated_at: now },
            { id: 'out-of-scope-id', updated_at: now },
        ]);
        if (!ok) return;

        fs.copyFileSync(dbPath, bakPath);
        const restoreCmd = `echo restored >> ${restoreLog} && cp ${bakPath} ${dbPath}`;

        const contract = {
            e2e: {
                production_scoped_access: {
                    scope_tokens: [
                        { kind: 'row_filter', table: 'families', column: 'id', allowed_values: ['test-family-7a3f9c'], out_of_scope_action: 'hard_fail' },
                    ],
                },
                backup: {
                    driver: 'custom',
                    custom_restore_cmd: restoreCmd,
                },
            },
        };

        const r = runPostcheck(
            ['--window-from', '0', '--window-to', 'now', '--db', dbPath, '--contract', '-'],
            contract,
            { AUTOSPEC_DIR: tmpDir }
        );

        // Should exit non-zero (scope violation found)
        assert.notStrictEqual(r.exitCode, 0, `expected scope violation but postcheck passed. stdout: ${r.stdout}`);
        // Restore should have been invoked
        assert.ok(
            fs.existsSync(restoreLog) && fs.readFileSync(restoreLog, 'utf8').includes('restored'),
            `restore cmd should have been called on scope violation. stdout: ${r.stdout} stderr: ${r.stderr}`
        );
    } finally {
        fs.rmSync(tmpDir, { recursive: true, force: true });
    }
});

// ── Restore failure writes .CRITICAL sentinel ──────────────────────────────────

test('postcheck: writes .CRITICAL sentinel when restore fails', () => {
    if (!hasSqlite3()) return;

    const tmpDir = makeTmpDir();
    try {
        const dbPath = path.join(tmpDir, 'test.db');
        const criticalPath = path.join(tmpDir, '.CRITICAL');
        const now = Math.floor(Date.now() / 1000);

        const ok = createSqliteDb(dbPath, [
            { id: 'out-of-scope-id', updated_at: now },
        ]);
        if (!ok) return;

        const contract = {
            e2e: {
                production_scoped_access: {
                    scope_tokens: [
                        { kind: 'row_filter', table: 'families', column: 'id', allowed_values: ['test-family-7a3f9c'], out_of_scope_action: 'hard_fail' },
                    ],
                },
                backup: {
                    driver: 'custom',
                    custom_restore_cmd: 'false',  // restore always fails
                },
            },
        };

        const r = runPostcheck(
            ['--window-from', '0', '--window-to', 'now', '--db', dbPath, '--contract', '-'],
            contract,
            { AUTOSPEC_DIR: tmpDir }
        );

        // Should exit 2 on restore failure
        assert.strictEqual(r.exitCode, 2, `expected exit 2 (CRITICAL), got ${r.exitCode}. stdout: ${r.stdout} stderr: ${r.stderr}`);
        // .CRITICAL sentinel must exist
        assert.ok(fs.existsSync(criticalPath), `.CRITICAL sentinel should be written when restore fails`);
    } finally {
        fs.rmSync(tmpDir, { recursive: true, force: true });
    }
});
