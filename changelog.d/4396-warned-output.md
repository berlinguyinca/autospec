### Added

- `autospec-core::warned_output` — primitives for the invariant that a tool
  that warns on stderr and still writes a result to stdout is a tool whose
  exit status must be checked (issue #4396: `comm -23` fed with `sort -un`
  output warned "file 1 is not in sorted order" and still emitted 588
  "fresh patches" when the true answer was 164, and its unchecked status
  turned the warning into a confident wrong answer nearly reported as the
  size of the backlog). Set operations are checked against the collation
  the consumer requires at the point of use — never the collation the
  producer happened to guarantee (`SetOp` / `check_order` /
  `OrderVerdict::Unordered` names both collations and both remedies: sort
  defensively at the point of use with the consumer's exact collation, or do
  the set operation in a language with real sets). Pipeline steps are
  classified by their warning and their status check (`Step` /
  `StepOutcome` — a warned step with an unchecked status is
  `ConfidentWrongAnswer`: strictly worse than a crash, because nothing
  downstream can tell). Reported numbers are asserted against a known bound
  (`CountBound` — more than the total is `FAIL`, the number was not
  measured; a filtered count equal to the unfiltered total is `WARN`, the
  filter may not have run — in the incident, 588 == 588 was the giveaway,
  and the earlier 121-against-11 report would have been a straight `FAIL`).
  A spec asking for a count of outstanding work must name the filter
  (`BacklogQuestion` / `Filter`): "has a patch" (588), "has no branch or PR"
  (164), and "…and the issue is still open" (75) are three different numbers
  for the same question — reporting the wrong one is an 8x misstatement of
  the remaining work (#4396, 2026-09-12).
