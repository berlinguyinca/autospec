# Rust Foreground Conductor Design

## Goal

Make `autospec autonomous run-foreground` a Rust-owned control-plane command
with no shell, script, or command-string conductor fallback.

## Context

The core conductor now owns typed continuation decisions. The foreground route
still has two incompatible authorities: it invokes `autospec-autonomous.sh`
through `bash`, and detached `start`/`restart` create a conductor command string
that `sh -c` launches. Neither route may remain responsible for foreground
control flow.

The present cutover must also remain honest about implementation execution. The
legacy drain starts an LLM-backed implementation workflow, but no typed Rust
implementation-agent protocol exists yet. Reporting a command exit as an issue
completion would requeue unimplemented work or falsely release its lease.

## Design

`queue.rs` will expose crate-visible typed helpers for bounded safety review and
ready-queue planning. `autonomous.rs` will use those helpers, the core
`ConductorState`, and the existing Rust claim CLI API to run one repository
cycle:

1. Run mainline health admission. A failed admission exits before queue work.
2. Scan the Rust ready queue. An empty repository snapshot transitions to
   `ALL_DONE`; a nonempty but blocked queue remains nonterminal.
3. Run one bounded Rust safety-review pass, rescan, and select the first typed
   batch item when one is ready.
4. Acquire the existing GitHub claim lease for that issue.
5. Persist the selected Rust conductor state below the existing
   repository-scoped autonomous state directory.
6. Launch one direct child using an `ExecutorRequest` with
   `program = current_exe` and an explicit argument vector for
   `autospec autonomous executor-result`.
7. The child produces the structured, Rust-only deferred outcome. The parent
   records that outcome in `ConductorState`, persists it, and then uses the
   existing typed claim reconciliation path. The selected issue remains paused
   and claimed rather than being reported as completed.

`executor-result` is intentionally an internal protocol command for this
slice. It neither runs an agent nor accepts a shell command. A later issue must
introduce a real, typed implementation-executor contract before foreground
execution can move a successful implementation through release/merge gates.

Detached `start` and `restart` will construct the foreground child as a typed
program plus argument vector and spawn it directly. Monitor and supervisor
remain separate companion behavior in this slice; their compatibility command
strings do not own foreground work selection or dispatch.

## Persistence and recovery

The foreground state file is strict conductor JSON at
`<autonomous-state-scope>/foreground-conductor-<scope-key>.json`, where the
scope key partitions repository and explicit slice runs. A fresh foreground
invocation parses this file before replacing it. A paused selected issue stays
selected, with its explicit deferred outcome and lease intact, until a later
explicit recovery protocol is added. This prevents accidental claim loss or
false success after a process restart.

## Boundaries

- No `bash`, `sh -c`, script path, environment command override, or command
  string may be used to launch the foreground conductor.
- No external implementation agent is launched and no outcome is treated as an
  implementation success in this slice.
- Queue safety review, readiness, claims, and reconciliation remain existing
  Rust-owned APIs; this change does not duplicate their policies.
- `ALL_DONE` remains a ready-queue observation, not an authorization to stop
  the broader discovery/autonomy waterfall.

## Tests

The CLI integration suite will use fake `gh` and a real compiled Rust
`autospec` child. It proves health blocks before dispatch, a typed selected
issue produces an argument-vector child request and a persisted paused state,
and a fresh process keeps that selected issue paused. Static source assertions
cover the absence of script/backend fallbacks from the foreground path and the
typed detached conductor spawn.

## Follow-on

A separate retirement issue will replace the remaining legacy autonomous
waterfall and introduce the real typed implementation-executor protocol. It
must not restore a shell authority while doing so.
