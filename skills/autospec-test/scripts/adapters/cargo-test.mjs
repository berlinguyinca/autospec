// adapters/cargo-test.mjs
// Assertion-shift adapter for Rust test files (cargo test).
// Export: bucket(fileDiff, filePath) -> Verdict[]

// Rust assertion patterns
const RUST_ASSERT_PATTERNS = [
    /\bassert!\s*\(/,
    /\bassert_eq!\s*\(/,
    /\bassert_ne!\s*\(/,
    /\bassert_matches!\s*\(/,
    /\bpanic!\s*\(/,
    /\bunwrap\s*\(\)/,
    /\bexpect\s*\(/,
];

// Rust assertion strictness rank
const RUST_ASSERT_RANK = {
    'assert_eq!': 5,
    'assert_ne!': 4,
    'assert_matches!': 4,
    'assert!': 3,
    'expect(': 2,
    'unwrap()': 1,
};

// Rust skip patterns
const RUST_SKIP = [
    /\#\[ignore\]/,
    /\/\/\s*assert/,  // commented-out assertion
];

function isRustAssertion(line) {
    return RUST_ASSERT_PATTERNS.some(p => p.test(line));
}

function detectRustAssertOp(line) {
    return Object.keys(RUST_ASSERT_RANK).find(op => line.includes(op));
}

function classifyRustChange(removedLine, addedLine) {
    // Check #[ignore] FIRST — it appears on its own line with no assertion keyword
    if (RUST_SKIP.some(p => p.test(addedLine)) && !RUST_SKIP.some(p => p.test(removedLine))) {
        return 'LOOSENING';
    }
    if (RUST_SKIP.some(p => p.test(removedLine)) && !RUST_SKIP.some(p => p.test(addedLine))) {
        return 'STRENGTHENING';
    }

    const wasAssert = isRustAssertion(removedLine);
    const isAssert = isRustAssertion(addedLine);

    if (wasAssert && !isAssert) return 'LOOSENING';
    if (!wasAssert && isAssert) return 'STRENGTHENING';

    // Operator rank change
    const beforeOp = detectRustAssertOp(removedLine);
    const afterOp = detectRustAssertOp(addedLine);
    if (beforeOp && afterOp && beforeOp !== afterOp) {
        const delta = RUST_ASSERT_RANK[afterOp] - RUST_ASSERT_RANK[beforeOp];
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
                const b = classifyRustChange(rem.content, add.content);
                if (b) {
                    verdicts.push({
                        file: filePath,
                        line: add.line || hunk.startLine || 0,
                        bucket: b,
                        framework: 'cargo-test',
                        detail: `${rem.content.trim()} → ${add.content.trim()}`,
                    });
                }
            } else if (rem && !add && (isRustAssertion(rem.content) || RUST_SKIP.some(p => p.test(rem.content)))) {
                // Removal of assertion OR skip decorator
                const isSkipRemoval = !isRustAssertion(rem.content) && RUST_SKIP.some(p => p.test(rem.content));
                verdicts.push({
                    file: filePath,
                    line: hunk.startLine || 0,
                    bucket: isSkipRemoval ? 'STRENGTHENING' : 'LOOSENING',
                    framework: 'cargo-test',
                    detail: isSkipRemoval ? `skip removed: ${rem.content.trim()}` : `assertion removed: ${rem.content.trim()}`,
                });
            } else if (!rem && add && (isRustAssertion(add.content) || RUST_SKIP.some(p => p.test(add.content)))) {
                verdicts.push({
                    file: filePath,
                    line: add.line || hunk.startLine || 0,
                    bucket: 'STRENGTHENING',
                    framework: 'cargo-test',
                    detail: `assertion added: ${add.content.trim()}`,
                });
            }
        }
    }

    return verdicts;
}
