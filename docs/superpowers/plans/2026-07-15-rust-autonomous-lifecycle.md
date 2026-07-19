# Rust Autonomous Lifecycle Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the shell-owned autonomous lifecycle policy and command-string unit launches with typed Rust policy, a testable CLI decision endpoint, and native process arguments.

**Architecture:** `autospec-core::autonomous_lifecycle` is a pure policy module with validated state and a single `decide` function. `autospec-cli::commands::autonomous` parses the JSON-facing lifecycle command and translates lifecycle decisions into existing start, restart, stop, and foreground admission behavior; it owns filesystem and child-process effects. The runtime launches its own executable with typed argv values, never `sh -c` or an environment-provided command string.

**Tech Stack:** Rust workspace; `std` only; existing manual JSON helpers and integration fixtures.

## Global Constraints

- Add no dependencies.
- Keep the primary checkout read-only; make every edit in `/private/tmp/wt-issue-2079`.
- Preserve public `autospec autonomous start|restart|stop|run-foreground` command names and JSON fields unless a tested lifecycle field is added.
- Keep all shell, GitHub, `omx`, filesystem, and process side effects outside `autospec-core`.
- Reject malformed, cross-scope, stale, terminal, and ownership-mismatched lifecycle inputs before any executable decision.
- Preserve the 300-second and 10,800-second stale thresholds and the per-issue failure cap of three as explicit typed values.
- Do not delete legacy scripts, installer fallbacks, Bats suites, or their documentation in this issue.

---

### Task 1: Add pure typed lifecycle policy

**Files:**
- Create: `crates/autospec-core/src/autonomous_lifecycle.rs`
- Modify: `crates/autospec-core/src/lib.rs`
- Create: `crates/autospec-core/tests/autonomous_lifecycle.rs`

**Interfaces:**
- Produces: `LifecycleInput`, `LifecycleDecision`, `LifecycleTier`, `LifecycleReject`, and `decide(&LifecycleInput) -> LifecycleDecision`.
- Consumes: no CLI or filesystem types; all inputs are validated primitive values and closed enums.

- [ ] **Step 1: Write the failing core tests**

```rust
#[test]
fn stop_precedes_ready_tier_one() {
    let input = LifecycleInput::ready("owner/repo").with_stop(StopMode::Graceful);
    assert_eq!(decide(&input), LifecycleDecision::Stop { mode: StopMode::Graceful });
}

#[test]
fn stale_or_cross_scope_claim_is_non_executable() {
    assert!(matches!(decide(&LifecycleInput::stale_claim()), LifecycleDecision::Reject(_)));
    assert!(matches!(decide(&LifecycleInput::cross_scope_claim()), LifecycleDecision::Reject(_)));
}
```

- [ ] **Step 2: Run the core test target to verify it fails**

Run: `cargo test -p autospec-core --test autonomous_lifecycle`

Expected: FAIL because `autospec_core::autonomous_lifecycle` does not exist.

- [ ] **Step 3: Implement the closed policy types and precedence**

```rust
pub enum LifecycleTier { Tier1, Tier15, Tier2, Tier3, Tier4, Tier5, Tier6, Tier7, Idle }
pub enum LifecycleDecision {
    Stop { mode: StopMode },
    Reject(LifecycleReject),
    Park { reason: ParkReason },
    Run { tier: LifecycleTier },
}
pub fn decide(input: &LifecycleInput) -> LifecycleDecision {
    if let Some(mode) = input.stop_mode { return LifecycleDecision::Stop { mode }; }
    if let Some(reason) = input.reject_reason() { return LifecycleDecision::Reject(reason); }
    if let Some(reason) = input.park_reason() { return LifecycleDecision::Park { reason }; }
    LifecycleDecision::Run { tier: input.next_tier() }
}
```

Implement explicit stop, human, health, budget, ownership, ready, promotion, discovery, growth, and idle-rescan branches. Expose `STALE_LEASE_SECS`, `ABANDONED_LEASE_SECS`, and `ISSUE_FAILURE_CAP` constants.

- [ ] **Step 4: Extend core coverage for every tier and rejection**

Add table-driven cases for tiers `1`, `1.5`, `2` through `7`, `idle`, health/budget parks, terminal claims, failure cap, and both stale thresholds. Assert no rejected input returns `Run`.

- [ ] **Step 5: Run the focused core target and commit**

Run: `cargo test -p autospec-core --test autonomous_lifecycle`

Expected: PASS.

Commit with a Lore-formatted `feat:` message describing why policy must be independent of shell effects.

### Task 2: Expose lifecycle decisions through the autonomous CLI

**Files:**
- Modify: `crates/autospec-cli/src/commands/autonomous.rs`
- Create: `crates/autospec-cli/tests/autonomous_lifecycle_commands.rs`

**Interfaces:**
- Consumes: `autospec_core::autonomous_lifecycle::{decide, LifecycleInput, LifecycleDecision}`.
- Produces: `autospec autonomous lifecycle decide` JSON containing one `decision` and optional `tier` or `reason`.

- [ ] **Step 1: Write failing CLI integration tests**

```rust
let output = Command::new(env!("CARGO_BIN_EXE_autospec"))
    .args(["autonomous", "lifecycle", "decide", "--repo", "test/repo", "--ready-tier", "1"])
    .output().expect("run lifecycle decision");
assert_eq!(String::from_utf8_lossy(&output.stdout), "{\"decision\":\"run\",\"tier\":\"1\"}\n");
```

Add separate cases for unknown flags, repeated flags, cross-scope claim repo, stale lease, immediate stop, health halt, budget park, and idle-rescan.

- [ ] **Step 2: Run the new CLI test to verify it fails**

Run: `cargo test -p autospec-cli --test autonomous_lifecycle_commands`

Expected: FAIL because `lifecycle` is not an autonomous subcommand.

- [ ] **Step 3: Parse only explicit lifecycle flags and serialize decisions**

Add a `lifecycle(args)` dispatch before generic `Options::parse`. Require one `decide` action, a non-empty canonical `--repo`, and exactly one value for every optional typed flag. Map accepted strings to core enums; return the existing malformed-input class for invalid values. Serialize `Run`, `Stop`, `Park`, and `Reject` with stable JSON and no side effects.

- [ ] **Step 4: Run focused CLI regression tests**

Run: `cargo test -p autospec-cli --test autonomous_lifecycle_commands`

Expected: PASS with JSON bytes and nonzero classes asserted for invalid input.

- [ ] **Step 5: Commit the CLI policy boundary**

Commit with a Lore-formatted `feat:` message documenting the stable decision protocol and its compatibility boundary.

### Task 3: Remove command-string unit spawning from start and restart

**Files:**
- Modify: `crates/autospec-cli/src/commands/autonomous.rs`
- Modify: `crates/autospec-cli/tests/cli_commands.rs`
- Modify: `crates/autospec-cli/tests/autonomous_lifecycle_commands.rs`

**Interfaces:**
- Produces: `NativeUnitCommand { program: PathBuf, args: Vec<String> }` and one native spawning path for conductor, monitor, and supervisor.
- Replaces: `LaunchCommands`, `spawn_unit`, `AUTOSPEC_AUTONOMOUS_MONITOR_CMD`, and `AUTOSPEC_AUTONOMOUS_SUPERVISOR_CMD` command-string authority.

- [ ] **Step 1: Write failing regression tests for native launch authority**

```rust
assert!(!source.contains("Command::new(\"sh\")"));
assert!(!source.contains("AUTOSPEC_AUTONOMOUS_MONITOR_CMD"));
assert!(!source.contains("AUTOSPEC_AUTONOMOUS_SUPERVISOR_CMD"));
```

Update detached start/restart fixtures to set `AUTOSPEC_AUTONOMOUS_COMPANIONS=0` when they do not exercise companions, and add one start/restart assertion that `launch.json` records native argv rather than a shell command.

- [ ] **Step 2: Run the affected CLI tests to verify they fail**

Run: `cargo test -p autospec-cli --test cli_commands autonomous_start`

Expected: FAIL while the string-command launch path remains.

- [ ] **Step 3: Replace shell launch commands with native argv**

```rust
fn unit_command(options: &Options, subcommand: &str) -> Result<NativeUnitCommand, String> {
    Ok(NativeUnitCommand { program: std::env::current_exe()?, args: unit_args(options, subcommand) })
}
```

Use `Command::new(&command.program).args(&command.args)` for every unit, preserve log and PID files, and persist argv arrays in `launch.json`. Make start and restart evaluate a lifecycle admission before starting any unit.

- [ ] **Step 4: Run launch and lifecycle regressions**

Run: `cargo test -p autospec-cli --test cli_commands autonomous_start && cargo test -p autospec-cli --test autonomous_lifecycle_commands`

Expected: PASS with no command-string fallback.

- [ ] **Step 5: Commit native runtime dispatch**

Commit with a Lore-formatted `refactor:` message explaining that direct argv prevents legacy shell authority from re-entering the Rust control plane.

### Task 4: Document compatibility, rollback, and verification

**Files:**
- Modify: `docs/specs/2026-07-15-rust-autonomous-lifecycle.md`
- Create: `docs/runbooks/rust-autonomous-lifecycle-rollback.md`
- Modify: `docs/cli-reference.md`

**Interfaces:**
- Documents: lifecycle JSON, stale thresholds, legacy dual-layout reads, native spawning, and rollback criteria.

- [ ] **Step 1: Write the rollback contract**

Document the stop command, verification of the lifecycle decision endpoint, archived state preservation, and the condition for restoring a prior compatible Rust release. State that this issue does not restore the deleted shell waterfall.

- [ ] **Step 2: Document CLI examples**

Add exact `autonomous lifecycle decide` examples for Tier 1, stop, stale rejection, and idle-rescan. Document that decision mode has no process or GitHub side effects.

- [ ] **Step 3: Verify documentation references match code**

Run: `rg -n 'lifecycle decide|STALE_LEASE_SECS|AUTOSPEC_AUTONOMOUS_MONITOR_CMD|sh -c' docs crates/autospec-cli/src/commands/autonomous.rs`

Expected: lifecycle terms match production names; retired environment command names and `sh -c` have no live Rust launch reference.

- [ ] **Step 4: Run the full local quality gate and commit**

Run: `cargo test --workspace --quiet && cargo fmt --all --check && cargo clippy --workspace -- -D warnings && cargo run -q -p autospec-cli -- validate --fast && git diff --check`

Expected: all commands exit zero.

Commit with a Lore-formatted `docs:` message describing the cutover and its rollback limits.
