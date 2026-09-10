//! A timed-out gate reported "fixes: <all seven checks>" (issue #3768).
//!
//! The conversion gate compared `validate`'s failing checks against a
//! recorded baseline and reported which had been fixed. Its command ran
//! under `timeout 900` against a measured wall-clock of 741–1189s, so it
//! was killed mid-run, printed no `- check_x: failed` lines at all, and:
//!
//! ```bash
//! fixed=$(comm -23 "$baseline" "$now")   # $now EMPTY -> everything "fixed"
//! ```
//!
//! The merged PR then carried `validate: no new failures, fixes: <all seven
//! required checks>` — measured on `main` right after, all seven still
//! failing, unchanged.
//!
//! **An empty result set is not "nothing failed"; it is "no verdict".**
//! The two are identical to every set operation and opposite in meaning.
//!
//! This is the third variant of one bug class in this backlog (#3764: a
//! guard checked via an unresolvable host read a failed probe as "no patch,
//! proceed"; #3754: a selector weighted by free slots read "no free slots
//! anywhere" as "no candidate"). The common defect: absence was never given
//! its own representation, so it collapsed silently into the success value.
//!
//! The rules this module enforces:
//!
//! 1. **Require positive evidence, not the absence of negative evidence**
//!    ([`Completion`], [`diff_failed_checks`]). A verdict is only produced
//!    from a command whose completion is independently established — an
//!    exit code, a timeout, an error. Only then is its content read.
//! 2. **Treat the unknown as unsafe** ([`GateDiffVerdict::NoVerdict`]). A
//!    gate that cannot evaluate holds the patch, with the reason; it never
//!    passes it and never claims a fix.
//! 3. **Distrust set operations on possibly-empty inputs.** `comm`,
//!    `grep -v`, and `diff` all describe an empty side as a huge change.
//!    The diff here runs only after rule 1, and the no-verdict state
//!    carries no set at all.
//!
//! Timeout budgets are checked the same way ([`check_timeout_budget`]):
//! every budget must strictly exceed the measured wall-clock of what it
//! bounds, and the measurement is recorded next to the budget — a budget
//! with no recorded measurement is a finding, not an OK.

use std::collections::BTreeSet;

/// Positive evidence about how the command a gate runs ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Completion {
    /// The command ran to completion and exited with this code.
    ///
    /// The code may be non-zero: `autospec validate` exits non-zero when
    /// checks fail, and that is still a completed run whose output is
    /// evidence. What matters is that the run *ended on its own*, not what
    /// it decided.
    Ran { exit_code: i32 },
    /// The wrapper killed the command at its timeout budget. Anything the
    /// command printed before being killed is partial, and an empty print
    /// is not "nothing failed".
    TimedOut,
    /// The command never completed for another reason: spawn failure, a
    /// wrapper that died, unreadable output.
    Errored { reason: String },
}

impl Completion {
    /// The gate command ended on its own, so its output is evidence.
    pub fn is_ran(&self) -> bool {
        matches!(self, Self::Ran { .. })
    }

    /// The hold line a no-verdict gate records. `None` for a completed run:
    /// only a completed run may be judged.
    pub fn hold_reason(&self) -> Option<String> {
        match self {
            Self::Ran { .. } => None,
            Self::TimedOut => Some(
                "gate command was killed by its timeout budget before completing; its output is not a verdict"
                    .to_string(),
            ),
            Self::Errored { reason } => Some(format!(
                "gate command errored before completing: {reason}; its output is not a verdict"
            )),
        }
    }
}

/// The outcome of comparing a gate's failing checks against a baseline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateDiffVerdict {
    /// The gate command completed, so its failing set is evidence.
    Judged {
        /// Failed now and not in the baseline. These hold the patch.
        new_failures: Vec<String>,
        /// In the baseline and not failed now. The fix claims only a
        /// completed run may make; empty when the baseline is empty.
        fixed: Vec<String>,
    },
    /// The gate command did not complete. No verdict — not a pass and not a
    /// fix list. The patch is held on `reason`.
    NoVerdict { reason: String },
}

impl GateDiffVerdict {
    /// Holds the patch: new failures, or no verdict at all. The unknown is
    /// unsafe — a gate that cannot evaluate holds, it never passes.
    pub fn holds(&self) -> bool {
        match self {
            Self::Judged { new_failures, .. } => !new_failures.is_empty(),
            Self::NoVerdict { .. } => true,
        }
    }

    /// Passes the patch: a completed run with no new failures. A no-verdict
    /// gate never passes.
    pub fn passes(&self) -> bool {
        matches!(self, Self::Judged { new_failures, .. } if new_failures.is_empty())
    }

    /// The fix claims this verdict carries. `None` for a no-verdict gate: an
    /// incomplete run may claim nothing — including "everything fixed", the
    /// read a bare `comm -23 baseline now` gives when `now` is empty.
    pub fn fixed(&self) -> Option<&[String]> {
        match self {
            Self::Judged { fixed, .. } => Some(fixed),
            Self::NoVerdict { .. } => None,
        }
    }

    /// The no-verdict state is a hold, distinct from success.
    pub fn is_no_verdict(&self) -> bool {
        matches!(self, Self::NoVerdict { .. })
    }

    /// The report line for one gate, with the gate's name.
    pub fn render(&self, gate: &str) -> String {
        match self {
            Self::Judged {
                new_failures,
                fixed,
            } if new_failures.is_empty() => {
                if fixed.is_empty() {
                    format!("{gate}: no new failures")
                } else {
                    format!("{gate}: no new failures, fixes: {}", fixed.join(", "))
                }
            }
            Self::Judged { new_failures, .. } => {
                format!("{gate}: new failures: {}", new_failures.join(", "))
            }
            Self::NoVerdict { reason } => format!("{gate}: NO-VERDICT — {reason}; holding"),
        }
    }
}

/// Compare the gate's current failing checks against the recorded baseline.
///
/// The comparison runs only when [`Completion::is_ran`]. A run that was
/// killed or errored produces [`GateDiffVerdict::NoVerdict`] regardless of
/// what `now` contains — including empty, the case in which a bare
/// `comm -23 baseline now` reads "everything fixed". A completed run is
/// judged on its failing set even when that set is empty: emptiness is only
/// evidence after rule 1.
pub fn diff_failed_checks(
    completion: &Completion,
    baseline: &BTreeSet<String>,
    now: &BTreeSet<String>,
) -> GateDiffVerdict {
    let Some(reason) = completion.hold_reason() else {
        let new_failures = now.difference(baseline).cloned().collect();
        let fixed = baseline.difference(now).cloned().collect();
        return GateDiffVerdict::Judged {
            new_failures,
            fixed,
        };
    };
    GateDiffVerdict::NoVerdict { reason }
}

/// One measured wall-clock observation of the command a budget bounds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeasuredRun {
    /// The measured wall-clock, in seconds.
    pub secs: u64,
    /// Where the measurement came from (run id, host, date), recorded next
    /// to the budget so the pairing survives.
    pub source: String,
}

/// A finding from [`check_timeout_budget`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BudgetFinding {
    /// No measured wall-clock is recorded next to the budget.
    MissingMeasurement { budget_secs: u64 },
    /// The budget does not strictly exceed the longest measured run: a
    /// command that legitimately takes that long is killed mid-run and its
    /// gate reads "no verdict" — or, before this module, "everything
    /// fixed".
    BudgetBelowMeasured {
        budget_secs: u64,
        measured_secs: u64,
        source: String,
    },
}

impl BudgetFinding {
    /// The finding line, naming the remedy.
    pub fn render(&self) -> String {
        match self {
            Self::MissingMeasurement { budget_secs } => {
                format!(
                    "timeout budget {budget_secs}s has no recorded wall-clock measurement: record the measurement next to the budget"
                )
            }
            Self::BudgetBelowMeasured {
                budget_secs,
                measured_secs,
                source,
            } => {
                format!(
                    "timeout budget {budget_secs}s does not exceed the measured wall-clock {measured_secs}s ({source}): raise the budget above the measurement"
                )
            }
        }
    }
}

/// Check that a timeout budget exceeds the measured wall-clock of the
/// command it bounds, and that the measurement is recorded.
///
/// "Exceeds" is strict: a budget equal to the measured wall-clock still
/// kills a slow-but-healthy run, and a budget below it kills it by
/// schedule. Returns the finding, if any.
pub fn check_timeout_budget(
    budget_secs: u64,
    measurements: &[MeasuredRun],
) -> Option<BudgetFinding> {
    let Some(longest) = measurements.iter().max_by_key(|run| run.secs) else {
        return Some(BudgetFinding::MissingMeasurement { budget_secs });
    };
    if longest.secs >= budget_secs {
        return Some(BudgetFinding::BudgetBelowMeasured {
            budget_secs,
            measured_secs: longest.secs,
            source: longest.source.clone(),
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The incident's baseline: all seven required checks failing on
    /// `main`, unchanged.
    fn baseline_seven() -> BTreeSet<String> {
        (1..=7).map(|n| format!("check_{n}")).collect()
    }

    fn set(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    // --- the incident ------------------------------------------------------

    #[test]
    fn a_timed_out_gate_with_empty_output_is_no_verdict_not_everything_fixed() {
        // The exact incident shape: `timeout 900` killed the gate, `$now`
        // is empty, and a bare `comm -23 baseline now` reports all seven
        // baseline failures as fixed.
        let verdict = diff_failed_checks(&Completion::TimedOut, &baseline_seven(), &set(&[]));
        assert!(verdict.is_no_verdict(), "{verdict:?}");
        assert!(verdict.holds(), "{verdict:?}");
        assert!(!verdict.passes(), "{verdict:?}");
        assert_eq!(verdict.fixed(), None, "{verdict:?}");
        let line = verdict.render("validate");
        assert!(line.contains("NO-VERDICT"), "{line}");
        assert!(line.contains("timeout"), "{line}");
        assert!(line.contains("holding"), "{line}");
        assert!(!line.contains("fixes:"), "{line}");
    }

    #[test]
    fn an_errored_gate_is_no_verdict_even_with_a_populated_now() {
        // A partial or stale output file is not evidence either: without a
        // completed run, even a `now` that names checks is not a verdict.
        let verdict = diff_failed_checks(
            &Completion::Errored {
                reason: "wrapper died".to_string(),
            },
            &baseline_seven(),
            &set(&["check_1"]),
        );
        assert!(verdict.is_no_verdict(), "{verdict:?}");
        assert!(verdict.holds());
        assert_eq!(verdict.fixed(), None);
        let line = verdict.render("validate");
        assert!(line.contains("wrapper died"), "{line}");
    }

    #[test]
    fn a_completed_run_with_empty_output_is_a_genuine_fix_claim() {
        // Emptiness is evidence only after rule 1: the gate completed, so an
        // empty failing set really does mean every baseline failure is fixed.
        let verdict = diff_failed_checks(
            &Completion::Ran { exit_code: 0 },
            &baseline_seven(),
            &set(&[]),
        );
        let GateDiffVerdict::Judged {
            new_failures,
            fixed,
        } = &verdict
        else {
            panic!("expected Judged, got {verdict:?}");
        };
        assert!(new_failures.is_empty());
        assert_eq!(fixed.len(), 7);
        assert!(!verdict.holds());
        assert!(verdict.passes());
        assert_eq!(verdict.fixed(), Some(fixed.as_slice()));
        let line = verdict.render("validate");
        assert!(
            line.starts_with("validate: no new failures, fixes: check_1"),
            "{line}"
        );
    }

    #[test]
    fn a_completed_run_is_judged_even_when_its_exit_code_is_nonzero() {
        // `autospec validate` exits non-zero when checks fail. A non-zero
        // exit is a completed run: the output is evidence and the new
        // failures hold the patch.
        let verdict = diff_failed_checks(
            &Completion::Ran { exit_code: 1 },
            &set(&["check_1", "check_2"]),
            &set(&["check_2", "check_3"]),
        );
        let GateDiffVerdict::Judged {
            new_failures,
            fixed,
        } = &verdict
        else {
            panic!("expected Judged, got {verdict:?}");
        };
        assert_eq!(new_failures, &["check_3".to_string()]);
        assert_eq!(fixed, &["check_1".to_string()]);
        assert!(verdict.holds());
        let line = verdict.render("validate");
        assert!(line.contains("new failures: check_3"), "{line}");
    }

    #[test]
    fn no_new_failures_and_no_fixes_renders_without_a_fixes_clause() {
        let verdict = diff_failed_checks(&Completion::Ran { exit_code: 0 }, &set(&[]), &set(&[]));
        assert!(verdict.passes());
        assert_eq!(verdict.render("validate"), "validate: no new failures");
    }

    // --- the timeout budget -------------------------------------------------

    #[test]
    fn the_incident_budget_does_not_exceed_the_measured_wall_clock() {
        // The measured wall-clock of the gate's command is 741–1189s; the
        // budget is 900s.
        let measurements = [
            MeasuredRun {
                secs: 741,
                source: "measured on main, run A".to_string(),
            },
            MeasuredRun {
                secs: 1189,
                source: "measured on main, run B".to_string(),
            },
        ];
        let finding =
            check_timeout_budget(900, &measurements).expect("900s against 1189s must be a finding");
        let BudgetFinding::BudgetBelowMeasured {
            budget_secs,
            measured_secs,
            source,
        } = &finding
        else {
            panic!("expected BudgetBelowMeasured, got {finding:?}");
        };
        assert_eq!(budget_secs, &900);
        assert_eq!(measured_secs, &1189);
        assert!(source.contains("run B"));
        assert!(finding.render().contains("900s"), "{}", finding.render());
        assert!(finding.render().contains("1189s"), "{}", finding.render());
    }

    #[test]
    fn a_budget_above_the_longest_measurement_is_ok() {
        let measurements = [
            MeasuredRun {
                secs: 741,
                source: "run A".to_string(),
            },
            MeasuredRun {
                secs: 1189,
                source: "run B".to_string(),
            },
        ];
        assert_eq!(check_timeout_budget(1200, &measurements), None);
    }

    #[test]
    fn a_budget_equal_to_the_measured_wall_clock_is_a_finding() {
        // "Exceeds" is strict: equal still kills a slow-but-healthy run.
        let measurements = [MeasuredRun {
            secs: 900,
            source: "run A".to_string(),
        }];
        assert!(matches!(
            check_timeout_budget(900, &measurements),
            Some(BudgetFinding::BudgetBelowMeasured { .. })
        ));
    }

    #[test]
    fn a_budget_with_no_recorded_measurement_is_a_finding() {
        let finding = check_timeout_budget(900, &[]).expect("no measurement must be a finding");
        let BudgetFinding::MissingMeasurement { budget_secs } = &finding else {
            panic!("expected MissingMeasurement, got {finding:?}");
        };
        assert_eq!(budget_secs, &900);
        assert!(
            finding.render().contains("record the measurement"),
            "{}",
            finding.render()
        );
    }
}
