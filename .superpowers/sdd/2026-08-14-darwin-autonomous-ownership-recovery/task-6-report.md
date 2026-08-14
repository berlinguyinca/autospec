# Task 6 report: Darwin executor ownership and cleanup

## Result

The autonomous executor bridge now runs natively on Darwin. Each executor launches as the leader of a dedicated process group, persists the leader's boot/start/group identity, and re-proves that exact identity before every group signal. Adoption, direct-command reconciliation, claim-loss cleanup, stall cleanup, draft creation, and completion all fail closed when identity or group membership is ambiguous. Linux retains its existing pidfd and child-subreaper implementation.

## RED evidence

- The initial Darwin portability filter still asserted the old unsupported-platform admission stub.
- The existing `harness_supervisor`, `adoption_cleanup`, `restart_direct`, and `sidecar_launch` filters initially executed zero Darwin tests because their modules were Linux-gated.
- After adding the Darwin contract harness, compilation failed because `darwin_supervisor` did not exist. This was the intended structural RED.
- The first termination test then failed with `Darwin executor process-group membership is permission-denied`, proving that a single `killpg(..., 0)` observation was insufficient while killed descendants were still being reaped. The cleanup wait now retains that uncertainty until the bounded deadline and never treats `EPERM` as an empty group.

## Implementation

- `DarwinOwnedGroup` launches with `setpgid(0, 0)` in `pre_exec`, captures birth identity on both sides of process-group observation, and accepts the launch only when PID, PGID, boot identity, and start identity are stable.
- Normal executor launches receive the existing credential-stripped environment plus credentialless Git/GitHub configuration. The trusted draft-PR adapter has an explicit separate launch entry point.
- Adoption uses `platform_process::observe_expected` and requires the persisted PGID to match the live leader's group.
- `SIGTERM` and any subsequent `SIGKILL` each require a fresh exact identity proof. Group emptiness is proven only by `ESRCH`; `EPERM` and other observation errors remain fail-closed.
- Completion requires both a terminal direct-child status and an empty process group. Reconciliation of an already-dead direct leader independently proves the group empty before retiring launch evidence.
- Claim loss and stalls terminate the exact group before clearing process identity. Ambiguous supervision records interruption while retaining the durable identity for recovery.
- Darwin output uses private durable sink/cursor/exit files and feeds the existing durable reader/event pipeline. Direct output retains the existing one-MiB bound.
- Linux-only pidfd, `/proc`, and `PR_SET_CHILD_SUBREAPER` code remains behind `cfg(target_os = "linux")`; the Linux cross-target build confirms those branches still compile.

## GREEN evidence

- `cargo test -p autospec-cli --bin autospec portability -- --nocapture` -> 1 passed.
- `cargo test -p autospec-cli --bin autospec harness_supervisor -- --nocapture` -> 1 passed.
- `cargo test -p autospec-cli --bin autospec adoption_cleanup -- --nocapture` -> 1 passed.
- `cargo test -p autospec-cli --bin autospec restart_direct -- --nocapture` -> 1 passed.
- `cargo test -p autospec-cli --bin autospec sidecar_launch -- --nocapture` -> 1 passed.
- `cargo test -p autospec-cli --bin autospec production_entry::autonomous_executor_bridge_ -- --nocapture` -> 10 passed.
- `cargo check -p autospec-cli --all-targets` -> exit 0 on `aarch64-apple-darwin` (warnings only).
- `cargo check -p autospec-cli --all-targets --target x86_64-unknown-linux-gnu` -> exit 0 (warnings only).
- `git diff --check` -> exit 0.

## Broad bridge-suite classification

`cargo test -p autospec-cli autonomous_executor_bridge -- --nocapture` completed with 142 passed and 67 failed. None of the failures came from Darwin supervision, output journals, cleanup, adoption, direct restart, or sidecar launch. All 67 panics belong to existing unowned macOS fixture-portability or global-fixture-race families:

- 32: fixtures build paths through `/tmp`, which is a symlink to `/private/tmp` on macOS and is correctly rejected by the existing private-path validator.
- 25: fixture repositories do not create `.git/hooks` before code canonicalizes that directory.
- 3: Linux-only executable literals (`/bin/true` and `/usr/bin/sleep`).
- 3: Linux sandbox/bwrap expectations.
- 2: missing premerge fixture roots.
- 1: a `/var/tmp` versus `/private/var/tmp` canonical-path expectation.
- 1: a globally concurrent producer failpoint expected exit 86 but observed the sibling failpoint's exit 87.

The broad result improved from 140 passed / 69 failed after extending the exact-group reap grace and requiring dead-leader reconciliation to prove group emptiness. The remaining fixes belong to unowned fixture modules such as `tests/support_base.rs`, `tests/full_suite.rs`, and `tests/production_entry.rs`; this task did not widen scope into them.

## Static analysis

`cargo clippy -p autospec-cli --all-targets -- -D warnings` remains blocked by 42 pre-existing errors outside the Task 6 implementation. They include unused imports/dead test helpers, pre-existing nonminimal boolean expressions, `platform_process.rs` Darwin portability lints, and `items_after_test_module` in `launch.rs`. After removing the Task 6 macOS-only unused imports, the rerun reports no error in `darwin_supervisor.rs`, `portability.rs`, or `tests/harness_supervisor.rs`, and no error in the new Darwin supervision sections.

## Files changed

- `crates/autospec-cli/src/commands/autonomous/executor_bridge.rs`
- `crates/autospec-cli/src/commands/autonomous/executor_bridge/darwin_supervisor.rs`
- `crates/autospec-cli/src/commands/autonomous/executor_bridge/portability.rs`
- `crates/autospec-cli/src/commands/autonomous/executor_bridge/tests/harness_supervisor.rs`
- `.superpowers/sdd/2026-08-14-darwin-autonomous-ownership-recovery/task-6-report.md`

The adoption, restart, and sidecar contract tests are colocated in `darwin_supervisor.rs` so their named filters run on Darwin without widening the Linux-only fixture modules.

## Concerns

- Darwin cannot use Linux subreaper/pidfd semantics. A daemon that deliberately leaves the launched process group is outside the owned group and is not claimed; ambiguity retains evidence rather than authorizing a signal.
- Repo-wide `cargo fmt --all -- --check` reports unrelated pre-existing formatting drift. Formatting was restricted to the assigned files, and `git diff --check` passes.
- The 67 broad-suite failures are not release-green, but their exact panic messages are fixture portability/race defects outside Task 6 ownership. Focused Darwin behavior and both native/cross-target compilation are green.

## Review fix round 1

The first review found that the initial direct `Command` implementation could not survive its
parent to finish output or publish a terminal record, and that Darwin draft creation bypassed the
Linux release transaction. The repair replaces that launch with a blocked sidecar supervisor which
owns the bounded output ring and publishes a two-sync `EXIT`/`DONE` terminal fence. User code is
released only after the exact sidecar birth tuple is durable. A dropped pre-release owner closes the
barrier and reaps the inert child; a dropped post-release owner leaves the sidecar adoptable.

Direct reconciliation now rejects ownership artifacts without their private intent and refuses to
retire a dead launch without both the fenced exit and whole-group `ESRCH`. Direct output validates
and stats its private ring before reading it, and output beyond one MiB remains bounded while its
cursor records the total and dropped bytes across restart. Cleanup ambiguity writes `Interrupted`,
retains the exact process identity, and appends a `recovery_required` event; identity is cleared only
after exact termination proves the group absent. Permission-denied and unknown group observations
remain blocking, while cleanup never signals an unrelated process group.

Darwin draft publication now uses the same blocked-child transaction as Linux: durable intent,
exact child identity, refreshed claim, durable release intent and receipt, then credentialed `gh`.
The Darwin-only predecessor shim was removed. Claim transfer now calls the authoritative retained
heartbeat receipt proof on Unix, including its descriptor-retained boundary revalidation.

### Fix-round evidence

- `cargo test -p autospec-cli --bin autospec darwin_ -- --test-threads=1` -> 14 passed.
- `cargo test -p autospec-cli --bin autospec draft_release -- --test-threads=1` -> 10 passed.
- `cargo test -p autospec-cli --bin autospec retained_bridge_predecessor_authority_is_exact_and_boundary_bound -- --test-threads=1` -> 1 passed.
- Task 5 heartbeat filters (`heartbeat_startup`, `startup_heartbeat_portable_unix`,
  `heartbeat_prior`, `heartbeat_quarantine`, `heartbeat_classify`, and
  `conductor_lease_takeover`) -> all commands exited 0.
- `cargo check -p autospec-cli --all-targets` -> exit 0 with the pre-existing Darwin test warnings.
- `git diff --check` -> exit 0.

`cargo run -q -p autospec-cli -- validate` completed all 143 repository checks: 132 passed
and 11 unrelated required checks failed. The Rust output-macro, claim CAS, conductor,
autonomy-guardrail, generated-YAML, Bash-syntax, and both autonomous contract checks passed.
The failures are confined to existing skill/install/explore policy suites
(`check_shared_script_install`, quality/ship/dogfood checks, install/parallel-dispatch,
explore discovery/contract checks, block expansion, and the autonomous phase-2 shell suite);
none names or exercises the Task 6 Rust ownership paths.

The strict clippy rerun remains blocked by pre-existing production/test lint debt outside the fix
round; it reports no error in the new Darwin supervisor, portability reconciliation, post-fork ring,
draft release tests, or retained-predecessor proof. The workspace run likewise reached the existing
macOS fixture failures described above (missing inherited Git hook paths, Linux sandbox assumptions,
and a fixed-port collision) while every new Darwin/draft/predecessor test passed.

## Review fix round 2

### RED evidence

- `cargo test -p autospec-cli --bin autospec darwin_ -- --test-threads=1` failed to
  compile because `DarwinOwnedGroup::cancel_unreleased` and the termination-uncertainty
  fault boundary did not exist. This reproduced the blocked-child cancellation gap before
  production changes.
- The new missing/tampered reservation fixtures target a live unrelated process group. Before
  the reconciliation fix, the existing code path could reach launch action without validating
  the attempt-ID reservation; the regression requires failure while the target and launch
  evidence remain untouched.
- The first actual-stall ambiguity run reached 18 passed / 1 failed because its nested `sleep`
  fixture allowed the test-owned sidecar leader to exit before test cleanup. Replacing it with a
  single-process inert loop isolated the intended proof-to-signal uncertainty boundary.

### Implementation

- An unreleased Darwin group now has a dedicated cancellation path: close the barrier, close the
  unused exec-status reader, reap the exact blocked supervisor, and require whole-group `ESRCH`.
  Persistence and event failures call this path directly. A failed release dispatches by retained
  barrier state, so a still-blocked launch is never sent through released-group signaling.
- Cancellation also fails closed across the former proof-to-signal race. A regression rewrites the
  persisted PGID to a live unrelated group, proves the actual blocked child is already reaped and
  its true group absent, and proves the unrelated reused PGID was never signaled.
- Darwin direct reconciliation validates the exact private attempt-ID reservation immediately
  after parsing the intent attempt ID and before reading or acting on the launch record.
- The crash-parent adoption test now emits 1,100,000 bytes, exits the original parent without
  destructors, adopts after restart, and verifies total/dropped cursor evidence plus a one-MiB ring.
- The cleanup ambiguity test now runs the real harness stall supervision path. A one-shot test fault
  fires after exact termination proof but before the signal; durable state is asserted
  `Interrupted` with exact identity retained and a `recovery_required` event.
- A separate forged identity points an exact owned leader at an unrelated reused PGID and proves
  termination rejects the mismatch while the unrelated group remains live.

### GREEN evidence

- `cargo test -p autospec-cli --bin autospec darwin_ -- --test-threads=1` -> 20 passed.
- `cargo test -p autospec-cli --bin autospec draft_release -- --test-threads=1` -> 10 passed.
- `cargo test -p autospec-cli --bin autospec darwin_reconciliation -- --test-threads=1` -> 4 passed.
- Task 5 filters (`heartbeat_startup`, `startup_heartbeat_portable_unix`, `heartbeat_prior`,
  `heartbeat_quarantine`, `heartbeat_classify`, `conductor_lease_takeover`) -> all six commands
  exited 0.
- `cargo check -p autospec-cli --all-targets` -> exit 0 with pre-existing warnings.
- `cargo check -p autospec-cli --all-targets --target x86_64-unknown-linux-gnu` -> exit 0 with
  pre-existing warnings.
- `git diff --check` -> exit 0.

The older generic direct-runner test remains non-runnable on Darwin because its fixture hardcodes
`/usr/bin/sleep`, which does not exist on macOS. Its failure occurs during executable
canonicalization before Task 6 direct supervision; the native Darwin direct reconciliation suite
above is green and owns the changed behavior.

The strict clippy rerun remains blocked by the same pre-existing unused/dead test helpers and
production lints in executor bridge, platform process, supervisor, autonomous, claim, and launch;
it reports no finding in the new Darwin cancellation, reservation, or round-2 regression code.

## Review fix round 3

### RED evidence

- A deterministic poll boundary published a complete `EXIT`/`DONE` record after the poller's
  initial exit read and killed the exact group before process observation. The regression did not
  compile before the boundary hook existed; the old implementation would have reused its stale
  `None` after observing `Dead` and rejected the completed executor.
- Missing and malformed post-death records were exercised at the same boundary to lock the
  fail-closed half of the contract.
- The process-global termination fault was consumed by a parallel cleanup thread, which failed
  with `injected Darwin termination uncertainty after exact proof` while the armed test thread did
  not. This proved cross-test fault injection was not identity-safe.
- Normal parallel Darwin runs exposed that group `SIGTERM` killed the identity-bearing supervisor
  before a TERM-ignoring user child exited. The subsequent exact proof for `SIGKILL` failed with
  `executor process group leader is dead`. A deterministic regression waits for the user child's
  TERM handler, signals the group, and asserts the exact supervisor remains observable.
- Parallel crash-adoption reached a transient `EPERM` group probe after the leader's death. A
  single post-death probe rejected before the required group-`ESRCH` boundary could settle.
- Parallel blocked-launch cancellation hung because concurrently forked supervisors can inherit
  another launch's barrier writer. A cloned-writer regression made the EOF-only cancellation path
  time out after 250 ms.

### Implementation

- `poll` now treats the pre-observation exit read only as live-record validation. After `Dead`, it
  reaps the exact child when available, waits for whole-group `ESRCH` (retrying transient probe
  uncertainty within the existing bounded grace), and only then rereads and consumes the durable
  `EXIT`/`DONE` record. Missing or malformed fences still fail closed.
- Darwin fault injection is thread-local, so only the test thread that arms the one-shot fault can
  consume it. The deterministic poll boundary is also thread-local and bound to the expected PID.
- The Darwin supervisor installs `SIGTERM` ignore before publishing readiness. Its forked user
  child restores the default `SIGTERM` disposition before `execve`, so user code retains ordinary
  TERM semantics while the exact group leader survives to authorize a separately re-proven
  `SIGKILL` escalation.
- Unreleased cancellation writes an explicit non-release byte before closing its barrier. The
  blocked supervisor exits immediately even if sibling forks inherited unrelated writer copies;
  user code is still never released or signaled.

### GREEN evidence

- Normal parallel `cargo test -p autospec-cli --bin autospec darwin_` -> 10 consecutive clean
  runs, 25 passed per run.
- `cargo test -p autospec-cli --bin autospec darwin_ -- --test-threads=1` -> 25 passed.
- `darwin_poll_consumes_exit_fence_published_after_initial_read` -> 20/20 repeated passes.
- `darwin_restart_adopts_durable_exit_after_original_parent_crash` -> 20/20 repeated passes.
- `darwin_unreleased_cancellation_` -> 3 passed, including inherited-writer cancellation.
- `darwin_term_ignoring_descendant_preserves_leader_for_exact_kill_escalation` -> 1 passed.
- `cargo test -p autospec-cli --bin autospec draft_release -- --test-threads=1` -> 10 passed.
- `cargo test -p autospec-cli --bin autospec darwin_reconciliation -- --test-threads=1` -> 4 passed.
- Task 5 filters (`heartbeat_startup`, `startup_heartbeat_portable_unix`, `heartbeat_prior`,
  `heartbeat_quarantine`, `heartbeat_classify`, `conductor_lease_takeover`) -> 4, 1, 3, 6, 9,
  and 13 tests passed respectively.
- `cargo check -p autospec-cli --all-targets` -> exit 0 with pre-existing warnings.
- `cargo check -p autospec-cli --all-targets --target x86_64-unknown-linux-gnu` -> exit 0 with
  pre-existing warnings.
- Targeted `rustfmt --check` and `git diff --check` -> exit 0.

## Review fix round 4

### RED evidence

- With the blocked supervisor's barrier read side closed, the cancellation marker returned
  `EPIPE` and the old early-return path left the exact child unreaped; `waitpid(WNOHANG)` still
  observed its `SIGKILL` status instead of `ECHILD`.
- Dropping an unreleased group while a cloned barrier writer remained open timed out after 250 ms,
  proving `Drop` still depended on EOF and could hang behind a descriptor inherited by another
  fork.
- The EINTR regression initially failed compilation because cancellation had no retryable marker
  boundary. The test retains a cloned writer, injects one `EINTR`, and therefore completes only if
  the same thread retries the explicit `C` marker rather than falling back to EOF.
- The first parallel EPIPE fixture exposed real fork semantics: a sibling supervisor inherited the
  original pipe reader, so that pipe could not deterministically produce EPIPE. The final fixture
  sends `C` to the real blocked supervisor, swaps in a separate pipe whose reader is explicitly
  closed, and then exercises the production EPIPE cleanup path without cross-test dependence.

### Implementation

- Unreleased cancellation now retries its one-byte `C` marker on `EINTR`. Every terminal marker
  outcome, including success, `EPIPE`, short write, and other errors, is retained while the local
  writer is closed and bounded cleanup reaps the exact child and requires group `ESRCH`.
- Marker failures are returned only after cleanup completes. If cleanup also fails, the result
  contains both the original marker diagnostic and the cleanup diagnostic; neither is discarded.
- Unreleased `Drop` uses the same explicit marker and bounded exact cleanup helper, so inherited
  writer descriptors cannot hold it in an unbounded EOF wait and user code is never released.
- The one-shot EINTR injection is thread-local, preserving the parallel-test isolation established
  in round 3.

### GREEN evidence

- Normal parallel `cargo test -p autospec-cli --bin autospec darwin_` -> 10 consecutive clean
  runs, 29 passed per run.
- `cargo test -p autospec-cli --bin autospec darwin_ -- --test-threads=1` -> 29 passed.
- EPIPE cleanup, inherited-writer Drop, and EINTR retry exact tests -> 20/20 passes each.
- `cargo test -p autospec-cli --bin autospec darwin_unreleased_ -- --test-threads=1` -> 7 passed.
- `cargo test -p autospec-cli --bin autospec draft_release -- --test-threads=1` -> 10 passed.
- `cargo test -p autospec-cli --bin autospec darwin_reconciliation -- --test-threads=1` -> 4 passed.
- Task 5 filters (`heartbeat_startup`, `startup_heartbeat_portable_unix`, `heartbeat_prior`,
  `heartbeat_quarantine`, `heartbeat_classify`, `conductor_lease_takeover`) -> 4, 1, 3, 6, 9,
  and 13 tests passed respectively.
- `cargo check -p autospec-cli --all-targets` and the `x86_64-unknown-linux-gnu` cross-target
  variant -> exit 0 with pre-existing warnings.
- Targeted `rustfmt --check` and `git diff --check` -> exit 0.
