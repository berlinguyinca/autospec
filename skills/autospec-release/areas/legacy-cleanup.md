# Area: legacy-cleanup

Detect deprecated routes, caches, buckets, configs, env vars, fixtures, and
infra modules that survived past their removal trigger. No revival to make
tests pass.

## Scope

- Search code, specs, docs, tests, fixtures, config, infra, examples for
  removed or deprecated behavior.
- Cross-check against deprecation issues filed during prior releases.

## Checks

- Dead code is deleted (proof: tests confirm unreachability).
- Stale spec/docs references are removed in the same change as the code.
- Compatibility paths are labeled and tracked with a removal issue.
- No `legacy_residue` or `spec_contradiction` findings outstanding.

## Findings shape

- `area: legacy-cleanup`
- `status`, `release_blocking`, `summary`, `evidence`.
- `release_blocking: true` for dead code masquerading as current behavior
  OR for stale spec/docs blocking comprehension.

## PASS criteria

- Every deprecated path is either deleted, labeled as compatibility, or
  has a tracked removal issue.
- No legacy revival exists solely to keep tests green.

## Token budget

Tier A reviewer; 50-120k context; 25 tool calls cap.
