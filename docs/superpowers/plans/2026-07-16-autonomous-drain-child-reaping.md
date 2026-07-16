# Autonomous Drain Child Reaping Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Restore the Rust workspace by reaping terminated drain children and keeping Rust-native CLI fixtures alive through their assertions.

**Architecture:** Keep the existing TERM-then-KILL process-group policy. Move child reaping into the bounded process-group observation loop so the leader's zombie entry cannot keep `kill -0 -<pgid>` successful; continue polling the group after the leader is reaped to retain descendant cleanup guarantees.

**Tech Stack:** Rust standard library process APIs and the existing `autospec-cli` integration test harness.

## Global Constraints

- Keep the change limited to issue #2100 and the regressions introduced by the Rust-native autonomous migration.
- Preserve exit code `124` for a drain terminated after its stall window.
- Preserve TERM escalation to KILL for descendants that ignore TERM.
- Add no dependencies and retain real-process integration coverage.

---

### Task 1: Reap the process-group leader during bounded termination polling

**Files:**
- Modify: `crates/autospec-cli/src/commands/autonomous/drain.rs:299-351`
- Test: `crates/autospec-cli/tests/autonomous_drain_commands.rs:403-442`

**Interfaces:**
- Consumes: `Child::try_wait`, `process_group_is_alive`, and the existing `terminate_child` TERM/KILL flow.
- Produces: `wait_for_process_group_exit(&mut Child, &str) -> Result<bool, CommandFailure>` that reaps the leader before testing group liveness.

- [x] **Step 1: Name the regression after the required reaping behavior**

Rename the existing test without changing its assertions:

```rust
#[test]
fn silent_live_child_is_reaped_before_process_group_liveness_check() {
```

- [x] **Step 2: Run the regression suite and verify RED**

Run:

```bash
cargo test -p autospec-cli --test autonomous_drain_commands
```

Expected: four stall tests fail with status `2` and `drain child process group did not exit`; the renamed reaping test is one of them.

- [x] **Step 3: Reap the leader inside each process-group observation iteration**

Update both call sites and the helper:

```rust
if wait_for_process_group_exit(child, &process_group)? {
    return Ok(ChildTermination::Terminated);
}

fn wait_for_process_group_exit(
    child: &mut Child,
    process_group: &str,
) -> Result<bool, CommandFailure> {
    for _ in 0..20 {
        child.try_wait().map_err(child_status_error)?;
        if !process_group_is_alive(process_group)? {
            return Ok(true);
        }
        thread::sleep(OBSERVER_POLL_INTERVAL);
    }
    Ok(false)
}
```

Remove the now-redundant `child.wait()` calls after successful group-exit observation. A `true` result is only possible after `try_wait` has reaped the leader and no descendant remains in the group.

- [x] **Step 4: Run focused and workspace verification**

Run:

```bash
cargo fmt --check
cargo test -p autospec-cli --test autonomous_drain_commands
cargo test --workspace
```

Expected: every command exits `0`; the workspace reports one intentionally ignored aborting child helper that its parent test executes directly.

- [x] **Step 5: Prepare the scoped fix for commit**

Stage only the plan, drain implementation, and drain integration test, then commit with the repository Lore trailers recording the zombie-leader root cause and verification.

---

### Task 2: Keep native foreground fixtures alive without removed command overrides

**Files:**
- Modify: `crates/autospec-cli/tests/cli_commands.rs:1535-3063,3233-3236`

**Interfaces:**
- Consumes: `fake_bin`, `path_with`, the real Rust-native foreground conductor, and POSIX parent-process liveness.
- Produces: `hermetic_autonomous_path` with a fake `gh` process that remains active only while its real foreground parent exists.

- [x] **Step 1: Verify the stale fixture failure**

Run:

```bash
cargo test -p autospec-cli --test cli_commands
```

Expected: stop/status/lease assertions fail because the foreground conductor exits before the test inspects it.

- [x] **Step 2: Keep the foreground command alive for its parent lifetime**

Replace the immediate fake GitHub failure with this bounded process-lifetime fixture:

```rust
fn hermetic_autonomous_path(fixture: &std::path::Path) -> String {
    let bin = fake_bin(
        fixture,
        None,
        Some("#!/bin/sh\nwhile kill -0 \"$PPID\" 2>/dev/null; do sleep 0.1; done\nexit 1\n"),
    );
    path_with(&bin)
}
```

The real Rust foreground conductor blocks in this real child process during assertions. When cleanup terminates the conductor, the child observes the missing parent and exits without leaving a long-running process.

- [x] **Step 3: Remove dead command-override inputs**

Delete every test assignment of `AUTOSPEC_AUTONOMOUS_MONITOR_CMD` and `AUTOSPEC_AUTONOMOUS_SUPERVISOR_CMD`; the Rust-native launcher intentionally does not consume them.

- [x] **Step 4: Run focused and workspace verification**

Run:

```bash
rg -n 'AUTOSPEC_AUTONOMOUS_(MONITOR|SUPERVISOR)_CMD' crates/autospec-cli/tests/cli_commands.rs
cargo test -p autospec-cli --test cli_commands
cargo test --workspace
```

Expected: `rg` returns no matches and both Cargo commands exit `0`; the workspace reports only its intentional aborting child helper as ignored.

- [x] **Step 5: Prepare the complete regression repair for commit**

Stage the three Rust files and this plan, then commit with the repository Lore trailers recording both root causes and the complete workspace verification.

---

### Task 3: Remove the deleted validation entrypoint from tracked report prose

**Files:**
- Modify: `.superpowers/sdd/task-3-implementer-report.md:78`

**Interfaces:**
- Consumes: the validation-parity test's tracked-file scan.
- Produces: historical verification prose that does not reproduce a deleted shell entrypoint's exact path.

- [x] **Step 1: Verify the parity regression**

Run:

```bash
cargo test -p autospec-cli --test validation_parity
```

Expected: `legacy_validation_surfaces_are_absent_from_tracked_files` fails on the tracked implementer report.

- [x] **Step 2: Preserve the report meaning without the deleted path**

Replace the final sentence with:

```markdown
The root checkout has no legacy validation shell entrypoint; Rust tests, formatting, lint, and diff checks are the available verification surface for this change.
```

- [x] **Step 3: Verify parity and the workspace**

Run:

```bash
cargo test -p autospec-cli --test validation_parity
cargo test --workspace
```

Expected: both commands exit `0`; the workspace reports only its intentional aborting child helper as ignored.

---

### Task 4: Clear the strict clippy baseline

**Files:**
- Modify: `crates/autospec-cli/tests/explore_commands.rs:228`
- Modify: `crates/autospec-cli/src/commands/autonomous.rs:3746-3791`

**Interfaces:**
- Consumes: existing exploration assertions and autonomous help rendering.
- Produces: equivalent behavior accepted by `clippy -D warnings`.

- [x] **Step 1: Verify strict clippy RED**

Run:

```bash
cargo clippy --workspace --all-targets -- -D warnings
```

Expected: `len_zero` and `items_after_test_module` fail the command.

- [x] **Step 2: Use the slice emptiness API**

Replace the exploration assertion with:

```rust
assert!(!json["domains"].as_array().unwrap().is_empty());
```

- [x] **Step 3: Keep non-test help code before the test module**

Move the unchanged `print_help` function above `#[cfg(test)] mod foreground_tests` so no production item follows the test module.

- [x] **Step 4: Verify lint, build, and workspace tests**

Run:

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo build --workspace
cargo test --workspace
```

Expected: every command exits `0`; the workspace continues to report the one intentional aborting child helper as ignored.
