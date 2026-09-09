//! Baseline-vs-patched test-failure differencing (#3727).
//!
//! The conversion pass stamps a patch `NEW-TEST-FAILURES` when the patched
//! run's failing-test set differs from the baseline run's. On a loaded
//! compute node a suite's failing set is *noisy*: the process-lifecycle and
//! concurrency tests fail intermittently under contention, so two runs of
//! the *same* code churn the set by one in / one out. A set difference over
//! a flaky set manufactures false verdicts in both directions — a flaky
//! failure that lands only in the patched run holds the patch, and one that
//! lands only in the baseline masks a real new failure.
//!
//! A comparison is only as sound as the stability of what it compares, so
//! the diff is treated as a *trigger*, not a verdict. This module encodes
//! the four rules that keep it sound as pure, testable primitives; the
//! caller performs the `cargo test` I/O with the plans these functions
//! return:
//!
//! 1. **The diff is never a verdict** ([`DiffVerdict`]). Non-empty new
//!    failures are a re-verification trigger, not a hold. Deterministic
//!    signals (fmt, build) remain trustworthy negatives; the test diff is
//!    not, so it never ends a patch by itself.
//! 2. **Re-run only the differing tests** ([`TestFailureDiff::reverify_set`])
//!    in isolation: the symmetric difference — seconds of compute, not the
//!    twenty minutes of a full suite.
//! 3. **Report the shape, not just the existence**
//!    ([`TestFailureDiff::shape`]): `7 -> 7, one in, one out` is
//!    diagnostic; a bare `NEW-TEST-FAILURES` stamp is not.
//! 4. **Quarantine the known-flaky family** ([`FlakyQuarantine`]): mark the
//!    load-sensitive tests so they are excluded from differencing and
//!    asserted separately, instead of churning the set.

use std::collections::BTreeSet;

/// A symmetric difference of at most this many tests, with the count
/// unchanged, is the flaky signature: churn, not regression. Re-running
/// that many tests in isolation is cheap enough to be the default move.
pub const DEFAULT_CHURN_THRESHOLD: usize = 2;

/// The known-flaky family: load-sensitive tests (process-lifecycle,
/// concurrency) that are asserted separately — on an unloaded runner, run
/// serially — instead of participating in the baseline diff.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FlakyQuarantine {
    exact: BTreeSet<String>,
    families: BTreeSet<String>,
}

impl FlakyQuarantine {
    pub fn new() -> Self {
        Self::default()
    }

    /// Quarantine one test by its exact name.
    pub fn add(&mut self, test: impl Into<String>) -> &mut Self {
        self.exact.insert(test.into());
        self
    }

    /// Quarantine a family: every test whose name starts with `prefix`.
    pub fn add_family(&mut self, prefix: impl Into<String>) -> &mut Self {
        self.families.insert(prefix.into());
        self
    }

    pub fn contains(&self, test: &str) -> bool {
        self.exact.contains(test) || self.families.iter().any(|family| test.starts_with(family))
    }

    pub fn is_empty(&self) -> bool {
        self.exact.is_empty() && self.families.is_empty()
    }
}

/// The shape of the difference between one baseline run's failing tests and
/// one patched run's, after the quarantine is applied. Counts are the raw
/// run totals (quarantined tests included); the failure lists are the
/// non-quarantined movement, quarantined movement reported separately.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TestFailureDiff {
    /// Total tests failing in the baseline run, quarantined included.
    pub baseline_count: usize,
    /// Total tests failing in the patched run, quarantined included.
    pub patched_count: usize,
    /// Non-quarantined tests failing after the patch, not before. Sorted.
    pub new_failures: Vec<String>,
    /// Non-quarantined tests failing before the patch, not after. Sorted.
    pub gone_failures: Vec<String>,
    /// Quarantined tests appearing in the patched run. Reported, never a
    /// regression: the quarantine asserts them separately. Sorted.
    pub quarantined_new: Vec<String>,
    /// Quarantined tests disappearing from the patched run. Reported. Sorted.
    pub quarantined_gone: Vec<String>,
}

/// Set-diff the failing tests of a baseline run against a patched run.
/// Input order is ignored and duplicate names collapse: the diff is over
/// the *sets* of failing tests, and membership is what the verdict hangs
/// on.
pub fn diff_test_failures(
    baseline: impl IntoIterator<Item = impl AsRef<str>>,
    patched: impl IntoIterator<Item = impl AsRef<str>>,
    quarantine: &FlakyQuarantine,
) -> TestFailureDiff {
    let baseline: BTreeSet<String> = baseline
        .into_iter()
        .map(|t| t.as_ref().to_string())
        .collect();
    let patched: BTreeSet<String> = patched
        .into_iter()
        .map(|t| t.as_ref().to_string())
        .collect();

    let mut diff = TestFailureDiff {
        baseline_count: baseline.len(),
        patched_count: patched.len(),
        ..TestFailureDiff::default()
    };

    for test in patched.difference(&baseline) {
        if quarantine.contains(test) {
            diff.quarantined_new.push(test.clone());
        } else {
            diff.new_failures.push(test.clone());
        }
    }
    for test in baseline.difference(&patched) {
        if quarantine.contains(test) {
            diff.quarantined_gone.push(test.clone());
        } else {
            diff.gone_failures.push(test.clone());
        }
    }

    diff
}

/// What a diff is allowed to conclude. A set difference over a flaky set is
/// a re-verification trigger, never a verdict: a patch is never held on
/// `NEW-TEST-FAILURES` alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffVerdict {
    /// No non-quarantined new failures. Holding the patch on test evidence
    /// is not justified.
    Clean,
    /// New failures were observed. The differing tests must be re-run in
    /// isolation (`tests`) before a regression is declared; when the diff
    /// carries the flaky signature the churn reading is the leading
    /// hypothesis, not the regression reading.
    Reverify {
        tests: Vec<String>,
        flaky_signature: bool,
    },
}

/// The outcome of re-running the differing tests in isolation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReverificationOutcome {
    /// The differing tests pass in isolation: the diff was churn. Nothing
    /// holds the patch, and the churned tests are quarantine candidates.
    Churn,
    /// At least one differing test still fails in isolation: a regression,
    /// now backed by isolated evidence rather than a loaded full-suite diff.
    Regression { confirmed: Vec<String> },
}

impl TestFailureDiff {
    /// The shape of the difference, for humans and logs:
    /// `7 -> 7, one in, one out`. Quarantined movement is reported on a
    /// separate clause so it cannot masquerade as regression evidence.
    pub fn shape(&self) -> String {
        let mut shape = format!("{} -> {}", self.baseline_count, self.patched_count);
        if self.new_failures.is_empty() && self.gone_failures.is_empty() {
            shape.push_str(", unchanged");
        } else {
            shape.push_str(&format!(
                ", {} in, {} out",
                count_word(self.new_failures.len()),
                count_word(self.gone_failures.len()),
            ));
        }
        if !self.quarantined_new.is_empty() || !self.quarantined_gone.is_empty() {
            shape.push_str(&format!(
                "; quarantined: {} in, {} out",
                count_word(self.quarantined_new.len()),
                count_word(self.quarantined_gone.len()),
            ));
        }
        shape
    }

    /// The flaky signature: the count did not change and the membership
    /// churned by a small symmetric difference — one in, one out — which is
    /// what a load-sensitive test does to a set, not what a regression does.
    pub fn is_flaky_signature(&self, threshold: usize) -> bool {
        !self.new_failures.is_empty()
            && self.new_failures.len() == self.gone_failures.len()
            && self.new_failures.len() <= threshold
    }

    /// The tests to re-run in isolation: the symmetric difference of the
    /// non-quarantined failure sets, sorted. Re-running exactly these —
    /// seconds of compute — is what turns the trigger into evidence.
    pub fn reverify_set(&self) -> Vec<String> {
        let mut set: BTreeSet<&str> = BTreeSet::new();
        set.extend(self.new_failures.iter().map(String::as_str));
        set.extend(self.gone_failures.iter().map(String::as_str));
        set.into_iter().map(String::from).collect()
    }

    /// What the diff alone is allowed to conclude: never a hold, at most a
    /// re-verification trigger over the differing tests.
    pub fn verdict(&self) -> DiffVerdict {
        if self.new_failures.is_empty() {
            DiffVerdict::Clean
        } else {
            DiffVerdict::Reverify {
                tests: self.reverify_set(),
                flaky_signature: self.is_flaky_signature(DEFAULT_CHURN_THRESHOLD),
            }
        }
    }
}

/// Resolve a re-verification trigger with the isolated re-run results:
/// `still_failing` are the reverify-set tests that failed when re-run in
/// isolation. Names outside the reverify set are ignored — a shared
/// baseline failure is not evidence about this patch.
pub fn resolve_reverification(
    diff: &TestFailureDiff,
    still_failing: impl IntoIterator<Item = impl AsRef<str>>,
) -> ReverificationOutcome {
    let reverify = diff.reverify_set();
    let expected: BTreeSet<&str> = reverify.iter().map(String::as_str).collect();
    let confirmed: Vec<String> = still_failing
        .into_iter()
        .map(|t| t.as_ref().to_string())
        .filter(|t| expected.contains(t.as_str()))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();

    if confirmed.is_empty() {
        ReverificationOutcome::Churn
    } else {
        ReverificationOutcome::Regression { confirmed }
    }
}

fn count_word(n: usize) -> String {
    match n {
        0 => "none".to_string(),
        1 => "one".to_string(),
        2 => "two".to_string(),
        n => n.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The observed incident: six shared failures, one test in, one out.
    const GONE: &str = "a_concurrent_lease_renewal_prevents_label_requeue";
    const NEW: &str =
        "autonomous_executor_bridge_partial_adoption_cleanup_failure_retains_identities";
    const SHARED: [&str; 6] = [
        "sandbox_containment_holds_under_retry",
        "scanner_subprocess_exits_cleanly",
        "adoption_cleanup_preserves_journal",
        "lease_expiry_releases_claim",
        "worker_pool_drains_on_shutdown",
        "concurrent_enqueue_serializes",
    ];

    fn incident_sets() -> (Vec<String>, Vec<String>) {
        let baseline: Vec<String> = SHARED
            .iter()
            .copied()
            .chain([GONE])
            .map(String::from)
            .collect();
        let patched: Vec<String> = SHARED
            .iter()
            .copied()
            .chain([NEW])
            .map(String::from)
            .collect();
        (baseline, patched)
    }

    #[test]
    fn the_observed_incident_reads_as_churn_not_regression() {
        let (baseline, patched) = incident_sets();
        let diff = diff_test_failures(&baseline, &patched, &FlakyQuarantine::new());

        assert_eq!(diff.shape(), "7 -> 7, one in, one out");
        assert!(diff.is_flaky_signature(DEFAULT_CHURN_THRESHOLD));
        assert_eq!(
            diff.verdict(),
            DiffVerdict::Reverify {
                tests: vec![GONE.to_string(), NEW.to_string()],
                flaky_signature: true,
            }
        );
    }

    #[test]
    fn reverify_set_is_the_symmetric_difference_in_sorted_order() {
        let (baseline, patched) = incident_sets();
        let diff = diff_test_failures(&baseline, &patched, &FlakyQuarantine::new());

        assert_eq!(diff.reverify_set(), vec![GONE.to_string(), NEW.to_string()]);
    }

    #[test]
    fn identical_sets_are_clean_and_unchanged() {
        let set: Vec<String> = SHARED.iter().copied().map(String::from).collect();
        let diff = diff_test_failures(&set, &set, &FlakyQuarantine::new());

        assert_eq!(diff.shape(), "6 -> 6, unchanged");
        assert_eq!(diff.verdict(), DiffVerdict::Clean);
        assert!(diff.reverify_set().is_empty());
        assert!(!diff.is_flaky_signature(DEFAULT_CHURN_THRESHOLD));
    }

    #[test]
    fn a_real_increase_is_still_only_a_trigger_never_a_hold() {
        let baseline: Vec<String> = SHARED[..4].iter().copied().map(String::from).collect();
        let patched: Vec<String> = SHARED.iter().copied().map(String::from).collect();
        let diff = diff_test_failures(&baseline, &patched, &FlakyQuarantine::new());

        assert_eq!(diff.shape(), "4 -> 6, two in, none out");
        assert!(!diff.is_flaky_signature(DEFAULT_CHURN_THRESHOLD));
        // Even a growing count is a re-verification trigger: the diff alone
        // never ends the patch.
        let expected = DiffVerdict::Reverify {
            tests: vec![
                "concurrent_enqueue_serializes".to_string(),
                "worker_pool_drains_on_shutdown".to_string(),
            ],
            flaky_signature: false,
        };
        assert_eq!(diff.verdict(), expected);
    }

    #[test]
    fn churn_above_the_threshold_is_not_the_flaky_signature() {
        let baseline: Vec<String> = SHARED.iter().copied().map(String::from).collect();
        let patched: Vec<String> = ["churn_a", "churn_b", "churn_c"]
            .into_iter()
            .chain(SHARED[..3].iter().copied())
            .map(String::from)
            .collect();
        let diff = diff_test_failures(&baseline, &patched, &FlakyQuarantine::new());

        // 3 in, 3 out: the count is unchanged but the churn is too large to
        // read as a single flaky family — a human looks at it.
        assert!(!diff.is_flaky_signature(DEFAULT_CHURN_THRESHOLD));
        assert!(diff.is_flaky_signature(3));
        assert!(!matches!(
            diff.verdict(),
            DiffVerdict::Reverify {
                flaky_signature: true,
                ..
            }
        ));
    }

    #[test]
    fn quarantined_movement_is_reported_and_excluded_from_the_verdict() {
        let (mut baseline, patched) = incident_sets();
        // The flaky family is quarantined by exact name and prefix: its
        // churn stops touching the verdict at all.
        let mut quarantine = FlakyQuarantine::new();
        quarantine.add_family("autonomous_executor_bridge_");
        quarantine.add(GONE);
        baseline.push("autonomous_executor_bridge_pool_recycle_holds".to_string());

        let diff = diff_test_failures(&baseline, &patched, &quarantine);

        assert!(diff.new_failures.is_empty());
        assert!(diff.gone_failures.is_empty());
        assert_eq!(diff.quarantined_new, vec![NEW.to_string()]);
        assert_eq!(
            diff.quarantined_gone,
            vec![
                GONE.to_string(),
                "autonomous_executor_bridge_pool_recycle_holds".to_string()
            ]
        );
        assert_eq!(
            diff.shape(),
            "8 -> 7, unchanged; quarantined: one in, two out"
        );
        assert_eq!(diff.verdict(), DiffVerdict::Clean);
    }

    #[test]
    fn quarantine_matches_exact_names_and_family_prefixes() {
        let quarantine = FlakyQuarantine::new();
        assert!(quarantine.is_empty());

        let mut quarantine = FlakyQuarantine::new();
        quarantine.add(GONE);
        quarantine.add_family("autonomous_executor_bridge");

        assert!(quarantine.contains(GONE)); // exact
        assert!(quarantine.contains(NEW)); // family prefix
        assert!(!quarantine.contains("unrelated_test"));
    }

    #[test]
    fn reverify_passes_in_isolation_read_as_churn() {
        let (baseline, patched) = incident_sets();
        let diff = diff_test_failures(&baseline, &patched, &FlakyQuarantine::new());

        assert_eq!(
            resolve_reverification(&diff, std::iter::empty::<&str>()),
            ReverificationOutcome::Churn
        );
    }

    #[test]
    fn reverify_failures_in_isolation_read_as_confirmed_regression() {
        let (baseline, patched) = incident_sets();
        let diff = diff_test_failures(&baseline, &patched, &FlakyQuarantine::new());

        assert_eq!(
            resolve_reverification(&diff, [NEW]),
            ReverificationOutcome::Regression {
                confirmed: vec![NEW.to_string()],
            }
        );
    }

    #[test]
    fn shared_baseline_failures_are_not_evidence_about_the_patch() {
        let (baseline, patched) = incident_sets();
        let diff = diff_test_failures(&baseline, &patched, &FlakyQuarantine::new());

        // A shared test still failing in isolation is baseline noise, not a
        // regression the patch introduced.
        assert_eq!(
            resolve_reverification(&diff, [SHARED[0]]),
            ReverificationOutcome::Churn
        );
        assert_eq!(
            resolve_reverification(&diff, [SHARED[0], NEW]),
            ReverificationOutcome::Regression {
                confirmed: vec![NEW.to_string()],
            }
        );
    }

    #[test]
    fn duplicate_and_out_of_order_inputs_collapse_to_set_membership() {
        let baseline = vec![GONE.to_string(), SHARED[0].to_string(), GONE.to_string()];
        let patched = vec![SHARED[0].to_string(), NEW.to_string()];
        let diff = diff_test_failures(&baseline, &patched, &FlakyQuarantine::new());

        assert_eq!(diff.baseline_count, 2);
        assert_eq!(diff.patched_count, 2);
        assert_eq!(diff.shape(), "2 -> 2, one in, one out");
    }
}
