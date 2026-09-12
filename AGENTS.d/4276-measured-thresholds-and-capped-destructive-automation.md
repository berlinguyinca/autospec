# Measured thresholds and capped destructive automation (issue #4276)

A watchdog built on a default constant and a "no output" signal killed 45%
of a fleet's healthy agents: the threshold (75 min) sat at the *median* of
measured run durations (4077 s; 190 of 418 runs longer than 75 min), not
the tail, because it was derived from the runner's `LIMIT:-2700` default —
while the dispatcher's call site passed `LIMIT=25200` (7 h) — and the "no
output for 15 min" signal is confoundable by buffered output (the same
family of error as #4259). A threshold at the median is not an outlier
detector: it is a coin flip applied to healthy work.

- **Measure the distribution before building the detector.** Compute the
  median and p90 of the metric the threshold applies to over real runs, and
  log the threshold against them before the detector is built. A threshold
  at or below the median is a defect, not a configuration choice
  (`ThresholdAudit::placement`, `is_outlier_threshold`). The regression
  test reconstructs the incident's distribution (418 runs, median 4077 s,
  p90 13939 s, max 25201 s, 190/418 > 75 min) and asserts the 45%.
- **Read the caller before trusting a default.** A value found in the
  callee's signature (`LIMIT:-2700`) is a default until a call site sets
  it. The operating value is what the caller passes; the default is
  operating only when no caller overrides it (`LimitProvenance`).
- **Prefer a detector that normal operation cannot confound.** "No output
  for N minutes" is confoundable — buffered output, slow writes, quiet
  phases all produce silence. "Past the limit its own dispatcher set and
  still in the call" is not, *given the limit is the operating value*, not
  a default the dispatcher overrides (`confoundable`).
- **Cap destructive automation, log it, and treat the cap as a
  measurement window.** Every kill is logged with its position in the cap
  (`KillLedger`); a zero cap is rejected. A cap hit whose kills sit at or
  below the p90 of normal work is a detector defect, not a fleet defect
  (`cap_verdict`): a detector calibrated on the tail should not exhaust its
  cap, and a cap hit is a signal to stop and re-measure — never a reason to
  raise the cap.
- **Asymmetric cost, asymmetric thresholds.** Killing a healthy agent costs
  a lost run; a wedged agent costs a slot for a bounded time. The
  threshold belongs far out in the tail — at the limit the system itself
  enforces, not a guess — and the build gate refuses a destructive detector
  whose threshold does not exceed the operating limit: a watchdog that
  sits below the limit its dispatcher sets is racing that timeout, and
  fires before the timeout's own verdict
  (`build_gate`, empty refusals = may build).

Checkable in `autospec_core::threshold_calibration` (`Distribution`,
`ThresholdAudit`, `LimitProvenance`, `confoundable`, `build_gate`,
`KillLedger`, `cap_verdict`). Tests: `crates/autospec-core/tests/threshold_calibration.rs`.
