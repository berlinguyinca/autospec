# Merged Claim Recovery Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Retire an exact claimed generation whose linked pull request is already merged before missing-worktree recovery.

**Architecture:** Add bridge-owned terminal reconciliation immediately after durable invocation loading. Bind the exact authoritative claim and PR to the local issue branch, prove the persisted head is contained in the merged head, preserve the A-to-B evidence boundary, then resume the existing merged finalizer without relaxing normal recovery.

**Tech Stack:** Rust, Git refs, GitHub CLI fixture, Cargo integration tests.

## Global Constraints

- Do not reconstruct the missing executor worktree.
- Do not terminalize from issue closure alone.
- Preserve compare-and-swap ownership semantics.
- Treat ancestry only as post-merge inclusion proof, never merge-admission proof.
- Remove only the exact prunable worktree registration after durable intent.
- Add no dependencies.

---

### Task 1: Reconcile the exact merged claim before local recovery

**Files:**
- Modify: `crates/autospec-cli/src/commands/autonomous/executor_bridge.rs`
- Test: `crates/autospec-cli/tests/autonomous_conductor_commands.rs`

**Interfaces:**
- Consumes: `PersistedInvocation`, `ExecutorBridgeRequest`, `DraftPrAdapter`, and `transition_bridge_claim`.
- Produces: terminal `Merged` state plus an idempotent externally-advanced reconciliation record.

- [ ] **Step 1: Write the failing integration test**

Add `foreground_retires_exact_merged_draft_when_worktree_is_missing`. Reuse the
real bridge fixture to persist `draft_created` at A, add reviewer commits through
B, mark the exact PR merged at B, remove the exact worktree, and seed
`executor_receipt_failed`. Assert the command succeeds, the claim is `merged`,
the A-to-B record and terminal receipt are exact, the exact prunable registration
is absent, an unrelated prunable registration remains, the conductor returns to
`Scan`, and the harness launch count stays `1`.

- [ ] **Step 2: Run the focused test and verify the invariant failure**

Run:

```bash
cargo test -p autospec-cli foreground_retires_exact_merged_draft_when_worktree_is_missing -- --nocapture
```

Expected: FAIL because current recovery reports `recovery worktree is missing before cleanup`.

- [ ] **Step 3: Add exact merged-PR terminal reconciliation**

In `executor_bridge.rs`, after loading the invocation, require exact request and
claim identity and invoke:

```text
gh pr view <PR> --repo <OWNER/REPO> --json number,state,isDraft,headRefName,headRefOid,baseRefName,mergeCommit
```

Return normal recovery unless the PR is merged. For a merged observation,
fail closed on mismatched identity, malformed OIDs, a local branch not equal to
the PR head, or a persisted head that is not its ancestor. Write the bound
A-to-B reconciliation record, persist `Merged` with the actual PR head and merge
OID, and call the existing merged finalizer.

- [ ] **Step 4: Make terminal cleanup exact and resumable**

After the durable worktree removal intent exists, allow a missing exact
path/branch registration only when Git marks it prunable. Remove that single
registration with `git worktree remove --force`; reject identity ambiguity and
preserve unrelated registrations.

- [ ] **Step 5: Verify focused and adjacent tests**

Run:

```bash
cargo test -p autospec-cli foreground_retires_exact_merged_draft_when_worktree_is_missing foreground_resumes_nonzero_draft_created_receipt_failure_without_second_harness -- --nocapture
```

Expected: both tests pass and no second harness launch is recorded.

- [ ] **Step 6: Add fail-closed and crash-resume coverage**

Cover non-ancestor and local-ref mismatch, PR identity/state/OID mismatches,
claim ownership loss, changed reconciliation content, and resume after each
durable boundary. Assert old review evidence is never reported as proof for B.

- [ ] **Step 7: Run repository gates and commit**

Run:

```bash
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --quiet -- --test-threads=1
target/debug/autospec validate --json
```

Commit with a Conventional/Lore message referencing issue `#2653` and the
required `Co-authored-by: OmX <omx@oh-my-codex.dev>` trailer.
