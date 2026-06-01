# Area: test-coverage

Verify unit + integration + e2e + smoke test coverage meets thresholds
defined by the repo contract.

## Scope

- `tests/` directory: bats, pytest, vitest, jest, junit suites.
- Lint, typecheck, static analysis gates that the repo declares as release
  gates.
- Smoke test artifacts (browser/E2E sessions) when the repo contract
  requires no-mock smoke paths.

## Checks

- All declared release-gate test suites pass on the current HEAD.
- New code shipped in the release window has matching test files.
- Mutation-proof + assertion-density floors hold (see `autospec-test`).
- No `benchmark_overfit`, `mutation_proof_missing`, or
  `automated_test_gap` findings outstanding.

## Findings shape

- `area: test-coverage`
- `status`, `release_blocking`, `summary`, `evidence`.
- `release_blocking: true` for any failing release-gate suite or missing
  required coverage band.

## PASS criteria

- All release-gate suites green.
- Mutation + assertion-density thresholds met.
- No automated-test-gap findings.

## Token budget

Tier B reviewer; 50-120k context; 25 tool calls cap.
