### Added

- `autospec-core::unfed_pass` — primitives for the invariants that a pass
  given no work must not report the same clean summary as an idle one: the
  unfed line names the selector that would feed the pass (`unfed_line`, the
  `iwconv.sh` sibling fix) and must differ from the examined line
  (`examined_line`, `identical_summary_finding`); selection is wired into
  execution by default, so a bare invocation runs the selector itself or
  refuses naming it (`plan_invocation`, `BareDefault`, `bare_invocation_finding`
  — an empty selector result is a true idle, not a finding); the exit trap
  reflects the exit path, never re-stating counters a guarded run never
  populated (`trap_line`, `trap_line_findings`); and the summary reports
  `examined=N` alongside the action counters, because a counter of zero is
  not evidence of work performed (`PassCounters::examined`,
  `PassCounters::reconciles`, `missing_examined_finding`) (#4296, 2026-09-11).
