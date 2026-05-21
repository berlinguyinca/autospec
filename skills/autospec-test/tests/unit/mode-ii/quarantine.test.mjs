// tests/unit/mode-ii/quarantine.test.mjs
// node --test
// Tests quarantine.mjs: 2-consecutive-violations triggers scoped_production_quarantined.

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const SCRIPTS_DIR = path.resolve(__dirname, '../../../scripts');
const QUARANTINE = path.join(SCRIPTS_DIR, 'quarantine.mjs');

function makeTmpDir() {
    return fs.mkdtempSync(path.join(os.tmpdir(), 'autospec-quarantine-'));
}

function runQuarantine(args, env = {}) {
    const result = spawnSync('node', [QUARANTINE, ...args], {
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

function readViolations(dir) {
    const p = path.join(dir, 'scoped-prod-violations.json');
    if (!fs.existsSync(p)) return null;
    return JSON.parse(fs.readFileSync(p, 'utf8'));
}

function readTestYml(dir) {
    const p = path.join(dir, 'test.yml');
    if (!fs.existsSync(p)) return null;
    return fs.readFileSync(p, 'utf8');
}

// ── First violation: increments counter, does not quarantine ──────────────────

test('quarantine: first violation increments counter, does not quarantine', () => {
    const tmpDir = makeTmpDir();
    try {
        // Provide a minimal test.yml
        fs.writeFileSync(path.join(tmpDir, 'test.yml'), 'mode: scoped_production\n');

        const r = runQuarantine(['--record-violation'], {
            AUTOSPEC_DIR: tmpDir,
        });
        assert.strictEqual(r.exitCode, 0, `quarantine failed: ${r.stderr}`);

        const v = readViolations(tmpDir);
        assert.ok(v !== null, 'violations file should exist');
        assert.ok(v.consecutive_violations >= 1, `consecutive_violations should be ≥1, got ${v.consecutive_violations}`);

        const yml = readTestYml(tmpDir);
        assert.ok(!yml.includes('scoped_production_quarantined'), 'should NOT quarantine after first violation');
    } finally {
        fs.rmSync(tmpDir, { recursive: true, force: true });
    }
});

// ── Second consecutive violation: triggers quarantine ─────────────────────────

test('quarantine: second consecutive violation sets mode to scoped_production_quarantined', () => {
    const tmpDir = makeTmpDir();
    try {
        fs.writeFileSync(path.join(tmpDir, 'test.yml'), 'mode: scoped_production\n');
        // Seed with 1 existing consecutive violation
        fs.writeFileSync(path.join(tmpDir, 'scoped-prod-violations.json'), JSON.stringify({
            consecutive_violations: 1,
            total_violations: 1,
            last_violation_ts: Math.floor(Date.now() / 1000),
        }));

        const r = runQuarantine(['--record-violation'], {
            AUTOSPEC_DIR: tmpDir,
        });
        assert.strictEqual(r.exitCode, 0, `quarantine failed: ${r.stderr}`);

        const v = readViolations(tmpDir);
        assert.strictEqual(v.consecutive_violations, 2, `expected 2 consecutive violations, got ${v.consecutive_violations}`);

        const yml = readTestYml(tmpDir);
        assert.ok(yml.includes('scoped_production_quarantined'), `test.yml should contain scoped_production_quarantined. Got: ${yml}`);
    } finally {
        fs.rmSync(tmpDir, { recursive: true, force: true });
    }
});

// ── Successful run resets consecutive count ────────────────────────────────────

test('quarantine: successful run resets consecutive violation counter', () => {
    const tmpDir = makeTmpDir();
    try {
        fs.writeFileSync(path.join(tmpDir, 'test.yml'), 'mode: scoped_production\n');
        fs.writeFileSync(path.join(tmpDir, 'scoped-prod-violations.json'), JSON.stringify({
            consecutive_violations: 1,
            total_violations: 3,
            last_violation_ts: Math.floor(Date.now() / 1000),
        }));

        const r = runQuarantine(['--record-success'], {
            AUTOSPEC_DIR: tmpDir,
        });
        assert.strictEqual(r.exitCode, 0, `quarantine record-success failed: ${r.stderr}`);

        const v = readViolations(tmpDir);
        assert.strictEqual(v.consecutive_violations, 0, `consecutive_violations should reset to 0 after success, got ${v.consecutive_violations}`);
    } finally {
        fs.rmSync(tmpDir, { recursive: true, force: true });
    }
});

// ── Quarantine does NOT update if already quarantined ─────────────────────────

test('quarantine: already-quarantined mode is preserved (idempotent)', () => {
    const tmpDir = makeTmpDir();
    try {
        fs.writeFileSync(path.join(tmpDir, 'test.yml'), 'mode: scoped_production_quarantined\n');
        fs.writeFileSync(path.join(tmpDir, 'scoped-prod-violations.json'), JSON.stringify({
            consecutive_violations: 2,
            total_violations: 5,
            last_violation_ts: Math.floor(Date.now() / 1000),
        }));

        const r = runQuarantine(['--record-violation'], {
            AUTOSPEC_DIR: tmpDir,
        });
        assert.strictEqual(r.exitCode, 0);

        const yml = readTestYml(tmpDir);
        assert.ok(yml.includes('scoped_production_quarantined'), 'mode should remain quarantined');
    } finally {
        fs.rmSync(tmpDir, { recursive: true, force: true });
    }
});
