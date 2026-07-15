# Rust Conductor State Machine Design

## Goal

Give the Rust control plane one pure, persisted model for deciding whether an
autonomous queue continues, pauses, completes a constrained slice, or proves
the whole repository queue is empty.

## Context

The ready-queue planner already exposes a typed selected issue and its
serialization reasons. The present foreground command still delegates its
control flow to a shell script, so this slice intentionally stops before any
executor launch. It establishes the model that the following foreground
cutover will consume.

## Design

`autospec_core::coordination::conductor` will contain a side-effect-free,
schema-versioned `ConductorState` and `ConductorEvent`. State retains the scan
scope (`Repository` or `Slice`), selected issue number, serialization reasons,
retry count and limit, recorded outcome, pause reason, and terminal reason. It
keeps transitions in `conductor.rs`, strict JSON persistence in private
`conductor/persistence.rs`, and validation invariants in private
`conductor/invariants.rs`, using the core JSON utilities without a dependency.

The transition function is the only place that decides continuation:

1. Scan results enter safety review, selection, `SLICE_COMPLETE`, or
   `ALL_DONE`.
2. A selected issue records a claim and then an executor outcome.
3. Retryable outcomes return to `Claim` only within their persisted retry limit;
   exhausted retries pause with the selected issue retained for explicit recovery.
   They cannot resume or rescan automatically: a caller must record explicit
   abandonment/recovery before the conductor clears that selection.
4. Pause and resume retain the selected issue and all decision context.
5. Completion of a serialized `priority:high` issue returns to scanning,
   never directly to a terminal state.

`ALL_DONE` requires an empty repository-scoped scan snapshot. An empty
slice-scoped snapshot represents only `SLICE_COMPLETE`, because it cannot
prove the rest of the repository queue is empty.

## Boundaries

- The module must not import `std::process`, invoke `gh`, read environment
  variables, launch an executor, or select a shell backend.
- It consumes only primitive decision inputs supplied by the later CLI layer;
  it does not duplicate ready-queue discovery, claim persistence, or executor
  behavior.
- `execution::queue` remains the local spec-ID queue and is not changed for
  remote issue continuation semantics.

## Tests

The core integration test creates real state values and checks JSON
round-tripping, retry behavior, pause/resume retention, serialized high
priority continuation, constrained completion, and repository-wide completion.
The test never invokes a process or mocks a shell conductor.

## Follow-on

Issue #2062 will call this state machine from `autonomous run-foreground`,
persist its state below the repository-scoped autonomous directory, and remove
the live shell foreground handoff.
