// adapters/go-test.mjs
// Assertion-shift adapter for Go test files (*_test.go).
// Export: bucket(fileDiff, filePath) -> Verdict[]

// Go testing assertion patterns
const GO_ASSERT_PATTERNS = [
    /\bt\.(?:Error|Errorf|Fatal|Fatalf|Fail|FailNow)\s*\(/,
    /\bt\.(?:Equal|NotEqual|True|False|Nil|NotNil|Contains|Len)\s*\(/,
    /\brequire\.(?:Equal|NotEqual|True|False|Nil|NotNil|Contains|Error|NoError)\s*\(/,
    /\bassert\.(?:Equal|NotEqual|True|False|Nil|NotNil|Contains|Error|NoError)\s*\(/,
    /\bif\s+.*!=.*\{\s*t\.(?:Error|Fatal)/,
];

// Go assertion strictness rank
const GO_ASSERT_RANK = {
    'require.Equal': 5,
    'assert.Equal': 4,
    'require.True': 3,
    'assert.True': 3,
    'require.Contains': 2,
    'assert.Contains': 2,
    'require.Error': 3,
    'assert.Error': 3,
    't.Fatal': 4,
    't.Error': 2,
    't.Fail': 1,
};

// Go skip patterns → LOOSENING
const GO_SKIP = [
    /\bt\.Skip\s*\(/,
    /\bt\.Skipf\s*\(/,
    /\/\/ t\.Errorf/,  // commented-out assertions
];

function isGoAssertion(line) {
    return GO_ASSERT_PATTERNS.some(p => p.test(line));
}

function detectGoAssertOp(line) {
    return Object.keys(GO_ASSERT_RANK).find(op => line.includes(op));
}

function classifyGoChange(removedLine, addedLine) {
    // Check skip FIRST — t.Skip() lines are not assertions
    if (GO_SKIP.some(p => p.test(addedLine)) && !GO_SKIP.some(p => p.test(removedLine))) {
        return 'LOOSENING';
    }
    if (GO_SKIP.some(p => p.test(removedLine)) && !GO_SKIP.some(p => p.test(addedLine))) {
        return 'STRENGTHENING';
    }

    const wasAssert = isGoAssertion(removedLine);
    const isAssert = isGoAssertion(addedLine);

    if (wasAssert && !isAssert) {
        return 'LOOSENING';
    }
    if (!wasAssert && isAssert) return 'STRENGTHENING';

    // Operator rank change
    const beforeOp = detectGoAssertOp(removedLine);
    const afterOp = detectGoAssertOp(addedLine);
    if (beforeOp && afterOp && beforeOp !== afterOp) {
        const delta = GO_ASSERT_RANK[afterOp] - GO_ASSERT_RANK[beforeOp];
        if (delta < 0) return 'LOOSENING';
        if (delta > 0) return 'STRENGTHENING';
    }

    if (wasAssert && isAssert && removedLine.trim() !== addedLine.trim()) {
        return 'SHIFTING';
    }

    return null;
}

export function bucket(fileDiff, filePath) {
    const verdicts = [];

    for (const hunk of (fileDiff.hunks || [])) {
        const removed = hunk.removed || [];
        const added = hunk.added || [];
        const maxPairs = Math.max(removed.length, added.length);

        for (let i = 0; i < maxPairs; i++) {
            const rem = removed[i];
            const add = added[i];

            if (rem && add) {
                const b = classifyGoChange(rem.content, add.content);
                if (b) {
                    verdicts.push({
                        file: filePath,
                        line: add.line || hunk.startLine || 0,
                        bucket: b,
                        framework: 'go-test',
                        detail: `${rem.content.trim()} → ${add.content.trim()}`,
                    });
                }
            } else if (rem && !add) {
                if (isGoAssertion(rem.content)) {
                    verdicts.push({
                        file: filePath,
                        line: hunk.startLine || 0,
                        bucket: 'LOOSENING',
                        framework: 'go-test',
                        detail: `assertion removed: ${rem.content.trim()}`,
                    });
                } else if (GO_SKIP.some(p => p.test(rem.content))) {
                    verdicts.push({
                        file: filePath,
                        line: hunk.startLine || 0,
                        bucket: 'STRENGTHENING',
                        framework: 'go-test',
                        detail: `skip removed: ${rem.content.trim()}`,
                    });
                }
            } else if (!rem && add) {
                if (isGoAssertion(add.content)) {
                    verdicts.push({
                        file: filePath,
                        line: add.line || hunk.startLine || 0,
                        bucket: 'STRENGTHENING',
                        framework: 'go-test',
                        detail: `assertion added: ${add.content.trim()}`,
                    });
                } else if (GO_SKIP.some(p => p.test(add.content))) {
                    verdicts.push({
                        file: filePath,
                        line: add.line || hunk.startLine || 0,
                        bucket: 'LOOSENING',
                        framework: 'go-test',
                        detail: `skip added: ${add.content.trim()}`,
                    });
                }
            }
        }
    }

    return verdicts;
}
