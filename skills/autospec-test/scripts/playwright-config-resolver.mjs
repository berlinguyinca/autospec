#!/usr/bin/env node
// scripts/playwright-config-resolver.mjs
// resolve(repoRoot) -> {baseURL, useBaseURL, webServerURL, projects, testDir}
//
// Supports playwright.config.{ts,js,mjs,cjs} via static text-parsing (no dynamic import).
// Static parse avoids needing Playwright or TypeScript installed in the autospec repo.
//
// Export: resolve(repoRoot) async function
// CLI: node playwright-config-resolver.mjs <repoRoot>

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

// Config file search order (most specific first)
const CONFIG_GLOB_ORDER = [
    'playwright.config.ts',
    'playwright.config.mjs',
    'playwright.config.js',
    'playwright.config.cjs',
];

function escapeRegex(s) {
    return s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

/**
 * Extract a string value for a key from a JS/TS object literal text.
 * Uses a simple quoted-string pattern — only matches string literal values,
 * not nested object values. Safe against nested objects (viewport, headers, etc).
 *
 * @param {string} text - raw config file text (or block thereof)
 * @param {string} key  - property name to extract
 * @returns {string|null}
 */
function extractStringValue(text, key) {
    const pattern = new RegExp(
        `(?:^|[,{\\s])${escapeRegex(key)}\\s*:\\s*(?:'([^']*)'|"([^"]*)")`,
        'gm'
    );
    const match = pattern.exec(text);
    if (!match) return null;
    return match[1] !== undefined ? match[1] : match[2];
}

/**
 * Extract the inner content of a named block: `key: { ... }`.
 * Tracks brace depth so nested objects inside the block are included correctly.
 * This is the fix for PR #331 finding #4: use.baseURL extraction must handle
 * nested objects (viewport, permissions, extraHTTPHeaders, etc).
 *
 * @param {string} text
 * @param {string} key
 * @returns {string|null}
 */
function extractBlock(text, key) {
    const startPattern = new RegExp(`(?:^|[,{\\s])${escapeRegex(key)}\\s*:\\s*\\{`, 'gm');
    const match = startPattern.exec(text);
    if (!match) return null;

    let depth = 1;
    let i = match.index + match[0].length;
    const start = i;
    while (i < text.length && depth > 0) {
        const ch = text[i];
        if (ch === '{') depth++;
        else if (ch === '}') depth--;
        i++;
    }
    if (depth !== 0) return null;
    return text.slice(start, i - 1);
}

/**
 * Extract webServer URL. webServer can be an object or array of objects.
 * @param {string} text
 * @returns {string|null}
 */
function extractWebServerURL(text) {
    const block = extractBlock(text, 'webServer');
    if (!block) return null;
    return extractStringValue(block, 'url');
}

/**
 * Main resolver.
 * @param {string} repoRoot - absolute path to target repo
 * @returns {Promise<{baseURL: string|null, useBaseURL: string|null, webServerURL: string|null, projects: any[], testDir: string|null, configFile: string|null}>}
 */
export async function resolve(repoRoot) {
    const result = {
        baseURL: null,
        useBaseURL: null,
        webServerURL: null,
        projects: [],
        testDir: null,
        configFile: null,
    };

    // Find config file
    let configPath = null;
    for (const name of CONFIG_GLOB_ORDER) {
        const candidate = path.join(repoRoot, name);
        if (fs.existsSync(candidate)) {
            configPath = candidate;
            result.configFile = name;
            break;
        }
    }

    if (!configPath) return result;

    const text = fs.readFileSync(configPath, 'utf8');

    // Extract top-level baseURL
    result.baseURL = extractStringValue(text, 'baseURL');

    // Extract `use: { ... }` block using depth-tracking extractor.
    // This correctly handles nested objects (viewport, permissions, extraHTTPHeaders, etc)
    // inside the use block — fixing the regex fragility from PR #331 finding #4.
    const useBlock = extractBlock(text, 'use');
    if (useBlock) {
        result.useBaseURL = extractStringValue(useBlock, 'baseURL');
        // Prefer use.baseURL as the effective baseURL if top-level not set
        if (!result.baseURL) {
            result.baseURL = result.useBaseURL;
        }
    }

    // Extract webServer URL
    result.webServerURL = extractWebServerURL(text);

    // Extract testDir
    result.testDir = extractStringValue(text, 'testDir');

    // Projects: detect presence via projects block
    const projectsMatch = text.match(/projects\s*:\s*\[/);
    if (projectsMatch) {
        const afterProjects = text.slice(text.indexOf(projectsMatch[0]) + projectsMatch[0].length);
        const nameMatches = afterProjects.match(/name\s*:/g);
        result.projects = nameMatches
            ? Array.from({ length: nameMatches.length }, (_, i) => ({ index: i }))
            : [];
    }

    return result;
}

// CLI entrypoint
const __filename = fileURLToPath(import.meta.url);
if (process.argv[1] && fs.realpathSync(path.resolve(process.argv[1])) === fs.realpathSync(path.resolve(__filename))) {
    const repoRoot = process.argv[2] || process.cwd();
    try {
        const result = await resolve(repoRoot);
        process.stdout.write(JSON.stringify(result, null, 2) + '\n');
        process.exit(0);
    } catch (err) {
        process.stderr.write(`playwright-config-resolver: error: ${err.message}\n`);
        process.exit(1);
    }
}
