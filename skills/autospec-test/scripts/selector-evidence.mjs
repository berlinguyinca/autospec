#!/usr/bin/env node
// scripts/selector-evidence.mjs
// Selector-evidence manifest builder — source-grep resolver.
//
// extractSelectors(specText) -> [{selector, type, line}]
// resolveSelector(selector, srcGlobs, repoRoot) -> "file:line" | null
// buildEvidence(specFile, srcGlobs, repoRoot) -> {selector: "file:line"|null}
// writeManifest(manifestData, repoRoot) -> manifestPath
//
// Manifest shape: {spec_file: {selector: "file:line"}}
// Written to: <repoRoot>/e2e/.autospec/selector-evidence.json
//
// Export: extractSelectors, resolveSelector, buildEvidence, writeManifest
// CLI:    node selector-evidence.mjs --spec <file> --src <glob,...> [--repo-root <dir>]

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __filename = fileURLToPath(import.meta.url);

// ── Manifest path (per shared contracts §8) ────────────────────────────────────
const MANIFEST_REL_PATH = path.join('e2e', '.autospec', 'selector-evidence.json');

// ── Selector extraction patterns ───────────────────────────────────────────────
// Each pattern has: regex, type label, capture group index for selector value

const SELECTOR_PATTERNS = [
    // getByTestId('submit-btn') or getByTestId("submit-btn")
    {
        re: /getByTestId\s*\(\s*['"`]([^'"`]+)['"`]/g,
        type: 'data-testid',
    },
    // [data-testid="submit-btn"] or [data-testid='submit-btn']
    {
        re: /data-testid\s*[=:]\s*['"`]([^'"`]+)['"`]/g,
        type: 'data-testid',
    },
    // getByLabel('Email address') — aria-label
    {
        re: /getByLabel\s*\(\s*['"`]([^'"`]+)['"`]/g,
        type: 'aria-label',
    },
    // aria-label="Email address" or aria-label='...'
    {
        re: /aria-label\s*=\s*['"`]([^'"`]+)['"`]/g,
        type: 'aria-label',
    },
    // getByText('Welcome back') — text content
    {
        re: /getByText\s*\(\s*['"`]([^'"`]+)['"`]/g,
        type: 'text',
    },
    // getByRole('button', { name: 'Save changes' }) — role name
    {
        re: /getByRole\s*\(\s*['"`][^'"`]*['"`]\s*,\s*\{[^}]*\bname\s*:\s*['"`]([^'"`]+)['"`]/g,
        type: 'role-name',
    },
];

// ── extractSelectors ───────────────────────────────────────────────────────────

/**
 * Extract all selectors referenced in a Playwright spec text.
 * Returns deduplicated list of {selector, type, line}.
 * Multiple occurrences of the same selector value are deduplicated (first occurrence wins).
 *
 * @param {string} specText - content of the spec file
 * @returns {Array<{selector: string, type: string, line: number}>}
 */
export function extractSelectors(specText) {
    const lines = specText.split('\n');
    const seen = new Set();
    const results = [];

    for (const { re, type } of SELECTOR_PATTERNS) {
        // Reset regex lastIndex for each pattern application
        re.lastIndex = 0;
        let match;
        while ((match = re.exec(specText)) !== null) {
            const selector = match[1];
            if (!selector || seen.has(selector)) continue;
            seen.add(selector);

            // Determine line number from match index
            const textBefore = specText.slice(0, match.index);
            const line = textBefore.split('\n').length;

            results.push({ selector, type, line });
        }
    }

    return results;
}

// ── resolveSelector ────────────────────────────────────────────────────────────

/**
 * Resolve a selector string to its first occurrence in app source files.
 * Returns "relative/path/to/file.tsx:lineNumber" or null if not found.
 *
 * Searches each source file for a literal occurrence of the selector string.
 * This covers: data-testid="...", aria-label="...", text content, role names.
 *
 * @param {string} selector - the selector string to find
 * @param {string[]} srcGlobs - globs or absolute paths to search
 * @param {string} repoRoot - repository root (for relative path computation)
 * @returns {string|null} - "file:line" relative to repoRoot, or null
 */
export function resolveSelector(selector, srcGlobs, repoRoot) {
    if (!selector || srcGlobs.length === 0) return null;

    const files = expandGlobs(srcGlobs);

    for (const filePath of files) {
        let text;
        try {
            text = fs.readFileSync(filePath, 'utf8');
        } catch {
            continue;
        }

        const lines = text.split('\n');
        for (let i = 0; i < lines.length; i++) {
            if (lines[i].includes(selector)) {
                const relPath = path.relative(repoRoot, filePath);
                return `${relPath}:${i + 1}`;
            }
        }
    }

    return null;
}

// ── buildEvidence ──────────────────────────────────────────────────────────────

/**
 * Build the selector-evidence object for a single spec file.
 * Returns {selector: "file:line" | null} — null means selector was not found in app source.
 *
 * @param {string} specFile - absolute path to the spec file
 * @param {string[]} srcGlobs - globs or absolute paths for app source files
 * @param {string} repoRoot - repository root for relative path computation
 * @returns {Object<string, string|null>}
 */
export function buildEvidence(specFile, srcGlobs, repoRoot) {
    let specText;
    try {
        specText = fs.readFileSync(specFile, 'utf8');
    } catch {
        return {};
    }

    const selectors = extractSelectors(specText);
    if (selectors.length === 0) return {};

    const evidence = {};
    for (const { selector } of selectors) {
        evidence[selector] = resolveSelector(selector, srcGlobs, repoRoot);
    }
    return evidence;
}

// ── writeManifest ──────────────────────────────────────────────────────────────

/**
 * Write the full selector-evidence manifest to e2e/.autospec/selector-evidence.json.
 * Creates parent directories if missing. Overwrites any existing manifest.
 *
 * @param {Object<string, Object<string, string|null>>} manifestData
 *   Shape: {spec_file: {selector: "file:line"|null}}
 * @param {string} repoRoot - repository root
 * @returns {string} - absolute path to the written manifest file
 */
export function writeManifest(manifestData, repoRoot) {
    const manifestPath = path.join(repoRoot, MANIFEST_REL_PATH);
    fs.mkdirSync(path.dirname(manifestPath), { recursive: true });
    fs.writeFileSync(manifestPath, JSON.stringify(manifestData, null, 2) + '\n', 'utf8');
    return manifestPath;
}

// ── Glob expansion ─────────────────────────────────────────────────────────────

/**
 * Expand an array of globs/paths to a list of file paths.
 * For simple absolute file paths (no glob chars), includes them directly.
 * For patterns with glob chars, does a recursive directory scan.
 *
 * Note: uses fs.readdirSync for portability (no external glob dependency).
 *
 * @param {string[]} patterns
 * @returns {string[]}
 */
function expandGlobs(patterns) {
    const results = [];
    const seen = new Set();

    for (const pattern of patterns) {
        if (!hasGlobChars(pattern)) {
            // Direct path
            if (fs.existsSync(pattern)) {
                const stat = fs.statSync(pattern);
                if (stat.isFile() && !seen.has(pattern)) {
                    results.push(pattern);
                    seen.add(pattern);
                } else if (stat.isDirectory()) {
                    collectFiles(pattern, results, seen);
                }
            }
            continue;
        }

        // Simple glob expansion: handle **/ prefix and file extension suffix
        // Pattern like /some/dir/src/** or /some/dir/**/*.tsx
        const { base, ext } = parseGlob(pattern);

        if (base && fs.existsSync(base)) {
            const files = collectFilesWithExt(base, ext);
            for (const f of files) {
                if (!seen.has(f)) {
                    results.push(f);
                    seen.add(f);
                }
            }
        }
    }

    return results;
}

function hasGlobChars(pattern) {
    return pattern.includes('*') || pattern.includes('?') || pattern.includes('{');
}

/**
 * Parse a glob pattern into {base, ext}.
 * e.g. "/dir/src/**" -> {base: "/dir/src", ext: null}
 *      "/dir/src/**\/*.tsx" -> {base: "/dir/src", ext: ".tsx"}
 *      "/dir/src/**\/*.{ts,tsx}" -> {base: "/dir/src", ext: null} (collect all)
 */
function parseGlob(pattern) {
    // Remove trailing ** or **/*
    let base = pattern;
    let ext = null;

    // Find the first glob character's path component
    const parts = pattern.split('/');
    const globIdx = parts.findIndex(p => hasGlobChars(p));
    if (globIdx > 0) {
        base = parts.slice(0, globIdx).join('/');
    } else if (globIdx === 0) {
        base = '/';
    } else {
        base = pattern;
    }

    // Check for extension pattern like *.tsx or *.{ts,tsx}
    const lastPart = parts[parts.length - 1];
    if (lastPart && !hasGlobChars(lastPart.replace(/\.\w+$/, ''))) {
        const extMatch = lastPart.match(/\*(\.\w+)$/);
        if (extMatch) ext = extMatch[1];
    }

    return { base, ext };
}

function collectFiles(dir, results, seen) {
    let entries;
    try {
        entries = fs.readdirSync(dir, { withFileTypes: true });
    } catch {
        return;
    }
    for (const entry of entries) {
        const full = path.join(dir, entry.name);
        if (entry.isDirectory() && !entry.name.startsWith('.')) {
            collectFiles(full, results, seen);
        } else if (entry.isFile() && !seen.has(full)) {
            results.push(full);
            seen.add(full);
        }
    }
}

function collectFilesWithExt(dir, ext) {
    const results = [];
    const seen = new Set();

    function recurse(d) {
        let entries;
        try {
            entries = fs.readdirSync(d, { withFileTypes: true });
        } catch {
            return;
        }
        for (const entry of entries) {
            const full = path.join(d, entry.name);
            if (entry.isDirectory() && !entry.name.startsWith('.') && entry.name !== 'node_modules') {
                recurse(full);
            } else if (entry.isFile() && !seen.has(full)) {
                if (!ext || full.endsWith(ext)) {
                    results.push(full);
                    seen.add(full);
                }
            }
        }
    }

    recurse(dir);
    return results;
}

// ── CLI entrypoint ─────────────────────────────────────────────────────────────

if (process.argv[1] && fs.existsSync(process.argv[1]) &&
    fs.realpathSync(path.resolve(process.argv[1])) === fs.realpathSync(path.resolve(__filename))) {
    const args = process.argv.slice(2);
    let specFile = null;
    const srcGlobs = [];
    let repoRoot = process.cwd();

    for (let i = 0; i < args.length; i++) {
        if (args[i] === '--spec') specFile = path.resolve(args[++i]);
        else if (args[i] === '--src') srcGlobs.push(...args[++i].split(','));
        else if (args[i] === '--repo-root') repoRoot = path.resolve(args[++i]);
    }

    if (!specFile) {
        process.stderr.write('Usage: selector-evidence.mjs --spec <file> --src <glob,...> [--repo-root <dir>]\n');
        process.exit(1);
    }

    // Read existing manifest if present
    const manifestPath = path.join(repoRoot, MANIFEST_REL_PATH);
    let manifest = {};
    if (fs.existsSync(manifestPath)) {
        try {
            manifest = JSON.parse(fs.readFileSync(manifestPath, 'utf8'));
        } catch { /* start fresh */ }
    }

    // Build evidence for this spec and merge into manifest
    const evidence = buildEvidence(specFile, srcGlobs, repoRoot);
    manifest[specFile] = evidence;

    const written = writeManifest(manifest, repoRoot);
    process.stdout.write(JSON.stringify(evidence, null, 2) + '\n');
    process.stderr.write(`selector-evidence: wrote manifest to ${written}\n`);

    const unresolved = Object.values(evidence).filter(v => v === null).length;
    if (unresolved > 0) {
        process.stderr.write(`selector-evidence: ${unresolved} unresolved selector(s)\n`);
        process.exit(1);
    }
    process.exit(0);
}
