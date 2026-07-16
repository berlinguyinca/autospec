# Rust Autonomous Resilience Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Rust the sole writer of autonomous resilience, failure, and spend state while safely reading the three established legacy slug layouts.

**Architecture:** `autospec-core` supplies a pure local-conductor lease and capacity evaluator, separate from the GitHub-backed issue-claim lease. A focused CLI adapter owns compatibility decoding, path resolution, PID probing, and atomic writes; `autonomous.rs` only routes commands and integrates resulting typed admission.

**Tech Stack:** Rust standard library, `autospec_core::autonomous_lifecycle`, existing no-dependency JSON helpers, Rust integration tests.

## Global Constraints

- New resilience, failure, and spend writes use `owner__repo`; read order is `owner__repo`, `owner_repo`, then `owner-repo`.
- `autonomous-operator/owner_repo` remains lifecycle-only and must not become a resilience writer.
- Malformed or foreign compatible state fails closed before a write; shell helpers are never invoked.
- Reclaim boundaries are inclusive: claimed at `>=300`, any status at `>=10800`, and same-host dead PID immediately.
- Failure state is monotonic; usage cap precedes issue cap; a zero cap disables that cap.
- No dependency or shell command authority is added. Every behavior starts with an observed failing test.

---

## File Structure

| File | Responsibility |
| --- | --- |
| `crates/autospec-core/src/autonomous_lifecycle.rs` | Pure local conductor lease and capacity decisions. |
| `crates/autospec-core/tests/autonomous_lifecycle.rs` | Exact boundary and capacity-order tests. |
| `crates/autospec-cli/src/commands/autonomous/resilience.rs` | Layout paths, atomic writer, lease/cap admission, and diagnostic adapter. |
| `crates/autospec-cli/src/commands/autonomous/resilience/records.rs` | Strict resilience, failure, and spend record parsing and serialization. |
| `crates/autospec-core/src/state/{mod.rs,json.rs}` | Narrow public seam for the existing strict parser; no parser duplication. |
| `crates/autospec-cli/src/commands/autonomous.rs` | Command routing and start/restart/status/foreground integration. |
| `crates/autospec-cli/tests/autonomous_resilience_commands.rs` | Black-box temporary-root command tests. |
| `docs/specs/2026-07-15-rust-autonomous-lifecycle.md` | Runtime compatibility contract. |
| `docs/cli-reference.md` | Public diagnostic command reference. |

### Task 1: Model pure local resilience policy

**Files:**
- Modify: `crates/autospec-core/src/autonomous_lifecycle.rs`
- Modify: `crates/autospec-core/tests/autonomous_lifecycle.rs`

**Interfaces:**
- Produces `ConductorLeaseInput`, `ConductorLeaseDecision`, `ConductorLeaseReclaim`, `CapacityInput`, `CapacityDecision`, `decide_conductor_lease`, and `decide_capacity`.
- The existing `LifecycleInput::with_failure_count` remains the one failure-cap evaluator.

- [ ] **Step 1: Write failing pure-policy tests**

```rust
#[test]
fn conductor_lease_reclaims_at_boundaries_and_for_dead_local_pid() {
    assert_eq!(
        decide_conductor_lease(ConductorLeaseInput::claimed(300, false)),
        ConductorLeaseDecision::Reclaim(ConductorLeaseReclaim::ClaimedExpired),
    );
    assert_eq!(
        decide_conductor_lease(ConductorLeaseInput::running(10_800, false)),
        ConductorLeaseDecision::Reclaim(ConductorLeaseReclaim::Abandoned),
    );
    assert_eq!(
        decide_conductor_lease(ConductorLeaseInput::running(1, true)),
        ConductorLeaseDecision::Reclaim(ConductorLeaseReclaim::DeadSameHostPid),
    );
}

#[test]
fn capacity_checks_usage_before_issue_and_zero_disables_a_cap() {
    assert_eq!(decide_capacity(CapacityInput::new(10, 10, 4, 4)), CapacityDecision::UsageCap);
    assert_eq!(decide_capacity(CapacityInput::new(10, 0, 4, 4)), CapacityDecision::IssueCap);
}
```

- [ ] **Step 2: Run the test and observe RED**

Run: `cargo test -p autospec-core --test autonomous_lifecycle conductor_lease_reclaims_at_boundaries_and_for_dead_local_pid`

Expected: FAIL because the local resilience types do not exist.

- [ ] **Step 3: Implement the minimal pure evaluator**

```rust
pub fn decide_conductor_lease(input: ConductorLeaseInput) -> ConductorLeaseDecision {
    if input.same_host_pid_dead {
        return ConductorLeaseDecision::Reclaim(ConductorLeaseReclaim::DeadSameHostPid);
    }
    let Some(age) = input.heartbeat_age_secs else {
        return ConductorLeaseDecision::Reclaim(ConductorLeaseReclaim::MissingHeartbeat);
    };
    if age >= ABANDONED_LEASE_SECS {
        return ConductorLeaseDecision::Reclaim(ConductorLeaseReclaim::Abandoned);
    }
    if input.claimed && age >= STALE_LEASE_SECS {
        return ConductorLeaseDecision::Reclaim(ConductorLeaseReclaim::ClaimedExpired);
    }
    ConductorLeaseDecision::Held
}
```

Add `decide_capacity` beside it: usage cap first, then issue cap, otherwise within cap.

- [ ] **Step 4: Verify GREEN and commit**

Run: `cargo test -p autospec-core --test autonomous_lifecycle`

Expected: PASS.

```bash
git add crates/autospec-core/src/autonomous_lifecycle.rs crates/autospec-core/tests/autonomous_lifecycle.rs
git commit -m "feat: model typed autonomous resilience policy"
```

### Task 2: Build the canonical-write compatibility adapter

**Files:**
- Create: `crates/autospec-cli/src/commands/autonomous/resilience.rs`
- Create: `crates/autospec-cli/src/commands/autonomous/resilience/records.rs`
- Modify: `crates/autospec-cli/src/commands/autonomous.rs`
- Modify: `crates/autospec-core/src/state/mod.rs`
- Modify: `crates/autospec-core/src/state/json.rs`
- Create: `crates/autospec-cli/tests/autonomous_resilience_commands.rs`

**Interfaces:**
- `ResilienceStore::from_env(repo)`, `read_state`, `read_failures`, `read_spend`, `write_state`, and `admit` are private to the `autonomous` command tree.
- `resilience::run(args)` emits stable JSON and returns the lifecycle-compatible exit code.

- [ ] **Step 1: Write failing black-box layout tests**

```rust
#[test]
fn resilience_decide_prefers_canonical_layout_and_writes_only_double_underscore() {
    let fixture = ResilienceFixture::new();
    fixture.write_state("owner__repo", valid_state("owner/repo", "claimed", 100));
    fixture.write_state("owner_repo", valid_state("owner/repo", "running", 1));
    let output = fixture.run(["resilience", "decide", "--repo", "owner/repo"]);
    assert_eq!(output.status.code(), Some(20));
    assert_eq!(stdout(&output), "{\\\"decision\\\":\\\"held\\\"}\\n");
}

#[test]
fn resilience_decide_rejects_malformed_and_foreign_state_without_a_write() {
    let fixture = ResilienceFixture::new();
    fixture.write_state("owner__repo", "{not-json}");
    let output = fixture.run(["resilience", "decide", "--repo", "owner/repo"]);
    assert_eq!(output.status.code(), Some(3));
    assert!(!fixture.canonical_state_path().exists());
}
```

The fixture sets `AUTOSPEC_STATE_DIR` and a dedicated spend root, creates no real
home-directory state, and runs the compiled `autospec` executable.

- [ ] **Step 2: Run the target and observe RED**

Run: `cargo test -p autospec-cli --test autonomous_resilience_commands`

Expected: FAIL because `autonomous resilience` is not routed.

- [ ] **Step 3: Add the nested adapter and route it**

```rust
// autonomous.rs
mod resilience;
if args.first().is_some_and(|arg| arg == "resilience") {
    return resilience::run(&args[1..]);
}
```

```rust
// autonomous/resilience.rs
pub(super) struct ResilienceStore { /* roots plus RepositoryScope */ }
impl ResilienceStore {
    fn state_candidates(&self) -> [PathBuf; 3] { /* __, _, - */ }
    fn read_state(&self) -> Result<Option<ResilienceState>, ResilienceReject> { /* scope check */ }
    fn write_state(&self, value: &ResilienceState) -> Result<(), String> { /* super::atomic_write */ }
}
```

Decode only documented resilience, failure, and spend fields through the
existing strict core parser. Keep record decoding in `records.rs`, reuse
`super::atomic_write`, and never call a shell script. Treat null lock PIDs as
released, reject zero lock PIDs, never compare empty or `unknown` hosts as local,
validate every companion record before fallback migration, and accept the
documented decimal-string failure issue identifier.

- [ ] **Step 4: Verify GREEN and commit**

Run: `cargo test -p autospec-cli --test autonomous_resilience_commands`

Expected: PASS for canonical first, underscore/hyphen fallback, malformed,
foreign, failure-cap, capacity precedence, released locks, host safety, strict
failure/spend records, and no-write migration rejection cases.

```bash
git add crates/autospec-cli/src/commands/autonomous.rs crates/autospec-cli/src/commands/autonomous/resilience.rs crates/autospec-cli/tests/autonomous_resilience_commands.rs
git commit -m "feat: add Rust autonomous resilience adapter"
```

### Task 3: Admit lifecycle operations through the same state adapter

**Files:**
- Modify: `crates/autospec-cli/src/commands/autonomous.rs`
- Modify: `crates/autospec-cli/src/commands/autonomous/resilience.rs`
- Modify: `crates/autospec-cli/tests/autonomous_resilience_commands.rs`

**Interfaces:**
- `ResilienceAdmission` supplies `failure_count`, a typed capacity result, and a conductor-lease result.
- `start`, `restart`, and `run-foreground` map it into `LifecycleInput` before filesystem or process mutation; `status` reads it without writing.

- [ ] **Step 1: Write failing admission-order regressions**

```rust
#[test]
fn start_rejects_foreign_resilience_before_operator_files_exist() {
    let fixture = ResilienceFixture::new();
    fixture.write_state("owner__repo", valid_state("other/repo", "running", 1));
    let output = fixture.run_start();
    assert_eq!(output.status.code(), Some(3));
    assert!(!fixture.operator_lifecycle_path().exists());
}

#[test]
fn foreground_parks_usage_before_issue_cap_and_dispatch() {
    let fixture = ResilienceFixture::new();
    fixture.write_spend("owner__repo", spend(10, 10, 5, 5));
    let output = fixture.run_foreground();
    assert_eq!(output.status.code(), Some(20));
    assert!(!fixture.foreground_state_path().exists());
}
```

- [ ] **Step 2: Run the two tests and observe RED**

Run: `cargo test -p autospec-cli --test autonomous_resilience_commands`

Expected: FAIL because start and foreground currently use default lifecycle state.

- [ ] **Step 3: Integrate one typed admission path**

```rust
let admission = resilience::ResilienceStore::from_options(&options, &layout)?.admit()?;
let input = LifecycleInput::from_scope(scope)
    .with_transition(LifecycleTransition::Foreground)
    .with_failure_count(admission.failure_count)
    .with_budget(admission.lifecycle_budget);
let lifecycle = decide_lifecycle(&input);
```

Apply this before directory creation, stop removal, process termination, or
foreground-state mutation. Extract `cycle-N` from a legacy `running:cycle-N`
status only when no separate cycle field exists.

- [ ] **Step 4: Verify GREEN and commit**

Run: `cargo test -p autospec-cli --test autonomous_resilience_commands && cargo test -p autospec-cli --test autonomous_lifecycle_commands`

Expected: PASS and no foreign/malformed record triggers a write or dispatch.

```bash
git add crates/autospec-cli/src/commands/autonomous.rs crates/autospec-cli/src/commands/autonomous/resilience.rs crates/autospec-cli/tests/autonomous_resilience_commands.rs
git commit -m "feat: admit autonomous operations through resilience state"
```

### Task 4: Document the contract and prove the cutover

**Files:**
- Modify: `docs/specs/2026-07-15-rust-autonomous-lifecycle.md`
- Modify: `docs/cli-reference.md`
- Modify: `crates/autospec-cli/tests/autonomous_resilience_commands.rs`

- [ ] **Step 1: Write a failing help/reference assertion**

```rust
#[test]
fn resilience_help_names_the_canonical_write_slug() {
    let output = Command::new(env!("CARGO_BIN_EXE_autospec"))
        .args(["autonomous", "resilience", "--help"])
        .output()
        .expect("run help");
    assert!(String::from_utf8_lossy(&output.stdout).contains("owner__repo"));
}
```

- [ ] **Step 2: Run and observe RED**

Run: `cargo test -p autospec-cli --test autonomous_resilience_commands resilience_help_names_the_canonical_write_slug`

Expected: FAIL until command help is implemented.

- [ ] **Step 3: Document exact behavior**

Document the `__ -> _ -> -` read order, canonical writes, lifecycle-only
operator directory, fail-closed records, inclusive lease limits, and the absence
of shell resilience authority.

- [ ] **Step 4: Run final proof**

Run:

```bash
cargo fmt --all --check
cargo clippy --workspace -- -D warnings
cargo test --workspace --quiet
cargo run -q -p autospec-cli -- validate --fast
git diff --check
```

Expected: every command exits 0.

- [ ] **Step 5: Commit docs and final tests**

```bash
git add docs/specs/2026-07-15-rust-autonomous-lifecycle.md docs/cli-reference.md crates/autospec-cli/tests/autonomous_resilience_commands.rs
git commit -m "docs: describe Rust autonomous resilience state"
```

## Plan Self-Review

- Spec coverage: Tasks 1-3 cover exact lease rules, canonical paths, failure and capacity behavior, status, start, restart, and foreground admission. Task 4 covers public documentation and full proof.
- Placeholder scan: Every task specifies files, interfaces, a test-first command, expected outcome, and a concrete implementation boundary.
- Type consistency: The adapter returns `ResilienceAdmission`; only `autonomous.rs` maps it into existing lifecycle types, so filesystem state never leaks into core policy.
