# Exact Immediate-Stop Release Recovery Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Retire an interrupted executor invocation without cleanup intent only when its authoritative claim is the exact immediate-stop `released/released` generation.

**Architecture:** Add one read-only claim observer for the generic released terminal shape and use it only in the missing-cleanup recovery branch. Preserve the existing bridge retry observer for `released/retryable_released` and every present-cleanup path.

**Tech Stack:** Rust, Git-backed claim refs, real-bridge CLI integration tests

## Global Constraints

- Do not mutate or rewrite historical claim refs during recovery.
- Match repository, issue, worker ID, claim ID, branch, empty PR, state, and step.
- Preserve fail-closed handling for unavailable, malformed, foreign, failed, merged, or needs-human evidence.
- Add no dependencies and keep the PR below 400 changed lines, 8 files, and 3 logical units.

---

### Task 1: Exact generic-release recovery boundary

**Files:**
- Modify: `crates/autospec-cli/src/commands/claim.rs`
- Modify: `crates/autospec-cli/src/commands/autonomous/executor_bridge.rs`
- Test: `crates/autospec-cli/tests/autonomous_conductor_commands.rs`

**Interfaces:**
- Consumes: `ClaimMutationIdentity<'_>` and the authoritative `RunStateRecord` returned by `read_claim_ref`.
- Produces: `observe_released_bridge_claim(identity) -> Result<bool, CommandFailure>`.

- [ ] **Step 1: Write the failing positive integration**

Seed an interrupted invocation without cleanup intent, its matching acquisition
receipt, and an authoritative claim record with the exact identity plus
`state="released"`, `step="released"`, and `pr=""`. Run one foreground cycle
and assert success plus acquisition-receipt retirement.

Run:

```bash
cargo test -p autospec-cli foreground_recovers_exact_immediate_stop_release_without_cleanup_intent
```

Expected: FAIL with `missing failure cleanup intent requires an exact retryable release`.

- [ ] **Step 2: Write the failing identity-table integration**

For worker ID, claim ID, branch, and PR, seed one mismatched authoritative
record while keeping the local acquisition and invocation unchanged. Assert
each foreground run fails and retains the acquisition receipt.

Run:

```bash
cargo test -p autospec-cli foreground_missing_failure_intent_requires_exact_released_identity
```

Expected: FAIL until the exact generic-release observer exists; after the
observer is added, every mismatch must remain rejected.

- [ ] **Step 3: Add the read-only observer**

Add this interface in `claim.rs`:

```rust
pub(crate) fn observe_released_bridge_claim(
    identity: ClaimMutationIdentity<'_>,
) -> Result<bool, CommandFailure>
```

It reads the authoritative claim once and returns true only for matching
repository issue lookup, worker ID, claim ID, branch, empty PR,
`state == "released"`, and `step == "released"`.

- [ ] **Step 4: Admit the exact shape only for absent cleanup intent**

In `recover_terminal_failure_identity`, when `failure-cleanup.json` is absent,
accept either `observe_released_bridge_claim(...)` or the existing exact
`BridgeClaimDisposition::Retryable` observation. Do not change the
present-cleanup branch.

- [ ] **Step 5: Run focused GREEN verification**

```bash
cargo test -p autospec-cli foreground_recovers_exact_immediate_stop_release_without_cleanup_intent
cargo test -p autospec-cli foreground_missing_failure_intent_requires_exact_released_identity
cargo test -p autospec-cli foreground_missing_failure_intent_requires_exact_retryable_release
cargo test -p autospec-cli foreground_recovers_released_executor_receipt_failure_and_other_claim_crash_windows
```

Expected: all four commands pass.

- [ ] **Step 6: Commit the implementation**

```bash
git add crates/autospec-cli/src/commands/claim.rs crates/autospec-cli/src/commands/autonomous/executor_bridge.rs crates/autospec-cli/tests/autonomous_conductor_commands.rs
git commit -m "fix: recover exact immediate-stop claim releases"
```

### Task 2: Full verification and live replay

**Files:**
- Verify: all files in Task 1 plus the design and plan documents.

**Interfaces:**
- Consumes: the exact-head branch and retained live #2748 state.
- Produces: merge evidence and a live conductor that retires the old acquisition before selecting fresh work.

- [ ] **Step 1: Run static and full local gates**

```bash
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace -- --test-threads=1
target/debug/autospec validate --json --jobs 1
```

Expected: zero clippy warnings, zero runnable test failures, and 142/142
validator checks.

- [ ] **Step 2: Review, publish, and merge**

Run exact-head implementation lint, obtain an independent `LGTM`, push the
branch, open the issue-closing PR, wait for all non-advisory hosted checks, and
admin-squash merge.

- [ ] **Step 3: Install and replay retained state**

Build/install the merged release, restart the repository-scoped conductor with
the Claude dispatcher, and verify the old #2748 acquisition is retired, a fresh
claim is acquired, and the executor remains live beyond one supervisor cycle.
