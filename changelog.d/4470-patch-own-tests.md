## convert: a patch failing its own new tests is named as such (#4470)

A patch whose own newly-added tests fail is a different defect from one that
regresses existing tests: the author had the failing evidence in hand and
emitted anyway. The conversion gate now tells them apart.

- a new `conversion_own_tests` module names the test functions a patch adds
  (`added_test_names` — the attribute and its `fn`, `#[tokio::test]` and
  `async`/`const`/`unsafe` modifiers included) and intersects them with the
  failing-test set a gate run attributes (`own_failing_tests`);
- when a `GateResult::Fail` names one of the patch's own new tests, the hold
  reason is `gate failed: the patch fails its own newly-added test(s): <names>`
  — distinct from a regression of existing tests, so the runner fixes the
  tests, not the code they cover;
- a patch whose own tests are not among the failures keeps the generic note:
  the attribution is never fabricated.

The added-test *count* (`tests_added_by_patch`, the unchanged-count
contradiction #4532) and the added-test *names* now share one definition of a
test attribute, so a gate failure and the contradiction cannot disagree about
what the patch added.
