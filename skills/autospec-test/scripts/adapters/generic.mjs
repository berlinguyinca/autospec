// adapters/generic.mjs
// Generic assertion-shift adapter — text/regex based (no AST).
// Used as fallback when no framework-specific adapter is available,
// and as shared logic for JS/TS adapters.
//
// Export: bucket(fileDiff, filePath) -> Verdict[]

/**
 * Assertion patterns for generic detection.
 * Each entry: { pattern, type }
 *   type: 'assert' | 'tolerance'
 */
const ASSERTION_PATTERNS = [
    // Jest/Vitest/Mocha expect style
    /\bexpect\s*\(.+\)\s*\./,
    // Chai assert
    /\bassert\s*\.\w+\s*\(/,
    // Node assert
    /\bassert(?:\.strict)?\s*\(/,
    // Python assert
    /^\s*assert\s+/,
    // Go testing
    /\bt\.(?:Error|Fatal|Fail|Equal|NotEqual|True|False|Nil|NotNil)\b/,
    // Rust assert
    /\bassert(?:_eq|_ne|_matches|!)?\s*!/,
    // Playwright expect
    /\bexpect\s*\(.+\)\.to(?:Be|Have|Match|Contain|Equal|Pass|Throw)/,
];

const TOLERANCE_PATTERNS = [
    // numeric tolerance widening: toBeCloseTo, toAlmostEqual, delta
    /\btoBeCloseTo\s*\([^,]+,\s*(\d+)\)/,
    /\bdelta\s*[=:]\s*([\d.]+)/,
    /\bprecision\s*[=:]\s*([\d.]+)/,
    /\balmost\s*(?:equal|equals)\s*\([^,]+,\s*([\d.]+)/i,
];

const SKIP_PATTERNS = [
    /\btest\.skip\b/,
    /\bit\.skip\b/,
    /\bdescribe\.skip\b/,
    /\bxtest\b/,
    /\bxit\b/,
    /\bxdescribe\b/,
    /\b@pytest\.mark\.skip\b/,
    /\bt\.Skip\b/,
    /\b#\[ignore\]/,
];

const ONLY_PATTERNS = [
    /\btest\.only\b/,
    /\bit\.only\b/,
    /\bdescribe\.only\b/,
    /\bftest\b/,
    /\bfit\b/,
];

/**
 * Check if a line contains an assertion.
 */
function isAssertion(line) {
    return matchesAny(ASSERTION_PATTERNS, line);
}

function matchesAny(patterns, line) {
    return patterns.some(pattern => pattern.test(line));
}

function classifyCoverageChange(removedLine, addedLine) {
    const wasSkipped = matchesAny(SKIP_PATTERNS, removedLine);
    const isSkipped = matchesAny(SKIP_PATTERNS, addedLine);
    if (!wasSkipped && isSkipped) return 'LOOSENING';
    if (wasSkipped && !isSkipped) return 'STRENGTHENING';

    const hadOnly = matchesAny(ONLY_PATTERNS, removedLine);
    const hasOnly = matchesAny(ONLY_PATTERNS, addedLine);
    if (!hadOnly && hasOnly) return 'LOOSENING';
    if (hadOnly && !hasOnly) return 'STRENGTHENING';
    return null;
}

function classifyToleranceChange(removedLine, addedLine) {
    for (const pattern of TOLERANCE_PATTERNS) {
        const removedMatch = removedLine.match(pattern);
        const addedMatch = addedLine.match(pattern);
        if (!removedMatch || !addedMatch) continue;

        const oldValue = parseFloat(removedMatch[1]);
        const newValue = parseFloat(addedMatch[1]);
        if (Number.isNaN(oldValue) || Number.isNaN(newValue)) return null;
        if (newValue > oldValue) return 'LOOSENING';
        if (newValue < oldValue) return 'STRENGTHENING';
    }
    return null;
}

const OPERATOR_RANK = {
    toStrictEqual: 3,
    toBe: 3,
    toEqual: 2,
    toMatchObject: 1,
    toContain: 1,
    toMatch: 1,
    toBeTruthy: 0,
    toBeDefined: 0,
};

function classifyOperatorChange(removedLine, addedLine) {
    const removedOperator = Object.keys(OPERATOR_RANK).find(operator => removedLine.includes(operator));
    const addedOperator = Object.keys(OPERATOR_RANK).find(operator => addedLine.includes(operator));
    if (!removedOperator || !addedOperator || removedOperator === addedOperator) return null;

    const oldRank = OPERATOR_RANK[removedOperator];
    const newRank = OPERATOR_RANK[addedOperator];
    if (newRank < oldRank) return 'LOOSENING';
    if (newRank > oldRank) return 'STRENGTHENING';
    return null;
}

/**
 * Classify a pair of (removed, added) assertion lines.
 * Returns 'LOOSENING' | 'SHIFTING' | 'STRENGTHENING' | null
 */
function classifyChange(removedLine, addedLine) {
    const coverageChange = classifyCoverageChange(removedLine, addedLine);
    if (coverageChange) return coverageChange;

    // Pure selector / comment fix — no assertion keyword on either side
    if (!isAssertion(removedLine) && !isAssertion(addedLine)) return null;

    // Assertion removed → LOOSENING
    if (isAssertion(removedLine) && !isAssertion(addedLine)) {
        return 'LOOSENING';
    }

    // Assertion added → STRENGTHENING
    if (!isAssertion(removedLine) && isAssertion(addedLine)) {
        return 'STRENGTHENING';
    }

    const toleranceChange = classifyToleranceChange(removedLine, addedLine);
    if (toleranceChange) return toleranceChange;

    const operatorChange = classifyOperatorChange(removedLine, addedLine);
    if (operatorChange) return operatorChange;

    // Same operator/type — value-only change → SHIFTING
    if (removedLine.trim() !== addedLine.trim()) {
        return 'SHIFTING';
    }

    return null;
}

/**
 * Bucket assertion changes from a file diff.
 *
 * @param {{added: {line: number, content: string}[], removed: {line: number, content: string}[], hunks: any[]}} fileDiff
 * @param {string} filePath
 * @returns {import('../assertion-shift-classifier.mjs').Verdict[]}
 */
export function bucket(fileDiff, filePath) {
    const verdicts = [];
    const framework = detectFramework(filePath);

    // Match removed/added lines within each hunk
    for (const hunk of (fileDiff.hunks || [])) {
        const removed = hunk.removed || [];
        const added = hunk.added || [];

        // Simple pairing: match removed↔added lines within hunk
        const maxPairs = Math.max(removed.length, added.length);

        for (let i = 0; i < maxPairs; i++) {
            const rem = removed[i];
            const add = added[i];

            if (rem && add) {
                // Changed line
                const bucket = classifyChange(rem.content, add.content);
                if (bucket) {
                    verdicts.push({
                        file: filePath,
                        line: add.line || hunk.startLine || 0,
                        bucket,
                        framework,
                        detail: `${rem.content.trim()} → ${add.content.trim()}`,
                    });
                }
            } else if (rem && !add) {
                // Purely removed line
                if (isAssertion(rem.content)) {
                    verdicts.push({
                        file: filePath,
                        line: hunk.startLine || 0,
                        bucket: 'LOOSENING',
                        framework,
                        detail: `assertion removed: ${rem.content.trim()}`,
                    });
                }
            } else if (!rem && add) {
                // Purely added line
                if (isAssertion(add.content)) {
                    verdicts.push({
                        file: filePath,
                        line: add.line || hunk.startLine || 0,
                        bucket: 'STRENGTHENING',
                        framework,
                        detail: `assertion added: ${add.content.trim()}`,
                    });
                }
            }
        }
    }

    return verdicts;
}

function detectFramework(filePath) {
    if (/\.(spec|test)\.(ts|tsx)$/.test(filePath)) return 'playwright';
    if (/test_.*\.py$|.*_test\.py$/.test(filePath)) return 'pytest';
    if (/_test\.go$/.test(filePath)) return 'go-test';
    if (/\.rs$/.test(filePath)) return 'cargo-test';
    return 'jest';
}
