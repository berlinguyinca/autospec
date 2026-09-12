# A correction applies to every instance of the method, not just where the flaw was found (issue #4428)

A baseline-diff gate — compare the patch's failing-test set against
main's, rather than pass/fail — was built because `autospec-cli` was red
on main and a plain run could not judge a patch there. The correction was
applied **only to `autospec-cli`**. For `autospec-core` the gate kept
pass/fail, having never asked whether that crate was also red. It was:
`runner_executes_the_newly_registered_bats_suites` fails on main. Three
patches — 4070, 4015 and 4094 — were recorded HELD with that test named as
their failure. All three were innocent; re-gated against the real
baseline, all three passed and are now merged.

The held reasons were written confidently and were wrong, and they would
have stayed wrong: a HELD entry is not re-examined, so the patches would
have sat there indefinitely with a plausible-looking explanation attached.

The fix was built as a *response to a discovered problem* rather than as a
*correct method*. The question "is pass/fail ever a valid gate against a
repository whose main is not green?" was never asked, and its answer is no
— for any crate. Same session, same shape: the throughput floor was
corrected three times, each fix addressing the instance in front rather
than the class (#4411).

- **A correction applies to every instance of the method, not the instance
  where the flaw was found.** After fixing a check, enumerate the other
  call sites and apply it there before moving on
  (`CorrectionAudit`). The instance that revealed the bug is rarely the
  only one affected — it is just the one that happened to be noticed. The
  enumeration of all instances comes from the caller's own text search or
  crate list; the audit can only show the gap between what was enumerated
  and what was fixed, and it names the unfixed instances.
- **Establish the baseline for every crate a gate runs against, at the
  start of the pass** (`GatePass`). "Main is green here" is a measurement,
  never an assumption: a crate without a measured baseline is neither
  admitted nor held (`Judgment::NoBaseline`) — a harness fault, not a
  property of the patch. There is no "presumed green" state on purpose;
  the absence of a baseline *is* the never-measured state.
- **Pass/fail is never a valid gate on a red main, for any crate.** On a
  crate whose measured main fails, the only valid judgment is the baseline
  diff — the patch's new failures are `patch − main` — and a pass/fail
  judgment is invalid whatever the run printed (`InvalidJudgment`),
  including a run that printed zero failures.
- **A HELD entry is examined against the measured baseline, or not at
  all** (`reexamine_held`). A hold whose cited failures all fail on main
  was main's, not the patch's — re-gated and dropped. An entry on a crate
  with no measured baseline is not guessed either way: it keeps its
  possibly-wrong explanation until the baseline exists, and the report
  says so.

Checkable in `autospec_core::baseline_coverage` (`MainStatus::from_failures`,
`GatePass::{establish_baseline, unbaselined, coverage_line}`, `judge`,
`CorrectionAudit`, `reexamine_held`). Tests:
`crates/autospec-core/tests/baseline_coverage.rs`, including the
regression that reconstructs the incident (a two-crate gate whose mains
are both red, the correction applied to one crate only — the pass/fail
judgment on the other is invalid, and the three HELD patches are
exonerated once the second baseline is measured) and the controls (the
fully baselined pass, the correction applied to every instance, and a
hold with a genuinely new failure that still stands).
