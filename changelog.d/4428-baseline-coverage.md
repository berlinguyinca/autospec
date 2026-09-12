### Added

- `autospec-core::baseline_coverage` — primitives for the invariant that a
  correction to a method applies to every instance of the method, not just
  the instance where the flaw was found (issue #4428: a baseline-diff gate
  was applied to `autospec-cli` only, leaving `autospec-core` judged by
  pass/fail on a red main; three innocent patches — 4070, 4015, 4094 —
  were held with a confidently-written and wrong reason). "Main is green
  here" is a measurement, never an assumption: `GatePass` carries the
  pass's scope and the measured baseline per crate, `unbaselined` /
  `coverage_line` name the unmeasured crates, and `judge` returns
  `NoBaseline` — never admitted, never held — for an unbaselined crate
  under either method. Pass/fail on a measured red main is
  `InvalidJudgment` whatever the run printed; the baseline diff admits or
  holds only on `patch − main`. `CorrectionAudit` shows what was fixed
  against what was enumerated and names the unfixed instances;
  `reexamine_held` re-examines HELD entries against the measured baseline
  (all-main citations exonerate, any patch-introduced citation holds, and
  an unbaselined crate is not guessed either way) (#4428, 2026-09-12).
