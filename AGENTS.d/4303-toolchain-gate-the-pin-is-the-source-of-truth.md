# Toolchain gate: the pin is the source of truth (issue #4303)

A CI step that passes `toolchain: stable` to
`actions-rust-lang/setup-rust-toolchain` does not select a toolchain: it
overrides the one the repository declared in `rust-toolchain.toml`. This
repository pins 1.91.0 — the only toolchain on the HPC cluster the agents run
on, and the version every lint in the gate was written against — while `stable`
floats past it. The incident: the gate ran on `stable`, where two lints that
do not exist in 1.91.0 (`manual_checked_division`,
`truncating_to_zero_length`) fired, and the gate was red on **every** input.
A gate that fails on every input is not measuring anything; it is a defect
report about the gate, and the standing instruction "ignore clippy failures"
was the diagnosis nobody filed. Five invariants, all checkable:

- **The pin is the single source of truth.** A workflow step that names a
  channel different from `rust-toolchain.toml` is `TOOLCHAIN_GATE_DRIFT`,
  reported with the workflow path, the step id, and both channels. A step
  that names the pinned version exactly is not drift — the pin says 1.91.0
  and the step says 1.91.0, so they agree. A step that names no channel
  inherits the pin and is clean.
- **Drift is directional.** A step on a newer channel than the pin is an
  *upgrade made by accident* — it changes the toolchain with no decision
  record and no re-validation of the gate's lint set. A step on an older
  channel is the drift the repository no longer declares. The two have
  different remediations and must be reported differently.
- **A gate that has never passed is a defect report, not a quality
  signal.** `gate_standing` classifies a gate's run history: zero passes in
  N runs is `DefectReport { runs: N }` — the gate is broken, file it — while
  any pass is a `QualitySignal` whose green rate is the measurement. An
  unobserved gate (no runs) is `Unobserved`, never `DefectReport`.
- **A pin bump is a recorded decision, not an observation.** Changing
  `rust-toolchain.toml` from 1.91.0 to 1.93.0 without a decision record
  (issue, PR discussion, or commit message saying why) is an
  `UnrecordedBump`: the toolchain moved, the gate's lint set moved with it,
  and nobody decided. `pin_change_verdict` distinguishes `Unchanged`,
  `RecordedBump`, and `UnrecordedBump { direction }` — an unchanged pin is
  not a decision, and a recorded bump is.
- **A standing instruction to ignore a gate is a bug report.** A workaround
  note ("clippy failures are expected; ignore") with no filed issue is
  `UnfiledBugReport` — the workaround is the diagnosis, and the fix is to
  file it. With a filed issue (e.g. `InferWeave/inferweave#323`), the
  workaround is `Filed` and clean.
- **A channel this cannot classify is refused, not guessed.**
  `parse_channel` accepts `major[.minor[.patch]]`, `stable`/`beta`/
  `nightly`, and a `-target-triple` suffix on any of them
  (`stable-x86_64-unknown-linux-gnu` — the triple is ignored, because it is
  not the toolchain). Anything else — `1.91.0-beta.2`, `1.9.1.0`, `weird` —
  is an error, because a wrong guess here is a wrong toolchain running the
  gate, which is the incident this module exists to prevent. Same refusal
  discipline as #4192: name what would tell you.

Checkable in `autospec_core::toolchain_gate` (`PinFile::from_toml`,
`parse_channel`, `pin_drift`, `drift_line`, `gate_standing`, `standing_line`,
`pin_change_verdict`, `workaround_verdict`, `audit`). Tests:
`crates/autospec-core/tests/toolchain_gate.rs`, including the regression
that reconstructs the incident end-to-end: the drifting step, the
never-passed gate, the unfiled workaround, and the clean post-fix audit.
