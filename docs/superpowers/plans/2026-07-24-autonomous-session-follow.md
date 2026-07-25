# Autonomous Session Follow Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add durable start-or-attach progress streaming and make it the default when the autonomous skill is invoked from Codex, Claude, or OpenCode.

**Architecture:** Keep the conductor, monitor, and supervisor in their existing detached process groups. Add launch-mode parsing and a repository-scoped follower to the Rust command boundary; interactive skill adapters request that mode by default while raw CLI startup remains detached.

**Tech Stack:** Rust 2021, existing `autospec-cli` process/lifecycle helpers, Bash/Bats validation, Markdown multi-harness skill adapters.

## Global Constraints

- Raw `autospec autonomous start` remains detached.
- `--follow`, `--detach`, and `--foreground` are mutually exclusive launch modes.
- `start --follow` attaches to a live scoped conductor without restarting it.
- `Ctrl-C` or caller termination affects only the follower.
- No desktop notifications are introduced.
- Edit only the canonical `skills/autospec-autonomous/SKILL.md`, then derive both mirrors with `scripts/derive-trio.sh`.
- Add no dependencies.

---

### Task 1: Rust launch-mode contract

**Files:**
- Modify: `crates/autospec-cli/src/commands/autonomous.rs`
- Test: `crates/autospec-cli/tests/autonomous_conductor_commands.rs`

**Interfaces:**
- Consumes: existing `Options`, `RunLayout`, `UnitStatus`, `start_after_lease`, and `read_unit`.
- Produces: `LaunchMode`, `validate_launch_mode(&Options)`, and a `start` path that can return immediately or follow.

- [ ] **Step 1: Write failing parser/help tests**

Add focused command tests that assert:

```rust
#[test]
fn autonomous_help_documents_all_start_modes() {
    let output = ForegroundFixture::new()
        .configured_command()
        .args(["autonomous", "--help"])
        .output()
        .expect("print autonomous help");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--follow"));
    assert!(stdout.contains("--detach"));
    assert!(stdout.contains("--foreground"));
}

#[test]
fn start_rejects_conflicting_launch_modes_before_mutation() {
    let fixture = ForegroundFixture::new();
    let output = fixture
        .configured_command()
        .args(["autonomous", "start", "--follow", "--foreground"])
        .output()
        .expect("reject conflicting modes");
    assert_eq!(output.status.code(), Some(2));
    assert!(!fixture.operator.exists());
}
```

- [ ] **Step 2: Run the focused tests and confirm RED**

Run:

```bash
cargo test -p autospec-cli --test autonomous_conductor_commands autonomous_help_documents_all_start_modes
cargo test -p autospec-cli --test autonomous_conductor_commands start_rejects_conflicting_launch_modes_before_mutation
```

Expected: failure because help omits `--follow`/`--detach` and the parser does not recognize them.

- [ ] **Step 3: Add the minimal typed launch-mode parser**

Extend `Options` with `follow: bool` and `detach: bool`, parse both flags, and validate before dispatch:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LaunchMode {
    Detached,
    Follow,
    Foreground,
}

fn validate_launch_mode(options: &Options) -> Result<LaunchMode, String> {
    let selected = usize::from(options.follow)
        + usize::from(options.detach)
        + usize::from(options.foreground);
    if selected > 1 {
        return Err("--follow, --detach, and --foreground are mutually exclusive".to_string());
    }
    if (options.follow || options.detach || options.foreground)
        && options.subcommand != "start"
    {
        return Err(format!(
            "launch modes are valid only with autospec autonomous start, not {}",
            options.subcommand
        ));
    }
    Ok(if options.follow {
        LaunchMode::Follow
    } else if options.foreground {
        LaunchMode::Foreground
    } else {
        LaunchMode::Detached
    })
}
```

Call the validator immediately after `parse`, route `LaunchMode::Foreground` to
`run_foreground`, and include all modes in `print_help`.

- [ ] **Step 4: Run the focused tests and confirm GREEN**

Run:

```bash
cargo test -p autospec-cli --test autonomous_conductor_commands autonomous_help_documents_all_start_modes
cargo test -p autospec-cli --test autonomous_conductor_commands start_rejects_conflicting_launch_modes_before_mutation
```

Expected: both tests pass.

---

### Task 2: Durable start-or-attach follower

**Files:**
- Modify: `crates/autospec-cli/src/commands/autonomous.rs`
- Test: `crates/autospec-cli/tests/autonomous_conductor_commands.rs`

**Interfaces:**
- Consumes: `LaunchMode::Follow`, `read_unit`, `UnitMetadataState`, `persisted_stop_mode`, and existing scoped log metadata.
- Produces: `follow_scoped_conductor(&RunLayout, &Options) -> Result<(), String>` and start-or-attach behavior.

- [ ] **Step 1: Write failing lifecycle tests**

Add integration tests with a blocked fake foreground worker:

```rust
#[test]
fn session_follow_attaches_without_restarting_and_detaches_safely() {
    let fixture = ForegroundFixture::new();
    fixture.initialize_git_remote();
    fixture.start_blocked_detached();
    let conductor_pid = fixture.recorded_conductor_pid().expect("conductor pid");

    let mut follower = fixture.spawn_following_start(1);
    wait_for_file_contents(&fixture.calls, "repos/test/repo/branches/main");
    assert_eq!(fixture.recorded_conductor_pid(), Some(conductor_pid));

    terminate_process_group(follower.id());
    let _ = follower.wait();
    assert!(process_is_running(conductor_pid));
    fixture.terminate_recorded_conductor();
}

#[test]
fn detached_flag_returns_after_start_without_following() {
    let fixture = ForegroundFixture::new();
    fixture.initialize_git_remote();
    let output = fixture
        .detached_command("start")
        .arg("--detach")
        .env("AUTOSPEC_AUTONOMOUS_COMPANIONS", "0")
        .output()
        .expect("explicit detached start");
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("autospec autonomous started"));
}
```

Add `start_blocked_detached(&self)`, `spawn_following_start(&self, u64) ->
std::process::Child`, and `terminate_recorded_conductor(&self)` fixture helpers.
They set `AUTOSPEC_FOREGROUND_BLOCK_GH=1`, isolate the follower process group,
wait for scoped PID metadata, and clean up the detached conductor.

- [ ] **Step 2: Run lifecycle tests and confirm RED**

Run:

```bash
cargo test -p autospec-cli --test autonomous_conductor_commands session_follow
cargo test -p autospec-cli --test autonomous_conductor_commands detached_flag
```

Expected: failure because `--follow`/`--detach` do not yet have runtime behavior.

- [ ] **Step 3: Implement start-or-attach**

Before acquiring a new lease, inspect the scoped conductor only in follow mode:

```rust
fn live_follow_target(layout: &RunLayout) -> Result<Option<UnitStatus>, String> {
    let unit = read_unit("conductor", layout);
    match unit.metadata_state {
        UnitMetadataState::Live => Ok(Some(unit)),
        UnitMetadataState::Absent | UnitMetadataState::Stale => Ok(None),
        UnitMetadataState::Ambiguous => Err(format!(
            "cannot follow ambiguous conductor metadata for {}",
            layout.repo
        )),
    }
}
```

If live, print an attach summary and call the scoped follower. Otherwise run the
existing lease/start transaction, print the normal start summary, then follow.
For `--dry-run --follow`, add a `follow: scoped conductor log` line without
creating state.

- [ ] **Step 4: Implement repository-scoped log following**

Replace the fixed-path assumption for session follow with a poll loop that:

```rust
fn follow_scoped_conductor(layout: &RunLayout, options: &Options) -> Result<(), String> {
    let mut logpath = String::new();
    let mut offset = 0usize;
    let mut iteration = 0u64;
    loop {
        iteration += 1;
        let unit = read_unit("conductor", layout);
        match unit.metadata_state {
            UnitMetadataState::Ambiguous => {
                return Err(format!(
                    "cannot follow ambiguous conductor metadata for {}",
                    layout.repo
                ));
            }
            UnitMetadataState::Live => {
                if unit.logpath != logpath {
                    println!("autospec autonomous follow: log={}", unit.logpath);
                    logpath = unit.logpath;
                    offset = 0;
                }
                offset = print_log_growth(&logpath, offset)?;
            }
            UnitMetadataState::Absent | UnitMetadataState::Stale => {
                if persisted_stop_mode(layout)?.is_some() {
                    println!("autospec autonomous follow: conductor stopped");
                    return Ok(());
                }
                let supervisor = read_unit("supervisor", layout);
                if !supervisor.running {
                    println!("autospec autonomous follow: conductor exited");
                    return Ok(());
                }
                println!("autospec autonomous follow: waiting for supervisor repair");
            }
        }
        if options.iterations > 0 && iteration >= options.iterations {
            return Ok(());
        }
        thread::sleep(Duration::from_secs(options.interval_sec.max(1)));
    }
}
```

`print_log_growth` must reset its offset when a logfile is truncated, flush
stdout after new content, and return the new byte offset.

- [ ] **Step 5: Run lifecycle tests and confirm GREEN**

Run:

```bash
cargo test -p autospec-cli --test autonomous_conductor_commands session_follow
cargo test -p autospec-cli --test autonomous_conductor_commands detached_flag
cargo test -p autospec-cli --test autonomous_conductor_commands detached_start
```

Expected: new tests pass and existing detached-start behavior remains green.

---

### Task 3: Interactive skill default and lock-step adapters

**Files:**
- Modify: `skills/autospec-autonomous/SKILL.md`
- Regenerate: `skills/autospec-autonomous/codex/prompt.md`
- Regenerate: `skills/autospec-autonomous/opencode/agent.md`
- Create: `tests/autonomous/test_session_follow_skill.bats`

**Interfaces:**
- Consumes: installed `autospec-autonomous` wrapper and Rust `start --follow`.
- Produces: deterministic interactive launch routing with explicit-mode precedence.

- [ ] **Step 1: Write the failing skill contract test**

Create a Bats test that checks the canonical body and derivation:

```bash
@test "interactive autonomous invocation follows by default" {
  run grep -F 'autospec-autonomous start --follow --repo-dir "$PWD"' \
    "$REPO_ROOT/skills/autospec-autonomous/SKILL.md"
  [ "$status" -eq 0 ]
}

@test "explicit launch modes override the interactive follow default" {
  run grep -F '`--detach` or `--foreground`' \
    "$REPO_ROOT/skills/autospec-autonomous/SKILL.md"
  [ "$status" -eq 0 ]
}

@test "autonomous skill adapters remain derived from the canonical body" {
  run "$REPO_ROOT/scripts/derive-trio.sh" \
    "$REPO_ROOT/skills/autospec-autonomous" --check
  [ "$status" -eq 0 ]
}
```

- [ ] **Step 2: Run the skill test and confirm RED**

Run:

```bash
bats tests/autonomous/test_session_follow_skill.bats
```

Expected: the default-follow assertions fail.

- [ ] **Step 3: Update the canonical invocation contract**

Add a direct-session section that instructs Codex, Claude, and OpenCode:

````markdown
## Direct interactive session launch

When invoked without an operator subcommand, or with an explicit `start`
subcommand and no explicit launch mode, run:

```bash
autospec-autonomous start --follow --repo-dir "$PWD"
```

Keep that tool call attached and forward its output to the initiating session.
If the operator supplies `--detach` or `--foreground`, preserve that explicit
mode and do not inject `--follow`. Never replace session output with a desktop
notification.
````

Add the three launch flags to the invocation synopsis and operator table.

- [ ] **Step 4: Derive and validate all adapters**

Run:

```bash
scripts/derive-trio.sh skills/autospec-autonomous --in-place
scripts/derive-trio.sh skills/autospec-autonomous --check
bats tests/autonomous/test_session_follow_skill.bats
```

Expected: derivation reports no drift and all Bats assertions pass.

---

### Task 4: Full verification and issue completion evidence

**Files:**
- Verify all files changed by Tasks 1-3.

**Interfaces:**
- Consumes: all preceding implementation and tests.
- Produces: formatter-clean, test-green, validation-green branch evidence.

- [ ] **Step 1: Format and run focused verification**

Run:

```bash
cargo fmt --all -- --check
cargo test -p autospec-cli --test autonomous_conductor_commands session_follow
cargo test -p autospec-cli --test autonomous_conductor_commands detached_start
bats tests/autonomous/test_session_follow_skill.bats
scripts/derive-trio.sh skills/autospec-autonomous --check
```

Expected: every command exits zero.

- [ ] **Step 2: Run the crate and repository gates**

Run:

```bash
cargo test -p autospec-cli
cargo clippy -p autospec-cli --all-targets -- -D warnings
autospec validate
```

Expected: all tests, Clippy, and repository validation pass with no skipped
required check.

- [ ] **Step 3: Inspect final scope and commit**

Run:

```bash
git diff --check
git status --short
git diff --stat origin/main...HEAD
```

Commit implementation and tests using Conventional Commit intent lines, Lore
trailers, the required OmX co-author trailer, and `Related: #2566`. Do not amend
the committed design checkpoint.
