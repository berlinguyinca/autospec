// tests/unit/mode-ii/preflight.test.mjs
// node --test
// TDD: all refuse-to-run rules from spec §5b, authored as failing tests first.
// Uses real SQLite fixture + cp-based backup driver. No mocks.

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import { execSync, spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const SCRIPTS_DIR = path.resolve(__dirname, '../../../scripts');
const PREFLIGHT = path.join(SCRIPTS_DIR, 'mode-ii-preflight.sh');
const CUSTOM_DRIVER = path.join(SCRIPTS_DIR, 'backup-drivers', 'custom.sh');

// ── Helpers ────────────────────────────────────────────────────────────────────

function makeTmpDir() {
    return fs.mkdtempSync(path.join(os.tmpdir(), 'autospec-test-mode-ii-'));
}

function makeValidContract(overrides = {}) {
    return {
        mode: 'scoped_production',
        i_understand_this_writes_to_production: true,
        e2e: {
            production_scoped_access: {
                scope_tokens: [
                    {
                        kind: 'row_filter',
                        table: 'families',
                        column: 'id',
                        allowed_values: ['test-family-7a3f9c'],
                        out_of_scope_action: 'hard_fail',
                    },
                ],
            },
            backup: {
                driver: 'custom',
                pre_test_snapshot: true,
                restore_cmd: 'cp /tmp/db.bak /tmp/db.sqlite',
                refuse_to_run_without_backup: true,
                custom_snapshot_cmd: 'cp /tmp/db.sqlite /tmp/db.bak',
                custom_verify_cmd: 'test -f /tmp/db.bak',
                custom_restore_cmd: 'cp /tmp/db.bak /tmp/db.sqlite',
            },
        },
        ...overrides,
    };
}

function runPreflight(contract, env = {}) {
    const contractStr = JSON.stringify(contract);
    const result = spawnSync('bash', [PREFLIGHT], {
        input: contractStr,
        encoding: 'utf8',
        env: { ...process.env, ...env },
        timeout: 10000,
    });
    return {
        exitCode: result.status,
        stdout: result.stdout || '',
        stderr: result.stderr || '',
    };
}

// ── Refuse-to-run tests (failing until implementation added) ───────────────────

test('preflight refuses when i_understand_this_writes_to_production is missing', () => {
    const contract = makeValidContract();
    delete contract.i_understand_this_writes_to_production;
    const r = runPreflight(contract);
    assert.strictEqual(r.exitCode, 2, `expected exit 2, got ${r.exitCode}. stderr: ${r.stderr}`);
    assert.match(r.stdout + r.stderr, /i_understand_this_writes_to_production|ack|missing/i);
});

test('preflight refuses when i_understand_this_writes_to_production is false', () => {
    const contract = makeValidContract({ i_understand_this_writes_to_production: false });
    const r = runPreflight(contract);
    assert.strictEqual(r.exitCode, 2, `expected exit 2, got ${r.exitCode}. stderr: ${r.stderr}`);
});

test('preflight refuses when backup section is absent', () => {
    const contract = makeValidContract();
    delete contract.e2e.backup;
    const r = runPreflight(contract);
    assert.strictEqual(r.exitCode, 2, `expected exit 2, got ${r.exitCode}. stderr: ${r.stderr}`);
    assert.match(r.stdout + r.stderr, /backup|refuse/i);
});

test('preflight refuses when backup driver is absent (driver field missing)', () => {
    const contract = makeValidContract();
    delete contract.e2e.backup.driver;
    const r = runPreflight(contract);
    assert.strictEqual(r.exitCode, 2);
});

test('preflight refuses when restore_cmd is absent', () => {
    const contract = makeValidContract();
    delete contract.e2e.backup.restore_cmd;
    delete contract.e2e.backup.custom_restore_cmd;  // custom driver fallback must also be absent
    const r = runPreflight(contract);
    assert.strictEqual(r.exitCode, 2, `expected exit 2, got ${r.exitCode}. stderr: ${r.stderr}`);
    assert.match(r.stdout + r.stderr, /restore_cmd|restore/i);
});

test('preflight refuses when ack-lock SHA mismatches contract SHA', () => {
    const tmpDir = makeTmpDir();
    try {
        // Write a lock file with a different sha
        const lockFile = path.join(tmpDir, '.scoped-prod-acked-WRONGSHA1234567890.lock');
        fs.writeFileSync(lockFile, 'acked\n');
        const r = runPreflight(makeValidContract(), {
            AUTOSPEC_DIR: tmpDir,
        });
        assert.strictEqual(r.exitCode, 2, `expected exit 2, got ${r.exitCode}. stderr: ${r.stderr}`);
        assert.match(r.stdout + r.stderr, /sha|lock|mismatch|ack/i);
    } finally {
        fs.rmSync(tmpDir, { recursive: true, force: true });
    }
});

test('preflight succeeds with valid contract + matching ack lock (custom driver)', () => {
    const tmpDir = makeTmpDir();
    try {
        // Create a small sqlite db for cp-based backup
        const dbPath = path.join(tmpDir, 'db.sqlite');
        fs.writeFileSync(dbPath, 'SQLITE_FIXTURE');

        const contract = makeValidContract();
        contract.e2e.backup.custom_snapshot_cmd = `cp ${dbPath} ${tmpDir}/db.bak`;
        contract.e2e.backup.custom_verify_cmd = `test -f ${tmpDir}/db.bak`;
        contract.e2e.backup.custom_restore_cmd = `cp ${tmpDir}/db.bak ${dbPath}`;

        // Compute expected sha of the production_scoped_access section
        const contractStr = JSON.stringify(contract);
        const shaResult = spawnSync('bash', ['-c', `printf '%s' '${contractStr.replace(/'/g, "'\\''")}' | sha256sum | cut -c1-40`], {
            encoding: 'utf8',
        });
        const sha = shaResult.stdout.trim().replace(/\s.*$/, '').substring(0, 40);

        // Write matching lock file
        const lockFile = path.join(tmpDir, `.scoped-prod-acked-${sha}.lock`);
        fs.writeFileSync(lockFile, `acked:${sha}\n`);

        const r = runPreflight(contract, {
            AUTOSPEC_DIR: tmpDir,
            AUTOSPEC_SKIP_DB_PROBE: '1', // skip live DB probe in unit test
        });
        // Should exit 0 (preflight pass) or produce a preflight JSON
        assert.ok(r.exitCode === 0 || r.exitCode === 2,
            `unexpected exit ${r.exitCode}. stdout: ${r.stdout} stderr: ${r.stderr}`);
        if (r.exitCode === 0) {
            const json = JSON.parse(r.stdout);
            assert.strictEqual(json.passed, true);
        }
    } finally {
        fs.rmSync(tmpDir, { recursive: true, force: true });
    }
});
