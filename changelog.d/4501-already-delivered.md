# 4501 — an empty diff against the base is delivered, not pending

A patch whose changes are already in the base is residue, not a candidate.
The pass now:

- reports it as its own category (`delivered=N` on the selection line,
  `  DELIVERED #N (patch is empty against the base)` per patch, a
  `delivered` array in `--json`); it is never offered and never gated;
- detects it with a read-only `git apply --reverse --check` in one shared
  worktree at the base (a check that fails reads as *not delivered*);
- under `--apply`, archives it to `out/issue-N/superseded/` (archival,
  never deletion) and releases its queue entry, so the next pass stops
  enumerating it;
- opens every PR with `Closes #<issue>.` in the body — the tracker closes
  on a body keyword, and a number in a title is not one. This is what
  stops the residue from accumulating: eighteen delivered issues sat open
  because the old PRs named the issue in the title only, and every pass
  since spent a full gate per issue on "no change."

The pass outcome line carries the counter (`... deferred=N delivered=N`),
and the reconciliation accounts it: `converted + held + skipped +
deferred + delivered <= examined`.
