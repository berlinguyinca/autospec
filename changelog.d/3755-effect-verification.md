### Added

- `autospec-core::effect_verification` — primitives for the invariants
  that a change to a running system is verified by observing its effect,
  never by re-reading the edit: only a probe that runs the thing, names
  the signal a healthy system produces, and completes inside the 60-second
  window answers the design-review question "how will I know within sixty
  seconds if it is not" (`Probe::answers_within_window`, `ship_verdict`,
  `plan_findings`, `sixty_second_answer`, `VERIFY_WINDOW_SECONDS`), and a
  change that cannot answer it ships only against an idle system
  (`ShipVerdict::WaitForIdle`, with the `REREAD_ONLY` and
  `NO_EFFECT_PROBE` findings); a verification that returns nothing is a
  failure, not a pass (`run_probe` -> `ProbeOutcome::NoSignal`,
  `outcome_line`), and a probe without a named signal fails closed
  (`PROBE_NO_SIGNAL`); the blast radius of a change to shared state is
  stated before the edit — who reads the path, what readers do if it
  changes mid-read, what makes the change exclusive (issue #3732)
  (`BlastRadius`, `blast_radius_findings`); a gate matches the
  repository's definition or it is a different gate, stricter *or* looser
  (issue #3753) (`GateApplication`, `ad_hoc_gate_findings`); and the
  verifiability filter runs before the quality ranking, so the better
  change that cannot be cheaply verified is never chosen
  (`choose_change`) (#3755, 2026-09-11).
