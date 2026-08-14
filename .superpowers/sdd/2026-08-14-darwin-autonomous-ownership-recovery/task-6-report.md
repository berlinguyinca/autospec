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
