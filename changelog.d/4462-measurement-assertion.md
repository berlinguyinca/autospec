### Added

- `autospec-core::measurement_assertion` — primitives for the invariant
  that a measurement must assert that it measured (issue #4462: four
  measurement bugs in one session, all in throwaway verification code, all
  reading as the reassuring answer). Four operations, each with a concrete
  assert:
  - `capture_verdict` / `CaptureVerdict` / `CaptureEvidence` — a parse of
    an output file asserts the file was written by this invocation:
    removed before the run, or mtime strictly after the start; anything
    else is fail-closed `Stale`, and an unreadable mtime is not a proof.
  - `Tally` / `TallyArityError` — `ok + failed == attempted`, so a
    counter that cannot account for every attempt is rejected rather than
    printed; `tally_contradiction` catches the case two arithmetically
    valid tallies in the same report disagree (`ok=0 failed=10` beside
    `non-empty content: 10/10`).
  - `marked_run` — a run's verdict from its completion marker: absence
    yields `GateVerdict::NotMeasured`, never pass and never fail.
  - `PersistenceGate` — a verdict from a sampled time series becomes
    actionable only after N consecutive sustaining samples (`new(1)` is
    rejected: one window is never sufficient); any non-sustaining sample
    resets the streak.
