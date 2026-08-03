# Released Predecessor Heartbeat Recovery Implementation Plan

**Goal:** Retire an exact dead heartbeat from a released predecessor before a successor claim publishes its heartbeat.

**Architecture:** Before successor CAS, inspect released-predecessor heartbeat evidence descriptor-relatively and resume the existing exact durable handoff.

## Global Constraints

- Preserve exact identity; keep unsafe, live, foreign, and ambiguous evidence fail-closed.
- Do not weaken stale timing, branch protection, or add dependencies.

### Task 1: Recover the released predecessor before successor acquisition

**Files:** Modify `crates/autospec-cli/src/commands/claim.rs`; test `crates/autospec-cli/tests/autonomous_conductor_commands.rs`.

**Interface:** `retire_released_predecessor_heartbeat(repo, issue, prior) -> Result<(), CommandFailure>`.

- [ ] **Step 1: Write failing integration tests**
Cover exact dead recovery, foreign/live/root rejection, and pending-handoff resumption before CAS.

- [ ] **Step 2: Run the focused test and confirm the current failure**

Run: `cargo test released_predecessor_heartbeat -- --test-threads=1`

Expected: exact recovery fails with `heartbeat_write_failed`; unsafe cases mutate the claim ref.

- [ ] **Step 3: Implement descriptor-relative pre-acquisition recovery**
Allow genuine absence; resume exact live/pending/completed evidence; reject all unsafe or mismatched evidence before CAS.

- [ ] **Step 4: Verify focused and repository gates**

Run `cargo test released_predecessor_heartbeat -- --test-threads=1`, `cargo test --workspace -- --test-threads=1`, `cargo clippy --workspace --all-targets -- -D warnings`, and `target/debug/autospec validate --jobs auto`.

- [ ] **Step 5: Commit and publish**

Commit the test, implementation, and this plan with a conventional Lore message; push `feat/2879-released-heartbeat`, open an `auto-implement` PR closing #2879, obtain LGTM, pass PR-aware lint and required CI, then admin-squash-merge.
