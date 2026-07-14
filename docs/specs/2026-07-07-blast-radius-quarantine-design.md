# Blast-radius quarantine design

Issue #1545 extends the autonomy guardrails foundation so high-blast-radius work
never creates a modal prompt and never auto-merges by accident. Risky diffs are
classified, labeled for async human review, and skipped so the conductor can
continue with other runnable work.

## Contract

1. `scripts/autonomous-guardrails.sh blast-radius --changed-files FILE --json`
   emits a deterministic classification with `label`, `reversibility`, changed
   `paths`, and `fenced_matches` from the configured registry.
2. The registry lives in `.autospec/autospec.yml` under `fenced_surfaces` and can
   be overridden with `--fenced-surfaces FILE` for fixtures or target repos.
3. `scripts/autonomous-premerge-gate.sh --quarantine-out FILE` writes
   `autospec.autonomous.quarantine.v1` provenance, applies `autospec:needs-human`,
   prints `quarantine fenced_surface`, and exits 0 without running QA/secaudit or
   printing `merge-ok`.
4. `scripts/autonomous-prioritize.sh score` records fenced/high-blast-radius
   candidates in `considered_and_skipped` with `reason: human_gate`, then selects
   the highest-scoring runnable candidate so the same cycle can keep working.

## Default fenced surfaces

The default registry includes trading-system money, risk, and execution paths;
schema migrations; auth/secret/token paths; public API/package contracts; and the
autonomous control plane. Trading-system entries are marked `severity: fenced`
because incorrect changes can move capital or bypass risk limits.

## Validation

`tests/autonomous/test_blast_radius_quarantine.bats` covers low-risk
classification, configured trading-risk matches, premerge quarantine provenance,
and same-cycle runnable-candidate selection. `autospec validate` gates this
suite and static registry/option checks under `check_blast_radius_quarantine_contract`.
