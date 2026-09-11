### Added

- `autospec_core::threshold_calibration`: measured thresholds and capped
  destructive automation (issue #4276). Encodes the incident's rules as
  checkable primitives — `Distribution` (nearest-rank median/p90 measured
  before building a detector), `ThresholdAudit` (a threshold at or below
  the median of the measured metric is a coin flip on healthy work, not an
  outlier detector), `LimitProvenance` (a callee default is the operating
  value only when no call site overrides it), `confoundable` (an "output
  silence" signal is confoundable by normal operation; "past the limit its
  own dispatcher set" is not), `build_gate` (refusal lines for a
  destructive detector, empty when it may be built), and `KillLedger`
  (capped, logged destructive automation; a cap hit whose kills sit at or
  below the p90 of normal work is a detector defect, not a fleet defect).
  Regression tests reconstruct the incident's distribution (418 runs,
  median 4077 s, p90 13939 s, 190/418 = 45% longer than the 75 min
  threshold) and assert it.
