# Rust Autonomous Executor Bridge Implementation Plan

**Goal:** Replace the foreground conductor's external result-file dependency
with a recoverable Rust-owned bridge that launches a configured implementation
harness and advances only independently verified PR evidence.

**Architecture:** A dedicated `executor_bridge` module owns harness resolution,
isolated worktree identity, direct child supervision, persisted phases, strict
artifact parsing, and PR proof. `autonomous.rs` remains the conductor and claim
authority. Existing `premerge` and `executor-result` commands remain the only
QA/security decision and result-ingestion boundaries.

**Tech stack:** Existing Rust workspace, Git and GitHub CLI adapters, existing
runtime alias table, existing claim/premerge types, and serial Rust integration
tests. No new dependencies.

## Constraints

- Work only in the issue worktree created from current `origin/main`.
- Write a failing regression before each behavior change.
- Never invoke `autospec-run`, `omx`, a shell conductor, or the primary checkout.
- Never treat process exit, free-form stdout, or a harness claim as proof.
- Keep the fixed executor-result artifact as compatibility input.
- Keep every invocation bound to one repository, issue, worker, branch, claim,
  base commit, worktree, and PR head.

### Task 1: Add typed harness and invocation contracts

**Files:**

- Add `crates/autospec-cli/src/commands/autonomous/executor_bridge.rs`
- Modify `crates/autospec-cli/src/commands/autonomous.rs`

1. Add failing unit tests for runtime-marker precedence, explicit override,
   alias-table parsing, unsafe dispatcher rejection, and exact Codex, Claude,
   and OpenCode argument vectors.
2. Implement `HarnessKind`, `HarnessConfig`, `BridgeIdentity`,
   `BridgePhase`, and strict persisted invocation JSON.
3. Resolve the installed alias table from the existing environment/config
   locations and resolve an absolute non-temporary executable.
4. Commit the typed contract and tests.

### Task 2: Provision and recover the isolated issue worktree

**Files:**

- Modify `crates/autospec-cli/src/commands/autonomous/executor_bridge.rs`
- Modify `crates/autospec-cli/tests/autonomous_conductor_commands.rs`

1. Add a failing integration fixture backed by a real local Git repository and
   bare remote.
2. Resolve the remote default branch, fetch it, and create the exact
   `autonomous/issue-<N>` branch in `/tmp/wt-autonomous-issue-<N>`.
3. Adopt only a matching clean branch/worktree; fail closed on dirty, detached,
   foreign, symlinked, or mismatched reuse.
4. Persist non-terminal state atomically before launch and recover the last
   independently verified phase after restart.
5. Commit worktree and recovery behavior.

### Task 3: Launch and supervise the implementation harness

**Files:**

- Modify `crates/autospec-cli/src/commands/autonomous/executor_bridge.rs`
- Modify `crates/autospec-cli/tests/autonomous_conductor_commands.rs`

1. Add failing tests proving one direct launch, explicit argv, output progress,
   stall termination, process-group cleanup, and no duplicate live child.
2. Build the dedicated implementer prompt from the exact issue, claim, branch,
   worktree, and base identity.
3. Stream bounded child output into structured executor events while refreshing
   progress state.
4. Replace the 30-second absolute timeout with progress-aware stall detection.
5. Make pending and interrupted phases non-terminal.
6. Commit supervision behavior.

### Task 4: Prove the draft PR and verifier evidence

**Files:**

- Modify `crates/autospec-cli/src/commands/autonomous/executor_bridge.rs`
- Modify `crates/autospec-cli/src/commands/autonomous/premerge.rs`
- Modify `crates/autospec-cli/tests/autonomous_conductor_commands.rs`

1. Add failing tests for unchanged HEAD, dirty state, foreign branch, missing or
   multiple PRs, wrong head OID, ready-before-verification, missing issue close,
   and malformed Closeout report.
2. Re-read Git and GitHub state after the implementer exits and accept exactly
   one matching draft PR.
3. Launch QA and security verifier prompts with strict JSON final artifacts.
4. Convert parsed verifier artifacts into the existing typed evidence, evaluate
   the immutable premerge decision, and require Pass.
5. Mark the draft ready and submit the existing strict executor result.
6. Commit the verified success path.

### Task 5: Wire the conductor and remove terminal pending replay

**Files:**

- Modify `crates/autospec-cli/src/commands/autonomous.rs`
- Modify `crates/autospec-cli/tests/autonomous_conductor_commands.rs`

1. Add a failing foreground regression proving a selected issue reaches the
   bridge rather than the fixed pending child.
2. Call the bridge from `ExecutorRequest`, preserve compatibility artifact
   ingestion, parse exact JSON, and persist terminal receipts only for terminal
   outcomes.
3. Avoid the duplicate blocked `record_executor_outcome` call after accepted
   success and keep claim reconciliation authoritative.
4. Commit conductor integration.

### Task 6: Document, review, merge, reinstall, and dogfood

**Files:**

- Modify `docs/cli-reference.md`
- Modify `docs/workflows.md`
- Modify `docs/superpowers/specs/2026-07-25-rust-autonomous-executor-bridge-design.md`
- Modify `docs/superpowers/plans/2026-07-25-rust-autonomous-executor-bridge.md`

1. Document harness selection, worktree isolation, recovery, progress, stop,
   evidence, and compatibility behavior.
2. Run targeted formatting, focused serial tests, full serial workspace tests,
   Clippy with warnings denied, fast validation, implementation lint, and diff
   checks.
3. Run an independent review and repair every blocking finding.
4. Open the issue-linked PR, wait for required CI, admin-squash-merge, fetch
   exact merged main, build from a clean detached worktree, and install it.
5. Restart autospec-gui autonomy with follow enabled and prove issue #36 creates
   and advances a PR, then observe #34 and #35 without touching its existing
   `.gitignore` change.

## Verification commands

Run:

```bash
cargo test -p autospec-cli autonomous_executor_bridge -- --test-threads=1
cargo test -p autospec-cli --test autonomous_conductor_commands -- --test-threads=1
cargo test --workspace -- --test-threads=1
cargo clippy --workspace --all-targets -- -D warnings
cargo run -q -p autospec-cli -- validate --fast
git diff --check
```
