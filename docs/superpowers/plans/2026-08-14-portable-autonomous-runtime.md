# Portable Autonomous Runtime Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the Rust autonomous claim-to-executor path work safely on Linux, macOS, Windows, and FreeBSD while preserving Linux's pidfd guarantees.

**Architecture:** Keep the existing durable claim, invocation, receipt, and renewal state machines, but move local heartbeat storage and child-tree ownership behind target-aware modules. Linux continues to use the existing pidfd implementation; macOS and FreeBSD own live process groups, while Windows owns a suspended-at-launch process through a kill-on-close Job Object. Recovery may classify durable identities but never signals a numeric PID unless the current process owns the corresponding OS resource.

**Tech Stack:** Rust 2021, `std::process`, target-gated Unix `nix`, target-gated Windows SDK FFI, serde JSON, GitHub Actions, Bats, `vmactions/freebsd-vm@v1`

**Spec:** `docs/superpowers/specs/2026-08-14-portable-autonomous-runtime-design.md`

## Global Constraints

- Support Linux, macOS, Windows, and FreeBSD through the complete autonomous claim-to-executor path.
- GitHub claim compare-and-swap state remains the cross-machine ownership authority.
- A local heartbeat is recovery metadata and never authorizes termination by numeric PID alone.
- A live supervisor terminates only a process tree it owns through an OS resource captured at launch.
- Recovered ambiguous process identity fails closed for signalling; exact remotely released claim state may retire only its exact matching heartbeat.
- Linux retains its current pidfd, procfs, subreaper, and descriptor-relative filesystem implementation; no weaker Linux fallback is permitted.
- Heartbeat state remains repository-scoped and user-private; Unix directories are `0700`, Unix files are `0600`, and Windows paths reject reparse-point traversal.
- Windows launch is suspended until the process is assigned to a Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`.
- The Windows FFI surface is target-gated and limited to process creation, process identity, waiting, and Job Object ownership; add no wrapper dependency for APIs supplied by Windows SDK bindings.
- All non-document changes follow red-green TDD.
- Do not mock operating-system process ownership in behavior tests.
- Do not change the read-only/autonomous GitHub authority model or add database writes.
- Run `bash scripts/validate.sh`, `cargo test --workspace`, lint, typecheck/static analysis available to this Rust workspace, and the platform-specific gates before completion.
- Every commit uses a conventional prefix and the repository's Lore trailers.

---

## File Structure

- `crates/autospec-cli/src/commands/claim/heartbeat_portable.rs` owns non-Linux heartbeat filesystem publication, exact-generation validation, and exact released retirement. It contains no process signalling.
- `crates/autospec-cli/src/commands/claim/heartbeat_predecessor.rs` selects Linux retirement or the portable metadata-only retirement path.
- `crates/autospec-cli/src/commands/claim.rs` builds portable heartbeat identity and delegates publication without changing the Linux publisher.
- `crates/autospec-cli/src/commands/autonomous/executor_bridge/process_owner.rs` defines the common live-child ownership contract and target module selection.
- `crates/autospec-cli/src/commands/autonomous/executor_bridge/process_owner/unix_group.rs` owns macOS/FreeBSD child process groups through retained `Child` handles.
- `crates/autospec-cli/src/commands/autonomous/executor_bridge/process_owner/windows_job.rs` owns Windows child trees through raw, target-gated Windows SDK handles.
- `crates/autospec-cli/src/commands/autonomous/executor_bridge/portability.rs` implements the non-Linux bridge entrypoints by reusing shared durable state and the ownership contract rather than returning Linux-only errors.
- `crates/autospec-cli/src/commands/autonomous/executor_bridge.rs` exposes shared state-machine helpers to the target backends while leaving Linux pidfd code behavior unchanged.
- `.github/workflows/rust.yml` contains explicit Linux, macOS, Windows, and FreeBSD gates.
- `tests/cli/test_rust_workflow.bats` locks the four-platform CI contract.

---

### Task 1: Portable Heartbeat Publication and Released Retirement

**Files:**
- Create: `crates/autospec-cli/src/commands/claim/heartbeat_portable.rs`
- Modify: `crates/autospec-cli/src/commands/claim.rs:4139-4405`
- Modify: `crates/autospec-cli/src/commands/claim.rs:7399`
- Modify: `crates/autospec-cli/src/commands/claim/heartbeat_predecessor.rs`
- Modify: `crates/autospec-cli/src/commands/claim/tests/heartbeat_startup.rs`
- Modify: `crates/autospec-cli/src/commands/claim/tests/heartbeat_prior.rs`
- Modify: `crates/autospec-cli/src/commands/claim/tests/conductor_lease_takeover.rs`

**Interfaces:**
- Consumes: `heartbeat_root()`, `repository_progress_key(&str) -> String`, `heartbeat_session_key(&str) -> String`, `parse_startup_heartbeat(&[u8])`, `ClaimMutationIdentity<'_>`, and the authoritative released `ClaimRefHead` already validated by `heartbeat_predecessor::retire`.
- Produces: `heartbeat_portable::process_identity(pid: u32) -> Result<(String, String, String), CommandFailure>`; `heartbeat_portable::publish(root: &Path, repo: &str, issue: u64, session_id: Option<&str>, document: &[u8]) -> Result<(), CommandFailure>`; `heartbeat_portable::retire_released(identity: ClaimMutationIdentity<'_>) -> Result<(), CommandFailure>`.

- [ ] **Step 1: Replace the non-Linux fail-closed tests with failing portable behavior tests**

  Add target-gated tests that create a private temporary heartbeat root, publish one immutable generation twice, reject a different `claim_id`, reject a final symlink, and retire only the matching released generation. The core assertions are:

  ```rust
  #[cfg(not(target_os = "linux"))]
  #[test]
  fn portable_publication_is_idempotent_but_rejects_another_generation() {
      let fixture = HeartbeatFixture::new();
      fixture.publish("claim-a").expect("initial publication");
      fixture.publish("claim-a").expect("idempotent replay");
      let error = fixture.publish("claim-b").expect_err("generation conflict");
      assert_eq!(error.message, "heartbeat publication target conflicts");
  }

  #[cfg(not(target_os = "linux"))]
  #[test]
  fn released_predecessor_retires_only_its_exact_heartbeat_without_signalling() {
      let fixture = HeartbeatFixture::new();
      fixture.publish("claim-a").expect("heartbeat");
      fixture.retire_released("claim-a").expect("exact retirement");
      assert!(!fixture.issue_path().exists());
  }
  ```

- [ ] **Step 2: Run the focused tests and confirm the red state**

  Run: `cargo test -p autospec-cli heartbeat -- --nocapture`

  Run: `cargo test -p autospec-cli conductor_lease_takeover -- --nocapture`

  Expected: FAIL because non-Linux publication/identity/retirement still returns Linux-only diagnostic errors; on Linux, compile the new portable pure-state helper tests with their direct module test entrypoints.

- [ ] **Step 3: Implement target-aware process identity without signalling authority**

  Keep the current Linux function untouched. On macOS use `sysctl`/`proc_pidinfo` creation data, on FreeBSD use `sysctl(KERN_PROC_PID)` creation data, and on Windows use `GetProcessTimes`; serialize the stable fields into `process_start`. Use the machine hostname plus a boot/session identity string where available. The callable boundary remains:

  ```rust
  #[cfg(not(target_os = "linux"))]
  fn startup_process_identity(pid: u32) -> Result<(String, String, String), CommandFailure> {
      heartbeat_portable::process_identity(pid)
  }
  ```

  Identity lookup proves only whether a durable record still names the same creation instance. Do not add any kill/signal call to the heartbeat module.

- [ ] **Step 4: Implement private, atomic portable publication**

  Implement same-directory create-new temporary files, `write_all`, `sync_all`, atomic rename, parent-directory sync where supported, and parsed immutable-generation comparison. The public logic must follow this shape:

  ```rust
  pub(super) fn publish(
      root: &Path,
      repo: &str,
      issue: u64,
      session_id: Option<&str>,
      document: &[u8],
  ) -> Result<(), CommandFailure> {
      let expected = parse_startup_heartbeat(document)?;
      let repo_dir = open_or_create_private_directory(root, &repository_progress_key(repo))?;
      publish_exact(&repo_dir, &format!("{issue}.json"), &expected, document)?;
      if let Some(session_id) = session_id {
          let sessions = open_or_create_private_directory(&repo_dir, "sessions")?;
          publish_exact(&sessions, &format!("{}.json", heartbeat_session_key(session_id)), &expected, document)?;
      }
      Ok(())
  }
  ```

  Unix validates ownership/type/mode without following final symlinks. Windows opens with reparse-point-safe flags and validates every path component before replacement. If publication of the optional session copy fails after the issue copy commits, the next identical call must complete idempotently.

- [ ] **Step 5: Implement exact released retirement and wire claim startup**

  `heartbeat_predecessor::retire` keeps its Linux body under `cfg(target_os = "linux")`; the non-Linux body validates released remote state and calls:

  ```rust
  heartbeat_portable::retire_released(ClaimMutationIdentity {
      repo,
      issue,
      worker_id: &record.worker_id,
      branch: &record.branch,
      claim_id,
  })
  ```

  `retire_released` parses the local file, compares repository/issue/worker/branch/claim ID exactly, removes only matching issue/session records, and treats absence as success. A mismatch returns success without deleting another generation. Change `write_startup_heartbeat` to call the existing Linux transaction on Linux and `heartbeat_portable::publish` everywhere else.

- [ ] **Step 6: Run heartbeat and claim regression tests**

  Run: `cargo test -p autospec-cli heartbeat -- --nocapture`

  Run: `cargo test -p autospec-cli conductor_lease_takeover -- --nocapture`

  Run: `cargo check -p autospec-cli`

  Expected: all pass on the host; Linux pidfd publication tests remain unchanged.

- [ ] **Step 7: Commit the heartbeat slice**

  ```bash
  git add crates/autospec-cli/src/commands/claim.rs crates/autospec-cli/src/commands/claim docs/superpowers/plans/2026-08-14-portable-autonomous-runtime.md
  git commit -m "fix: let every supported host publish claim heartbeats" -m "Constraint: Heartbeats never authorize PID-only signalling
  Rejected: Skip non-Linux heartbeats | loses recovery evidence
  Confidence: high
  Scope-risk: moderate
  Directive: Preserve Linux descriptor-relative publication unchanged
  Tested: Focused heartbeat and conductor lease tests plus cargo check"
  ```

---

### Task 2: macOS and FreeBSD Owned Process Groups

**Files:**
- Create: `crates/autospec-cli/src/commands/autonomous/executor_bridge/process_owner.rs`
- Create: `crates/autospec-cli/src/commands/autonomous/executor_bridge/process_owner/unix_group.rs`
- Modify: `crates/autospec-cli/src/commands/autonomous/executor_bridge.rs`
- Modify: `crates/autospec-cli/src/commands/autonomous/executor_bridge/portability.rs`

**Interfaces:**
- Consumes: existing `ProcessIdentity`, `AttemptTerminal`, output-sink paths, polling intervals, durable attempt state, and `std::process::Command` prepared by the bridge.
- Produces: `OwnedChildTree::spawn(command: &mut Command, launch_nonce: String) -> Result<Self, String>`; `OwnedChildTree::identity(&self) -> DurableProcessOwner`; `OwnedChildTree::try_wait(&mut self) -> Result<Option<ExitStatus>, String>`; `OwnedChildTree::wait(&mut self) -> Result<ExitStatus, String>`; `OwnedChildTree::terminate(&mut self) -> Result<ExitStatus, String>`; serializable `DurableProcessOwner { pid: u32, container_id: String, process_start: String, launch_nonce: String }`.

- [ ] **Step 1: Add failing ownership contract tests for Unix process groups**

  Tests run only on macOS/FreeBSD and launch real child processes. Cover zero exit, non-zero exit, a stalled child with a descendant, tree termination, and refusal to terminate from only a reconstructed durable identity:

  ```rust
  #[cfg(any(target_os = "macos", target_os = "freebsd"))]
  #[test]
  fn terminate_reaps_the_owned_process_group() {
      let mut command = shell_command("sleep 30 & wait");
      let mut owned = OwnedChildTree::spawn(&mut command, "nonce-a".into()).unwrap();
      let identity = owned.identity();
      let status = owned.terminate().expect("terminate owned group");
      assert!(!status.success());
      assert_eq!(identity.pid.to_string(), identity.container_id);
  }

  #[cfg(any(target_os = "macos", target_os = "freebsd"))]
  #[test]
  fn durable_identity_without_live_owner_cannot_signal() {
      let identity = DurableProcessOwner::fixture_for_current_process();
      assert_eq!(recover_owner(&identity), RecoveryDisposition::Quarantine);
  }
  ```

- [ ] **Step 2: Run the contract tests and confirm they fail**

  Run: `cargo test -p autospec-cli process_owner -- --nocapture`

  Expected: FAIL because `OwnedChildTree` and its Unix backend do not exist.

- [ ] **Step 3: Define the common ownership facade**

  The facade owns the platform enum privately so bridge callers cannot bypass ownership:

  ```rust
  pub(super) struct OwnedChildTree {
      inner: PlatformOwnedChild,
      identity: DurableProcessOwner,
  }

  #[derive(Clone, Debug, Eq, PartialEq)]
  pub(super) struct DurableProcessOwner {
      pub pid: u32,
      pub container_id: String,
      pub process_start: String,
      pub launch_nonce: String,
  }

  pub(super) enum RecoveryDisposition {
      Completed,
      SidecarOwns,
      Quarantine,
  }
  ```

  Linux bridge functions continue using their current `OwnedProcess`/pidfd types; the facade is selected only for macOS, FreeBSD, and Windows until a separate proven refactor is justified.

- [ ] **Step 4: Implement macOS/FreeBSD process-group ownership**

  Before spawn call `std::os::unix::process::CommandExt::process_group(0)`. Retain the returned `Child`; record its PID as the PGID and capture creation identity. `try_wait` and `wait` delegate to `Child`. `terminate` sends `SIGTERM` to `-pgid`, polls briefly, sends `SIGKILL` if required, and always calls `Child::wait`. Signal only through the live `OwnedChildTree`; expose no `kill(pid)` recovery API.

- [ ] **Step 5: Route non-Linux direct commands through the Unix owner**

  Replace the non-Linux `execute_direct_plan` error stub with the same durable intent/output/terminal sequence as Linux, but parameterize child launch/poll/terminate through `OwnedChildTree`. Extract only platform-neutral helpers from the existing Linux function. Preserve terminal mappings:

  ```rust
  match run_owned_attempt(&mut owned, &paths, stall_timeout)? {
      OwnedAttemptResult::Exited(status) => AttemptTerminal::Exit(status.code().unwrap_or(1)),
      OwnedAttemptResult::TimedOut(status) => AttemptTerminal::TimedOut,
      OwnedAttemptResult::OutputOverflow(status) => AttemptTerminal::OutputOverflow,
  }
  ```

  Reconciliation of an intent without a terminal receipt never kills from the recorded PID; it archives the attempt as interrupted/quarantined before a new authoritative generation may launch.

- [ ] **Step 6: Run Unix bridge regressions**

  Run on macOS: `cargo test -p autospec-cli process_owner execute_direct_plan -- --nocapture`

  Run: `cargo check -p autospec-cli`

  Run on Linux: `cargo test -p autospec-cli executor_bridge --lib -- --nocapture`

  Expected: macOS uses process groups; Linux pidfd tests and behavior remain green.

- [ ] **Step 7: Commit the Unix ownership slice**

  ```bash
  git add crates/autospec-cli/src/commands/autonomous/executor_bridge.rs crates/autospec-cli/src/commands/autonomous/executor_bridge
  git commit -m "fix: own autonomous child trees on BSD hosts" -m "Constraint: Recovered numeric PIDs never authorize signalling
  Rejected: Validate then signal a recovered PID | creation checks cannot close the reuse race
  Confidence: high
  Scope-risk: broad
  Directive: Keep live Child ownership coupled to process-group termination
  Tested: Real process-group contract tests, bridge tests, and Linux regression suite"
  ```

---

### Task 3: Windows Job Object Ownership

**Files:**
- Create: `crates/autospec-cli/src/commands/autonomous/executor_bridge/process_owner/windows_job.rs`
- Modify: `crates/autospec-cli/src/commands/autonomous/executor_bridge/process_owner.rs`
- Modify: `crates/autospec-cli/src/commands/autonomous/executor_bridge/portability.rs`
- Modify: `crates/autospec-cli/src/commands/autonomous/executor_bridge.rs`

**Interfaces:**
- Consumes: `OwnedChildTree`, `DurableProcessOwner`, `RecoveryDisposition`, prepared `Command` program/args/environment/current directory/stdio, and durable attempt paths from Task 2.
- Produces: a Windows `PlatformOwnedChild` with owned process, main-thread, and Job Object handles; the same `OwnedChildTree` methods and `DurableProcessOwner` fields used by Unix.

- [ ] **Step 1: Add failing Windows Job Object behavior tests**

  Use real Windows child processes and a helper mode in the Rust test binary. Cover suspended assignment before child code executes, zero/non-zero wait, descendant termination, handle cleanup, and durable identity quarantine:

  ```rust
  #[cfg(windows)]
  #[test]
  fn job_owns_descendant_before_primary_thread_runs() {
      let marker = TempPath::new("windows-owned-child-marker");
      let mut command = helper_command("spawn-descendant-and-wait", marker.path());
      let mut owned = OwnedChildTree::spawn(&mut command, "nonce-win".into()).unwrap();
      owned.terminate().expect("terminate job");
      assert!(!marker.descendant_survived());
  }

  #[cfg(windows)]
  #[test]
  fn windows_creation_filetime_is_part_of_durable_identity() {
      let mut command = helper_command("exit-zero", Path::new("."));
      let owned = OwnedChildTree::spawn(&mut command, "nonce-win".into()).unwrap();
      assert!(!owned.identity().process_start.is_empty());
  }
  ```

- [ ] **Step 2: Compile the new tests to confirm the red state**

  Run: `cargo check -p autospec-cli --tests --target x86_64-pc-windows-msvc`

  Expected: FAIL because the Windows backend is missing. If the target standard library is unavailable locally, install it with `rustup target add x86_64-pc-windows-msvc` and rerun; behavior execution occurs in the Windows CI gate.

- [ ] **Step 3: Implement minimal raw Windows SDK bindings**

  Define `repr(C)` structures and `extern "system"` functions only for `CreateProcessW`, `CreateJobObjectW`, `SetInformationJobObject`, `AssignProcessToJobObject`, `ResumeThread`, `GetProcessTimes`, `WaitForSingleObject`, `GetExitCodeProcess`, `TerminateJobObject`, and `CloseHandle`. Use `CREATE_SUSPENDED | CREATE_UNICODE_ENVIRONMENT`, `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, and RAII `OwnedHandle`:

  ```rust
  struct OwnedHandle(HANDLE);

  impl Drop for OwnedHandle {
      fn drop(&mut self) {
          unsafe { CloseHandle(self.0); }
      }
  }

  struct WindowsJobChild {
      job: OwnedHandle,
      process: OwnedHandle,
      thread: Option<OwnedHandle>,
      pid: u32,
  }
  ```

  Construct the UTF-16 command line with Windows quoting rules, copy the prepared environment/current directory, and preserve stdout/stderr handles as inheritable child handles while keeping unrelated handles non-inheritable.

- [ ] **Step 4: Enforce assign-before-resume and kill-on-close**

  The launch transaction is exactly: create job, set limits, create process suspended, assign process to job, capture creation `FILETIME`, persist in-memory identity, resume main thread. Any failure before resume terminates the job and closes all handles. `terminate` calls `TerminateJobObject`, waits on the process handle, reads its exit code, and returns an `ExitStatus`/portable exit representation without targeting a PID.

- [ ] **Step 5: Integrate Windows with direct-command and harness supervision paths**

  The non-Linux entrypoints in `portability.rs` use `OwnedChildTree` for both direct commands and validated harnesses. Durable state records PID, creation `FILETIME`, job/container identifier, and launch nonce. A current live owner may renew claims, observe progress, publish terminal receipts, and terminate its Job Object. A restarted parent with only JSON state returns `RecoveryDisposition::Quarantine` unless the invocation-bound sidecar receipt proves completion; it never opens and terminates a process by PID.

- [ ] **Step 6: Run Windows compilation and behavior tests**

  Run locally: `cargo check -p autospec-cli --tests --target x86_64-pc-windows-msvc`

  Run on Windows: `cargo test -p autospec-cli process_owner -- --nocapture`

  Run on Windows: `cargo test -p autospec-cli executor_bridge --lib -- --nocapture`

  Expected: raw FFI compiles without an added dependency and Job Object behavior tests pass.

- [ ] **Step 7: Commit the Windows ownership slice**

  ```bash
  git add crates/autospec-cli/src/commands/autonomous/executor_bridge.rs crates/autospec-cli/src/commands/autonomous/executor_bridge
  git commit -m "fix: contain autonomous child trees in Windows jobs" -m "Constraint: Assignment must occur before the primary thread executes
  Rejected: Spawn then assign with std::process | permits descendant escape before assignment
  Confidence: medium
  Scope-risk: broad
  Directive: Keep every Windows SDK symbol target-gated and every HANDLE RAII-owned
  Tested: Windows target compile and real Job Object behavior tests"
  ```

---

### Task 4: Portable Harness Lifecycle, Conductor Regression, and CI Gates

**Files:**
- Modify: `crates/autospec-cli/src/commands/autonomous/executor_bridge/portability.rs`
- Modify: `crates/autospec-cli/src/commands/autonomous/executor_bridge.rs`
- Modify: `crates/autospec-cli/src/commands/autonomous/executor_bridge/accountability_lifecycle.rs`
- Modify: `crates/autospec-cli/src/commands/claim/tests/conductor_lease_takeover.rs`
- Modify: `.github/workflows/rust.yml`
- Modify: `tests/cli/test_rust_workflow.bats`
- Modify: `docs/superpowers/specs/2026-08-14-portable-autonomous-runtime-design.md` only if implementation evidence requires clarifying an already-approved invariant

**Interfaces:**
- Consumes: portable heartbeat functions from Task 1 and `OwnedChildTree` plus `RecoveryDisposition` from Tasks 2-3.
- Produces: working non-Linux `run_executor_bridge`, `reconcile_direct_launch`, `execute_direct_plan`, `create_draft_pull_request`, and `supervise_validated_harness_with_claim_renewal` entrypoints with the same observable receipts/results as Linux; explicit four-platform CI jobs.

- [ ] **Step 1: Add the failing end-to-end non-Linux admission regression**

  Build a hermetic test around the existing conductor/bridge adapters: a released predecessor heartbeat exists, the successor claim is acquired, a real no-op harness runs, and a terminal receipt is reconciled. Assert the event stream never contains either former diagnostic:

  ```rust
  #[cfg(not(target_os = "linux"))]
  #[test]
  fn released_predecessor_advances_through_executor_on_supported_host() {
      let fixture = PortableConductorFixture::released_predecessor();
      let outcome = fixture.run_noop_executor().expect("portable autonomous run");
      assert_eq!(outcome.exit_code, 0);
      assert!(!fixture.events().contains("predecessor heartbeat retirement requires Linux pidfd ownership"));
      assert!(!fixture.events().contains("executor supervision requires Linux pidfd ownership"));
      assert!(fixture.has_terminal_receipt());
  }
  ```

- [ ] **Step 2: Run the admission regression and confirm the red state**

  Run: `cargo test -p autospec-cli released_predecessor_advances_through_executor_on_supported_host -- --nocapture`

  Expected: FAIL until every non-Linux entrypoint delegates to the portable state machine and owner.

- [ ] **Step 3: Finish portable harness lifecycle delegation**

  Move platform-neutral orchestration out of Linux-only `cfg` blocks: durable invocation load/write, claim renewal, output progress, snapshot verification, terminal receipt publication, review/PR state, and completed sidecar reconciliation. Keep only pidfd/subreaper/openat operations Linux-only. The non-Linux `run_executor_bridge` and `create_draft_pull_request` must call the shared lifecycle rather than reject admission.

  Recovery classification follows this total mapping:

  ```rust
  match (terminal_receipt, live_owner, durable_identity_match) {
      (Some(receipt), _, _) => reconcile_terminal_receipt(receipt),
      (None, Some(owner), true) => continue_live_supervision(owner),
      (None, None, false) => quarantine_without_signal(),
      (None, None, true) => quarantine_without_signal(),
      (None, Some(_), false) => quarantine_without_signal(),
  }
  ```

  Exact released remote claim plus matching heartbeat permits metadata retirement and successor admission even when an old process identity is ambiguous.

- [ ] **Step 4: Add four-platform CI tests before editing the workflow**

  Extend `tests/cli/test_rust_workflow.bats` so it expects:

  ```python
  assert jobs["build-test"]["runs-on"] == "ubuntu-latest"
  assert jobs["macos-test"]["runs-on"] == "macos-latest"
  assert jobs["windows-test"]["runs-on"] == "windows-latest"
  assert "vmactions/freebsd-vm@v1" in freebsd_uses
  for name in ("build-test", "macos-test", "windows-test"):
      assert "cargo test -p autospec-cli" in commands(jobs[name])
  assert "cargo test -p autospec-cli" in freebsd_commands
  ```

  Run: `bats tests/cli/test_rust_workflow.bats`

  Expected: FAIL because macOS is build-only and Windows/FreeBSD jobs are absent.

- [ ] **Step 5: Add executable platform gates**

  Rename `macos-build` to `macos-test`; run `cargo check -p autospec-cli`, focused portable heartbeat/process-owner/executor tests, and release build. Add `windows-test` on `windows-latest` using PowerShell-compatible cargo commands. Add an Ubuntu-hosted `freebsd-test` whose `vmactions/freebsd-vm@v1` `prepare` installs Rust and whose `run` checks/builds/tests `autospec-cli` inside FreeBSD. Keep Linux clippy/build/core tests and add the portable pure-state suites there.

- [ ] **Step 6: Run host validation and cross-target compilation**

  Run: `bats tests/cli/test_rust_workflow.bats`

  Run: `cargo fmt --all -- --check`

  Run: `cargo clippy --workspace --all-targets -- -D warnings`

  Run: `cargo test --workspace`

  Run: `cargo check -p autospec-cli --target x86_64-pc-windows-msvc`

  Run: `bash scripts/validate.sh`

  Expected: every available local command passes. Record Windows/FreeBSD runtime behavior evidence from their CI jobs; conditional compilation alone is not sufficient evidence.

- [ ] **Step 7: Run a macOS smoke reproduction of the original blocker**

  In an isolated temporary repository/config root, create a released predecessor plus matching heartbeat, run the autonomous claim/executor with a no-op hermetic harness, and inspect structured events. Expected: successor acquisition and terminal receipt; no `claim_deferred` loop and no Linux-only diagnostics. Do not point this smoke test at a production GitHub repository.

- [ ] **Step 8: Commit the integration and CI slice**

  ```bash
  git add crates/autospec-cli/src/commands/autonomous/executor_bridge.rs crates/autospec-cli/src/commands/autonomous/executor_bridge crates/autospec-cli/src/commands/claim/tests/conductor_lease_takeover.rs .github/workflows/rust.yml tests/cli/test_rust_workflow.bats
  git commit -m "test: enforce autonomous ownership on four operating systems" -m "Constraint: Supported means behavior-tested, not merely conditionally compiled
  Rejected: Build-only non-Linux gates | cannot prove process-tree ownership
  Confidence: high
  Scope-risk: moderate
  Directive: Keep Linux, macOS, Windows, and FreeBSD gates explicit
  Tested: Workspace validation plus macOS smoke and platform CI behavior suites"
  ```

---

## Completion Evidence

- The original macOS released-predecessor scenario advances rather than repeating `claim_deferred`.
- A real harness reaches a durable terminal receipt on macOS, Windows, and FreeBSD.
- Stall termination removes descendants through a retained process group or Job Object.
- Restart recovery reconciles receipts and quarantines ambiguous ownership without signalling by PID.
- Existing Linux pidfd tests pass unchanged.
- `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `bash scripts/validate.sh`, and workflow contract tests pass.
- GitHub Actions supplies green Linux, macOS, Windows, and FreeBSD behavior gates before support is claimed complete.
