//! One authoritative predicate for autonomous dispatch eligibility, and a
//! scheduled reconciler between the issue tracker and the dispatch worklist.
//!
//! The fleet once ran two strictly disjoint label vocabularies
//! (`auto-implement` and `llm-ready`) with a hand-maintained worklist in
//! between. A selector was *replaced* rather than *migrated*, and because the
//! actual dispatch path keyed on the file instead of on either label, an
//! issue could sit eligible and invisible for weeks (#3771). The invariants
//! this module encodes:
//!
//! * Exactly one predicate ([`is_dispatch_eligible`]) decides whether an
//!   issue is eligible for autonomous dispatch. The label that backs it is
//!   named in exactly one place: [`DISPATCH_ELIGIBILITY_LABEL`]. The
//!   ready-queue planner and the reconciler both go through it.
//! * [`reconcile`] keeps the dispatch worklist consistent with the issue
//!   tracker: adding eligible issues the worklist is missing and removing
//!   worklist entries that are no longer eligible are both its job. It is
//!   pure and idempotent — the caller owns the schedule (the fleet's
//!   ten-minute topup loop), and a second run over its own output changes
//!   nothing.
//! * Retiring a selector is a migration, not a replacement. An open issue
//!   that carries a retired selector and not the current one is reported as
//!   a hard error listing its number(s) — never silently skipped.
//! * Every run reports what it changed, and reports explicitly when it
//!   changed nothing, so a broken reconciler is distinguishable from an
//!   idle one.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::coordination::ready_queue::RemoteIssue;

/// The one authoritative dispatch-eligibility selector. This is the only
/// place the dispatch path names the label; every gate goes through
/// [`is_dispatch_eligible`].
pub const DISPATCH_ELIGIBILITY_LABEL: &str = "auto-implement";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispatchEligibilityPolicy {
    current: String,
    retired: Vec<String>,
}

impl Default for DispatchEligibilityPolicy {
    fn default() -> Self {
        Self {
            current: DISPATCH_ELIGIBILITY_LABEL.to_string(),
            retired: Vec::new(),
        }
    }
}

impl DispatchEligibilityPolicy {
    /// Build a policy. Fails if `current` is empty or also appears among
    /// `retired` — a label cannot be current and retired at once.
    pub fn new(
        current: impl Into<String>,
        retired: impl IntoIterator<Item = impl Into<String>>,
    ) -> Result<Self, ReconcileError> {
        let current = current.into();
        if current.is_empty() {
            return Err(invalid_policy("current selector is empty"));
        }
        let retired = retired.into_iter().map(Into::into).collect::<Vec<String>>();
        if retired
            .iter()
            .any(|label| label.eq_ignore_ascii_case(&current))
        {
            return Err(invalid_policy(&format!(
                "current selector `{current}` also appears among retired selectors"
            )));
        }
        Ok(Self { current, retired })
    }

    pub fn current(&self) -> &str {
        &self.current
    }

    pub fn retired(&self) -> &[String] {
        &self.retired
    }

    fn retired_match(&self, labels: &[String]) -> Option<&str> {
        self.retired
            .iter()
            .find(|label| has_label(labels, label))
            .map(String::as_str)
    }
}

fn has_label(labels: &[String], label: &str) -> bool {
    labels
        .iter()
        .any(|current| current.eq_ignore_ascii_case(label))
}

fn invalid_policy(reason: &str) -> ReconcileError {
    ReconcileError {
        kind: ReconcileErrorKind::InvalidPolicy {
            reason: reason.to_string(),
        },
        report: None,
    }
}

/// Verdict of the single dispatch-eligibility predicate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EligibilityVerdict {
    /// Open and carrying the current selector: dispatchable.
    Eligible,
    /// Not dispatchable: closed, or missing the current selector with no
    /// retired selector present.
    Ineligible,
    /// Carries a retired selector and not the current one: a migration gap.
    /// Never dispatchable; the reconciler fails loudly on these.
    Stranded { retired: String },
}

/// The one authoritative predicate: evaluate an issue against the policy and
/// return the full verdict.
pub fn evaluate_dispatch_eligibility(
    issue: &RemoteIssue,
    policy: &DispatchEligibilityPolicy,
) -> EligibilityVerdict {
    if issue.closed {
        return EligibilityVerdict::Ineligible;
    }
    if has_label(&issue.labels, policy.current()) {
        return EligibilityVerdict::Eligible;
    }
    match policy.retired_match(&issue.labels) {
        Some(retired) => EligibilityVerdict::Stranded {
            retired: retired.to_string(),
        },
        None => EligibilityVerdict::Ineligible,
    }
}

/// The one authoritative predicate: does the issue qualify for autonomous
/// dispatch?
pub fn is_dispatch_eligible(issue: &RemoteIssue, policy: &DispatchEligibilityPolicy) -> bool {
    matches!(
        evaluate_dispatch_eligibility(issue, policy),
        EligibilityVerdict::Eligible
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconcileInput {
    /// The full set of open issues from the tracker, label-agnostic. If the
    /// feed is filtered by label, retired-only issues are invisible and the
    /// migration check silently degrades — feed the unfiltered list.
    pub tracker_issues: Vec<RemoteIssue>,
    /// The current dispatch worklist, in order.
    pub worklist: Vec<u64>,
    pub policy: DispatchEligibilityPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconcileReport {
    pub added: Vec<u64>,
    pub removed: Vec<u64>,
    pub kept: Vec<u64>,
    /// The worklist after this run. Feed it back as the next run's
    /// `worklist`; the next run then reports no changes.
    pub updated_worklist: Vec<u64>,
}

impl ReconcileReport {
    pub fn changed(&self) -> bool {
        !self.added.is_empty() || !self.removed.is_empty()
    }

    /// One-line account of the run. Always present, and says so explicitly
    /// when nothing changed, so an idle reconciler is distinguishable from
    /// a broken one.
    pub fn summary(&self) -> String {
        if self.changed() {
            format!(
                "dispatch-reconciler: added {}, removed {}, kept {}",
                numbers_or_none(&self.added),
                numbers_or_none(&self.removed),
                self.kept.len()
            )
        } else {
            format!("dispatch-reconciler: no changes; kept {}", self.kept.len())
        }
    }
}

fn numbers_or_none(numbers: &[u64]) -> String {
    if numbers.is_empty() {
        "none".to_string()
    } else {
        numbers
            .iter()
            .map(|number| format!("#{number}"))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReconcileErrorKind {
    InvalidPolicy {
        reason: String,
    },
    /// Open issues matched a retired selector and not the current one. The
    /// selector was replaced instead of migrated; the listed issue numbers
    /// must be migrated onto the current selector.
    StrandedIssues {
        issues: Vec<u64>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconcileError {
    pub kind: ReconcileErrorKind,
    /// The changes computed in the failed run, still reported so a failed
    /// run is auditable. Absent for policy errors, where nothing ran.
    pub report: Option<ReconcileReport>,
}

impl fmt::Display for ReconcileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", message(self))
    }
}

fn message(error: &ReconcileError) -> String {
    match &error.kind {
        ReconcileErrorKind::InvalidPolicy { reason } => {
            format!("dispatch-reconciler: ERROR: invalid policy: {reason}")
        }
        ReconcileErrorKind::StrandedIssues { issues } => format!(
            "dispatch-reconciler: ERROR: {} issue(s) stranded on a retired selector and not the current one: {}{}",
            issues.len(),
            numbers_or_none(issues),
            report_suffix(&error.report)
        ),
    }
}

fn report_suffix(report: &Option<ReconcileReport>) -> String {
    match report {
        Some(report) => format!("; {}", report.summary()),
        None => String::new(),
    }
}

impl std::error::Error for ReconcileError {}

/// Reconcile the dispatch worklist against the tracker. Pure and idempotent;
/// the caller runs it on a schedule. Fails loudly — listing issue numbers —
/// when any open issue is stranded on a retired selector.
pub fn reconcile(input: &ReconcileInput) -> Result<ReconcileReport, ReconcileError> {
    let tracker = deduplicate_issues(&input.tracker_issues);
    let verdicts = tracker
        .values()
        .map(|issue| {
            (
                issue.number,
                evaluate_dispatch_eligibility(issue, &input.policy),
            )
        })
        .collect::<Vec<_>>();
    let eligible = verdicts
        .iter()
        .filter(|(_, verdict)| matches!(verdict, EligibilityVerdict::Eligible))
        .map(|(number, _)| *number)
        .collect::<BTreeSet<u64>>();
    let stranded = verdicts
        .iter()
        .filter_map(|(number, verdict)| {
            matches!(verdict, EligibilityVerdict::Stranded { .. }).then_some(*number)
        })
        .collect::<Vec<u64>>();

    let mut seen = BTreeSet::new();
    let mut kept = Vec::new();
    let mut removed = BTreeSet::new();
    for &number in &input.worklist {
        if !seen.insert(number) {
            continue;
        }
        if eligible.contains(&number) {
            kept.push(number);
        } else {
            removed.insert(number);
        }
    }
    let added = eligible
        .iter()
        .filter(|number| !seen.contains(number))
        .copied()
        .collect::<Vec<u64>>();
    let mut updated_worklist = kept.clone();
    updated_worklist.extend_from_slice(&added);

    let report = ReconcileReport {
        added,
        removed: removed.into_iter().collect(),
        kept,
        updated_worklist,
    };
    if stranded.is_empty() {
        Ok(report)
    } else {
        Err(ReconcileError {
            kind: ReconcileErrorKind::StrandedIssues { issues: stranded },
            report: Some(report),
        })
    }
}

fn deduplicate_issues(issues: &[RemoteIssue]) -> BTreeMap<u64, RemoteIssue> {
    let mut deduplicated = BTreeMap::new();
    for issue in issues {
        deduplicated
            .entry(issue.number)
            .or_insert_with(|| issue.clone());
    }
    deduplicated
}
