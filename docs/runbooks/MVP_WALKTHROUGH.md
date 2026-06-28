# Autospec Constitution MVP Walkthrough

These walkthroughs are local/operator-invoked. Autospec does not add GitHub Actions, cron, schedulers, auto-merge, or self-approval.

## Walkthrough A — Existing Repo Onboarding

```bash
bash scripts/autospec-start.sh --dry-run
bash scripts/autospec-onboard-existing-repo.sh --dry-run --profiles web,ai-platform
bash scripts/autospec-onboard-existing-repo.sh --confirm --profiles web,ai-platform
bash scripts/autospec-constitution-audit.sh
bash scripts/autospec-audit-to-backlog.sh --dry-run
bash scripts/autospec-audit-to-backlog.sh --confirm
bash scripts/autospec-supervisor-cycle.sh --dry-run --next
```

Optional doctrine coverage check before release hardening:

```bash
bash scripts/autospec-doctrine-audit.sh --dry-run --all
bash scripts/autospec-spec-coverage.sh --dry-run
```

## Walkthrough B — New Project Bootstrap

```bash
bash scripts/autospec-bootstrap-new-project.sh --dry-run --name example --profiles web,ai-platform --application-type web
bash scripts/autospec-bootstrap-new-project.sh --confirm --name example --profiles web,ai-platform --application-type web
bash scripts/autospec-generate-ai-nlai-scaffold.sh --dry-run
bash scripts/autospec-generate-product-baseline-scaffold.sh --dry-run
```

## Walkthrough C — One Autonomous Issue Cycle

```bash
bash scripts/autospec-autonomy-status.sh
bash scripts/autospec-supervisor-cycle.sh --dry-run --next
bash scripts/autospec-supervisor-cycle.sh --confirm --next
bash scripts/autospec-verify-worker-pr.sh --dry-run --pr <number>
bash scripts/autospec-promote-pr.sh --dry-run --pr <number>
```

## Safety Contract

- Dry-run first.
- Confirm required for writes.
- No auto-merge.
- No self-approval.
- No direct default-branch pushes.
- No scheduled automation.
