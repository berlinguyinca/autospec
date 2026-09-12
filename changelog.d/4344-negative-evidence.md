### Added

- `autospec-core::negative_evidence` — primitives for the invariant that a
  confident negative from a tool whose coverage you have not verified is
  not evidence of absence (issue #4344: four consistent zeros from GitHub
  code search over a private repository nearly justified building a second
  gateway beside an existing one, because GitHub does not index private
  repositories and returns zero rather than an error). Negatives are
  classified against a control query — a term certain to be in the source,
  run through the same tool over the same source: a zero control makes the
  index the finding and voids every zero from that tool over that source
  (`GroupVerdict::ToolIsTheFinding`), a positive control or a direct
  enumeration makes the negative hold (`HoldsBasis::Controlled` /
  `HoldsBasis::Enumerated` — a listing is present or it errors), and a
  search with no control is `CoverageUnverified`, never a reason to
  terminate the investigation. Consistent zeros from one tool over one
  source share a failure mode, so `NegativeReport::weight` counts
  independent `(tool, source)` groups, not queries: four zeros are one
  observation. `CoverageLimit` records a tool's coverage limits where the
  tool will be read (#4344, 2026-09-11).
