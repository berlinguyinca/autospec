# Verdict shelf life: a pass summary carries its expiry (issue #3673)

One conversion pass, measured: 158 minutes for 89 patches, **49** commits
to `main` during the pass, **80** verdicts issued before the trunk moved —
**8** of them `HELD`. The pass reported one bare line,
`converted=N held=N skipped=N`, as though it described one state of the
world. It did not: eight of those holds were decided against a trunk that
had since gained 49 commits, and a patch held on a build error against the
old trunk may build cleanly against the new one, or vice versa. The
machinery already handled the staleness correctly — the hold memo is keyed
on `(patch hash, base sha)`, so a stale hold is *retested* on the next pass
rather than replayed — and only the *summary* misled. That is the cheap
version of the stale-baseline fault that produced 13 false
`NEW-TEST-FAILURES`, and the cheap version is the one worth fixing before
it becomes the expensive one again.

- **A verdict records the base sha it was decided against.** A verdict
  with no recorded base is a defect, never a missing fact to default
  (`Verdict::new` returns `None`). The summary reports per bucket how
  many are against the current trunk versus stale: `held=15 (7 current,
  8 against an older base, will be retested)` — a bucket with no stale
  verdicts stays bare, because "all current" is the state the bare line
  honestly describes. The "will be retested" clause is on `held` only: a
  `converted` verdict is terminal and a `skipped` one is merely
  re-evaluated, so neither is retested in that sense.
- **A pass that outlives the trunk's change interval says so**, rather
  than presenting an average over a moving target (`pass_span`,
  `PassSpan::warn_line`). The trunk moving during the pass is the
  stronger fact: `TrunkMoved` wins over `OutlivedInterval`, and a zero
  or absent interval is not a measurement and never trips the span.
- **Where a pass is long, cheap decisions are re-checked at the end**
  rather than trusted from the start: "PR already exists" and "issue
  closed" are seconds to recompute and are the most likely to have
  changed (`due_rechecks`, `recheck_line`). Expensive verdicts (builds,
  gates) are not re-run mid-pass — the `(patch hash, base sha)` memo key
  re-tests them on the next pass automatically, and it is re-running
  them that made the pass 158 minutes.
- The deeper fix is throughput (#3635): a pass short relative to the
  trunk's change rate has no shelf-life problem at all.

Checkable in `autospec_core::verdict_shelf` (`Verdict`, `VerdictKind`,
`BucketShelf`, `PassSummary::{line, shelf, span}`, `pass_span`,
`PassSpan::warn_line`, `due_rechecks`, `recheck_line` — pure in-memory,
no subprocess, so the shell conversion pass can adopt them as the single
source of truth). Tests:
`crates/autospec-core/tests/verdict_shelf.rs`, including the regression
that reconstructs the incident pass (158 min, 89 patches, 49 trunk
commits, 8 stale holds) and asserts the bare line is not what the pass
prints.
