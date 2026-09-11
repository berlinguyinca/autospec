### Added

- `autospec_core::toolchain_gate`: a CI gate must run the toolchain the
  repository declared in `rust-toolchain.toml`, never an override the step
  typed in; drift is directional (a step on a newer or older channel than the
  pin is an upgrade made by accident, not "stable is fine"); a gate that has
  never passed is a defect report, not a quality signal; a pin bump is a
  recorded decision with work attached, not an observation; and a standing
  instruction to ignore a gate with no filed bug report is itself the bug
  report. `TOOLCHAIN_GATE_DRIFT` is the audit finding id. (issue #4303)
