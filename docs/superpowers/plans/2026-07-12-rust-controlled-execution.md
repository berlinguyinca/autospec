# Rust Controlled Execution Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ingest a typed local agent result into a durable queue and expose controlled queue creation and resume inspection without executing agents or validation commands.

**Architecture:** `autospec-core::execution` owns the typed outcome, queue transition, and durable result artifact. `autospec-cli` accepts an explicit run ID, spec ID, outcome, and result file; it never infers an outcome from prose or launches a process. `run` creates a queue only, while `resume` reports the latest incomplete queue only.

**Tech Stack:** Rust 2021 standard library, existing strict JSON parser, Cargo integration tests.

## Global Constraints

- No new dependencies, remote writes, agent spawning, shell execution, or validation execution.
- Keep `scripts/validate.sh` as the R1 validation executor until fixture parity proves a Rust result aggregator.
- Bind every result to explicit `run_id` and `spec_id`; do not derive either from free-form agent JSON.
- Bind every ingestion to an explicit safe `result_id`; persist canonical result envelopes under `.autospec/runs/<run-id>/agent-results/<spec-id>/<result-id>.json` using temporary-file recovery.
- Apply a result to a queue entry at most once by recording its `result_id`; replaying the same result after a crash must not consume another retry.
- `passed` and `failed` outcomes require an explicit validation summary; `blocked` requires at least one blocker.
- Preserve queue retry and terminal-state invariants.

### Task 1: Typed agent-result ingestion core

**Files:**
- Create: `crates/autospec-core/src/execution/result.rs`
- Modify: `crates/autospec-core/src/execution/mod.rs`
- Modify: `crates/autospec-core/src/agent/contract.rs`
- Test: `crates/autospec-core/tests/agent_contracts.rs`

- [x] Write failing tests for strict parsing, explicit typed outcomes, missing validation/blocker rejection, and deterministic JSON round-trip.
- [x] Add `AgentOutcome::{Passed,Failed { failure_kind },Blocked}` plus `AgentResult::from_json` using the existing strict parser.
- [x] Add `IngestedAgentResult::new(run_id, spec_id, result_id, outcome, result)` that validates IDs and outcome requirements without interpreting prose.
- [x] Verify `cargo test -p autospec-core --test agent_contracts` passes.

### Task 2: Durable result artifact and queue application

**Files:**
- Modify: `crates/autospec-core/src/execution/result.rs`
- Modify: `crates/autospec-core/src/execution/queue.rs`
- Test: `crates/autospec-core/tests/execution_queue.rs`

- [x] Write failing recovery and path-binding tests for `.autospec/runs/<run-id>/agent-results/<spec-id>/<result-id>.json`.
- [x] Atomically persist and load canonical result envelopes with valid-primary precedence, typed recovery errors, and same-ID conflict rejection.
- [x] Apply `Passed` by recording a passed validation and transitioning the matching queue entry; apply `Failed` through the validation failure path; apply `Blocked` with the joined blocker summary.
- [x] Save the updated queue only after result persistence succeeds; make same-ID replays idempotent across a persistence crash.
- [x] Verify focused agent and queue tests pass.

### Task 3: Controlled CLI commands

**Files:**
- Modify: `crates/autospec-cli/src/commands/run.rs`
- Modify: `crates/autospec-cli/src/commands/resume.rs`
- Modify: `crates/autospec-cli/tests/cli_commands.rs`
- Modify: `docs/cli-reference.md`
- Modify: `docs/workflows.md`

- [x] Write failing CLI tests for `run --run <id> --spec <id>...`, `run --ingest <file> --run <id> --spec <id> --outcome <status>`, and `resume --json`.
- [x] Make `run` create a persisted queue when given specs; reject an existing run and execute no external command.
- [x] Make `run --ingest` load a result file and apply its explicit outcome to an existing queue.
- [x] Make `resume` render the latest incomplete run and its next entry; error when no incomplete run exists.
- [x] Document that these commands manage local state only and do not execute agents or validation.

### Task 4: R1 execution parity and release gates

**Files:**
- Create: `crates/autospec-cli/tests/fixtures/validation-results/*.json`
- Modify: `crates/autospec-core/src/validation/*`
- Modify: `crates/autospec-cli/src/commands/validate.rs`
- Modify: `docs/specs/2026-07-11-rust-core-runtime-consolidation-design.md`

- [x] Capture the shell validation result shape in golden fixtures before implementation.
- [x] Implement only result aggregation in Rust; keep shell execution behind `AUTOSPEC_FORCE_LEGACY_SHELL=1`.
- [x] Add a shadow comparison command and prove fixture-equivalent JSON/pass-fail output.
- [x] Record the current wrapper topology and explicitly defer direct-executor process/time/output metrics until delegation is eligible.

### Task 5: Context monitor and fallback retirement decision

**Files:**
- Modify: `docs/specs/2026-07-11-rust-core-runtime-consolidation-design.md`
- Create: `docs/reports/2026-07-12-rust-context-monitor-cutover.md`
- Modify: `docs/cli-reference.md`

- [x] Compare the Rust context state machine to Python fixtures and record install/process/latency evidence.
- [x] State a migration decision, with the force-Python escape hatch if cut over.
- [x] Record every fallback’s fixture, shadow proof, one-release escape hatch, removal issue, and observed delegation result.
- [x] Do not remove a wrapper fallback until every row has evidence.
