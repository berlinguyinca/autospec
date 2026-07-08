# Rule-to-Recipe Planning

Rule-to-recipe planning maps failed, partial, unknown, or manual-review rule results to implementation recipes.

```bash
bash scripts/autospec-rule-to-recipe-plan.sh --dry-run
bash scripts/autospec-rule-to-recipe-plan.sh --dry-run --rule <rule_id>
```

Statuses:

- `recipe_available`: worker may build a patch plan and dry-run execution.
- `recipe_available_but_disabled`: capability registry blocks execution.
- `planning_only`: generate docs/spec/issues, not implementation.
- `requires_target_repo_runtime`: target app work is outside engine scope.
- `requires_human_guidance`: human architecture or product decision needed.
- `unsupported`: no recipe is available.
- `deferred`: intentionally beyond MVP.
