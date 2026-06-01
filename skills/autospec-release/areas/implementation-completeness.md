# Area: implementation-completeness

Verify every spec acceptance criterion has matching implementation. No
half-shipped features may pass.

## Scope

- Walk every non-superseded spec.
- For each numbered acceptance criterion, identify the proving code path
  (script, function, schema, config, infra module).
- Detect partial implementations: criterion mentioned in code comments but
  not actually wired into runtime behavior.

## Checks

- Outsourced-implementation gate: a third-party API/SDK call cannot satisfy
  a spec describing an in-repo algorithm unless the spec explicitly delegates.
- New skills must be installed by their `install.sh` AND referenced in the
  master suite `bootstrap.sh` / `install.sh`.
- Phase 4 implementer prompts must reflect the latest spec contract.

## Findings shape

- `area: implementation-completeness`
- `status`, `release_blocking`, `summary`, `source_spec`, `evidence`.
- `release_blocking: true` when an acceptance criterion has no proving code.

## PASS criteria

- Every acceptance criterion maps to live code.
- No `outsourced_implementation` findings (per release SKILL.md Stage 9).
- New skills are wired into bootstrap + install paths.

## Token budget

Tier A reviewer; 50-120k context; 25 tool calls cap.
