//! Gate results are recorded, not read.
//!
//! A gate's outcome is measured from the run itself: the process's own exit
//! status, the parsed summary counts, and — for a runner that prints no
//! summary line — the count of the runner's failure markers across the full
//! output. A figure read from the tail of the output is not a field here:
//! the final line of a run is the last case executed, never the verdict. A
//! report built from [`GateResult`] can only state what was measured, and
//! [`review_gate_claim`] rejects a claim that the recorded evidence does not
//! carry.

use super::parse_test_run;

/// The definition of a gate: what it runs and how that runner counts
/// failures.
///
/// The failure marker is part of the definition, not chosen ad hoc per
/// run: `not ok` for bats, `FAILED` for cargo. A per-run choice is where a
/// reader picks a pattern that matches what the visible tail happens to
/// show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GateDefinition {
    /// The label the gate's evidence line carries.
    pub name: &'static str,
    /// The marker the runner prints per failing case, counted across the
    /// full output when the runner emits no summary line.
    pub failure_marker: &'static str,
}

impl GateDefinition {
    /// `cargo test`: a per-target `test result:` line is the summary. The
    /// `FAILED` marker is counted only when that summary is absent.
    pub fn cargo_test() -> Self {
        Self {
            name: "cargo-test",
            failure_marker: "FAILED",
        }
    }

    /// `bats`: no summary line. A failing case is a `not ok` line, printed
    /// inline, and the runner keeps going — so the final line of a run is
    /// the last case executed, never the verdict.
    pub fn bats() -> Self {
        Self {
            name: "bats",
            failure_marker: "not ok",
        }
    }
}

/// The mechanically recorded result of one gate execution.
///
/// This is the only evidence a report may carry about a gate: the exit
/// status, the parsed summary counts, and the failure-marker count, each
/// taken from the run itself. A figure read from the tail of the output is
/// not a field here, and a report built from this record can only state
/// what was measured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GateResult {
    /// The process's own exit status, or `None` when the command never ran.
    /// A claim built on a result without one is rejected, never trusted.
    pub exit_status: Option<i32>,
    /// The parsed summary: the runner's own passed count.
    pub passed: u64,
    /// The parsed summary: the runner's own failed count.
    pub failed: u64,
    /// Whether the output contained the runner's summary line.
    pub summary_present: bool,
    /// The count of failure-marker lines across the full output. For a
    /// runner without a summary line this is the failure count.
    pub failed_markers: u64,
}

impl GateResult {
    /// The failure count the gate is judged on: the summary's failed count
    /// when the runner printed one, else the failure-marker count.
    ///
    /// A summary line is the runner's own count and is authoritative; the
    /// marker count is the substitute for runners that print failures
    /// inline and never a summary. The rule is the same either way: count
    /// something, never read the end.
    pub fn failure_count(&self) -> u64 {
        if self.summary_present {
            self.failed
        } else {
            self.failed_markers
        }
    }

    /// Whether the recorded evidence supports reporting the gate as
    /// passing: a recorded zero exit status and a zero failure count.
    pub fn supports_pass(&self) -> bool {
        self.exit_status == Some(0) && self.failure_count() == 0
    }

    /// The evidence line a report carries for this gate, generated from
    /// the recorded values rather than written as prose: the exit status,
    /// and either the parsed summary or the marker count — so a reader can
    /// tell a measurement from an estimate without re-running anything.
    pub fn evidence(&self, definition: &GateDefinition) -> String {
        let exit = match self.exit_status {
            Some(code) => code.to_string(),
            None => "unrecorded".to_string(),
        };
        if self.summary_present {
            format!(
                "{}: exit {exit} — {} passed; {} failed (parsed summary)",
                definition.name, self.passed, self.failed
            )
        } else {
            format!(
                "{}: exit {exit} — {} `{}` marker lines (no summary line)",
                definition.name, self.failed_markers, definition.failure_marker
            )
        }
    }
}

/// Record a gate execution mechanically: the exit status, the parsed
/// summary, and the failure-marker count.
///
/// The marker comes from the gate definition, and both the summary and the
/// marker count are taken across the full output — not its tail. Long
/// output is the normal case for a test suite, so every count here is
/// taken from the whole run.
pub fn record_gate(
    definition: &GateDefinition,
    exit_status: Option<i32>,
    output: &str,
) -> GateResult {
    let summary = parse_test_run(output);
    GateResult {
        exit_status,
        passed: summary.passed,
        failed: summary.failed,
        summary_present: summary.targets > 0,
        failed_markers: count_failure_markers(output, definition.failure_marker),
    }
}

/// The count of lines carrying the runner's failure marker, across the
/// full output.
fn count_failure_markers(output: &str, marker: &str) -> u64 {
    u64::try_from(output.lines().filter(|line| line.contains(marker)).count()).unwrap_or(u64::MAX)
}

/// What a report claims about a gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateClaim {
    /// The report says the gate passed.
    Passing,
    /// The report says the gate failed.
    Failing,
}

/// Why a gate claim is rejected at review: the claim and the recorded
/// evidence disagree, or the evidence cannot carry the claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateRejection {
    /// The claim rests on a result with no recorded exit status: nothing
    /// was measured, so nothing may be trusted.
    NoExitStatusRecorded,
    /// Reported as passing, but the recorded exit status is non-zero.
    NonZeroExit { exit_status: i32 },
    /// Reported as passing, but the recorded failure count is above zero.
    FailureCountAboveZero { failed: u64 },
    /// Reported as failing, but the recorded evidence supports a pass.
    FailingWithCleanEvidence,
}

/// The review of a claimed gate outcome against its recorded result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateReview {
    /// The claim agrees with the recorded evidence.
    Accepted,
    /// The claim disagrees with the recorded evidence — the defect.
    Rejected(GateRejection),
}

impl GateReview {
    pub fn is_accepted(&self) -> bool {
        matches!(self, Self::Accepted)
    }
}

/// Review a reported gate outcome against its recorded result.
///
/// The two must agree: a claim of passing is accepted only when the exit
/// status was recorded as zero and the recorded failure count is zero. A
/// claim without a recorded exit status is rejected rather than trusted —
/// that is how a tail-read figure sat in the same paragraph, in the same
/// format, with the same apparent confidence as a measured one.
pub fn review_gate_claim(claim: GateClaim, result: &GateResult) -> GateReview {
    let Some(exit) = result.exit_status else {
        return GateReview::Rejected(GateRejection::NoExitStatusRecorded);
    };
    match claim {
        GateClaim::Passing if exit != 0 => {
            GateReview::Rejected(GateRejection::NonZeroExit { exit_status: exit })
        }
        GateClaim::Passing if result.failure_count() > 0 => {
            GateReview::Rejected(GateRejection::FailureCountAboveZero {
                failed: result.failure_count(),
            })
        }
        GateClaim::Failing if exit == 0 && result.failure_count() == 0 => {
            GateReview::Rejected(GateRejection::FailingWithCleanEvidence)
        }
        _ => GateReview::Accepted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The #4007 shape: six failures earlier in a 69-case bats run, the
    /// tail three `ok` lines. The final line is the last case executed,
    /// never the verdict.
    fn bats_run_with_interleaved_failures() -> String {
        let failing: [u64; 6] = [3, 11, 24, 37, 45, 58];
        let mut output = String::new();
        for i in 1..=69u64 {
            let line = if failing.contains(&i) { "not ok" } else { "ok" };
            output.push_str(&format!("{line} {i} some test\n"));
        }
        output
    }

    #[test]
    fn a_green_tail_with_nonzero_exit_is_rejected() {
        let result = record_gate(
            &GateDefinition::bats(),
            Some(1),
            &bats_run_with_interleaved_failures(),
        );

        assert_eq!(result.failed_markers, 6);
        assert_eq!(result.failure_count(), 6);
        assert!(!result.supports_pass());
        assert_eq!(
            review_gate_claim(GateClaim::Passing, &result),
            GateReview::Rejected(GateRejection::NonZeroExit { exit_status: 1 })
        );
    }

    #[test]
    fn failures_interleaved_among_later_passes_count_the_whole_run() {
        // The failures cluster earlier and the runner keeps going: a count
        // read from the tail reads 0; counting the run reads 4.
        let output = [
            "not ok 1 first\n",
            "ok 2\n",
            "not ok 3\n",
            "ok 4\n",
            "ok 5\n",
            "not ok 6\n",
            "ok 7\n",
            "not ok 8\n",
            "ok 9\n",
            "ok 10\n",
            "ok 11\n",
            "ok 12\n",
        ]
        .concat();

        let result = record_gate(&GateDefinition::bats(), Some(1), &output);
        assert!(!result.summary_present);
        assert_eq!(result.failure_count(), 4);
        assert!(!result.supports_pass());
    }

    #[test]
    fn a_claim_without_a_recorded_exit_status_is_rejected() {
        // Nothing was measured: even an all-green output cannot carry a
        // claim of passing. The figure is rejected rather than trusted.
        let result = record_gate(&GateDefinition::bats(), None, "ok 1\nok 2\nok 3\n");
        assert!(!result.supports_pass());
        assert_eq!(
            review_gate_claim(GateClaim::Passing, &result),
            GateReview::Rejected(GateRejection::NoExitStatusRecorded)
        );
    }

    #[test]
    fn a_cargo_summary_line_is_authoritative_over_markers() {
        // `FAILED` appears in the summary line, but the runner's own count
        // is authoritative when the summary is present.
        let output =
            "test result: FAILED. 3 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out\n";
        let result = record_gate(&GateDefinition::cargo_test(), Some(101), output);

        assert!(result.summary_present);
        assert_eq!(result.passed, 3);
        assert_eq!(result.failed, 2);
        assert_eq!(result.failure_count(), 2);
        assert_eq!(result.failed_markers, 1);
        assert!(!result.supports_pass());
        assert_eq!(
            review_gate_claim(GateClaim::Passing, &result),
            GateReview::Rejected(GateRejection::NonZeroExit { exit_status: 101 })
        );
    }

    #[test]
    fn a_cargo_run_without_a_summary_counts_failed_markers() {
        // A build failure can end before any `test result:` line: the
        // definition's `FAILED` marker is the substitute, not an ad hoc
        // per-run choice.
        let output = "test a ... FAILED\ntest b ... ok\nerror: could not compile `crate`\n";
        let result = record_gate(&GateDefinition::cargo_test(), Some(101), output);

        assert!(!result.summary_present);
        assert_eq!(result.failed_markers, 1);
        assert_eq!(result.failure_count(), 1);
    }

    #[test]
    fn a_claim_that_agrees_with_the_record_is_accepted() {
        let clean = record_gate(&GateDefinition::bats(), Some(0), "ok 1\nok 2\n");
        assert!(clean.supports_pass());
        assert_eq!(
            review_gate_claim(GateClaim::Passing, &clean),
            GateReview::Accepted
        );

        let failing = record_gate(&GateDefinition::bats(), Some(1), "not ok 1\nok 2\n");
        assert_eq!(
            review_gate_claim(GateClaim::Failing, &failing),
            GateReview::Accepted
        );
    }

    #[test]
    fn a_claim_disagreeing_in_either_direction_is_rejected() {
        // The two must agree: both directions of disagreement are defects.
        let failing = record_gate(&GateDefinition::bats(), Some(1), "not ok 1\nok 2\n");
        assert_eq!(
            review_gate_claim(GateClaim::Passing, &failing),
            GateReview::Rejected(GateRejection::NonZeroExit { exit_status: 1 })
        );

        // A non-zero failure count rejects a passing claim even on exit 0
        // (the runner that exits 0 with failed cases is the other half of
        // the disagreement).
        let green_exit_with_failures = GateResult {
            exit_status: Some(0),
            summary_present: false,
            failed_markers: 3,
            ..Default::default()
        };
        assert_eq!(
            review_gate_claim(GateClaim::Passing, &green_exit_with_failures),
            GateReview::Rejected(GateRejection::FailureCountAboveZero { failed: 3 })
        );

        let clean = record_gate(&GateDefinition::bats(), Some(0), "ok 1\nok 2\n");
        assert_eq!(
            review_gate_claim(GateClaim::Failing, &clean),
            GateReview::Rejected(GateRejection::FailingWithCleanEvidence)
        );
    }

    #[test]
    fn the_evidence_line_is_generated_from_the_recorded_values() {
        // The report line is produced from the record, not written as
        // prose: a reader can tell a measurement from an estimate.
        let bats = GateDefinition::bats();
        let result = record_gate(&bats, Some(1), "not ok 1 x\nok 2\nok 3\n");
        assert_eq!(
            result.evidence(&bats),
            "bats: exit 1 — 1 `not ok` marker lines (no summary line)"
        );

        let cargo = GateDefinition::cargo_test();
        let cargo_result = record_gate(
            &cargo,
            Some(101),
            "test result: FAILED. 3 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out\n",
        );
        assert_eq!(
            cargo_result.evidence(&cargo),
            "cargo-test: exit 101 — 3 passed; 2 failed (parsed summary)"
        );

        // No recorded exit status says so in the line itself, not with a
        // plausible number.
        let unrun = record_gate(&bats, None, "ok 1\n");
        assert_eq!(
            unrun.evidence(&bats),
            "bats: exit unrecorded — 0 `not ok` marker lines (no summary line)"
        );
    }
}
