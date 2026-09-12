//! Rule 5 — both results, or the attribution is a guess (issue #4070).
//!
//! Drift was the largest loss in the conversion pipeline: of the test-failure
//! holds, most were `VERIFIED` patches — green on the tree the agent produced,
//! against the base it was dispatched from — that failed only because `main`
//! moved ten to seventy commits while the patch was in flight. The failure was
//! invisible at every step: the agent's gates were green, the runner graded
//! honestly, the conversion pass reported a truthful test failure, and the
//! work was lost anyway.
//!
//! `VERIFIED` answers a question about a tree that no longer exists: "this
//! patch was correct against commit X". The only question that matters at
//! conversion time is "is this patch correct against `main` now". The fix is
//! not a naive rebase-and-retest at the end of the run — that re-runs the
//! full suite on a tree the agent never saw, and a failure there is not
//! attributable to the agent, which is the false attribution this pipeline
//! has already been corrected for twice. What is needed is **both results**,
//! recorded separately, so the pipeline can distinguish "this patch is wrong"
//! from "this patch aged out".
//!
//! 1. **Both results are recorded, each with the commit it was checked
//!    against** ([`CheckReceipt`], [`DualVerification::new`]). A receipt
//!    stamped on a commit that is not the endpoint of the recorded drift is
//!    refused at construction: the two are independently observed, and a
//!    disagreement is a measurement bug, not a fact about the patch.
//! 2. **Green on base, red on head is `SUPERSEDED`, not a failure**
//!    ([`DualVerification::classify`]). It does not block its issue from
//!    re-dispatch — the work is re-attempted against current `main` — and the
//!    reason names the tests that differ and how far the base had drifted.
//! 3. **The conversion pass consumes the two results and reports drift
//!    losses separately from defects** ([`tally`], [`LossTally`]). A tally
//!    with one bucket for test failures conflates the two and re-hides the
//!    40% of losses that are the pipeline's own throughput, not the
//!    agents' quality.
//! 4. **The drift rate is reported** ([`DriftRate`]): merges to `main` per
//!    hour against the median time from dispatch to conversion, so the
//!    trade-off between fleet throughput and patch invalidation is visible
//!    rather than implicit — a queue drained twice as fast is not twice as
//!    productive, and the number that says when is this one.
//!
//! Everything here is pure: no I/O, no git, no clock. The caller runs the
//! suite against the two commits, measures the drift, and consumes the
//! verdict.

use crate::rebaseline::{BaseDrift, MeasureError};

/// The result of one suite run, stamped on the commit it ran against.
///
/// An empty failing-test list is a pass; the commit is what keeps the result
/// honest — a green suite on an old tree is not a green suite on `main`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckReceipt {
    /// The commit the suite ran against.
    pub commit_sha: String,
    /// The failing tests, in the order the harness reported them.
    pub failed_tests: Vec<String>,
}

impl CheckReceipt {
    /// Builds a receipt. An empty commit is refused: a result with no commit
    /// is a missing fact about which tree it describes, never a default.
    /// Empty test names are dropped; the harness does not report them.
    pub fn new(
        commit_sha: &str,
        failed_tests: impl IntoIterator<Item = impl AsRef<str>>,
    ) -> Result<Self, MeasureError> {
        let commit = commit_sha.trim();
        if commit.is_empty() {
            return Err("commit_sha is empty: a check result without a commit describes no tree");
        }
        Ok(Self {
            commit_sha: commit.to_string(),
            failed_tests: failed_tests
                .into_iter()
                .map(|t| t.as_ref().trim().to_string())
                .filter(|t| !t.is_empty())
                .collect(),
        })
    }

    /// True when the suite was green on this commit.
    pub fn passed(&self) -> bool {
        self.failed_tests.is_empty()
    }
}

/// The two results a run owes before its patch may be judged at all.
///
/// Verification answers a question about a tree; these are the answers to
/// the two questions that matter — against the base the run was dispatched
/// from, and against the trunk tip at the end of the run — with the drift
/// between them, so a downstream hold can be read as "the patch is wrong"
/// versus "the patch aged out" without re-running anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DualVerification {
    /// The suite result against the base the run was dispatched from.
    pub verified_against_base: CheckReceipt,
    /// The suite result against the current trunk tip at the end of the run.
    pub verified_against_head: CheckReceipt,
    /// How far the trunk moved between the two commits.
    pub drift: BaseDrift,
}

impl DualVerification {
    /// Builds the record and cross-checks it.
    ///
    /// Each receipt is stamped on the commit it ran against and the drift is
    /// measured between two commits; the receipts must land on the drift's
    /// endpoints. A receipt stamped elsewhere is a measurement bug — the
    /// suite ran on a tree neither the base nor the tip — and is refused
    /// here rather than discovered, misattributed, at conversion time.
    pub fn new(
        verified_against_base: &CheckReceipt,
        verified_against_head: &CheckReceipt,
        drift: &BaseDrift,
    ) -> Result<Self, MeasureError> {
        if verified_against_base.commit_sha != drift.base_sha {
            return Err(
                "verified_against_base is stamped on a commit that is not the drift base: measure the drift from the commit the suite actually ran on",
            );
        }
        if verified_against_head.commit_sha != drift.tip_sha {
            return Err(
                "verified_against_head is stamped on a commit that is not the trunk tip: the head check ran on a tree that is neither the base nor the tip",
            );
        }
        Ok(Self {
            verified_against_base: verified_against_base.clone(),
            verified_against_head: verified_against_head.clone(),
            drift: drift.clone(),
        })
    }

    /// The attribution the two results carry.
    ///
    /// Red on its own base is a defect: the agent's tree failed its own gate,
    /// whatever `main` has since done. Green on both converts. Green on base
    /// and red on head is `SUPERSEDED` — the patch aged out, the work is
    /// re-attempted against current `main`, and the issue is never blocked.
    pub fn classify(&self) -> DualVerdict {
        if !self.verified_against_base.passed() {
            return DualVerdict::Defect;
        }
        if self.verified_against_head.passed() {
            return DualVerdict::Convert;
        }
        DualVerdict::Superseded {
            new_failures: self.verified_against_head.failed_tests.clone(),
        }
    }

    /// The record a conversion pass consumes: both results with their
    /// commits, the drift between them, and the verdict. One line of
    /// `key=value` tokens, so it can be written to the run's status file and
    /// read back without re-running a suite.
    pub fn render(&self) -> String {
        format!(
            "verified_against_base={}={} verified_against_head={}={} drift_commits={} verdict={}",
            self.verified_against_base.commit_sha,
            outcome(&self.verified_against_base),
            self.verified_against_head.commit_sha,
            outcome(&self.verified_against_head),
            self.drift.commits_behind,
            self.classify().label(),
        )
    }
}

fn outcome(receipt: &CheckReceipt) -> String {
    if receipt.passed() {
        "PASS".to_string()
    } else {
        format!("FAIL({})", receipt.failed_tests.len())
    }
}

/// What the two results say about the patch, and what happens to the issue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DualVerdict {
    /// Red against its own base: the patch is wrong, not aged. Attributed
    /// to the run and held like any other test failure.
    Defect,
    /// Green on base, red on head: the patch aged out. Re-attempted against
    /// current `main`; the issue stays eligible.
    Superseded {
        /// The tests that differ between the two results — everything that
        /// passed on the base and fails on the tip.
        new_failures: Vec<String>,
    },
    /// Green on both: convert.
    Convert,
}

impl DualVerdict {
    /// The label the record and the tally carry.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Defect => "defect",
            Self::Superseded { .. } => "SUPERSEDED",
            Self::Convert => "convert",
        }
    }

    /// Whether the verdict blocks its issue from re-dispatch.
    ///
    /// Only a defect blocks: that is the one case where the patch itself is
    /// the problem. A superseded patch is routed to a fast re-attempt, and a
    /// convert is already done — holding either of them is the loss this
    /// rule exists to remove.
    pub fn blocks_issue(&self) -> bool {
        matches!(self, Self::Defect)
    }

    /// The human-readable reason, for the hold line or the monitor log.
    ///
    /// A `SUPERSEDED` reason names the tests that differ and how far the base
    /// had drifted: that is the evidence the work is re-attempted, not
    /// re-held, and it keeps "this patch is wrong" from being conflated with
    /// "this patch aged out" in any downstream report.
    pub fn reason(&self, v: &DualVerification) -> String {
        match self {
            Self::Defect => format!(
                "DEFECT: failed against its own base {} ({}) — the patch is wrong, not aged; hold",
                v.verified_against_base.commit_sha,
                tests_or_none(&v.verified_against_base.failed_tests)
            ),
            Self::Superseded { new_failures } => format!(
                "SUPERSEDED: passed against base {}, failed against head {} after {} commits of drift; differing tests: {} — re-attempt against current main, do not hold",
                v.verified_against_base.commit_sha,
                v.verified_against_head.commit_sha,
                v.drift.commits_behind,
                tests_or_none(new_failures),
            ),
            Self::Convert => format!(
                "CONVERT: green against base {} and head {}",
                v.verified_against_base.commit_sha,
                v.verified_against_head.commit_sha
            ),
        }
    }
}

fn tests_or_none(tests: &[String]) -> String {
    if tests.is_empty() {
        String::from("none")
    } else {
        tests.join(", ")
    }
}

/// The conversion tally, with drift losses in a bucket of their own.
///
/// The two loss categories have different causes and different fixes — a
/// defect is re-worked, a superseded patch is re-attempted — so a tally that
/// merges them into one "test failures" count re-hides the trade-off between
/// fleet throughput and patch invalidation that this rule makes visible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LossTally {
    /// Green on both: converted.
    pub converted: u64,
    /// Red against its own base: a defect in the patch.
    pub defects: u64,
    /// Green on base, red on head: drift losses, not defects.
    pub superseded: u64,
}

/// Aggregates the verdicts one conversion pass produced.
///
/// The pass consumes the two results recorded at the end of each run; it
/// does not re-derive them, and it reports the buckets separately, so the
/// drift share of the losses is a number in the pass summary rather than an
/// after-the-fact audit.
pub fn tally<'a>(verdicts: impl IntoIterator<Item = &'a DualVerdict>) -> LossTally {
    let mut t = LossTally {
        converted: 0,
        defects: 0,
        superseded: 0,
    };
    for v in verdicts {
        match v {
            DualVerdict::Convert => t.converted += 1,
            DualVerdict::Defect => t.defects += 1,
            DualVerdict::Superseded { .. } => t.superseded += 1,
        }
    }
    t
}

impl LossTally {
    /// The pass summary line: three buckets, drift never merged into
    /// defects.
    pub fn line(&self) -> String {
        format!(
            "converted={} defects={} drift_losses={}",
            self.converted, self.defects, self.superseded
        )
    }

    /// The drift share of everything the pass did not convert, as a
    /// percentage. `None` when nothing was lost: a share of zero losses is
    /// not a measurement.
    pub fn drift_share_percent(&self) -> Option<f64> {
        let lost = self.defects + self.superseded;
        if lost == 0 {
            return None;
        }
        Some(self.superseded as f64 * 100.0 / lost as f64)
    }
}

/// The drift rate: how fast the trunk moves against how long a patch lives.
///
/// The pipeline's loss rate is this number, and nothing in the pipeline
/// worsens it more slowly — every successful merge invalidates every
/// in-flight patch. Reporting merges per hour against the median dispatch-to-
/// conversion lag makes the trade-off between fleet throughput and patch
/// invalidation visible, so "drain the queue faster" is a decision made with
/// the number in front of it rather than an implicit assumption.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DriftRate {
    /// Merges to the trunk per hour.
    pub merges_per_hour: f64,
    /// The median time from dispatch to conversion, in seconds.
    pub median_lag_secs: u64,
}

impl DriftRate {
    /// Builds the rate from a measured merge count, the observation window it
    /// covers, and the median dispatch-to-conversion lag.
    ///
    /// `None` when the window or the lag is zero: a zero-length window or a
    /// zero-length patch life is not a measurement, and a rate computed from
    /// one would be a number that happens to be comparable to other numbers.
    /// Zero merges in a real window is a measurement: the rate is zero.
    pub fn new(merges: u64, window_secs: u64, median_lag_secs: u64) -> Option<Self> {
        if window_secs == 0 || median_lag_secs == 0 {
            return None;
        }
        Some(Self {
            merges_per_hour: merges as f64 * 3600.0 / window_secs as f64,
            median_lag_secs,
        })
    }

    /// Expected trunk advances during one median patch lifetime: the
    /// per-patch invalidation pressure.
    pub fn advances_during_median_lag(&self) -> f64 {
        self.merges_per_hour * self.median_lag_secs as f64 / 3600.0
    }

    /// The report line.
    pub fn line(&self) -> String {
        format!(
            "drift rate: {:.1} merges/hour against a median dispatch-to-conversion lag of {}s -> {:.1} trunk advance(s) per patch",
            self.merges_per_hour,
            self.median_lag_secs,
            self.advances_during_median_lag(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rebaseline::support::{drift, BASE, TIP};

    fn green(commit: &str) -> CheckReceipt {
        CheckReceipt::new(commit, Vec::<&str>::new()).unwrap()
    }

    fn red(commit: &str, tests: &[&str]) -> CheckReceipt {
        CheckReceipt::new(commit, tests).unwrap()
    }

    fn both(base: &CheckReceipt, head: &CheckReceipt, behind: u64) -> DualVerification {
        DualVerification::new(base, head, &drift(behind)).unwrap()
    }

    // --- rule 1: both results are recorded, cross-checked ------------------

    #[test]
    fn a_receipt_needs_a_commit() {
        assert!(CheckReceipt::new("", ["t1"]).is_err());
        assert!(green(BASE).passed());
        assert!(!red(BASE, &["t1"]).passed());
    }

    #[test]
    fn a_receipt_stamped_off_the_drift_endpoints_is_refused() {
        let base = green(BASE);
        let head = green(TIP);
        let d = drift(6);
        // Head receipt stamped on a commit that is neither endpoint.
        let stale = red("cccc3333", &[]);
        assert!(DualVerification::new(&base, &stale, &d).is_err());
        // Base receipt stamped on the tip.
        let swapped = green(TIP);
        assert!(DualVerification::new(&swapped, &head, &d).is_err());
        assert!(DualVerification::new(&base, &head, &d).is_ok());
    }

    #[test]
    fn the_endpoint_errors_name_the_field_not_the_commits() {
        let base = green(BASE);
        let head = green(TIP);
        let d = drift(6);
        let stale = red("cccc3333", &[]);
        assert_eq!(
            DualVerification::new(&base, &stale, &d).unwrap_err(),
            "verified_against_head is stamped on a commit that is not the trunk tip: the head check ran on a tree that is neither the base nor the tip"
        );
        assert_eq!(
            DualVerification::new(&red(TIP, &[]), &head, &d).unwrap_err(),
            "verified_against_base is stamped on a commit that is not the drift base: measure the drift from the commit the suite actually ran on"
        );
    }

    // --- rule 2: the classification -----------------------------------------

    #[test]
    fn green_on_both_converts() {
        let v = both(&green(BASE), &green(TIP), 6);
        let verdict = v.classify();
        assert_eq!(verdict, DualVerdict::Convert);
        assert!(!verdict.blocks_issue());
        assert!(verdict.reason(&v).contains("CONVERT"));
    }

    #[test]
    fn red_on_both_is_a_defect() {
        let v = both(&red(BASE, &["t1", "t2"]), &red(TIP, &["t1"]), 6);
        let verdict = v.classify();
        assert_eq!(verdict, DualVerdict::Defect);
        // A red base blocks, whatever the head says: the patch is wrong.
        assert!(verdict.blocks_issue());
        let reason = verdict.reason(&v);
        assert!(reason.contains("DEFECT"));
        assert!(reason.contains("t1, t2"));
        assert!(reason.contains(BASE));
    }

    #[test]
    fn green_on_base_red_on_head_is_superseded_and_leaves_the_issue_eligible() {
        let v = both(
            &green(BASE),
            &red(TIP, &["crate_a::slow_test", "crate_b::io_test"]),
            12,
        );
        let verdict = v.classify();
        assert_eq!(
            verdict,
            DualVerdict::Superseded {
                new_failures: vec!["crate_a::slow_test".into(), "crate_b::io_test".into()]
            }
        );
        // The issue stays eligible for re-dispatch: this is a re-attempt,
        // not a hold.
        assert!(!verdict.blocks_issue());
        assert_eq!(verdict.label(), "SUPERSEDED");
        // The reason names the tests that differ and the drift distance.
        let reason = verdict.reason(&v);
        assert!(reason.contains("SUPERSEDED"));
        assert!(reason.contains("crate_a::slow_test, crate_b::io_test"));
        assert!(reason.contains("12 commits of drift"));
        assert!(reason.contains(BASE));
        assert!(reason.contains(TIP));
    }

    #[test]
    fn zero_drift_green_on_both_still_converts() {
        let v = both(&green(BASE), &green(BASE), 0);
        assert_eq!(v.classify(), DualVerdict::Convert);
    }

    // --- the record a conversion pass consumes ------------------------------

    #[test]
    fn render_carries_both_results_their_commits_and_the_verdict() {
        let v = both(&green(BASE), &red(TIP, &["t1", "t2"]), 12);
        let line = v.render();
        assert!(line.contains(&format!("verified_against_base={BASE}=PASS")));
        assert!(line.contains(&format!("verified_against_head={TIP}=FAIL(2)")));
        assert!(line.contains("drift_commits=12"));
        assert!(line.contains("verdict=SUPERSEDED"));
    }

    // --- rule 3: drift losses are a bucket of their own ---------------------

    #[test]
    fn the_tally_keeps_drift_losses_out_of_the_defect_count() {
        let v_defect = both(&red(BASE, &["t1"]), &red(TIP, &["t1"]), 3);
        let v_superseded = both(&green(BASE), &red(TIP, &["t2"]), 9);
        let v_convert = both(&green(BASE), &green(TIP), 1);
        let verdicts = [
            v_defect.classify(),
            v_superseded.classify(),
            v_convert.classify(),
        ];
        let t = tally(verdicts.iter());
        assert_eq!(
            t,
            LossTally {
                converted: 1,
                defects: 1,
                superseded: 1
            }
        );
        assert_eq!(t.line(), "converted=1 defects=1 drift_losses=1");
        // Half of what was lost was drift, and the tally says so.
        assert_eq!(t.drift_share_percent(), Some(50.0));
        let none: Vec<DualVerdict> = Vec::new();
        assert!(tally(none.iter()).drift_share_percent().is_none());
    }

    #[test]
    fn a_pass_with_no_losses_reports_zero_buckets_not_no_buckets() {
        // Zero drift: both receipts are stamped on the same commit.
        let v = both(&green(BASE), &green(BASE), 0);
        let verdict = v.classify();
        let t = tally([&verdict]);
        assert_eq!(t.line(), "converted=1 defects=0 drift_losses=0");
        assert_eq!(t.drift_share_percent(), None);
    }

    // --- rule 4: the drift rate ---------------------------------------------

    #[test]
    fn the_rate_is_merges_per_hour_against_the_median_lag() {
        // The measured fleet: 61 merges in the last six hours, one-hour
        // median runs.
        let r = DriftRate::new(61, 6 * 3600, 3600).unwrap();
        assert!((r.merges_per_hour - 10.166_666).abs() < 0.001);
        // One-hour lag at ~10 merges/hour: essentially one trunk advance per
        // patch, which is the number that says the loss is structural.
        assert!((r.advances_during_median_lag() - 10.166_666).abs() < 0.001);
        let line = r.line();
        assert!(line.contains("10.2 merges/hour"));
        assert!(line.contains("median dispatch-to-conversion lag of 3600s"));
        assert!(line.contains("10.2 trunk advance(s) per patch"));
    }

    #[test]
    fn a_zero_window_or_lag_is_not_a_measurement() {
        assert_eq!(DriftRate::new(10, 0, 3600), None);
        assert_eq!(DriftRate::new(10, 3600, 0), None);
        // Zero merges in a real window is a measurement: the rate is zero.
        let r = DriftRate::new(0, 3600, 3600).unwrap();
        assert_eq!(r.advances_during_median_lag(), 0.0);
    }
}
