# Exact Immediate-Stop Release Recovery

## Problem

An immediate stop can persist an interrupted executor invocation before any
failure-cleanup intent exists, then release the exact claim through the generic
claim-release path. That path records `state=released` and `step=released`.
Terminal executor recovery currently recognizes only the bridge retry shape,
`state=released` and `step=retryable_released`, so the conductor exits and the
supervisor repeats the same failure.

## Design

Add a read-only claim observation dedicated to the generic released terminal
shape. It succeeds only when the authoritative claim ref matches the repository,
issue, worker, claim ID, branch, empty PR, `state=released`, and `step=released`.

When failure-cleanup intent is absent, executor recovery accepts either that
exact immediate-stop release or the existing exact bridge retry release. All
other terminal states, identity mismatches, non-empty PRs, malformed evidence,
and unavailable claim evidence remain fail-closed.

## Alternatives rejected

- Rewriting immediate stop to use the bridge retry transition would not repair
  already-persisted `released/released` claims and would broaden the stop path.
- Migrating the historical claim ref would mutate remote audit evidence and
  would not prevent recurrence on another crash boundary.
- Treating every `released` claim as sufficient would discard the exact worker,
  claim, branch, issue, and PR binding that makes recovery safe.

## Verification

The existing real-bridge conductor fixture will cover a successful
`released/released` recovery and negative cases for mismatched worker, claim ID,
branch, and PR. The complete workspace suite and canonical validator remain the
merge gates.
