#!/usr/bin/env node
// scripts/findings-generator.mjs
// Non-blocking LLM findings generator for autospec-test.
//
// Reads gate result JSON and emits .autospec/test-findings.md.
// Content-hash gated: identical inputs produce identical output (idempotent).
// In --dry-run mode: writes a static findings stub without calling LLM.
//
// Export: generate(gateResult, outputPath, options) async function
// CLI: node findings-generator.mjs --gate-result <file> --output <file> [--dry-run]

import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import { fileURLToPath } from 'node:url';

const HASH_MARKER = '<!-- autospec-findings-hash:';

/**
 * Compute content hash of gate result for idempotency.
 * @param {object} gateResult
 * @returns {string} hex digest (first 16 chars)
 */
function hashGateResult(gateResult) {
    return crypto
        .createHash('sha256')
        .update(JSON.stringify(gateResult))
        .digest('hex')
        .slice(0, 16);
}

/**
 * Generate static findings markdown (dry-run mode, no LLM call).
 * @param {object} gateResult
 * @param {string} contentHash
 * @returns {string}
 */
function generateDryRunFindings(gateResult, contentHash) {
    const stage = gateResult.stage || 'unknown';
    const passed = gateResult.passed ? 'passed' : 'failed';
    const metrics = gateResult.metrics || {};

    const lines = [
        `${HASH_MARKER} ${contentHash} -->`,
        `<!-- autospec-test-findings -->`,
        `# autospec-test findings`,
        ``,
        `**Stage:** ${stage}  **Result:** ${passed}`,
        ``,
        `## Summary`,
        ``,
        `Gate result: \`passed=${gateResult.passed}\`, stage=\`${stage}\``,
        ``,
    ];

    if (metrics.ui_element_coverage && !metrics.ui_element_coverage.passed) {
        lines.push(`## UI Element Coverage Gaps`);
        lines.push(``);
        const missing = metrics.ui_element_coverage.missing || [];
        for (const m of missing) {
            lines.push(`- **${m.route}**: \`${m.selector}\``);
        }
        lines.push(``);
    }

    if (metrics.behavior_categories && !metrics.behavior_categories.passed) {
        lines.push(`## Missing Behavior Categories`);
        lines.push(``);
        const missingCats = metrics.behavior_categories.missing || [];
        for (const c of missingCats) {
            lines.push(`- \`${c}\``);
        }
        lines.push(``);
    }

    lines.push(`## Suggestions`);
    lines.push(``);
    lines.push(`*(dry-run mode — no LLM analysis performed)*`);
    lines.push(``);

    return lines.join('\n');
}

/**
 * Generate findings using Codex CLI (live mode).
 * Falls back to dry-run content if Codex is not available.
 *
 * @param {object} gateResult
 * @param {string} contentHash
 * @returns {Promise<string>}
 */
async function generateLLMFindings(gateResult, contentHash) {
    // Check for Codex CLI availability
    const { execFile } = await import('node:child_process');
    const { promisify } = await import('node:util');
    const execFileAsync = promisify(execFile);

    try {
        await execFileAsync('codex', ['--version'], { timeout: 5000 });
    } catch {
        // Codex not available — fall back to dry-run
        return generateDryRunFindings(gateResult, contentHash);
    }

    const prompt = [
        `You are reviewing an autospec E2E gate result. Provide concise, actionable suggestions.`,
        `Gate result: ${JSON.stringify(gateResult, null, 2)}`,
        `Output a brief markdown section with specific suggestions to fix each failure.`,
        `Keep your response under 500 words.`,
    ].join('\n');

    try {
        const { stdout } = await execFileAsync(
            'codex',
            ['--quiet', '--model', 'gpt-4.1', prompt],
            { timeout: 60000 }
        );
        return [
            `${HASH_MARKER} ${contentHash} -->`,
            `<!-- autospec-test-findings -->`,
            `# autospec-test findings`,
            ``,
            stdout.trim(),
            ``,
        ].join('\n');
    } catch {
        return generateDryRunFindings(gateResult, contentHash);
    }
}

/**
 * Generate findings file, idempotent on identical gate results.
 *
 * @param {object} gateResult - parsed gate result JSON
 * @param {string} outputPath - path to write .autospec/test-findings.md
 * @param {{dryRun?: boolean}} [options]
 * @returns {Promise<{action: 'written'|'skipped', outputPath: string, contentHash: string}>}
 */
export async function generate(gateResult, outputPath, options = {}) {
    const contentHash = hashGateResult(gateResult);

    // Idempotency: if output exists and hash matches, skip
    if (fs.existsSync(outputPath)) {
        const existing = fs.readFileSync(outputPath, 'utf8');
        if (existing.includes(`${HASH_MARKER} ${contentHash} -->`)) {
            return { action: 'skipped', outputPath, contentHash };
        }
    }

    // Ensure output directory exists
    const outputDir = path.dirname(outputPath);
    if (!fs.existsSync(outputDir)) {
        fs.mkdirSync(outputDir, { recursive: true });
    }

    const dryRun = options.dryRun === true;
    const content = dryRun
        ? generateDryRunFindings(gateResult, contentHash)
        : await generateLLMFindings(gateResult, contentHash);

    fs.writeFileSync(outputPath, content, 'utf8');
    return { action: 'written', outputPath, contentHash };
}

// CLI entrypoint
const __filename = fileURLToPath(import.meta.url);
if (process.argv[1] && fs.realpathSync(path.resolve(process.argv[1])) === fs.realpathSync(path.resolve(__filename))) {
    const args = process.argv.slice(2);
    let gateResultFile = null;
    let outputPath = '.autospec/test-findings.md';
    let dryRun = false;

    for (let i = 0; i < args.length; i++) {
        if (args[i] === '--gate-result') gateResultFile = args[i + 1];
        if (args[i] === '--output') outputPath = args[i + 1];
        if (args[i] === '--dry-run') dryRun = true;
    }

    if (!gateResultFile) {
        process.stderr.write('Usage: findings-generator.mjs --gate-result <file> --output <file> [--dry-run]\n');
        process.exit(1);
    }

    let gateResult;
    try {
        gateResult = JSON.parse(fs.readFileSync(gateResultFile, 'utf8'));
    } catch (err) {
        process.stderr.write(`findings-generator: parse error: ${err.message}\n`);
        process.exit(1);
    }

    try {
        const result = await generate(gateResult, outputPath, { dryRun });
        process.stdout.write(JSON.stringify(result, null, 2) + '\n');
        process.exit(0);
    } catch (err) {
        process.stderr.write(`findings-generator: error: ${err.message}\n`);
        process.exit(1);
    }
}
