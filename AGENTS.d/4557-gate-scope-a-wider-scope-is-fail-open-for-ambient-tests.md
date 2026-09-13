# Gate scope: a wider scope is fail-open for ambient tests (issue #4557)

Measured 2026-09-12 on commit `00e51d4f`, the same tree in both cases: a
225-target workspace run reported 5394 passed, 0 failed, while the same
targets run alone failed three tests — `foreground_scan_recovers_stale_pending_startup_heartbeat_pending_before_acquire`,
`foreground_recovers_with_integrated_inactive_local_branch`, and
`executor_supervision_descendant_capture_reserves_descriptor_headroom`. CI
agreed with the alone run. Main's `rust-suites` had been red for at least six
consecutive pushes while local workspace gates reported clean.

#4532 fixed the opposite error: the gate's scope was too **narrow**, so its
numbers did not describe the *patch*. This one is the mirror image: the scope
was too **wide**, so the numbers did not describe the *tree* — a target that
only fails alone is invisible to a workspace run. "Gate at the widest scope,
fail closed" is the right rule for patch coverage and the wrong rule for
order- and environment-dependent tests. The two defects pull in opposite
directions and cannot both be fixed by choosing a scope.

- **A test's verdict must not depend on which other targets ran beside it.**
  A test that passes only in a crowd, or only alone, is reporting on its
  environment. Make the dependency explicit — reserve the descriptors, pin the
  clock, seed the branch state — or mark it `#[ignore]` with the reason, but
  do not leave it reporting a verdict it cannot support. The three tests above
  were found to be worse than flaky: two pinned a recovery gate
  (`branch_blocks_stale_recovery`, #2864) that #4104 deliberately removed, so
  they failed deterministically in healthy environments and only "passed" in
  crowded runs where a load-induced lookup failure happened to block recovery;
  the third measured process-global descriptor counts against 1100 concurrent
  test threads. The stale sub-cases were removed with the history in a
  comment; the fd test is `#[ignore]`d in the workspace run.
- **The gate must run each target the way its verdict requires.** CI's
  `rust-suites` job carries a `Test ambient-sensitive tests in isolation` step
  that runs each ambient-sensitive test exact and alone
  (`--exact`, plus `--include-ignored` for the workspace-ignored fd test). A
  local gate that only runs the workspace scope is measuring a different thing
  than that step, and its green says nothing about those tests.
- **A green local gate against a red main is a contradiction to be
  investigated, not a result to be reported.** Six red pushes and a clean local
  sweep should have been the alarm. Compare against main's CI conclusion, not
  only against the local numbers.
- **Any rule about gate scope must say which of the two it protects.**
  Fail-closed for coverage of a patch, or fail-open for order- and
  environment-dependent tests — never both silently. The workspace-scope rule
  in AGENTS.md states its protection; so must any new scope rule.
