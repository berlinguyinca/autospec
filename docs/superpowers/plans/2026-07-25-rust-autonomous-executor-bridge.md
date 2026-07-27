# Rust Autonomous Executor Bridge Implementation Plan

**Goal:** Replace the foreground conductor's external result-file dependency
with a recoverable Rust-owned bridge that implements, verifies, reviews, merges,
and cleans up one exact claimed issue without operator intervention.

**Architecture:** A dedicated `executor_bridge` module owns harness resolution,
repository-scoped worktree/runtime identity, direct child supervision,
persisted phases, deterministic validation, and the PR-through-merge
transaction. `autonomous.rs` remains the conductor and `claim.rs` remains the
lease authority. Existing typed premerge decisions and executor-result evidence
remain the result-ingestion boundaries.

**Tech stack:** Existing Rust workspace, Git and GitHub CLI adapters, existing
runtime alias table, runtime broker, claim/premerge types, and serial Rust
integration tests. No new dependencies.

## Implementation record

The conductor integration keeps the high-level bridge entrypoint in
`executor_bridge.rs` so one Rust authority owns process launch, durable
receipts, Git/GitHub proof, and cleanup. Conductor state remains responsible for
queue selection and retry scheduling only. Recovery state and terminal receipts
are claim-generation scoped, and a durable raw supervisor allows a restarted
conductor to observe one live harness and later consume its exact exit without
launching a duplicate.

Runtime recovery records the environment directory, session identity, and
original manifest snapshot before provisioning. This is stricter than deriving
cleanup from the current repository manifest: a changed or removed manifest
cannot redirect cleanup, and an incomplete persisted runtime binding fails
closed. Retry preserves dirty issue-worktree evidence, then advances clean
committed work to a changed base without force under a fresh claim generation.

## Constraints

- Work only in the issue worktree created from the configured base resolver:
  explore sandbox, `AUTOSPEC_BASE_BRANCH`, repo config, then remote default.
- Write a failing regression before each behavior change.
- Never invoke `autospec-run`, `omx`, a shell conductor, or the primary checkout.
- Never treat process exit, free-form stdout, or a harness claim as proof.
- Keep the fixed executor-result artifact as compatibility input.
- Bind every invocation to one repository, issue, worker, branch, claim,
  configured base ref/OID, private worktree, runtime session, and PR head.
- A model may block verification but cannot create QA/security Pass evidence.
- Rust owns push, draft creation, ready transition, CI admission, merge,
  terminal claim release, and cleanup.

### Task 1: Install and verify required executor scanners

**Files:**

- Modify `install.sh`
- Modify `skills/autospec-shared/scripts/ensure-tool.sh`
- Modify `tests/install/test_required_dependencies.sh`

1. Add failing installer tests proving gitleaks, semgrep, trivy, and
   license-checker are attempted and then verified for autonomous installs.
2. Extend cross-platform installer fallbacks, including the existing approved
   sudo system-package path, without running real package managers in tests.
3. Preserve global/per-tool skip controls, but report the exact missing scanner
   so executor security admission can fail closed.
4. Commit scanner dependency provisioning.

### Task 2: Add typed harness and invocation contracts

**Files:**

- Add `crates/autospec-cli/src/commands/autonomous/executor_bridge.rs`
- Modify `crates/autospec-cli/src/commands/autonomous.rs`

1. Add failing unit tests for runtime-marker precedence, explicit override,
   alias-table parsing, unsafe dispatcher rejection, and exact Codex, Claude,
   and OpenCode argument vectors.
2. Implement `HarnessKind`, `HarnessConfig`, `BridgeIdentity`, `BridgePhase`,
   process identity, and strict persisted invocation JSON.
3. Resolve the installed alias table from the existing environment/config
   locations and resolve an absolute non-temporary executable.
4. Use Codex workspace-write containment; use local-only Claude policy plus
   before/after mutation snapshots where OS containment is unavailable.
   Fail closed for OpenCode because `--pure` does not constrain built-in tools;
   launch it only through an explicitly configured, proven containment adapter.
5. Commit the typed contract and tests.

### Task 3: Provision and recover isolated worktree and runtime state

**Files:**

- Modify `crates/autospec-cli/src/commands/autonomous/executor_bridge.rs`
- Modify `crates/autospec-cli/src/commands/runtime/env.rs`
- Modify `crates/autospec-cli/tests/autonomous_conductor_commands.rs`

1. Add a failing integration fixture backed by a real local Git repository and
   bare remote.
2. Resolve validated `.autospec/explore-mode.json` sandbox base first; otherwise
   resolve `AUTOSPEC_BASE_BRANCH`, then `.autospec/autospec.yml`
   `git.base_branch`, then remote default. Reject a main target in explore mode
   and persist the exact base ref and OID.
3. Fetch and create the exact `feat/autonomous-issue-<N>` branch under the private
   `/tmp/autospec-executor/<repository-scope>/issue-<N>` root.
4. Adopt only a matching clean branch/worktree; fail closed on dirty, detached,
   foreign, symlinked, wrong-owner, or mismatched reuse.
5. Provision manifest-backed runtime isolation through the typed
   `runtime env session` adapter; use no runtime session without a manifest.
6. Persist non-terminal state atomically before launch and recover the last
   independently verified phase after restart.
7. Test two repositories with the same issue number and two isolated runtime
   manifests, then commit worktree/runtime recovery behavior.

### Task 4: Launch and supervise the implementation harness

**Files:**

- Modify `crates/autospec-cli/src/commands/autonomous/executor_bridge.rs`
- Modify `crates/autospec-cli/tests/autonomous_conductor_commands.rs`

1. Add failing tests proving one direct launch, exact argv, output progress,
   stall termination, verified process-group cleanup, and no duplicate child.
2. Build the dedicated implementer prompt from exact issue, claim, branch,
   worktree, base, local-only, Closeout, and no-remote-mutation requirements.
3. Stream bounded child output into structured executor events while refreshing
   local progress state.
4. Persist canonical executable, argv digest, PID/PGID, and boot/start identity;
   observe or signal a child only on an exact live identity match.
5. Replace the 30-second absolute timeout with progress-aware stall detection.
6. Make pending and interrupted phases non-terminal.
7. Add PID-reuse and malicious primary/protected-ref mutation fixtures.
8. Commit supervision behavior.

### Task 5: Add compare-and-set claim renewal

**Files:**

- Modify `crates/autospec-cli/src/commands/claim.rs`
- Modify `crates/autospec-cli/src/commands/autonomous/executor_bridge.rs`
- Modify `crates/autospec-cli/tests/claim_commands.rs`
- Modify `crates/autospec-cli/tests/autonomous_conductor_commands.rs`

1. Add failing tests for exact-generation refresh, stale generation, changed
   worker/branch/claim ID, simulated elapsed time beyond TTL, and takeover.
2. Add a claim API that re-reads the authoritative run-state comment, verifies
   exact identity, updates only heartbeat/step/PR fields, and confirms the
   written generation.
3. Refresh during implementation, verification, CI wait, and review; abort
   inertly before further remote mutation when ownership is lost.
4. Commit claim renewal and takeover safety.

### Task 6: Prove and create the draft PR

**Files:**

- Modify `crates/autospec-cli/src/commands/autonomous/executor_bridge.rs`
- Modify `crates/autospec-core/src/claim/mod.rs`
- Modify `crates/autospec-cli/src/commands/claim.rs`
- Modify `crates/autospec-cli/tests/autonomous_conductor_commands.rs`
- Modify `crates/autospec-core/tests/claim_tiebreak.rs`

1. Add failing tests for unchanged HEAD, dirty state, foreign branch, missing or
   multiple PRs, extra branches/PRs, base OID drift, wrong head OID, primary
   mutation, missing issue close, and malformed Closeout report.
2. Extend strict pull-request evidence with `isDraft` and update every fixture.
3. Re-read Git and pre-launch mutation snapshots after the implementer exits.
4. Run deterministic implementation lint before Rust pushes only the exact
   issue branch and creates one draft PR with the validated Closeout report.
5. Require exact draft/head/base/issue-close identity after creation.
6. Commit draft-PR proof and Rust-owned mutation.

### Task 7: Produce full QA and security evidence

**Files:**

- Modify `crates/autospec-cli/src/commands/autonomous/executor_bridge.rs`
- Modify `crates/autospec-cli/src/commands/autonomous/premerge.rs`
- Modify `crates/autospec-cli/tests/autonomous_conductor_commands.rs`

1. Add failing tests for direct Primary smoke execution, sequential `&&`,
   rejected shell operators, full-suite resolution, validation failure,
   degraded or failing scanners, and fabricated model Pass output.
2. Parse the one Primary smoke line into a bounded direct-command plan, execute
   each `&&` segment without a shell inside the isolated runtime session, and
   stop on the first failure.
3. Resolve and directly run the full target-repository suite: bounded
   `AUTOSPEC_FULL_TEST_COMMAND`, every Operator/full verification command, or
   detected ecosystem lint/typecheck/test/build commands. Primary smoke remains
   an additional focused gate. Run the full suite again after the branch is
   current with its configured base immediately before merge.
4. Run the Rust implementation linter plus direct gitleaks, semgrep, trivy, and
   license-checker scans with exact artifact and exit capture. Missing required
   scanners fail closed after the installer dependency check; a model may
   triage or add findings but cannot convert a failed/missing scanner to Pass.
5. Produce typed QA/security evidence only from observed runtime, linter, and
   scanner results, evaluate premerge, and require Pass.
6. Commit deterministic premerge evidence.

### Task 8: Review, wait for CI, merge, and release

**Files:**

- Modify `crates/autospec-cli/src/commands/autonomous/executor_bridge.rs`
- Modify `crates/autospec-cli/src/commands/claim.rs`
- Modify `crates/autospec-cli/tests/autonomous_conductor_commands.rs`

1. Add failing tests for draft-to-ready, pending/failing/advisory CI, non-LGTM
   review, base drift after premerge, merge failure after accepted result,
   observed merged state, terminal claim release, and invocation-owned cleanup.
   Include retryable release-to-queue and exhausted
   release-to-`autospec:needs-human` cases that leave no active claim.
2. Mark the exact draft ready only after premerge Pass, poll all non-advisory
   required checks, and refresh the claim throughout the wait.
3. Launch a bounded independent reviewer; strict LGTM can admit only after all
   deterministic/runtime gates pass and any finding blocks.
4. Re-read configured base/head immediately before admission. On drift, update
   without force and regenerate every commit-bound gate: smoke, full suite,
   scanner artifacts, typed QA/security, Pass receipt, push, CI, and LGTM.
5. While the exact PR remains open, ingest strict successful executor-result
   evidence for the stable head/receipt. Then admin-squash-merge, confirm merged
   state, write terminal merged claim state, tear down the owned runtime session,
   and remove the worktree. A merge failure resumes from accepted evidence.
6. On retryable failure, preserve WIP and release back to `auto-implement`. On
   exhaustion/non-retryable failure, preserve evidence, label
   `autospec:needs-human`, remove queue/claim ownership, and continue other work.
7. Commit end-to-end completion.

### Task 9: Wire the conductor and remove terminal pending replay

**Files:**

- Modify `crates/autospec-cli/src/commands/autonomous.rs`
- Modify `crates/autospec-cli/tests/autonomous_conductor_commands.rs`

1. Add a failing foreground regression proving a selected issue reaches the
   bridge rather than the fixed pending child.
2. Call the bridge from `ExecutorRequest`, preserve compatibility artifact
   ingestion, parse exact JSON, and persist receipts only for terminal outcomes.
3. Avoid duplicate blocked `record_executor_outcome` after accepted success,
   and reject any terminal state that still owns `in-progress-by-bot`.
4. Record conductor success only after the bridge observes merged PR state.
5. Commit conductor integration.

### Task 10: Document, review, merge, reinstall, and dogfood

**Files:**

- Modify `docs/cli-reference.md`
- Modify `docs/workflows.md`
- Modify `docs/superpowers/specs/2026-07-25-rust-autonomous-executor-bridge-design.md`
- Modify `docs/superpowers/plans/2026-07-25-rust-autonomous-executor-bridge.md`

1. Document harness selection, base resolution, repository-scoped worktree and
   runtime isolation, recovery, claim renewal, progress, stop, deterministic
   evidence, CI/review, merge, cleanup, and compatibility behavior.
2. Run targeted formatting, focused serial tests, full serial workspace tests,
   Clippy with warnings denied, fast validation, implementation lint, and diff
   checks.
3. Run an independent review and repair every blocking finding.
4. Open the issue-linked PR, wait for required CI, admin-squash-merge, fetch
   exact merged main, build from a clean detached worktree, and install it.
5. Verify all four scanners from the installed environment.
6. Restart autospec-gui autonomy with follow enabled and prove issue #36 creates
   and merges a PR, then observe #34 and #35 without touching its existing
   `.gitignore` change.

## Verification commands

Run:

```bash
cargo test -p autospec-cli autonomous_executor_bridge -- --test-threads=1
cargo test -p autospec-cli --test autonomous_conductor_commands -- --test-threads=1
cargo test -p autospec-cli --test claim_commands -- --test-threads=1
cargo test -p autospec-core --test claim_tiebreak -- --test-threads=1
cargo test --workspace -- --test-threads=1
cargo clippy --workspace --all-targets -- -D warnings
cargo run -q -p autospec-cli -- validate --fast
git diff --check
```
