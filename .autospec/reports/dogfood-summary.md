# Autospec Dogfood Summary

This repository can dogfood the Autospec Constitution MVP with local sibling policy repositories.

## Recommended Command

```bash
bash scripts/autospec-mvp-smoke.sh --dry-run
```

Latest local dogfood smoke in this hardening pass returned `pass_with_warnings`.
Use `.autospec/reports/mvp-smoke.md` for the current warning list after each run.

## Safety

- No GitHub issue publishing by default.
- No confirmed worker execution by default.
- No auto-merge or self-approval.
