# Gate coverage: a partial gate's pass is not a CI pass (issue #4304)

The #4197 job was named `"Rust baseline (fmt / clippy / test)"` and ran
**seven** steps. The engineer wrote a three-command local gate, ran it green,
and reported the gate clean. #4197 made sure the *name* matched the *steps*;
here the name matched its own three tokens perfectly, and the gate still ran
four steps short. The label was never the problem — the gate's pass was
reported as if it were the CI job's pass.

- **Generate the gate, never transcribe it.** A local gate is
  `derive_gate(job)` — the job's `run:` steps in order — and
  `render_gate_script(job)` emits it with a `gate step N/total:` line before
  each command so the script carries its own coverage on the console. A gate
  written by hand from a description is a second source of truth about what CI
  does, and the two diverge in the direction that hides work.
- **A name may not enumerate steps at all.** #4197 lint-checked an enumerating
  name *against* the steps and passed when every token was a real step. That
  is the weaker rule: `"fmt / clippy / test"` are three real steps and the
  name is still the trap, because the harm is the **omission** — a reader runs
  what is written. A name either states no step list (`PurposeNamed`) or is
  generated from the steps (`GeneratedEnumeration`). `PartialEnumeration` is a
  finding even when every token is backed, and a token with no step behind it
  (a step removed from CI, left in the label) is named in `unbacked`.
- **A pass states its coverage or it is rejected.** "The gate is clean" is a
  claim about a different program than the one under review. `coverage(job,
  gate)` measures the gate against the job and `Coverage::statement` renders
  `ran steps 1-3 of 7 (skipped 4-7)`; `challenge_claim` rejects
  `GateClaim::GateIsClean` over a partial run as `Overstated` with the
  restatement in hand, and rejects a `RanSteps` claim whose indices don't match
  the measurement as `WrongCoverage`. A gate that runs a command no CI step
  runs is `Divergent` — it proves something CI does not.
- **A gate that regenerates an artifact must carry the drift check.**
  `ArtifactContract` names the regenerated artifact, the file that pins it, and
  the failure the check prevents; `artifact_drift` returns `MissingCheck` when
  the gate regenerates without checking, and `CheckNotRun` naming the CI step
  that does run the check when the declared check is absent from the gate. The
  failure it prevents is written into the finding, so the operator sees the
  EINTEGRITY bug at the gate rather than at publish time.
- **Regression tests run in the configuration the bug required.** `incident_job()`
  is the 7-step job with the 3-token name, `incident_gate()` is the 3-command
  gate; the control is a purpose-named job whose gate runs every step.

Checkable in `autospec_core::gate_coverage` (`derive_gate`,
`render_gate_script`, `coverage`, `Coverage::{equivalence, statement,
paired_lists}`, `format_step_ranges`, `claim_from_words`, `challenge_claim`,
`name_policy`, `DriftCheck::new`, `ArtifactContract::new`, `artifact_drift`,
`audit`). Tests: `crates/autospec-core/tests/gate_coverage.rs`.
