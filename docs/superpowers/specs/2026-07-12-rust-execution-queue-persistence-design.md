# Rust Execution-Queue Persistence Design

**Date:** 2026-07-12
**Status:** approved for implementation
**Depends on:** [Rust Spec-State Persistence Design](2026-07-12-rust-spec-state-persistence-design.md), [#1861](https://github.com/berlinguyinca/autospec/issues/1861)

## Goal

Make the existing Rust `ExecutionQueue` durable and resumable by storing each local run at `.autospec/runs/<run-id>/queue.json`, without invoking agents or changing the public `autospec run` and `autospec resume` stubs yet.

## Scope

This slice promotes the dependency-free JSON and recovery-write mechanics into shared crate-private helpers, then uses them for a versioned queue document. Each queue entry persists its status, attempt count, failure classification, blocker, `started_at`, `updated_at`, and the latest validation result. The queue can load a named run and discover the most recently updated incomplete run beneath `.autospec/runs/`.

The queue remains a local model. It does not execute commands, call an agent, modify GitHub, or switch any CLI command from its current explicit stub behavior.

## Queue document

```json
{
  "schema": 1,
  "run_id": "run-v66",
  "updated_at": 1720000000,
  "entries": [
    {
      "spec_id": "v65-spec-state-validation",
      "status": "passed",
      "attempts": 1,
      "failure_kind": null,
      "blocker": null,
      "started_at": 1720000000,
      "updated_at": 1720000001,
      "validation": {"status": "passed", "summary": "cargo test --workspace"}
    }
  ]
}
```

Entries retain their original order. `run_id` must be a single path segment, and every `spec_id` must satisfy the existing spec-ID policy. Terminal entries are `passed`, `blocked`, `deferred`, and `superseded`; a run is incomplete while any entry is `pending`, `running`, or retryable `failed`.

## Decisions

1. **Use seconds since the Unix epoch as injected `u64` timestamps.** Public `*_at` methods accept a timestamp for deterministic tests; existing convenience methods use the system clock. This avoids a time dependency.
2. **Recover queue JSON exactly as spec state recovers state JSON.** A valid primary wins. A valid temporary file replaces a missing or malformed primary. Neither valid file means failure, not an empty queue.
3. **Discover the latest incomplete run deterministically.** Read only immediate directory entries below `.autospec/runs/`, ignore non-directories and malformed queues, sort ties by run ID, and choose the greatest persisted `updated_at` among valid incomplete runs. If no valid incomplete queue exists, return `None`.
4. **Preserve current queue method behavior.** Existing `mark_passed`, `record_failure`, `block`, reports, and handoff markdown keep their signatures; timestamp-aware variants underpin persistence without changing their call sites.
5. **Keep the CLI non-executing.** Queue durability is a prerequisite for future `run`/`resume`; this slice does not enable them.

## Acceptance criteria

- A queue with passed, failed, blocked, deferred, and superseded entries round-trips through its run path without reordering.
- Missing and malformed primary queue files recover only from a complete temporary file; malformed documents without recovery are errors.
- A persisted run preserves timestamps and validation result metadata.
- `load_latest_incomplete(root)` selects the newest valid incomplete queue and ignores complete or malformed runs.
- Invalid run IDs, duplicate entry IDs, unknown states, and malformed validation metadata are rejected.
- Existing queue tests and all Rust/fast repository checks pass; the public CLI stubs remain unchanged.

## Non-goals and next dependency

Agent-result ingestion, command execution, and `autospec run`/`resume` wiring remain deferred. Once durable queues are proven, the next slice can add the safe agent contract and use the persisted queue for explicit, non-destructive command paths.
