# Autospec Autonomy Runbook

Autospec autonomy is dry-run first. Commands that can write to GitHub require
`--confirm`; none of the commands in this runbook merge, approve, or push to the
default branch.

## Happy Path

```bash
bash scripts/autospec-autonomy-dry-run.sh
bash scripts/autospec-publish-issues.sh --dry-run
bash scripts/autospec-publish-issues.sh --confirm
bash scripts/autospec-autonomy-status.sh
bash scripts/autospec-build-digital-twin.sh
bash scripts/autospec-constitution-audit.sh
bash scripts/autospec-supervisor-loop.sh --dry-run --max-cycles 3
bash scripts/autospec-supervisor-loop.sh --confirm --max-cycles 3
bash scripts/autospec-supervisor-cycle.sh --dry-run --issue <number>
bash scripts/autospec-supervisor-cycle.sh --confirm --issue <number>
bash scripts/autospec-autonomy-status.sh
```

## Dry-Run And Confirm

Use `--dry-run` to generate local plans and reports without GitHub writes. Use
`--confirm` only when the generated plan is acceptable and credentials are
available. Confirmed commands still keep the same safety boundaries: no merge, no
approval, no direct default-branch push, and no unbounded issue loop.

## Local-Only Operation

Autospec autonomy is operator-invoked only in this batch. No GitHub Actions,
scheduled workflows, cron setup, or background daemon is enabled. The operator
starts each loop from a local shell command, and every loop has an explicit
`--max-cycles` bound.

```bash
bash scripts/autospec-autonomy-status.sh
bash scripts/autospec-supervisor-loop.sh --dry-run --max-cycles 3
bash scripts/autospec-supervisor-loop.sh --confirm --max-cycles 3
bash scripts/autospec-stop.sh --graceful
bash scripts/autospec-resume.sh
bash scripts/autospec-sync-guidance.sh --dry-run
```

The supervisor loop honors `~/.autospec/stop.flag` before starting and before
each cycle. Confirmed loops acquire `.autospec/run.lock`, write
`.autospec/state/current-run.json`, append `.autospec/state/run-history.json`,
and release the lock on normal completion. Dry-runs inspect and plan without
requiring the lock.

The loop consults `scripts/autospec-autonomy-budget.sh` and
`scripts/autospec-repeated-failures.sh` before each cycle. It stops on exhausted
budget, repeated failures, guidance requirements, permission failures, lock
contention, stop flags, and unsafe supervisor results.

## Publishing Issues

`scripts/autospec-publish-issues.sh` publishes local backlog drafts to GitHub
issues idempotently. It records the mapping in
`.autospec/state/published-issues.json` and uses body markers to avoid
duplicates.

## Running One Worker

`scripts/autospec-worker-v1.sh` handles one issue under worker v1 gates.
`scripts/autospec-worker-one.sh --remediate` is narrower: it only processes an
existing verifier remediation plan for one PR and refuses non-autospec branches.

## Verifier

`scripts/autospec-verify-worker-pr.sh --dry-run --pr <number>` creates
`.autospec/reports/verifier-report.md` and `.json`. Missing evidence is a
finding, not success. The verifier never approves, merges, pushes code, or fixes
worker output.

## Promotion Gate

`scripts/autospec-promote-pr.sh --dry-run --pr <number>` checks verifier results
and decides whether a draft PR is ready for human review. Confirmed mode may add
`autospec:verified` and `autospec:needs-human-review`, and remove failure labels.
Promotion is not approval and not merge.

## Remediation

`scripts/autospec-plan-remediation.sh --dry-run --pr <number>` converts verifier
findings into blocking, required, recommended, follow-up, and needs-guidance
groups. Worker remediation is allowed only when every selected finding is
worker-addressable and still inside worker v1 risk and patch budgets.

## Stuck And Guidance Flow

`scripts/autospec-publish-stuck.sh --confirm --work-item <id>` creates or updates
a stuck GitHub issue with idempotency markers and guidance labels. Use
`scripts/autospec-sync-guidance.sh --confirm` to pull guidance comments and resume
labels into local state. Guidance sync never resumes work automatically.

## Single-Cycle Supervisor

`scripts/autospec-supervisor-cycle.sh` runs one bounded cycle. It syncs state,
selects at most one issue, routes through worker and verifier surfaces, then
plans promotion, remediation, or stuck handling. It always writes final reports
under `.autospec/reports` and records runs in
`.autospec/state/supervisor-runs.json`.

## Status Report

`scripts/autospec-autonomy-status.sh` writes
`.autospec/reports/autonomy-status.md` and `.json`. The Markdown report is the
primary operator dashboard for managed issues, worker PRs, verifier state,
stuck/guidance queues, and next commands.

## Safety Guarantees

- Dry-run remains the default.
- GitHub writes require `--confirm`.
- One supervisor cycle processes at most one implementation issue.
- Promotion never merges or approves.
- Stuck/guidance sync never resumes work automatically.
- Worker remediation refuses unsafe findings, forbidden paths, and
  non-autospec branches.

## Common Failure Modes

| Failure | Recovery |
| --- | --- |
| Missing verifier report | Run `bash scripts/autospec-verify-worker-pr.sh --dry-run --pr <number>`. |
| Verifier failure | Run `bash scripts/autospec-plan-remediation.sh --dry-run --pr <number>`. |
| Worker stuck | Run `bash scripts/autospec-publish-stuck.sh --dry-run --work-item <id>`. |
| Guidance was added in GitHub | Run `bash scripts/autospec-sync-guidance.sh --dry-run`. |
| Status unclear | Run `bash scripts/autospec-autonomy-status.sh`. |
