#!/usr/bin/env node
// scripts/error-signature.mjs
// normalize(error_text) -> sha256 hex digest
//
// Strips line numbers, browser tags, randomized identifiers so that
// two errors from the same root cause hash to the same signature.
//
// Export: normalize(errorText) -> string (hex digest)
// CLI: node error-signature.mjs <error_text_file>
//   OR: echo "error text" | node error-signature.mjs

import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __filename = fileURLToPath(import.meta.url);

/**
 * Normalization rules applied in order.
 * Each rule: [pattern, replacement]
 */
const NORMALIZATION_RULES = [
    // Strip ISO-8601 timestamps FIRST (before line:col rule strips the colons)
    [/\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?Z?/g, 'TIMESTAMP'],

    // Strip UUIDs FIRST (before HEXID rule strips partial matches)
    [/\b[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\b/gi, 'UUID'],

    // Strip browser/worker tags (e.g. "[chromium]", "[firefox]", "[webkit]", "[worker1]")
    // Also strip bare word "workerN" and "pageN" identifiers
    [/\[(chromium|firefox|webkit|worker\d*|page\d*)\]/gi, '[BROWSER]'],
    [/\bworker\d+\b/gi, 'WORKER'],
    [/\bpage\d+\b/gi, 'PAGE'],

    // Strip epoch timestamps
    [/\b\d{10,13}\b/g, 'EPOCH'],

    // Strip hex IDs (32-64 chars)
    [/\b[0-9a-f]{32,64}\b/gi, 'HEXID'],

    // Strip line:col numbers (e.g. ":123:45" or " line 123")
    [/:\d+:\d+/g, ':L:C'],
    [/\bline\s+\d+\b/gi, 'line L'],
    [/\bat line \d+/gi, 'at line L'],

    // Strip temp file paths (e.g. /tmp/autospec-abc123)
    [/\/(?:tmp|var\/folders|private\/tmp)\/[^\s"')]+/g, '/tmp/TMPPATH'],

    // Strip process IDs
    [/\bpid\s+\d+/gi, 'pid PID'],
    [/\bprocess\s+\d+/gi, 'process PID'],

    // Strip port numbers in URLs (but keep protocol and host)
    [/(:)\d{4,5}(\/|$|\s)/g, ':PORT$2'],

    // Normalize whitespace (multiple spaces/tabs → single space)
    [/[ \t]+/g, ' '],

    // Strip trailing whitespace per line
    [/ +$/gm, ''],

    // Collapse blank lines
    [/\n{3,}/g, '\n\n'],
];

/**
 * Normalize error text for stable hashing.
 *
 * @param {string} errorText
 * @returns {string} normalized text
 */
export function normalize(errorText) {
    if (!errorText || typeof errorText !== 'string') return '';

    let text = errorText;
    for (const [pattern, replacement] of NORMALIZATION_RULES) {
        text = text.replace(pattern, replacement);
    }
    return text.trim();
}

/**
 * Compute SHA-256 hex digest of normalized error text.
 *
 * @param {string} errorText
 * @returns {string} hex digest (64 chars)
 */
export function signature(errorText) {
    const normalized = normalize(errorText);
    return crypto.createHash('sha256').update(normalized, 'utf8').digest('hex');
}

// CLI entrypoint
if (process.argv[1] && fs.realpathSync(path.resolve(process.argv[1])) === fs.realpathSync(path.resolve(__filename))) {
    let input = '';

    if (process.argv[2]) {
        // Read from file
        try {
            input = fs.readFileSync(process.argv[2], 'utf8');
        } catch (err) {
            process.stderr.write(`error-signature: cannot read file: ${err.message}\n`);
            process.exit(1);
        }
    } else {
        // Read from stdin
        const chunks = [];
        process.stdin.on('data', chunk => chunks.push(chunk));
        await new Promise(resolve => process.stdin.on('end', resolve));
        input = Buffer.concat(chunks).toString('utf8');
    }

    const sig = signature(input);
    process.stdout.write(sig + '\n');
    process.exit(0);
}
