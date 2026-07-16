# Rust Autonomous Drain Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the R1 shell drain watchdog behavior needed by #1826 with a
typed Rust `autospec autonomous drain` command that does not kill a progressing
child.

**Architecture:** `autospec-core` decides whether a drain child should wait,
warn, complete, or terminate from typed observations. `autospec-cli` owns the
child process, direct `gh` inspection, scoped heartbeat/artifact reads,
observation persistence, and direct process termination. The legacy shell
drain remains untouched until the later #2076 launcher-deletion child.

**Tech Stack:** Rust standard library, existing `autospec-core`, existing
`autospec-cli`, `gh`, and `omx`; no new dependencies.

## Global Constraints

- Keep all stateful watchdog policy in Rust; do not invoke `sh`, `bash`, or
  `scripts/autospec-autonomous-run-drain.sh`.
- Spawn `omx` and `gh` with direct argument vectors, never interpolated shell
  command strings.
- A completed child always wins over a timeout; no termination after a
  successful final `try_wait`.
- GitHub is checked only at the local stall boundary; a failed check is not
  progress evidence.
- Store no lease token or raw child output in the observation record.
- Preserve the existing shell launcher only as historical compatibility until a
  separate deletion issue redirects it to Rust.

---

### Task 1: Model drain decisions in the core crate

**Files:**
- Create: `crates/autospec-core/src/autonomous/drain.rs`
- Modify: `crates/autospec-core/src/autonomous/mod.rs`
- Test: `crates/autospec-core/tests/autonomous_drain.rs`

**Interfaces:**
- Produces `DrainObservation`, `DrainProgress`, and `DrainDecision` for the
  CLI adapter.
- Consumes no filesystem, process, environment, or GitHub data.

- [ ] **Step 1: Write the failing core decision tests**

```rust
use autospec_core::autonomous::drain::{decide, DrainDecision, DrainObservation, DrainProgress};

#[test]
fn external_progress_resets_a_quiet_live_child() {
    let observation = DrainObservation::live(30, 30, DrainProgress::Github);
    assert_eq!(decide(&observation), DrainDecision::WarnExternalProgress);
}

#[test]
fn a_completed_child_precedes_stall_termination() {
    let observation = DrainObservation::completed(124, 30, 30);
    assert_eq!(decide(&observation), DrainDecision::Complete { exit_code: 124 });
}
```

- [ ] **Step 2: Run the core test to verify it fails**

Run: `cargo test -p autospec-core --test autonomous_drain --quiet`

Expected: failure because the `autonomous::drain` module is absent.

- [ ] **Step 3: Implement the closed decision contract**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrainProgress { None, ChildOutput, Artifact, Heartbeat, Github }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrainDecision {
    Wait,
    WarnExternalProgress,
    Complete { exit_code: i32 },
    TerminateStalled,
}

pub fn decide(observation: &DrainObservation) -> DrainDecision {
    if let Some(exit_code) = observation.child_exit_code { return DrainDecision::Complete { exit_code }; }
    if observation.elapsed_secs < observation.stall_secs { return DrainDecision::Wait; }
    match observation.progress {
        DrainProgress::Heartbeat | DrainProgress::Github => DrainDecision::WarnExternalProgress,
        DrainProgress::ChildOutput | DrainProgress::Artifact => DrainDecision::Wait,
        DrainProgress::None => DrainDecision::TerminateStalled,
    }
}
```

- [ ] **Step 4: Run the core test to verify it passes**

Run: `cargo test -p autospec-core --test autonomous_drain --quiet`

Expected: all drain decision tests pass.

- [ ] **Step 5: Commit the pure policy**

```bash
git add crates/autospec-core/src/autonomous crates/autospec-core/tests/autonomous_drain.rs
git commit -m "feat: model autonomous drain progress decisions"
```

### Task 2: Add the Rust drain subprocess adapter and persisted observation

**Files:**
- Modify: `crates/autospec-cli/src/commands/autonomous.rs`
- Create: `crates/autospec-cli/src/commands/autonomous/drain.rs`
- Test: `crates/autospec-cli/tests/autonomous_drain_commands.rs`

**Interfaces:**
- Consumes `autospec_core::autonomous::drain::{decide, DrainDecision,
  DrainObservation, DrainProgress}`.
- Produces `autospec autonomous drain --repo OWNER/REPO --repo-dir PATH
  [--stall-secs N] [--poll-secs N] [--json]`.
- Writes `<operator-scope>/drain-observation.json` with schema version,
  timestamp, progress source, warning state, and decision.

- [ ] **Step 1: Write failing CLI integration tests**

```rust
#[test]
fn quiet_child_with_a_scoped_heartbeat_completes_without_termination() {
    let output = fixture.run_drain_with_heartbeat_progress();
    assert!(output.status.success());
    assert!(!stdout(&output).contains("terminate"));
}

#[test]
fn quiet_child_with_github_progress_warns_and_completes() {
    let output = fixture.run_drain_with_github_progress();
    assert!(output.status.success());
    assert!(stdout(&output).contains("quiet_stdout_external_progress"));
}

#[test]
fn silent_live_child_is_terminated_only_after_final_exit_reconciliation() {
    let output = fixture.run_stalled_drain();
    assert_eq!(output.status.code(), Some(124));
    assert!(fixture.termination_marker().exists());
}
```

- [ ] **Step 2: Run the integration test to verify it fails**

Run: `cargo test -p autospec-cli --test autonomous_drain_commands --quiet`

Expected: failure because `autonomous drain` is not a recognized subcommand.

- [ ] **Step 3: Implement direct child supervision**

```rust
Command::new("omx")
    .args(["exec", "--cd", repo_dir, "--dangerously-bypass-approvals-and-sandbox", "$autospec-run"])
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()
```

Use reader threads to forward child output and atomically record output activity.
On each poll, sample artifact/heartbeat mtimes. At a local timeout, call the
existing direct `gh` adapter once to compare active issue/PR state to the last
snapshot. Convert the result into `DrainObservation`, persist the decision,
and call `child.try_wait()` again before a `TerminateStalled` kill. Return the
child's real exit code if it completed.

- [ ] **Step 4: Run the integration test to verify it passes**

Run: `cargo test -p autospec-cli --test autonomous_drain_commands --quiet`

Expected: all quiet-progress, completed-child, and true-stall cases pass.

- [ ] **Step 5: Commit the adapter**

```bash
git add crates/autospec-cli/src/commands/autonomous.rs crates/autospec-cli/src/commands/autonomous/drain.rs crates/autospec-cli/tests/autonomous_drain_commands.rs
git commit -m "feat: supervise autonomous drain progress in Rust"
```

### Task 3: Prove the new command cannot restore shell authority

**Files:**
- Modify: `crates/autospec-cli/tests/autonomous_drain_commands.rs`
- Modify: `docs/cli-reference.md`
- Modify: `docs/superpowers/specs/2026-07-16-rust-autonomous-drain-design.md`

**Interfaces:**
- Documents the new command and the later #2076 wrapper-handoff boundary.
- Produces a static source assertion rejecting shell and legacy-drain paths.

- [ ] **Step 1: Write the failing negative-reachability test**

```rust
#[test]
fn rust_drain_source_does_not_restore_shell_or_legacy_drain_authority() {
    let source = fs::read_to_string(workspace_root().join("crates/autospec-cli/src/commands/autonomous/drain.rs"))?;
    for forbidden in ["Command::new(\"sh\")", "Command::new(\"bash\")", "autospec-autonomous-run-drain.sh"] {
        assert!(!source.contains(forbidden), "forbidden authority: {forbidden}");
    }
}
```

- [ ] **Step 2: Run the test to verify it fails before the guard exists**

Run: `cargo test -p autospec-cli --test autonomous_drain_commands rust_drain_source_does_not_restore_shell_or_legacy_drain_authority --quiet`

Expected: failure because the test has not yet been added.

- [ ] **Step 3: Add the static guard and CLI documentation**

Document explicit options, structured warning/termination JSON, direct `omx`
integration, and the fact that shell launch wiring remains a separate deletion
child. Do not document a legacy fallback.

- [ ] **Step 4: Run focused and full validation**

Run:

```bash
cargo fmt --all --check
cargo clippy --workspace -- -D warnings
cargo test --workspace --quiet
cargo run -q -p autospec-cli -- validate --fast
```

Expected: all commands pass.

- [ ] **Step 5: Commit the guard and docs**

```bash
git add crates/autospec-cli/tests/autonomous_drain_commands.rs docs/cli-reference.md docs/superpowers/specs/2026-07-16-rust-autonomous-drain-design.md
git commit -m "test: prevent legacy shell drain authority from returning"
```

## Plan self-review

- The core policy owns no I/O and the CLI owns every external adapter.
- Every #1826 acceptance requirement maps to Task 1 or Task 2.
- No task delegates implementation to a shell command string or restores the
  legacy drain script.
- #1602, #1872, and #1697 are explicitly kept out of this narrow child so they
  can be implemented and reviewed independently before the #2076 deletion.
