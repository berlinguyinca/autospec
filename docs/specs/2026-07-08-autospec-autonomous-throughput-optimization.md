# Autospec Autonomous Throughput Optimization

**Date:** 2026-07-08
**Status:** implemented
**Builds on:** `docs/specs/2026-07-06-autospec-autonomous-platform-design.md`, `docs/specs/2026-07-08-autospec-sovereign-control-plane-design.md`

## Goal

Make autospec autonomous work complete faster without weakening merge safety,
privacy controls, worktree isolation, or validation confidence.

## Current Bottlenecks

Recent dogfood runs show three practical throughput problems:

1. A claimed issue can sit with a worktree but no heartbeat or PR, leaving
   operators to hand-inspect logs and temp worktrees.
2. CI-pending or stalled work can occupy the conductor's attention while other
   independent issues are ready.
3. Dependency and batch policy are conservative enough that the queue often looks
   blocked even when safe small work could continue.

## Non-goals

- Do not bypass hooks, CI, self-review, or merge gates.
- Do not merge without required local/CI evidence.
- Do not make `main` mutable by workers.
- Do not enable broad parallelism for `reasoning:deep`, high-risk, audit,
  release, or shared-file work until the scheduler can prove independence.
- Do not require the observatory service for local autonomous execution.

## Optimization Plan

### Phase 1 — Make Stalls Observable

`autospec-run-status.sh`, `autospec-autonomous-status`, and the future observatory
progress API must show claimed work that has no heartbeat. The operator view
should distinguish:

- active heartbeat;
- stale heartbeat;
- claimed issue with no heartbeat;
- PR open and waiting on CI;
- blocked by dependency;
- stopped/parked by quota or control signal.

Acceptance criteria:

- `autospec-run-status.sh --json` includes `claimed_without_heartbeat`.
- Human status prints claimed issues without heartbeats even when the heartbeat
  table is empty.
- Tests cover a claimed issue with no heartbeat and a claimed issue with a
  matching heartbeat.

### Phase 2 — Reclaim or Take Over Stalled Claims

After status can identify stalls, autospec should enforce a reclaim threshold:

- no heartbeat + no PR + no file changes for N minutes: release claim;
- no heartbeat + uncommitted worktree changes: inspect and either finish,
  commit WIP, or reassign;
- heartbeat stale beyond threshold: use existing watchdog/run-state checks
  before release.

Acceptance criteria:

- No issue remains claimed silently past the threshold.
- Reclaim decisions are logged and visible in status/timeline.
- Same-host live work is not reclaimed merely because a long validation is
  running.

### Phase 3 — Work While CI Waits

When a PR is open and local validation has passed, the conductor should wait for
CI through the existing sentinel/background wait path and select another safe
issue if one exists.

Acceptance criteria:

- CI-pending PRs do not block unrelated ready issues.
- Merge still happens only after required checks settle green.
- CI failures route back to the owning issue and do not poison unrelated work.

### Phase 4 — Safe Local Parallelism

Allow more than one worker per repository only when the scheduler can prove:

- disjoint implementation paths;
- no shared branch/worktree;
- no `reasoning:deep`, audit, release, or priority-high serialization marker;
- worker cap not reached;
- repo health and main health are green.

Acceptance criteria:

- Default remains conservative.
- `AUTOSPEC_MAX_CONCURRENT_REPO_WORKERS` controls the cap.
- The queue explains why each issue is parallel-safe or serialized.

## Implementation Status

- Phase 1 is implemented by `autospec-run-status.sh`, including
  `claimed_without_heartbeat`.
- Phase 2 is implemented by the existing watchdog, run-state, claim, and
  release helpers; validation covers stale claim reclaim, open-PR hold, same-host
  liveness preservation, and absent run-state reclaim.
- Phase 3 is implemented through the existing `ci-wait.sh` sentinel contract and
  conductor batch selection: CI-waiting work remains claimed while remaining
  worker capacity can select unrelated safe issues.
- Phase 4 is implemented by `list-ready-issues.sh` and the conductor: batch
  selection is capped by `AUTOSPEC_MAX_CONCURRENT_REPO_WORKERS`, excludes path
  conflicts, and serializes `reasoning:deep`, `priority:high`, `regression`,
  `audit`, and `release` work with explicit reasons.

## Validation

- `bats skills/autospec-shared/tests/unit/autospec-run-status.bats`
- `bash -n skills/autospec-run/scripts/autospec-run-status.sh`
- `bash scripts/validate-public-launch-readiness.sh`
- `bash scripts/validate.sh` before merge because status helpers are installed
  across the autospec runtime.
