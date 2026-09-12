# Platform gates: a red platform job cannot be ignored (issue #4312)

A platform-specific CI job (`macos-test`, `freebsd-test`) is
the sole verification of the code behind its `#[cfg]` gate: when it goes red,
every line behind the gate merges unverified, and a single-run view cannot
tell a job that broke moments ago from one that has been red for days. The
#4306 macOS breakage hid for a day and #4311 repeated the same shape on
Windows; each instance was fixed at the code level, and the class is the
tooling that makes a red platform job impossible to ignore.

- **A platform job's failure is a coverage loss, not a flaky check.**
  `classify_failure` maps a failing job to `CoverageLoss { job, platform }`
  when the job is the sole CI verification of a platform surface and to
  `Ordinary` otherwise. The two render differently and escalate differently:
  a red `macos-test` is a total loss of macOS coverage, never "probably
  flaky, re-run it".
- **The alarm fires on the rate, not on individual runs.** `rate_alarm` fires
  when the gate's pass rate for the default branch is all-failing (0/N, N >
  0) — conspicuous on its own, no state change required — and names the lost
  coverage when the platform is known. It reuses `merge_gate::PassRate`
  rather than duplicating it; the empty window is unknown, not red.
- **The local gate is not authority for surfaces it cannot compile.**
  `local_authority` scans the patch for `#[cfg(...)]` attributes and returns
  a `Hold` naming every surface the host cannot compile and the job that
  solely verifies it: the merge defers to the named CI jobs. Over-reporting
  (a cfg in a comment) is the safe direction — an unnecessary hold, never an
  unverified merge; `cfg!(...)` is a runtime branch and is not a surface.
- **The parser claims only what it can classify.** `target_os`,
  `target_family`, the bare `unix`/`windows` families, and a single
  `not(...)` around them; feature flags, `target_arch`, `any(...)`/`all(...)`
  and unknown values return `None` — absence is the honest encoding, not an
  invented semantics.

Checkable in `autospec_core::platform_gate` (`parse_platform_predicate`,
`patch_surfaces`, `local_authority`, `classify_failure`, `rate_alarm`).
Tests: `crates/autospec-core/tests/platform_gate.rs`, including the
regression that reconstructs the incident (an all-failing 0/40 rate, a red
platform job, and a macOS-gated patch on a Linux host) and the control that
a plain patch never holds.
