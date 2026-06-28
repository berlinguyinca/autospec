# Dogfooding Autospec on Autospec

Autospec can audit this repository using local sibling policy repositories. This runbook is dry-run first and does not publish issues or run confirmed worker actions by default.

## Local Policy Repos

Expected sibling layout:

```text
IdeaProjects/
  autospec/
  autospec-constitution/
  autospec-baselines/
```

Use only profiles that exist in the local baseline repository. The example config uses `internal-tool`, `ai-platform`, and `testing`; if additional profiles such as `cli` or `documentation` are added later, include them then.

## Dry-Run Sequence

```bash
bash scripts/autospec-start.sh --dry-run
bash scripts/autospec-mvp-smoke.sh --dry-run
bash scripts/autospec-constitution-audit.sh
bash scripts/autospec-audit-to-backlog.sh --dry-run
bash scripts/autospec-autonomy-status.sh
bash scripts/autospec-supervisor-cycle.sh --dry-run --next
```

## Safety

- Do not publish dogfood issues by default.
- Do not run confirmed worker actions by default.
- Do not merge or approve Autospec-generated PRs automatically.
- Keep dogfood runs operator-invoked and local.
