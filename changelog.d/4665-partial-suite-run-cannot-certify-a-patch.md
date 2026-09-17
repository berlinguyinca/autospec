## convert: a run that skipped most of the suite can no longer certify a patch (#4665)

A run reported `VERIFIED` with `test_passed=1120 test_failed=30
new_failing_tests=0 fixed_baseline_failures=120`. Those counters add to 1150 of
a 10 073-test suite: the run covered 11 % and stopped, because `cargo test`
aborts after the first failing test *binary* and the runner called it without
`--no-fail-fast`. The issue was queued for merge as verified, and 120 baseline
failures were credited to a patch that had not reached them.

- `PARTIAL-COVERAGE` joins the run-status vocabulary as an **emitted** status.
  An undeclared name resolves to `None`, and `None` falls through to the green
  arm — which is how an unknown becomes a confirmation (#4206).
- A new `execution::status_triage::coverage` module makes coverage a derived
  fact: `Coverage::from_counts` compares the counters with the declared suite
  size and separates `Complete`, `Partial { ran, total }` and `Unrecorded`.
  A missing total is *not* a shortfall — every record written before the field
  existed would otherwise become ungradeable — and a zero total is treated the
  same way, because an empty capture is not a suite that passed.
- `entitled` decides what a label may claim: **a short run can prove a failure
  and cannot prove a pass**, so a `VERIFIED` over a prefix becomes
  `PARTIAL-COVERAGE` while `NEW-TEST-FAILURES` is left exactly as recorded.
- `tally_fixes` scores a baseline against both observed sets, so an entry the
  run never observed is reported as `unobserved=` instead of counted as fixed.
  Absence from a failure list is only evidence about the tests that ran.
- `triage` routes a green label over a short run to `GateLocally` as the new
  `GateBasis::PartialCoverage`, whose line states
  `tests_run=1150 of 10073 (11%)`. The local gate is not duplicated work: it is
  the measurement the agent's run skipped.
- `dispatch_guard::classify_report` reports the entitled status, so the hold
  reason an operator reads says the run fell short rather than saying `VERIFIED`
  and leaving the division to them. The artifact stays convertible and is never
  archived — a run that stopped short is not a run that failed.
- `AgentReport` parses `test_passed`, `test_failed` and `tests_total`, refusing a
  non-integer rather than defaulting it to zero; the parsing itself moved to
  `execution::status_triage::record` to keep the policy module inside its size
  ratchet.

The fleet side still owes its half: `--no-fail-fast` on the test stage, a
`tests_total` written with every graded run, and a baseline capture that refuses
to write a truncated baseline. Until `tests_total` is written, the check reports
coverage as unrecorded rather than guessing — see issue #4665 for that slice.
