# Portable Autonomous Runtime Design

## Goal

Make `autospec autonomous` claim and execute work safely on Linux, macOS,
Windows, and FreeBSD without weakening Linux's existing pidfd ownership model.

## Problem

The current non-Linux build contains deliberate fail-closed stubs at three
consecutive runtime boundaries:

1. released predecessor heartbeat retirement;
2. startup heartbeat publication and process identity;
3. executor supervision.

The first stub leaves a macOS conductor retrying `claim_deferred` forever. If
that stub is bypassed, the next two stubs still prevent implementation. A valid
repair must therefore cover the complete claim-to-executor path.

## Safety invariants

- GitHub claim compare-and-swap state is the cross-machine ownership authority.
- A local heartbeat is recovery metadata, never authority to terminate a
  process from its numeric PID alone.
- A live supervisor may terminate only a process tree it owns through an OS
  resource captured at launch.
- Recovered state with ambiguous process identity fails closed for signalling,
  but exact remotely released claim state may retire its matching heartbeat.
- Linux keeps its existing pidfd, procfs, subreaper, and descriptor-relative
  filesystem implementation unchanged.
- Heartbeat state remains repository-scoped and user-private.
- Every platform backend must compile and have behavior tests in CI.

## Architecture

### 1. Common ownership contract

Introduce a narrow executor ownership interface with these operations:

- launch a child in an isolated process container;
- observe whether the owned child has exited;
- wait and collect its exit result;
- terminate the owned process tree;
- expose immutable launch identity for durable receipts.

The interface is implemented by target-specific modules. Callers retain the
existing durable invocation, receipt, claim-renewal, and reconciliation state
machines; only process ownership moves behind the interface.

### 2. Linux backend

Retain the current pidfd and subreaper implementation. No portable fallback is
selected on Linux, including Linux systems where pidfds are unavailable. Those
systems continue to fail closed rather than silently accepting weaker cleanup.

### 3. macOS and FreeBSD backend

Spawn the harness as leader of a dedicated process group using stable
`CommandExt::process_group(0)`. Retain the `Child` handle as live ownership and
use `try_wait`/`wait` for exit and reaping. Termination targets the owned process
group, followed by `Child::wait`.

Durable recovery records include PID, process-group ID, a random launch nonce,
and platform process creation identity. Recovery may classify an old record as
stale after creation identity no longer matches. It may not signal an arbitrary
recovered PID because macOS and FreeBSD cannot close the validation-to-signal
PID-reuse race. A recovered live sidecar remains the owner of its child tree.

### 4. Windows backend

Launch the harness suspended, create a Job Object, enable
`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, assign the process, then resume its main
thread. Retain the process and job handles. Waiting uses the process handle;
tree termination uses the Job Object. Durable identity includes PID, creation
`FILETIME`, and launch nonce.

The FFI surface is target-gated and limited to process creation/identity and Job
Object operations. No dependency is added solely to wrap APIs already exposed
by the Windows SDK bindings available to Rust targets.

### 5. Portable heartbeat store

Heartbeat documents use immutable claim identity: repository, issue, worker,
branch, claim ID, launch nonce, PID, platform creation identity, and optional
session ID.

Publication creates a random temporary file in the same private directory,
writes and syncs the document, then atomically renames it into place. Existing
targets are accepted only when their parsed immutable generation matches.
Intermediate symlinks/reparse points and non-regular final targets are rejected.
Unix directories use mode `0700` and files `0600`; Windows uses a user-private
state root and rejects reparse-point traversal.

Retirement requires both:

1. an exact local heartbeat generation match; and
2. an exact authoritative remote claim record in `released` state.

Retirement archives or removes only that matching heartbeat. It never signals
the recorded PID. Linux may additionally apply its existing stronger pidfd
liveness checks.

### 6. Recovery behavior

While a platform supervisor remains alive, its owned process resource is the
sole termination authority. After a supervisor crash:

- a still-running sidecar continues supervising and publishing receipts;
- a terminal receipt is reconciled without replay;
- an ambiguous orphan is quarantined and never killed by PID;
- an exact released claim permits heartbeat retirement and successor claim.

This preserves at-most-one authoritative mutation lane without requiring every
OS to emulate pidfds.

## Alternatives rejected

### Skip heartbeats outside Linux

This avoids the immediate claim error but removes local progress evidence and
leaves stale records unrecoverable. It also does not solve executor admission.

### Route every non-Linux run through the legacy shell drain

This restores some macOS behavior quickly but makes Windows dependent on a Unix
shell/process-group emulation and bypasses the Rust executor's durable receipt
model. It is a recovery surface, not the portable ownership architecture.

### Validate then signal an arbitrary PID

Creation timestamps reduce accidental PID reuse but do not close the Unix
check-to-signal race. This is insufficient for destructive process-tree cleanup.

## Testing

Development follows red-green TDD.

- Claim tests reproduce a released predecessor on non-Linux and require the
  successor to advance without `predecessor_heartbeat_retirement_failed`.
- Heartbeat tests cover exact publication, idempotent replay, conflicting
  generation rejection, symlink/reparse rejection, and exact released
  retirement.
- Supervisor contract tests cover successful exit, non-zero exit, stall
  termination, descendant cleanup, receipt recovery, and refusal to signal an
  unowned recovered PID.
- Linux tests preserve existing pidfd behavior.
- macOS tests exercise the process-group backend.
- Windows tests exercise Job Object ownership.
- FreeBSD CI runs through `vmactions/freebsd-vm@v1`, builds the CLI, and executes
  the process-group and portable pure-state tests inside the VM.
- CI adds explicit Linux, macOS, Windows, and FreeBSD gates. A platform is not
  declared supported from conditional compilation alone.

## Delivery slices

1. Portable heartbeat identity, publication, and released retirement.
2. Target-specific executor ownership interface and Unix process-group backend.
3. Windows Job Object backend.
4. Four-platform CI and end-to-end conductor regression.

Each slice is independently tested and committed. No slice changes the Linux
pidfd implementation unless a regression test demonstrates a shared-contract
defect.
