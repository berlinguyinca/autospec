# Readiness loops must name their actuator (issue #4268)

The InferWeave frontier loop recomputed which issues had every
dependency closed and reported "8 ready, 0 dispatched, 8 blocked: no
staged spec" — then ran again 30 minutes later with the same numbers.
`iw-dispatch.sh`, the actuator the loop was feeding, requires a staged
spec at `iw/issues/<n>.md`, and no component in the loop produced that
file, so the loop could never dispatch: a readiness computation with
no actuator, reporting decisions it could not execute, with counts
that read as steady progress over a loop that was dead.

- **A loop that reports decisions must report whether it could
  execute them.** The report carries the actuator's status next to
  the ready/dispatched/blocked counts (`ActuatorStatus`:
  `CouldExecute`, or `CannotExecute` naming the unmet precondition and
  its owner). A report with decisions and no stated actuator status is
  a finding (`ACTUATION_NOT_REPORTED`); `CouldExecute` with
  `dispatched < ready` is a dispatch defect (`ACTUATION_GAP`); and
  counts that do not reconcile (`dispatched + blocked > ready`) are a
  state that cannot exist (`REPORT_DOES_NOT_RECONCILE`). "8 ready,
  0 dispatched" without an actuator line is unfalsifiable in the same
  way a bare `candidates=0` is — the denominator here is whether the
  decision was executable at all.
- **Every precondition the actuator enforces must have an owner that
  satisfies it.** An unsatisfied precondition with no owner
  (`OWNERLESS_PRECONDITION`) is a permanent block reporting itself as
  normal operation — the same class of error as a guard with no named
  release ("Stored-output lifecycle and blocker escalation" above).
  The durable fix names the owner in the loop: `iw-stage.sh`, called
  from the frontier loop, stages the spec; an owned hold renders with
  the owner, so the block is visible as temporary
  (`Precondition::hold_line`).
- **A readiness loop is not working until it has been observed to
  dispatch.** `EndToEndEvidence` — passes with zero dispatches are
  compute, not act, and the audit says so (`NOT_OBSERVED_TO_ACT`)
  instead of letting the loop age into "steady operation". The
  regression tests run the fixed loop end-to-end once (staging step
  in the path, dispatch observed) and assert the audit is empty — a
  test that only checks the report cannot see the loop, the way a
  homogeneous test cannot see the heterogeneous bug.

Checkable in `autospec_core::loop_actuation` (`LoopReport`,
`ActuatorStatus`, `actuation_findings`, `Precondition`,
`precondition_findings`, `EndToEndEvidence`, `audit` — pure
in-memory). Tests:
`crates/autospec-core/tests/loop_actuation.rs`, including the
regression that reconstructs the incident pass (8 ready, 0
dispatched, 8 blocked on an ownerless precondition, 40
never-dispatched passes) and the fixed loop with the staging step
called from the frontier.
