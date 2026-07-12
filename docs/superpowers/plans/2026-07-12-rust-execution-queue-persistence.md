# Rust Execution-Queue Persistence Implementation Plan

**Goal:** Persist and resume the Rust execution queue without enabling command or agent execution.

**Architecture:** Extract the existing dependency-free JSON parser and recovery-write primitives into crate-private shared modules. Extend `ExecutionQueue` with timestamp-aware transitions, validation-result metadata, strict queue-document parsing, named-run persistence, and latest-incomplete discovery. Preserve existing queue APIs as convenience wrappers.

**Tech Stack:** Rust 2021 standard library, existing spec-ID policy, Cargo integration tests, repository shell validation.

## Constraints

- No new dependencies.
- Do not change the `autospec run` or `autospec resume` CLI stubs.
- Preserve existing `ExecutionQueue` method signatures and report text.
- Never interpret a malformed queue as an empty run.
- Reuse the validated JSON/recovery mechanics from state persistence; do not copy another parser.

### Task 1: Establish red persistence and resume tests

**Files:**
- Modify: `crates/autospec-core/tests/execution_queue.rs`

- [ ] Add a unique temporary-root helper or reuse the state-test pattern.
- [ ] Add failing tests for queue JSON round-trip, temporary-file recovery, malformed-document rejection, and latest incomplete run selection.
- [ ] Add failing tests for persisted timestamps, validation metadata, duplicate IDs, and invalid run IDs.
- [ ] Run `cargo test -p autospec-core --test execution_queue` and confirm the missing APIs fail.

### Task 2: Share persistence internals without a new dependency

**Files:**
- Move: `crates/autospec-core/src/state/json.rs` → `crates/autospec-core/src/json.rs`
- Move/refactor: `crates/autospec-core/src/state/storage.rs` → `crates/autospec-core/src/persistence.rs`
- Modify: `crates/autospec-core/src/lib.rs`
- Modify: `crates/autospec-core/src/state/mod.rs`

- [ ] Make the JSON value/parser and recovery-aware path helpers `pub(crate)` rather than public API.
- [ ] Parameterize filenames and relative directories while retaining primary-wins and temporary recovery semantics.
- [ ] Keep all state persistence tests green after the refactor.

### Task 3: Add durable queue state and resume discovery

**Files:**
- Modify: `crates/autospec-core/src/execution/queue.rs`
- Modify: `crates/autospec-core/src/execution/mod.rs`
- Modify: `crates/autospec-core/tests/execution_queue.rs`

- [ ] Add timestamps and validation-result metadata with deterministic `*_at` transition methods.
- [ ] Serialize/parse the versioned queue document and validate every entry before write or load.
- [ ] Persist a named queue to `.autospec/runs/<run-id>/queue.json` with recovery behavior.
- [ ] Implement deterministic `load_latest_incomplete(root)` over valid immediate run directories.
- [ ] Keep existing handoff/report behavior and focused queue tests green.

### Task 4: Document and verify the bounded slice

**Files:**
- Modify: `docs/workflows.md`
- Modify: `schemas/autospec-run-report.schema.json` only if its current queue/run contract needs alignment.

- [ ] Document durable queue/resume-model behavior and the still-non-executing CLI boundary.
- [ ] Run `cargo test --workspace`, `cargo fmt --all --check`, and `cargo clippy --workspace --all-targets -- -D warnings`.
- [ ] Run `bash scripts/validate.sh --fast` and independently review the diff.
- [ ] Commit with a conventional Lore-format message before starting agent ingestion or CLI wiring.
