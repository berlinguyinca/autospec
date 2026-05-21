// adapters/playwright.mjs
// Assertion-shift adapter for Playwright test files (.spec.ts, .test.ts, etc.)
// Export: bucket(fileDiff, filePath) -> Verdict[]

import { bucket as genericBucket } from './generic.mjs';

// Playwright-specific assertion operators ranked by strictness
const PW_OPERATOR_RANK = {
    'toStrictEqual': 5,
    'toBe': 4,
    'toEqual': 3,
    'toMatchObject': 2,
    'toContain': 2,
    'toHaveText': 2,
    'toHaveValue': 2,
    'toMatch': 1,
    'toBeTruthy': 0,
    'toBeDefined': 0,
    'toBeVisible': 2,
    'toBeEnabled': 2,
    'toBeChecked': 2,
    'toBeDisabled': 1,
    'toBeHidden': 1,
};

// Playwright-specific LOOSENING patterns
const PW_LOOSENING = [
    // Removing timeout (makes test flakier)
    /timeout\s*:\s*\d+/,
    // Adding { timeout: large } to expect
    /\.toHave.*\{\s*timeout\s*:\s*(\d+)/,
];

/**
 * Playwright-specific bucket function.
 * Extends generic with Playwright operator rankings.
 */
export function bucket(fileDiff, filePath) {
    // Use generic bucketing as base
    const verdicts = genericBucket(fileDiff, filePath);

    // Post-process: refine verdicts using Playwright-specific operator ranks
    for (const v of verdicts) {
        if (v.bucket === 'SHIFTING') {
            // Check if operator changed rank
            const [before, after] = v.detail.split(' → ');
            if (before && after) {
                const beforeOp = Object.keys(PW_OPERATOR_RANK).find(op => before.includes(op));
                const afterOp = Object.keys(PW_OPERATOR_RANK).find(op => after.includes(op));
                if (beforeOp && afterOp && beforeOp !== afterOp) {
                    const delta = PW_OPERATOR_RANK[afterOp] - PW_OPERATOR_RANK[beforeOp];
                    if (delta < 0) v.bucket = 'LOOSENING';
                    else if (delta > 0) v.bucket = 'STRENGTHENING';
                }
            }
        }
    }

    // Check for timeout increases in added lines (LOOSENING)
    for (const hunk of (fileDiff.hunks || [])) {
        for (const add of (hunk.added || [])) {
            if (PW_LOOSENING[1].test(add.content)) {
                const timeoutMatch = add.content.match(/timeout\s*:\s*(\d+)/);
                if (timeoutMatch && parseInt(timeoutMatch[1], 10) > 30000) {
                    verdicts.push({
                        file: filePath,
                        line: add.line || hunk.startLine || 0,
                        bucket: 'LOOSENING',
                        framework: 'playwright',
                        detail: `timeout increased to ${timeoutMatch[1]}ms in expect`,
                    });
                }
            }
        }
    }

    return verdicts;
}
