//! Exit-code evidence, the verdict recorded beside it, and the audit of a
//! population of both (issue #4206).
//!
//! This is the checking half of the run-status vocabulary. The vocabulary
//! itself — the names a component may write into `status=` — lives in the
//! parent; this module answers the separate question of whether a recorded
//! verdict is supported by the exit codes recorded with it.
//!
//! It exists because 183 runs were labelled `VERIFIED` with a failing
//! `test_rc` in the same record. One contradicted record reads as a bad run
//! and a constant label reads as a healthy pipeline; only the two together
//! say the verdict is not evidence, so both halves are audited here, per
//! record and across the population.

use super::{canonical_status, unknown_status_refusal, Status};
use std::collections::BTreeMap;

/// The exit-code evidence a run records beside its verdict.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RunEvidence {
    /// `cargo build` exit code, `None` when the stage did not record one.
    pub build_rc: Option<i32>,
    /// test stage exit code, `None` when the stage did not record one.
    pub test_rc: Option<i32>,
    /// `cargo fmt --check` exit code, `None` when the stage did not record one.
    pub fmt_rc: Option<i32>,
}

/// What the evidence supports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Derivation {
    /// the exit codes determine this status.
    Status(Status),
    /// the exit codes do not determine a status; the string names what is missing.
    Insufficient(&'static str),
}

impl RunEvidence {
    /// Evidence with all three stage exit codes recorded.
    pub fn new(build_rc: i32, test_rc: i32, fmt_rc: i32) -> Self {
        RunEvidence {
            build_rc: Some(build_rc),
            test_rc: Some(test_rc),
            fmt_rc: Some(fmt_rc),
        }
    }

    /// The stages whose exit code is absent, in check order.
    pub fn missing(&self) -> Vec<&'static str> {
        let mut missing = Vec::new();
        if self.fmt_rc.is_none() {
            missing.push("fmt_rc");
        }
        if self.build_rc.is_none() {
            missing.push("build_rc");
        }
        if self.test_rc.is_none() {
            missing.push("test_rc");
        }
        missing
    }

    /// The evidence rendered as it appears in a status record, `?` for a stage that
    /// recorded nothing.
    pub fn evidence_line(&self) -> String {
        let show = |v: Option<i32>| match v {
            Some(rc) => rc.to_string(),
            None => "?".to_string(),
        };
        format!(
            "build_rc={} test_rc={} fmt_rc={}",
            show(self.build_rc),
            show(self.test_rc),
            show(self.fmt_rc)
        )
    }

    /// Derives the status from the exit codes. Precedence is the most decisive
    /// evidence first — formatting, then build, then tests — matching the triage
    /// precedence in [`crate::execution::status_triage`].
    ///
    /// Green is only derivable when all three stages recorded zero. A record that
    /// omitted a stage says nothing about it, so it cannot support `VERIFIED`.
    /// The timeout statuses are not derivable from exit codes at all: a timed-out
    /// run records no code, which is the fact being asserted.
    pub fn derive(&self) -> Derivation {
        if matches!(self.fmt_rc, Some(rc) if rc != 0) {
            return Derivation::Status(Status::FmtDirty);
        }
        if matches!(self.build_rc, Some(rc) if rc != 0) {
            return Derivation::Status(Status::BuildFail);
        }
        if matches!(self.test_rc, Some(rc) if rc != 0) {
            return Derivation::Status(Status::NewTestFailures);
        }
        if self.fmt_rc == Some(0) && self.build_rc == Some(0) && self.test_rc == Some(0) {
            return Derivation::Status(Status::Verified);
        }
        Derivation::Insufficient("green needs build_rc, test_rc and fmt_rc all recorded and zero")
    }
}

/// A recorded verdict compared against the evidence recorded with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerdictCheck {
    /// no verdict was recorded; the caller derives one and reports it.
    NoVerdict,
    /// the recorded label is what the evidence gives.
    Consistent {
        /// the label, and the status it resolves to.
        recorded: Status,
    },
    /// the evidence determines a status and the record says something else.
    Contradiction {
        /// the label that was written,
        recorded: Status,
        /// the status the exit codes give,
        derived: Status,
        /// the fields that contradict the label.
        evidence: String,
    },
    /// a green label over evidence that does not establish green.
    Undecidable {
        /// the green label that was written,
        recorded: Status,
        /// the stages whose exit code is absent.
        missing: Vec<&'static str>,
    },
    /// the label is one the exit codes cannot confirm or refute, such as a timeout.
    Unverifiable {
        /// the label, and why the exit codes say nothing about it.
        recorded: Status,
        /// the reason.
        reason: &'static str,
    },
    /// the label is not in the vocabulary, so it resolves to nothing.
    Undeclared {
        /// the raw label as recorded.
        recorded: String,
    },
}

impl VerdictCheck {
    /// Whether this outcome means the record's verdict cannot be trusted.
    ///
    /// An undeclared label is a defect of the same order as a contradiction: in one
    /// case the verdict disagrees with the fields beside it, in the other it names
    /// nothing at all (`BUILD-FAILDED`, `UNKNOWN-NO-FMT-BASELINE`). `Unverifiable`
    /// and `NoVerdict` are not defects — they are the honest absence of evidence,
    /// which is a gap to record, not a wrong answer.
    pub fn is_defect(&self) -> bool {
        matches!(
            self,
            VerdictCheck::Contradiction { .. }
                | VerdictCheck::Undecidable { .. }
                | VerdictCheck::Undeclared { .. }
        )
    }

    /// One-line rendering, verdict first.
    pub fn line(&self) -> String {
        match self {
            VerdictCheck::NoVerdict => {
                "no verdict recorded; derive one from the fields".to_string()
            }
            VerdictCheck::Consistent { recorded } => {
                format!("verdict {} matches the evidence", recorded.as_str())
            }
            VerdictCheck::Contradiction {
                recorded,
                derived,
                evidence,
            } => format!(
                "verdict {} contradicted by its own fields: {} implies {}",
                recorded.as_str(),
                evidence,
                derived.as_str()
            ),
            VerdictCheck::Undecidable { recorded, missing } => format!(
                "verdict {} not supported by the record: green is not derivable with {} unrecorded",
                recorded.as_str(),
                missing.join(", ")
            ),
            VerdictCheck::Unverifiable { recorded, reason } => {
                format!(
                    "verdict {} not checkable from exit codes: {reason}",
                    recorded.as_str()
                )
            }
            VerdictCheck::Undeclared { recorded } => unknown_status_refusal("verdict", recorded),
        }
    }
}

/// Compares a recorded status against the exit codes recorded beside it
/// (invariant 2). A verdict is a claim about the fields; this is the check.
pub fn check_verdict(recorded: Option<&str>, evidence: &RunEvidence) -> VerdictCheck {
    let raw = match recorded {
        None => return VerdictCheck::NoVerdict,
        Some(r) => r.trim(),
    };
    let Some(recorded_status) = canonical_status(raw) else {
        return VerdictCheck::Undeclared {
            recorded: raw.to_string(),
        };
    };
    match evidence.derive() {
        Derivation::Status(derived) => {
            if derived == recorded_status {
                VerdictCheck::Consistent {
                    recorded: recorded_status,
                }
            } else {
                VerdictCheck::Contradiction {
                    recorded: recorded_status,
                    derived,
                    evidence: evidence.evidence_line(),
                }
            }
        }
        Derivation::Insufficient(_) => {
            if recorded_status == Status::Verified {
                VerdictCheck::Undecidable {
                    recorded: recorded_status,
                    missing: evidence.missing(),
                }
            } else {
                VerdictCheck::Unverifiable {
                    recorded: recorded_status,
                    reason:
                        "a negative verdict needs the failing stage's exit code to be checkable",
                }
            }
        }
    }
}

/// How a population of verdict labels is distributed (invariant 3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelDistribution {
    /// labels counted, with their counts.
    pub counts: BTreeMap<String, usize>,
    /// total labels seen, including those the vocabulary does not declare.
    pub total: usize,
}

/// Counts a population of recorded status labels. Names are counted as recorded;
/// an alias counts separately from its canonical status, because a population that
/// spells the same status two ways is itself a finding.
pub fn label_distribution<'a>(labels: impl IntoIterator<Item = &'a str>) -> LabelDistribution {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut total = 0usize;
    for label in labels {
        *counts.entry(label.trim().to_string()).or_insert(0) += 1;
        total += 1;
    }
    LabelDistribution { counts, total }
}

impl LabelDistribution {
    /// Number of distinct labels observed.
    pub fn distinct(&self) -> usize {
        self.counts.len()
    }

    /// The dominant label and its count, if any.
    pub fn top(&self) -> Option<(&str, usize)> {
        self.counts
            .iter()
            .max_by_key(|(_, n)| **n)
            .map(|(k, v)| (k.as_str(), *v))
    }

    pub fn count_of(&self, label: &str) -> usize {
        self.counts.get(label).copied().unwrap_or(0)
    }

    /// Whether the label separates nothing: a population large enough to vary that
    /// shows exactly one value. One record is not degenerate, it is one record.
    pub fn is_degenerate(&self) -> bool {
        self.total >= 2 && self.distinct() == 1
    }

    /// One-line rendering.
    pub fn line(&self) -> String {
        match self.top() {
            None => "label distribution: empty population".to_string(),
            Some((label, n)) if self.is_degenerate() => format!(
                "label distribution: {label} {n}/{} (100%) — the label is constant across the population: it separates nothing and is not evidence",
                self.total
            ),
            Some((label, n)) => format!(
                "label distribution: {} distinct label(s) over {} record(s), most common {label} {n}/{}",
                self.distinct(),
                self.total,
                self.total
            ),
        }
    }
}

/// One run's record: what it claimed and what it measured.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunRecord {
    /// the status the run wrote, verbatim.
    pub status: Option<String>,
    /// the exit codes it wrote beside it.
    pub evidence: RunEvidence,
}

impl RunRecord {
    /// A record with a status and all three exit codes.
    pub fn new(status: &str, build_rc: i32, test_rc: i32, fmt_rc: i32) -> Self {
        RunRecord {
            status: Some(status.to_string()),
            evidence: RunEvidence::new(build_rc, test_rc, fmt_rc),
        }
    }
}

/// The verdict audit of a whole population: per-record checks plus the shape of
/// the labels together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerdictAudit {
    /// records audited.
    pub total: usize,
    /// records whose verdict the evidence supports.
    pub consistent: usize,
    /// per-record outcomes for the records that are contradicted, unprovable-green
    /// or undeclared.
    pub defects: Vec<VerdictCheck>,
    /// the label shape of the population.
    pub distribution: LabelDistribution,
}

impl VerdictAudit {
    /// Whether the population's verdicts can be used as evidence at all: no record
    /// contradicted, and the label actually varies.
    pub fn trustworthy(&self) -> bool {
        self.defects.is_empty() && !self.distribution.is_degenerate()
    }

    /// One-line rendering, for the monitor log that reads as normal operation.
    pub fn line(&self) -> String {
        let mut parts = vec![format!(
            "verdict audit: {}/{} record(s) supported by their fields",
            self.consistent, self.total
        )];
        if !self.defects.is_empty() {
            parts.push(format!("{} record(s) contradicted", self.defects.len()));
        }
        if self.distribution.is_degenerate() {
            parts.push("label is constant".to_string());
        }
        format!(
            "{}: {}",
            if self.trustworthy() { "ok" } else { "WARN" },
            parts.join(", ")
        )
    }
}

/// Audits a population of run records: every verdict against its own fields, plus
/// the distribution of labels across the population.
///
/// This is the check the cluster needed: 183 runs labelled `VERIFIED`, every one of
/// them carrying a failing `test_rc` in the same record. Either half alone reads as
/// plausible — one contradicted record looks like a bad run, a constant label looks
/// like a healthy pipeline. Together they say the verdict is not evidence.
pub fn audit_population(records: &[RunRecord]) -> VerdictAudit {
    let mut defects = Vec::new();
    let mut consistent = 0usize;
    let mut labels: Vec<&str> = Vec::with_capacity(records.len());
    for record in records {
        if let Some(status) = record.status.as_deref() {
            labels.push(status);
        }
        match check_verdict(record.status.as_deref(), &record.evidence) {
            VerdictCheck::Consistent { .. } => consistent += 1,
            check @ (VerdictCheck::Contradiction { .. }
            | VerdictCheck::Undecidable { .. }
            | VerdictCheck::Undeclared { .. }) => defects.push(check),
            _ => {}
        }
    }
    VerdictAudit {
        total: records.len(),
        consistent,
        defects,
        distribution: label_distribution(labels),
    }
}
