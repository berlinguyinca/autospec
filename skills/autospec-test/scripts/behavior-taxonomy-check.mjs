#!/usr/bin/env node
// scripts/behavior-taxonomy-check.mjs
// Reads test-results/ traces; maps primitives per spec §4 Metric D.
//
// For each declared behavior category: checks if ≥1 trace annotation
// OR matching action primitive exists.
//
// Export: analyze(resultsDir, categories) async function
// CLI: node behavior-taxonomy-check.mjs --results-dir <dir> --categories <file>

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

/**
 * Primitive rule table per spec §4 Metric D.
 * Each category maps to a function that returns true if a trace action satisfies it.
 * @type {Record<string, (action: {type: string, selector?: string, deltaY?: number}) => boolean>}
 */
export const PRIMITIVES = {
    sort: (a) =>
        (a.type === 'click' && a.selector && /columnheader|sort/i.test(a.selector)),
    scroll: (a) =>
        a.type === 'wheel' || a.type === 'scroll',
    upload: (a) =>
        a.type === 'setInputFiles',
    download: (a) =>
        (a.type === 'click' && a.selector && /download/i.test(a.selector)) ||
        a.type === 'download',
    filter: (a) =>
        (a.type === 'fill' && a.selector && /filter|search/i.test(a.selector)) ||
        (a.type === 'click' && a.selector && /filter/i.test(a.selector)),
    paginate: (a) =>
        (a.type === 'click' && a.selector &&
         /next.*page|prev.*page|pagination|page-\d/i.test(a.selector)),
    bulk_select: (a) =>
        (a.type === 'click' && a.selector &&
         /select.all|checkbox|bulk/i.test(a.selector)),
    keyboard_nav: (a) =>
        a.type === 'press' || a.type === 'keyboard' || a.type === 'keydown',
    drag_drop: (a) =>
        a.type === 'dragstart' || a.type === 'drop' || a.type === 'drag',
};

/**
 * Check if a trace file satisfies a category via annotation or primitive.
 *
 * @param {object} trace - parsed trace JSON
 * @param {string} category
 * @returns {boolean}
 */
function traceMatchesCategory(trace, category) {
    // Check annotation-based satisfaction (explicit tagging by test author)
    const annotations = Array.isArray(trace.annotations) ? trace.annotations : [];
    for (const ann of annotations) {
        if (ann.type === 'category' && ann.description === category) {
            return true;
        }
    }

    // Check primitive-based satisfaction (action presence in trace)
    const primitiveCheck = PRIMITIVES[category];
    if (primitiveCheck) {
        const actions = Array.isArray(trace.actions) ? trace.actions : [];
        if (actions.some(primitiveCheck)) {
            return true;
        }
    }

    return false;
}

/**
 * Load and parse all JSON trace files from a results directory.
 *
 * @param {string} resultsDir
 * @returns {object[]} array of parsed trace objects
 */
function loadTraces(resultsDir) {
    if (!fs.existsSync(resultsDir)) return [];

    const traces = [];
    const entries = fs.readdirSync(resultsDir, { withFileTypes: true });

    for (const entry of entries) {
        if (!entry.isFile() || !entry.name.endsWith('.json')) continue;
        const filePath = path.join(resultsDir, entry.name);
        try {
            const parsed = JSON.parse(fs.readFileSync(filePath, 'utf8'));
            traces.push(parsed);
        } catch {
            // Skip unparseable files
        }
    }

    return traces;
}

/**
 * Analyze test results for behavior taxonomy coverage.
 *
 * @param {string} resultsDir - path to test-results/ directory
 * @param {string[]} categories - declared categories to check
 * @returns {Promise<{passed: boolean, missing: string[], passing: string[]}>}
 */
export async function analyze(resultsDir, categories) {
    if (!Array.isArray(categories) || categories.length === 0) {
        return { passed: true, missing: [], passing: [] };
    }

    const traces = loadTraces(resultsDir);
    const passing = [];
    const missing = [];

    for (const category of categories) {
        const satisfied = traces.some(trace => traceMatchesCategory(trace, category));
        if (satisfied) {
            passing.push(category);
        } else {
            missing.push(category);
        }
    }

    return {
        passed: missing.length === 0,
        missing,
        passing,
    };
}

// CLI entrypoint
const __filename = fileURLToPath(import.meta.url);
if (process.argv[1] && fs.realpathSync(path.resolve(process.argv[1])) === fs.realpathSync(path.resolve(__filename))) {
    const args = process.argv.slice(2);
    let resultsDir = null;
    let categoriesFile = null;

    for (let i = 0; i < args.length; i++) {
        if (args[i] === '--results-dir') resultsDir = args[i + 1];
        if (args[i] === '--categories') categoriesFile = args[i + 1];
    }

    if (!resultsDir || !categoriesFile) {
        process.stderr.write('Usage: behavior-taxonomy-check.mjs --results-dir <dir> --categories <file>\n');
        process.exit(1);
    }

    let categories;
    try {
        categories = JSON.parse(fs.readFileSync(categoriesFile, 'utf8'));
    } catch (err) {
        process.stderr.write(`behavior-taxonomy-check: parse error: ${err.message}\n`);
        process.exit(1);
    }

    try {
        const result = await analyze(resultsDir, categories);
        process.stdout.write(JSON.stringify(result, null, 2) + '\n');
        process.exit(result.passed ? 0 : 1);
    } catch (err) {
        process.stderr.write(`behavior-taxonomy-check: error: ${err.message}\n`);
        process.exit(1);
    }
}
