# Rust Core Runtime Consolidation

## Validate Wrapper Fallback

`scripts/validate.sh` now attempts `autospec validate` before entering the
legacy shell body. The Rust command re-enters the shell with
`AUTOSPEC_FORCE_LEGACY_SHELL=1` while validation logic is still being ported.

The temporary fallback is tied to epic #1861. Remove it only after the remaining
runtime consolidation issues have moved the selected validation paths into Rust
and the shell wrapper is reduced to a thin compatibility entrypoint.

## Rust Context Monitor API

`crates/autospec-core/src/context/mod.rs` exposes the side-effect-free Rust
parity surface for the Python context monitor engine. The public API is:

- `ContextMonitorEngine` — owns the threshold state machine.
- `ContextState` — reports `Normal`, `Compacted`, or `Rolled` state.
- `ContextAction` — describes caller-executed actions by kind and optional
  payload.

The engine preserves the Python rollover contract from
`packages/autospec_context_monitor/autospec_context_monitor/engine.py`:
usage at or above 50% emits `compact` first, a later compacted reading at or
above 80% emits `handoff`, `clear`, and `resume` in that order, and usage below
30% resets `Compacted` or `Rolled` state back to `Normal` via a `noop` action
with a diagnostic payload. Rust callers must execute returned actions outside
the core crate; the core module only classifies usage and mutates local state.

## Watchdog Linked PR Liveness

`autospec-watchdog.sh` and `list-ready-issues.sh` treat linked open PRs as
active ownership for their closing issue. A PR body that uses a GitHub closing
keyword such as `Fixes #1859` keeps that issue out of stale-claim reclaim and
out of the ready queue while the PR is still open. Nonterminal check rows,
missing check rows on a newly opened PR, unavailable PR evidence, or malformed
PR evidence all fail closed so the issue is not reselected before PR
finalization. This preserves the handoff from worker ownership to PR/CI
ownership without introducing a new queue state.
