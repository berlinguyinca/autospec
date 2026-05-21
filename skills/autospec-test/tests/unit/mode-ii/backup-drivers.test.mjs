// tests/unit/mode-ii/backup-drivers.test.mjs
// node --test
// Tests the backup driver interface: snapshot, verify, restore
// Uses real cp-based custom driver + sqlite fixture. No mocks.

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const SCRIPTS_DIR = path.resolve(__dirname, '../../../scripts');
const CUSTOM_DRIVER = path.join(SCRIPTS_DIR, 'backup-drivers', 'custom.sh');

function makeTmpDir() {
    return fs.mkdtempSync(path.join(os.tmpdir(), 'autospec-bkp-'));
}

function runDriver(args, env = {}) {
    const result = spawnSync('bash', [CUSTOM_DRIVER, ...args], {
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

// ── Custom driver (cp-based) ───────────────────────────────────────────────────

test('custom driver: snapshot creates backup file and exits 0', () => {
    const tmpDir = makeTmpDir();
    try {
        const src = path.join(tmpDir, 'db.sqlite');
        const bak = path.join(tmpDir, 'db.bak');
        fs.writeFileSync(src, 'SQLITE_DATA_V1');

        const r = runDriver(['snapshot'], {
            AUTOSPEC_CUSTOM_SNAPSHOT_CMD: `cp ${src} ${bak}`,
            AUTOSPEC_CUSTOM_VERIFY_CMD: `test -f ${bak}`,
            AUTOSPEC_CUSTOM_RESTORE_CMD: `cp ${bak} ${src}`,
        });
        assert.strictEqual(r.exitCode, 0, `snapshot failed: ${r.stderr}`);
        assert.ok(fs.existsSync(bak), 'backup file should exist after snapshot');
    } finally {
        fs.rmSync(tmpDir, { recursive: true, force: true });
    }
});

test('custom driver: verify exits 0 when backup exists', () => {
    const tmpDir = makeTmpDir();
    try {
        const src = path.join(tmpDir, 'db.sqlite');
        const bak = path.join(tmpDir, 'db.bak');
        fs.writeFileSync(src, 'DATA');
        fs.writeFileSync(bak, 'DATA');

        const r = runDriver(['verify'], {
            AUTOSPEC_CUSTOM_SNAPSHOT_CMD: `cp ${src} ${bak}`,
            AUTOSPEC_CUSTOM_VERIFY_CMD: `test -f ${bak}`,
            AUTOSPEC_CUSTOM_RESTORE_CMD: `cp ${bak} ${src}`,
        });
        assert.strictEqual(r.exitCode, 0, `verify failed: ${r.stderr}`);
    } finally {
        fs.rmSync(tmpDir, { recursive: true, force: true });
    }
});

test('custom driver: verify exits non-zero when backup missing', () => {
    const tmpDir = makeTmpDir();
    try {
        const bak = path.join(tmpDir, 'db.bak'); // does not exist

        const r = runDriver(['verify'], {
            AUTOSPEC_CUSTOM_SNAPSHOT_CMD: 'true',
            AUTOSPEC_CUSTOM_VERIFY_CMD: `test -f ${bak}`,
            AUTOSPEC_CUSTOM_RESTORE_CMD: 'true',
        });
        assert.notStrictEqual(r.exitCode, 0, 'verify should fail when backup missing');
    } finally {
        fs.rmSync(tmpDir, { recursive: true, force: true });
    }
});

test('custom driver: restore copies backup back and exits 0', () => {
    const tmpDir = makeTmpDir();
    try {
        const src = path.join(tmpDir, 'db.sqlite');
        const bak = path.join(tmpDir, 'db.bak');
        fs.writeFileSync(src, 'CORRUPTED');
        fs.writeFileSync(bak, 'CLEAN_BACKUP');

        const r = runDriver(['restore'], {
            AUTOSPEC_CUSTOM_SNAPSHOT_CMD: `cp ${src} ${bak}`,
            AUTOSPEC_CUSTOM_VERIFY_CMD: `test -f ${bak}`,
            AUTOSPEC_CUSTOM_RESTORE_CMD: `cp ${bak} ${src}`,
        });
        assert.strictEqual(r.exitCode, 0, `restore failed: ${r.stderr}`);
        assert.strictEqual(fs.readFileSync(src, 'utf8'), 'CLEAN_BACKUP', 'source should be restored to backup content');
    } finally {
        fs.rmSync(tmpDir, { recursive: true, force: true });
    }
});

test('custom driver: restore exits non-zero when restore_cmd fails', () => {
    const r = runDriver(['restore'], {
        AUTOSPEC_CUSTOM_SNAPSHOT_CMD: 'true',
        AUTOSPEC_CUSTOM_VERIFY_CMD: 'true',
        AUTOSPEC_CUSTOM_RESTORE_CMD: 'false',  // always fails
    });
    assert.notStrictEqual(r.exitCode, 0, 'restore should propagate failure exit code');
});

test('custom driver: exits non-zero for unknown subcommand', () => {
    const r = runDriver(['unknown_cmd'], {
        AUTOSPEC_CUSTOM_SNAPSHOT_CMD: 'true',
        AUTOSPEC_CUSTOM_VERIFY_CMD: 'true',
        AUTOSPEC_CUSTOM_RESTORE_CMD: 'true',
    });
    assert.notStrictEqual(r.exitCode, 0, 'unknown subcommand should fail');
});

test('custom driver: snapshot prints snapshot id to stdout', () => {
    const tmpDir = makeTmpDir();
    try {
        const bak = path.join(tmpDir, 'db.bak');
        const r = runDriver(['snapshot'], {
            AUTOSPEC_CUSTOM_SNAPSHOT_CMD: `touch ${bak}`,
            AUTOSPEC_CUSTOM_VERIFY_CMD: `test -f ${bak}`,
            AUTOSPEC_CUSTOM_RESTORE_CMD: 'true',
        });
        assert.strictEqual(r.exitCode, 0);
        assert.ok(r.stdout.trim().length > 0, 'snapshot should print an id to stdout');
    } finally {
        fs.rmSync(tmpDir, { recursive: true, force: true });
    }
});
