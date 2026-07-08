# Autonomy v2

Autonomy v2 lets Autospec move from failed Constitution/Baseline rules to bounded recipe-backed work. It remains operator-invoked, dry-run by default, and does not merge, approve, schedule, migrate databases, change auth/security behavior, or upgrade dependencies.

## Core Flow

```bash
bash scripts/autospec-detect-stack-profile.sh
bash scripts/autospec-recipe-index.sh
bash scripts/autospec-rule-to-recipe-plan.sh --dry-run
bash scripts/autospec-build-patch-plan.sh --dry-run --issue <number>
bash scripts/autospec-worker-one.sh --dry-run --issue <number> --auto-recipe
bash scripts/autospec-rule-recheck.sh --dry-run --issue <number>
bash scripts/autospec-verify-worker-pr.sh --dry-run --pr <number>
```

## Safety Model

- Recipes are allowed only through the worker capability registry.
- Patch plans define allowed files, forbidden files, budgets, rollback, validation, and human-guidance triggers.
- Stack-specific scaffolds require high-confidence stack detection.
- Unsafe work becomes stuck/guidance or a smaller decomposed issue.
- `--confirm` is required before any local write by template application or worker execution.

## Status

```bash
bash scripts/autospec-autonomy-v2-status.sh
```
