### Added

- `autospec_core::positive_control` — primitives for the invariant that a
  negative search result requires a positive control (issue #4422: the analyst
  reported "zero issues declare dependencies" when the true answer was 133,
  because the inline-text query — `Depends on #N`, `Dependencies:`, `Blocked
  by`, `Requires` — matched none of the `## Dependencies` headings the issues
  actually use). The tool could see the corpus; the failure was the query, and
  a zero over a corpus is evidence about the corpus *and* the query at once.
  A `NegativeClaim` is checked against a `PositiveControl` — a known-present
  fixture of the data the query should match — by `verdict`: a fixture the
  query does not find is `QueryIsTheFinding` (the query is broken, the zero is
  not-matched, not absent), no control at all is `NoPositiveControl`, and once
  the query is validated the format must be `FormatBasis::Established` from
  real examples (else `FormatNotEstablished`) and an
  `ClaimStrength::Extraordinary` claim must carry a second method (else
  `NeedsSecondMethod`). Only `Verdict::holds` may be reported as absence — the
  tooling gate that a parser failing to find the example in its own test data
  fails rather than reports zero. Tests:
  `crates/autospec-core/tests/positive_control.rs`, including the incident
  end-to-end: a zero over a working population on a guessed format with no
  control is untrusted, the fixture the inline-text query does not match makes
  the query the finding, and a format-derived query that finds its fixture and
  satisfies the scepticism invariant holds (#4422, 2026-09-11).
