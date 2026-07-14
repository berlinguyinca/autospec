# Rust Claim Control-Plane Implementation Plan

**Goal:** Replace the GitHub-backed run-state and distributed issue-lease authority with `autospec claim`, leaving no lifecycle decision in `run-state.sh`, `claim-issue.sh`, or `release-issue.sh`.

**Scope:** This vertical owns the state-comment protocol, lowest-comment-ID CAS selection, server-timestamp stale-lease policy, label transitions, terminal merge records, and linked-PR reconciliation. Queue selection and watchdog scheduling consume this typed API in a later vertical; they do not retain a parallel lease implementation.

**Constraints:** No new dependencies. GitHub access continues through direct `gh` subprocess arguments. GitHub comments are untrusted input: malformed comments are ignored; unknown timestamps fail closed as fresh; terminal merge records prevent new claims. Existing installed scripts may be one-line compatibility launchers only until all callers move to `autospec claim`.

## Contract

`autospec claim state read|upsert|clear|reconcile-linked-pr` preserves the schema-1 marked-comment protocol:

- Read selects the lowest numeric marked comment ID, validates its repository and issue binding, and emits an empty success result when no valid state exists.
- Upsert patches the lowest marked comment or creates one, preserves `claimed_at`, retries transient PATCH failures, and deletes only higher-ID duplicate state comments.
- Reconcile finds the lowest-numbered linked open PR with exactly one Closeout report, records its PR before posting one idempotent handoff blocker.

`autospec claim acquire|release` owns lease transitions:

- Acquire requires `auto-implement` plus a passing typed safety review, writes startup evidence before the label move, uses the lowest comment ID as the CAS winner, and rechecks state and labels after the configured confirmation reads.
- Fresh foreign leases are never reclaimed. Missing or invalid server timestamps are treated as fresh. A terminal merged record makes acquisition return the documented unavailable status.
- Release writes terminal merge evidence idempotently, updates state, and transitions labels atomically as far as GitHub permits.

## Delivery steps

- [x] Add strict core models for schema-1 comment envelopes, marker extraction, issue binding, lowest-ID selection, and stable JSON rendering.
- [x] Add `autospec claim state` CLI operations with a typed `gh` adapter and integration fixtures for duplicate ordering, retry, foreign-repository state, and reconciliation.
- [x] Add `autospec claim acquire|release` with TDD coverage for safety gate refusal, fresh/stale leases, lost-race self-cleanup, terminal merge, and label/heartbeat ordering.
- [ ] Convert the three legacy scripts to one-line `autospec claim` launchers, update direct callers and lock-step skill documentation, then remove their embedded implementations.
- [ ] Run the focused Rust and Bats parity suites, full workspace tests/clippy/format, `autospec validate`, and a runtime audit proving the three scripts are no longer R1 authorities.

## Acceptance evidence

The cutover is complete only when the Rust command passes the existing mocked-GitHub race fixtures, a process cannot delete another worker's marked comment, malformed remote comments cannot cause a reclaim, and all live callers use `autospec claim` rather than invoking a shell state implementation.
