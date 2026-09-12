# A differential gate must prove the measurement happened (issue #4404)

A gate compared a patch's failing-test set against a baseline's failing-test
set and passed the patch when it introduced no new failures. Sound reasoning,
and it works — until the patch does not compile.

A build failure produces **zero test results**, therefore an **empty failure
set**, and the empty set introduces no new failures. The gate reported:

```
baseline: 2  with patch: 0
PASS (no new failures beyond baseline)
```

The patch had 19 `E0596` errors and ran no tests at all. Measured on
autospec #4148, by a gate written earlier in the same session specifically to
handle a repository whose `main` is red.

## The general shape

**An absent result is not a good result.** Any comparison of the form "did
this make things worse" silently rewards a run that produced nothing, because
nothing is never worse. The stronger the baseline's failure list, the more
favourably a total failure compares against it.

This is the same family as a tool that warns on stderr and still writes to
stdout, and as an empty dashboard reading as "nothing is happening" — all
three were hit in a single day. The unifying error is treating
**absence of evidence** as evidence.

## The invariant

A differential gate must first establish that the measurement **happened**:

1. check the runner's exit status, and distinguish *compile/setup failure*
   from *test failure* — they are different verdicts, not a spectrum;
2. assert the run produced the expected shape of output — at least one
   `test result:` line, and a total count within range of the baseline's;
3. only then compare the failure sets.

Rule of thumb: before comparing two measurements, prove both exist. A
comparison is only meaningful between two things that were actually measured.

## For specs

Any spec describing a "compare against baseline" check must state what
happens when the run under test fails to produce a measurement at all — and
the answer must never be "it passes." The existing spec that describes a
baseline comparison does state it: `docs/conversion-gate.md` §2 makes
`TESTS-DO-NOT-COMPILE` and `BUILD-FAILED` terminal statuses that never admit
a patch to the queue, and "a baseline can justify (attribute) a test failure,
but no baseline can justify a test that does not compile."

Checkable in `autospec_core::gate_verdict` (issue #4434 expressed this rule
as a type): `GateVerdict::NotMeasured` is a state that `is_pass` never
reads, `TestRun::measured` is the "did anything run" proof, and
`differential` refuses an unmeasured baseline, an unmeasured candidate
("an empty failure set is not an improvement, it means the suite never
ran"), and a candidate that emitted fewer `test result:` lines than the
baseline. The same invariant appears as a terminal status in
`autospec_core::conversion_gate` (`TESTS-DO-NOT-COMPILE`, `BUILD-FAILED` —
see `docs/conversion-gate.md`) and as the `HARNESS_NEVER_RAN` finding in
`scripts/test-failures-baseline.sh`.

Tests: `crates/autospec-core/tests/gate_verdict.rs`, including the regression
that reconstructs this incident — `a_build_failure_is_not_an_improvement`
(an empty failure set against a 2-failure baseline must be `NotMeasured`,
not `Pass`, and the message must say the suite never ran) — plus
`a_skipped_suite_is_not_a_pass` (exit 0 with no tests run is not a pass),
`measuring_less_is_not_improving`, and `an_unmeasured_baseline_refuses_to_judge`.
