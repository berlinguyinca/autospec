//! CI job name/step drift (issue #4197).
//!
//! A job's display name is documentation; its steps are the contract. The
//! incident: a conversion gate was built to "match CI" by reading a CI job's
//! name — `"Next.js baseline (lint / typecheck / build)"` — to learn what it
//! runs. The name enumerates three of the job's four steps; it omits `test`.
//! The gate was derived from the name, so it ran lint, typecheck and build
//! and silently skipped the test step: 68 tests never ran, and nothing
//! warned, because nothing compared the name to the steps it claimed to
//! list.
//!
//! Four invariants, each a primitive here:
//!
//! 1. **A job's display name is documentation; its steps are the contract.
//!    Never derive behaviour from a label.** [`CiJob::command_list`] is the
//!    only source of a job's commands — the `run:` steps, in order. The name
//!    is never parsed for commands; it is documentation that may be wrong.
//! 2. **A local gate must be generated from the CI definition, not restated
//!    beside it.** [`gate_matches_job`] is the mechanical assertion "does my
//!    gate's command list equal the job's step list?": a gate that is missing
//!    a step is [`GateDrift::OutOfSync`] with the skipped step named, and
//!    its line is a `WARN:` — an assertion, not a habit.
//! 3. **A name that enumerates should be tested against what it
//!    enumerates.** [`name_enumeration`] parses the parenthetical list out
//!    of the name; [`name_matches_steps`] asserts the list matches the
//!    steps' names. A name that omits a step is [`NameDrift::OutOfSync`]
//!    with the omitted step named — this is the check that would have caught
//!    the incident.
//! 4. **When a gate is extended, re-derive the name — do not edit it.**
//!    [`derive_name`] regenerates the name from the steps, in order;
//!    [`name_is_stale`] is `true` when a hand-edited name no longer equals
//!    what the steps derive to.
//!
//! [`audit`] is the runtime half: on each gate-definition change it emits
//! the gate-vs-steps line (invariant 2), the name-vs-steps line (invariant
//! 3), and the re-derive hint (invariant 4). A drifted name is a `WARN:` on
//! the same pass the breakage is introduced, never a silent re-version.
//!
//! Everything here is pure: no I/O, no subprocesses. The caller supplies the
//! parsed [`CiJob`] (from the workflow YAML) and the local gate's command
//! list, and reports the result.

use serde::{Deserialize, Serialize};

/// One step of a CI job, as parsed from the workflow. The `run` command is
/// the contract; the `name` is documentation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CiStep {
    /// The step's display name. May be empty when the step has no name — an
    /// unnamed step is out of scope for name-matching (it cannot be
    /// enumerated by name) but is still bound by the gate check, which
    /// compares `run` commands.
    pub name: String,
    /// The `run:` command — the contract the step performs.
    pub run: String,
}

/// A CI job as parsed from the workflow. [`CiJob::steps`] are the contract;
/// [`CiJob::name`] is a label.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CiJob {
    /// The job's key under `jobs:` (e.g. `"baseline"`).
    pub id: String,
    /// The display name (the label — documentation, never the contract).
    pub name: String,
    /// The steps, in order — the contract.
    pub steps: Vec<CiStep>,
}

impl CiJob {
    /// The contract (invariant 1): the `run:` commands, in order. A local
    /// gate is built from this list — never from [`CiJob::name`]. The name
    /// is deliberately not parsed here: it is a label that may be wrong, and
    /// a command list derived from it is exactly how this incident's `test`
    /// step was skipped.
    pub fn command_list(&self) -> Vec<String> {
        self.steps.iter().map(|s| s.run.clone()).collect()
    }
}

/// The drift between a local gate's command list and a job's step command
/// list (invariant 2). A gate that is out of sync silently skips or adds
/// steps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GateDrift {
    /// The gate's command list equals the job's step command list.
    InSync,
    /// The gate's command list differs from the job's steps.
    OutOfSync {
        /// Commands the job runs that the gate does not — steps the gate
        /// silently skips. This is the incident: the `run` command of the
        /// `test` step was here, and 68 tests were never run.
        missing_from_gate: Vec<String>,
        /// Commands the gate runs that the job does not.
        extra_in_gate: Vec<String>,
    },
}

impl GateDrift {
    /// The line a lint prints for this drift. `InSync` is an `OK:`;
    /// `OutOfSync` is a `WARN:` that names the job, the skipped steps, and
    /// the added steps, so the operator sees exactly which step the gate
    /// dropped — not that "something changed".
    pub fn line(&self, job: &CiJob) -> String {
        match self {
            GateDrift::InSync => format!(
                "OK: gate for job '{}' runs its {} step(s)",
                job.id,
                job.steps.len()
            ),
            GateDrift::OutOfSync {
                missing_from_gate,
                extra_in_gate,
            } => {
                let mut line = format!(
                    "WARN: gate for job '{}' does not match its {} step(s)",
                    job.id,
                    job.steps.len()
                );
                if !missing_from_gate.is_empty() {
                    line.push_str(&format!("; gate skips: {}", missing_from_gate.join(", ")));
                }
                if !extra_in_gate.is_empty() {
                    line.push_str(&format!("; gate adds: {}", extra_in_gate.join(", ")));
                }
                line
            }
        }
    }
}

/// Multiset subtraction, in order: the elements of `need` not covered by
/// `have`, preserving `need`'s order and accounting for multiplicity. A
/// command appearing twice in `need` and once in `have` is still reported as
/// missing once.
fn multiset_missing(have: &[String], need: &[String]) -> Vec<String> {
    let mut pool: Vec<String> = have.to_vec();
    let mut missing = Vec::new();
    for n in need {
        if let Some(pos) = pool.iter().position(|h| h == n) {
            pool.remove(pos);
        } else {
            missing.push(n.clone());
        }
    }
    missing
}

/// The assertion of invariant 2: does the local gate's command list equal
/// the job's step command list? The gate is compared against
/// [`CiJob::command_list`] — the steps — never against [`CiJob::name`]. A
/// gate missing a step is [`GateDrift::OutOfSync`] with the skipped step
/// named; the caller (the lint) turns that into a `WARN:` and blocks, so the
/// check is an assertion, not a habit.
pub fn gate_matches_job(gate_commands: &[String], job: &CiJob) -> GateDrift {
    let steps = job.command_list();
    let missing_from_gate = multiset_missing(gate_commands, &steps);
    let extra_in_gate = multiset_missing(&steps, gate_commands);
    if missing_from_gate.is_empty() && extra_in_gate.is_empty() {
        GateDrift::InSync
    } else {
        GateDrift::OutOfSync {
            missing_from_gate,
            extra_in_gate,
        }
    }
}

/// The tokens an enumerating name claims the job runs (invariant 3): the
/// contents of the last `(...)` group in the name, split on `/` and trimmed.
/// `None` when the name carries no parenthetical enumeration — a name that
/// does not enumerate is `NameDrift::NotEnumerating`, not checked by
/// [`name_matches_steps`].
pub fn name_enumeration(name: &str) -> Option<Vec<String>> {
    let open = name.rfind('(')?;
    let close = name.rfind(')')?;
    if close <= open {
        return None;
    }
    let tokens: Vec<String> = name[open + 1..close]
        .split('/')
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect();
    if tokens.is_empty() {
        None
    } else {
        Some(tokens)
    }
}

/// The drift between an enumerating job name and the steps it enumerates
/// (invariant 3). The name's enumeration is matched against the lowercased
/// names of the steps that have names; comparison is case-insensitive and
/// order-insensitive (multiset). This is the lint that would have caught the
/// incident: the name omitted `test`, and this reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NameDrift {
    /// The name carries no parenthetical enumeration: nothing to check.
    NotEnumerating,
    /// The name enumerates its steps and the enumeration matches them.
    InSync,
    /// The name enumerates its steps and the enumeration does not match them.
    OutOfSync {
        /// The tokens the name claims, lowercased, in order.
        named: Vec<String>,
        /// The identifiers of the named steps, lowercased, in order.
        steps: Vec<String>,
        /// Steps the job runs that the name does not list — the incident's
        /// `test`.
        missing: Vec<String>,
        /// Tokens the name lists that no named step backs.
        extra: Vec<String>,
    },
}

/// The lowercased names of the steps that have names, in order. Empty-named
/// steps are excluded: they cannot be enumerated by name (the gate check is
/// what binds them).
fn step_names_lowercase(job: &CiJob) -> Vec<String> {
    job.steps
        .iter()
        .map(|s| s.name.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .collect()
}

impl NameDrift {
    /// The line a lint prints for this drift. `NotEnumerating` and `InSync`
    /// are `OK:`; `OutOfSync` is a `WARN:` that names the job, the full
    /// enumeration, the steps, and the omitted / unbacked entries.
    pub fn line(&self, job: &CiJob) -> String {
        match self {
            NameDrift::NotEnumerating => format!(
                "OK: name of job '{}' does not enumerate its steps; nothing to check",
                job.id
            ),
            NameDrift::InSync => format!(
                "OK: name of job '{}' enumerates its steps and matches them",
                job.id
            ),
            NameDrift::OutOfSync {
                named,
                steps,
                missing,
                extra,
            } => {
                let mut line = format!(
                    "WARN: name of job '{}' enumerates [{}] but the steps are [{}]",
                    job.id,
                    named.join(", "),
                    steps.join(", ")
                );
                if !missing.is_empty() {
                    line.push_str(&format!("; name omits: {}", missing.join(", ")));
                }
                if !extra.is_empty() {
                    line.push_str(&format!("; name lists with no step: {}", extra.join(", ")));
                }
                line
            }
        }
    }
}

/// The assertion of invariant 3: does the name's enumeration match the
/// steps' names? A name that omits a step is [`NameDrift::OutOfSync`] with
/// the omitted step named.
pub fn name_matches_steps(job: &CiJob) -> NameDrift {
    let Some(named) = name_enumeration(&job.name) else {
        return NameDrift::NotEnumerating;
    };
    let steps = step_names_lowercase(job);
    let named_lc: Vec<String> = named.iter().map(|s| s.to_lowercase()).collect();
    let missing = multiset_missing(&named_lc, &steps);
    let extra = multiset_missing(&steps, &named_lc);
    if missing.is_empty() && extra.is_empty() {
        NameDrift::InSync
    } else {
        NameDrift::OutOfSync {
            named: named_lc,
            steps,
            missing,
            extra,
        }
    }
}

/// Re-derive the job's display name from its steps (invariant 4): keep the
/// prefix before the last parenthetical group and regenerate the group from
/// the steps' names, in order. A gate extended by a new step is a gate whose
/// name is re-derived, not hand-edited — a hand-edited name is how this
/// incident's `test` stayed out of the enumeration.
pub fn derive_name(job: &CiJob) -> String {
    let has_group = job
        .name
        .rfind('(')
        .is_some_and(|open| job.name.rfind(')').is_some_and(|close| close > open));
    let prefix = if has_group {
        let open = job.name.rfind('(').unwrap();
        job.name[..open].trim_end().to_string()
    } else {
        job.name.trim_end().to_string()
    };
    let step_names: Vec<String> = job
        .steps
        .iter()
        .map(|s| s.name.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if step_names.is_empty() {
        return prefix;
    }
    format!("{} ({})", prefix, step_names.join(" / "))
}

/// `true` when an enumerating name no longer equals what the steps derive to
/// (invariant 4): the name was hand-edited, or a step was added or removed,
/// and must be re-derived via [`derive_name`]. A name that does not
/// enumerate is not stale — that is `NameDrift::NotEnumerating`, a separate
/// state.
pub fn name_is_stale(job: &CiJob) -> bool {
    let Some(named) = name_enumeration(&job.name) else {
        return false;
    };
    let named_lc: Vec<String> = named.iter().map(|s| s.to_lowercase()).collect();
    let steps_lc = step_names_lowercase(job);
    !(multiset_missing(&named_lc, &steps_lc).is_empty()
        && multiset_missing(&steps_lc, &named_lc).is_empty())
}

/// The runtime half (the lint a gate-definition change triggers): emit the
/// gate-vs-steps line (invariant 2), the name-vs-steps line (invariant 3),
/// and the re-derive hint (invariant 4). `InSync` / `NotEnumerating` / a
/// fresh name emit `OK:`; anything else emits a `WARN:` that names the job
/// and the exact drift, so a drifted name is a warning on the same pass the
/// breakage is introduced — never a silent re-version.
pub fn audit(gate_commands: &[String], job: &CiJob) -> Vec<String> {
    let mut lines = Vec::new();
    lines.push(gate_matches_job(gate_commands, job).line(job));
    lines.push(name_matches_steps(job).line(job));
    if name_is_stale(job) {
        lines.push(format!(
            "WARN: name of job '{}' is stale; re-derive it: {}",
            job.id,
            derive_name(job)
        ));
    }
    lines
}
