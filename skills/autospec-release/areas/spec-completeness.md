# Area: spec-completeness

Audit `docs/specs/**.md` against implementation reality. Each spec must have
matching live code; no stale specs may linger.

## Scope

- Enumerate `docs/specs/**.md` on the current branch.
- For each spec, identify the acceptance-criteria block and locate the
  implementing code, configs, infra modules, or fixtures.
- Apply spec supersession (recency) via
  `scripts/resolve-spec-supersession.sh` — superseded specs are NOT release
  blockers and must be downgraded to `informational`.
- Cross-check against open + closed GitHub issues to detect "shipped but the
  spec was rewritten" or "spec rewritten but never shipped".

## Findings shape

Emit per-finding rows into `release-verdict.json` with:

- `area: spec-completeness`
- `status`: `PASS` / `PARTIAL` / `FAIL` / `NOT_TESTED`
- `release_blocking`: true when an acceptance criterion is unimplemented or
  contradicted by current code; false when superseded.
- `summary`: one-line description.
- `source_spec`: spec path.
- `evidence`: file paths + line numbers.

## PASS criteria

- Every non-superseded spec has at least one matching implementation file.
- No acceptance criterion is contradicted by code on the current branch.
- No stale spec references behavior that was removed.

## Token budget

Tier A reviewer; 50-120k context; 25 tool calls cap.
