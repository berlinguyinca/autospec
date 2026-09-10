//! Rule 1 — re-baseline before finalising (issue #3708).
//!
//! A patch is finalised against the trunk tip, never against the checkout a
//! run happened to start from: fetch the trunk, rebase the work, re-run the
//! gates. The plan is ordered, and its last step is the gates, because a
//! green receipt stamped on the old base says nothing about the rebase
//! ([`receipt_currency`]).

use crate::rebaseline::BaseDrift;

/// One step of the re-baseline a run must take before its patch is emitted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RebaselineStep {
    /// Fetch the trunk: the tip must be read from the remote, not from the
    /// checkout's idea of where `main` was.
    FetchTrunk,
    /// Rebase the work onto the fetched tip.
    Rebase { onto: String },
    /// Re-run the gates against the new base. A green gate from the old base
    /// says nothing about the tree the patch will land on.
    RerunGates,
}

impl RebaselineStep {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::FetchTrunk => "fetch_trunk",
            Self::Rebase { .. } => "rebase",
            Self::RerunGates => "rerun_gates",
        }
    }

    /// The imperative form a dispatch brief or monitor log line shows.
    pub fn imperative(&self) -> String {
        match self {
            Self::FetchTrunk => "rebaseline: fetch the trunk tip".to_string(),
            Self::Rebase { onto } => format!("rebaseline: rebase onto {onto}"),
            Self::RerunGates => "rebaseline: re-run the gates against the new base".to_string(),
        }
    }
}

/// What a run must do before its patch may be emitted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FinalisePlan {
    /// The base is still the tip: emit as-is.
    Emit,
    /// The trunk moved: re-baseline, in order, and only then emit.
    Rebaseline { steps: Vec<RebaselineStep> },
}

impl FinalisePlan {
    /// True when no re-baseline is needed.
    pub fn emits_directly(&self) -> bool {
        matches!(self, Self::Emit)
    }

    /// The ordered imperative steps, for a log line or a dispatch brief.
    pub fn lines(&self) -> Vec<String> {
        match self {
            Self::Emit => vec!["emit: base is the trunk tip".to_string()],
            Self::Rebaseline { steps } => steps.iter().map(RebaselineStep::imperative).collect(),
        }
    }
}

/// Decides what a run must do before it finalises.
///
/// A drift of zero commits is the only case that emits directly. Any other
/// drift re-baselines: the alternative is a patch whose every downstream
/// verdict — gate, review, hold — was computed against a tree it will not
/// land on, and those verdicts are then re-tested by someone else at
/// conversion time, which is the cost this rule moves to the cheapest host.
pub fn pre_finalise_plan(drift: &BaseDrift) -> FinalisePlan {
    if drift.at_tip() {
        return FinalisePlan::Emit;
    }
    FinalisePlan::Rebaseline {
        steps: vec![
            RebaselineStep::FetchTrunk,
            RebaselineStep::Rebase {
                onto: drift.tip_sha.clone(),
            },
            RebaselineStep::RerunGates,
        ],
    }
}

/// Whether a gate result still says anything about the tree the patch will
/// land on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiptCurrency {
    /// The receipt was recorded on the tip it is being judged against.
    Valid,
    /// The receipt was recorded on some other revision — a green gate from
    /// before a rebase, or from a base the trunk has left behind. It is not
    /// evidence about this patch on this base; re-run the stage.
    Void,
}

/// Grades a gate receipt by the revision it was recorded on.
///
/// A green gate is evidence about the tree it ran on and no other. This is
/// the [`crate::conversion_gate`] scope rule read from the other end: a gate
/// that never saw the current tree cannot have verified it, whatever its exit
/// code said.
pub fn receipt_currency(receipt_base: &str, tip_sha: &str) -> ReceiptCurrency {
    if receipt_base.trim() == tip_sha.trim() {
        ReceiptCurrency::Valid
    } else {
        ReceiptCurrency::Void
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rebaseline::support::{drift, BASE, TIP};

    #[test]
    fn an_untouched_base_emits_directly() {
        assert_eq!(pre_finalise_plan(&drift(0)), FinalisePlan::Emit);
        assert!(pre_finalise_plan(&drift(0)).emits_directly());
    }

    #[test]
    fn a_moved_trunk_rebaselines_in_order() {
        let plan = pre_finalise_plan(&drift(1));
        let FinalisePlan::Rebaseline { ref steps } = plan else {
            panic!("a drifted base must not emit directly");
        };
        assert_eq!(
            *steps,
            vec![
                RebaselineStep::FetchTrunk,
                RebaselineStep::Rebase {
                    onto: TIP.to_string()
                },
                RebaselineStep::RerunGates,
            ]
        );
        assert_eq!(plan.lines().len(), 3);
        assert!(plan.lines()[1].contains(TIP));
    }

    #[test]
    fn gates_run_before_a_rebase_are_void() {
        assert_eq!(receipt_currency(BASE, BASE), ReceiptCurrency::Valid);
        assert_eq!(receipt_currency(BASE, TIP), ReceiptCurrency::Void);
        assert_eq!(receipt_currency(TIP, TIP), ReceiptCurrency::Valid);
    }
}
