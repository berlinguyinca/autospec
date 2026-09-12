# CI name drift: the steps are the contract, not the name (issue #4197)

A job's display name is documentation; its steps are the contract. The
incident: a conversion gate was built to "match CI" by reading a CI job's
name — `"Next.js baseline (lint / typecheck / build)"` — to learn what it
runs. The name enumerates three of the job's four steps and omits `test`, so
the gate derived from the name ran lint, typecheck and build and silently
skipped the test step: 68 tests never ran, and nothing warned, because
nothing compared the name to the steps it claimed to list.

- **Never derive behaviour from a label.** The only source of a job's
  commands is its `run:` steps, in order (`CiJob::command_list`). The name
  is documentation that may be wrong and is never parsed for commands.
- **A local gate is generated from the CI definition, not restated beside
  it.** `gate_matches_job(gate, job)` is the mechanical assertion "does my
  gate's command list equal the job's step list?". A gate missing a step is
  `GateDrift::OutOfSync` with the skipped step named; its line is a `WARN:`,
  so the check is an assertion, not a habit.
- **An enumerating name is tested against what it enumerates.**
  `name_enumeration` parses the last `(...)` group out of the name; `name_matches_steps`
  asserts the list matches the steps' names (case-insensitive, order-
  insensitive). A name that omits a step is `NameDrift::OutOfSync` with the
  omitted step named — the check that would have caught the incident.
- **When a gate is extended, re-derive the name — do not edit it.**
  `derive_name` regenerates the name from the steps, in order; `name_is_stale`
  is `true` when a hand-edited name no longer equals what the steps derive
  to. `audit(gate, job)` is the lint a gate-definition change triggers, and a
  drifted name is a `WARN:` on the same pass the breakage is introduced —
  never a silent re-version.
- **Regression tests run in the configuration the bug required.** The bug
  required an enumerating name that omits a step; on a job whose name
  matches its steps every check agrees and the bug is invisible. `tests/ci_name_drift.rs`
  instantiates the drifted-name job (the incident) and a matching-name
  control that proves the checks are not false-positiving.

Checkable in `autospec_core::ci_name_drift` (`CiJob::command_list`,
`gate_matches_job`, `name_enumeration`, `name_matches_steps`, `derive_name`,
`name_is_stale`, `audit`). Tests: `crates/autospec-core/tests/ci_name_drift.rs`.
