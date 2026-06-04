#!/usr/bin/env node
// scripts/stage2a-orchestrator.mjs
// Stage 2A orchestration (#996) — clustering, helper centralization, parse gate,
// coverage report. Drives the disciplined Playwright-authoring stage.
//
// Exports:
//   clusterRoutes(routes, {fanout_max, route_clusters}) -> [{name, routes}]
//       auto-cluster crawler routes by first path segment, cap at fanout_max,
//       or honor an explicit route_clusters override.
//   centralizeHelpers(helpersDir, {files}) -> {created, skipped, conflicts}
//       idempotent single-writer creation of shared helper files. Never clobbers
//       a divergent existing file (authors treat helpers as read-only).
//   parseGate(specDir, {playwrightBin}) -> {cmd, args}
//       the `playwright test --list` invocation shape used as a parse gate.
//   coverageReport(crawlerManifest, coveredRoutes, repoRoot) -> {total, covered, pct}
//       {total,covered,pct} from the crawler manifest denominator (NOT author
//       self-report), written to e2e/.autospec/coverage.json.
//
// Reuses (does not fork): selector-evidence.mjs (resolver), ui-crawler.mjs
// manifest shape, authoring-config.mjs loader.
//
// CLI: node stage2a-orchestrator.mjs coverage --manifest <file> --covered <a,b,..> [--repo-root <dir>]
//      node stage2a-orchestrator.mjs cluster  --routes <a,b,..> [--fanout-max N]

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __filename = fileURLToPath(import.meta.url);

const COVERAGE_REL_PATH = path.join('e2e', '.autospec', 'coverage.json');
const DEFAULT_FANOUT_MAX = 4;

// ── clusterRoutes ────────────────────────────────────────────────────────────────

/**
 * Cluster crawler-discovered routes into author-assignable groups.
 *
 * Auto mode: group by first path segment ('/' -> 'root'). If the number of
 * groups exceeds fanout_max, the smallest groups are merged into a shared
 * 'misc' cluster until the cap is met (every route stays assigned exactly once).
 *
 * Explicit override: when route_clusters is an array of {name, routes}, it is
 * returned verbatim (normalized) — authors get exactly those clusters.
 *
 * @param {string[]} routes
 * @param {object} [opts]
 * @param {number} [opts.fanout_max=4] - max concurrent author subagents / clusters
 * @param {'auto'|Array<{name:string,routes:string[]}>} [opts.route_clusters='auto']
 * @returns {Array<{name: string, routes: string[]}>}
 */
export function clusterRoutes(routes = [], opts = {}) {
    const fanoutMax = Number.isInteger(opts.fanout_max) && opts.fanout_max > 0
        ? opts.fanout_max
        : DEFAULT_FANOUT_MAX;
    const routeClusters = opts.route_clusters === undefined ? 'auto' : opts.route_clusters;

    // ── Explicit override ──────────────────────────────────────────────────────
    if (Array.isArray(routeClusters)) {
        return routeClusters.map(c => ({
            name: String(c.name),
            routes: Array.isArray(c.routes) ? c.routes.slice() : [],
        }));
    }

    // ── Auto clustering by first path segment ──────────────────────────────────
    const groups = new Map();
    for (const route of routes) {
        const name = firstSegment(route);
        if (!groups.has(name)) groups.set(name, []);
        groups.get(name).push(route);
    }

    let clusters = Array.from(groups.entries()).map(([name, rs]) => ({ name, routes: rs }));

    // ── Cap at fanout_max by merging smallest groups into 'misc' ───────────────
    if (clusters.length > fanoutMax) {
        // Sort by size descending; keep the largest (fanoutMax - 1), merge the rest.
        clusters.sort((a, b) => b.routes.length - a.routes.length);
        const keep = clusters.slice(0, fanoutMax - 1);
        const overflow = clusters.slice(fanoutMax - 1);
        const miscRoutes = overflow.flatMap(c => c.routes);
        keep.push({ name: 'misc', routes: miscRoutes });
        clusters = keep;
    }

    return clusters;
}

/**
 * First path segment of a route, used as the cluster name.
 * '/' or '' -> 'root'; '/users/1' -> 'users'.
 * @param {string} route
 * @returns {string}
 */
function firstSegment(route) {
    const trimmed = String(route).split('?')[0].split('#')[0];
    const seg = trimmed.replace(/^\/+/, '').split('/')[0];
    return seg === '' ? 'root' : seg;
}

// ── centralizeHelpers ────────────────────────────────────────────────────────────

/**
 * Idempotently create shared helper files under helpersDir.
 *
 * Single-writer discipline: the orchestrator is the only writer of helper files.
 * - Missing file        -> created.
 * - Existing, identical  -> skipped (idempotent).
 * - Existing, divergent  -> conflict reported, file LEFT UNTOUCHED (never clobbered).
 *
 * @param {string} helpersDir - absolute path to e2e/helpers
 * @param {object} opts
 * @param {Object<string,string>} opts.files - {relativeName: content}
 * @returns {{created: string[], skipped: string[], conflicts: string[]}}
 */
export function centralizeHelpers(helpersDir, opts = {}) {
    const files = opts.files || {};
    const created = [];
    const skipped = [];
    const conflicts = [];

    fs.mkdirSync(helpersDir, { recursive: true });

    for (const [name, content] of Object.entries(files)) {
        const target = path.join(helpersDir, name);
        if (fs.existsSync(target)) {
            const existing = fs.readFileSync(target, 'utf8');
            if (existing === content) {
                skipped.push(name);
            } else {
                conflicts.push(name);
            }
            continue;
        }
        fs.mkdirSync(path.dirname(target), { recursive: true });
        fs.writeFileSync(target, content, 'utf8');
        created.push(name);
    }

    return { created, skipped, conflicts };
}

// ── parseGate ────────────────────────────────────────────────────────────────────

/**
 * Build the `playwright test --list` invocation used as a parse gate: it
 * compiles + enumerates every test without running them, catching syntax /
 * import errors before the (expensive) real run.
 *
 * Returns a {cmd, args} pair suitable for child_process.spawn/execFile.
 *
 * @param {string} specDir - directory of authored spec files (e.g. e2e/specs)
 * @param {object} [opts]
 * @param {string} [opts.playwrightBin='npx playwright'] - launcher
 * @returns {{cmd: string, args: string[]}}
 */
export function parseGate(specDir, opts = {}) {
    const bin = (opts.playwrightBin && String(opts.playwrightBin).trim()) || 'npx playwright';
    const parts = bin.split(/\s+/).filter(Boolean);
    const cmd = parts[0];
    const binArgs = parts.slice(1);
    return {
        cmd,
        args: [...binArgs, 'test', '--list', specDir],
    };
}

// ── coverageReport ───────────────────────────────────────────────────────────────

/**
 * Compute {total, covered, pct} where `total` is the crawler manifest route
 * denominator (NOT author self-report) and `covered` is the count of crawler
 * routes the authored specs actually exercised. `covered` is intersected with
 * the known route set so a lying author cannot inflate coverage past 100%.
 *
 * Writes the report to <repoRoot>/e2e/.autospec/coverage.json (snake_case keys).
 *
 * @param {{routes?: string[], manifest?: Array}} crawlerManifest
 * @param {string[]} coveredRoutes - routes the authored specs exercised
 * @param {string} repoRoot
 * @returns {{total: number, covered: number, pct: number}}
 */
export function coverageReport(crawlerManifest, coveredRoutes = [], repoRoot = process.cwd()) {
    const knownRoutes = Array.isArray(crawlerManifest && crawlerManifest.routes)
        ? crawlerManifest.routes
        : [];
    const known = new Set(knownRoutes);
    const total = known.size;

    // Intersect covered with known routes — denominator authority is the crawler.
    const coveredSet = new Set();
    for (const r of coveredRoutes) {
        if (known.has(r)) coveredSet.add(r);
    }
    const covered = coveredSet.size;

    const pct = total === 0 ? 0 : Math.round((covered / total) * 100);

    const report = { total, covered, pct };

    const outPath = path.join(repoRoot, COVERAGE_REL_PATH);
    fs.mkdirSync(path.dirname(outPath), { recursive: true });
    fs.writeFileSync(outPath, JSON.stringify(report, null, 2) + '\n', 'utf8');

    return report;
}

// ── CLI entrypoint ─────────────────────────────────────────────────────────────

if (process.argv[1] && fs.existsSync(process.argv[1]) &&
    fs.realpathSync(path.resolve(process.argv[1])) === fs.realpathSync(path.resolve(__filename))) {
    const [, , sub, ...rest] = process.argv;
    const args = {};
    for (let i = 0; i < rest.length; i++) {
        if (rest[i].startsWith('--')) args[rest[i].slice(2)] = rest[i + 1];
    }

    if (sub === 'coverage') {
        const manifestPath = args.manifest;
        const repoRoot = args['repo-root'] ? path.resolve(args['repo-root']) : process.cwd();
        if (!manifestPath) {
            process.stderr.write('Usage: stage2a-orchestrator.mjs coverage --manifest <file> --covered <a,b,..> [--repo-root <dir>]\n');
            process.exit(1);
        }
        const manifest = JSON.parse(fs.readFileSync(path.resolve(manifestPath), 'utf8'));
        const covered = (args.covered || '').split(',').map(s => s.trim()).filter(Boolean);
        const report = coverageReport(manifest, covered, repoRoot);
        process.stdout.write(JSON.stringify(report, null, 2) + '\n');
        process.exit(0);
    } else if (sub === 'cluster') {
        const routes = (args.routes || '').split(',').map(s => s.trim()).filter(Boolean);
        const fanoutMax = args['fanout-max'] ? parseInt(args['fanout-max'], 10) : DEFAULT_FANOUT_MAX;
        const clusters = clusterRoutes(routes, { fanout_max: fanoutMax, route_clusters: 'auto' });
        process.stdout.write(JSON.stringify(clusters, null, 2) + '\n');
        process.exit(0);
    } else {
        process.stderr.write('Usage: stage2a-orchestrator.mjs <coverage|cluster> [options]\n');
        process.exit(1);
    }
}
