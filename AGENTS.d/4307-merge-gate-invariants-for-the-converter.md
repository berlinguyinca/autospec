# Merge-gate invariants for the converter (issue #4307)

The patch-to-PR converter (`convpass.sh`) ran a local gate — fmt, build,
clippy, test on one host — and merged on its result. The local gate covered
three of the six jobs the repository's `rust-suites` CI runs; it never
reached macOS, Windows or FreeBSD. A macOS-gated test file (#4306) with five
compile errors sat merged and hidden, because the workflow's pass rate on
`main` was 0/40 — a red gate read as a green one, and a merge record that
said nothing about the three jobs it did not check read as if it had.

- **The local gate is a pre-filter, not the gate.** The converter's merge
  requires the repository's real CI to have passed for the PR. A local-gate
  failure is a refusal (cheapest check first), but a local-gate pass is not a
  CI verdict: `Pending`, `Failed` and `NotRun` are all `CiNotPassed`, and
  only `Passed` approves. The fold "local pass ⇒ merge" is the incident, and
  it is now a named decision variant rather than an implicit default.
- **The merge record names what it did not check.** `Coverage` splits the
  workflow's jobs into `checked` and `unchecked` in workflow order, and
  `record_names_unchecked` refuses a merge record that does not name every
  unchecked job id verbatim. "Gate passed" with three jobs silently missing
  is a refusal, not a default — absence within the record is a gap, not an
  all-clear.
- **Refuse while the target workflow is red on the base branch.** A merge
  into a base where `rust-suites` is failing is `BaseBranchRed { failing_jobs }`
  — the failing jobs named — even when the local gate and the PR's own CI are
  green. The incident's 0/40 `main` would have blocked every conversion until
  fixed, instead of papering over it on every merge.
- **The pass rate is a number that is reported, not a feeling that is
  assumed.** `PassRate` carries the counters and renders `workflow on
  branch: N/M passed (P%)` via `line()`; `all_failing` flags the 0/40 state
  explicitly. A gate whose every run fails is flagged, never smoothed into
  "mostly passing", and a rate with no runs is not all-failing — there is
  nothing yet to fail.

Checkable in `autospec_core::merge_gate` (`decide`, `MergeDecision`,
`Coverage`, `record_names_unchecked`, `PassRate`). Tests:
`crates/autospec-core/tests/merge_gate.rs`.
