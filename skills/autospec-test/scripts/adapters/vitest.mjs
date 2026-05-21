// adapters/vitest.mjs
// Assertion-shift adapter for Vitest test files.
// Export: bucket(fileDiff, filePath) -> Verdict[]

import { bucket as genericBucket } from './generic.mjs';

// Vitest-specific operator rank (subset differs from jest)
const VT_OPERATOR_RANK = {
    'toStrictEqual': 5,
    'toBe': 4,
    'toEqual': 3,
    'toMatchObject': 2,
    'toContain': 2,
    'toMatch': 1,
    'toBeTruthy': 0,
    'toBeDefined': 0,
    'toSatisfy': 2,
};

export function bucket(fileDiff, filePath) {
    const verdicts = genericBucket(fileDiff, filePath);

    // Refine SHIFTING verdicts using Vitest operator ranks
    for (const v of verdicts) {
        if (v.bucket === 'SHIFTING') {
            const [before, after] = v.detail.split(' → ');
            if (before && after) {
                const beforeOp = Object.keys(VT_OPERATOR_RANK).find(op => before.includes(op));
                const afterOp = Object.keys(VT_OPERATOR_RANK).find(op => after.includes(op));
                if (beforeOp && afterOp && beforeOp !== afterOp) {
                    const delta = VT_OPERATOR_RANK[afterOp] - VT_OPERATOR_RANK[beforeOp];
                    if (delta < 0) v.bucket = 'LOOSENING';
                    else if (delta > 0) v.bucket = 'STRENGTHENING';
                }
            }
        }
    }

    return verdicts;
}
