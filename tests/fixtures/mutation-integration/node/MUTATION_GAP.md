# Node Mutation Gap Fixture

This fixture documents a deliberate mutation gap in `source.js`.

## The gap

`isPositive(x)` uses `x > 0`. A stryker mutant flips this to `x >= 0`.
The existing test suite does not cover `isPositive(0)`, so the mutant survives.

## What M3 catches

Running `mutation-adapters/stryker.sh source.js` against this fixture would report:

```json
{"total": 1, "killed": 0, "file": "tests/fixtures/mutation-integration/node/source.js"}
```

Exit 1 (mutant survived) → `MUTATION_KILL_FLOOR=80` gate fails.

## Intended use

Integration test 7 in `tests/mutation-integration.bats` verifies this fixture exists
and is documented — stryker is not invoked directly to avoid requiring npx in CI.
