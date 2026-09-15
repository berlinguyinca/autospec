# A terminal disposition for patches that can never convert (#4637)

A produced patch suppresses re-dispatch of its issue for as long as it sits
on disk (the dispatch guard's "already produced" test is the file's
presence). That is right while the patch is convertible; it is the deadlock
while it is not — measured at 193 queue entries (78 open issues after the
closed-entry correction) and 27 idle agent slots, every one of them a
patch conflict-bound in a shape the pass will never merge.

- A conflict whose **every** conflicted file is a shape the pass will never
  merge (unclassifiable, or regenerate-from-source) is now terminal:
  `convert --apply` invalidates the patch instead of holding it. The
  `INVALIDATED #N: <reason>` line is printed at the moment, the disposition
  is recorded in the issue's directory as `disposition.txt` (status, reason,
  the base sha the verdict was made against, the time), and the patch is
  archived to `out/issue-N/superseded/` — archival, never deletion — so the
  "already produced" test can no longer see it and the issue re-enters
  dispatch for regeneration against current main.
- A conflict with at least one certified keep-both file, a parser failure
  over a certified shape, an unenumerable conflict, or an infrastructure
  fault is **not** structural: it stays an ordinary HELD, re-offered next
  pass. The pass acts on dead patches only where it can prove every file is
  dead.
- `PassCounters` gained `invalidated`, rendered on the outcome line and
  included in `reconciles()`: the category is visible at the summary level
  instead of folded into `skipped`.
- `convert/conflict.rs` gained `is_structural_refusal` (the all-files
  predicate, evaluated on the still-alive worktree); `convert/invalidated.rs`
  (new) carries the disposition record and the archive.
- `convert.rs` stayed inside the size ratchet by compressing doc blocks that
  restated what the function-level docs already say.

Not yet done (fleet-host half, separate): queue eviction of closed issues,
label removal on close, and `NO-OUTPUT` attempt backoff — the second
invariant in #4637, which lives in `topup.sh` and the claim path, not here.
