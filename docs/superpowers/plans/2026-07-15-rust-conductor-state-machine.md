# Rust Conductor State Machine Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a pure, serializable Rust state machine that owns autonomous queue continuation and terminal decisions.

**Architecture:** `coordination/conductor.rs` defines all state, events, typed selection metadata, JSON persistence, and a single transition function. The existing ready queue remains the source of selection and serialization labels; this slice converts that evidence into state without launching a process or calling GitHub.

**Tech Stack:** Rust standard library; existing `autospec_core::state::json` parser; existing Rust integration-test harness.

## Global Constraints

- Do not add dependencies.
- Do not invoke processes, shell commands, GitHub, environment lookup, or an executor from the core conductor.
- Keep `execution::queue` unchanged unless an adapter is proven necessary; it models local spec IDs, not remote issue continuation.
- A constrained scan can only yield `SLICE_COMPLETE`; only an empty repository scan can yield `ALL_DONE`.
- A completed selected issue with `priority:high` serialization evidence must rescan before any terminal decision.

---

### Task 1: Define the persisted conductor contract

**Files:**
- Create: `crates/autospec-core/src/coordination/conductor.rs`
- Create: `crates/autospec-core/src/coordination/conductor/invariants.rs`
- Create: `crates/autospec-core/src/coordination/conductor/persistence.rs`
- Modify: `crates/autospec-core/src/coordination/mod.rs`
- Test: `crates/autospec-core/tests/autonomous_conductor.rs`

**Interfaces:**
- Consumes: repository, primitive issue number, scan scope, serialization reasons, retry limit, and dispatch outcome supplied by a later CLI adapter.
- Produces: `ConductorState`, `ConductorPhase`, `ConductorScope`, and `ConductorEvent` re-exported from `autospec_core::coordination`.

- [x] **Step 1: Write the failing state-contract test**

```rust
use autospec_core::coordination::{ConductorEvent, ConductorPhase, ConductorScope, ConductorState};

#[test]
fn serialized_high_priority_completion_returns_to_scan() {
    let selected = ConductorState::new(ConductorScope::Repository)
        .transition(ConductorEvent::Selected {
            issue: 42,
            serialization_reasons: vec!["priority:high".into()],
        })
        .expect("selection is valid");
    let next = selected
        .transition(ConductorEvent::DispatchSucceeded)
        .expect("result is valid");
    assert_eq!(next.phase, ConductorPhase::Scan);
    assert_eq!(next.selected_issue, None);
}
```

- [x] **Step 2: Run test to verify it fails**

Run: `cargo test -p autospec-core --test autonomous_conductor serialized_high_priority_completion_returns_to_scan`

Expected: FAIL because `coordination::conductor` and the exported conductor types do not exist.

- [x] **Step 3: Write minimal implementation**

Add `ConductorScope::{Repository,Slice}`, `ConductorPhase::{Scan,Review,Select,Claim,Dispatch,DispatchRecorded,Retry,Paused,SliceComplete,AllDone}`, a `ConductorState` with scope, selection, serialization reasons, retry count and limit, outcome, pause reason, and terminal reason, plus a checked `transition` function. Make successful serialized work return to `Scan`.

- [x] **Step 4: Run test to verify it passes**

Run: `cargo test -p autospec-core --test autonomous_conductor serialized_high_priority_completion_returns_to_scan`

Expected: PASS.

### Task 2: Make terminal and recovery decisions persisted and testable

**Files:**
- Modify: `crates/autospec-core/src/coordination/conductor.rs`
- Modify: `crates/autospec-core/src/coordination/conductor/invariants.rs`
- Modify: `crates/autospec-core/src/coordination/conductor/persistence.rs`
- Modify: `crates/autospec-core/tests/autonomous_conductor.rs`

**Interfaces:**
- Consumes: a current `ConductorState` and a `ConductorEvent`.
- Produces: strict `ConductorState::to_json` and `ConductorState::parse_json` round trips plus checked transition errors for invalid events.

- [x] **Step 1: Write failing terminal and recovery tests**

```rust
#[test]
fn constrained_empty_scan_is_slice_complete_not_all_done() {
    let state = ConductorState::new(ConductorScope::Slice)
        .transition(ConductorEvent::ScanEmpty)
        .expect("empty slice is a decision");
    assert_eq!(state.phase, ConductorPhase::SliceComplete);
}

#[test]
fn repository_empty_scan_is_all_done() {
    let state = ConductorState::new(ConductorScope::Repository)
        .transition(ConductorEvent::ScanEmpty)
        .expect("empty repository is a decision");
    assert_eq!(state.phase, ConductorPhase::AllDone);
}
```

Add pause/resume round-trip and retry-count tests that prove a retryable recorded outcome is not terminal.

- [x] **Step 2: Run tests to verify they fail**

Run: `cargo test -p autospec-core --test autonomous_conductor`

Expected: FAIL because empty scan, JSON persistence, pause/resume, and retry transitions are missing.

- [x] **Step 3: Implement strict persistence and recovery transitions**

Use the existing `JsonParser`, reject unknown keys and unsupported schema versions, and serialize every state field. Make `Pause` retain selection and retry data; make `Resume` restore only a valid nonterminal phase. Make a retryable dispatch increment the retry count, return a retry phase below the persisted limit, and pause safely at exhaustion. Exhaustion must retain the selected issue until a separate explicit abandonment transition; it cannot resume or rescan automatically.

- [x] **Step 4: Run core conductor suite to verify it passes**

Run: `cargo test -p autospec-core --test autonomous_conductor`

Expected: PASS with retry, pause/resume, serialized priority continuation, `SLICE_COMPLETE`, `ALL_DONE`, and JSON round-trip coverage.

### Task 3: Publish the core state contract

**Files:**
- Modify: `docs/workflows.md`
- Test: `crates/autospec-core/tests/autonomous_conductor.rs`

**Interfaces:**
- Consumes: exported phase names from `autospec_core::coordination`.
- Produces: user-facing meanings for `SLICE_COMPLETE` and `ALL_DONE` aligned with tested transition rules.

- [x] **Step 1: Add the workflow documentation**

Document every conductor phase and state that the module is pure control-plane logic. Define `SLICE_COMPLETE` as a constrained scan ending without global completion proof and `ALL_DONE` as an empty repository scan after all serialized continuation work has rescanned.

- [x] **Step 2: Run focused regression and format check**

Run: `cargo fmt --all --check && cargo test -p autospec-core --test autonomous_conductor`

Expected: PASS.

- [x] **Step 3: Commit**

Run: `git add crates/autospec-core/src/coordination/conductor.rs crates/autospec-core/src/coordination/mod.rs crates/autospec-core/tests/autonomous_conductor.rs docs/workflows.md docs/superpowers/specs/2026-07-15-rust-conductor-state-machine-design.md docs/superpowers/plans/2026-07-15-rust-conductor-state-machine.md && git commit -m "feat: model Rust conductor transitions"`

## Plan Self-Review

- Spec coverage: state ownership, persistence, continuation, recovery, both terminal decisions, documentation, and the no-shell boundary map to Tasks 1–3.
- Placeholder scan: no deferred implementation or unspecified test steps remain.
- Type consistency: every later task uses the types defined in Task 1; `ConductorState::transition` is the sole transition API.
