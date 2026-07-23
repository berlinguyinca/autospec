# Rust Autonomous Lifecycle Detach Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Keep Rust-launched autonomous lifecycle units alive after the launcher exits and preserve status compatibility with live legacy plain-PID metadata.

**Architecture:** The Rust launcher will place each lifecycle unit in its own Unix process group before spawning it, matching the isolation already used by supervised child processes. PID metadata classification will accept a plain positive PID only as a compatibility record whose state is derived from process liveness; scoped JSON metadata retains strict repository and scope validation.

**Tech Stack:** Rust, `std::os::unix::process::CommandExt`, `nix`, Cargo integration tests.

## Global Constraints

- Work only in a linked `feat/*` worktree for GitHub issue #2476.
- Add no dependencies.
- Follow TDD: each behavior must fail for the expected reason before implementation.
- Preserve fail-closed handling for malformed, foreign-repository, foreign-scope, and indeterminate metadata.
- Do not modify the running shell-fallback conductor in `autospec-gui`.

---

### Task 1: Rust lifecycle detachment and legacy PID compatibility

**Files:**
- Modify: `crates/autospec-cli/src/commands/autonomous.rs`
- Modify: `crates/autospec-cli/tests/autonomous_conductor_commands.rs`
- Modify: `crates/autospec-cli/tests/cli_commands.rs`

**Interfaces:**
- Consumes: `spawn_unit()`, `classify_unit_metadata()`, `ProcessProbe`, and the existing detached lifecycle fixture.
- Produces: Unix child process-group isolation and compatible classification of live or missing plain positive PID records.

- [ ] **Step 1: Write failing regression tests**

Add one integration test proving a detached `start` child remains alive after the launcher command exits, with fixture cleanup that terminates the recorded process group. Extend `autonomous_metadata_tests` so plain `"42"` metadata is `Live` with `ProcessProbe::Alive`, `Stale` with `ProcessProbe::Missing`, and remains `Ambiguous` with `ProcessProbe::Indeterminate`.

Add a CLI fixture regression proving cleanup terminates both scoped JSON PID metadata and legacy plain PID metadata.

- [ ] **Step 2: Verify the regressions fail**

Run:

```bash
cargo test -p autospec-cli --test autonomous_conductor_commands
```

Expected: the new lifecycle-survival assertion and legacy PID classification assertions fail against the current implementation.

- [ ] **Step 3: Implement the minimal production changes**

On Unix, call `process.process_group(0)` in `spawn_unit()` before `spawn()`. In `classify_unit_metadata()`, recognize a raw positive integer as legacy metadata and return `Live` or `Stale` only from a determinate process probe; keep JSON repo/scope identity checks unchanged.

Update CLI test cleanup to parse both metadata formats and terminate the recorded process group with a direct-PID fallback.

- [ ] **Step 4: Verify focused and full validation**

Run:

```bash
cargo test -p autospec-cli --test autonomous_conductor_commands
cargo test -p autospec-cli --test cli_commands autonomous_status
autospec validate
```

Expected: all focused tests pass and `autospec validate` reports zero required failures.

- [ ] **Step 5: Commit and publish**

Commit with a Lore-format `fix:` message, push `feat/rust-autonomous-lifecycle-detach`, and open a PR closing #2476.
