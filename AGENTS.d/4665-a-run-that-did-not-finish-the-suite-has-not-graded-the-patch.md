# A run that did not finish the suite has not graded the patch

Issue #4665: a run came back `VERIFIED` with its counters beside it —
`test_passed=1120 test_failed=30 new_failing_tests=0
fixed_baseline_failures=120`. `1120 + 30 = 1150` of a 10 073-test suite: the
run covered 11 % and stopped, because `cargo test` aborts after the first
failing test *binary* and the runner invoked it without `--no-fail-fast`.
Both summary numbers were wrong, in opposite directions, from that one cause.

## The defect

`new_failing_tests=0` did not mean the patch breaks nothing. It means nothing
broke in the 11 % that ran, and any regression in a test binary scheduled
after the abort was invisible while the issue sat in the conversion queue
marked good.

`fixed_baseline_failures=120` was worse, because it was confidently wrong. The
baseline holds 149 known failures; a test that never ran cannot fail, so
absence from the failure list was scored as a repair. A patch to one module
does not fix 120 unrelated tests — that number counts tests the run never
reached, and it was reported as a benefit of the change.

The reader had everything needed and used none of it: the arithmetic that
separates "verified" from "unmeasured" is `test_passed + test_failed` against
the suite size, both of which were already in the record or one field away.

In the repository's own policy the exposure sat in two places. Triage's green
arm returned `GateLocally { AgentGreen }` for `VERIFIED` with no reference to
how much had run — which is survivable, because the local gate re-runs every
stage — but it named its reason "agent reported green", so the operator read a
confirmation where the record supported none. And a green label was reported
verbatim wherever a status is surfaced for a human, leaving the division to the
reader.

## The fix

`PARTIAL-COVERAGE` is an emitted status in the run-status vocabulary, and
coverage is a derived fact rather than a caller's assertion
(`execution::status_triage::coverage`):

- `Coverage::from_counts` compares the executed counters against the declared
  suite size, distinguishing `Complete`, `Partial { ran, total }` and
  `Unrecorded`;
- `entitled` decides what a label may claim: a `VERIFIED` written over a
  prefix of the suite is reported as `PARTIAL-COVERAGE`, while
  `NEW-TEST-FAILURES` is left alone, because an observed failure is an observed
  failure however little else ran;
- `tally_fixes` scores a baseline against both observed sets, so an entry the
  run never observed is reported as unobserved instead of counted as fixed;
- triage routes a green label over a short run to `GateLocally` with
  `GateBasis::PartialCoverage`, whose rendered line states
  `tests_run=1150 of 10073 (11%)`;
- `dispatch_guard::classify_report` reports the entitled status, so the reason
  an operator reads says the run fell short instead of saying `VERIFIED`.

## The invariant

1. **A pass needs a complete run; a failure does not.** A run that executed a
   prefix of the suite can prove a failure and cannot prove that every test it
   never reached would have passed. Only a claim of passing is downgraded — and
   the downgrade is not a failing verdict, because the run did not show the
   patch broken; it showed that it cannot say either way.

2. **Absence from a failure list is not a repair.** "Fixed a baseline failure"
   means the test ran and passed. Test membership in a result set is only
   evidence about the tests that ran, so an entry in neither the passed nor the
   failed set is reported as unobserved. Scoring unreached work as a repair
   turns a truncated run into the most productive patch of the batch.

3. **Coverage is stated in the verdict, not left to the reader.** `VERIFIED`
   and `tests_run=1150 of 10073` must not be able to appear together without
   the first being downgraded. A consumer that has to divide two numbers to
   notice a shortfall will not notice it.

4. **A missing suite size is not a shortfall.** Every record written before the
   total existed would otherwise become ungradeable, which converts a missing
   fact into a false one. `Unrecorded` keeps the verdict and says on the line
   that coverage is unknown, so the gap accumulates visibly. A zero total is
   treated the same way: an empty capture is not a suite that passed.

5. **A short run is re-measured, not rejected.** It is not a failed run: the
   artifact stays for the pass's own gate, which is precisely the measurement
   the run skipped. Archiving it would destroy a possibly-good patch on the
   strength of someone else's missing flag, and holding it would assert a
   deterministic property of the submission that was never established.

## The general rule

Before treating an absence as a result, ask which observations could have
produced it and whether the run was capable of making them. A count of zero
from a run that stopped early, a "fixed" from a test that never executed, and a
"green" from a suite that was 11 % present are all the same mistake: reading the
boundary of an observation as a property of the thing observed.
