//! The agent repair pass (#3787).
//!
//! The invariant: an agent repairs what a tool can repair without judgement —
//! `cargo fmt --all`, then `cargo clippy --fix`, then re-gates — *before* it
//! records any failed status, and it iterates to green while budget remains. A
//! defect that a tool can fix must never reach a human or a conversion pass,
//! and a status that carries an exit code without diagnostic text is the
//! opposite of information: it is what sent the defects of #3787 to conversion
//! to be diagnosed by hand hours later.
//!
//! This module is the policy the agent follows. It is pure — applying a step
//! and re-running the gate are the caller's I/O, supplied as closures — so the
//! loop, the budget, and the recorded status are testable without a toolchain.

/// Whether a tool can repair the defect without judgement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefectClass {
    /// A tool repairs it: rustfmt rewrites the source, or `cargo clippy --fix`
    /// applies a MachineApplicable rustfix suggestion. No model call involved.
    MachineApplicable,
    /// Needs judgement: the suggestion moves a value still in use, the SQL is
    /// doubled, the fix is a one-line diagnosis no tool makes.
    Manual,
}

/// One defect the gate reported, with the diagnostic text that says what it is.
///
/// Every failure carries diagnostic text, not just an exit code (#3787):
/// `test_failed=0` for tests that never ran is the opposite of information.
/// The diagnostic is what makes a failure actionable in one step instead of
/// hours of manual triage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateDefect {
    check: String,
    class: DefectClass,
    diagnostic: String,
}

impl GateDefect {
    pub fn new(
        check: impl Into<String>,
        class: DefectClass,
        diagnostic: impl Into<String>,
    ) -> Result<Self, String> {
        let check = check.into();
        let diagnostic = diagnostic.into();
        if check.trim().is_empty() {
            return Err("gate defect must name the check that reported it".to_string());
        }
        if diagnostic.trim().is_empty() {
            return Err(
                "gate defect carries no diagnostic text: a failure must say what failed, \
                 not only that it did (#3787)"
                    .to_string(),
            );
        }
        Ok(Self {
            check,
            class,
            diagnostic,
        })
    }

    /// The check that reported the defect (`fmt`, `clippy`, `tests`, ...).
    pub fn check(&self) -> &str {
        &self.check
    }

    pub fn class(&self) -> DefectClass {
        self.class
    }

    /// The diagnostic text: what failed, where, and why.
    pub fn diagnostic(&self) -> &str {
        &self.diagnostic
    }

    /// True when a tool repairs this defect without judgement.
    pub fn is_machine_applicable(&self) -> bool {
        self.class == DefectClass::MachineApplicable
    }
}

/// A check the agent could not run, named with the reason (#3787 gate parity).
///
/// The agent's gate set is the merge gate set, including fixtures the agent
/// provisions itself. A check it could not run must be named as skipped with
/// the reason — `skipped=integration-tests=needs live PostgreSQL` — not folded
/// silently into a green status. That silence is the #44 defect: `VERIFIED
/// test_failed=0` while the migration's integration tests never ran because
/// the agent would not provision the fixture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedCheck {
    check: String,
    reason: String,
}

impl SkippedCheck {
    pub fn new(check: impl Into<String>, reason: impl Into<String>) -> Result<Self, String> {
        let check = check.into();
        let reason = reason.into();
        if check.trim().is_empty() {
            return Err("skipped check must name the check it skips".to_string());
        }
        if reason.trim().is_empty() {
            return Err(
                "skipped check carries no reason: naming the check without saying why it \
                 was skipped is the blindness this exists to close (#3787)"
                    .to_string(),
            );
        }
        Ok(Self { check, reason })
    }

    pub fn check(&self) -> &str {
        &self.check
    }

    /// Why the check was not run.
    pub fn reason(&self) -> &str {
        &self.reason
    }
}

/// One step of the repair loop, in the fixed order the pass runs them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepairStep {
    /// `cargo fmt --all` — unformatted source is the most common hold and
    /// needs no judgement at all.
    Fmt,
    /// `cargo clippy --fix --allow-dirty` — applies MachineApplicable rustfix
    /// suggestions with no judgement at all.
    ClippyFix,
    /// Re-run the gate. The repairs are only as good as the gate saying so.
    Regate,
}

impl RepairStep {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fmt => "fmt",
            Self::ClippyFix => "clippy-fix",
            Self::Regate => "regate",
        }
    }

    /// The argv this step runs, or `None` for [`RepairStep::Regate`], which
    /// re-runs the gate instead of a fix command.
    pub fn argv(self) -> Option<&'static [&'static str]> {
        match self {
            Self::Fmt => Some(&["cargo", "fmt", "--all"]),
            Self::ClippyFix => Some(&["cargo", "clippy", "--fix", "--allow-dirty"]),
            Self::Regate => None,
        }
    }

    /// True when the step is a repair (listed in the recorded status) as
    /// opposed to the re-gate that proves the repairs took.
    pub fn is_repair(self) -> bool {
        !matches!(self, Self::Regate)
    }
}

/// The budget the repair loop spends.
///
/// The unit is the caller's (seconds, tokens, iterations); the pass charges
/// one unit per repair step it applies. What the budget encodes is the rule
/// from #3787: iterate to green while budget remains. The run behind #44
/// stopped at 5h39m of an 8h budget with two mechanical defects unaddressed;
/// an agent holding a failing gate and budget should be fixing, not exiting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RepairBudget {
    total: u64,
    spent: u64,
}

impl RepairBudget {
    pub fn with_capacity(total: u64) -> Self {
        Self { total, spent: 0 }
    }

    pub fn remaining(self) -> u64 {
        self.total.saturating_sub(self.spent)
    }

    pub fn is_exhausted(self) -> bool {
        self.remaining() == 0
    }

    /// Charge one unit, saturating at the total so the loop cannot overspend.
    pub fn charge_one(&mut self) {
        self.spent = (self.spent + 1).min(self.total);
    }

    /// A run that exits while the gate is failing with budget left unused is
    /// itself a defect, and is reported as one (#3787).
    pub fn early_exit_defect(self, gate_failing: bool) -> Option<String> {
        if gate_failing && !self.is_exhausted() {
            Some(format!(
                "run exited with a failing gate and {} budget unit(s) unused: an agent \
                 holding a failing gate and budget should be fixing, not exiting (#3787)",
                self.remaining()
            ))
        } else {
            None
        }
    }
}

/// The repair pass: gate → repair what is repairable → re-gate → stop when
/// green, when the budget is exhausted, or when the remaining failure needs
/// judgement the agent must then describe.
pub struct RepairPass;

impl RepairPass {
    /// The ordered steps to run before a failing gate status is recorded:
    /// `fmt` first, then `clippy --fix`, then re-gate.
    ///
    /// The order matters: clippy suggestions on unformatted source are noisy,
    /// and the re-gate is what proves the repairs took. The plan is empty when
    /// no defect is [`DefectClass::MachineApplicable`]: there is nothing a
    /// tool can repair, and the caller records the failure with its
    /// diagnostics instead.
    pub fn steps(defects: &[GateDefect]) -> Vec<RepairStep> {
        if defects.iter().any(GateDefect::is_machine_applicable) {
            vec![RepairStep::Fmt, RepairStep::ClippyFix, RepairStep::Regate]
        } else {
            Vec::new()
        }
    }

    /// Run the pass and produce the status the agent records.
    ///
    /// `gate` is the merge gate set, `ran` the checks the gate run actually
    /// executed, and `skipped` the checks named with a reason. `apply` performs
    /// a fix step (the caller's I/O); `regate` re-runs the gate and returns
    /// the defects that survive.
    ///
    /// The loop re-plans after every re-gate because a repair can expose
    /// further machine-applicable defects; the budget is what stops it. The
    /// [`GateVerdict`] returned is the only status the caller may record: a
    /// failing status is produced here, after the pass, never before it.
    pub fn run(
        gate: &[String],
        ran: &[String],
        skipped: Vec<SkippedCheck>,
        initial: &[GateDefect],
        budget: &mut RepairBudget,
        mut apply: impl FnMut(RepairStep) -> Result<(), String>,
        mut regate: impl FnMut() -> Vec<GateDefect>,
    ) -> Result<GateVerdict, String> {
        let mut repairs: Vec<RepairStep> = Vec::new();
        let mut defects = initial.to_vec();
        loop {
            if defects.is_empty() {
                return GateVerdict::verified(gate, ran, skipped, repairs);
            }
            let plan = Self::steps(&defects);
            if plan.is_empty() || budget.is_exhausted() {
                // Either nothing left is machine-fixable — the remaining
                // failure needs judgement, described by its diagnostics — or
                // the budget is spent. The pass stops and the caller records
                // what survived, with its diagnostic text.
                return GateVerdict::failed(gate, ran, skipped, repairs, defects);
            }
            for step in plan {
                match step {
                    RepairStep::Regate => defects = regate(),
                    _ => {
                        if budget.is_exhausted() {
                            return GateVerdict::failed(gate, ran, skipped, repairs, defects);
                        }
                        apply(step)?;
                        repairs.push(step);
                        budget.charge_one();
                    }
                }
            }
        }
    }
}

/// The status the agent records after the repair pass — and the only status
/// it may record.
///
/// A verified status lists the repairs it applied and names every check it
/// could not run, with the reason. A failed status carries the diagnostic
/// text of every surviving defect. What it can never be is a bare status code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateVerdict {
    verified: bool,
    repairs: Vec<RepairStep>,
    skipped: Vec<SkippedCheck>,
    residual: Vec<GateDefect>,
}

impl GateVerdict {
    /// The gate is green. Every gate check must have been run or named as
    /// skipped with a reason: a required check that is neither is the #44
    /// blindness — `VERIFIED` while a check the change is riskiest under
    /// never ran.
    pub fn verified(
        gate: &[String],
        ran: &[String],
        skipped: Vec<SkippedCheck>,
        repairs: Vec<RepairStep>,
    ) -> Result<Self, String> {
        check_coverage(gate, ran, &skipped)?;
        Ok(Self {
            verified: true,
            repairs,
            skipped,
            residual: Vec::new(),
        })
    }

    /// The gate is still failing. The surviving defects carry their
    /// diagnostic text; every gate check was run or named as skipped.
    pub fn failed(
        gate: &[String],
        ran: &[String],
        skipped: Vec<SkippedCheck>,
        repairs: Vec<RepairStep>,
        residual: Vec<GateDefect>,
    ) -> Result<Self, String> {
        if residual.is_empty() {
            return Err(
                "a failed verdict must name the defects that survive; an empty residual \
                 is a pass, not a failure"
                    .to_string(),
            );
        }
        check_coverage(gate, ran, &skipped)?;
        Ok(Self {
            verified: false,
            repairs,
            skipped,
            residual,
        })
    }

    /// True when the gate is green and the status is recorded as verified.
    pub fn is_verified(&self) -> bool {
        self.verified
    }

    /// The repairs the pass applied, in order. Listed in the recorded status.
    pub fn repairs_applied(&self) -> &[RepairStep] {
        &self.repairs
    }

    /// The checks named as skipped, each with its reason.
    pub fn skipped(&self) -> &[SkippedCheck] {
        &self.skipped
    }

    /// The defects that survived the pass, each with its diagnostic text.
    pub fn residual(&self) -> &[GateDefect] {
        &self.residual
    }

    /// The recorded status line: verified or failed, the repairs applied, the
    /// checks skipped and why, and the diagnostic text of every defect that
    /// failed.
    pub fn render(&self) -> String {
        let mut parts: Vec<String> =
            vec![if self.verified { "VERIFIED" } else { "FAILED" }.to_string()];
        if !self.repairs.is_empty() {
            parts.push(format!(
                "repairs={}",
                self.repairs
                    .iter()
                    .map(|step| step.as_str())
                    .collect::<Vec<_>>()
                    .join(",")
            ));
        }
        if !self.skipped.is_empty() {
            parts.push(format!(
                "skipped={}",
                self.skipped
                    .iter()
                    .map(|s| format!("{}={}", s.check(), s.reason()))
                    .collect::<Vec<_>>()
                    .join(",")
            ));
        }
        for defect in &self.residual {
            parts.push(format!("{}: {}", defect.check(), defect.diagnostic()));
        }
        parts.join(" ")
    }
}

/// The gate-parity check (#3787): every check in the merge gate set was run or
/// named as skipped with a reason. Returns the checks that are neither.
fn check_coverage(gate: &[String], ran: &[String], skipped: &[SkippedCheck]) -> Result<(), String> {
    let missing: Vec<&String> = gate
        .iter()
        .filter(|check| {
            !ran.contains(check) && !skipped.iter().any(|s| s.check() == check.as_str())
        })
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "gate check(s) neither run nor named as skipped: {}; a status cannot be \
             blind to a check it did not run (#3787)",
            missing
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn gate_defect(class: DefectClass, diagnostic: &str) -> GateDefect {
        GateDefect::new("clippy", class, diagnostic).unwrap()
    }

    fn full_gate() -> Vec<String> {
        vec!["fmt".to_string(), "clippy".to_string(), "tests".to_string()]
    }

    /// Regression test for #3787: a patch that only introduces
    /// MachineApplicable lints and unformatted source is repaired by the
    /// repair pass and recorded as verified, listing the repairs — never
    /// surfacing as a hold or a failed status.
    #[test]
    fn machine_applicable_patch_is_repaired_and_recorded_verified() {
        // The first gate reports two defects, both machine-fixable.
        let initial = vec![
            GateDefect::new(
                "fmt",
                DefectClass::MachineApplicable,
                "src/lib.rs is not formatted: 3 diffs",
            )
            .unwrap(),
            GateDefect::new(
                "clippy",
                DefectClass::MachineApplicable,
                "manual_is_multiple_of at src/lib.rs:41 (MachineApplicable)",
            )
            .unwrap(),
        ];
        let gate = full_gate();
        let ran = gate.clone();
        let mut budget = RepairBudget::with_capacity(10);
        let mut applied: Vec<RepairStep> = Vec::new();
        let regates = Cell::new(0);

        let verdict = RepairPass::run(
            &gate,
            &ran,
            Vec::new(),
            &initial,
            &mut budget,
            |step| {
                applied.push(step);
                Ok(())
            },
            || {
                regates.set(regates.get() + 1);
                Vec::new() // the repairs cleared everything: re-gate is green
            },
        )
        .unwrap();

        // The repair pass ran fmt, then clippy --fix, then re-gated.
        assert_eq!(applied, vec![RepairStep::Fmt, RepairStep::ClippyFix]);
        assert_eq!(regates.get(), 1);

        // Recorded as verified, listing the repairs — never a hold.
        assert!(
            verdict.is_verified(),
            "a machine-repairable patch must be recorded verified, not a hold: {}",
            verdict.render()
        );
        assert_eq!(
            verdict.repairs_applied(),
            &[RepairStep::Fmt, RepairStep::ClippyFix]
        );
        assert_eq!(verdict.render(), "VERIFIED repairs=fmt,clippy-fix");
        // Nothing was spent on exit: the gate is green, so an early exit is
        // not a defect even though budget remains.
        assert!(budget.early_exit_defect(false).is_none());
    }

    #[test]
    fn steps_are_ordered_fmt_then_clippy_fix_then_regate() {
        let defects = vec![gate_defect(
            DefectClass::MachineApplicable,
            "manual_is_multiple_of",
        )];
        assert_eq!(
            RepairPass::steps(&defects),
            vec![RepairStep::Fmt, RepairStep::ClippyFix, RepairStep::Regate]
        );
    }

    #[test]
    fn steps_are_empty_when_no_defect_is_machine_applicable() {
        let defects = vec![
            gate_defect(DefectClass::Manual, "duplicated SQL literal in seed.rs"),
            GateDefect::new(
                "tests",
                DefectClass::Manual,
                "assertion at seed.rs:88 fails",
            )
            .unwrap(),
        ];
        assert!(RepairPass::steps(&defects).is_empty());
        assert!(RepairPass::steps(&[]).is_empty());
    }

    #[test]
    fn manual_defects_are_recorded_failed_with_diagnostics_not_repaired() {
        let initial = vec![gate_defect(
            DefectClass::Manual,
            "duplicated SQL literal in seed.rs:21 and seed.rs:88",
        )];
        let gate = full_gate();
        let ran = gate.clone();
        let mut budget = RepairBudget::with_capacity(10);
        let mut applied: Vec<RepairStep> = Vec::new();
        let regates = Cell::new(0);

        let verdict = RepairPass::run(
            &gate,
            &ran,
            Vec::new(),
            &initial,
            &mut budget,
            |step| {
                applied.push(step);
                Ok(())
            },
            || {
                regates.set(regates.get() + 1);
                unreachable!("no machine-applicable defect: nothing to repair or re-gate")
            },
        )
        .unwrap();

        // Nothing was applied and nothing was re-gated: there was nothing a
        // tool could repair.
        assert!(applied.is_empty());
        assert_eq!(regates.get(), 0);

        // The failure is recorded with its diagnostic text.
        assert!(!verdict.is_verified());
        assert_eq!(verdict.repairs_applied(), &[] as &[RepairStep]);
        assert_eq!(verdict.residual().len(), 1);
        assert!(
            verdict
                .render()
                .contains("duplicated SQL literal in seed.rs:21 and seed.rs:88"),
            "the recorded failure must carry the diagnostic text: {}",
            verdict.render()
        );
    }

    #[test]
    fn run_iterates_until_green_when_a_repair_exposes_more_defects() {
        let machine = || {
            vec![gate_defect(
                DefectClass::MachineApplicable,
                "collapsible_if at src/lib.rs:60 (MachineApplicable)",
            )]
        };
        let gate = full_gate();
        let ran = gate.clone();
        let mut budget = RepairBudget::with_capacity(10);
        let regates = Cell::new(0);
        let mut rounds = 0;

        let verdict = RepairPass::run(
            &gate,
            &ran,
            Vec::new(),
            &machine(),
            &mut budget,
            |step| {
                assert!(step.is_repair());
                Ok(())
            },
            || {
                regates.set(regates.get() + 1);
                rounds += 1;
                if rounds == 1 {
                    // The first repair exposed a further machine-applicable
                    // defect; the loop must plan again, not stop.
                    machine()
                } else {
                    Vec::new()
                }
            },
        )
        .unwrap();

        assert_eq!(regates.get(), 2);
        assert!(verdict.is_verified());
        assert_eq!(
            verdict.repairs_applied(),
            &[
                RepairStep::Fmt,
                RepairStep::ClippyFix,
                RepairStep::Fmt,
                RepairStep::ClippyFix
            ]
        );
        assert_eq!(budget.remaining(), 6);
    }

    #[test]
    fn run_stops_when_the_budget_is_exhausted_and_records_the_residual() {
        // The gate never goes green: the same machine-applicable defect
        // survives every re-gate.
        let stubborn = || {
            vec![gate_defect(
                DefectClass::MachineApplicable,
                "manual_is_multiple_of at src/lib.rs:41 (MachineApplicable)",
            )]
        };
        let gate = full_gate();
        let ran = gate.clone();
        let mut budget = RepairBudget::with_capacity(2);

        let verdict = RepairPass::run(
            &gate,
            &ran,
            Vec::new(),
            &stubborn(),
            &mut budget,
            |step| {
                assert!(step.is_repair());
                Ok(())
            },
            stubborn,
        )
        .unwrap();

        // One full repair round was funded; the second was not.
        assert!(budget.is_exhausted());
        assert_eq!(
            verdict.repairs_applied(),
            &[RepairStep::Fmt, RepairStep::ClippyFix]
        );
        assert!(!verdict.is_verified());
        assert_eq!(verdict.residual().len(), 1);
        // The budget is exhausted, so exiting with the gate failing is not
        // the early-exit defect.
        assert!(budget.early_exit_defect(true).is_none());
    }

    #[test]
    fn exiting_early_with_a_failing_gate_and_budget_is_itself_a_defect() {
        let mut budget = RepairBudget::with_capacity(8);
        budget.charge_one();

        let defect = budget.early_exit_defect(true).expect("must be reported");
        assert!(
            defect.contains("failing gate") && defect.contains("7 budget unit(s) unused"),
            "unexpected early-exit diagnostic: {defect}"
        );

        // Green gate or exhausted budget: not a defect.
        assert!(budget.early_exit_defect(false).is_none());
        assert!(RepairBudget::with_capacity(0)
            .early_exit_defect(true)
            .is_none());
    }

    #[test]
    fn a_failure_without_diagnostic_text_is_rejected() {
        let err = GateDefect::new("clippy", DefectClass::Manual, "   ").unwrap_err();
        assert!(
            err.contains("no diagnostic text"),
            "unexpected error: {err}"
        );
        assert!(GateDefect::new("  ", DefectClass::Manual, "x").is_err());
    }

    #[test]
    fn a_skipped_check_without_a_reason_is_rejected() {
        let err = SkippedCheck::new("integration-tests", "").unwrap_err();
        assert!(err.contains("no reason"), "unexpected error: {err}");
        assert!(SkippedCheck::new("", "needs live PostgreSQL").is_err());
    }

    #[test]
    fn verified_requires_every_gate_check_run_or_skipped() {
        // The #44 case: the migration's integration tests never ran and were
        // not named. A verified status with a blind check is the defect.
        let gate = vec![
            "fmt".to_string(),
            "clippy".to_string(),
            "integration-tests".to_string(),
        ];
        let ran = vec!["fmt".to_string(), "clippy".to_string()];
        let err = GateVerdict::verified(&gate, &ran, Vec::new(), Vec::new()).unwrap_err();
        assert!(
            err.contains("integration-tests"),
            "the blind check must be named: {err}"
        );

        // Naming the check with the reason is what makes the status honest.
        let skipped =
            vec![SkippedCheck::new("integration-tests", "needs live PostgreSQL").unwrap()];
        let verdict = GateVerdict::verified(&gate, &ran, skipped, Vec::new()).unwrap();
        assert!(
            verdict
                .render()
                .contains("skipped=integration-tests=needs live PostgreSQL"),
            "the recorded status must name the skipped check and why: {}",
            verdict.render()
        );
    }

    #[test]
    fn failed_verdict_must_name_its_residual_defects() {
        let gate = full_gate();
        let ran = gate.clone();
        let err = GateVerdict::failed(&gate, &ran, Vec::new(), Vec::new(), Vec::new()).unwrap_err();
        assert!(err.contains("empty residual"), "unexpected error: {err}");
    }

    #[test]
    fn repair_steps_expose_their_argv() {
        assert_eq!(RepairStep::Fmt.argv(), Some(&["cargo", "fmt", "--all"][..]));
        assert_eq!(
            RepairStep::ClippyFix.argv(),
            Some(&["cargo", "clippy", "--fix", "--allow-dirty"][..])
        );
        assert!(RepairStep::Regate.argv().is_none());
        assert!(RepairStep::Fmt.is_repair());
        assert!(RepairStep::ClippyFix.is_repair());
        assert!(!RepairStep::Regate.is_repair());
    }
}
