#!/usr/bin/env node
// scripts/lint-playwright-author.mjs
// Playwright spec linter — deterministic RULE_ID checks for disciplined authoring.
//
// lintSpec(specPath, {appSrcGlobs, assignedFile}) -> {ok, findings:[{rule, msg, line}]}
// runWithRetry(authorFn, specPath, MAX=5) -> Promise<any>
//
// RULE_IDs (stable, prefix PW_):
//   PW_MOCK_BANNED       — hard fail: any mock/intercept import or usage
//   PW_SELECTOR_UNVERIFIED — hard fail: selector not verified against app source
//   PW_STRICT_MODE_RISK  — hard fail: unscoped getByText/getByRole without exact:true
//   PW_SHARED_FILE_EDIT  — hard fail: specPath differs from assignedFile
//   PW_SEED_BYPASS       — hard fail: direct DB writes in spec
//   PW_NO_PERSISTENCE_ASSERT — warn: click+submit with no API-state assertion
//
// Export: lintSpec(), runWithRetry()
// CLI:    node lint-playwright-author.mjs <specPath> [--assigned <file>] [--src-globs <glob,...>]

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

// #996/#1003 carry-over: wire selector-evidence.mjs's resolver into the lint
// path so PW_SELECTOR_UNVERIFIED resolves selectors against real source
// (file:line) instead of a naive whole-blob substring scan.
import { extractSelectors, resolveSelector as resolveSelectorEvidence }
    from './selector-evidence.mjs';

const __filename = fileURLToPath(import.meta.url);

// ── RULE_ID severity map ───────────────────────────────────────────────────────
const SEVERITY = {
    PW_MOCK_BANNED: 'fail',
    PW_SELECTOR_UNVERIFIED: 'fail',
    PW_STRICT_MODE_RISK: 'fail',
    PW_SHARED_FILE_EDIT: 'fail',
    PW_SEED_BYPASS: 'fail',
    PW_NO_PERSISTENCE_ASSERT: 'warn',
};

// ── Directive table — corrective message injected on retry ────────────────────
const DIRECTIVES = {
    PW_MOCK_BANNED:
        'Remove all mock/intercept usage (msw, nock, sinon, page.route, route.fulfill, route.abort). ' +
        'Use the real backend — no mocking ever.',
    PW_SELECTOR_UNVERIFIED:
        'Every selector (data-testid, aria-label, role name, getByText literal) must exist in app ' +
        'component source. Verify each selector, add // source: file:line comments for non-obvious ones.',
    PW_STRICT_MODE_RISK:
        'Add {exact:true} to every unscoped page.getByText() and page.getByRole() call, ' +
        'or scope them with a container locator.',
    PW_SHARED_FILE_EDIT:
        'You may only write to your assigned spec file. Do not edit helpers, config, or other specs.',
    PW_SEED_BYPASS:
        'Remove all direct DB writes (prisma., knex(, raw SQL). Use real-API helper functions for seeding.',
    PW_NO_PERSISTENCE_ASSERT:
        'After a UI write (click+submit), add an API-state assertion via the real-API helper ' +
        'to verify the change persisted.',
};

// ── Mock-ban patterns ──────────────────────────────────────────────────────────

// Library names that are banned (checked in import/require source strings)
const BANNED_MOCK_LIBS = ['msw', 'nock', 'sinon'];

// Banned Playwright API patterns (text-level; AST-level concat check is separate)
const BANNED_PW_PATTERNS = [
    /\bpage\.route\s*\(/,
    /[Rr]oute\.fulfill\s*\(/,  // route.fulfill( or someRoute.fulfill(
    /[Rr]oute\.abort\s*\(/,    // route.abort( or someRoute.abort(
];

// String-concat evasion patterns for mock libraries
// Matches: 'ms' + 'w', "ms"+'w', template literals with mock lib fragments
const CONCAT_EVASION_PATTERNS = BANNED_MOCK_LIBS.flatMap(lib => {
    // Split lib into 2-char prefix + remainder: 'ms'+'w', 'no'+'ck', 'si'+'non'
    const splits = [];
    for (let i = 1; i < lib.length; i++) {
        splits.push({
            a: lib.slice(0, i),
            b: lib.slice(i),
        });
    }
    return splits.map(({ a, b }) =>
        new RegExp(`['"\`]${escapeRegex(a)}['"\`]\\s*\\+\\s*['"\`]${escapeRegex(b)}['"\`]`)
    );
});

function escapeRegex(s) {
    return s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

// ── DB seed bypass patterns ────────────────────────────────────────────────────
const DB_SEED_PATTERNS = [
    /\bprisma\s*\./,
    /\bknex\s*\(/,
    /\b(INSERT\s+INTO|DELETE\s+FROM|UPDATE\s+\w+\s+SET)\b/i,
];

// ── Strict-mode risk: unscoped getByText/getByRole without exact:true ──────────
// Matches page.getByText('...') or page.getByRole('...') optionally with options
// but without exact:true
function hasExactTrue(optionsStr) {
    // optionsStr is the text inside the outer { } of the options object
    return /\bexact\s*:\s*true\b/.test(optionsStr);
}

/**
 * Extract the options object text (content inside outermost braces) for a call
 * like getByText('foo', { exact: true }) or getByRole('button', { name: 'x' }).
 * Returns null if no options argument found.
 *
 * @param {string} line
 * @param {number} callEnd - index after the opening paren of the call
 * @returns {string|null}
 */
function extractOptionsText(line, callEnd) {
    // Find the opening { after the first argument's closing quote
    const rest = line.slice(callEnd);
    const braceIdx = rest.indexOf('{');
    if (braceIdx === -1) return null;
    let depth = 0;
    let start = -1;
    for (let i = braceIdx; i < rest.length; i++) {
        if (rest[i] === '{') { depth++; if (start === -1) start = i + 1; }
        else if (rest[i] === '}') {
            depth--;
            if (depth === 0) return rest.slice(start, i);
        }
    }
    return null;
}

/**
 * Check a line for unscoped getByText/getByRole without exact:true.
 * Returns true if it IS a strict-mode risk.
 */
function isStrictModeRisk(line) {
    // Match page.getByText( or page.getByRole(
    const pattern = /\bpage\.(getByText|getByRole)\s*\(/g;
    let match;
    while ((match = pattern.exec(line)) !== null) {
        const afterParen = match.index + match[0].length;
        const opts = extractOptionsText(line, afterParen);
        if (!opts || !hasExactTrue(opts)) {
            return true;
        }
    }
    return false;
}

// ── PW_NO_PERSISTENCE_ASSERT ──────────────────────────────────────────────────
// Heuristic: spec has a .click() call (submit pattern) but no API-state assertion.
// We look for: await *.click() + "submit" or button pattern, then check if
// the spec has any fetch/request/api call pattern after it.
const SUBMIT_PATTERNS = [
    /\.click\(\)/,
    /\.press\(['"]Enter['"]\)/,
];
const API_ASSERT_PATTERNS = [
    /\brequest\./,
    /\bfetch\(/,
    /\bapiContext\./,
    /\/api\//,
    /\.json\(\)/,
    // also accept apiHelper, seedHelper, resetHelper patterns
    /\bapi[A-Z]/,
    /Helper\./,
];

// ── Main lint function ─────────────────────────────────────────────────────────

/**
 * Lint a Playwright spec file against all RULE_IDs.
 *
 * @param {string} specPath - absolute path to the spec file
 * @param {object} opts
 * @param {string[]} opts.appSrcGlobs - globs for app source files (for PW_SELECTOR_UNVERIFIED)
 * @param {string} opts.assignedFile - the author's assigned spec file path (for PW_SHARED_FILE_EDIT)
 * @param {boolean} [opts.resolveSelector] - #1003 carry-over: when true, use
 *   selector-evidence.mjs's resolver (file:line) instead of the naive substring
 *   scan. Requires opts.repoRoot for relative-path computation.
 * @param {string} [opts.repoRoot] - repo root for the evidence resolver.
 * @returns {Promise<{ok: boolean, findings: Array<{rule: string, msg: string, line: number}>}>}
 */
export async function lintSpec(specPath, opts = {}) {
    const { appSrcGlobs = [], assignedFile = null, resolveSelector = false, repoRoot = process.cwd() } = opts;
    const findings = [];

    function addFinding(rule, msg, line) {
        findings.push({ rule, msg, line });
    }

    // ── PW_SHARED_FILE_EDIT ────────────────────────────────────────────────────
    // Check before reading — if the file isn't the assigned file, report it.
    if (assignedFile !== null && path.resolve(specPath) !== path.resolve(assignedFile)) {
        addFinding(
            'PW_SHARED_FILE_EDIT',
            `Author is editing ${specPath} but was assigned to ${assignedFile}. ` +
            'Only edit your assigned spec file.',
            0
        );
        // Still lint the rest for informational findings
    }

    // Read the file
    let text;
    try {
        text = fs.readFileSync(specPath, 'utf8');
    } catch (err) {
        addFinding('PW_MOCK_BANNED', `Could not read spec file: ${err.message}`, 0);
        return { ok: false, findings };
    }

    const lines = text.split('\n');

    // ── PW_MOCK_BANNED — import/require level ─────────────────────────────────
    for (let i = 0; i < lines.length; i++) {
        const line = lines[i];
        const lineNum = i + 1;

        // Check import/require of banned libs
        for (const lib of BANNED_MOCK_LIBS) {
            // import ... from 'msw...' or require('msw...')
            const importPattern = new RegExp(
                `(?:import\\s.*from\\s+['"\`][^'"\`]*\\b${escapeRegex(lib)}\\b|` +
                `require\\s*\\(\\s*['"\`][^'"\`]*\\b${escapeRegex(lib)}\\b)`
            );
            if (importPattern.test(line)) {
                addFinding('PW_MOCK_BANNED', `Banned mock library '${lib}' imported on line ${lineNum}.`, lineNum);
            }
        }

        // Check string-concat evasion
        for (const pat of CONCAT_EVASION_PATTERNS) {
            if (pat.test(line)) {
                addFinding(
                    'PW_MOCK_BANNED',
                    `Possible string-concat evasion of mock library ban detected on line ${lineNum}: ${line.trim()}`,
                    lineNum
                );
            }
        }

        // Check banned Playwright intercept patterns
        for (const pat of BANNED_PW_PATTERNS) {
            if (pat.test(line)) {
                addFinding(
                    'PW_MOCK_BANNED',
                    `Banned network intercept pattern '${pat.source}' on line ${lineNum}.`,
                    lineNum
                );
            }
        }
    }

    // ── PW_STRICT_MODE_RISK ────────────────────────────────────────────────────
    for (let i = 0; i < lines.length; i++) {
        const line = lines[i];
        const lineNum = i + 1;
        if (isStrictModeRisk(line)) {
            addFinding(
                'PW_STRICT_MODE_RISK',
                `Unscoped page.getByText/getByRole without {exact:true} on line ${lineNum}. ` +
                'Add exact:true or scope with a container locator.',
                lineNum
            );
        }
    }

    // ── PW_SEED_BYPASS ─────────────────────────────────────────────────────────
    for (let i = 0; i < lines.length; i++) {
        const line = lines[i];
        const lineNum = i + 1;
        for (const pat of DB_SEED_PATTERNS) {
            if (pat.test(line)) {
                addFinding(
                    'PW_SEED_BYPASS',
                    `Direct DB write pattern '${pat.source}' in spec on line ${lineNum}. ` +
                    'Use real-API helpers for seeding instead.',
                    lineNum
                );
                break; // one finding per line
            }
        }
    }

    // ── PW_NO_PERSISTENCE_ASSERT (warn) ────────────────────────────────────────
    // Heuristic: spec has a submit-pattern (click+submit) but no API-state assertion
    const hasSubmitPattern = lines.some(l => SUBMIT_PATTERNS.some(p => p.test(l)));
    const hasApiAssert = lines.some(l => API_ASSERT_PATTERNS.some(p => p.test(l)));
    if (hasSubmitPattern && !hasApiAssert) {
        // Find the first submit-pattern line for reporting
        const submitLine = lines.findIndex(l => SUBMIT_PATTERNS.some(p => p.test(l))) + 1;
        addFinding(
            'PW_NO_PERSISTENCE_ASSERT',
            `Spec has a UI write (click/submit) on line ${submitLine} but no API-state assertion. ` +
            'Add a real-API assertion to verify the change persisted.',
            submitLine
        );
    }

    // ── PW_SELECTOR_UNVERIFIED ─────────────────────────────────────────────────
    // Only run if appSrcGlobs is non-empty (requires app source access).
    // #1003 carry-over: when resolveSelector=true, delegate to selector-evidence.mjs's
    // resolver (per-selector file:line resolution) so the check actually fires;
    // otherwise fall back to the legacy substring scan.
    if (appSrcGlobs.length > 0) {
        const selectorFindings = resolveSelector
            ? checkSelectorsViaEvidence(text, appSrcGlobs, repoRoot)
            : await checkSelectorsVerified(lines, appSrcGlobs);
        findings.push(...selectorFindings);
    }

    // Determine ok: fails if any hard-fail finding exists
    const ok = !findings.some(f => SEVERITY[f.rule] === 'fail');

    return { ok, findings };
}

/**
 * Check that selectors referenced in the spec exist in app source files.
 * Only called when appSrcGlobs is non-empty.
 *
 * @param {string[]} lines
 * @param {string[]} appSrcGlobs
 * @returns {Promise<Array<{rule, msg, line}>>}
 */
async function checkSelectorsVerified(lines, appSrcGlobs) {
    const findings = [];

    // Collect app source text
    let appSourceText = '';
    const { glob } = await getGlobFn();
    for (const pattern of appSrcGlobs) {
        const files = await glob(pattern);
        for (const f of files) {
            try {
                appSourceText += fs.readFileSync(f, 'utf8') + '\n';
            } catch {
                // skip unreadable
            }
        }
    }

    // Extract selectors from spec lines
    const selectorPatterns = [
        // data-testid="..."
        { re: /data-testid=['"`]([^'"`]+)['"`]/, label: 'data-testid' },
        // aria-label="..."
        { re: /aria-label=['"`]([^'"`]+)['"`]/, label: 'aria-label' },
        // getByText('...')
        { re: /getByText\s*\(\s*['"`]([^'"`]+)['"`]/, label: 'getByText' },
        // getByRole('...', { name: '...' })
        { re: /getByRole\s*\(\s*['"`][^'"`]*['"`]\s*,\s*\{\s*name\s*:\s*['"`]([^'"`]+)['"`]/, label: 'role-name' },
    ];

    for (let i = 0; i < lines.length; i++) {
        const line = lines[i];
        const lineNum = i + 1;
        for (const { re, label } of selectorPatterns) {
            const match = re.exec(line);
            if (match) {
                const selector = match[1];
                // Check if selector appears in app source
                if (!appSourceText.includes(selector)) {
                    findings.push({
                        rule: 'PW_SELECTOR_UNVERIFIED',
                        msg: `${label} selector '${selector}' on line ${lineNum} not found in app source files. ` +
                            'Verify the selector exists or add a // source: file:line comment.',
                        line: lineNum,
                    });
                }
            }
        }
    }

    return findings;
}

/**
 * #1003 carry-over: verify selectors using selector-evidence.mjs's resolver.
 * Each selector referenced in the spec is resolved against app source via
 * resolveSelector(); an unresolved selector (no file:line) becomes a
 * PW_SELECTOR_UNVERIFIED hard-fail finding.
 *
 * @param {string} specText - full spec text
 * @param {string[]} appSrcGlobs - globs/paths for app source
 * @param {string} repoRoot - repo root for relative path computation
 * @returns {Array<{rule, msg, line}>}
 */
function checkSelectorsViaEvidence(specText, appSrcGlobs, repoRoot) {
    const findings = [];
    const selectors = extractSelectors(specText);
    for (const { selector, type, line } of selectors) {
        const evidence = resolveSelectorEvidence(selector, appSrcGlobs, repoRoot);
        if (evidence === null) {
            findings.push({
                rule: 'PW_SELECTOR_UNVERIFIED',
                msg: `${type} selector '${selector}' on line ${line} not found in app source. ` +
                    'Verify the selector exists or add a // source: file:line comment.',
                line,
            });
        }
    }
    return findings;
}

/**
 * Get a glob function (tries built-in glob, falls back to simple fs.readdirSync traversal).
 */
async function getGlobFn() {
    // Try node:fs glob (Node 22+)
    try {
        const { glob } = await import('node:fs/promises');
        if (typeof glob === 'function') {
            return { glob: (pattern) => Array.from(glob(pattern)) };
        }
    } catch { /* not available */ }

    // Try fast-glob or glob package
    try {
        const { glob } = await import('glob');
        return { glob };
    } catch { /* not available */ }

    // Minimal fallback — just return empty (selectors can't be verified without glob)
    return { glob: async () => [] };
}

// ── runWithRetry ───────────────────────────────────────────────────────────────

/**
 * Run an author function with adaptive retry up to MAX attempts.
 * On each failure, extracts RULE_ID findings from the error message and injects
 * corrective directives into the next attempt's call.
 *
 * @param {Function} authorFn - async (specPath, directives: string[]) => any
 * @param {string} specPath
 * @param {number} MAX - maximum attempts (default 5)
 * @returns {Promise<any>} - resolves with authorFn result on success
 * @throws {Error} - if all MAX attempts fail
 */
export async function runWithRetry(authorFn, specPath, MAX = 5) {
    let lastError = null;
    let directives = [];

    for (let attempt = 1; attempt <= MAX; attempt++) {
        try {
            return await authorFn(specPath, directives);
        } catch (err) {
            lastError = err;
            // Extract RULE_IDs from the error message and build directives
            directives = extractDirectives(err.message || '');
            // If no RULE_IDs found in error, pass the raw error message as directive
            if (directives.length === 0 && err.message) {
                directives = [`Fix the following error: ${err.message}`];
            }
        }
    }

    throw new Error(
        `runWithRetry: all ${MAX} attempts failed. Last error: ${lastError ? lastError.message : 'unknown'}. ` +
        'Spec rejected — surface to operator for manual review.'
    );
}

/**
 * Extract corrective directives from an error/findings message by scanning for RULE_IDs.
 *
 * @param {string} message
 * @returns {string[]}
 */
function extractDirectives(message) {
    const ruleIds = Object.keys(DIRECTIVES);
    const found = new Set();
    for (const ruleId of ruleIds) {
        if (message.includes(ruleId)) {
            found.add(ruleId);
        }
    }
    if (found.size === 0) return [];
    return Array.from(found).map(ruleId => `${ruleId}: ${DIRECTIVES[ruleId]}`);
}

// ── CLI entrypoint ─────────────────────────────────────────────────────────────

if (process.argv[1] && fs.existsSync(process.argv[1]) &&
    fs.realpathSync(path.resolve(process.argv[1])) === fs.realpathSync(path.resolve(__filename))) {
    const args = process.argv.slice(2);
    let specPath = null;
    let assignedFile = null;
    const srcGlobs = [];

    for (let i = 0; i < args.length; i++) {
        if (args[i] === '--assigned') assignedFile = args[++i];
        else if (args[i] === '--src-globs') srcGlobs.push(...args[++i].split(','));
        else if (!specPath) specPath = args[i];
    }

    if (!specPath) {
        process.stderr.write('Usage: lint-playwright-author.mjs <specPath> [--assigned <file>] [--src-globs <glob,...>]\n');
        process.exit(1);
    }

    try {
        const result = await lintSpec(path.resolve(specPath), {
            appSrcGlobs: srcGlobs,
            assignedFile: assignedFile ? path.resolve(assignedFile) : path.resolve(specPath),
        });

        process.stdout.write(JSON.stringify(result, null, 2) + '\n');

        if (!result.ok) {
            process.stderr.write(`lint-playwright-author: FAIL — ${result.findings.filter(f => SEVERITY[f.rule] === 'fail').length} hard finding(s)\n`);
            for (const f of result.findings) {
                const sev = SEVERITY[f.rule] || 'fail';
                process.stderr.write(`  [${sev.toUpperCase()}] ${f.rule} (line ${f.line}): ${f.msg}\n`);
            }
            process.exit(1);
        }

        if (result.findings.length > 0) {
            process.stderr.write(`lint-playwright-author: WARN — ${result.findings.length} warning(s)\n`);
            for (const f of result.findings) {
                process.stderr.write(`  [WARN] ${f.rule} (line ${f.line}): ${f.msg}\n`);
            }
        }

        process.exit(0);
    } catch (err) {
        process.stderr.write(`lint-playwright-author: error: ${err.message}\n`);
        process.exit(1);
    }
}
