# Conversion-pass guard (issue #3654)

A conversion pass runs inside a git worktree and memoizes its verdict in a
durable memo keyed on (patch hash, base sha). The original worktree lock
excluded only other converters, so a human operator could reset, clean, or
prune the worktree while a pass was in flight. The corrupted result — the
`HELD: build error` verdict — was memoized as fact and poisoned every later
pass over the same input.

`autospec_core::convert_pass` closes the gap in four parts, one per
acceptance criterion.

## 1. The worktree marker (`marker`)

While a conversion pass is in flight, the pass holds a marker file at
`<worktree>/.autospec/convert-pass` naming its holder and PID:

```json
{ "holder": "operator@host", "pid": 4242, "since_epoch_secs": 1760000000 }
```

- Acquisition is atomic (`O_EXCL`): two passes racing on one worktree cannot
  both hold it; the loser gets `MarkerError::Held` naming the winner.
- Release is RAII (`Marker`'s `Drop`) and removes only a marker that still
  names the releasing process, so a stale marker left by a killed holder is
  never deleted by someone else.
- A marker that exists but cannot be parsed is **held, full stop**: the guard
  fails closed rather than failing open.

API: `Marker::acquire`, `Marker::read`, `Marker::held`, `Marker::path`,
`MarkerRecord`, `MarkerError`.

## 2. Maintenance refusal (`maintenance`)

The shared maintenance helpers check the marker and refuse while a pass is
in flight, naming the holder and PID:

- `reset(root)` — a `reset --hard HEAD` of the worktree
- `clean(root)` — `git clean -fd`
- `prune(root)` — `git worktree prune`
- `maintain(root, action)` — the generic entry point over `MaintenanceAction`

A held worktree yields `MaintenanceError::Held { action, holder, pid }`.
An unreadable marker yields `MaintenanceError::MarkerUnreadable` — refusing,
never running, because failing open is how the corrupted verdict got made.

## 3. Conditional verdict commit (`run_pass`)

`run_pass(worktree, holder, key, memo, op)` runs the conversion under the
marker and commits the verdict to the `VerdictMemo` **only when `op`
returns `Ok`**. A run that errored, or panicked, was interrupted by
something — possibly human maintenance — and says nothing trustworthy about
the target; it commits nothing. The marker is held for the whole pass and
released on every exit, panic included. A `Failed { reason }` verdict from a
run that completed normally *is* committed: the run finished, so its
negative verdict is trustworthy.

`VerdictMemo` stores verdicts as JSON at a caller-chosen path, keyed by
`MemoKey { patch_hash, base_sha }`, written atomically (tmp file + rename).

## 4. Cheap logged invalidation

`VerdictMemo::invalidate(key, &mut |line| ...)` drops one cached verdict,
persists the drop, and writes exactly one log line naming the dropped key —
the operator's cheap way to throw out a suspect entry. Invalidating an
absent key is a quiet no-op. The library crate never prints on its own;
the caller supplies the log sink (stderr writer, journal, ...).

## Usage

```rust
use autospec_core::convert_pass::{run_pass, MemoKey, Verdict, VerdictMemo};

let memo = VerdictMemo::open(path::Path::new("/var/lib/autospec/convert-memo.json"))?;
let key = MemoKey::new(&patch_hash, &base_sha);
match run_pass(&worktree, "operator@host", &key, &mut memo, || convert()) {
    Ok(verdict) => { /* verdict is now durable under `key` */ }
    Err(reason) => { /* interrupted: nothing was memoized */ }
}
```
