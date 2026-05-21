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
    return ASSERTION_PATTERNS.some(p => p.test(line));
}

/**
 * Classify a pair of (removed, added) assertion lines.
 * Returns 'LOOSENING' | 'SHIFTING' | 'STRENGTHENING' | null
 */
function classifyChange(removedLine, addedLine) {
    // Check skip/only patterns FIRST — these affect test coverage regardless of assertion presence.
    // e.g. test('x') → test.skip('x') has no assertion keywords but is still LOOSENING.
    const wasSkipped = SKIP_PATTERNS.some(p => p.test(removedLine));
    const isSkipped = SKIP_PATTERNS.some(p => p.test(addedLine));
    if (!wasSkipped && isSkipped) return 'LOOSENING';  // skip added
    if (wasSkipped && !isSkipped) return 'STRENGTHENING';  // skip removed

    // .only added: LOOSENING (reduces test coverage)
    const hadOnly = ONLY_PATTERNS.some(p => p.test(removedLine));
    const hasOnly = ONLY_PATTERNS.some(p => p.test(addedLine));
    if (!hadOnly && hasOnly) return 'LOOSENING';
    if (hadOnly && !hasOnly) return 'STRENGTHENING';

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

    // Check tolerance changes
    for (const tp of TOLERANCE_PATTERNS) {
        const removedMatch = removedLine.match(tp);
        const addedMatch = addedLine.match(tp);
        if (removedMatch && addedMatch) {
            const oldVal = parseFloat(removedMatch[1]);
            const newVal = parseFloat(addedMatch[1]);
            if (isNaN(oldVal) || isNaN(newVal)) break;
            if (newVal > oldVal) return 'LOOSENING';  // wider tolerance
            if (newVal < oldVal) return 'STRENGTHENING';  // tighter tolerance
        }
    }

    // Operator weakening patterns
    // e.g. toStrictEqual → toEqual (looser), toBe → toEqual (looser)
    const operatorRank = {
        'toStrictEqual': 3, 'toBe': 3,
        'toEqual': 2,
        'toMatchObject': 1, 'toContain': 1, 'toMatch': 1,
        'toBeTruthy': 0, 'toBeDefined': 0,
    };
    const removedOp = Object.keys(operatorRank).find(op => removedLine.includes(op));
    const addedOp = Object.keys(operatorRank).find(op => addedLine.includes(op));
    if (removedOp && addedOp && removedOp !== addedOp) {
        const oldRank = operatorRank[removedOp];
        const newRank = operatorRank[addedOp];
        if (newRank < oldRank) return 'LOOSENING';
        if (newRank > oldRank) return 'STRENGTHENING';
    }

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
