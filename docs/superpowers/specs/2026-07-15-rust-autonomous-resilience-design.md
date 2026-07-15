# Rust Autonomous Resilience Design

## Purpose

Move autonomous monitor resilience authority from the stateful shell helpers into
the Rust control plane without breaking an in-flight operator that still has a
legacy state file. This is the second child of #2076, following the merged typed
executor-result and lifecycle-admission work (#2077 and #2079).

## Decision

Use a dual-read, canonical-write adapter. Rust reads the established resilience,
failure, and spend-ledger layouts in their documented compatibility order, but
every new state write uses the unambiguous `owner__repo` slug. The existing
`autonomous-operator/owner_repo` directory continues to own only the Rust
lifecycle process metadata; it is not a resilience-state destination.

This is preferred over either a one-time state migration or a permanent
multi-writer arrangement:

1. A one-time migration can strand a live monitor between reads and writes.
2. Permanent shell/Rust co-ownership would preserve the shell authority the
   epic is retiring.
3. Dual-read plus a single Rust writer lets the next child delete shell code
   after a bounded compatibility release.

## Boundaries

Pure policy lives in `autospec-core`; it is deterministic and has no filesystem,
process, environment, or GitHub dependency. The CLI owns the compatibility
decoder, path resolution, atomic persistence, and same-host PID probe.

The migration covers these records:

| Record | Read order | New write path |
| --- | --- | --- |
| Resilience state | `autonomous/owner__repo/state.json`, then `_`, then `-` | `autonomous/owner__repo/state.json` |
| Per-issue failures | `autonomous/owner__repo/issues/N.json`, then `_`, then `-` | `autonomous/owner__repo/issues/N.json` |
| Spend ledger | `autonomous-spend/owner__repo/spend.json`, then `_`, then `-` | `autonomous-spend/owner__repo/spend.json` |
| Operator lifecycle | existing `autonomous-operator/owner_repo` only | unchanged |

The compatibility reader validates `repo` before accepting a record. A malformed
JSON value or a different repository is an explicit reject; it is never treated
as a missing record. Reads prefer the first valid canonical location and do not
merge data from two layouts.

## Policy

The core policy introduces a conductor lease that is deliberately separate from
the GitHub-backed issue claim lease. A conductor lease contains the current
status, heartbeat age, holder host and PID, and whether a same-host PID is still
alive. Its decisions are:

- Reclaim immediately when the holder is on this host and its PID is dead.
- Reclaim when no heartbeat is present.
- Reclaim at an age of **at least 10,800 seconds** for any status.
- Reclaim at an age of **at least 300 seconds** when status is `claimed`.
- Otherwise report the lease held.

Per-issue failures remain monotonic because that is the existing behavior: an
explicit failure count can be persisted and the admission policy rejects at the
cap (default three); successful outcomes do not silently reset it. Usage caps
are evaluated before issue caps. A zero cap disables that cap. These typed
results feed the lifecycle decision before foreground dispatch.

## CLI Adapter and Commands

`autospec autonomous resilience decide` is a narrow diagnostic and test surface
for path resolution and policy. It accepts an explicit repository and fixture
state root, emits stable JSON for run, held, reclaimed, malformed, foreign,
failure-cap, and budget/usage-cap outcomes, and performs no shell invocation.

`start`, `restart`, `status`, and `run-foreground` call the same adapter. Start
and restart inspect the lease before launch. Status reports canonical-or-fallback
state without changing it. Foreground evaluates failure and usage admission
before the shell-independent typed conductor can dispatch work. Persistence uses
same-directory uniquely named temporary files followed by rename, preserving the
atomic-write discipline introduced for lifecycle records.

## Error Handling

Malformed state, a foreign scope, and non-numeric required fields yield a stable
reject JSON and a nonzero exit before any canonical write. A held lease and a
parked cap are expected operational outcomes and use the existing lifecycle
non-run exit class. I/O failures remain diagnostic errors and never fall back to
the shell helper.

## Testing and Proof

Core tests prove exact lease boundaries, same-host PID behavior supplied by the
adapter, monotonic failure-cap behavior, and usage-before-issue-cap precedence.
CLI integration tests use temporary state roots and cover canonical precedence,
single-underscore and hyphen fallback, malformed and foreign records, atomic
write destinations, and start/foreground admission. The final local proof is:

```bash
cargo test -p autospec-cli --test autonomous_resilience_commands
cargo test --workspace --quiet
cargo fmt --all --check
cargo clippy --workspace -- -D warnings
cargo run -q -p autospec-cli -- validate --fast
git diff --check
```

## Non-goals

This slice does not delete the shell waterfall, Tier-1 drain bridge, generic
continuation loop, legacy versioned canaries, or installer fallback. Those have
additional consumers. It only establishes the Rust parity and compatibility
evidence required for the dedicated deletion child.
