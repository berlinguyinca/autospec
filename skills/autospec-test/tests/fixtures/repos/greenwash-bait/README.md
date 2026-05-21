# greenwash-bait fixture

Synthetic target repo that contains a real regression with tempting assertion-weakening tests.
Used by the assertion-shift classifier for operator/full verification.

## Structure
- `tests/calc.test.js` — test file with LOOSENING and SHIFTING changes
- `src/calc.js` — source file (non-test, for co-edit testing)
