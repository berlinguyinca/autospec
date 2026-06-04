// skills/autospec-test/tests/unit/authoring-config.test.mjs
// node --test  (Node.js built-in test runner)
// Tests for scripts/authoring-config.mjs — loadAuthoringConfig()

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import fs from 'node:fs';
import os from 'node:os';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const SCRIPTS_DIR = path.resolve(__dirname, '../../scripts');

const { loadAuthoringConfig } = await import(`file://${SCRIPTS_DIR}/authoring-config.mjs`);

// ── Helpers ────────────────────────────────────────────────────────────────────

function writeTempYml(content) {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'authoring-config-test-'));
    const ymlPath = path.join(dir, 'test.yml');
    fs.writeFileSync(ymlPath, content, 'utf8');
    return { dir, ymlPath };
}

function cleanup(dir) {
    fs.rmSync(dir, { recursive: true, force: true });
}

// ── Defaults ───────────────────────────────────────────────────────────────────

test('loadAuthoringConfig: returns authoring disabled by default when no yml', async () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'authoring-config-test-'));
    const ymlPath = path.join(dir, 'missing.yml');
    try {
        const cfg = await loadAuthoringConfig(ymlPath);
        assert.equal(cfg.authoring.enabled, false, 'authoring.enabled should default to false');
        assert.equal(cfg.authoring.spec_dir, 'e2e/specs');
        assert.equal(cfg.authoring.helpers_dir, 'e2e/helpers');
        assert.equal(cfg.authoring.route_clusters, 'auto');
        assert.equal(cfg.authoring.coverage_target_pct, 80);
        assert.equal(cfg.authoring.fanout_max, 4);
    } finally {
        cleanup(dir);
    }
});

test('loadAuthoringConfig: returns reset defaults when no yml', async () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'authoring-config-test-'));
    const ymlPath = path.join(dir, 'missing.yml');
    try {
        const cfg = await loadAuthoringConfig(ymlPath);
        assert.equal(cfg.reset.endpoint, '/api/test/reset');
        assert.equal(cfg.reset.generate_if_missing, false, 'generate_if_missing defaults false');
        assert.equal(cfg.reset.guard_env, 'AUTOSPEC_TEST_STACK');
    } finally {
        cleanup(dir);
    }
});

test('loadAuthoringConfig: returns control_effects defaults when no yml', async () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'authoring-config-test-'));
    const ymlPath = path.join(dir, 'missing.yml');
    try {
        const cfg = await loadAuthoringConfig(ymlPath);
        assert.equal(cfg.control_effects.enabled, true, 'control_effects.enabled defaults true');
    } finally {
        cleanup(dir);
    }
});

// ── Happy path: well-formed yml ───────────────────────────────────────────────

test('loadAuthoringConfig: reads authoring block from yml', async () => {
    const { dir, ymlPath } = writeTempYml(`
e2e:
  authoring:
    enabled: true
    spec_dir: tests/e2e/specs
    helpers_dir: tests/e2e/helpers
    route_clusters: auto
    coverage_target_pct: 90
    fanout_max: 6
  reset:
    endpoint: /api/test/reset
    generate_if_missing: false
    guard_env: AUTOSPEC_TEST_STACK
  control_effects:
    enabled: true
  forbidden_url_patterns:
    - "^https?://prod.example.com"
`);
    try {
        const cfg = await loadAuthoringConfig(ymlPath);
        assert.equal(cfg.authoring.enabled, true);
        assert.equal(cfg.authoring.spec_dir, 'tests/e2e/specs');
        assert.equal(cfg.authoring.helpers_dir, 'tests/e2e/helpers');
        assert.equal(cfg.authoring.coverage_target_pct, 90);
        assert.equal(cfg.authoring.fanout_max, 6);
    } finally {
        cleanup(dir);
    }
});

test('loadAuthoringConfig: reads reset block from yml', async () => {
    const { dir, ymlPath } = writeTempYml(`
e2e:
  authoring:
    enabled: false
  reset:
    endpoint: /api/v2/reset
    generate_if_missing: true
    guard_env: MY_TEST_ENV
  control_effects:
    enabled: false
  forbidden_url_patterns:
    - "^https?://prod.example.com"
`);
    try {
        const cfg = await loadAuthoringConfig(ymlPath);
        assert.equal(cfg.reset.endpoint, '/api/v2/reset');
        assert.equal(cfg.reset.generate_if_missing, true);
        assert.equal(cfg.reset.guard_env, 'MY_TEST_ENV');
    } finally {
        cleanup(dir);
    }
});

test('loadAuthoringConfig: reads reset.cmd instead of endpoint', async () => {
    const { dir, ymlPath } = writeTempYml(`
e2e:
  authoring:
    enabled: false
  reset:
    cmd: "make db-reset"
    generate_if_missing: false
  control_effects:
    enabled: false
  forbidden_url_patterns:
    - "^https?://prod.example.com"
`);
    try {
        const cfg = await loadAuthoringConfig(ymlPath);
        assert.equal(cfg.reset.cmd, 'make db-reset');
        assert.equal(cfg.reset.endpoint, undefined);
    } finally {
        cleanup(dir);
    }
});

test('loadAuthoringConfig: reads control_effects block from yml', async () => {
    const { dir, ymlPath } = writeTempYml(`
e2e:
  authoring:
    enabled: false
  reset:
    generate_if_missing: false
  control_effects:
    enabled: false
  forbidden_url_patterns:
    - "^https?://prod.example.com"
`);
    try {
        const cfg = await loadAuthoringConfig(ymlPath);
        assert.equal(cfg.control_effects.enabled, false);
    } finally {
        cleanup(dir);
    }
});

// ── Fail-closed: generate_if_missing without guard_env ───────────────────────

test('loadAuthoringConfig: throws if generate_if_missing=true and guard_env missing', async () => {
    const { dir, ymlPath } = writeTempYml(`
e2e:
  authoring:
    enabled: false
  reset:
    endpoint: /api/test/reset
    generate_if_missing: true
  control_effects:
    enabled: false
  forbidden_url_patterns:
    - "^https?://prod.example.com"
`);
    try {
        await assert.rejects(
            async () => await loadAuthoringConfig(ymlPath),
            (err) => {
                assert.ok(err.message.includes('guard_env'), `Expected guard_env mention in: ${err.message}`);
                return true;
            }
        );
    } finally {
        cleanup(dir);
    }
});

test('loadAuthoringConfig: throws if generate_if_missing=true and guard_env is empty string', async () => {
    const { dir, ymlPath } = writeTempYml(`
e2e:
  authoring:
    enabled: false
  reset:
    endpoint: /api/test/reset
    generate_if_missing: true
    guard_env: ""
  control_effects:
    enabled: false
  forbidden_url_patterns:
    - "^https?://prod.example.com"
`);
    try {
        await assert.rejects(
            async () => await loadAuthoringConfig(ymlPath),
            (err) => {
                assert.ok(err.message.includes('guard_env'), `Expected guard_env mention in: ${err.message}`);
                return true;
            }
        );
    } finally {
        cleanup(dir);
    }
});

test('loadAuthoringConfig: no throw if generate_if_missing=false and guard_env missing', async () => {
    const { dir, ymlPath } = writeTempYml(`
e2e:
  authoring:
    enabled: false
  reset:
    endpoint: /api/test/reset
    generate_if_missing: false
  control_effects:
    enabled: false
  forbidden_url_patterns:
    - "^https?://prod.example.com"
`);
    try {
        const cfg = await loadAuthoringConfig(ymlPath);
        assert.equal(cfg.reset.generate_if_missing, false);
    } finally {
        cleanup(dir);
    }
});

// ── Fail-closed: forbidden_url_patterns ──────────────────────────────────────

test('loadAuthoringConfig: throws if forbidden_url_patterns is missing', async () => {
    const { dir, ymlPath } = writeTempYml(`
e2e:
  authoring:
    enabled: false
  reset:
    generate_if_missing: false
  control_effects:
    enabled: false
`);
    try {
        await assert.rejects(
            async () => await loadAuthoringConfig(ymlPath),
            (err) => {
                assert.ok(
                    err.message.includes('forbidden_url_patterns'),
                    `Expected forbidden_url_patterns mention in: ${err.message}`
                );
                return true;
            }
        );
    } finally {
        cleanup(dir);
    }
});

test('loadAuthoringConfig: throws if forbidden_url_patterns is empty array', async () => {
    const { dir, ymlPath } = writeTempYml(`
e2e:
  authoring:
    enabled: false
  reset:
    generate_if_missing: false
  control_effects:
    enabled: false
  forbidden_url_patterns: []
`);
    try {
        await assert.rejects(
            async () => await loadAuthoringConfig(ymlPath),
            (err) => {
                assert.ok(
                    err.message.includes('forbidden_url_patterns'),
                    `Expected forbidden_url_patterns mention in: ${err.message}`
                );
                return true;
            }
        );
    } finally {
        cleanup(dir);
    }
});

test('loadAuthoringConfig: forbidden_url_patterns_intentionally_empty bypasses check', async () => {
    const { dir, ymlPath } = writeTempYml(`
e2e:
  authoring:
    enabled: false
  reset:
    generate_if_missing: false
  control_effects:
    enabled: false
  forbidden_url_patterns: []
  forbidden_url_patterns_intentionally_empty: true
`);
    try {
        const cfg = await loadAuthoringConfig(ymlPath);
        assert.equal(cfg.authoring.enabled, false);
    } finally {
        cleanup(dir);
    }
});

// ── No e2e block at all — defaults apply, but forbidden_url_patterns missing throws ──

test('loadAuthoringConfig: throws if yml exists with no e2e.forbidden_url_patterns', async () => {
    const { dir, ymlPath } = writeTempYml(`
e2e:
  authoring:
    enabled: false
  reset:
    generate_if_missing: false
  control_effects:
    enabled: true
`);
    try {
        await assert.rejects(
            async () => await loadAuthoringConfig(ymlPath),
            (err) => {
                assert.ok(
                    err.message.includes('forbidden_url_patterns'),
                    `Expected forbidden_url_patterns mention in: ${err.message}`
                );
                return true;
            }
        );
    } finally {
        cleanup(dir);
    }
});

// ── Return shape ──────────────────────────────────────────────────────────────

test('loadAuthoringConfig: returned object has exactly authoring, reset, control_effects keys', async () => {
    const { dir, ymlPath } = writeTempYml(`
e2e:
  authoring:
    enabled: true
  reset:
    generate_if_missing: true
    guard_env: MY_GUARD
  control_effects:
    enabled: true
  forbidden_url_patterns:
    - "^https?://prod.example.com"
`);
    try {
        const cfg = await loadAuthoringConfig(ymlPath);
        assert.ok('authoring' in cfg, 'must have authoring key');
        assert.ok('reset' in cfg, 'must have reset key');
        assert.ok('control_effects' in cfg, 'must have control_effects key');
    } finally {
        cleanup(dir);
    }
});
