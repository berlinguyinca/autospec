# Portable autonomous runtime final fix report

Date: 2026-08-14

Scope: final whole-branch fix wave for the five Important findings against `39510b39..8118e099`.

## Outcome

All five findings are fixed with regression coverage. The affected macOS behavior suites pass locally, the workflow contract is enforced by Bats, and the autospec CLI test graph compiles for Windows, FreeBSD, and Linux. No recovery path gained numeric-PID signalling authority; Linux's production pidfd and heartbeat implementations remain intact.

## Finding 1 — portable draft reconciliation retained a synthetic process binding

### RED

- Extended `portable_draft_post_request_crash_reconciles_without_replay` to require both in-memory and durable `draft_process` retirement after authoritative PR reconciliation.
- Before the fix, the test failed with `authoritative portable PR reconciliation retained the synthetic draft process`.

### Fix

- `executor_bridge.rs` now validates the exact private cleanup receipt for the invocation before clearing the portable synthetic draft binding.
- Validation requires the complete schema, exact invocation ID, `tree_cleanup: "proven"`, and a null or valid 32-bit exit code.
- Only after that proof does authoritative PR reconciliation persist `draft_process: None`.

### GREEN

- `cargo test -p autospec-cli --bin autospec draft_release -- --nocapture` — 7 passed.

## Finding 2 — BSD process-group ownership was reaped before descendants were safely drained

### RED

- Added `execute_direct_plan_cleans_descendant_after_successful_leader_exit`, which launches a real background descendant and requires it to be gone after a successful leader exit.
- Added a 64-iteration `completed_empty_groups_never_signal_after_owner_is_consumed` stress regression that forbids any second signal/probe after ownership retirement.
- Before the fix, the direct test observed a live descendant and the stress test observed a post-retirement SIGTERM attempt.

### Fix

- Unix ownership now observes leader completion with `waitid(..., WNOWAIT)` so the unreaped leader pins the process-group identity while descendants are drained.
- Signalling authority is consumed before TERM/KILL cleanup, and the leader is reaped only after group cleanup.
- `wait`, `try_wait`, `terminate`, and `Drop` now converge on the same exactly-once ownership lifecycle.
- No durable/recovery PID is signalled.

### GREEN

- `cargo test -p autospec-cli --bin autospec process_owner -- --nocapture` — 10 passed.
- `cargo test -p autospec-cli --bin autospec portability -- --nocapture` — 10 passed, 1 harness helper intentionally ignored.

## Finding 3 — heartbeat retirement could strand the session copy after an issue-detach crash

### RED

- Added `retirement_resumes_after_crash_between_issue_and_session_detachment`, which panics immediately after the issue heartbeat is detached, retries retirement, and then publishes a successor.
- Before the fix, retry returned with the old session heartbeat still present.

### Fix

- Retirement stage names are deterministic SHA-256-derived names bound to the exact claim identity and live entry name.
- Retry first resumes an existing exact stage, then proceeds to the session retirement stage.
- Both Unix and Windows recovery reopen the detached generation through the retained directory handle; mismatches still fail closed.

### GREEN

- `cargo test -p autospec-cli --bin autospec heartbeat -- --nocapture` — 32 passed.

## Finding 4 — portable heartbeat publication could escape the validated directory or leave unsafe aliases

### RED

- Added `publication_remains_bound_to_open_repository_after_parent_swap`; before the fix publication followed the replacement path instead of the retained repository handle.
- Added `publication_retry_cleans_crash_staging_aliases`; before the fix a pre-rename crash left a temporary alias.
- Added a Windows regression requiring multi-link destinations to be rejected.

### Fix

- Root, repository, sessions, stage, and destination operations are descriptor/handle-relative.
- Publication uses a deterministic exact-generation stage and safely resumes it after a crash.
- macOS/FreeBSD use descriptor-relative `renameatx_np(..., RENAME_EXCL)`; Linux's test-only portable module uses `renameat2(RENAME_NOREPLACE)`; Windows uses handle-relative rename rather than hard-link/delete publication.
- The published target is reopened relative to the retained directory and required to match the staged file identity with exactly one link.

### GREEN

- Heartbeat suite: 32 passed.
- Windows, FreeBSD, and Linux target checks compile successfully.

## Finding 5 — CI filters could pass without running the intended behavior

### RED

- Strengthened `tests/cli/test_rust_workflow.bats` to require exact fully-qualified test names, exact-pass-count checks, Linux pidfd coverage, supported-host admission, and PowerShell exit-code handling.
- The old workflow failed the strengthened contract test.

### Fix

- Each OS lane now invokes exact named behavior tests and verifies the expected `... ok` line occurs exactly once.
- Linux explicitly selects the production pidfd identity test plus full admission.
- macOS and FreeBSD select portable process ownership plus full admission.
- Windows selects Job Object creation identity plus full admission and captures `$LASTEXITCODE` immediately under `$ErrorActionPreference = 'Stop'`.
- The supported-host admission module now also compiles on Linux so the same end-to-end admission test is selectable there.

### GREEN

- `bats tests/cli/test_rust_workflow.bats` — 1 passed.
- Exact local heartbeat and supported-host admission selectors each ran once and passed. The Linux-only pidfd selector correctly compiles only for the Linux job; the Bats contract prevents a zero-test CI success.

## Changed files

- `.github/workflows/rust.yml` — exact positive-count OS behavior gates.
- `crates/autospec-cli/src/commands/autonomous/executor_bridge.rs` — exact portable draft cleanup verification and binding retirement.
- `crates/autospec-cli/src/commands/autonomous/executor_bridge/portability.rs` — supported-host coverage and real descendant cleanup regression.
- `crates/autospec-cli/src/commands/autonomous/executor_bridge/process_owner.rs` — updated exactly-once wait/termination contract regression.
- `crates/autospec-cli/src/commands/autonomous/executor_bridge/process_owner/unix_group.rs` — WNOWAIT ownership lifecycle and stress regression.
- `crates/autospec-cli/src/commands/autonomous/executor_bridge/tests/draft_release.rs` — durable synthetic-binding retirement regression.
- `crates/autospec-cli/src/commands/claim/heartbeat_portable.rs` — handle-relative crash-safe publication and resumable exact retirement.
- `tests/cli/test_rust_workflow.bats` — workflow selection and positive-count assertions.

## Verification

- Formatting: `cargo fmt --all -- --check` — pass.
- Diff hygiene: `git diff --check` — pass.
- Clippy: `cargo clippy -p autospec-cli --bin autospec --tests -- -D warnings` — pass.
- Build: `cargo build --workspace` — pass.
- Heartbeat: 32 passed.
- Process ownership: 10 passed.
- Portability/admission: 10 passed, 1 intentionally ignored helper.
- Draft release: 7 passed.
- Workflow Bats: 1 passed.
- Cross-target checks:
  - `x86_64-pc-windows-msvc` — pass (pre-existing unrelated warnings only).
  - `x86_64-unknown-freebsd` — pass.
  - `x86_64-unknown-linux-gnu` — pass (test-only portable module dead-code warnings only).

## Validation gaps and baseline evidence

- The canonical `cargo run -q -p autospec-cli -- validate` command replaces the removed legacy shell validator.
- `cargo test --workspace` is not green on this macOS checkout: 555 passed, 32 failed, 1 ignored. Representative failures (`/bin/false` absent and private-path tests rejecting macOS `/tmp`/`/var` indirection) reproduce from a clean `origin/main` worktree and do not touch this change set.
- `cargo test -p autospec-core --lib` similarly reports 58 passed and 5 pre-existing compose-normalization failures caused by the same macOS temporary-path safety constraint. `autospec-core` is unchanged by this fix wave.
- Native Windows and FreeBSD execution remains delegated to CI; local evidence is cross-target compilation plus OS-specific exact-test workflow enforcement.

## Self-review

- Confirmed all new signalling remains restricted to a live `UnixOwnedChild` resource; recovery continues to classify/quarantine only.
- Confirmed Linux production heartbeat publication and pidfd ownership paths were not replaced by the portable implementation.
- Confirmed deterministic stages are accepted only after exact-generation validation and publication destinations are verified by file identity and link count.
- Confirmed no dependencies, secrets, debug files, or temporary artifacts were added.
