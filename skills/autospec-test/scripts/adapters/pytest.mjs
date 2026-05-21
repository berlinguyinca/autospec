// adapters/pytest.mjs
// Assertion-shift adapter for pytest test files.
// Invokes Python-level analysis via text patterns (no subprocess needed for basic detection).
// Export: bucket(fileDiff, filePath) -> Verdict[]

// Python assertion patterns
const PY_ASSERT_PATTERNS = [
    /^\s*assert\s+/,
    /\bpytest\.raises\s*\(/,
    /\bpytest\.warns\s*\(/,
    /\bassert_called(?:_once)?(?:_with)?\s*\(/,
    /\bassertEqual\s*\(/,
    /\bassertRaises\s*\(/,
    /\bassertAlmostEqual\s*\(/,
];

const PY_TOLERANCE_PATTERNS = [
    /\babs\s*=\s*([\d.e+-]+)/,
    /\brel\s*=\s*([\d.e+-]+)/,
    /\bapprox\s*\([^,)]+,\s*(?:abs|rel)\s*=\s*([\d.e+-]+)/,
    /\bplaces\s*=\s*(\d+)/,
    /\bdelta\s*=\s*([\d.e+-]+)/,
];

const PY_LOOSENING = [
    // Adding pytest.mark.skip
    /\bpytest\.mark\.skip\b/,
    /\bpytest\.mark\.xfail\b/,
    // Weakening assert to assertTrue (any truth check)
    /\bassertTrue\b/,
];

function isPyAssertion(line) {
    return PY_ASSERT_PATTERNS.some(p => p.test(line));
}

function classifyPyChange(removedLine, addedLine) {
    // Check skip/xfail FIRST (before assertion guard) — decorator lines contain no assertion keyword
    if (PY_LOOSENING.some(p => p.test(addedLine)) && !PY_LOOSENING.some(p => p.test(removedLine))) {
        return 'LOOSENING';
    }
    if (PY_LOOSENING.some(p => p.test(removedLine)) && !PY_LOOSENING.some(p => p.test(addedLine))) {
        return 'STRENGTHENING';
    }

    const wasAssert = isPyAssertion(removedLine);
    const isAssert = isPyAssertion(addedLine);

    if (wasAssert && !isAssert) return 'LOOSENING';
    if (!wasAssert && isAssert) return 'STRENGTHENING';

    // Tolerance changes
    for (const tp of PY_TOLERANCE_PATTERNS) {
        const rm = removedLine.match(tp);
        const am = addedLine.match(tp);
        if (rm && am) {
            const oldVal = parseFloat(rm[1]);
            const newVal = parseFloat(am[1]);
            if (!isNaN(oldVal) && !isNaN(newVal)) {
                // For 'places': higher = stricter
                if (tp.source.includes('places')) {
                    if (newVal < oldVal) return 'LOOSENING';
                    if (newVal > oldVal) return 'STRENGTHENING';
                } else {
                    if (newVal > oldVal) return 'LOOSENING';
                    if (newVal < oldVal) return 'STRENGTHENING';
                }
            }
        }
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
                const b = classifyPyChange(rem.content, add.content);
                if (b) {
                    verdicts.push({
                        file: filePath,
                        line: add.line || hunk.startLine || 0,
                        bucket: b,
                        framework: 'pytest',
                        detail: `${rem.content.trim()} → ${add.content.trim()}`,
                    });
                }
            } else if (rem && !add) {
                if (isPyAssertion(rem.content)) {
                    verdicts.push({
                        file: filePath,
                        line: hunk.startLine || 0,
                        bucket: 'LOOSENING',
                        framework: 'pytest',
                        detail: `assertion removed: ${rem.content.trim()}`,
                    });
                } else if (PY_LOOSENING.some(p => p.test(rem.content))) {
                    // skip/xfail decorator removed → STRENGTHENING
                    verdicts.push({
                        file: filePath,
                        line: hunk.startLine || 0,
                        bucket: 'STRENGTHENING',
                        framework: 'pytest',
                        detail: `skip removed: ${rem.content.trim()}`,
                    });
                }
            } else if (!rem && add) {
                if (isPyAssertion(add.content)) {
                    verdicts.push({
                        file: filePath,
                        line: add.line || hunk.startLine || 0,
                        bucket: 'STRENGTHENING',
                        framework: 'pytest',
                        detail: `assertion added: ${add.content.trim()}`,
                    });
                } else if (PY_LOOSENING.some(p => p.test(add.content))) {
                    // skip/xfail decorator added → LOOSENING
                    verdicts.push({
                        file: filePath,
                        line: add.line || hunk.startLine || 0,
                        bucket: 'LOOSENING',
                        framework: 'pytest',
                        detail: `skip added: ${add.content.trim()}`,
                    });
                }
            }
        }
    }

    return verdicts;
}
