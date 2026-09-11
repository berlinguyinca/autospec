### Added

- `autospec-core::verdict_shelf` — primitives for the invariant that a
  verdict has a shelf life and a pass summary carries its expiry: every
  verdict records the base sha it was decided against (a verdict with no
  recorded base is refused, never defaulted, `Verdict::new`), and the
  summary reports per bucket how many are against the current trunk
  versus stale — `held=15 (7 current, 8 against an older base, will be
  retested)` (`PassSummary::line`, `BucketShelf`); a pass that outlives
  the trunk's change interval says so instead of presenting an average
  over a moving target (`pass_span`, `PassSpan::warn_line` — the trunk
  moving during the pass is the stronger fact, and a zero or absent
  interval is not a measurement); and where a pass is long, cheap
  decisions ("PR already exists", "issue closed") are re-checked at the
  end rather than trusted from the start, while expensive verdicts are
  left to the `(patch hash, base sha)` memo key (`due_rechecks`,
  `recheck_line`) (#3673, 2026-09-11).
