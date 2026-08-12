# Automatic review-policy advancement

The autonomous conductor runs `autonomous-self-improvement.sh advance` before
its Tier-3 `apply` step. Advancement reads repository-scoped review outcomes and
gaps, evaluates active canaries against the closed validation recipe, and records
promotion, hold, or rollback in the append-only lifecycle ledger.

The subsequent `apply --apply` command can file strengthening or neutral work
through the normal `needs-classify` path. Weakening proposals remain report-only.
The conductor passes explicit `--review-outcomes` and `--gaps` paths.
