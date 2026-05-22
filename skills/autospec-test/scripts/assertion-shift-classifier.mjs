#!/usr/bin/env node
// scripts/assertion-shift-classifier.mjs
// AST-based assertion-shift classifier for autospec-test.
//
// Buckets every modified test-file assertion as:
//   LOOSENING   — tolerance widened, type weakened, assertion removed
//   SHIFTING    — value-only shift, same operator/type
//   STRENGTHENING — tolerance tightened, check added
//
// Per-bucket policy (spec §5c):
//   LOOSENING   → blocks gate (label needs-human-review + e2e:assertion-loosening)
//   SHIFTING    → blocks unless (co-edited non-test file AND JUSTIFICATION: in commit message)
//   STRENGTHENING → always passes
//
// Export: classify(options) async function
// CLI: node assertion-shift-classifier.mjs --repo <dir> --base <ref> --head <ref>

import fs from 'node:fs';
import path from 'node:path';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { fileURLToPath } from 'node:url';

const execFileAsync = promisify(execFile);
const __filename = fileURLToPath(import.meta.url);
const ADAPTERS_DIR = path.join(path.dirname(__filename), 'adapters');

// v2 contract diff classifier — loaded lazily when .autospec/test.yml is in the diff
let _v2BucketsModule = null;
async function loadV2Buckets() {
    if (_v2BucketsModule) return _v2BucketsModule;
    try {
        const v2Path = path.join(path.dirname(__filename), 'assertion-shift-v2-buckets.mjs');
        _v2BucketsModule = await import(`file://${v2Path}`);
    } catch {
        _v2BucketsModule = null;
    }
    return _v2BucketsModule;
}

/**
 * @typedef {Object} Verdict
 * @property {string} file       - relative path to the test file
 * @property {number} line       - line number of the assertion change (1-based)
 * @property {'LOOSENING'|'SHIFTING'|'STRENGTHENING'} bucket
 * @property {string} framework  - detected test framework
 * @property {string} detail     - human-readable description of the change
 */

/**
 * @typedef {Object} GateResult
 * @property {boolean} passed
 * @property {string} reason     - machine-readable reason code (when !passed)
 * @property {Verdict[]} verdicts
 * @property {string[]} loosening_files
 * @property {string[]} shifting_unjustified_files
 */

// Framework detection patterns
const FRAMEWORK_PATTERNS = {
    playwright: /\.(spec|test)\.(ts|js|mjs)$/,
    jest:       /\.(test|spec)\.(ts|js|tsx|jsx)$/,
    vitest:     /\.(test|spec)\.(ts|js|mjs|tsx|jsx)$/,
    mocha:      /\.(test|spec)\.js$/,
    pytest:     /test_.*\.py$|.*_test\.py$/,
    'go-test':  /_test\.go$/,
    'cargo-test': /\.rs$/,
};

// Framework detection order (more specific first)
const FRAMEWORK_ORDER = ['playwright', 'pytest', 'go-test', 'cargo-test', 'vitest', 'jest', 'mocha'];

/**
 * Detect which test framework a file belongs to.
 * @param {string} filePath
 * @returns {string} framework name
 */
function detectFramework(filePath) {
    for (const fw of FRAMEWORK_ORDER) {
        if (FRAMEWORK_PATTERNS[fw].test(filePath)) {
            return fw;
        }
    }
    return 'jest'; // default
}

/**
 * Run git diff to get modified lines for a file.
 * Returns {added: string[], removed: string[], hunks: Array<{added, removed, startLine}>}
 */
async function getFileDiff(repoRoot, baseRef, headRef, filePath) {
    try {
        const { stdout } = await execFileAsync('git', [
            '-C', repoRoot,
            'diff', `${baseRef}..${headRef}`, '--', filePath
        ], { maxBuffer: 10 * 1024 * 1024 });
        return parseDiff(stdout);
    } catch {
        return { added: [], removed: [], hunks: [] };
    }
}

/**
 * Parse unified diff output into added/removed lines with line numbers.
 */
function parseDiff(diffText) {
    const added = [];
    const removed = [];
    const hunks = [];

    let currentHunk = null;
    let newLineNum = 0;

    for (const line of diffText.split('\n')) {
        if (line.startsWith('@@')) {
            // @@ -old_start,old_count +new_start,new_count @@
            const match = line.match(/\+(\d+)/);
            newLineNum = match ? parseInt(match[1], 10) : 0;
            currentHunk = { added: [], removed: [], startLine: newLineNum };
            hunks.push(currentHunk);
        } else if (line.startsWith('+') && !line.startsWith('+++')) {
            const content = line.slice(1);
            added.push({ line: newLineNum, content });
            if (currentHunk) currentHunk.added.push({ line: newLineNum, content });
            newLineNum++;
        } else if (line.startsWith('-') && !line.startsWith('---')) {
            const content = line.slice(1);
            removed.push({ line: 0, content }); // removed lines don't have new line numbers
            if (currentHunk) currentHunk.removed.push({ line: 0, content });
        } else if (!line.startsWith('\\')) {
            newLineNum++;
        }
    }

    return { added, removed, hunks };
}

/**
 * Get the list of files changed between two refs.
 * @returns {Promise<{testFiles: string[], nonTestFiles: string[]}>}
 */
async function getChangedFiles(repoRoot, baseRef, headRef) {
    try {
        const { stdout } = await execFileAsync('git', [
            '-C', repoRoot,
            'diff', '--name-only', `${baseRef}..${headRef}`
        ]);
        const files = stdout.trim().split('\n').filter(Boolean);
        const testFiles = files.filter(f => isTestFile(f));
        const nonTestFiles = files.filter(f => !isTestFile(f));
        return { testFiles, nonTestFiles };
    } catch {
        return { testFiles: [], nonTestFiles: [] };
    }
}

/**
 * Determine if a file is a test file.
 */
function isTestFile(filePath) {
    return Object.values(FRAMEWORK_PATTERNS).some(pat => pat.test(filePath)) ||
        /[/\\]tests?[/\\]/.test(filePath) ||
        /[/\\]__tests?__[/\\]/.test(filePath) ||
        /[/\\]spec[/\\]/.test(filePath);
}

/**
 * Get commit messages between base and head refs.
 */
async function getCommitMessages(repoRoot, baseRef, headRef) {
    try {
        const { stdout } = await execFileAsync('git', [
            '-C', repoRoot,
            'log', '--format=%B', `${baseRef}..${headRef}`
        ]);
        return stdout;
    } catch {
        return '';
    }
}

/**
 * Main classifier entry point.
 *
 * @param {object} options
 * @param {string} options.repoRoot  - absolute path to the target repo
 * @param {string} options.baseRef   - base git ref (e.g. 'main', 'origin/main')
 * @param {string} options.headRef   - head git ref (e.g. 'HEAD', 'pr-branch')
 * @param {string} [options.diffText] - pre-computed diff text (for unit tests)
 * @returns {Promise<{verdicts: Verdict[], gate: GateResult}>}
 */
export async function classify(options) {
    const { repoRoot, baseRef = 'HEAD~1', headRef = 'HEAD', diffText } = options;

    // Get changed files — in diffText (unit test) mode, also accept nonTestFilesChanged override
    let testFiles, nonTestFiles;
    if (diffText !== undefined) {
        const parsed = parseFilesFromDiff(diffText);
        testFiles = parsed.testFiles;
        // Accept explicit nonTestFilesChanged for unit tests (avoids git calls)
        nonTestFiles = Array.isArray(options.nonTestFilesChanged)
            ? options.nonTestFilesChanged
            : parsed.nonTestFiles;
    } else {
        ({ testFiles, nonTestFiles } = await getChangedFiles(repoRoot, baseRef, headRef));
    }

    // Get commit messages to check for JUSTIFICATION:
    const commitMessages = diffText !== undefined
        ? (options.commitMessages || '')
        : await getCommitMessages(repoRoot, baseRef, headRef);

    const hasJustification = /JUSTIFICATION:\s*.+/m.test(commitMessages);
    const hasCoEdit = nonTestFiles.length > 0;

    const allVerdicts = [];

    for (const testFile of testFiles) {
        const framework = detectFramework(testFile);
        let fileDiff;

        if (diffText) {
            // Extract per-file diff from combined diff text
            fileDiff = extractFileDiff(diffText, testFile);
        } else {
            fileDiff = await getFileDiff(repoRoot, baseRef, headRef, testFile);
        }

        // Load framework adapter
        let adapter;
        try {
            const adapterPath = path.join(ADAPTERS_DIR, `${framework}.mjs`);
            adapter = await import(`file://${adapterPath}`);
        } catch {
            // Fall back to generic adapter
            try {
                const genericPath = path.join(ADAPTERS_DIR, 'generic.mjs');
                adapter = await import(`file://${genericPath}`);
            } catch {
                continue;
            }
        }

        // Run adapter
        const verdicts = adapter.bucket(fileDiff, testFile);
        allVerdicts.push(...verdicts);
    }

    // v2 second pass: if .autospec/test.yml appears in the diff, classify contract changes
    const contractFile = '.autospec/test.yml';
    const diffSource = diffText !== undefined ? diffText : null;
    let contractDiffPresent = false;
    if (diffSource !== null) {
        contractDiffPresent = diffSource.includes(`a/${contractFile}`) || diffSource.includes(`b/${contractFile}`);
    } else {
        // Check git diff for contract file
        try {
            const { stdout: contractDiff } = await execFileAsync('git', [
                '-C', repoRoot, 'diff', `${baseRef}..${headRef}`, '--', contractFile
            ]);
            if (contractDiff.trim().length > 0) {
                contractDiffPresent = true;
                // stash for v2 classifier
                options._contractDiff = contractDiff;
            }
        } catch { /* ignore */ }
    }

    if (contractDiffPresent) {
        const v2Module = await loadV2Buckets();
        if (v2Module && typeof v2Module.classifyV2ContractDiff === 'function') {
            try {
                // Extract before/after YAML from the diff or git show
                let beforeYaml = '';
                let afterYaml = '';
                if (diffSource !== null || options._contractDiff) {
                    const rawDiff = diffSource !== null ? diffSource : options._contractDiff;
                    // Extract lines from the contract file hunk
                    const escaped = contractFile.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
                    const fileStart = new RegExp(`^diff --git a/${escaped}`, 'm');
                    const startIdx = rawDiff.search(fileStart);
                    if (startIdx !== -1) {
                        const nextDiff = rawDiff.indexOf('\ndiff --git', startIdx + 1);
                        const section = nextDiff === -1 ? rawDiff.slice(startIdx) : rawDiff.slice(startIdx, nextDiff);
                        const beforeLines = [];
                        const afterLines = [];
                        for (const line of section.split('\n')) {
                            if (line.startsWith('-') && !line.startsWith('---')) beforeLines.push(line.slice(1));
                            else if (line.startsWith('+') && !line.startsWith('+++')) afterLines.push(line.slice(1));
                            else if (!line.startsWith('@@') && !line.startsWith('diff') && !line.startsWith('index')) {
                                const ctx = line.startsWith(' ') ? line.slice(1) : line;
                                beforeLines.push(ctx);
                                afterLines.push(ctx);
                            }
                        }
                        beforeYaml = beforeLines.join('\n');
                        afterYaml = afterLines.join('\n');
                    }
                } else {
                    try {
                        const { stdout: bef } = await execFileAsync('git', ['-C', repoRoot, 'show', `${baseRef}:${contractFile}`]);
                        beforeYaml = bef;
                    } catch { /* file may be new */ }
                    try {
                        const { stdout: aft } = await execFileAsync('git', ['-C', repoRoot, 'show', `${headRef}:${contractFile}`]);
                        afterYaml = aft;
                    } catch { /* file may be deleted */ }
                }
                const v2Verdicts = await v2Module.classifyV2ContractDiff({ beforeYaml, afterYaml });
                allVerdicts.push(...v2Verdicts);
            } catch { /* v2 classifier error — degrade gracefully, do not block */ }
        }
    }

    // Compute gate decision
    const looseningVerdicts = allVerdicts.filter(v => v.bucket === 'LOOSENING');
    const shiftingVerdicts = allVerdicts.filter(v => v.bucket === 'SHIFTING');

    const looseningFiles = [...new Set(looseningVerdicts.map(v => v.file))];
    const shiftingFiles = [...new Set(shiftingVerdicts.map(v => v.file))];

    // SHIFTING passes only when co-edit AND JUSTIFICATION: both present
    const shiftingJustified = hasJustification && hasCoEdit;
    const shiftingUnjustifiedFiles = shiftingJustified ? [] : shiftingFiles;

    const passed = looseningFiles.length === 0 && shiftingUnjustifiedFiles.length === 0;

    let reason = '';
    if (!passed) {
        if (looseningFiles.length > 0 && shiftingUnjustifiedFiles.length > 0) {
            reason = 'loosening_and_unjustified_shift';
        } else if (looseningFiles.length > 0) {
            reason = 'assertion_loosening';
        } else {
            reason = 'unjustified_assertion_shift';
        }
    }

    const gate = {
        passed,
        reason,
        verdicts: allVerdicts,
        loosening_files: looseningFiles,
        shifting_unjustified_files: shiftingUnjustifiedFiles,
        has_justification: hasJustification,
        has_co_edit: hasCoEdit,
    };

    return { verdicts: allVerdicts, gate };
}

/**
 * Parse test/non-test files from a combined diff string.
 */
function parseFilesFromDiff(diffText) {
    const testFiles = [];
    const nonTestFiles = [];
    const filePattern = /^diff --git a\/(.+) b\//gm;
    let match;
    while ((match = filePattern.exec(diffText)) !== null) {
        const f = match[1];
        if (isTestFile(f)) testFiles.push(f);
        else nonTestFiles.push(f);
    }
    return { testFiles, nonTestFiles };
}

/**
 * Extract the diff section for a specific file from a combined diff.
 */
function extractFileDiff(diffText, filePath) {
    const escaped = filePath.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
    const fileStart = new RegExp(`^diff --git a/${escaped}`, 'm');
    const startIdx = diffText.search(fileStart);
    if (startIdx === -1) return { added: [], removed: [], hunks: [] };

    const nextDiff = diffText.indexOf('\ndiff --git', startIdx + 1);
    const section = nextDiff === -1 ? diffText.slice(startIdx) : diffText.slice(startIdx, nextDiff);
    return parseDiff(section);
}

// CLI entrypoint
if (process.argv[1] && fs.realpathSync(path.resolve(process.argv[1])) === fs.realpathSync(path.resolve(__filename))) {
    const args = process.argv.slice(2);
    let repoRoot = process.cwd();
    let baseRef = 'HEAD~1';
    let headRef = 'HEAD';

    for (let i = 0; i < args.length; i++) {
        if (args[i] === '--repo') repoRoot = args[i + 1];
        if (args[i] === '--base') baseRef = args[i + 1];
        if (args[i] === '--head') headRef = args[i + 1];
    }

    try {
        const result = await classify({ repoRoot, baseRef, headRef });
        process.stdout.write(JSON.stringify(result, null, 2) + '\n');
        process.exit(result.gate.passed ? 0 : 1);
    } catch (err) {
        process.stderr.write(`assertion-shift-classifier: error: ${err.message}\n`);
        process.exit(2);
    }
}
