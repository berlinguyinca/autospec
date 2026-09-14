# A health signal's first use must not be a deletion (#4555)

Three fleet health signals were wrong before their first destructive use — one
exactly inverted (mtime staleness: the freshest entries were dead, the
stalest all running), one that timed out precisely on the busiest workers, one
that mis-sorted its input and said 0 where the truth was nonzero. None had
ever been checked against a known-healthy and a known-dead input; the
deletion discovered the error.

- `decomposer-contract.md` gains a *Reaper-task decomposition* section: a
  reaper child (health check, liveness probe, staleness reaping, GC,
  reconcile-by-removal) is queueable only if its spec names the source of
  truth, both directions as test cases (known-healthy / known-dead), the
  act-or-report first release, and what `unknown` means; otherwise it is
  filed `autospec:blocked-prerequisite`, not `auto-implement`.
- `implementer-contract.md` gains the `REAPER_CONTRACT` corrective directive:
  demonstrate both directions as tests before the signal gates anything;
  prefer the authoritative negative to the inferred positive; separate
  detection from removal (first release reports, then acts); a timestamp is
  liveness only if something writes it on a period; `unknown` never
  authorises removal.
- The decomposer-contract trims that made room are pure compressions (the
  v2-flow routing restatement, the trio-rule restatement, two
  parentheticals) — no behavior removed.
