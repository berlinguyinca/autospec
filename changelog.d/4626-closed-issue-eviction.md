## convert: closed-issue eviction (#4626)

A held patch whose issue is closed is residue, not pending work. The
conversion pass now asks the tracker for the state of each
held-recorded candidate (`gh api repos/R/issues/N`) before selection:

- a closed issue's patch is named on its own `CLOSED #N` line, never
  offered, never gated, and under `--apply` archived to
  `out/issue-N/superseded/` (the queue entry releases with it) with its
  hold record removed from the ledger; the outcome line counts it
  `closed=N` and the `--json` plan carries the `closed` array;
- only held-recorded candidates are asked — the ledger is bounded, the
  fresh backlog is not, and a fresh patch for a closed issue comes from
  a closed queue entry, which the dispatch side evicts;
- a state that cannot be read (no repo, no `gh`, a failed call) answers
  "no": unknown never authorises acting, so an unreachable tracker
  leaves the hold in place;
- selection order: delivered > closed > attempt > hold — a merged PR on
  a closed issue is residue to archive, not a live attempt.
