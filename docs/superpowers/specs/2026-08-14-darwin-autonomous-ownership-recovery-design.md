# Darwin Autonomous Ownership and Recovery Design

## Purpose

Autospec autonomous mode must complete work safely on macOS instead of entering a deterministic claim-failure loop. The repair also closes two platform-independent recovery gaps: `autonomous resume --dry-run` currently mutates state, and an expired `heartbeat-pending:none` claim can be rejected by lifecycle admission before the existing authoritative recovery logic can run.

The result must preserve the Linux ownership contract, introduce an equivalent Darwin contract without shell-parsed process identity, recover interrupted claims through existing compare-and-swap authority, and keep one accountability epic accurate throughout recovery.

## Current failure

On macOS, `write_startup_heartbeat` returns `heartbeat publisher unavailable` after the authoritative claim generation has already been created. The claim is restored to `claimed` with step `heartbeat-pending:none`, and the conductor defers it.

After the default 300-second rescan interval, lifecycle preflight classifies that claim as stale and exits before `acquire_for_conductor` reaches `recover_authoritative_stale_startup`. The terminal conductor path then closes the accountability epic. The supervisor correctly refuses to adopt a closed terminal epic under its active-only policy, leaving the run stopped.

Separately, `autonomous resume` is routed through `restart`, whose implementation does not honor `Options::dry_run`. A purported dry run can therefore stop processes, acquire lifecycle ownership, reopen and edit the epic, write local state, and spawn new companions.

## Goals

- Make startup-heartbeat publication, inspection, liveness classification, and safe retirement operational on Darwin.
- Preserve Linux pidfd, `/proc`, subreaper, and descriptor-relative behavior unchanged.
- Give Darwin process ownership an exact boot-and-birth identity so PID reuse cannot transfer authority.
- Supervise Darwin executor process groups without leaking descendants after interruption.
- Route only `claimed` plus exact `heartbeat-pending:none` records through authoritative recovery before stale lifecycle rejection.
- Recover eligible stranded claims during crash resume even when the failed acquisition never moved the issue to `in-progress-by-bot`.
- Guarantee every `--dry-run` launch or resume path is read-only.
- Preserve and extend the same accountability epic across a recovered run.

## Non-goals

- Weakening claim compare-and-swap checks, generation matching, filesystem ownership, or symlink defenses.
- Replacing Linux ownership with a lowest-common-denominator implementation.
- Treating PID presence alone as proof of ownership.
- Adding a third-party runtime dependency or requiring a Linux VM for normal macOS use.
- Reclaiming fresh, live, malformed, cross-repository, wrong-branch, or otherwise ambiguous claims.

## Architecture

### Platform process identity

Introduce a narrow internal platform adapter that returns a stable process identity and liveness result:

```text
ProcessIdentity {
    boot_id: String,
    start_identity: String,
}

observe_process(pid, expected_identity) -> Live | Dead | Unknown
```

Linux keeps its existing `/proc` and pidfd implementation. Darwin derives the boot identity from the kernel boot-time record and the process start identity from native process metadata. Both values are serialized canonically. Observation returns `Live` only when the PID exists and the observed boot/start tuple exactly matches; a mismatch is `Unknown`, not `Dead`, so PID reuse never authorizes cleanup.

The adapter must use native system calls through existing system crates or direct platform FFI. It must not execute `ps`, parse localized timestamps, or introduce a new dependency.

### Portable Unix heartbeat publication

The existing Unix descriptor primitives become the common directory-resolution layer. Linux continues to use `openat2` with its current resolve flags. Darwin uses the existing single-component `openat` path with `O_NOFOLLOW`, then verifies parent identity, opened directory identity, and name binding before and after the open.

Heartbeat publication remains a transaction:

1. Validate private root and repository directories.
2. Open the destination relative to trusted directory descriptors.
3. Reject symlinks, non-regular files, foreign ownership, or permissive modes.
4. Write a private temporary file containing the exact claim generation and process identity.
5. Synchronize file contents, atomically publish, synchronize the directory, and verify the published identity.
6. Persist the acquisition receipt only after publication is durable.

Darwin recovery uses the same document parser, nonce, claim-generation checks, and quarantine format. Liveness is delegated to the platform adapter. `Unknown` evidence remains blocking.

### Darwin executor ownership

The executor bridge receives a Darwin supervisor alongside the existing Linux supervisor. It creates a dedicated process group before executing the harness, records the leader's exact boot/start identity, and observes termination through native process events. Cleanup signals the owned process group only after the recorded leader identity still matches.

Darwin has no Linux subreaper equivalent. Descendant containment therefore relies on an isolated process group established before the harness can spawn children. Completion requires the leader to exit and the owned process group to contain no live members. If membership or identity cannot be proven, the bridge fails closed and retains recovery evidence rather than declaring completion.

Linux-specific pidfd and subreaper code remains behind the existing Linux implementation boundary. Shared bridge code consumes only the platform ownership interface.

### Claim recovery ordering

Lifecycle evidence treats only this exact incomplete-acquisition record specially:

```text
state = claimed
step = heartbeat-pending:none
```

For that record, lifecycle admission delegates ownership resolution to `acquire_for_conductor` instead of rejecting it as stale first. The acquisition path remains authoritative:

- Before the recovery timeout, a foreign pending generation is deferred without mutation.
- After the timeout, existing CAS recovery verifies repository, issue, branch, generation, heartbeat identity, and liveness.
- Proven-dead evidence is quarantined, the exact generation advances to `available/stale_startup_recovered`, labels are normalized, and acquisition retries.
- Fresh, live, mismatched, malformed, or ambiguous evidence remains untouched.

All other stale claim states retain current lifecycle rejection behavior.

### Resume and supervision

Crash resume scans both `in-progress-by-bot` issues and `auto-implement` issues with authoritative nonterminal claim state. A stale pending candidate is passed to `claim state recover-stale-startup` before relaunch. Dry-run reports the proposed recovery and relaunch but invokes neither.

The foreground conductor remains the primary automatic recovery owner. Once recovery ordering is corrected, an ordinary continuous cycle recovers the pending generation instead of terminalizing. The supervisor therefore sees an active accountability segment rather than a closed terminal mismatch. Explicit resume continues to reopen a verified stopped epic and append a new journal segment.

### Strict dry-run boundary

Launch preview is separated from launch execution and used by both `start` and `resume`. The preview may parse options and construct commands, but must return before:

- repository lifecycle acquisition or release;
- process termination or spawn;
- stop-sentinel mutation;
- accountability binding, reopening, projection, or journal writes;
- launch/state/heartbeat writes;
- claim recovery or label mutation.

JSON output identifies the requested subcommand rather than always reporting `start`.

## Accountability

Recovery appends short events to the existing run epic:

- the failed heartbeat publication and deferred claim;
- the evidence that made recovery safe;
- the recovered claim generation;
- the resumed implementation attempt;
- the final PR and verification result.

The epic's overview and Mermaid flow are regenerated from the durable journal. A dry run emits no accountability event because it has not changed the run.

## Failure handling

- Darwin platform calls returning incomplete or inconsistent identity produce `Unknown` and retain ownership evidence.
- Filesystem identity drift, symlinks, foreign ownership, or unsafe modes fail before publication or cleanup.
- A lost claim CAS leaves the winning generation untouched and returns to queue scanning.
- Recovery failure is retryable while the conductor remains active; it does not terminalize solely because the pending claim crossed the timeout.
- Executor cleanup that cannot prove group ownership records a recoverable failure and never signals an unverified process.
- Unsupported non-Unix platforms fail before creating an epic or claim.

## Testing

### Platform-independent regressions

- `autonomous resume --epic N --dry-run` performs zero local, process, GitHub, accountability, or claim mutations.
- A stale `heartbeat-pending:none` record reaches authoritative recovery from the normal Scan path.
- A fresh foreign pending claim defers without mutation.
- Ordinary stale claim states still fail lifecycle admission.
- Crash resume includes eligible `auto-implement` claims and remains read-only under `--dry-run`.

### Darwin regressions

- Current-process boot/start identity is stable across repeated reads.
- A nonexistent PID is dead; a reused or mismatched identity is unknown.
- Heartbeat publication creates only private regular files and survives read-back verification.
- Symlinked roots, repository directories, targets, and replacement races are rejected.
- A dead exact heartbeat generation can be quarantined and recovered; live and ambiguous generations cannot.
- Executor process-group cleanup terminates owned descendants and refuses a mismatched leader identity.

### Linux regressions

- Existing pidfd, `/proc`, subreaper, heartbeat, claim, and executor suites remain unchanged and green.
- Linux CI continues running the full workspace, clippy, and validation gates.

### End-to-end canary

After merge and generation-based reinstall, resume the existing accountability epic for issue `#2686`. Evidence must show:

- the issue is claimed without `heartbeat_write_failed`;
- implementation reaches a branch and PR;
- the closeout report and required tests are recorded;
- the PR is merged only through existing gates;
- the issue closes and the epic reaches a truthful terminal state;
- no stale claim, companion process, or recovery mismatch remains.

## Rollout

1. Land strict dry-run and recovery-order regressions.
2. Land portable heartbeat publication and Darwin identity.
3. Land Darwin executor process-group ownership.
4. Run macOS focused tests and compile the full workspace.
5. Run Linux CI and all repository validation scripts.
6. Rebuild and atomically install a new runtime generation from merged `main`.
7. Execute the `#2686` canary and record its evidence in the existing epic.

If Darwin ownership misses any required proof, release remains blocked on that platform while Linux behavior stays available and unchanged.
