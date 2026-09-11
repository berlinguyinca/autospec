### Added

- `autospec_core::loop_actuation`: a readiness computation with no
  actuator is decoration (issue #4268). The InferWeave frontier loop
  reported `8 ready, 0 dispatched, 8 blocked: no staged spec` on every
  30-minute pass and never dispatched, because `iw-dispatch.sh`
  requires a staged spec at `iw/issues/<n>.md` and no component in the
  loop produced that file. Encodes the fix as checkable primitives —
  `LoopReport` (ready/dispatched/blocked counts *and* the actuator
  status, where `None` is the incident: decisions reported without
  stating whether they could be executed), `actuation_findings`
  (`ACTUATION_NOT_REPORTED` for decisions without a stated actuator,
  `ACTUATION_GAP` for `CouldExecute` with `dispatched < ready`, and
  `REPORT_DOES_NOT_RECONCILE` for counts that describe a state that
  cannot exist), `Precondition` / `precondition_findings` (every
  precondition the actuator enforces must have an owner that satisfies
  it — `OWNERLESS_PRECONDITION` is the permanent block reporting itself
  as normal operation, while an owned hold renders via
  `Precondition::hold_line` with its release path), and
  `EndToEndEvidence` (a readiness loop is not working until it has
  been observed to dispatch — `loop_working()`, with `NOT_OBSERVED_TO_ACT`
  in the combined `audit` — passes with zero dispatches are compute,
  not act).
