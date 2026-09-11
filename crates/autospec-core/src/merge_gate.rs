//! Merge-gate invariants for the patch-to-PR converter (issue #4307).
//!
//! The converter turns an agent patch into a merged PR. Its gate was a local
//! approximation: fmt, build, clippy and test on a single host — three of the
//! six jobs the repository's real CI runs. A macOS-gated test file (#4306) with
//! five compile errors sat merged and hidden because the local gate never
//! compiled it, the merge record said the gate passed without naming what it
//! had not checked, and the pass rate of the repository's own workflow on the
//! default branch (0/40) was never surfaced — so a red gate was read as a
//! green one.
//!
//! Four invariants, each a primitive here:
//!
//! 1. **The converter's merge requires the real CI gate.** A local gate is a
//!    pre-filter, never a merge authority: `decide` approves only when the
//!    PR's own CI run has passed, and a local-gate failure is a pre-filter
//!    rejection that is not even a CI verdict.
//! 2. **The merge record names what it did not check.** `Coverage` splits the
//!    workflow's jobs into checked and unchecked;
//!    `record_names_unchecked` refuses a merge record that does not name every
//!    unchecked job. Absence within a record is not absence of the gap.
//! 3. **Refuse to merge while the target workflow is red on the base
//!    branch.** `decide` refuses on `BaseBranchStatus::Red` even when the local
//!    gate and the PR's own CI are green — merging into a red base hides the
//!    next failure inside the base's.
//! 4. **The gate's pass rate on the default branch is a number, not a
//!    feeling.** `PassRate` carries the counters and renders them with
//!    `line()`; a gate that has failed every run is flagged by
//!    `all_failing`, never smoothed into "mostly passing".

use serde::{Deserialize, Serialize};

/// The outcome of the converter's local gate (fmt / build / clippy / test on
/// one host). A pre-filter only — see invariant 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GateResult {
    Passed,
    Failed,
}

/// The state of the PR's own CI run on the target repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CiStatus {
    Pending,
    Passed,
    Failed,
    NotRun,
}

impl CiStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            CiStatus::Pending => "pending",
            CiStatus::Passed => "passed",
            CiStatus::Failed => "failed",
            CiStatus::NotRun => "not-run",
        }
    }
}

/// The health of the target workflow on the base branch (invariant 3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BaseBranchStatus {
    Green,
    Red { failing_jobs: Vec<String> },
    Unknown,
}

impl BaseBranchStatus {
    pub fn is_green(&self) -> bool {
        matches!(self, BaseBranchStatus::Green)
    }
}

/// The merge decision (invariants 1 and 3). `Approved` is the only state that
/// authorises a merge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MergeDecision {
    /// The local gate failed. A pre-filter rejection — it is not even a CI
    /// verdict, so no CI status was consulted.
    LocalGateFailed,
    /// The target workflow is red on the base branch; the failing jobs are
    /// named. Refused even when the local gate and the PR's own CI passed.
    BaseBranchRed { failing_jobs: Vec<String> },
    /// The local gate passed but the PR's own CI has not. The local result is
    /// recorded for the record, never read as the gate.
    CiNotPassed { status: CiStatus },
    /// Local gate passed, base branch green, the PR's own CI passed.
    Approved,
}

impl MergeDecision {
    pub fn approved(&self) -> bool {
        matches!(self, MergeDecision::Approved)
    }

    /// One line for the merge record and the monitor log.
    pub fn line(&self) -> String {
        match self {
            MergeDecision::LocalGateFailed => {
                "merge refused: local gate failed (pre-filter only — never a merge)".to_string()
            }
            MergeDecision::BaseBranchRed { failing_jobs } => {
                let jobs = failing_jobs.join(", ");
                format!("merge refused: target workflow red on base branch (failing: {jobs})")
            }
            MergeDecision::CiNotPassed { status } => format!(
                "merge not approved: real CI {s} — the local gate is a pre-filter, not the gate",
                s = status.as_str()
            ),
            MergeDecision::Approved => {
                "merge approved: local gate, base branch and real CI all passed".to_string()
            }
        }
    }
}

/// Decide the merge. Order matters: the local gate is the cheapest check and a
/// pre-filter only (invariant 1); a red base branch refuses before the PR's CI
/// is consulted (invariant 3); and only a passed PR CI approves.
pub fn decide(local: GateResult, ci: CiStatus, base: BaseBranchStatus) -> MergeDecision {
    if local == GateResult::Failed {
        return MergeDecision::LocalGateFailed;
    }
    if let BaseBranchStatus::Red { failing_jobs } = base {
        return MergeDecision::BaseBranchRed { failing_jobs };
    }
    if ci != CiStatus::Passed {
        return MergeDecision::CiNotPassed { status: ci };
    }
    MergeDecision::Approved
}

/// Which of the workflow's jobs the local gate covered (invariant 2). Job ids
/// in workflow order; `unchecked` is the gap the merge record must name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Coverage {
    pub checked: Vec<String>,
    pub unchecked: Vec<String>,
}

impl Coverage {
    /// Split `workflow_jobs` by `locally_checked`. Ids that are checked but
    /// not in the workflow are ignored; workflow order is preserved.
    pub fn new(workflow_jobs: &[String], locally_checked: &[String]) -> Self {
        let checked_set = locally_checked
            .iter()
            .collect::<std::collections::BTreeSet<_>>();
        let mut checked = Vec::new();
        let mut unchecked = Vec::new();
        for job in workflow_jobs {
            if checked_set.contains(job) {
                checked.push(job.clone());
            } else {
                unchecked.push(job.clone());
            }
        }
        Coverage { checked, unchecked }
    }

    pub fn complete(&self) -> bool {
        self.unchecked.is_empty()
    }

    /// The line a merge record must carry when the coverage is not complete.
    pub fn line(&self) -> String {
        format!(
            "local gate covers {}/{} CI jobs",
            self.checked.len(),
            self.checked.len() + self.unchecked.len()
        )
    }

    pub fn line_with_unchecked(&self) -> String {
        if self.complete() {
            self.line()
        } else {
            format!("{}; unchecked: {}", self.line(), self.unchecked.join(", "))
        }
    }
}

/// Invariant 2, made mechanical: the merge record names every job the local
/// gate did not check. A record that is silent about the gap is a lie of
/// omission, not a missing fact to default.
pub fn record_names_unchecked(record: &str, coverage: &Coverage) -> bool {
    coverage
        .unchecked
        .iter()
        .all(|job| record.contains(job.as_str()))
}

/// The gate's pass rate on the default branch (invariant 4): the counters
/// behind a line like `rust-suites on main: 0/40 passed (0.0%)`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PassRate {
    pub workflow: String,
    pub branch: String,
    pub passed: u32,
    pub total: u32,
}

impl PassRate {
    pub fn new(workflow: impl Into<String>, branch: impl Into<String>) -> Self {
        PassRate {
            workflow: workflow.into(),
            branch: branch.into(),
            passed: 0,
            total: 0,
        }
    }

    /// Fold one more run in. The counters are the state; nothing else is.
    pub fn record(&mut self, outcome: bool) {
        self.total += 1;
        if outcome {
            self.passed += 1;
        }
    }

    pub fn ratio(&self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            self.passed as f64 / self.total as f64
        }
    }

    /// A gate that has failed every run it has run. This is the state that hid
    /// the macOS breakage for a day: 0/40, never surfaced.
    pub fn all_failing(&self) -> bool {
        self.total > 0 && self.passed == 0
    }

    pub fn line(&self) -> String {
        format!(
            "{} on {}: {}/{} passed ({:.1}%)",
            self.workflow,
            self.branch,
            self.passed,
            self.total,
            self.ratio() * 100.0
        )
    }
}
