# Rust Foreground Conductor Implementation Plan

**Goal:** Cut `autospec autonomous run-foreground`, `start`, and `restart`
over to a typed Rust foreground control path without claiming that deferred
implementation work has succeeded.

**Architecture:** The command layer obtains queue decisions through
crate-visible Rust helpers, drives the pure core conductor, records an explicit
Rust-only executor request/result, and preserves the selected claim in a
repository-scoped state file partitioned by repository or exact slice. Detached launch uses a program and argument
vector for the current executable; only companion monitor/supervisor behavior
continues to use its existing compatibility launch path.

**Tech stack:** Existing Rust workspace, `autospec-core` conductor/ready queue,
and Rust integration tests. No new dependencies.

## Constraints

- Do not invoke `bash`, a script path, `sh -c`, a shell command override, or a
  fallback conductor from any live foreground route.
- Do not launch an external implementation agent or report a deferred executor
  response as a successful implementation.
- Keep the existing claim safety and reconciliation policies authoritative.
- Preserve a selected, deferred issue across a new foreground invocation.

### Task 1: Expose typed queue cycle inputs

**Files:**

- Modify `crates/autospec-cli/src/commands/queue.rs`
- Test `crates/autospec-cli/tests/autonomous_conductor_commands.rs`

1. Add crate-visible bounded review and ready-plan helpers that reuse existing
   CLI policies and return typed results without printing.
2. Keep public `queue` output unchanged by delegating to those helpers.
3. Write a failing foreground regression with fake GitHub responses.

### Task 2: Implement the typed foreground cycle

**Files:**

- Modify `crates/autospec-cli/src/commands/autonomous.rs`
- Modify `crates/autospec-cli/tests/cli_commands.rs`
- Add `crates/autospec-cli/tests/autonomous_conductor_commands.rs`

1. Add private `ExecutorRequest` and the Rust-only `executor-result` protocol.
2. Drive health, queue review/selection, claim acquisition, persistence, direct
   program-and-argument child execution, result recording, and reconciliation.
3. Persist strict core state; on a later process, retain a paused selection
   instead of dispatching it again.
4. Replace the foreground script and `bash` launch.
5. Replace the detached conductor lane of `start` and `restart` with direct
   current-executable spawning; monitor/supervisor compatibility stays scoped
   outside this change.

### Task 3: Document and verify the boundary

**Files:**

- Modify `docs/cli-reference.md`
- Modify `docs/workflows.md`
- Modify `docs/superpowers/specs/2026-07-15-rust-foreground-conductor-design.md`
- Modify `docs/superpowers/plans/2026-07-15-rust-foreground-conductor.md`

1. Document Rust-owned foreground state, its deferred outcome, and `ALL_DONE`.
2. Run focused and full workspace validation, formatter, Clippy, fast validator,
   smoke check, diff check, and implementation linter.
3. Independently review for legacy foreground reachability before merge.

## Verification commands

Run `cargo test -p autospec-cli --test autonomous_conductor_commands`,
`cargo test --workspace --quiet`, `cargo fmt --all --check`,
`cargo clippy --workspace -- -D warnings`,
`cargo run -q -p autospec-cli -- validate --fast`,
`bash tests/smoke/explore_metabolomics_scan.sh`, and `git diff --check`.
