//! Local-gate coverage: a gate is generated from CI's steps, and a pass is
//! only evidence at the coverage it actually has (issue #4304).
//!
//! Issue #4197 established that a job's name is documentation and its `run:`
//! steps are the contract, and shipped [`crate::ci_name_drift`] to compare a
//! gate against them. Two months later the same author made the same mistake
//! with the correct rule in front of them, which is the finding: comparing a
//! gate to the steps is only half of it. The other half is what a gate says
//! about itself when it runs a subset.
//!
//! The incident: the CI job was named `"Rust baseline (fmt / clippy /
//! test)"` and ran **seven** steps — 1 `cargo fmt --all --check`,
//! 2 `cargo clippy --workspace --all-targets`, 3 `cargo test --workspace`,
//! 4 the `--ignored` tests that need a live Postgres, 5 a
//! `schema-gen --check` regenerate-and-diff over the bindings, 6 a pinned
//! digest check (`check-pinned-digests.sh`: lockfile pins vs the tarball
//! contents), 7 `test-ci-skeleton.sh`. The engineer ran steps 1-3 — the
//! three named in the label — and reported the gate clean. Step 7 caught the
//! defect; the fix was a two-line pin. Nothing in the tooling asked him which
//! of the seven steps had run.
//!
//! The primitives here close the four gaps that incident exposed:
//!
//! 1. **A local gate is generated from the job definition, never transcribed
//!    from its name** ([`derive_gate`], [`render_gate_script`]). When the
//!    gate cannot run everything, it prints the step list it ran alongside
//!    the list the job defines ([`Coverage::paired_lists`]) and fails on the
//!    mismatch ([`Coverage::equivalence`]) rather than reporting a clean pass.
//! 2. **A job name may not enumerate its steps** ([`name_policy`]). Naming
//!    three of seven is worse than naming none: it is an invitation to run
//!    three of seven and believe the gate. Either the name states no step
//!    list ([`NamePolicy::PurposeNamed`]) or it is generated from the steps
//!    ([`NamePolicy::GeneratedEnumeration`]); a partial enumeration is a
//!    finding naming the steps it omits ([`enumeration_is_a_trap`]).
//! 3. **"My local gate passed" is evidence only with its coverage**
//!    ([`challenge_claim`]). A complete-gate claim over a partial gate is
//!    [`ClaimVerdict::Overstated`] and is restated as `ran steps 1-3 of 7`
//!    ([`Coverage::statement`]); a coverage claim whose numbers do not match
//!    the measurement is [`ClaimVerdict::WrongCoverage`].
//! 4. **A cross-artifact drift check belongs in every gate that can
//!    regenerate the artifact** ([`artifact_drift`]). CI ran the digest check
//!    as step 6; the local gate dropped it, so the gate could not have found
//!    the bug it was built to catch. A contract that names a regenerating
//!    step without a drift check — or declares a check the gate never runs —
//!    is a finding, and the finding names the CI step that already runs it.

use crate::ci_name_drift::{derive_name, name_enumeration, CiJob};

/// Whitespace-only normalisation of a command line.
///
/// Comparison is exact apart from whitespace: stripping anything else
/// (a toolchain prefix, an argument order) would be guessing that two
/// different commands are the same command.
fn norm(cmd: &str) -> String {
    cmd.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Escape a string for use inside double quotes in a generated script.
fn shell_quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    for ch in text.chars() {
        match ch {
            '\\' | '"' | '$' | '`' => {
                out.push('\\');
                out.push(ch);
            }
            _ => out.push(ch),
        }
    }
    out
}

/// The generated local gate for a job: every `run:` command, in order.
///
/// This is the only admissible source for a gate's command list. A gate
/// written by reading the job's name, or by remembering what the job "does",
/// is a transcription, and a transcription is silently shorter than the
/// thing it copies (issue #4197: 3 of 4 steps; issue #4304: 3 of 7).
pub fn derive_gate(job: &CiJob) -> Vec<String> {
    job.command_list()
}

/// Render the generated gate as a shell script.
///
/// The script is the derivation made concrete: one echoed step header per CI
/// step, so the pass output carries its own coverage (`gate step 4/7: …`) and
/// a reader can see which step number is 4. The step list is not editable by
/// hand — regenerate it.
pub fn render_gate_script(job: &CiJob) -> String {
    let total = job.steps.len();
    let mut out = String::new();
    out.push_str("#!/usr/bin/env bash\n");
    out.push_str(&format!(
        "# GENERATED from CI job '{}' — do not edit the step list by hand.\n",
        shell_quote(&job.id)
    ));
    out.push_str(&format!(
        "# Steps: {total} (every `run:` step of the job, in order).\n"
    ));
    out.push_str("set -euo pipefail\n");
    for (idx, step) in job.steps.iter().enumerate() {
        let label = step.name.trim();
        let label = if label.is_empty() {
            format!("step {}", idx + 1)
        } else {
            label.to_string()
        };
        out.push_str(&format!(
            "echo \"gate step {}/{}: {}\"\n",
            idx + 1,
            total,
            shell_quote(&label)
        ));
        out.push_str(&format!("{}\n", step.run));
    }
    out
}

/// Which steps of a job a gate actually covers. Step numbers are 1-based, as
/// they are in the workflow file and in the pass output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Coverage {
    /// Job step numbers the gate runs.
    pub ran: Vec<usize>,
    /// Job step numbers the gate does not run.
    pub skipped: Vec<usize>,
    /// Gate commands no CI step runs: the gate diverges from CI.
    pub extra: Vec<String>,
}

impl Coverage {
    /// Whether the gate is the same gate as the job.
    ///
    /// `Same` is the only state in which "the gate is clean" means "CI is
    /// clean". Everything else must be reported with its coverage.
    pub fn equivalence(&self) -> GateEquivalence {
        if !self.extra.is_empty() {
            GateEquivalence::Divergent {
                skipped: self.skipped.clone(),
                extra: self.extra.clone(),
            }
        } else if self.skipped.is_empty() {
            GateEquivalence::Same
        } else {
            GateEquivalence::Partial {
                skipped: self.skipped.clone(),
            }
        }
    }

    /// The coverage statement: `ran steps 1-3 of 7 (skipped 4-7)`.
    ///
    /// This is what a partial gate says instead of "the gate is clean"
    /// (invariant 3). The total is the job's step count, never the gate's:
    /// a denominator the author chose is how the subset became invisible.
    pub fn statement(&self, job: &CiJob) -> String {
        let total = job.steps.len();
        let ran = format_step_ranges(&self.ran);
        match self.equivalence() {
            GateEquivalence::Same => format!("ran all {total} steps of job '{}'", job.id),
            GateEquivalence::Partial { skipped } => format!(
                "ran steps {} of {} (skipped {})",
                ran,
                total,
                format_step_ranges(&skipped)
            ),
            GateEquivalence::Divergent { skipped, extra } => format!(
                "ran steps {} of {} (skipped {}, and {} command(s) no CI step runs)",
                ran,
                total,
                format_step_ranges(&skipped),
                extra.len()
            ),
        }
    }

    /// The two lists, side by side: what the job defines and what the gate
    /// runs, per step.
    ///
    /// This is the fallback the invariant demands when full automation of
    /// the gate is not possible — print the step list the gate ran alongside
    /// the list CI defines, so the mismatch is on the screen rather than in
    /// the author's head.
    pub fn paired_lists(&self, job: &CiJob) -> Vec<String> {
        let mut lines = vec![format!(
            "job '{}' defines {} steps; this gate runs {}",
            job.id,
            job.steps.len(),
            self.ran.len()
        )];
        for (idx, step) in job.steps.iter().enumerate() {
            let n = idx + 1;
            let state = if self.ran.contains(&n) {
                "RAN"
            } else {
                "SKIPPED"
            };
            lines.push(format!("CI step {n:>2} [{state:>7}] {}", step.run));
        }
        for extra in &self.extra {
            lines.push(format!("GATE extra  [   ADDED] {extra}"));
        }
        lines
    }
}

/// Whether a gate is the CI gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateEquivalence {
    /// The gate runs exactly the job's steps.
    Same,
    /// The gate runs a strict subset: a partial gate, whose pass is scoped to
    /// the steps it ran.
    Partial { skipped: Vec<usize> },
    /// The gate runs commands the job does not, with or without omissions: it
    /// is a different program's gate, not a smaller run of the same one.
    Divergent {
        skipped: Vec<usize>,
        extra: Vec<String>,
    },
}

/// Measure which job steps a gate's command list covers.
///
/// Comparison is whitespace-normalised and positional-free: a gate that runs
/// the steps out of order still covers them, and a command the job never runs
/// is reported as `extra` rather than silently ignored.
pub fn coverage(gate_commands: &[String], job: &CiJob) -> Coverage {
    let mut pool: Vec<Option<String>> = gate_commands.iter().map(|c| Some(norm(c))).collect();
    let mut ran = Vec::new();
    let mut skipped = Vec::new();
    for (idx, step) in job.steps.iter().enumerate() {
        let want = norm(&step.run);
        let hit = pool
            .iter()
            .position(|slot| slot.as_deref() == Some(want.as_str()));
        match hit {
            Some(i) => {
                pool[i] = None;
                ran.push(idx + 1);
            }
            None => skipped.push(idx + 1),
        }
    }
    let extra: Vec<String> = gate_commands
        .iter()
        .zip(pool)
        .filter_map(|(raw, left)| left.map(|_| raw.clone()))
        .collect();
    Coverage {
        ran,
        skipped,
        extra,
    }
}

/// Format step numbers as ranges: `[1,2,3,5,7,8]` → `1-3, 5, 7-8`.
pub fn format_step_ranges(steps: &[usize]) -> String {
    if steps.is_empty() {
        return "none".into();
    }
    let mut sorted = steps.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    let mut parts: Vec<String> = Vec::new();
    let mut start = sorted[0];
    let mut prev = sorted[0];
    for n in &sorted[1..] {
        if *n == prev + 1 {
            prev = *n;
            continue;
        }
        parts.push(range_text(start, prev));
        start = *n;
        prev = *n;
    }
    parts.push(range_text(start, prev));
    parts.join(", ")
}

fn range_text(start: usize, end: usize) -> String {
    if start == end {
        start.to_string()
    } else {
        format!("{start}-{end}")
    }
}

/// What a gate's pass claim asserts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateClaim {
    /// "the gate is clean", "CI-equivalent", "green": a claim over the whole
    /// job. An unscoped or unparseable claim reads as one — the burden of
    /// scoping is on the claimant, not on the reader.
    GateIsClean,
    /// "ran steps 1-3 of 7": a claim scoped to the steps it names, with the
    /// total taken from the job so the subset is visible.
    RanSteps { ran: Vec<usize>, total: usize },
}

/// Classify a pass claim written in words.
///
/// A claim that states its coverage ("ran steps 1-3 of 7") is
/// [`GateClaim::RanSteps`]; anything else — including "the gate passed", "all
/// green", and anything the parser cannot read — is a complete-gate claim,
/// which is the conservative reading: an unscoped claim asserts the whole
/// job.
pub fn claim_from_words(words: &str) -> GateClaim {
    match parse_stated_coverage(words) {
        Some((ran, total)) => GateClaim::RanSteps { ran, total },
        None => GateClaim::GateIsClean,
    }
}

/// Parse `steps <ranges> of <total>` out of a claim.
fn parse_stated_coverage(words: &str) -> Option<(Vec<usize>, usize)> {
    let lower = words.to_lowercase();
    let at = lower.find("step")? + "step".len();
    let rest = lower[at..]
        .trim_start()
        .trim_start_matches('s')
        .trim_start();
    let first = rest.find(|c: char| c.is_ascii_digit())?;
    let rest = &rest[first..];
    let of_at = rest.find(" of ")?;
    let ran = parse_step_ranges(&rest[..of_at])?;
    let digits: String = rest[of_at + " of ".len()..]
        .trim_start()
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    let total = digits.parse::<usize>().ok()?;
    if total == 0 || ran.is_empty() {
        return None;
    }
    Some((ran, total))
}

/// Parse `1-3, 5, 7-8` into step numbers.
fn parse_step_ranges(text: &str) -> Option<Vec<usize>> {
    let mut out = Vec::new();
    for part in text.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        match part.split_once('-') {
            Some((a, b)) => {
                let (a, b) = (
                    a.trim().parse::<usize>().ok()?,
                    b.trim().parse::<usize>().ok()?,
                );
                if a > b || a == 0 {
                    return None;
                }
                out.extend(a..=b);
            }
            None => out.push(part.parse::<usize>().ok()?),
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// The verdict on a pass claim measured against the gate's real coverage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimVerdict {
    /// The claim is scoped exactly as wide as the measurement.
    Accepted,
    /// A complete-gate claim over a gate that ran a subset or diverged. Not
    /// false — unscoped. The restatement is the claim to make instead.
    Overstated { restatement: String },
    /// A coverage claim whose numbers differ from the measurement: the wrong
    /// steps, or a total the job does not have.
    WrongCoverage { actual: String },
}

impl ClaimVerdict {
    /// One-line rendering for a review or a closeout.
    pub fn line(&self) -> String {
        match self {
            Self::Accepted => "OK: pass claim is scoped to the coverage measured".into(),
            Self::Overstated { restatement } => {
                format!("WARN: 'the gate is clean' over a partial gate — say '{restatement}'")
            }
            Self::WrongCoverage { actual } => {
                format!("WARN: pass claim's step numbers do not match the gate — {actual}")
            }
        }
    }
}

/// Challenge a pass claim against the gate's measured coverage (invariant 3).
///
/// "My local gate passed" is only evidence if the gate is the same gate, and
/// the way to state that is coverage — `ran steps 1-3 of 7` — not "the gate
/// is clean".
pub fn challenge_claim(claim: &GateClaim, measured: &Coverage, job: &CiJob) -> ClaimVerdict {
    match claim {
        GateClaim::GateIsClean => match measured.equivalence() {
            GateEquivalence::Same => ClaimVerdict::Accepted,
            _ => ClaimVerdict::Overstated {
                restatement: measured.statement(job),
            },
        },
        GateClaim::RanSteps { ran, total } => {
            let mut want = measured.ran.clone();
            want.sort_unstable();
            want.dedup();
            let mut got = ran.clone();
            got.sort_unstable();
            got.dedup();
            if *total == job.steps.len() && got == want {
                ClaimVerdict::Accepted
            } else {
                ClaimVerdict::WrongCoverage {
                    actual: measured.statement(job),
                }
            }
        }
    }
}

/// What a job's display name does about its steps (invariant 2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NamePolicy {
    /// The name states no step list: it is a label for a purpose, and nobody
    /// can mistake it for the contract.
    PurposeNamed,
    /// The name lists every step of the job, exactly — it is generated from
    /// the steps, so it cannot fall behind them.
    GeneratedEnumeration,
    /// The name lists some steps and not others. The trap: a reader runs the
    /// named steps and reports the gate clean.
    PartialEnumeration {
        /// The step names the label lists, in the order parsed.
        named: Vec<String>,
        /// How many steps the label lists.
        named_count: usize,
        /// How many steps the job actually has.
        total_steps: usize,
        /// Steps the label does not list, by name, or `step N (unnamed)`.
        not_listed: Vec<String>,
        /// Labelled steps with no step behind them (the name ran ahead of the
        /// job, e.g. a step removed from CI but left in the label).
        unbacked: Vec<String>,
    },
}

impl NamePolicy {
    /// Whether the name is an invitation to run a subset and believe the gate.
    pub fn is_trap(&self) -> bool {
        matches!(self, Self::PartialEnumeration { .. })
    }

    /// One-line rendering: `OK:` for an admissible name, `WARN:` naming every
    /// omitted step for a trap.
    pub fn line(&self, job: &CiJob) -> String {
        match self {
            Self::PurposeNamed => format!("OK: job '{}' names no step list", job.id),
            Self::GeneratedEnumeration => {
                format!("OK: job '{}' name is generated from its steps", job.id)
            }
            Self::PartialEnumeration {
                named,
                named_count,
                total_steps,
                not_listed,
                unbacked,
            } => {
                let mut line = format!(
                    "WARN: job '{}' names {named_count} of its {total_steps} steps ({}) and omits {}",
                    job.id,
                    named.join(" / "),
                    not_listed.join(", ")
                );
                if !unbacked.is_empty() {
                    line.push_str(&format!(
                        "; label lists no such step: {}",
                        unbacked.join(", ")
                    ));
                }
                line.push_str(&format!(
                    " — naming some of the steps is worse than naming none: rename the job for its purpose, or generate the name ({})",
                    derive_name(job)
                ));
                line
            }
        }
    }
}

/// Decide whether a job's name may enumerate its steps (invariant 2).
///
/// Issue #4197 lint-checked an enumerating name *against* the steps. This is
/// the stronger rule: a name that enumerates only part of the job is a
/// finding even when every token it lists is a real step, because the harm is
/// the omission, not the mismatch. A name either lists nothing or is
/// generated from the steps.
pub fn name_policy(job: &CiJob) -> NamePolicy {
    let Some(named) = name_enumeration(&job.name) else {
        return NamePolicy::PurposeNamed;
    };
    let mut listed: Vec<bool> = vec![false; job.steps.len()];
    let mut unbacked: Vec<String> = Vec::new();
    for token in &named {
        let want = token.trim().to_lowercase();
        let hit = job
            .steps
            .iter()
            .enumerate()
            .position(|(i, step)| !listed[i] && step.name.trim().to_lowercase() == want);
        match hit {
            Some(i) => listed[i] = true,
            None => unbacked.push(token.clone()),
        }
    }
    let not_listed: Vec<String> = job
        .steps
        .iter()
        .enumerate()
        .filter(|(i, _)| !listed[*i])
        .map(|(i, step)| {
            let label = step.name.trim();
            if label.is_empty() {
                format!("step {} (unnamed)", i + 1)
            } else {
                label.to_string()
            }
        })
        .collect();
    if not_listed.is_empty() && unbacked.is_empty() {
        NamePolicy::GeneratedEnumeration
    } else {
        NamePolicy::PartialEnumeration {
            named,
            named_count: listed.iter().filter(|l| **l).count(),
            total_steps: job.steps.len(),
            not_listed,
            unbacked,
        }
    }
}

/// A malformed contract field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractError(&'static str);

impl ContractError {
    /// The empty field's name.
    pub fn field(&self) -> &'static str {
        self.0
    }
}

impl std::fmt::Display for ContractError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "`{}` must not be empty: a contract with an unnamed part checks nothing",
            self.0
        )
    }
}

impl std::error::Error for ContractError {}

/// A check that a regenerated artifact still matches the pin that records it
/// (a digest manifest, a lockfile integrity hash).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriftCheck {
    /// The command the gate must run.
    pub command: String,
    /// The failure the check exists to catch — why it may not be dropped as
    /// noise (`npm error code EINTEGRITY inside the web image publish job,
    /// far from its cause`).
    pub rationale: String,
}

impl DriftCheck {
    /// Construct a drift check. A check with no written rationale is refused:
    /// a cross-artifact check looks like noise to the next person who is in a
    /// hurry, and the rationale is what keeps it in the gate.
    pub fn new(command: &str, rationale: &str) -> Result<Self, ContractError> {
        if command.trim().is_empty() {
            return Err(ContractError("command"));
        }
        if rationale.trim().is_empty() {
            return Err(ContractError("rationale"));
        }
        Ok(Self {
            command: command.to_string(),
            rationale: rationale.to_string(),
        })
    }
}

/// The contract between a gate, an artifact it can regenerate, and the place
/// that artifact's value is pinned (invariant 4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactContract {
    /// The artifact regenerated (`inferweave-bindings-0.1.0.tgz`).
    pub artifact: String,
    /// The file that pins it (`apps/web/package-lock.json`).
    pub pin_file: String,
    /// The failure this contract prevents, in the terms it was found in.
    pub failure_mode: String,
    /// 1-based gate step numbers that regenerate the artifact.
    pub regenerated_by: Vec<usize>,
    /// The check that compares the regenerated artifact to its pin.
    pub drift_check: Option<DriftCheck>,
}

impl ArtifactContract {
    /// Construct a contract. The artifact, the pin, and the failure mode are
    /// all mandatory: `failure_mode` is the reason the check exists and the
    /// only thing that survives as guidance once the incident is closed.
    pub fn new(
        artifact: &str,
        pin_file: &str,
        failure_mode: &str,
        regenerated_by: Vec<usize>,
        drift_check: Option<DriftCheck>,
    ) -> Result<Self, ContractError> {
        if artifact.trim().is_empty() {
            return Err(ContractError("artifact"));
        }
        if pin_file.trim().is_empty() {
            return Err(ContractError("pin_file"));
        }
        if failure_mode.trim().is_empty() {
            return Err(ContractError("failure_mode"));
        }
        Ok(Self {
            artifact: artifact.to_string(),
            pin_file: pin_file.to_string(),
            failure_mode: failure_mode.to_string(),
            regenerated_by,
            drift_check,
        })
    }
}

/// Whether a gate that regenerates an artifact also checks it against its pin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtifactDrift {
    /// The gate regenerates nothing covered by this contract: no obligation.
    NotRegenerated,
    /// The gate regenerates the artifact and runs the drift check.
    Covered { command: String },
    /// The gate regenerates the artifact and no drift check is declared for
    /// it: the pin can move out of agreement with the artifact and nothing
    /// here will notice.
    MissingCheck {
        artifact: String,
        pin_file: String,
        regenerated_by: Vec<usize>,
    },
    /// A drift check exists for the artifact but the gate does not run it —
    /// the cheapest kind of loss, because the check is already written (and
    /// when `ci_step` is set, CI already runs it at that step).
    CheckNotRun {
        artifact: String,
        command: String,
        ci_step: Option<usize>,
    },
}

impl ArtifactDrift {
    /// One-line rendering for a gate report.
    pub fn line(&self, contract: &ArtifactContract) -> String {
        match self {
            Self::NotRegenerated => format!(
                "OK: gate does not regenerate `{}`",
                contract.artifact
            ),
            Self::Covered { command } => format!(
                "OK: gate regenerates `{}` and runs `{command}` against {}",
                contract.artifact, contract.pin_file
            ),
            Self::MissingCheck {
                artifact,
                pin_file,
                regenerated_by,
            } => format!(
                "WARN: gate regenerates `{}` at step(s) {} but runs no cross-artifact drift check against {pin_file} — the failure it prevents: {}",
                artifact,
                format_step_ranges(regenerated_by),
                contract.failure_mode
            ),
            Self::CheckNotRun {
                artifact,
                command,
                ci_step,
            } => format!(
                "WARN: drift check `{command}` for `{}` is declared and not run by this gate ({}) — the failure it prevents: {}",
                artifact,
                ci_step
                    .map(|n| format!("CI runs it at step {n}"))
                    .unwrap_or_else(|| "CI does not run it either".into()),
                contract.failure_mode
            ),
        }
    }
}

/// Decide whether a gate covers the cross-artifact drift of one contract
/// (invariant 4).
///
/// The rule is not "CI has the check" — CI had it at step 6 and the local
/// gate dropped it. The rule is: a gate that can regenerate an artifact whose
/// value is pinned elsewhere must run the pin check itself, or it cannot find
/// the bug the check was written for.
pub fn artifact_drift(
    job: &CiJob,
    gate_commands: &[String],
    contract: &ArtifactContract,
) -> ArtifactDrift {
    if contract.regenerated_by.is_empty() {
        return ArtifactDrift::NotRegenerated;
    }
    let Some(check) = &contract.drift_check else {
        return ArtifactDrift::MissingCheck {
            artifact: contract.artifact.clone(),
            pin_file: contract.pin_file.clone(),
            regenerated_by: contract.regenerated_by.clone(),
        };
    };
    let want = norm(&check.command);
    if gate_commands.iter().any(|c| norm(c) == want) {
        return ArtifactDrift::Covered {
            command: check.command.clone(),
        };
    }
    let ci_step = job
        .command_list()
        .iter()
        .position(|c| norm(c) == want)
        .map(|i| i + 1);
    ArtifactDrift::CheckNotRun {
        artifact: contract.artifact.clone(),
        command: check.command.clone(),
        ci_step,
    }
}

/// Audit a local gate against its CI job: coverage, name policy, and every
/// artifact contract, one line each.
///
/// The last line is the verdict a reviewer reads. It never says the gate is
/// clean when a step was skipped: the pass output and the claim are the same
/// artifact here.
pub fn audit(job: &CiJob, gate_commands: &[String], contracts: &[ArtifactContract]) -> Vec<String> {
    let measured = coverage(gate_commands, job);
    let mut lines = Vec::new();
    let statement = measured.statement(job);
    match measured.equivalence() {
        GateEquivalence::Same => lines.push(format!("OK: {statement}")),
        _ => {
            lines.push(format!("WARN: {statement}"));
            lines.extend(measured.paired_lists(job).into_iter().skip(1));
        }
    }
    lines.push(name_policy(job).line(job));
    let mut findings = usize::from(name_policy(job).is_trap());
    if measured.equivalence() != GateEquivalence::Same {
        findings += 1;
    }
    for contract in contracts {
        let drift = artifact_drift(job, gate_commands, contract);
        if matches!(
            drift,
            ArtifactDrift::MissingCheck { .. } | ArtifactDrift::CheckNotRun { .. }
        ) {
            findings += 1;
        }
        lines.push(drift.line(contract));
    }
    if findings == 0 {
        lines.push("OK: the gate is the CI gate — every step runs, no step is missing, no regenerated artifact is unchecked".into());
    } else {
        lines.push(format!(
            "WARN: {findings} finding(s): a pass here is not a CI pass — report coverage as '{}'",
            measured.statement(job)
        ));
    }
    lines
}
