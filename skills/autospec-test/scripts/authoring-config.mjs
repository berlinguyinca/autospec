#!/usr/bin/env node
// scripts/authoring-config.mjs
// loadAuthoringConfig(testYmlPath) -> {authoring, reset, control_effects}
//
// Loads and validates the e2e.authoring / e2e.reset / e2e.control_effects
// blocks from a .autospec/test.yml file, merging with conservative defaults.
//
// Fail-closed validation:
//   - Throws if reset.generate_if_missing=true and reset.guard_env is missing/empty
//   - Throws if e2e.forbidden_url_patterns is missing/empty (unless
//     forbidden_url_patterns_intentionally_empty=true is set)
//
// Export: loadAuthoringConfig(testYmlPath) async function
// CLI:    node authoring-config.mjs <testYmlPath>

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';

const execFileAsync = promisify(execFile);

const __filename = fileURLToPath(import.meta.url);

// ── Defaults (conservative) ────────────────────────────────────────────────────

const AUTHORING_DEFAULTS = {
    enabled: false,
    spec_dir: 'e2e/specs',
    helpers_dir: 'e2e/helpers',
    route_clusters: 'auto',
    coverage_target_pct: 80,
    fanout_max: 4,
};

const RESET_DEFAULTS = {
    endpoint: '/api/test/reset',
    generate_if_missing: false,
    guard_env: 'AUTOSPEC_TEST_STACK',
};

const CONTROL_EFFECTS_DEFAULTS = {
    enabled: true,
};

// ── YAML parser — uses yq if available, falls back to Node built-in ────────────

/**
 * Parse a YAML file to a plain JS object.
 * Tries `yq -o=json` first (same dependency as load-contract.sh),
 * then falls back to a lightweight built-in YAML-subset parser for simple
 * key: value / list structures (sufficient for test.yml).
 *
 * @param {string} filePath
 * @returns {Promise<object>}
 */
async function parseYamlFile(filePath) {
    // Try yq (preferred — handles all YAML)
    try {
        const { stdout } = await execFileAsync('yq', ['-o=json', '.', filePath], { timeout: 10000 });
        const trimmed = stdout.trim();
        if (trimmed && trimmed !== 'null') {
            return JSON.parse(trimmed);
        }
        return {};
    } catch {
        // yq not available or failed — try js-yaml if available
    }

    // Try js-yaml (may be installed as a dev dependency)
    try {
        const { createRequire } = await import('node:module');
        const require = createRequire(import.meta.url);
        const yaml = require('js-yaml');
        const text = fs.readFileSync(filePath, 'utf8');
        return yaml.load(text) || {};
    } catch {
        // Not available
    }

    // Fallback: minimal YAML parser for the subset used in test.yml
    // Handles: scalar keys, quoted/unquoted string values, boolean, integer,
    // nested objects (indented blocks), and simple string lists.
    const text = fs.readFileSync(filePath, 'utf8');
    return parseMinimalYaml(text);
}

/**
 * Minimal YAML parser for the subset used in .autospec/test.yml.
 * Not a general-purpose parser — handles the shapes present in test.yml:
 *   - top-level and nested key: value scalars
 *   - boolean true/false
 *   - integer values
 *   - simple string list items (- "value" or - value)
 *   - nested objects via indentation
 *
 * @param {string} text
 * @returns {object}
 */
function parseMinimalYaml(text) {
    const lines = text.split('\n');
    const root = {};
    const stack = [{ indent: -1, obj: root }];

    for (let i = 0; i < lines.length; i++) {
        const rawLine = lines[i];
        const stripped = rawLine.replace(/#.*$/, '').trimEnd(); // strip comments
        if (!stripped.trim()) continue;

        const indent = stripped.length - stripped.trimStart().length;
        const line = stripped.trimStart();

        // Pop stack to current indent level
        while (stack.length > 1 && stack[stack.length - 1].indent >= indent) {
            stack.pop();
        }

        const parent = stack[stack.length - 1].obj;

        // List item
        if (line.startsWith('- ')) {
            const val = parseScalar(line.slice(2).trim());
            const lastKey = Object.keys(parent).pop();
            if (lastKey !== undefined && !Array.isArray(parent[lastKey])) {
                parent[lastKey] = [];
            }
            if (lastKey !== undefined) {
                parent[lastKey].push(val);
            }
            continue;
        }

        // Key: value or Key: (nested block)
        const colonIdx = line.indexOf(':');
        if (colonIdx === -1) continue;
        const key = line.slice(0, colonIdx).trim();
        const rest = line.slice(colonIdx + 1).trim();

        if (rest === '' || rest.startsWith('#')) {
            // Start of a nested object
            const nested = {};
            parent[key] = nested;
            stack.push({ indent, obj: nested });
        } else {
            parent[key] = parseScalar(rest);
        }
    }

    return root;
}

/**
 * Parse a YAML scalar value string to JS primitive.
 * Handles: quoted strings, booleans, integers, plain strings.
 *
 * @param {string} s
 * @returns {string|boolean|number}
 */
function parseScalar(s) {
    // Strip inline comment
    const noComment = s.replace(/\s+#.*$/, '').trim();

    // Quoted string (single or double)
    if ((noComment.startsWith('"') && noComment.endsWith('"')) ||
        (noComment.startsWith("'") && noComment.endsWith("'"))) {
        return noComment.slice(1, -1);
    }
    // Boolean
    if (noComment === 'true') return true;
    if (noComment === 'false') return false;
    // Null
    if (noComment === 'null' || noComment === '~') return null;
    // Integer
    if (/^-?\d+$/.test(noComment)) return parseInt(noComment, 10);
    // Float
    if (/^-?\d+\.\d+$/.test(noComment)) return parseFloat(noComment);
    // Plain string
    return noComment;
}

// ── Main export ────────────────────────────────────────────────────────────────

/**
 * Load and validate the e2e.authoring, e2e.reset, and e2e.control_effects
 * blocks from a .autospec/test.yml file.
 *
 * Returns defaults for any missing blocks/keys. Throws for invalid combinations
 * (fail-closed behaviour per spec §4).
 *
 * @param {string} testYmlPath - absolute or relative path to test.yml
 * @returns {Promise<{authoring: object, reset: object, control_effects: object}>}
 * @throws {Error} if fail-closed validation rules are violated
 */
export async function loadAuthoringConfig(testYmlPath) {
    let raw = {};

    if (fs.existsSync(testYmlPath)) {
        raw = await parseYamlFile(testYmlPath);
        if (!raw || typeof raw !== 'object') {
            raw = {};
        }
    }

    const e2e = (raw && typeof raw.e2e === 'object' && raw.e2e !== null) ? raw.e2e : {};

    // ── Merge authoring block with defaults ────────────────────────────────────
    const rawAuthoring = (typeof e2e.authoring === 'object' && e2e.authoring !== null)
        ? e2e.authoring : {};
    const authoring = { ...AUTHORING_DEFAULTS, ...rawAuthoring };

    // ── Merge reset block with defaults ───────────────────────────────────────
    const rawReset = (typeof e2e.reset === 'object' && e2e.reset !== null)
        ? e2e.reset : {};

    // Build reset by merging defaults, but do NOT apply guard_env default when
    // generate_if_missing=true — the caller must supply an explicit guard_env.
    // This makes the fail-closed rule meaningful (the default would silently satisfy it).
    const effectiveGenerateIfMissing = rawReset.generate_if_missing !== undefined
        ? rawReset.generate_if_missing
        : RESET_DEFAULTS.generate_if_missing;

    let resetBase;
    if (rawReset.cmd !== undefined) {
        // cmd variant — no endpoint default
        resetBase = { generate_if_missing: false, guard_env: 'AUTOSPEC_TEST_STACK', ...rawReset };
    } else if (effectiveGenerateIfMissing === true) {
        // generate_if_missing=true: guard_env must be explicitly supplied — no default injection
        resetBase = { ...RESET_DEFAULTS, ...rawReset };
        // Strip the defaulted guard_env if it was NOT explicitly set by the caller
        if (!('guard_env' in rawReset)) {
            delete resetBase.guard_env;
        }
    } else {
        resetBase = { ...RESET_DEFAULTS, ...rawReset };
    }
    const reset = resetBase;

    // ── Merge control_effects block with defaults ──────────────────────────────
    const rawControlEffects = (typeof e2e.control_effects === 'object' && e2e.control_effects !== null)
        ? e2e.control_effects : {};
    const control_effects = { ...CONTROL_EFFECTS_DEFAULTS, ...rawControlEffects };

    // ── Fail-closed validation ─────────────────────────────────────────────────
    // Validation order: guard_env check first (more specific), then forbidden_url_patterns.

    // Rule 1: generate_if_missing=true requires a non-empty guard_env
    if (reset.generate_if_missing === true) {
        const guardEnv = reset.guard_env;
        if (!guardEnv || (typeof guardEnv === 'string' && guardEnv.trim() === '')) {
            throw new Error(
                'authoring-config: validation error: reset.generate_if_missing=true requires ' +
                'reset.guard_env to be set to a non-empty environment variable name. ' +
                'Set reset.guard_env (e.g. AUTOSPEC_TEST_STACK) to name the guard env var ' +
                'that must be present before the generated reset endpoint is exposed.'
            );
        }
    }

    // Rule 2: forbidden_url_patterns must be present and non-empty in yml
    // (only when a yml file exists — if yml is missing, we're in default-only mode
    //  which is safe since authoring.enabled defaults to false)
    if (fs.existsSync(testYmlPath)) {
        const patterns = e2e.forbidden_url_patterns;
        const intentionallyEmpty = e2e.forbidden_url_patterns_intentionally_empty === true;

        if (!intentionallyEmpty) {
            if (!Array.isArray(patterns) || patterns.length === 0) {
                throw new Error(
                    'authoring-config: validation error: e2e.forbidden_url_patterns is missing or empty. ' +
                    'Set at least one forbidden URL pattern (e.g. "^https?://prod\\\\.example\\\\.com") to ' +
                    'prevent tests from running against production environments. ' +
                    'If no URL restrictions apply, set e2e.forbidden_url_patterns_intentionally_empty: true.'
                );
            }
        }
    }

    return { authoring, reset, control_effects };
}

// ── CLI entrypoint ─────────────────────────────────────────────────────────────

if (process.argv[1] && fs.existsSync(process.argv[1]) &&
    fs.realpathSync(path.resolve(process.argv[1])) === fs.realpathSync(path.resolve(__filename))) {
    const testYmlPath = process.argv[2];
    if (!testYmlPath) {
        process.stderr.write('Usage: authoring-config.mjs <testYmlPath>\n');
        process.exit(1);
    }

    try {
        const cfg = await loadAuthoringConfig(path.resolve(testYmlPath));
        process.stdout.write(JSON.stringify(cfg, null, 2) + '\n');
    } catch (err) {
        process.stderr.write(`authoring-config: ${err.message}\n`);
        process.exit(2);
    }
}
