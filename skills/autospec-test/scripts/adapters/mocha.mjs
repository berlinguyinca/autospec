// adapters/mocha.mjs
// Assertion-shift adapter for Mocha test files (Chai assertions).
// Export: bucket(fileDiff, filePath) -> Verdict[]

import { bucket as genericBucket } from './generic.mjs';

// Chai BDD operator rank
const CHAI_OPERATOR_RANK = {
    'deep.equal': 4,
    'equal': 3,
    'include': 2,
    'match': 1,
    'exist': 0,
    'ok': 0,
    'true': 3,
    'false': 3,
    'null': 3,
    'undefined': 3,
    'eql': 3,
};

function detectChaiOperator(line) {
    return Object.keys(CHAI_OPERATOR_RANK).find(op => {
        const pattern = new RegExp(`\\.${op.replace('.', '\\.')}\\b`);
        return pattern.test(line);
    });
}

export function bucket(fileDiff, filePath) {
    const verdicts = genericBucket(fileDiff, filePath);

    // Refine with Chai operator ranks
    for (const v of verdicts) {
        if (v.bucket === 'SHIFTING') {
            const [before, after] = v.detail.split(' → ');
            if (before && after) {
                const beforeOp = detectChaiOperator(before);
                const afterOp = detectChaiOperator(after);
                if (beforeOp && afterOp && beforeOp !== afterOp) {
                    const delta = CHAI_OPERATOR_RANK[afterOp] - CHAI_OPERATOR_RANK[beforeOp];
                    if (delta < 0) v.bucket = 'LOOSENING';
                    else if (delta > 0) v.bucket = 'STRENGTHENING';
                }
            }
        }
    }

    return verdicts;
}
