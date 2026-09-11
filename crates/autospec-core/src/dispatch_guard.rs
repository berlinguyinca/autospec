//! Dispatch guard against unconverted output (#3764).
//!
//! The dispatch path re-dispatches an issue by destroying the issue's output
//! directory — `rm -rf out/issue-N` — on the assumption that whatever is in
//! it is stale debris from a previous run. That assumption is exactly what
//! this guard exists to verify, and the failure it exists to rule out is a
//! guard that verifies it through a channel that fails open: the original
//! dispatcher asked an unreachable ssh alias whether a patch existed, the
//! question errored, and the error was read as "no file" — so a finished,
//! unconverted patch was destroyed along with the rest of the directory.
//!
//! Two invariants:
//!
//! 1. **The dispatch precondition is "no unconverted output", not "no
//!    running process"** ([`CheckId::UnconvertedPatch`]). A finished runner
//!    leaves its patch behind; a process check cannot see it. The
//!    unconverted-patch check is therefore *mandatory*: a guard that cannot
//!    verify it holds rather than authorizes ([`GuardReason::CheckMissing`]).
//! 2. **A check that cannot answer is unsafe, not clear**
//!    ([`CheckOutcome::Failed`], [`GuardReason::CheckFailed`]). Fail closed:
//!    a transport error, a `stat` error, an unparseable result — any of
//!    them means the world is unreadable, and an unreadable world is not
//!    "no patch exists".
//!
//! Everything here is pure and testable: no I/O, no clock, no subprocess.
//! The caller runs its checks, reports each as a [`CheckReport`], and
//! [`decide`] renders the verdict.

use crate::run_status::Status;
use serde::{Deserialize, Serialize};

/// The checks a guard can run before an issue is dispatched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckId {
    /// No unconverted output (patch) exists for the issue. This is the
    /// dispatch precondition: dispatch destroys the output directory, so an
    /// unconverted patch inside it is destroyed with the dispatch.
    UnconvertedPatch,
    /// No runner process is working the issue. A secondary check: a
    /// *finished* runner leaves its patch behind, so this check alone can
    /// never authorize dispatch.
    RunningProcess,
}

impl CheckId {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UnconvertedPatch => "unconverted_patch",
            Self::RunningProcess => "running_process",
        }
    }
}

/// What a single check observed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckOutcome {
    /// The check ran and saw no dangerous state.
    Clear,
    /// The check ran and saw a dangerous state: the patch exists, or the
    /// process is running.
    Dangerous,
    /// The check itself failed — a transport error, a stat error, an
    /// unparseable result. This is not [`Clear`]: a check that cannot answer
    /// is unsafe (fail-closed).
    Failed { detail: String },
}

impl CheckOutcome {
    pub fn dangerous(&self) -> bool {
        matches!(self, Self::Dangerous)
    }

    pub fn failed(&self) -> bool {
        matches!(self, Self::Failed { .. })
    }
}

/// One check and what it observed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckReport {
    pub check: CheckId,
    pub outcome: CheckOutcome,
    /// Evidence a human can verify the check against — the patch path, the
    /// process name. Rendered alongside the outcome.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<String>,
}

impl CheckReport {
    pub fn clear(check: CheckId, evidence: Option<String>) -> Self {
        Self {
            check,
            outcome: CheckOutcome::Clear,
            evidence,
        }
    }

    pub fn dangerous(check: CheckId, evidence: Option<String>) -> Self {
        Self {
            check,
            outcome: CheckOutcome::Dangerous,
            evidence,
        }
    }

    pub fn failed(check: CheckId, detail: impl Into<String>) -> Self {
        Self {
            check,
            outcome: CheckOutcome::Failed {
                detail: detail.into(),
            },
            evidence: None,
        }
    }
}

/// Terminal run statuses that indicate the agent run failed irrecoverably.
///
/// A patch produced by a run with one of these statuses is permanently
/// unconvertible: the conversion pass (`status_triage`) refuses it, and the
/// dispatch guard would hold forever if the artifact were left in place
/// (#3784). The guard archives the artifact to free the dispatch slot.
///
/// The set is **closed**: a status not listed here is assumed convertible
/// (tolerant to growth). Only the statuses enumerated here — where the run
/// failed in a way no amount of re-dispatch can fix without a fresh run —
/// are archived.
///
/// Entries are derived from the shared [`Status`] enum (backed by
/// `config/run-status-vocabulary.tsv`) rather than restating wire strings.
/// The legacy spelling `BUILD-FAILED` is an alias in the vocabulary that
/// resolves to `Status::BuildFail` via [`crate::run_status::canonical_status`] before the
/// membership check, so it need not be listed here (#4206).
pub const FAILED_RUN_STATUSES: &[&str] = &[
    Status::BuildFail.as_str(),
    Status::Timeout.as_str(),
    Status::TimeoutNoOutput.as_str(),
    Status::TestTimeout.as_str(),
];

/// The classified outcome of an unconverted artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactOutcome {
    /// The run that produced the patch failed irrecoverably; the patch can
    /// never be converted. The guard should archive it to free the dispatch
    /// slot.
    FailedRun { status: String },
    /// The patch is convertible (or at least not known to be unconvertible);
    /// the guard must hold so the conversion pass can process it. The
    /// `status` names the recorded terminal status for operator visibility.
    Convertible { status: String },
    /// No terminal status was recorded next to the patch; the guard cannot
    /// classify the outcome. Fail-closed: hold, never archive.
    Unrecorded { detail: String },
}

/// Classify an unconverted artifact from its recorded terminal status.
///
/// `None` (no `status.txt`, or the file parsed with no status field) means
/// the outcome is [`ArtifactOutcome::Unrecorded`]: the guard holds and
/// never archives.
///
/// A status whose canonical form is in [`FAILED_RUN_STATUSES`] means the run
/// failed irrecoverably; the artifact is [`ArtifactOutcome::FailedRun`] and
/// the guard archives it to free the dispatch slot.
///
/// Legacy spellings (`BUILD-FAILED`, `TESTS-DO-NOT-COMPILE`) reach this rule
/// through the vocabulary alias table, not through a second literal here.
///
/// Any other status is [`ArtifactOutcome::Convertible`]: the conversion pass
/// can still act on the patch, so the guard holds with the status named.
/// This is the tolerant-to-growth default: an unknown status is held, never
/// destroyed.
pub fn classify_artifact_outcome(status: Option<&str>) -> ArtifactOutcome {
    match status {
        None => ArtifactOutcome::Unrecorded {
            detail: "no terminal status recorded next to the patch".to_string(),
        },
        Some(s) => match crate::run_status::canonical_status(s) {
            Some(c) if FAILED_RUN_STATUSES.contains(&c.as_str()) => ArtifactOutcome::FailedRun {
                status: s.to_string(),
            },
            _ => ArtifactOutcome::Convertible {
                status: s.to_string(),
            },
        },
    }
}

/// Why dispatch must not proceed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuardReason {
    /// An unconverted patch exists for the issue; dispatch would destroy it.
    UnconvertedPatch {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        evidence: Option<String>,
    },
    /// An unconverted patch exists and its recorded terminal status makes it
    /// convertible: the conversion pass must process it before dispatch can
    /// proceed. The `status` names the recorded status for operator
    /// visibility.
    PatchAwaitingConversion {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        evidence: Option<String>,
        status: String,
    },
    /// An unconverted patch exists but no terminal status was recorded next
    /// to it; the guard cannot classify the outcome and holds (fail-closed).
    /// The `detail` names what was missing.
    PatchOutcomeUnrecorded {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        evidence: Option<String>,
        detail: String,
    },
    /// A runner process is already working the issue.
    RunningProcess,
    /// A check the guard relies on was not run at all. A guard that cannot
    /// verify its precondition holds rather than authorizes.
    CheckMissing { check: CheckId },
    /// A check that could run errored; a check that cannot answer is unsafe.
    CheckFailed { check: CheckId, detail: String },
}

/// The guard's verdict.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuardDecision {
    /// No check saw a dangerous state and every mandatory check ran;
    /// dispatch may proceed.
    Dispatch,
    /// At least one check forbids dispatch; `reason` names which one.
    Hold { reason: GuardReason },
}

/// The verdict with the evidence it was rendered from, so a `--dry-run` can
/// report the decision and a wrapper can branch on it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuardReport {
    pub issue: String,
    pub decision: GuardDecision,
    pub checks: Vec<CheckReport>,
}

impl GuardReport {
    pub fn held(&self) -> bool {
        matches!(self.decision, GuardDecision::Hold { .. })
    }

    /// The one-line verdict, shaped for cron wrappers and log lines.
    pub fn line(&self) -> String {
        let prefix = format!("DISPATCH issue {}", self.issue);
        match &self.decision {
            GuardDecision::Dispatch => format!("{prefix} authorized: {}", self.check_summary()),
            GuardDecision::Hold { reason } => format!("{prefix} HELD: {}", reason_line(reason)),
        }
    }

    /// The full report a `--dry-run` prints: one line per check, then the
    /// verdict.
    pub fn lines(&self) -> Vec<String> {
        let mut lines = Vec::with_capacity(self.checks.len() + 1);
        for report in &self.checks {
            lines.push(format!(
                "  check {}: {}",
                report.check.as_str(),
                outcome_line(report)
            ));
        }
        lines.push(self.line());
        lines
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".to_string())
    }

    fn check_summary(&self) -> String {
        self.checks
            .iter()
            .map(|report| {
                let state = match &report.outcome {
                    CheckOutcome::Clear => "clear",
                    CheckOutcome::Dangerous => "DANGEROUS",
                    CheckOutcome::Failed { .. } => "FAILED",
                };
                format!("{}={state}", report.check.as_str())
            })
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn outcome_line(report: &CheckReport) -> String {
    let mut line = match &report.outcome {
        CheckOutcome::Clear => "clear".to_string(),
        CheckOutcome::Dangerous => "DANGEROUS".to_string(),
        CheckOutcome::Failed { detail } => format!("FAILED ({detail})"),
    };
    if let Some(evidence) = &report.evidence {
        line.push_str(&format!(" ({evidence})"));
    }
    line
}

fn reason_line(reason: &GuardReason) -> String {
    match reason {
        GuardReason::UnconvertedPatch {
            evidence: Some(evidence),
        } => {
            format!("unconverted patch exists at {evidence}; dispatch would destroy it")
        }
        GuardReason::UnconvertedPatch { evidence: None } => {
            "an unconverted patch exists; dispatch would destroy it".to_string()
        }
        GuardReason::PatchAwaitingConversion {
            evidence: Some(evidence),
            status,
        } => {
            format!(
                "unconverted patch exists at {evidence} (status: {status}); awaiting conversion — dispatch would destroy it"
            )
        }
        GuardReason::PatchAwaitingConversion {
            evidence: None,
            status,
        } => {
            format!(
                "unconverted patch exists (status: {status}); awaiting conversion — dispatch would destroy it"
            )
        }
        GuardReason::PatchOutcomeUnrecorded {
            evidence: Some(evidence),
            detail,
        } => {
            format!(
                "unconverted patch exists at {evidence} but {detail}; cannot classify outcome — dispatch would destroy it"
            )
        }
        GuardReason::PatchOutcomeUnrecorded {
            evidence: None,
            detail,
        } => {
            format!(
                "unconverted patch exists but {detail}; cannot classify outcome — dispatch would destroy it"
            )
        }
        GuardReason::RunningProcess => "a runner process is already working the issue".to_string(),
        GuardReason::CheckMissing { check } => format!(
            "check {} was not run; a guard that cannot verify its precondition holds",
            check.as_str()
        ),
        GuardReason::CheckFailed { check, detail } => format!(
            "check {} failed ({detail}); a check that cannot answer is unsafe",
            check.as_str()
        ),
    }
}

/// Build a [`GuardReport`] that holds because of a classified artifact
/// outcome, carrying the evidence from the check.
///
/// Used by the CLI guard after it classifies a dangerous (patch-exists)
/// check result: the hold reason names the specific outcome so an operator
/// can see *why* dispatch is blocked (#3784 AC 3).
pub fn classified_hold(issue: &str, check: CheckReport, outcome: ArtifactOutcome) -> GuardReport {
    let reason = match outcome {
        ArtifactOutcome::Convertible { status } => GuardReason::PatchAwaitingConversion {
            evidence: check.evidence.clone(),
            status,
        },
        ArtifactOutcome::Unrecorded { detail } => GuardReason::PatchOutcomeUnrecorded {
            evidence: check.evidence.clone(),
            detail,
        },
        // FailedRun is never a hold: the CLI archives the artifact instead.
        // This arm exists so the match is exhaustive; it should not be
        // reached in practice.
        ArtifactOutcome::FailedRun { status } => GuardReason::UnconvertedPatch {
            evidence: check
                .evidence
                .clone()
                .or_else(|| Some(format!("status: {status}"))),
        },
    };
    GuardReport {
        issue: issue.to_string(),
        decision: GuardDecision::Hold { reason },
        checks: vec![check],
    }
}

/// Render the verdict for `issue` from the checks that were run.
///
/// The ordering encodes the #3764 lesson:
///
/// 1. A *seen* danger beats a *failed* check — an observed unconverted patch
///    (or observed running process) is evidence, and it is reported before
///    any check failure.
/// 2. A check that errored holds the dispatch (fail-closed).
/// 3. The unconverted-patch check is mandatory: with no report at all there
///    is no basis to say the precondition holds. A missing running-process
///    check is tolerable — it was never the right precondition.
pub fn decide(issue: &str, checks: &[CheckReport]) -> GuardReport {
    let unconverted: Vec<&CheckReport> = checks
        .iter()
        .filter(|report| report.check == CheckId::UnconvertedPatch)
        .collect();
    let process: Vec<&CheckReport> = checks
        .iter()
        .filter(|report| report.check == CheckId::RunningProcess)
        .collect();

    if let Some(report) = unconverted
        .iter()
        .copied()
        .find(|report| report.outcome.dangerous())
    {
        return hold(
            issue,
            checks,
            GuardReason::UnconvertedPatch {
                evidence: report.evidence.clone(),
            },
        );
    }
    if process
        .iter()
        .copied()
        .any(|report| report.outcome.dangerous())
    {
        return hold(issue, checks, GuardReason::RunningProcess);
    }
    if let Some((check, detail)) = failed_in(&unconverted) {
        return hold(issue, checks, GuardReason::CheckFailed { check, detail });
    }
    if unconverted.is_empty() {
        return hold(
            issue,
            checks,
            GuardReason::CheckMissing {
                check: CheckId::UnconvertedPatch,
            },
        );
    }
    if let Some((check, detail)) = failed_in(&process) {
        return hold(issue, checks, GuardReason::CheckFailed { check, detail });
    }
    GuardReport {
        issue: issue.to_string(),
        decision: GuardDecision::Dispatch,
        checks: checks.to_vec(),
    }
}

/// The first errored check in `reports`, with its failure detail.
fn failed_in(reports: &[&CheckReport]) -> Option<(CheckId, String)> {
    reports
        .iter()
        .copied()
        .find_map(|report| match &report.outcome {
            CheckOutcome::Failed { detail } => Some((report.check, detail.clone())),
            _ => None,
        })
}

fn hold(issue: &str, checks: &[CheckReport], reason: GuardReason) -> GuardReport {
    GuardReport {
        issue: issue.to_string(),
        decision: GuardDecision::Hold { reason },
        checks: checks.to_vec(),
    }
}
