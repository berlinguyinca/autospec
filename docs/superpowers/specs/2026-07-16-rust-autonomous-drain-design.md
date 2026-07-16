# Rust Autonomous Drain Design

**Issue:** #1826
**Parent:** #2076 / #1861
**Status:** approved by the standing Rust-cutover execution directive

## Goal

Move the autonomous Tier-1 drain watchdog from
`scripts/autospec-autonomous-run-drain.sh` into the Rust control plane so a
quiet but healthy child is never killed while its heartbeat or GitHub state is
advancing.

## Scope

Add `autospec autonomous drain`, which launches the fixed `omx exec ...
$autospec-run` integration without `bash -c`, observes the child and
repository-scoped progress, and terminates only a still-live child that has
exceeded the configured stall window with no local or external progress.

The command must:

1. Model drain decisions as pure Rust values in `autospec-core`.
2. Treat child completion as authoritative before considering a timeout.
3. Reset the idle deadline for child output, changed progress artifacts,
   repository-scoped heartbeats, or changed GitHub issue/PR observations.
4. Emit a single structured warning when progress is external-only and stdout
   is quiet.
5. Persist the most recent drain observation below the existing repository
   operator state so `status` and later timeline work have a typed record.
6. Use direct subprocess arguments for `omx`, `gh`, and process termination;
   no shell command string, shell parser, or legacy drain fallback is allowed.

## Alternatives considered

1. Patch the existing shell watchdog. Rejected: it extends R1 shell authority
   and leaves the deletion blocker unresolved.
2. Disable timeout termination. Rejected: it avoids a false positive by
   allowing genuinely wedged children to consume the conductor indefinitely.
3. Poll GitHub every watchdog interval. Rejected: it creates unnecessary
   network/API load; Rust observes local progress continuously and asks GitHub
   only at an otherwise-stalled timeout boundary.

## Architecture

`autospec_core::autonomous::drain` owns a pure `DrainObservation` and
`DrainDecision` contract. A live child with any local progress yields `Wait`;
external-only progress yields `WarnExternalProgress`; a completed child yields
`Complete`; only a still-live, elapsed child without progress yields
`TerminateStalled`.

`autospec-cli` owns the adapters. It starts the fixed `omx` integration with
`Command`, drains child stdout/stderr on reader threads, samples the same
repository-scoped heartbeat layouts already used by `timeline`, and records one
GitHub baseline before polling. It re-snapshots GitHub only when a local timeout
would otherwise occur. A changed snapshot resets the timer and emits a warning.
Before termination it performs a final `try_wait`; a completed child returns its
real exit status without a kill. Child and GitHub observer processes each run in
their own process group so a timeout or abandoned observation cannot leave a
descendant holding the drain pipes. GitHub reads are bounded and continuously
reconcile the supervised child, so a hung API command cannot hide completion.

The command writes `drain-observation.json` under the repository's existing
autonomous operator directory. The record contains schema version, timestamp,
child state, source of latest progress, warning state, and the final decision.
It contains no lease token or raw child output.

## Error handling

- Invalid timeout/poll values and malformed repository scope are diagnostics
  with exit `2` and no JSON decision.
- Failure or timeout while inspecting GitHub is non-progress, not evidence that
  GitHub is unchanged; the command can still wait on local progress or
  terminate only after the normal timeout.
- A child exit is returned as that child's code. It is never recast as a
  watchdog stall.
- Process termination is attempted only after the pure policy decision and a
  final child-exit reconciliation. Termination failure is a diagnostic.

## Boundaries

- `scripts/autospec-autonomous-run-drain.sh` is R1 historical authority. This
  slice does not call, patch, or install it.
- `scripts/autospec-autonomous.sh` and `scripts/lib/autospec-loop.sh` remain
  unchanged until the later #2076 deletion/handoff child redirects live
  launchers to `autospec autonomous drain`.
- #1602 owns parsing `.autospec/autonomous.yml`; this command accepts explicit
  typed options now and must not duplicate YAML parsing.
- #1872 owns dry-waterfall explanation and ideation escalation; this command
  records only drain progress.
- #1697 owns converting a premerge failure into quarantine-and-continue; this
  command treats every child exit as a child exit, never as a stall.

## Tests

Core tests cover decision precedence and timer resets. CLI integration tests
use a fake `omx` and fake `gh` to prove:

1. quiet validation with heartbeat or GitHub progress completes successfully
   without a termination;
2. a child that exits during final reconciliation is returned untouched;
3. a silent, live child past the timeout is terminated once and records a
   stalled decision;
4. external-only progress emits the structured warning and persists its
   observation.

The primary local command is:

```bash
cargo test -p autospec-cli --test autonomous_drain_commands
```

## Completion criteria

- `autospec autonomous drain` is Rust-owned and has no `sh`, `bash`, or legacy
  drain-script invocation path.
- The #1826 false-stall scenario is covered by Rust integration tests.
- `cargo fmt --all --check`, `cargo clippy --workspace -- -D warnings`,
  `cargo test --workspace --quiet`, and `autospec validate --fast` pass.
- The later #2076 deletion child can replace the shell launch wiring without
  reimplementing watchdog behavior.
