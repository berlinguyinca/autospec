//! First-run ownership and provenance status for newly landed gates (issue #3883).
//!
//! A gate has no value until it has run, and the author is frequently not the
//! party who can run it: cluster agents deliberately have no Docker, no
//! registry credentials, and an unauthenticated `gh`. "The patch is written"
//! and "the check works" are separated by an unowned gap unless the record
//! names who closes it. Measured in the InferWeave `init gateway` E2E, making
//! the gate run once took four changes after the one that wrote it.
//!
//! Two records make the gap owned:
//!
//! - [`GateIntake`] is what a spec or issue that adds a gate must leave
//!   behind: who performs the first real execution, when, and — where the
//!   implementing environment cannot run the gate — that fact, recorded up
//!   front instead of discovered at runtime. [`intake_findings`] names the
//!   missing fields.
//! - [`GateRun`] is one execution with an outcome. [`gate_status`] folds the
//!   runs into a report: a gate is *proven* only by a green run, an
//!   explicitly *owed* run is tracked honest debt, and everything else is
//!   *unproven* — rendered distinctly from a passing gate, never folded into
//!   one. A red run proves the gate ran, not that it passes.
//!
//! [`plan_budget`] applies the measured ~4x ratio between integration work
//! and authoring work so a plan budgets the run, not just the writing.

use serde::Serialize;

/// Measured ratio of "make the check runnable" work to "write the check"
/// work (issue #3883: one dispatch wrote the gate, four more changes made it
/// run once).
pub const INTEGRATION_TO_AUTHORIZATION_RATIO: f64 = 4.0;

/// Rule ID: the record does not say who performs the gate's first real
/// execution.
pub const GATE_FIRST_RUN_OWNER_MISSING: &str = "GATE_FIRST_RUN_OWNER_MISSING";

/// Rule ID: the record does not say when the gate's first real execution
/// happens.
pub const GATE_FIRST_RUN_WHEN_MISSING: &str = "GATE_FIRST_RUN_WHEN_MISSING";

/// Rule ID: the implementing environment cannot run the gate, but the record
/// does not say so — the fact will be discovered at runtime instead.
pub const GATE_NOT_RUNNABLE_UNRECORDED: &str = "GATE_NOT_RUNNABLE_UNRECORDED";

/// Outcome of one gate execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum GateRunOutcome {
    /// The gate ran and passed.
    Green,
    /// The gate ran and failed.
    Red,
    /// An honest `OWED`: the execution slot recorded the debt explicitly
    /// instead of producing a green run.
    Owed,
}

/// One execution of a gate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GateRun {
    /// Who ran the gate.
    pub by: String,
    /// Unix epoch seconds.
    pub at: i64,
    pub outcome: GateRunOutcome,
}

/// Provenance status of a gate, folded from all of its runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum GateStatus {
    /// At least one green run; cites the latest one.
    Proven { by: String, at: i64 },
    /// No green run, but an explicit OWED run; cites the latest one. Tracked
    /// honest debt, not an unowned gap.
    Owed { by: String, at: i64 },
    /// Never green and never explicitly owed. `red_runs` counts failed runs:
    /// they prove the gate ran, not that it passes.
    Unproven { red_runs: u64 },
}

impl GateStatus {
    /// The report line. Unproven is always rendered as `unproven:`, never as
    /// a pass or a debt, so an untracked red gate cannot read as either.
    pub fn render(&self) -> String {
        match self {
            GateStatus::Proven { by, at } => {
                format!("proven: green run by {by} at {at}")
            }
            GateStatus::Owed { by, at } => {
                format!("OWED: no green run yet; debt explicitly recorded by {by} at {at}")
            }
            GateStatus::Unproven { red_runs: 0 } => {
                "unproven: never run — no green run, no explicit OWED".to_string()
            }
            GateStatus::Unproven { red_runs } => {
                format!("unproven: {red_runs} red run(s), no green run, no explicit OWED")
            }
        }
    }
}

/// Fold a gate's runs into its provenance status.
///
/// Precedence: any green run proves the gate; otherwise any explicit OWED
/// run makes it tracked debt; otherwise it is unproven, however many red
/// runs it has. Ties on `at` keep the earliest-listed run, so the result is
/// deterministic for a given run list.
pub fn gate_status(runs: &[GateRun]) -> GateStatus {
    let mut green: Option<&GateRun> = None;
    let mut owed: Option<&GateRun> = None;
    let mut red_runs = 0u64;
    for run in runs {
        match run.outcome {
            GateRunOutcome::Green => {
                if green.map_or(true, |earlier| run.at > earlier.at) {
                    green = Some(run);
                }
            }
            GateRunOutcome::Owed => {
                if owed.map_or(true, |earlier| run.at > earlier.at) {
                    owed = Some(run);
                }
            }
            GateRunOutcome::Red => red_runs += 1,
        }
    }
    if let Some(run) = green {
        return GateStatus::Proven {
            by: run.by.clone(),
            at: run.at,
        };
    }
    if let Some(run) = owed {
        return GateStatus::Owed {
            by: run.by.clone(),
            at: run.at,
        };
    }
    GateStatus::Unproven { red_runs }
}

/// What a spec or issue that adds a gate must record (invariants 1 and 2 of
/// issue #3883).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateIntake {
    /// The gate being added, e.g. `e2e-init-gateway`.
    pub gate: String,
    /// Who performs the first real execution. "Owed to CI" is a fine answer;
    /// leaving it unstated is the finding.
    pub first_run_owner: Option<String>,
    /// When the first real execution happens: a date, an issue number, a
    /// named CI run.
    pub first_run_when: Option<String>,
    /// Whether the implementing environment can execute the gate at all.
    pub author_can_execute: bool,
    /// Required when `author_can_execute` is false: why the environment
    /// cannot run the gate, so the fact is recorded instead of discovered at
    /// runtime.
    pub non_runnable_reason: Option<String>,
}

/// One missing piece of a [`GateIntake`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GateIntakeFinding {
    pub rule_id: &'static str,
    pub message: String,
}

/// Findings for a gate's intake record: the missing first-run owner, the
/// missing first-run time, and an unrecorded non-runnable environment.
pub fn intake_findings(intake: &GateIntake) -> Vec<GateIntakeFinding> {
    let mut findings = Vec::new();
    if blank(&intake.first_run_owner) {
        findings.push(GateIntakeFinding {
            rule_id: GATE_FIRST_RUN_OWNER_MISSING,
            message: format!(
                "gate '{}' adds a CI gate without recording who performs its first real execution",
                intake.gate
            ),
        });
    }
    if blank(&intake.first_run_when) {
        findings.push(GateIntakeFinding {
            rule_id: GATE_FIRST_RUN_WHEN_MISSING,
            message: format!(
                "gate '{}' adds a CI gate without recording when its first real execution happens",
                intake.gate
            ),
        });
    }
    if !intake.author_can_execute && blank(&intake.non_runnable_reason) {
        findings.push(GateIntakeFinding {
            rule_id: GATE_NOT_RUNNABLE_UNRECORDED,
            message: format!(
                "the implementing environment cannot run gate '{}', but the record does not say so",
                intake.gate
            ),
        });
    }
    findings
}

fn blank(value: &Option<String>) -> bool {
    value.as_deref().map(str::trim).unwrap_or("").is_empty()
}

/// Total dispatches a plan must budget for one gate.
///
/// Authoring plus the measured integration ratio, rounded up. One dispatch
/// to "add the E2E gate" budgets `plan_budget(1)` = 5, because the fourth
/// change that finally makes the check run is not optional work.
pub fn plan_budget(authoring_dispatches: u64) -> u64 {
    let authoring = authoring_dispatches as f64;
    (authoring + authoring * INTEGRATION_TO_AUTHORIZATION_RATIO).ceil() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(by: &str, at: i64, outcome: GateRunOutcome) -> GateRun {
        GateRun {
            by: by.to_string(),
            at,
            outcome,
        }
    }

    #[test]
    fn populated_case_inferweave_init_gateway() {
        // The intake the spec left behind: the authoring agent is a cluster
        // worker with no Docker and private packages, so it cannot run the
        // gate itself; the first real run is owed to CI.
        let intake = GateIntake {
            gate: "e2e-init-gateway".to_string(),
            first_run_owner: Some("CI".to_string()),
            first_run_when: Some("next nightly run".to_string()),
            author_can_execute: false,
            non_runnable_reason: Some(
                "cluster worker has no Docker; the pinned packages are private".to_string(),
            ),
        };
        assert!(
            intake_findings(&intake).is_empty(),
            "a complete intake record has no findings"
        );

        // One dispatch wrote the gate; four more changes made it run (#240
        // permissions, #241 mtime, #242 pull, #243 logs) — all red, all
        // blaming something other than the gate.
        let red_runs = [
            run("ci", 1_000, GateRunOutcome::Red),
            run("ci", 2_000, GateRunOutcome::Red),
            run("ci", 3_000, GateRunOutcome::Red),
            run("ci", 4_000, GateRunOutcome::Red),
        ];
        let status = gate_status(&red_runs);
        assert_eq!(status, GateStatus::Unproven { red_runs: 4 });
        assert_eq!(
            status.render(),
            "unproven: 4 red run(s), no green run, no explicit OWED"
        );

        // The fifth change (#244) made it green. Proven — and it found the
        // real subject the hermetic tests could not: no CMD in the published
        // image.
        let mut with_green = red_runs.to_vec();
        with_green.push(run("ci", 5_000, GateRunOutcome::Green));
        assert_eq!(
            gate_status(&with_green),
            GateStatus::Proven {
                by: "ci".to_string(),
                at: 5_000
            }
        );

        // Proven and unproven must never read the same.
        assert_ne!(gate_status(&with_green).render(), status.render());
    }

    #[test]
    fn explicit_owed_run_is_tracked_debt_not_unproven() {
        let runs = [
            run("ci", 1_000, GateRunOutcome::Red),
            run("ci", 2_000, GateRunOutcome::Owed),
        ];
        assert_eq!(
            gate_status(&runs),
            GateStatus::Owed {
                by: "ci".to_string(),
                at: 2_000
            }
        );
        assert_eq!(
            gate_status(&runs).render(),
            "OWED: no green run yet; debt explicitly recorded by ci at 2000"
        );
    }

    #[test]
    fn green_run_beats_owed_and_red() {
        let runs = [
            run("ci", 1_000, GateRunOutcome::Owed),
            run("ci", 2_000, GateRunOutcome::Red),
            run("ci", 3_000, GateRunOutcome::Green),
        ];
        assert_eq!(
            gate_status(&runs),
            GateStatus::Proven {
                by: "ci".to_string(),
                at: 3_000
            }
        );
        assert!(gate_status(&runs).render().starts_with("proven:"));
    }

    #[test]
    fn latest_green_run_is_cited() {
        let runs = [
            run("ci", 1_000, GateRunOutcome::Green),
            run("ci", 2_000, GateRunOutcome::Red),
            run("human", 3_000, GateRunOutcome::Green),
        ];
        assert_eq!(
            gate_status(&runs),
            GateStatus::Proven {
                by: "human".to_string(),
                at: 3_000
            }
        );
    }

    #[test]
    fn never_run_is_unproven_with_zero_red_runs() {
        assert_eq!(gate_status(&[]), GateStatus::Unproven { red_runs: 0 });
        assert_eq!(
            gate_status(&[]).render(),
            "unproven: never run — no green run, no explicit OWED"
        );
    }

    #[test]
    fn red_runs_alone_stay_unproven() {
        let runs = [
            run("ci", 1_000, GateRunOutcome::Red),
            run("ci", 2_000, GateRunOutcome::Red),
        ];
        let status = gate_status(&runs);
        assert_eq!(status, GateStatus::Unproven { red_runs: 2 });
        assert!(
            status.render().starts_with("unproven:"),
            "red runs prove the gate ran, not that it passes: {status:?}"
        );
    }

    #[test]
    fn intake_missing_owner_and_when_are_named() {
        let intake = GateIntake {
            gate: "e2e-init-gateway".to_string(),
            first_run_owner: None,
            first_run_when: None,
            author_can_execute: true,
            non_runnable_reason: None,
        };
        let findings = intake_findings(&intake);
        assert_eq!(
            findings.iter().map(|f| f.rule_id).collect::<Vec<_>>(),
            vec![GATE_FIRST_RUN_OWNER_MISSING, GATE_FIRST_RUN_WHEN_MISSING]
        );
        assert!(findings[0].message.contains("who performs"));
        assert!(findings[1].message.contains("when"));
    }

    #[test]
    fn whitespace_only_fields_count_as_missing() {
        let intake = GateIntake {
            gate: "e2e-init-gateway".to_string(),
            first_run_owner: Some("   ".to_string()),
            first_run_when: Some(String::new()),
            author_can_execute: true,
            non_runnable_reason: None,
        };
        assert_eq!(intake_findings(&intake).len(), 2);
    }

    #[test]
    fn unrecorded_non_runnable_environment_is_a_finding() {
        let intake = GateIntake {
            gate: "e2e-init-gateway".to_string(),
            first_run_owner: Some("CI".to_string()),
            first_run_when: Some("next nightly run".to_string()),
            author_can_execute: false,
            non_runnable_reason: None,
        };
        let findings = intake_findings(&intake);
        assert_eq!(
            findings.iter().map(|f| f.rule_id).collect::<Vec<_>>(),
            vec![GATE_NOT_RUNNABLE_UNRECORDED]
        );
        assert!(findings[0].message.contains("cannot run"));
    }

    #[test]
    fn recorded_non_runnable_environment_is_clean() {
        let intake = GateIntake {
            gate: "e2e-init-gateway".to_string(),
            first_run_owner: Some("CI".to_string()),
            first_run_when: Some("next nightly run".to_string()),
            author_can_execute: false,
            non_runnable_reason: Some("no Docker on the cluster worker".to_string()),
        };
        assert!(intake_findings(&intake).is_empty());
    }

    #[test]
    fn plan_budgets_the_integration_not_just_the_authoring() {
        // One dispatch wrote the gate; the measured work to make it run is
        // ~4x that.
        assert_eq!(plan_budget(1), 5);
        assert_eq!(plan_budget(0), 0);
        assert_eq!(plan_budget(2), 10);
        assert!(
            plan_budget(1) > 1,
            "a one-dispatch budget for a new gate is wrong about the work"
        );
    }
}
