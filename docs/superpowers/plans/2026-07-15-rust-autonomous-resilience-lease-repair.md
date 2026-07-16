# Rust Autonomous Resilience Lease Repair Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Atomically reserve one Rust conductor before launch while preserving fail-closed compatibility and diagnostic I/O behavior.

**Architecture:** The CLI adapter owns a short Unix transaction lock and a token-fenced canonical resilience record. Existing core lease and capacity evaluators remain pure policy. Commands acquire or adopt a token before side effects; invalid record contents are JSON rejects and filesystem failures are diagnostics.

**Tech Stack:** Rust standard library, Unix `flock` FFI, existing JSON parser and atomic writer, Rust integration tests.

## Global Constraints

- Do not add a dependency, shell command, shell fallback, force-takeover flag, or remote lock.
- Read compatible records as `owner__repo`, then `owner_repo`, then `owner-repo`; never skip an invalid first existing record.
- The transaction covers read, validation, core-policy decision, and canonical write. `atomic_write` is durability, never a compare-and-swap.
- Rust-owned records carry opaque `lease_token` and monotonic `lease_generation`; legacy token-less records remain valid lease evidence.
- Child adoption occurs before lifecycle, health, queue, claim, executor, or foreground-state mutation. Mismatch is non-executable.
- Content corruption/scope errors are JSON reject + exit 3; I/O and invalid supplied options are diagnostic + exit 2 with no decision JSON.
- Omitted caps retain default/environment behavior; zero remains valid; explicitly empty caps reject.
- Restart owns a fresh lease before termination or stop clearing. A fresh existing lease parks.

## File Structure

| File | Responsibility |
| --- | --- |
| `crates/autospec-cli/src/commands/autonomous/resilience/records.rs` | Strict token/generation and status identity parsing. |
| `crates/autospec-cli/src/commands/autonomous/resilience.rs` | Transaction lock, acquisition/adoption/release, error boundary. |
| `crates/autospec-cli/src/commands/autonomous.rs` | Typed budget parsing and lifecycle token wiring. |
| `crates/autospec-cli/tests/autonomous_resilience_commands.rs` | Black-box diagnostics, inputs, contention, and adoption regressions. |
| `docs/specs/2026-07-15-rust-autonomous-lifecycle.md` | Atomic lease/error contract. |
| `docs/cli-reference.md` | User-facing decision/exit contract. |

### Task 1: Preserve diagnostic/error and input distinctions

**Files:**
- Modify: `crates/autospec-cli/src/commands/autonomous/resilience.rs`
- Modify: `crates/autospec-cli/src/commands/autonomous/resilience/records.rs`
- Modify: `crates/autospec-cli/src/commands/autonomous.rs`
- Modify: `crates/autospec-cli/tests/autonomous_resilience_commands.rs`

**Interfaces:**
- Private `StoreError::Reject(ResilienceReject) | Diagnostic(String)` separates read content from I/O.
- `StatusState.repo: String` is required.
- `Options::{budget_tokens,budget_issues}: Option<u64>` preserves option presence.

- [ ] **Step 1: Add failing black-box tests**

```rust
#[test]
fn state_read_io_is_diagnostic_not_malformed_reject() {
    let fixture = ResilienceFixture::new();
    fs::create_dir_all(fixture.canonical_state_path()).unwrap();
    let output = fixture.run(&["resilience", "decide", "--repo", "owner/repo"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(stdout(&output).is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("resilience state"));
}

#[test]
fn status_requires_matching_record_repo() { /* missing and foreign repo exit 3, no writes */ }

#[test]
fn empty_supplied_lifetime_budget_is_diagnostic() { /* both flags exit 2 before state */ }
```

- [ ] **Step 2: Confirm RED**

Run: `cargo test -p autospec-cli --test autonomous_resilience_commands -- --nocapture`

Expected: a state directory is currently reported as `malformed_state`, an unscoped status succeeds, and an empty cap uses defaults.

- [ ] **Step 3: Implement exact boundary**

```rust
enum StoreError { Reject(ResilienceReject), Diagnostic(String) }

match fs::read_to_string(&path) {
    Ok(raw) => ResilienceState::parse(&raw)
        .map_err(|_| StoreError::Reject(ResilienceReject::MalformedState)),
    Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
    Err(error) => return Err(StoreError::Diagnostic(
        format!("cannot read resilience state {}: {error}", path.display())
    )),
}
```

Require status `repo`, compare it unconditionally, map only `Reject` to stable JSON, and parse supplied budgets as `u64` when `Options` is parsed. Only `None` reads the environment/default.

- [ ] **Step 4: Verify and commit**

Run: `cargo test -p autospec-cli --test autonomous_resilience_commands && cargo test -p autospec-cli --test autonomous_lifecycle_commands`

Expected: all targeted regressions and compatibility cases pass. Commit a Conventional + Lore message for the error-classification contract.

### Task 2: Add token-fenced atomic lease acquisition

**Files:**
- Modify: `crates/autospec-cli/src/commands/autonomous/resilience.rs`
- Modify: `crates/autospec-cli/src/commands/autonomous/resilience/records.rs`
- Modify: `crates/autospec-cli/tests/autonomous_resilience_commands.rs`

**Interfaces:**
- Private `ConductorLease { token: String, generation: u64 }`.
- `ResilienceStore::acquire(issue, usage_cap, issue_cap) -> Result<(ResilienceAdmission, ConductorLease), StoreError>`.
- `adopt(token)` and `release(&lease)` change state only when token matches.

- [ ] **Step 1: Add failing ownership tests**

```rust
#[test]
fn competing_conductors_hold_one_owner_before_operator_write() { /* exactly one owner */ }

#[test]
fn stale_token_cannot_adopt_or_release_reclaimed_lease() { /* replacement unchanged */ }
```

- [ ] **Step 2: Confirm RED**

Run: `cargo test -p autospec-cli --test autonomous_resilience_commands -- --nocapture`

Expected: contenders both obtain read-only admission; no token fences a delayed owner.

- [ ] **Step 3: Implement one transaction linearization point**

```rust
struct LeaseTransaction { file: fs::File }

impl LeaseTransaction {
    fn try_open(path: &Path) -> Result<Self, StoreError> {
        // create/open conductor.lease.lock and take non-blocking Unix LOCK_EX.
        // Contention returns the existing held outcome; unsupported platforms diagnose.
    }
}

fn acquire(&self, issue: Option<u64>, usage: u64, issues: u64)
    -> Result<(ResilienceAdmission, ConductorLease), StoreError> {
    let _lock = LeaseTransaction::try_open(&self.lock_path())?;
    // read -> validate -> core decision -> atomically write claimed canonical state.
}
```

Use documented `unsafe` Unix FFI only behind `cfg(unix)`. Generate the opaque token from PID, timestamp nanoseconds, and a process atomic sequence. Keep existing core policy as the only lease/cap evaluator; migrate fallback state only inside this transaction.

- [ ] **Step 4: Verify and commit**

Run: `cargo fmt --all --check && cargo test -p autospec-cli --test autonomous_resilience_commands && cargo clippy -p autospec-cli -- -D warnings`

Expected: only one contender owns a fresh lease and stale token operations are fenced. Commit a Conventional + Lore atomic-lease message.

### Task 3: Wire start/restart/foreground ownership and public contract

**Files:**
- Modify: `crates/autospec-cli/src/commands/autonomous.rs`
- Modify: `crates/autospec-cli/src/commands/autonomous/resilience.rs`
- Modify: `crates/autospec-cli/tests/autonomous_resilience_commands.rs`
- Modify: `docs/specs/2026-07-15-rust-autonomous-lifecycle.md`
- Modify: `docs/cli-reference.md`

**Interfaces:**
- Start/restart acquire after stop/repo validation and before process/local mutation.
- `spawn_unit` receives token through `Command::env`, never persisted argv.
- Foreground adopts `AUTOSPEC_CONDUCTOR_LEASE_TOKEN` or acquires before health/queue/state work and releases only a matching token after terminal persistence.

- [ ] **Step 1: Add failing command-boundary tests**

```rust
#[test]
fn fresh_lease_blocks_restart_before_kill_or_stop_clear() { /* exit 20; dummy PID survives */ }

#[test]
fn delayed_child_with_replaced_token_exits_before_foreground_mutation() { /* no fake GitHub call */ }
```

- [ ] **Step 2: Confirm RED**

Run: `cargo test -p autospec-cli --test autonomous_resilience_commands -- --nocapture`

Expected: current command flow does not pass/adopt a token and restart can terminate before it owns a lease.

- [ ] **Step 3: Implement lifecycle ownership**

```rust
let lease = resilience::acquire_lifecycle(&layout.repo, options.issue, options.budgets)?;
let conductor = spawn_unit(..., Some(("AUTOSPEC_CONDUCTOR_LEASE_TOKEN", lease.token())))?;
// On launch failure: terminate any child, then release only this matching lease.
```

At foreground entry, keep stored-stop precedence. Adopt an environment token when present; otherwise acquire. A mismatch exits before lifecycle, health, queue, claim, executor, or foreground writes. Document local lock scope, token fencing, diagnostics/rejects, and no shell authority.

- [ ] **Step 4: Final proof and commit**

Run: `cargo fmt --all --check; cargo clippy --workspace -- -D warnings; cargo test --workspace --quiet; cargo run -q -p autospec-cli -- validate --fast; git diff --check`

Expected: every command exits 0. Commit a Conventional + Lore lifecycle-ownership message.

## Plan Self-Review

- Coverage: Task 1 handles every error/input review finding; Task 2 adds serialized ownership and fencing; Task 3 wires all mutating commands and public contract.
- Placeholder scan: every task has paths, interfaces, failing test shape, exact command, expected result, and implementation boundary.
- Type consistency: `StoreError` and lease tokens remain CLI-private; core remains the sole pure policy evaluator.
