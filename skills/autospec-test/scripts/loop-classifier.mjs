#!/usr/bin/env node
// scripts/loop-classifier.mjs
// Failure classifier for the self-heal loop.
//
// classify({gate_json, findings_md, last_3_iterations}) ->
//   {classification, target_failures, suggested_files, estimated_minutes, priority}
//
// Priority order (highest to lowest) per spec §6:
//   product_bug > missing_unit_test > missing_test > selector_brittle
//   > failing_unit_test > failing_test > flaky_test
//
// Export: classify(options) function
// CLI: node loop-classifier.mjs --gate-result <file> [--findings <file>]

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

// ── Docs-amendment extension (Phase 3) ───────────────────────────────────────
// Route docs-gate JSON through the docs classifier before v1 fallback.
// Path: skills/autospec-shared/scripts/loop-classifier-docs-extension.mjs
let _docsClassify = null;
try {
    const docsExtPath = path.resolve(
        __dirname,
        '../../../autospec-shared/scripts/loop-classifier-docs-extension.mjs'
    );
    if (fs.existsSync(docsExtPath)) {
        const { classify: dc } = await import(`file://${docsExtPath}`);
        _docsClassify = dc;
    }
} catch {
    // Extension not installed — degrade gracefully; v1 logic runs unmodified.
}

/** Priority rank per spec §6 (higher = more urgent) */
const PRIORITY_RANK = {
    product_bug: 7,
    missing_unit_test: 6,
    missing_test: 5,
    selector_brittle: 4,
    failing_unit_test: 3,
    failing_test: 2,
    flaky_test: 1,
    empty_action: 0,
};

/**
 * Classify failures from gate JSON and findings.
 *
 * @param {object} options
 * @param {object} options.gate_json - parsed gate result JSON
 * @param {string} [options.findings_md] - findings markdown text
 * @param {object[]} [options.last_3_iterations] - last 3 iteration records from loop state
 * @returns {{classification: string, target_failures: string[], suggested_files: string[], estimated_minutes: number, priority: number}}
 */
export function classify(options) {
    const { gate_json = {}, findings_md = '', last_3_iterations = [] } = options;

    const candidates = [];

    // ── product_bug detection ─────────────────────────────────────────────────
    // Test is correct but product code fails. Heuristic: test fails with
    // 'Expected ... but received' where the assertion is structurally sound.
    if (gate_json.stage === 'unit' || gate_json.stage === 'e2e') {
        const testSummary = gate_json.test_run_summary || {};
        const stderr = (testSummary.stderr_tail || '').toLowerCase();
        const stdout = (testSummary.stdout_tail || '').toLowerCase();
        const combined = stderr + stdout;

        const productBugSignals = [
            /expected.*but (received|got)/i,
            /assertion.*failed.*expected/i,
            /\bwant\b.*\bgot\b/i,
            /\bassert_eq!.*left.*right/i,
        ];
        if (productBugSignals.some(p => p.test(combined))) {
            candidates.push({
                classification: 'product_bug',
                target_failures: ['product code returning wrong value'],
                suggested_files: ['src/', 'lib/', 'pkg/'],
                estimated_minutes: 15,
                priority: PRIORITY_RANK.product_bug,
            });
        }
    }

    // ── missing_unit_test detection ───────────────────────────────────────────
    if (gate_json.stage === 'unit') {
        const metrics = gate_json.metrics?.unit || {};
        const missingFns = metrics.missing_function_tests || [];
        if (missingFns.length > 0 || metrics.passed === false) {
            candidates.push({
                classification: 'missing_unit_test',
                target_failures: missingFns,
                suggested_files: ['tests/', '__tests__/', 'test/'],
                estimated_minutes: 10,
                priority: PRIORITY_RANK.missing_unit_test,
            });
        }
    }

    // ── missing_test (E2E gap) detection ─────────────────────────────────────
    if (gate_json.stage === 'e2e') {
        const metrics = gate_json.metrics?.e2e || {};
        const uiCov = metrics.ui_element_coverage || {};
        const behaviorCov = metrics.behavior_categories || {};
        const missing = [];
        if (uiCov.passed === false) {
            missing.push(...(uiCov.missing || []));
        }
        if (behaviorCov.passed === false) {
            missing.push(...(behaviorCov.missing || []).map(c => `behavior:${c}`));
        }
        if (missing.length > 0) {
            candidates.push({
                classification: 'missing_test',
                target_failures: missing,
                suggested_files: ['tests/', 'e2e/', 'playwright/', 'spec/'],
                estimated_minutes: 15,
                priority: PRIORITY_RANK.missing_test,
            });
        }
    }

    // ── selector_brittle detection ────────────────────────────────────────────
    const testSummary = gate_json.test_run_summary || {};
    const combinedOutput = ((testSummary.stderr_tail || '') + (testSummary.stdout_tail || '')).toLowerCase();
    const selectorSignals = [
        /locator.*strict mode/i,
        /waiting for.*selector/i,
        /timeout.*waiting/i,
        /element.*not found/i,
        /no element.*matches/i,
        /element handle.*disposed/i,
    ];
    if (selectorSignals.some(p => p.test(combinedOutput))) {
        candidates.push({
            classification: 'selector_brittle',
            target_failures: ['selector resolution failed'],
            suggested_files: ['playwright.config.ts', 'playwright.config.js', 'tests/'],
            estimated_minutes: 10,
            priority: PRIORITY_RANK.selector_brittle,
        });
    }

    // ── failing_unit_test detection ───────────────────────────────────────────
    if (gate_json.stage === 'unit' && gate_json.passed === false) {
        const reason = gate_json.reason || '';
        if (reason === 'tests_red') {
            candidates.push({
                classification: 'failing_unit_test',
                target_failures: ['unit tests red'],
                suggested_files: ['tests/', '__tests__/', 'test/'],
                estimated_minutes: 10,
                priority: PRIORITY_RANK.failing_unit_test,
            });
        }
    }

    // ── failing_test (E2E) detection ──────────────────────────────────────────
    if (gate_json.stage === 'e2e' && gate_json.passed === false) {
        const reason = gate_json.reason || '';
        if (reason === 'tests_red') {
            candidates.push({
                classification: 'failing_test',
                target_failures: ['E2E tests red'],
                suggested_files: ['tests/', 'e2e/', 'spec/'],
                estimated_minutes: 15,
                priority: PRIORITY_RANK.failing_test,
            });
        }
    }

    // ── flaky_test detection ──────────────────────────────────────────────────
    // Flaky: passes in some of last 3 iterations, fails in others
    if (last_3_iterations.length >= 2) {
        const passedResults = last_3_iterations.map(it => it.gate_passed);
        const hasPass = passedResults.some(p => p === true);
        const hasFail = passedResults.some(p => p === false);
        if (hasPass && hasFail) {
            candidates.push({
                classification: 'flaky_test',
                target_failures: ['intermittent test failures'],
                suggested_files: ['playwright.config.ts', 'playwright.config.js', 'tests/'],
                estimated_minutes: 10,
                priority: PRIORITY_RANK.flaky_test,
            });
        }
    }

    // ── docs-amendment extension (Phase 3) ───────────────────────────────────
    // If the gate JSON looks like a doc-drift result, route to docs classifier.
    // Doc findings are inserted ahead of the v1 fallback but after all v1
    // signal-based candidates so existing priority ordering is preserved.
    if (_docsClassify) {
        const docsResult = _docsClassify(gate_json, last_3_iterations);
        if (docsResult) {
            candidates.push({
                classification: docsResult.classification,
                target_failures: docsResult.target_files || [],
                suggested_files: docsResult.target_files || [],
                estimated_minutes: docsResult.estimated_minutes,
                priority: docsResult.priority,
            });
        }
    }

    // ── empty_action fallback ─────────────────────────────────────────────────
    if (candidates.length === 0) {
        return {
            classification: 'empty_action',
            target_failures: [],
            suggested_files: [],
            estimated_minutes: 0,
            priority: PRIORITY_RANK.empty_action,
        };
    }

    // Return highest-priority candidate
    candidates.sort((a, b) => b.priority - a.priority);
    const best = candidates[0];
    return {
        classification: best.classification,
        target_failures: best.target_failures,
        suggested_files: best.suggested_files,
        estimated_minutes: best.estimated_minutes,
        priority: best.priority,
    };
}

// CLI entrypoint
if (process.argv[1] && fs.realpathSync(path.resolve(process.argv[1])) === fs.realpathSync(path.resolve(__filename))) {
    const args = process.argv.slice(2);
    let gateResultFile = null;
    let findingsFile = null;

    for (let i = 0; i < args.length; i++) {
        if (args[i] === '--gate-result') gateResultFile = args[i + 1];
        if (args[i] === '--findings') findingsFile = args[i + 1];
    }

    if (!gateResultFile) {
        process.stderr.write('Usage: loop-classifier.mjs --gate-result <file> [--findings <file>]\n');
        process.exit(1);
    }

    let gateJson, findingsMd = '';
    try {
        gateJson = JSON.parse(fs.readFileSync(gateResultFile, 'utf8'));
    } catch (err) {
        process.stderr.write(`loop-classifier: parse error: ${err.message}\n`);
        process.exit(1);
    }

    if (findingsFile && fs.existsSync(findingsFile)) {
        findingsMd = fs.readFileSync(findingsFile, 'utf8');
    }

    const result = classify({ gate_json: gateJson, findings_md: findingsMd, last_3_iterations: [] });
    process.stdout.write(JSON.stringify(result, null, 2) + '\n');
    process.exit(0);
}
