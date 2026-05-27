# Autospec Distributed Run Coordination Design

Date: 2026-05-26

## Goal

Allow multiple `/autospec-run` processes on different workstations to process one
GitHub issue queue without duplicate claims, file conflicts, or manual operator
coordination.

## Problem

Autospec already coordinates local work through `auto-implement` and
`in-progress-by-bot` labels, repo-scoped heartbeat files, dependency text such as
`Depends on #N`, and `scripts/autospec-watchdog.sh`. That is enough for one
operator or one workstation, but it is not a full distributed protocol.

When several workstations scan the same queue, they need a shared view of:

- which worker owns an issue,
- which implementation paths are currently active,
- which dependency edges block safe execution,
- which stale claims can be reclaimed, and
- which independent issues can safely run in parallel.

The motivating example is a queue where #1395, #1389, #1362, and #1364 can run in
parallel, #1398 must wait for #1401 because they touch the same builder files,
and a demo-web dependency chain should not race its scaffold, client, tabs, and
final E2E runbook work.

## Team Personality

Selected team: **Reliability/backend coordination team**

Roles:

- backend developer
- platform engineer
- sysadmin/SRE
- security advisor
- test engineer

This team fits because the feature is distributed coordination, not just prompt
copy. The main risks are race conditions, stale ownership, cross-workstation
visibility gaps, unsafe reclaim behavior, and loss of auditability. The team
emphasis for child issues is conservative state transitions, deterministic shell
helpers, GitHub-native persistence, and tests that simulate worker races.

### Review Counter-Team

Counter-team: **Maintainer and product-operations review team**

Roles:

- maintainer
- operator UX reviewer
- documentation owner
- regression-test reviewer

This team should challenge whether the protocol stays understandable for normal
autospec operators, whether comments/labels clutter issues, whether old single
worker flows still work, and whether docs explain recovery without requiring
interactive support. Review must stay inside the coordination feature: do not ask
for unrelated autospec-run rewrites, but do block changes that make current
single-host behavior fragile or undocumented.

## Architecture

Use GitHub Issues as the shared source of truth. Workstations do not communicate
directly, and local heartbeat files remain secondary cache/state only.

Add a small distributed coordination layer to `autospec-run`:

- `skills/autospec-run/scripts/list-ready-issues.sh` plans the queue.
- `skills/autospec-run/scripts/claim-issue.sh` performs an atomic claim attempt.
- `skills/autospec-run/scripts/upsert-run-state.sh` writes the GitHub run-state
  comment.
- `skills/autospec-run/scripts/release-issue.sh` releases or marks failed work.
- `scripts/autospec-watchdog.sh` learns to reconcile GitHub run-state comments
  in addition to local heartbeats.

The current monitor keeps its existing behavior, but candidate selection changes
from "pick the first ready issue" to "ask the coordinator for the first safe,
unclaimed issue under the active profile." A failed claim is non-fatal: the
monitor refreshes the queue and tries the next safe candidate.

## API Shape

Default `/autospec-run` remains non-interactive and should require no new flags.
It automatically uses the coordinator before selecting work.

Optional flags and helper modes:

- `--worker-id <id>` or `AUTOSPEC_WORKER_ID` overrides worker identity.
- `--coordination-status` prints active workers, claimed issues, blockers, stale
  claims, conflicts, and recommended parallel batches.
- `--max-parallel-safe` prints the next safe batch without claiming.
- `--claim <issue>` attempts one deterministic claim and exits with a
  machine-readable result.
- `--release <issue>` releases a claim during failure, stop, or manual recovery.

GitHub labels:

- `auto-implement` remains the ready queue.
- `in-progress-by-bot` remains the active claim label.
- `blocked-by-autospec` marks issues skipped for dependency or template blockers.
- `coordination-conflict` marks issues skipped because active work owns
  overlapping paths.

## Data Model

Each worker has a stable process identity:

```text
<hostname>:<user>:<harness>:<pid>:<started_at_epoch>
```

Each claimed issue gets exactly one upserted GitHub issue comment:

```markdown
<!-- autospec-run-state:begin -->
{
  "schema": 1,
  "repo": "owner/repo",
  "issue": 1395,
  "worker_id": "host:user:codex:123:1770000000",
  "state": "claimed",
  "branch": "feat/example",
  "pr": "",
  "step": "claimed",
  "paths": ["skills/autospec-run/SKILL.md"],
  "claimed_at": "2026-05-26T14:00:00Z",
  "updated_at": "2026-05-26T14:05:00Z",
  "ttl_seconds": 10800
}
<!-- autospec-run-state:end -->
```

States mirror existing heartbeat steps:

- `claimed`
- `worktree_ready`
- `tests_started`
- `tests_passed`
- `pr_created`
- `awaiting_ci`
- `reviewed`
- `merged`
- `failed`
- `released`

Conflict keys are derived from:

- file paths in `## Implementation outline`,
- branch names,
- open PR touched files when available, and
- optional future metadata entries that declare explicit owned paths.

Two issues can run together only when dependency edges are satisfied and conflict
keys do not overlap.

## Error Handling

The coordinator is conservative. If it cannot prove work is safe, it skips the
candidate instead of blocking the whole monitor.

- Claim race: only the worker whose label mutation succeeds continues; losers
  refresh and try another issue.
- Transient GitHub error: retry once with jitter, then skip the scan cycle.
- Stale `claimed` state before worktree creation: reclaim after a short timeout,
  default 300 seconds.
- Stale post-PR work: use a longer timeout because CI and review can be slow.
- Active open PR: do not reclaim unless the PR is closed, merged, or stale beyond
  the stricter PR timeout.
- File conflict: leave `auto-implement` in place, add or update a
  `coordination-conflict` explanation, and choose another issue.
- Malformed issue body: label `needs-autospec-template` or `needs-quality-bar`.
- Worker crash: watchdog restores recoverable claims and marks old run-state as
  `released`.
- Clock skew: prefer GitHub comment timestamps; local timestamps are fallback.
- No safe work: print a queue summary and back off without asking the user.

## Testing

Use deterministic shell and Bats tests first, with one opt-in GitHub integration
test.

Required local tests:

- claim race: two simulated workers attempt the same issue and only one wins,
- dependency blocking: `Depends on #N` excludes an issue until #N is closed,
- parallel batch planning: independent issues group together,
- path conflict detection: overlapping paths block parallel selection,
- run-state upsert: repeated updates replace one marked comment,
- stale reclaim: expired claims restore `auto-implement`,
- malformed state: invalid JSON or wrong schema is ignored or replaced safely,
- cross-repo isolation: `owner/a` state never affects `owner/b`,
- stop/resume compatibility: immediate stop does not leave conflicting state,
- lock-step validation across the `autospec-run` trio.

Opt-in integration test:

- create a throwaway GitHub repo with a queue shaped like the motivating example,
- launch two coordinator claim attempts,
- assert they claim different safe issues and skip dependency/path conflicts,
- delete the throwaway repo.

Verification commands:

```bash
bats tests/heartbeat.bats
bats tests/unit/test_autospec_distributed_coordination.bats
bash scripts/validate.sh
```
