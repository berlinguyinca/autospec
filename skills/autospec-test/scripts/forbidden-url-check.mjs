#!/usr/bin/env node
// scripts/forbidden-url-check.mjs
// check(config, patterns) -> {violations: [{field, value, pattern}]}
//
// Checks every URL-shaped field in a resolved Playwright config object against
// forbidden_url_patterns. Exits 2 if any violation found, 0 if clean.
//
// URL fields checked (per spec §5a Layer A):
//   - baseURL (top-level)
//   - use.baseURL
//   - webServer.url (object or array of objects)
//   - projects[*].use.baseURL
//
// Export: check(config, patterns) function
// CLI: node forbidden-url-check.mjs --config <file> --patterns <file>

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

/**
 * Extract all URL-shaped field values from a Playwright config object.
 * Returns [{field, value}] for every non-empty string URL field found.
 *
 * @param {object} config - parsed Playwright config
 * @returns {{field: string, value: string}[]}
 */
function extractURLFields(config) {
    const fields = [];

    function add(field, value) {
        if (typeof value === 'string' && value.trim()) {
            fields.push({ field, value: value.trim() });
        }
    }

    // Top-level baseURL
    add('baseURL', config.baseURL);

    // use.baseURL (nested object — must extract safely without regex on raw text)
    if (config.use && typeof config.use === 'object') {
        add('use.baseURL', config.use.baseURL);
    }

    // webServer — can be object or array
    if (config.webServer) {
        const servers = Array.isArray(config.webServer) ? config.webServer : [config.webServer];
        servers.forEach((srv, i) => {
            const prefix = Array.isArray(config.webServer) ? `webServer[${i}].url` : 'webServer.url';
            add(prefix, srv && srv.url);
        });
    }

    // projects[*].use.baseURL
    if (Array.isArray(config.projects)) {
        config.projects.forEach((proj, i) => {
            if (proj && proj.use && typeof proj.use === 'object') {
                add(`projects[${i}].use.baseURL`, proj.use.baseURL);
            }
        });
    }

    return fields;
}

/**
 * Check all URL fields against forbidden patterns.
 *
 * @param {object} config   - Playwright config object
 * @param {string[]} patterns - array of regex strings
 * @returns {{violations: {field: string, value: string, pattern: string}[]}}
 */
export function check(config, patterns) {
    if (!config || typeof config !== 'object') return { violations: [] };
    if (!Array.isArray(patterns) || patterns.length === 0) return { violations: [] };

    const urlFields = extractURLFields(config);
    const violations = [];

    for (const { field, value } of urlFields) {
        for (const pat of patterns) {
            let re;
            try {
                re = new RegExp(pat);
            } catch {
                // Invalid regex — skip
                continue;
            }
            if (re.test(value)) {
                violations.push({ field, value, pattern: pat });
                break; // first match per field
            }
        }
    }

    return { violations };
}

// CLI entrypoint
const __filename = fileURLToPath(import.meta.url);
if (process.argv[1] && fs.realpathSync(path.resolve(process.argv[1])) === fs.realpathSync(path.resolve(__filename))) {
    // Parse --config <file> --patterns <file>
    const args = process.argv.slice(2);
    let configFile = null;
    let patternsFile = null;
    for (let i = 0; i < args.length; i++) {
        if (args[i] === '--config') configFile = args[i + 1];
        if (args[i] === '--patterns') patternsFile = args[i + 1];
    }

    if (!configFile || !patternsFile) {
        process.stderr.write('Usage: forbidden-url-check.mjs --config <file> --patterns <file>\n');
        process.exit(1);
    }

    let config, patterns;
    try {
        config = JSON.parse(fs.readFileSync(configFile, 'utf8'));
        patterns = JSON.parse(fs.readFileSync(patternsFile, 'utf8'));
    } catch (err) {
        process.stderr.write(`forbidden-url-check: parse error: ${err.message}\n`);
        process.exit(1);
    }

    const result = check(config, patterns);
    process.stdout.write(JSON.stringify(result, null, 2) + '\n');

    if (result.violations.length > 0) {
        process.stderr.write(
            `forbidden-url-check: BLOCKED — ${result.violations.length} forbidden URL violation(s):\n`
        );
        for (const v of result.violations) {
            process.stderr.write(`  ${v.field}: ${v.value} (matches ${v.pattern})\n`);
        }
        process.exit(2);
    }
    process.exit(0);
}
