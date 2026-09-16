//! How a triage decision is written out (#3715, #4651).
//!
//! The policy in the parent decides *what* the pass does with a patch; this
//! module decides *what the operator reads*. The split is not cosmetic: the
//! rendering is the part that has to carry the agent's own evidence into the
//! run log — a hold that names only "agent reported a failure" cannot be
//! checked from the line it printed — and it is the part that grows whenever
//! the vocabulary gains a case, while the precedence rules rarely move.
//!
//! Rule 7 of the parent's contract: a hold must show the agent's own
//! evidence, the rc, the file count, the status —
//! `HELD: agent reported fmt_rc=1 (32 files), status=TIMEOUT`.

use super::signal;
use super::{AgentHoldReason, AgentReport, GateBasis, TriageDecision};

/// The hold line the pass prints for an agent-reported hold (rule 7):
/// the agent's own evidence, not just the hold's reason.
///
/// `held_line(&report, AgentHoldReason::Unformatted)` for a report with
/// `fmt_rc=1`, `fmt_files=Some(32)`, `status=TIMEOUT` prints
/// `HELD: agent reported fmt_rc=1 (32 files), status=TIMEOUT`.
pub fn held_line(report: &AgentReport, reason: AgentHoldReason) -> String {
    let evidence = match reason {
        AgentHoldReason::Unformatted => fmt_evidence(report),
        AgentHoldReason::Unbuilt => match report.build_rc {
            Some(rc) => format!("build_rc={rc}"),
            None => "build failure".to_string(),
        },
    };
    match report.status.as_deref() {
        Some(status) => format!("HELD: agent reported {evidence}, status={status}"),
        None => format!("HELD: agent reported {evidence}"),
    }
}

/// The fmt evidence the agent's report carries, for a hold or a
/// format-and-recheck line: `fmt_rc` with the file count when both are
/// recorded, either alone when only one is, and the bare name when the
/// run recorded neither.
fn fmt_evidence(report: &AgentReport) -> String {
    match (report.fmt_rc, report.fmt_files) {
        (Some(rc), Some(files)) => format!("fmt_rc={rc} ({files} files)"),
        (Some(rc), None) => format!("fmt_rc={rc}"),
        (None, Some(files)) => format!("fmt_files={files}"),
        (None, None) => "fmt failure".to_string(),
    }
}

/// The one line the pass prints for any triage decision (rule 7): what
/// the pass is doing, and the report evidence it is acting on.
pub fn decision_line(decision: &TriageDecision, report: &AgentReport) -> String {
    match decision {
        TriageDecision::Redispatch { status } => {
            format!("RE-DISPATCH: agent reported status={status}; the run never reached the gate")
        }
        TriageDecision::RaiseForReview { status } => {
            format!(
                "RAISE-FOR-REVIEW: agent reported status={status}; the run produced no output, so re-dispatch would reproduce the same emptiness (#3936)"
            )
        }
        // A signalled run is refused with its attribution, because the
        // attribution is the only finding the run left (#4651): who ended
        // it, and the fact that the runner's own timeout did not.
        TriageDecision::Signalled { .. } => format!(
            "REFUSED: the run was terminated by a signal, so no verdict about the patch exists; {} (#4651)",
            signal::Termination::classify(report.agent_rc)
                .attribution(report.signal.as_deref(), report.agent_rc)
        ),
        TriageDecision::Hold { reason } => held_line(report, *reason),
        TriageDecision::FormatAndRecheck => match report.status.as_deref() {
            Some(status) => format!(
                "FORMAT-AND-RECHECK: agent reported {}, status={status}; formatting and re-checking locally before judging",
                fmt_evidence(report)
            ),
            None => format!(
                "FORMAT-AND-RECHECK: agent reported {}; formatting and re-checking locally before judging",
                fmt_evidence(report)
            ),
        },
        TriageDecision::GateLocally { basis } => {
            format!("GATE-LOCALLY: {}", basis_line(basis, report))
        }
    }
}

fn basis_line(basis: &GateBasis, report: &AgentReport) -> String {
    match basis {
        GateBasis::AgentReportedTestFailure { status } => match status {
            Some(status) => format!(
                "agent reported test failure (status={status}); re-verifying against current main"
            ),
            None => match report.test_rc {
                Some(rc) => format!(
                    "agent reported test failure (test_rc={rc}); re-verifying against current main"
                ),
                None => "agent reported test failure; re-verifying against current main"
                    .to_string(),
            },
        },
        GateBasis::NoBaseline => {
            "status=UNKNOWN-NO-BASELINE; no baseline to attribute the failures — gating against current main"
                .to_string()
        }
        GateBasis::AgentGreen => "agent reported green; confirming against current main".to_string(),
        GateBasis::ReportUnreadable { detail } => {
            format!("status file unreadable ({detail}); gating against current main")
        }
    }
}
