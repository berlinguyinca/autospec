# A patch failing its own new tests is an unfinished attempt

Issue #4470: a patch for #3762 arrived carrying `crates/autospec-core/tests/
issue_lint.rs` — a test file that did not exist on main — and three of that
file's own tests failed against the patch's own implementation. The only way
that happens is that the tests were never run, or were run and failed and the
patch was emitted anyway.

## The defect

A patch that regresses an existing test is a mistake about the rest of the
system. A patch whose *own* tests fail is a process failure: the author had
the evidence in hand and emitted regardless. The two need different remedies,
and the conversion pass was holding both with the same generic gate-failure
note, so the runner could not tell "your new test is wrong" from "you broke
something else".

## The fix

The gate already attributes the failing-test set from a run's authoritative
`failures:` block. The fix names the tests a patch *adds* and intersects:
when a gate failure names one of the patch's own new tests, the hold reason
says so explicitly — `the patch fails its own newly-added test(s)` — instead
of the generic note. The count used by the unchanged-count contradiction
(#4532) and these names now share one definition of a test attribute.

## The invariants

1. **Own tests are the author's process failure, distinct from a regression.**
   The two have different owners and different remedies; a hold reason that
   collapses them sends the runner to the wrong fix.

2. **The attribution is never fabricated.** A patch whose own tests are not
   among the failures keeps the generic note. `own_failing_tests` returns
   empty unless a name literally matches; a name the extractor missed degrades
   to the generic note, it never invents a false attribution.

3. **Count and names agree.** `tests_added_by_patch` (a number) and
   `added_test_names` (the list) must use the same definition of "a test the
   patch adds", or a gate failure and the unchanged-count contradiction can
   disagree about what the patch contains.

## The general rule

When two failures are cheap to tell apart and have different remedies, tell
them apart at the point of judgment — the cost of a wrong attribution is that
the fix targets the wrong owner. And when a change carries its own test and
that test fails, the verdict is about the author's process, not the code it
covers.
