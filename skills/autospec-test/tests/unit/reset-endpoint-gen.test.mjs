// skills/autospec-test/tests/unit/reset-endpoint-gen.test.mjs
// node --test  (Node.js built-in test runner)
// Tests for scripts/reset-endpoint-gen.mjs
// Covers: detectFramework, generateResetRoute, preflightHostname, resolveResetStrategy

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import fs from 'node:fs';
import os from 'node:os';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const SCRIPTS_DIR = path.resolve(__dirname, '../../scripts');

const { detectFramework, generateResetRoute, preflightHostname, resolveResetStrategy } =
    await import(`file://${SCRIPTS_DIR}/reset-endpoint-gen.mjs`);

// ── Helpers ────────────────────────────────────────────────────────────────────

function tmpDir() {
    return fs.mkdtempSync(path.join(os.tmpdir(), 'reset-gen-test-'));
}

function cleanup(dir) {
    fs.rmSync(dir, { recursive: true, force: true });
}

function writeFile(dir, relPath, content) {
    const full = path.join(dir, relPath);
    fs.mkdirSync(path.dirname(full), { recursive: true });
    fs.writeFileSync(full, content, 'utf8');
    return full;
}

function writePkg(dir, deps = {}, devDeps = {}) {
    writeFile(dir, 'package.json', JSON.stringify({
        name: 'test-app',
        dependencies: deps,
        devDependencies: devDeps,
    }));
}

// ── preflightHostname ─────────────────────────────────────────────────────────

test('preflightHostname: localhost is safe', () => {
    const r = preflightHostname('http://localhost:3000');
    assert.equal(r.ok, true);
    assert.equal(r.reason, null);
});

test('preflightHostname: 127.0.0.1 is safe', () => {
    const r = preflightHostname('http://127.0.0.1:3000/foo');
    assert.equal(r.ok, true);
});

test('preflightHostname: 192.168.x.x is safe', () => {
    const r = preflightHostname('http://192.168.1.100:3000');
    assert.equal(r.ok, true);
});

test('preflightHostname: 10.x.x.x is safe', () => {
    const r = preflightHostname('http://10.0.0.1:8080');
    assert.equal(r.ok, true);
});

test('preflightHostname: vercel.app is not safe', () => {
    const r = preflightHostname('https://myapp.vercel.app');
    assert.equal(r.ok, false);
    assert.ok(r.reason && r.reason.length > 0, 'reason must explain the failure');
});

test('preflightHostname: netlify.app is not safe', () => {
    const r = preflightHostname('https://myapp.netlify.app');
    assert.equal(r.ok, false);
});

test('preflightHostname: prod.example.com is not safe', () => {
    const r = preflightHostname('https://prod.example.com');
    assert.equal(r.ok, false);
});

test('preflightHostname: staging.myapp.com is not safe', () => {
    const r = preflightHostname('https://staging.myapp.com/api');
    assert.equal(r.ok, false);
});

test('preflightHostname: empty string returns ok=false', () => {
    const r = preflightHostname('');
    assert.equal(r.ok, false);
});

test('preflightHostname: invalid URL returns ok=false', () => {
    const r = preflightHostname('not-a-url');
    assert.equal(r.ok, false);
    assert.ok(r.reason && r.reason.length > 0);
});

test('preflightHostname: fly.dev is not safe', () => {
    const r = preflightHostname('https://myapp.fly.dev');
    assert.equal(r.ok, false);
});

test('preflightHostname: herokuapp.com is not safe', () => {
    const r = preflightHostname('https://myapp.herokuapp.com');
    assert.equal(r.ok, false);
});

// ── detectFramework ───────────────────────────────────────────────────────────

test('detectFramework: detects express from package.json', async () => {
    const dir = tmpDir();
    try {
        writePkg(dir, { express: '^4.18.0' });
        const fw = await detectFramework(dir);
        assert.equal(fw.framework, 'express');
        assert.equal(fw.confidence, 'high');
    } finally {
        cleanup(dir);
    }
});

test('detectFramework: detects fastify from package.json', async () => {
    const dir = tmpDir();
    try {
        writePkg(dir, { fastify: '^4.0.0' });
        const fw = await detectFramework(dir);
        assert.equal(fw.framework, 'fastify');
        assert.equal(fw.confidence, 'high');
    } finally {
        cleanup(dir);
    }
});

test('detectFramework: detects nextjs from package.json', async () => {
    const dir = tmpDir();
    try {
        writePkg(dir, { next: '^14.0.0', react: '^18.0.0' });
        const fw = await detectFramework(dir);
        assert.equal(fw.framework, 'nextjs');
        assert.equal(fw.confidence, 'high');
    } finally {
        cleanup(dir);
    }
});

test('detectFramework: detects nextjs app router directory', async () => {
    const dir = tmpDir();
    try {
        writePkg(dir, { next: '^14.0.0' });
        fs.mkdirSync(path.join(dir, 'app', 'api'), { recursive: true });
        const fw = await detectFramework(dir);
        assert.equal(fw.framework, 'nextjs');
        assert.ok(fw.routesDir && fw.routesDir.includes('app/api'),
            `Expected app/api routesDir, got ${fw.routesDir}`);
    } finally {
        cleanup(dir);
    }
});

test('detectFramework: detects express routes directory', async () => {
    const dir = tmpDir();
    try {
        writePkg(dir, { express: '^4.0.0' });
        fs.mkdirSync(path.join(dir, 'routes'), { recursive: true });
        const fw = await detectFramework(dir);
        assert.equal(fw.framework, 'express');
        assert.equal(fw.routesDir, 'routes');
    } finally {
        cleanup(dir);
    }
});

test('detectFramework: returns unknown for empty repo', async () => {
    const dir = tmpDir();
    try {
        const fw = await detectFramework(dir);
        assert.equal(fw.framework, 'unknown');
        assert.equal(fw.confidence, 'low');
    } finally {
        cleanup(dir);
    }
});

test('detectFramework: prefers next over express when both present', async () => {
    const dir = tmpDir();
    try {
        writePkg(dir, { next: '^14.0.0', express: '^4.0.0' });
        const fw = await detectFramework(dir);
        // next check comes first in implementation
        assert.equal(fw.framework, 'nextjs');
    } finally {
        cleanup(dir);
    }
});

// ── generateResetRoute ────────────────────────────────────────────────────────

test('generateResetRoute: throws when guardEnv is empty', async () => {
    const dir = tmpDir();
    try {
        await assert.rejects(
            () => generateResetRoute(dir, { guardEnv: '' }),
            /guardEnv must be a non-empty/
        );
    } finally {
        cleanup(dir);
    }
});

test('generateResetRoute: throws on production hostname', async () => {
    const dir = tmpDir();
    try {
        await assert.rejects(
            () => generateResetRoute(dir, { guardEnv: 'AUTOSPEC_TEST_STACK', baseUrl: 'https://prod.myapp.com' }),
            /pre-flight hostname check failed/
        );
    } finally {
        cleanup(dir);
    }
});

test('generateResetRoute: dry-run does not write file', async () => {
    const dir = tmpDir();
    try {
        writePkg(dir, { express: '^4.0.0' });
        const result = await generateResetRoute(dir, {
            guardEnv: 'AUTOSPEC_TEST_STACK',
            baseUrl: 'http://localhost:3000',
            dryRun: true,
        });
        assert.equal(result.dryRun, true);
        assert.equal(result.generated, false);
        assert.equal(result.filePath, null);
        assert.ok(result.content.length > 0, 'content must be present even in dry-run');
    } finally {
        cleanup(dir);
    }
});

test('generateResetRoute: express — writes file with AUTOSPEC-GENERATED header', async () => {
    const dir = tmpDir();
    try {
        writePkg(dir, { express: '^4.0.0' });
        const result = await generateResetRoute(dir, {
            guardEnv: 'MY_TEST_GUARD',
            baseUrl: 'http://localhost:3000',
        });
        assert.equal(result.generated, true);
        assert.ok(result.filePath, 'filePath must be set');
        assert.ok(fs.existsSync(result.filePath), 'file must exist on disk');
        const content = fs.readFileSync(result.filePath, 'utf8');
        assert.ok(content.includes('AUTOSPEC-GENERATED test-only'), 'must include AUTOSPEC-GENERATED header');
        assert.ok(content.includes('MY_TEST_GUARD'), 'must embed guardEnv name');
        assert.ok(content.includes('404'), 'must have 404 guard');
    } finally {
        cleanup(dir);
    }
});

test('generateResetRoute: fastify — writes file with guard-env check', async () => {
    const dir = tmpDir();
    try {
        writePkg(dir, { fastify: '^4.0.0' });
        const result = await generateResetRoute(dir, {
            guardEnv: 'AUTOSPEC_TEST_STACK',
            baseUrl: 'http://localhost:3000',
        });
        const content = fs.readFileSync(result.filePath, 'utf8');
        assert.ok(content.includes('AUTOSPEC-GENERATED test-only'));
        assert.ok(content.includes('AUTOSPEC_TEST_STACK'));
        assert.equal(result.framework, 'fastify');
    } finally {
        cleanup(dir);
    }
});

test('generateResetRoute: nextjs — writes file with guard-env check', async () => {
    const dir = tmpDir();
    try {
        writePkg(dir, { next: '^14.0.0' });
        const result = await generateResetRoute(dir, {
            guardEnv: 'AUTOSPEC_TEST_STACK',
            baseUrl: 'http://localhost:3000',
        });
        const content = fs.readFileSync(result.filePath, 'utf8');
        assert.ok(content.includes('AUTOSPEC-GENERATED test-only'));
        assert.ok(content.includes('AUTOSPEC_TEST_STACK'));
        assert.equal(result.framework, 'nextjs');
    } finally {
        cleanup(dir);
    }
});

test('generateResetRoute: unknown framework — writes generic handler with warning', async () => {
    const dir = tmpDir();
    try {
        const result = await generateResetRoute(dir, {
            guardEnv: 'AUTOSPEC_TEST_STACK',
            baseUrl: 'http://localhost:3000',
        });
        assert.equal(result.framework, 'unknown');
        assert.ok(result.warning && result.warning.length > 0, 'must emit warning for unknown framework');
        const content = fs.readFileSync(result.filePath, 'utf8');
        assert.ok(content.includes('AUTOSPEC-GENERATED test-only'));
    } finally {
        cleanup(dir);
    }
});

test('generateResetRoute: pre-detected frameworkInfo skips detection', async () => {
    const dir = tmpDir();
    try {
        // No package.json — would be unknown, but we provide frameworkInfo
        const result = await generateResetRoute(dir, {
            guardEnv: 'AUTOSPEC_TEST_STACK',
            baseUrl: 'http://localhost:3000',
            frameworkInfo: { framework: 'express', routesDir: 'routes', mainFile: null, confidence: 'high' },
        });
        assert.equal(result.framework, 'express');
    } finally {
        cleanup(dir);
    }
});

test('generateResetRoute: created directories recursively', async () => {
    const dir = tmpDir();
    try {
        writePkg(dir, { express: '^4.0.0' });
        fs.mkdirSync(path.join(dir, 'routes'), { recursive: true });
        const result = await generateResetRoute(dir, {
            guardEnv: 'AUTOSPEC_TEST_STACK',
            baseUrl: 'http://localhost:3000',
        });
        assert.ok(fs.existsSync(path.dirname(result.filePath)),
            'parent directories must be created');
    } finally {
        cleanup(dir);
    }
});

// ── resolveResetStrategy ──────────────────────────────────────────────────────

test('resolveResetStrategy: declared endpoint takes priority', async () => {
    const dir = tmpDir();
    const strategy = await resolveResetStrategy(
        { endpoint: '/api/test/reset', guard_env: 'AUTOSPEC_TEST_STACK' },
        dir
    );
    assert.equal(strategy.kind, 'declared_endpoint');
    assert.equal(strategy.endpoint, '/api/test/reset');
    assert.equal(strategy.cmd, null);
});

test('resolveResetStrategy: declared cmd taken when no endpoint', async () => {
    const dir = tmpDir();
    const strategy = await resolveResetStrategy(
        { cmd: 'make db-reset' },
        dir
    );
    assert.equal(strategy.kind, 'declared_cmd');
    assert.equal(strategy.cmd, 'make db-reset');
    assert.equal(strategy.endpoint, null);
});

test('resolveResetStrategy: generate_if_missing=true → generated strategy', async () => {
    const dir = tmpDir();
    const strategy = await resolveResetStrategy(
        { generate_if_missing: true, guard_env: 'AUTOSPEC_TEST_STACK' },
        dir
    );
    assert.equal(strategy.kind, 'generated');
    assert.equal(strategy.endpoint, '/api/test/reset');
    assert.equal(strategy.guardEnv, 'AUTOSPEC_TEST_STACK');
});

test('resolveResetStrategy: generate_if_missing=true without guard_env → clone_isolation fallback', async () => {
    const dir = tmpDir();
    const strategy = await resolveResetStrategy(
        { generate_if_missing: true },
        dir
    );
    assert.equal(strategy.kind, 'clone_isolation');
    assert.ok(strategy.description.length > 0);
});

test('resolveResetStrategy: no config → clone_isolation fallback', async () => {
    const dir = tmpDir();
    const strategy = await resolveResetStrategy(null, dir);
    assert.equal(strategy.kind, 'clone_isolation');
});

test('resolveResetStrategy: empty config → clone_isolation fallback', async () => {
    const dir = tmpDir();
    const strategy = await resolveResetStrategy({}, dir);
    assert.equal(strategy.kind, 'clone_isolation');
});

test('resolveResetStrategy: description is always a non-empty string', async () => {
    const dir = tmpDir();
    for (const cfg of [
        null,
        {},
        { endpoint: '/api/test/reset' },
        { cmd: 'make reset' },
        { generate_if_missing: true, guard_env: 'GUARD' },
    ]) {
        const s = await resolveResetStrategy(cfg, dir);
        assert.ok(typeof s.description === 'string' && s.description.length > 0,
            `description must be non-empty for cfg=${JSON.stringify(cfg)}`);
    }
});

test('resolveResetStrategy: fallback chain order — endpoint > cmd > generate > clone', async () => {
    const dir = tmpDir();
    // endpoint wins even when cmd and generate_if_missing are also set
    const s = await resolveResetStrategy(
        { endpoint: '/api/test/reset', cmd: 'make reset', generate_if_missing: true, guard_env: 'GUARD' },
        dir
    );
    assert.equal(s.kind, 'declared_endpoint');
});
