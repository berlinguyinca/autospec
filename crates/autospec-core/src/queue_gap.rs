//! The dispatch queue's gap reconciliation (#4450).
//!
//! The measured incident: 178 open issues carried the eligibility label, 90 of
//! them were in the queue, 75 already had a branch or a PR, and **37 were in
//! none of the three sets** — filed, labelled, and invisible to dispatch. They
//! had not been rejected or held; nothing reported anything. The queue looked
//! healthy because a queue that has quietly stopped growing is indistinguishable
//! from a queue that is keeping up: 120 entries, agents running against them,
//! and the newest entry 66 issues behind the newest issue filed.
//!
//! [`crate::dispatch_pipeline::SchedulingReconciliation`] compares the admitted
//! set against the queue, but it counts an issue that already has a branch or a
//! PR as admitted-but-unschedulable, so a caller facing a nonzero count cannot
//! tell "the refresher stopped" from "work in flight is excluded on purpose".
//! And the reconciliation only ran when someone thought to run it: nothing
//! compared the two sets on every pass, so the gap grew for as long as nobody
//! looked.
//!
//! This module keeps two invariants:
//!
//! 1. **The four counts, every run.** [`QueueGap`] computes
//!    `eligible - queued - has_branch_or_pr` and [`QueueGap::line`] renders all
//!    four numbers on every run — including the run where the difference is
//!    zero, because the zero is the evidence the reconciler ran. A non-empty
//!    difference is a defect ([`QueueGap::is_defect`]) and is *reported*, never
//!    silently appended to the queue: patching the symptom would hide why the
//!    issues stopped flowing.
//!
//! 2. **A step whose implementation is absent is an error, not a no-op.** The
//!    loop step told the agent to run a refresher script that did not exist, and
//!    the absence produced no signal at any point ([`crate::procedure`] checks
//!    that at publication time). [`missing_components`] runs the same check on
//!    the components a reconcile depends on and renders each absence as
//!    [`MissingComponent::line`], naming the step and the command that resolves
//!    to nothing. A run that declared no components says so out loud
//!    ([`MISSING_COMPONENTS_NONE`]) rather than passing quietly.
//!
//! The types are pure and report-only: the caller (the CLI `dispatch queue-gap`
//! subcommand) decides exit codes and prints.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::procedure::{unresolved_steps, Procedure};

/// The line rendered when a reconcile declares no required components: nothing
/// checked that the step's implementation exists, which is how a missing
/// refresher stayed invisible.
pub const MISSING_COMPONENTS_NONE: &str = "QUEUE GAP: no required components declared — nothing verified that the step's implementation exists";

/// The maximum number of issue numbers a report line names; the count is always
/// exact, the list is for triage.
pub const MAX_LISTED_ISSUES: usize = 20;

/// The reconciliation of what should be dispatchable against what is queued,
/// with the legitimate exclusions kept as their own count (#4450).
///
/// Every vector is deduplicated and ascending, so the counts are set counts and
/// the lines are stable across runs over the same inputs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub struct QueueGap {
    /// Filed issues that should reach dispatch (open + eligibility label).
    pub eligible: Vec<u64>,
    /// Issue numbers present in the queue artifact.
    pub queued: Vec<u64>,
    /// Issues legitimately absent from the queue: they already have a branch or
    /// a PR, so the refresher excludes them on purpose.
    pub has_branch_or_pr: Vec<u64>,
    /// `eligible - queued - has_branch_or_pr`: filed, labelled, and in neither
    /// set. This is the defect count.
    pub missing: Vec<u64>,
}

impl QueueGap {
    /// Reconcile the three inputs. The inputs may be unsorted, duplicated, or
    /// overlap; `missing` is recomputed from the set difference, never trusted
    /// from a caller.
    pub fn new(
        eligible: impl IntoIterator<Item = u64>,
        queued: impl IntoIterator<Item = u64>,
        has_branch_or_pr: impl IntoIterator<Item = u64>,
    ) -> Self {
        let eligible: BTreeSet<u64> = eligible.into_iter().collect();
        let queued: BTreeSet<u64> = queued.into_iter().collect();
        let covered: BTreeSet<u64> = has_branch_or_pr.into_iter().collect();
        let missing = eligible
            .difference(&queued)
            .copied()
            .collect::<BTreeSet<u64>>()
            .difference(&covered)
            .copied()
            .collect::<Vec<u64>>();
        Self {
            eligible: eligible.into_iter().collect(),
            queued: queued.into_iter().collect(),
            has_branch_or_pr: covered.into_iter().collect(),
            missing,
        }
    }

    /// The number of filed issues that should reach dispatch.
    pub fn eligible_count(&self) -> usize {
        self.eligible.len()
    }

    /// The number of issues in the queue artifact.
    pub fn queued_count(&self) -> usize {
        self.queued.len()
    }

    /// The number of issues absent from the queue because a branch or PR covers
    /// them.
    pub fn has_branch_or_pr_count(&self) -> usize {
        self.has_branch_or_pr.len()
    }

    /// The number of issues in none of the three sets: the defect count.
    pub fn missing_count(&self) -> usize {
        self.missing.len()
    }

    /// A non-empty difference is a defect. Nothing here corrects it: the reason
    /// issues stopped flowing is diagnosable only while they are still missing.
    pub fn is_defect(&self) -> bool {
        !self.missing.is_empty()
    }

    /// Whether the partition holds: `eligible` is exactly the disjoint union of
    /// what the queue holds, what a branch or PR covers, and what is missing.
    /// A reconciliation whose numbers do not add up is a counting defect, so
    /// every line it prints is untrustworthy.
    pub fn reconciles(&self) -> bool {
        let eligible: BTreeSet<u64> = self.eligible.iter().copied().collect();
        let queued: BTreeSet<u64> = self.queued.iter().copied().collect();
        let covered: BTreeSet<u64> = self.has_branch_or_pr.iter().copied().collect();
        let expected = eligible
            .difference(&queued)
            .copied()
            .collect::<BTreeSet<u64>>()
            .difference(&covered)
            .copied()
            .collect::<BTreeSet<u64>>();
        let reported = self.missing.iter().copied().collect::<BTreeSet<u64>>();
        expected == reported
    }

    /// The report line: **all four counts, on every run**, including the run
    /// where the difference is zero. The zero is the evidence the reconciler
    /// ran; suppressing it makes the missing run indistinguishable from a
    /// healthy one.
    pub fn line(&self) -> String {
        let counts = format!(
            "eligible {}, queued {}, has_branch_or_pr {}, missing {}",
            self.eligible_count(),
            self.queued_count(),
            self.has_branch_or_pr_count(),
            self.missing_count()
        );
        if !self.is_defect() {
            return format!("queue gap: {counts}");
        }
        format!(
            "QUEUE GAP DEFECT: {counts} ({}); filed work never reached dispatch — reported, not corrected: find out why the refresher stopped adding it",
            list_issues(&self.missing)
        )
    }
}

/// The issue numbers for a report line: up to [`MAX_LISTED_ISSUES`] of them,
/// then `+N more` so the count stays honest when the list is long.
fn list_issues(numbers: &[u64]) -> String {
    let head = join_numbers(numbers.iter().take(MAX_LISTED_ISSUES).copied());
    if numbers.len() > MAX_LISTED_ISSUES {
        format!(
            "{head}, +{} more ({} named in total)",
            numbers.len() - MAX_LISTED_ISSUES,
            numbers.len()
        )
    } else {
        head
    }
}

fn join_numbers(numbers: impl Iterator<Item = u64>) -> String {
    numbers
        .map(|number| number.to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

/// A step the run requires whose implementation does not resolve — the absence
/// that must be an error rather than a skipped step (#4450, #3772).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct MissingComponent {
    /// The step that names the component, as the procedure declares it.
    pub step: String,
    /// The command the step names, which resolves to nothing.
    pub command: String,
}

impl MissingComponent {
    /// The report text: the message names the step *and* the command that is
    /// missing, and states the rule — an absent implementation fails the run
    /// instead of turning the step into a no-op.
    pub fn line(&self) -> String {
        format!(
            "MISSING COMPONENT: step {} names {}, which resolves to no implementation — an absent component is an error, never a no-op",
            self.step, self.command
        )
    }
}

impl From<crate::procedure::UnresolvedStep> for MissingComponent {
    fn from(step: crate::procedure::UnresolvedStep) -> Self {
        Self {
            step: step.step,
            command: step.command,
        }
    }
}

/// Every required step of `procedure` whose implementation does not resolve
/// under `resolves`, in procedure order.
///
/// Resolution is injected (the same contract as [`crate::procedure::validate`]):
/// the check proves the artifact exists without running it. A resolution that
/// cannot run is not this function's decision — the caller passes a resolver
/// that answers.
pub fn missing_components(
    procedure: &Procedure,
    resolves: &impl Fn(&str) -> bool,
) -> Vec<MissingComponent> {
    unresolved_steps(procedure, resolves)
        .into_iter()
        .map(MissingComponent::from)
        .collect()
}
