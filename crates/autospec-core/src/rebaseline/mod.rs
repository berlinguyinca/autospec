//! Re-baseline in-flight work before it is finalised (issue #3708).
//!
//! One issue was dispatched five times in about two hours and held every
//! time — ten patches held on **base drift** in that session, none of them
//! for the agent's own quality. Each field the rejected patches were missing
//! had been added by a pull request that merged *while* the agent was still
//! running: seven merges to the trunk in 2h50m, one every ~24 minutes,
//! against runs of 7–45 minutes, and every one of those merges was the
//! supervisor converting finished work.
//!
//! The loop is self-inflicted. Each merge invalidates **every** in-flight
//! patch, so the cost of one merge is multiplied by the number of patches in
//! flight, and the merge rate is the project's own success rate. Nothing in
//! the pipeline makes the base move slower; the only lever is work happening
//! closer to the trunk. Five rules this module encodes, plus the two
//! classifications that let the converter stop guessing:
//!
//! 1. **An agent re-baselines before it finalises** ([`pre_finalise_plan`]).
//!    A patch is finalised against the trunk tip, not against the checkout it
//!    happened to start from: fetch, rebase, re-run the gates. Work measured
//!    against a base the patch will not land on is not evidence about the
//!    patch ([`receipt_currency`]).
//! 2. **A patch names the base it was verified against** ([`PatchMeta`]). The
//!    converter then distinguishes "stale but clean" from "conflicts in its
//!    own subject matter" without applying anything, and a patch emitted
//!    without a base ([`MetaViolation`]) is a defect in the emit, not a
//!    missing fact about the patch.
//! 3. **A `HELD` is a hypothesis, not a decision** ([`hold_status`]). A hold
//!    recorded against a trunk that has since moved `N` commits re-tests as
//!    an experiment, never as a re-affirmation; a hold recorded against the
//!    current tip is a decision and re-testing it costs the same answer.
//! 4. **Drift is measured, not assumed** ([`BaseDrift`], [`DriftExposure`]).
//!    A base that is the tip has drifted zero commits; a base that is not the
//!    tip is never zero behind. The exposure ratio says whether the pipeline
//!    is running faster than the trunk moves at all.
//! 5. **Both results, or the attribution is a guess** (issue #4070,
//!    [`dual`]): the run verifies the patch against its dispatch base *and*
//!    against the current trunk tip, recording each result with the commit it
//!    was checked against. Green on base and red on head is `SUPERSEDED` — a
//!    drift loss routed to a fast re-attempt, never a defect that blocks the
//!    issue — and the conversion pass reports drift losses in a bucket of
//!    their own, alongside the drift rate the loss is really a function of.
//!
//! Everything here is pure and testable: no I/O, no git, no clock. The caller
//! measures the base against the tip and acts on the verdict.
//!
//! The module is split by rule: `finalise` (re-baseline before emit),
//! `patch_meta` (the emit names its base), `verdict` (what a hold and a
//! clean-but-stale apply are worth), `dual` (both results and the drift
//! loss). This file holds the measurement every other rule is computed from.

mod dual;
mod finalise;
mod patch_meta;
mod verdict;

pub use dual::{tally, CheckReceipt, DriftRate, DualVerdict, DualVerification, LossTally};
pub use finalise::{
    pre_finalise_plan, receipt_currency, FinalisePlan, RebaselineStep, ReceiptCurrency,
};
pub use patch_meta::{parse_patch_meta, MetaViolation, PatchMeta};
pub use verdict::{
    classify_staleness, hold_status, DriftExposure, ExposureClass, HoldStatus, Staleness,
    DEFAULT_HELD_HYPOTHESIS_COMMITS,
};

/// Errors from constructing a measurement out of two revision ids.
pub type MeasureError = &'static str;

// ---------------------------------------------------------------------------
// Rule 4 — the drift measurement
// ---------------------------------------------------------------------------

/// How far the trunk moved between the base a run started from and the tip
/// that run is being judged against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaseDrift {
    /// The commit the work was based on.
    pub base_sha: String,
    /// The trunk tip it is being measured against.
    pub tip_sha: String,
    /// Commits between the two, counted on the trunk.
    pub commits_behind: u64,
}

impl BaseDrift {
    /// Measures a base against a tip.
    ///
    /// The count is cross-checked against the two ids, because the two are
    /// independently observed and a disagreement is a measurement bug, not a
    /// fact about the repository: identical ids with a non-zero count mean
    /// the count came from somewhere else, and different ids with a zero
    /// count mean the drift was assumed rather than measured.
    pub fn measure(
        base_sha: &str,
        tip_sha: &str,
        commits_behind: u64,
    ) -> Result<Self, MeasureError> {
        let base = base_sha.trim();
        let tip = tip_sha.trim();
        if base.is_empty() {
            return Err("base_sha is empty: a run with no base cannot be re-baselined");
        }
        if tip.is_empty() {
            return Err("tip_sha is empty: the trunk tip was never read");
        }
        if base == tip && commits_behind != 0 {
            return Err("base equals tip but commits_behind is non-zero");
        }
        if base != tip && commits_behind == 0 {
            return Err("base differs from tip but commits_behind is zero: measure, do not assume");
        }
        Ok(Self {
            base_sha: base.to_string(),
            tip_sha: tip.to_string(),
            commits_behind,
        })
    }

    /// True when the base is still the tip: nothing has moved.
    pub fn at_tip(&self) -> bool {
        self.commits_behind == 0
    }

    /// True when the trunk has moved since the base.
    pub fn drifted(&self) -> bool {
        !self.at_tip()
    }
}

/// Shared fixtures for the rule modules' tests.
#[cfg(test)]
pub(crate) mod support {
    use super::BaseDrift;

    pub(crate) const BASE: &str = "aaaa1111";
    pub(crate) const TIP: &str = "bbbb2222";

    /// A base `behind` commits from the tip (`behind == 0` means the base is
    /// still the tip, so the two ids are equal).
    pub(crate) fn drift(behind: u64) -> BaseDrift {
        if behind == 0 {
            BaseDrift::measure(BASE, BASE, 0).unwrap()
        } else {
            BaseDrift::measure(BASE, TIP, behind).unwrap()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::support::{BASE, TIP};
    use super::BaseDrift;

    #[test]
    fn measure_rejects_empty_revisions() {
        assert!(BaseDrift::measure("", TIP, 3).is_err());
        assert!(BaseDrift::measure(BASE, "  ", 3).is_err());
    }

    #[test]
    fn measure_cross_checks_count_against_ids() {
        assert!(BaseDrift::measure(BASE, BASE, 4).is_err());
        assert!(BaseDrift::measure(BASE, TIP, 0).is_err());
        let d = BaseDrift::measure(BASE, TIP, 4).unwrap();
        assert!(d.drifted());
        assert!(!d.at_tip());
    }
}
