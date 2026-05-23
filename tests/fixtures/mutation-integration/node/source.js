// Deliberate mutation gap fixture for Node/JS integration test.
// The function below has a mutant that survives all tests:
//   if (x > 0) → if (x >= 0) — the test doesn't check x=0 boundary.
// M3 (stryker) would catch this surviving mutant.

/**
 * Returns true when x is strictly positive.
 * @param {number} x
 * @returns {boolean}
 */
function isPositive(x) {
    return x > 0;
}

module.exports = { isPositive };
