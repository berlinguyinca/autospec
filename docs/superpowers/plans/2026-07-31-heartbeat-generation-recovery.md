# Heartbeat Generation Recovery Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prevent and recover the autonomous claim deadlock caused when a released generation's startup heartbeat blocks its successor.

**Architecture:** Ship two ordered, independently reviewable fixes. First, make the existing exact generic-release recovery retire its matching heartbeat before its acquisition receipt. Second, allow stale-startup recovery to hand off expired, dead, self-consistent prior-generation heartbeat evidence before releasing a stranded `heartbeat-pending` successor.

**Tech Stack:** Rust 2021, Git-backed claim refs, private Unix heartbeat files, existing crash-safe heartbeat handoff transactions.

## Global Constraints

- Keep each implementation PR at or below 400 changed lines, 8 files, and 3 logical units.
- Do not weaken repository, issue, worker, branch, claim ID, PR, nonce, ownership, mode, symlink, or process-liveness checks.
- Use the existing heartbeat handoff transaction; never unlink heartbeat evidence directly.
- Preserve fresh, live, malformed, symlinked, and cross-repository evidence without mutation.
- Run the full serial Rust workspace suite and canonical 142-check validator before each merge.

---

### Task 1: Retire the Exact Released Generation Heartbeat

**Files:**
- Modify: `crates/autospec-cli/src/commands/claim.rs`
- Modify: `crates/autospec-cli/src/commands/autonomous/executor_bridge.rs`
- Test: `crates/autospec-cli/tests/autonomous_conductor_commands.rs`

**Interfaces:**
- Consumes: `ClaimMutationIdentity`, `read_claim_ref`, and `retire_released_startup_heartbeat`.
- Produces: `recover_released_bridge_claim(identity: ClaimMutationIdentity<'_>) -> Result<bool, CommandFailure>`.

- [ ] **Step 1: Extend the exact foreground regression**

Seed the startup heartbeat for the retained claim inside `run_missing_cleanup_recovery`. The positive case must assert that the heartbeat is absent from the live issue path and retained by the existing handoff transaction. Every worker, claim ID, branch, and PR mismatch must assert that both the heartbeat and acquisition receipt remain.

- [ ] **Step 2: Run the focused regression and verify RED**

Run:

```bash
cargo test -p autospec-cli --test autonomous_conductor_commands foreground_recovers_exact_immediate_stop_release_without_cleanup_intent -- --exact
```

Expected: FAIL because the exact generic-release path returns success while the old live heartbeat path still exists.

- [ ] **Step 3: Add the exact mutating recovery helper**

Replace the read-only generic-release observer with a helper shaped as follows:

```rust
pub(crate) fn recover_released_bridge_claim(
    identity: ClaimMutationIdentity<'_>,
) -> Result<bool, CommandFailure> {
    let Some(selected) = read_claim_ref(identity.repo, identity.issue)? else {
        return Ok(false);
    };
    let exact = selected.record.worker_id == identity.worker_id
        && selected.record.claim_id.as_deref() == Some(identity.claim_id)
        && selected.record.branch == identity.branch
        && selected.record.state == "released"
        && selected.record.step == "released"
        && selected.record.pr.is_empty();
    if exact {
        retire_released_startup_heartbeat(identity)?;
    }
    Ok(exact)
}
```

Use this helper only in `recover_terminal_failure_identity` when the failure-cleanup intent is absent. Leave `observe_terminal_bridge_claim(...Retryable)` unchanged because the retryable transition already retires its heartbeat.

- [ ] **Step 4: Run the positive and mismatch regressions**

Run:

```bash
cargo test -p autospec-cli --test autonomous_conductor_commands foreground_recovers_exact_immediate_stop_release_without_cleanup_intent -- --exact
cargo test -p autospec-cli --test autonomous_conductor_commands foreground_missing_failure_intent_requires_exact_released_identity -- --exact
```

Expected: PASS; the positive archives the heartbeat, and every mismatch preserves it.

- [ ] **Step 5: Commit Task 1**

Commit only the three files above with a conventional Lore commit that records the crash boundary, the exact identity constraint, focused tests, and the remaining full-suite gate.

---

### Task 2: Reclaim a Stranded Heartbeat-Pending Successor

**Files:**
- Modify: `crates/autospec-cli/src/commands/claim.rs`
- Test: `crates/autospec-cli/tests/claim_commands.rs`

**Interfaces:**
- Consumes: `recover_authoritative_stale_startup`, `classify_startup_heartbeat_snapshot`, and `handoff_retained_heartbeat`.
- Produces: a fail-closed classification of an expired dead prior-generation heartbeat occupying the current issue path.

- [ ] **Step 1: Add the stranded-successor regression**

Extend `claim_stale_heartbeat_recovery` with a claim ref whose state is `claimed`, step is `heartbeat-pending:none`, timestamp exceeds the supplied recovery timeout, and identity is `worker-new/claim-new`. Put a valid expired dead heartbeat at the issue path for `worker-old/claim-old` in the same repository, issue, and branch.

Assert:

```rust
assert!(String::from_utf8_lossy(&output.stdout).contains("\"recovered\":true"));
assert!(!heartbeat.exists());
assert!(claim_ref_message(&repo, issue).contains("\"state\":\"available\""));
assert!(claim_ref_message(&repo, issue).contains("\"step\":\"stale_startup_recovered\""));
```

Add negative cases proving that a fresh prior-generation heartbeat and an expired heartbeat for another repository or issue remain blocking and leave the claim ref unchanged.

- [ ] **Step 2: Run the focused regression and verify RED**

Run:

```bash
cargo test -p autospec-cli --test claim_commands claim_stale_heartbeat_recovery -- --exact
```

Expected: FAIL because `recover_authoritative_stale_startup` returns before inspecting any heartbeat lifecycle step.

- [ ] **Step 3: Admit only stale lifecycle records**

Remove the unconditional `heartbeat_lifecycle_step` rejection from the early guard. Keep the existing requirements that the record is `claimed`, has no PR or branch ref, and is older than the explicit recovery timeout.

When the issue heartbeat does not match the current claim identity, parse it and build an expectation from its own immutable fields. Admit it for handoff only when all of these checks hold:

```rust
evidence.repo == repo_name
    && evidence.issue == issue.to_string()
    && evidence.branch == record.branch
    && evidence.pr.is_empty()
```

Then reuse `classify_startup_heartbeat_snapshot` so the existing nonce, TTL, mode, ownership, process-start, and dead-process checks remain authoritative. Any parse, identity, freshness, liveness, symlink, or revalidation failure returns the existing non-mutating blocked outcome.

- [ ] **Step 4: Run focused and adjacent recovery tests**

Run:

```bash
cargo test -p autospec-cli --test claim_commands claim_stale_heartbeat_recovery -- --exact
cargo test -p autospec-cli --test claim_commands stale_startup_recovery_advances_ref_to_available_before_requeue -- --exact
cargo test -p autospec-cli --test claim_commands claim_state_recover_stale_startup_preserves_a_fresh_claim_without_label_mutation -- --exact
```

Expected: PASS with no direct heartbeat deletion and no mutation of protected negative cases.

- [ ] **Step 5: Commit Task 2**

Commit only `claim.rs` and `claim_commands.rs` with a conventional Lore commit that records the five-minute recovery timeout and fail-closed evidence boundaries.

---

### Task 3: Verify, Merge, Install, and Replay

**Files:**
- Verify only: Task 1 and Task 2 implementation branches.
- Runtime evidence: `~/.autospec/autonomous-operator/berlinguyinca_autospec/`

**Interfaces:**
- Consumes: both merged fixes.
- Produces: a running conductor that retires the stranded #2748 generation, publishes a fresh heartbeat, and starts one sustained executor.

- [ ] **Step 1: Gate each exact implementation head**

Run on each branch:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace -- --test-threads=1
target/debug/autospec validate --json --jobs 1
```

Require the implementation linter, independent `LGTM`, and all non-skipped hosted CI checks before merging.

- [ ] **Step 2: Install merged main**

Run:

```bash
./install.sh --skill autospec --harness all --update
cmp -s target/release/autospec ~/.autospec/bin/autospec
```

Expected: installer succeeds for Claude, OpenCode, and Codex; binary comparison exits 0.

- [ ] **Step 3: Replay the retained live state**

Run:

```bash
AUTOSPEC_HANDOFF_DISPATCHER_KIND=claude ~/.autospec/bin/autospec autonomous restart --force --repo berlinguyinca/autospec --repo-dir "$PWD" --poll-interval-sec 10 --json
```

Observe that #2748 transitions through `available/stale_startup_recovered`, gets a new claim ID, writes a new `2748.json`, and launches one Claude executor that remains live for at least 30 seconds. Preserve unrelated process IDs and keep the healthy conductor running.
