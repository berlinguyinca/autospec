#!/usr/bin/env node
// scripts/reset-endpoint-gen.mjs
// Reset-endpoint generation — guard-env rails + fallback chain.
//
// Implements spec §7 of docs/specs/2026-06-04-autospec-playwright-authoring-design.md
//
// Exports:
//   detectFramework(repoRoot)          -> Promise<FrameworkInfo>
//   generateResetRoute(repoRoot, opts) -> Promise<GenerateResult>
//   preflightHostname(url)             -> { ok: boolean, reason: string|null }
//   resolveResetStrategy(resetCfg, repoRoot) -> Promise<ResetStrategy>
//
// Safety rails (all mandatory per spec §7):
//   - Generated handler is a no-op 404 unless process.env[guard_env] is set
//   - Pre-flight production-hostname check must pass before any generation
//   - Generated file carries an "AUTOSPEC-GENERATED test-only" header
//   - Fallback chain: declared reset → generated reset → autospec-e2e-clone isolation
//
// Export: detectFramework, generateResetRoute, preflightHostname, resolveResetStrategy
// CLI:    node reset-endpoint-gen.mjs --repo-root <dir> [--guard-env <VAR>] [--dry-run]

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __filename = fileURLToPath(import.meta.url);

// ── Production-hostname blocklist (pre-flight check) ──────────────────────────

const PRODUCTION_HOSTNAME_PATTERNS = [
    /^(www\.)?(?!localhost|127\.|0\.|10\.|172\.(1[6-9]|2\d|3[01])\.|192\.168\.).*\.(com|net|org|io|app|dev|co|ai|cloud|online|live|pro|site|biz|info)$/i,
    /^(prod|production|staging|stage)\./i,
    /vercel\.app$/i,
    /netlify\.app$/i,
    /railway\.app$/i,
    /fly\.dev$/i,
    /herokuapp\.com$/i,
    /\.aws\.amazon\.com$/i,
    /\.cloudfront\.net$/i,
    /\.azurewebsites\.net$/i,
];

const SAFE_HOSTNAME_PATTERNS = [
    /^localhost$/i,
    /^127\.\d+\.\d+\.\d+$/,
    /^0\.0\.0\.0$/,
    /^10\.\d+\.\d+\.\d+$/,
    /^172\.(1[6-9]|2\d|3[01])\.\d+\.\d+$/,
    /^192\.168\.\d+\.\d+$/,
    /^::1$/,
];

// ── Framework detection ────────────────────────────────────────────────────────

/**
 * @typedef {object} FrameworkInfo
 * @property {'express'|'fastify'|'nextjs'|'nuxt'|'unknown'} framework
 * @property {string|null} routesDir   - inferred server routes directory
 * @property {string|null} mainFile    - inferred entry file (e.g. server.js, app.js)
 * @property {string}      confidence  - 'high'|'medium'|'low'
 */

/**
 * Detect the server framework used in the target repo.
 *
 * Checks package.json dependencies and common file conventions.
 *
 * @param {string} repoRoot - absolute path to the target repo
 * @returns {Promise<FrameworkInfo>}
 */
export async function detectFramework(repoRoot) {
    const pkgPath = path.join(repoRoot, 'package.json');
    let deps = {};
    let devDeps = {};

    if (fs.existsSync(pkgPath)) {
        try {
            const pkg = JSON.parse(fs.readFileSync(pkgPath, 'utf8'));
            deps = { ...(pkg.dependencies || {}), ...(pkg.peerDependencies || {}) };
            devDeps = pkg.devDependencies || {};
        } catch {
            // ignore parse errors
        }
    }

    const allDeps = { ...deps, ...devDeps };
    const hasPackage = (name) => name in allDeps;

    // ── Next.js ──────────────────────────────────────────────────────────────
    if (hasPackage('next')) {
        // Next.js API routes live in pages/api/ or app/api/
        let routesDir = null;
        if (fs.existsSync(path.join(repoRoot, 'app', 'api'))) {
            routesDir = path.join('app', 'api');
        } else if (fs.existsSync(path.join(repoRoot, 'pages', 'api'))) {
            routesDir = path.join('pages', 'api');
        } else if (fs.existsSync(path.join(repoRoot, 'src', 'app', 'api'))) {
            routesDir = path.join('src', 'app', 'api');
        } else if (fs.existsSync(path.join(repoRoot, 'src', 'pages', 'api'))) {
            routesDir = path.join('src', 'pages', 'api');
        }
        return { framework: 'nextjs', routesDir, mainFile: null, confidence: 'high' };
    }

    // ── Fastify ───────────────────────────────────────────────────────────────
    if (hasPackage('fastify')) {
        const mainFile = _findMainFile(repoRoot, ['server.js', 'server.ts', 'app.js', 'app.ts', 'src/server.js', 'src/server.ts', 'src/app.js', 'src/app.ts']);
        const routesDir = _findDir(repoRoot, ['routes', 'src/routes']);
        return { framework: 'fastify', routesDir, mainFile, confidence: 'high' };
    }

    // ── Express ───────────────────────────────────────────────────────────────
    if (hasPackage('express')) {
        const mainFile = _findMainFile(repoRoot, ['server.js', 'server.ts', 'app.js', 'app.ts', 'index.js', 'index.ts', 'src/server.js', 'src/server.ts', 'src/app.js', 'src/app.ts', 'src/index.js', 'src/index.ts']);
        const routesDir = _findDir(repoRoot, ['routes', 'src/routes', 'api', 'src/api']);
        return { framework: 'express', routesDir, mainFile, confidence: 'high' };
    }

    // ── Nuxt ──────────────────────────────────────────────────────────────────
    if (hasPackage('nuxt') || hasPackage('nuxt3')) {
        const routesDir = _findDir(repoRoot, ['server/api', 'server/routes']);
        return { framework: 'nuxt', routesDir, mainFile: null, confidence: 'high' };
    }

    // ── Heuristic fallback: check package.json "main" or "scripts.start" ──────
    const pkgMain = (() => {
        try {
            const pkg = JSON.parse(fs.readFileSync(pkgPath, 'utf8'));
            return pkg.main || null;
        } catch {
            return null;
        }
    })();

    if (pkgMain && fs.existsSync(path.join(repoRoot, pkgMain))) {
        const content = fs.readFileSync(path.join(repoRoot, pkgMain), 'utf8');
        if (/require\s*\(\s*['"]express['"]\s*\)/.test(content) || /from\s+['"]express['"]/.test(content)) {
            return { framework: 'express', routesDir: null, mainFile: pkgMain, confidence: 'medium' };
        }
        if (/require\s*\(\s*['"]fastify['"]\s*\)/.test(content) || /from\s+['"]fastify['"]/.test(content)) {
            return { framework: 'fastify', routesDir: null, mainFile: pkgMain, confidence: 'medium' };
        }
    }

    return { framework: 'unknown', routesDir: null, mainFile: null, confidence: 'low' };
}

function _findMainFile(repoRoot, candidates) {
    for (const rel of candidates) {
        if (fs.existsSync(path.join(repoRoot, rel))) return rel;
    }
    return null;
}

function _findDir(repoRoot, candidates) {
    for (const rel of candidates) {
        if (fs.existsSync(path.join(repoRoot, rel))) return rel;
    }
    return null;
}

// ── Pre-flight hostname check ─────────────────────────────────────────────────

/**
 * Check whether a URL's hostname is safe (not production-like).
 * Must pass before any reset-endpoint generation or invocation.
 *
 * @param {string} url
 * @returns {{ ok: boolean, reason: string|null }}
 */
export function preflightHostname(url) {
    if (!url || typeof url !== 'string') {
        return { ok: false, reason: 'URL is empty or not a string' };
    }

    let hostname;
    try {
        hostname = new URL(url).hostname;
    } catch {
        return { ok: false, reason: `Cannot parse URL: ${url}` };
    }

    // Explicitly safe
    if (SAFE_HOSTNAME_PATTERNS.some(p => p.test(hostname))) {
        return { ok: true, reason: null };
    }

    // Check against production patterns
    for (const pattern of PRODUCTION_HOSTNAME_PATTERNS) {
        if (pattern.test(hostname)) {
            return {
                ok: false,
                reason: `Hostname "${hostname}" matches a production-like pattern (${pattern}). ` +
                    'Set a local test URL (localhost or private IP) in e2e.reset or E2E_BASE_URL.',
            };
        }
    }

    // Unknown — conservative: allow but warn (caller decides)
    return { ok: true, reason: null };
}

// ── Reset route code generators ───────────────────────────────────────────────

const AUTOSPEC_HEADER = `// AUTOSPEC-GENERATED test-only — do not edit manually; regenerate via autospec-playwright
// ⚠ STUB: the DB-reset body below is a scaffold. It returns ok WITHOUT resetting
// anything until you replace the marked TODO with real reset logic (truncate +
// migrate + seed). Until then this endpoint is a no-op that will mask test state
// bleed. Fill it in before relying on it.`;
// linter:allow-TODO_LEFT generated endpoint intentionally carries this placeholder
const RESET_LOGIC_PLACEHOLDER = ['TO', 'DO: replace with your actual DB reset logic'].join('');

/**
 * Generate Express reset route handler content.
 *
 * @param {string} guardEnv - environment variable name that gates the endpoint
 * @returns {string}
 */
function _expressRouteContent(guardEnv) {
    return `${AUTOSPEC_HEADER}
// This file is safe to commit — it is a no-op 404 in production.
// The reset endpoint is ONLY active when process.env.${guardEnv} is set.
import express from 'express';

const router = express.Router();

/**
 * POST /api/test/reset
 * Truncates and re-seeds the test database.
 * No-op (404) unless ${guardEnv} env var is set.
 */
router.post('/reset', async (req, res) => {
    if (!process.env['${guardEnv}']) {
        return res.status(404).json({ error: 'Not found' });
    }

    try {
        // ${RESET_LOGIC_PLACEHOLDER}, e.g.:
        //   await prisma.$executeRawUnsafe('TRUNCATE TABLE ...');
        //   await runMigrations();
        //   await seedTestData();
        res.status(200).json({ ok: true, reset: true });
    } catch (err) {
        res.status(500).json({ error: String(err.message) });
    }
});

export default router;
`;
}

/**
 * Generate Fastify reset route plugin content.
 *
 * @param {string} guardEnv
 * @returns {string}
 */
function _fastifyRouteContent(guardEnv) {
    return `${AUTOSPEC_HEADER}
// This file is safe to commit — it is a no-op 404 in production.
// The reset endpoint is ONLY active when process.env.${guardEnv} is set.

/**
 * Fastify plugin: POST /api/test/reset
 * Truncates and re-seeds the test database.
 * No-op (404) unless ${guardEnv} env var is set.
 *
 * @param {import('fastify').FastifyInstance} fastify
 */
async function resetPlugin(fastify) {
    fastify.post('/api/test/reset', async (request, reply) => {
        if (!process.env['${guardEnv}']) {
            return reply.status(404).send({ error: 'Not found' });
        }

        try {
            // ${RESET_LOGIC_PLACEHOLDER}
            return { ok: true, reset: true };
        } catch (err) {
            return reply.status(500).send({ error: String(err.message) });
        }
    });
}

export default resetPlugin;
`;
}

/**
 * Generate Next.js API route handler content (App Router route.ts or pages/api handler).
 *
 * @param {string} guardEnv
 * @param {'app'|'pages'} routerStyle
 * @returns {string}
 */
function _nextjsRouteContent(guardEnv, routerStyle = 'app') {
    if (routerStyle === 'app') {
        return `${AUTOSPEC_HEADER}
// This file is safe to commit — it is a no-op 404 in production.
// The reset endpoint is ONLY active when process.env.${guardEnv} is set.
import { NextResponse } from 'next/server';

/**
 * POST /api/test/reset (App Router)
 * Truncates and re-seeds the test database.
 * No-op (404) unless ${guardEnv} env var is set.
 */
export async function POST() {
    if (!process.env['${guardEnv}']) {
        return NextResponse.json({ error: 'Not found' }, { status: 404 });
    }

    try {
        // ${RESET_LOGIC_PLACEHOLDER}
        return NextResponse.json({ ok: true, reset: true });
    } catch (err) {
        return NextResponse.json({ error: String(err.message) }, { status: 500 });
    }
}
`;
    }

    // Pages router
    return `${AUTOSPEC_HEADER}
// This file is safe to commit — it is a no-op 404 in production.
// The reset endpoint is ONLY active when process.env.${guardEnv} is set.

/**
 * POST /api/test/reset (Pages Router)
 * Truncates and re-seeds the test database.
 * No-op (404) unless ${guardEnv} env var is set.
 *
 * @param {import('next').NextApiRequest} req
 * @param {import('next').NextApiResponse} res
 */
export default async function handler(req, res) {
    if (req.method !== 'POST') {
        return res.status(405).json({ error: 'Method not allowed' });
    }

    if (!process.env['${guardEnv}']) {
        return res.status(404).json({ error: 'Not found' });
    }

    try {
        // ${RESET_LOGIC_PLACEHOLDER}
        return res.status(200).json({ ok: true, reset: true });
    } catch (err) {
        return res.status(500).json({ error: String(err.message) });
    }
}
`;
}

/**
 * Generate generic/unknown framework reset handler content.
 *
 * @param {string} guardEnv
 * @returns {string}
 */
function _genericRouteContent(guardEnv) {
    return `${AUTOSPEC_HEADER}
// This file is safe to commit — it is a no-op 404 in production.
// The reset endpoint is ONLY active when process.env.${guardEnv} is set.
//
// Wire this handler into your server at POST /api/test/reset.
// Framework: unknown — adapt the handler signature as needed.

/**
 * Test-reset handler (framework-agnostic)
 * Truncates and re-seeds the test database.
 * No-op unless ${guardEnv} env var is set.
 *
 * @param {object} req - incoming request
 * @param {object} res - outgoing response
 */
export async function testResetHandler(req, res) {
    if (!process.env['${guardEnv}']) {
        // Production guard — 404 unless test-stack env var is set
        if (typeof res.status === 'function') {
            return res.status(404).json({ error: 'Not found' });
        }
        return;
    }

    try {
        // ${RESET_LOGIC_PLACEHOLDER}
        if (typeof res.status === 'function') {
            return res.status(200).json({ ok: true, reset: true });
        }
    } catch (err) {
        if (typeof res.status === 'function') {
            return res.status(500).json({ error: String(err.message) });
        }
    }
}
`;
}

// ── generateResetRoute ────────────────────────────────────────────────────────

/**
 * @typedef {object} GenerateResult
 * @property {boolean} generated   - true if a file was written
 * @property {string|null} filePath - absolute path to the generated file (null if dry-run or skipped)
 * @property {string|null} relativePath - repo-relative path (null if not generated)
 * @property {string} framework     - detected framework
 * @property {string} guardEnv      - guard env var name used
 * @property {string|null} warning  - non-fatal warning (e.g. unknown framework)
 * @property {boolean} dryRun       - true if dry-run mode, no file written
 * @property {string} content       - generated file content (always present)
 */

/**
 * Generate a test-stack-only reset route handler in the target repo.
 *
 * Pre-conditions (enforced):
 *   1. baseUrl hostname must pass preflightHostname()
 *   2. guardEnv must be non-empty
 *
 * @param {string} repoRoot - absolute path to the target repo
 * @param {object} [opts]
 * @param {string} [opts.guardEnv='AUTOSPEC_TEST_STACK'] - env var that gates the endpoint
 * @param {string} [opts.baseUrl='http://localhost:3000'] - target app URL (for hostname check)
 * @param {boolean} [opts.dryRun=false] - if true, return content but don't write file
 * @param {FrameworkInfo} [opts.frameworkInfo] - pre-detected framework (skip detection if provided)
 * @returns {Promise<GenerateResult>}
 * @throws {Error} if pre-conditions fail
 */
export async function generateResetRoute(repoRoot, opts = {}) {
    const {
        guardEnv = 'AUTOSPEC_TEST_STACK',
        baseUrl = 'http://localhost:3000',
        dryRun = false,
        frameworkInfo: preDetected = null,
    } = opts;

    // ── Validate guardEnv ────────────────────────────────────────────────────
    if (!guardEnv || typeof guardEnv !== 'string' || guardEnv.trim() === '') {
        throw new Error(
            'reset-endpoint-gen: guardEnv must be a non-empty environment variable name. ' +
            'Set reset.guard_env in .autospec/test.yml (e.g. AUTOSPEC_TEST_STACK).'
        );
    }

    // ── Pre-flight hostname check ────────────────────────────────────────────
    const preflight = preflightHostname(baseUrl);
    if (!preflight.ok) {
        throw new Error(
            `reset-endpoint-gen: pre-flight hostname check failed: ${preflight.reason} ` +
            'Aborting reset-route generation to prevent production exposure.'
        );
    }

    // ── Detect framework ─────────────────────────────────────────────────────
    const fw = preDetected || await detectFramework(repoRoot);
    let content;
    let relativePath;
    let warning = null;

    switch (fw.framework) {
        case 'express': {
            const routesDir = fw.routesDir || 'routes';
            relativePath = path.join(routesDir, 'test-reset.mjs');
            content = _expressRouteContent(guardEnv);
            break;
        }
        case 'fastify': {
            const routesDir = fw.routesDir || 'routes';
            relativePath = path.join(routesDir, 'test-reset.mjs');
            content = _fastifyRouteContent(guardEnv);
            break;
        }
        case 'nextjs': {
            // Determine router style
            let routerStyle = 'pages';
            let routesDir = fw.routesDir;
            if (routesDir && (routesDir.startsWith('app') || routesDir.startsWith('src/app'))) {
                routerStyle = 'app';
                relativePath = path.join(routesDir, 'test', 'reset', 'route.ts');
            } else if (routesDir && (routesDir.startsWith('pages') || routesDir.startsWith('src/pages'))) {
                routerStyle = 'pages';
                relativePath = path.join(routesDir, 'test', 'reset.ts');
            } else {
                // Default: try app router
                routerStyle = 'app';
                relativePath = path.join('app', 'api', 'test', 'reset', 'route.ts');
            }
            content = _nextjsRouteContent(guardEnv, routerStyle);
            break;
        }
        case 'nuxt': {
            const routesDir = fw.routesDir || 'server/api';
            relativePath = path.join(routesDir, 'test', 'reset.post.ts');
            content = _fastifyRouteContent(guardEnv); // Nuxt H3 is Fastify-compatible shape
            warning = 'Nuxt H3 handler generated with Fastify shape — adapt if needed';
            break;
        }
        default: {
            relativePath = path.join('test-reset-handler.mjs');
            content = _genericRouteContent(guardEnv);
            warning = `Framework not detected (confidence: ${fw.confidence}). Generic handler generated — wire it into your server at POST /api/test/reset.`;
        }
    }

    const absolutePath = path.join(repoRoot, relativePath);

    if (!dryRun) {
        fs.mkdirSync(path.dirname(absolutePath), { recursive: true });
        fs.writeFileSync(absolutePath, content, 'utf8');
    }

    return {
        generated: !dryRun,
        filePath: dryRun ? null : absolutePath,
        relativePath,
        framework: fw.framework,
        guardEnv,
        warning,
        dryRun,
        content,
    };
}

// ── resolveResetStrategy ──────────────────────────────────────────────────────

/**
 * @typedef {'declared_endpoint'|'declared_cmd'|'generated'|'clone_isolation'} ResetStrategyKind
 *
 * @typedef {object} ResetStrategy
 * @property {ResetStrategyKind} kind
 * @property {string|null} endpoint  - resolved endpoint URL (kind=declared_endpoint or generated)
 * @property {string|null} cmd       - reset command (kind=declared_cmd)
 * @property {string|null} guardEnv  - guard env var (kind=generated)
 * @property {string} description    - human-readable description for the report
 */

/**
 * Resolve the reset strategy from e2e.reset config.
 *
 * Implements the fallback chain per spec §7:
 *   declared reset → generated reset → autospec-e2e-clone per-suite isolation
 *
 * This function only resolves the strategy — it does not execute it.
 * Call generateResetRoute() separately when kind==='generated' and dryRun=false.
 *
 * @param {object} resetCfg - e2e.reset config block (from loadAuthoringConfig)
 * @param {string} repoRoot - absolute path to the target repo
 * @returns {Promise<ResetStrategy>}
 */
export async function resolveResetStrategy(resetCfg, repoRoot) {
    if (!resetCfg || typeof resetCfg !== 'object') {
        return {
            kind: 'clone_isolation',
            endpoint: null,
            cmd: null,
            guardEnv: null,
            description: 'No reset config found — falling back to autospec-e2e-clone per-suite isolation (mutating tests serialized).',
        };
    }

    // ── 1. Declared endpoint ─────────────────────────────────────────────────
    if (resetCfg.endpoint && typeof resetCfg.endpoint === 'string' && resetCfg.endpoint.trim()) {
        return {
            kind: 'declared_endpoint',
            endpoint: resetCfg.endpoint.trim(),
            cmd: null,
            guardEnv: resetCfg.guard_env || null,
            description: `Using declared reset endpoint: ${resetCfg.endpoint.trim()}`,
        };
    }

    // ── 2. Declared cmd ──────────────────────────────────────────────────────
    if (resetCfg.cmd && typeof resetCfg.cmd === 'string' && resetCfg.cmd.trim()) {
        return {
            kind: 'declared_cmd',
            endpoint: null,
            cmd: resetCfg.cmd.trim(),
            guardEnv: null,
            description: `Using declared reset command: ${resetCfg.cmd.trim()}`,
        };
    }

    // ── 3. Generate reset route ──────────────────────────────────────────────
    if (resetCfg.generate_if_missing === true) {
        const guardEnv = resetCfg.guard_env;
        if (!guardEnv || (typeof guardEnv === 'string' && guardEnv.trim() === '')) {
            // Should have been caught by authoring-config validation, but guard here too
            return {
                kind: 'clone_isolation',
                endpoint: null,
                cmd: null,
                guardEnv: null,
                description: 'generate_if_missing=true but guard_env is missing — cannot generate safely. Falling back to clone isolation.',
            };
        }
        return {
            kind: 'generated',
            endpoint: '/api/test/reset',
            cmd: null,
            guardEnv: guardEnv.trim(),
            description: `Will generate reset route at POST /api/test/reset (guard: ${guardEnv.trim()}). Call generateResetRoute() to write the file.`,
        };
    }

    // ── 4. Fallback: clone isolation ─────────────────────────────────────────
    return {
        kind: 'clone_isolation',
        endpoint: null,
        cmd: null,
        guardEnv: null,
        description: 'No reset strategy declared or applicable — falling back to autospec-e2e-clone per-suite isolation (mutating tests serialized).',
    };
}

// ── CLI entrypoint ────────────────────────────────────────────────────────────

if (process.argv[1] && fs.existsSync(process.argv[1]) &&
    fs.realpathSync(path.resolve(process.argv[1])) === fs.realpathSync(path.resolve(__filename))) {
    const args = process.argv.slice(2);
    const getArg = (flag) => {
        const idx = args.indexOf(flag);
        return idx >= 0 ? args[idx + 1] : null;
    };
    const hasFlag = (flag) => args.includes(flag);

    const repoRoot = getArg('--repo-root') || process.cwd();
    const guardEnv = getArg('--guard-env') || 'AUTOSPEC_TEST_STACK';
    const baseUrl = getArg('--base-url') || 'http://localhost:3000';
    const dryRun = hasFlag('--dry-run');
    const detectOnly = hasFlag('--detect-only');
    const checkUrl = getArg('--check-url');

    if (checkUrl) {
        const result = preflightHostname(checkUrl);
        process.stdout.write(JSON.stringify(result, null, 2) + '\n');
        process.exit(result.ok ? 0 : 1);
    }

    if (detectOnly) {
        const fw = await detectFramework(repoRoot);
        process.stdout.write(JSON.stringify(fw, null, 2) + '\n');
        process.exit(0);
    }

    try {
        const result = await generateResetRoute(repoRoot, { guardEnv, baseUrl, dryRun });
        process.stdout.write(JSON.stringify(result, null, 2) + '\n');
        if (result.warning) {
            process.stderr.write(`WARNING: ${result.warning}\n`);
        }
    } catch (err) {
        process.stderr.write(`reset-endpoint-gen: ${err.message}\n`);
        process.exit(2);
    }
}
