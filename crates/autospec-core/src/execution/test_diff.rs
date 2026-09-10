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
//! 5. **Attribution is repeated observation, not one sample**
//!    (issue #4178, [`resolve_reverification`]). A baseline sample and a
//!    patched sample cannot distinguish "the patch broke it" from "the test
//!    is flaky": a 13-second test in a 142-test parallel suite fails under
//!    contention and passes 5/5 alone, and it was charged to two patches in
//!    one day on the strength of a single differing sample. So every
//!    reverify test is re-run in isolation (`--exact --test-threads=1`,
//!    [`isolation_reverify_args`]) at least [`ISOLATION_RUNS`] times, and
//!    only a newly-failing test that fails in *every* re-run is confirmed.
//!    An unconfirmed difference is not a verdict either: the test is
//!    identified and quarantined ([`QuarantineMark`]) — a test that passes
//!    in isolation is a resource-contention flake — and the HELD line
//!    records which claim was made, because `HELD: new test failure --
//!    <name> (confirmed: fails 3/3 in isolation)` and `(unconfirmed:
//!    passes in isolation, suspect contention)` are different claims, and
//!    only the former rejects the patch.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// A symmetric difference of at most this many tests, with the count
/// unchanged, is the flaky signature: churn, not regression. Re-running
/// that many tests in isolation is cheap enough to be the default move.
pub const DEFAULT_CHURN_THRESHOLD: usize = 2;

/// How many isolated re-runs (`--exact --test-threads=1`) a newly-failing
/// test must accumulate before its failure may be confirmed (issue #4178).
/// Attribution is repeated observation: a single failing sample — even one
/// from an isolated run — is one more flaky sample, not evidence.
pub const ISOLATION_RUNS: u32 = 3;

/// The libtest filter for the isolated re-run of one test (issue #4178,
/// invariant 1): the exact name, one thread — no parallelism to mask the
/// failure or to produce the contention a loaded suite does. Append after
/// the `--` separator of the `cargo test` invocation, e.g. `cargo test -p
/// <pkg> --test <target> -- --exact <name> --test-threads=1`.
pub fn isolation_reverify_args(name: &str) -> Vec<String> {
    vec![
        "--".to_string(),
        "--exact".to_string(),
        name.to_string(),
        "--test-threads=1".to_string(),
    ]
}

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

/// The outcome of re-running the differing tests in isolation, repeatedly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReverificationOutcome {
    /// No differing test was confirmed: nothing in the diff is backed by
    /// repeated isolated observation, so nothing holds the patch. The
    /// differing tests are churn — flaky, or a load artifact of the run
    /// pair — and the unconfirmed ones are quarantine candidates.
    Unconfirmed,
    /// At least one newly-failing test failed in every one of at least
    /// [`ISOLATION_RUNS`] isolated re-runs: a regression, now backed by
    /// repeated isolated observation rather than a single diff sample.
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

/// The repeated observation of one reverify test (issue #4178): the
/// outcome of its isolated re-runs. The diff only points at the test; this
/// is the evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reobservation {
    /// The test's full name.
    pub name: String,
    /// Newly failing: in the patched run's failure set and not in the
    /// baseline's. The only direction a failure can be charged to the patch
    /// — a test that stopped failing passed in the patched run, so its
    /// isolated failures are noise about the baseline, not about the patch.
    pub newly_failing: bool,
    /// Failed in this many of the isolated re-runs.
    pub failed_runs: u32,
    /// The number of isolated re-runs observed.
    pub runs: u32,
}

impl Reobservation {
    /// Confirmed: failed in *every* isolated re-run, with at least
    /// [`ISOLATION_RUNS`] of them. A single failing sample — `1/1` — is not
    /// confirmation; it is one more flaky sample.
    pub fn confirmed(&self) -> bool {
        self.runs >= ISOLATION_RUNS && self.failed_runs == self.runs
    }

    /// Clean in isolation: passed every isolated re-run. For a newly-failing
    /// test this is the resource-contention signature — it fails in the
    /// parallel suite and passes alone, so the failure is a property of the
    /// loaded run, not of the patch.
    pub fn passes_in_isolation(&self) -> bool {
        self.runs > 0 && self.failed_runs == 0
    }

    /// Flaky under isolation as well: some runs fail, some pass — or the
    /// observation is too thin to confirm either way.
    pub fn flaky(&self) -> bool {
        !self.confirmed() && !self.passes_in_isolation()
    }

    /// The only observation that may reject the patch (issue #4178,
    /// invariant 4): a newly-failing test whose failure was confirmed by
    /// repeated isolated observation. Unconfirmed failures, and failures of
    /// tests that were not newly failing, never reject.
    pub fn rejects(&self) -> bool {
        self.newly_failing && self.confirmed()
    }

    /// How an unconfirmed differing test should be handled (issue #4178,
    /// invariant 3): identified, then quarantined — never tolerated into
    /// every future comparison.
    pub fn quarantine_mark(&self) -> Option<QuarantineMark> {
        if self.rejects() {
            return None; // charged to the patch, not quarantined
        }
        if self.newly_failing && self.passes_in_isolation() {
            Some(QuarantineMark::Contention)
        } else if !self.passes_in_isolation() {
            Some(QuarantineMark::Flake)
        } else {
            None // stopped failing and clean in isolation: nothing to mark
        }
    }

    /// The HELD line for a newly-failing test (issue #4178, invariant 4):
    /// it records *whether the failure was confirmed*, because the bare
    /// `HELD: new test failures not on main -- <name>` reads as an
    /// established fact, and confirmed and unconfirmed are different
    /// claims. Both recorded forms carry the status parenthetical;
    /// `None` for gone-failing tests: the patch was never charged with
    /// them, so there is no `new test failure` claim to record.
    pub fn held_line(&self) -> Option<String> {
        if !self.newly_failing {
            return None;
        }
        Some(match (self.confirmed(), self.passes_in_isolation()) {
            (true, _) => format!(
                "HELD: new test failure -- {} (confirmed: fails {}/{} in isolation)",
                self.name,
                self.failed_runs,
                self.runs
            ),
            (false, true) => format!(
                "HELD: new test failure -- {} (unconfirmed: passes in isolation, suspect contention)",
                self.name
            ),
            (false, false) => format!(
                "HELD: new test failure -- {} (unconfirmed: fails {}/{} in isolation, suspect contention)",
                self.name,
                self.failed_runs,
                self.runs
            ),
        })
    }
}

/// How an unconfirmed differing test should be handled (issue #4178,
/// invariant 3): it is *identified* — marked with the reason it differs —
/// and quarantined out of the diff, never tolerated into every future
/// comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuarantineMark {
    /// Fails in the parallel suite and passes in isolation: a
    /// resource-contention flake. Serialize its target (`--test-threads=1`)
    /// or fix the test; it stays out of the diff meanwhile.
    Contention,
    /// Fails intermittently, under isolation as well: a flaky test. Fix or
    /// delete it; it stays out of the diff meanwhile.
    Flake,
}

/// What the repeated-observation re-verification concluded (issue #4178):
/// the per-test observations, and whether the patch is charged with any of
/// them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReverificationResolution {
    /// The outcome: a hold only on confirmed failures.
    pub outcome: ReverificationOutcome,
    /// Every reverify test's observation, in reverify-set (sorted) order.
    pub observations: Vec<Reobservation>,
}

impl ReverificationResolution {
    /// The newly-failing tests whose failure was confirmed: the only
    /// failures the patch is charged with.
    pub fn confirmed(&self) -> Vec<&str> {
        self.observations
            .iter()
            .filter(|o| o.rejects())
            .map(|o| o.name.as_str())
            .collect()
    }

    /// The unconfirmed differing tests, marked for quarantine.
    pub fn quarantine(&self) -> Vec<(String, QuarantineMark)> {
        self.observations
            .iter()
            .filter_map(|o| o.quarantine_mark().map(|mark| (o.name.clone(), mark)))
            .collect()
    }

    /// The HELD lines (issue #4178, invariant 4): one per newly-failing
    /// test, each recording whether its failure was confirmed.
    pub fn held_lines(&self) -> Vec<String> {
        self.observations
            .iter()
            .filter_map(|o| o.held_line())
            .collect()
    }
}

/// A reverify test with no isolated re-runs recorded. Attribution without
/// observation is no attribution (issue #4178, invariants 1 and 2): the
/// isolated re-runs are the evidence, and without them the diff stays a
/// trigger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingReobservation {
    /// The differing test that was never re-run in isolation.
    pub test: String,
}

impl fmt::Display for MissingReobservation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}: differing test never re-run in isolation; re-run it with \"{}\" ({} runs) before attributing (issue #4178)",
            self.test,
            isolation_reverify_args(&self.test).join(" "),
            ISOLATION_RUNS
        )
    }
}

/// Resolve a re-verification trigger with the *repeated* isolated
/// observations (issue #4178): `isolation_results` maps every reverify-set
/// test to the outcome of its isolated re-runs — one entry per run, `true`
/// when that run failed, at least [`ISOLATION_RUNS`] runs for a failure to
/// be confirmable.
///
/// This is the contract the single-sample `still_failing` set violated: a
/// test that failed once in isolation may simply be flaky, and charging it
/// on one differing sample held patch #3834 on a test that passes 5/5
/// alone. Only a newly-failing test that fails in *every* one of at least
/// [`ISOLATION_RUNS`] re-runs is confirmed; everything else is reported as
/// unconfirmed — and quarantined — and nothing holds the patch.
///
/// Every reverify-set test must have a non-empty result: a differing test
/// that was never re-run is a [`MissingReobservation`], not a pass. Names
/// outside the reverify set are ignored — a shared baseline failure is not
/// evidence about this patch.
pub fn resolve_reverification(
    diff: &TestFailureDiff,
    isolation_results: &BTreeMap<String, Vec<bool>>,
) -> Result<ReverificationResolution, MissingReobservation> {
    let new_failures: BTreeSet<&str> = diff.new_failures.iter().map(String::as_str).collect();
    let mut observations = Vec::with_capacity(diff.reverify_set().len());
    for name in diff.reverify_set() {
        let runs = isolation_results
            .get(&name)
            .filter(|runs| !runs.is_empty())
            .ok_or_else(|| MissingReobservation { test: name.clone() })?;
        observations.push(Reobservation {
            name: name.clone(),
            newly_failing: new_failures.contains(name.as_str()),
            failed_runs: runs.iter().filter(|failed| **failed).count() as u32,
            runs: runs.len() as u32,
        });
    }
    let confirmed: Vec<String> = observations
        .iter()
        .filter(|o| o.rejects())
        .map(|o| o.name.clone())
        .collect();
    let outcome = if confirmed.is_empty() {
        ReverificationOutcome::Unconfirmed
    } else {
        ReverificationOutcome::Regression { confirmed }
    };
    Ok(ReverificationResolution {
        outcome,
        observations,
    })
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

    /// Isolation results for exactly two tests, in a `BTreeMap`.
    fn two_results(
        first: (&str, Vec<bool>),
        second: (&str, Vec<bool>),
    ) -> BTreeMap<String, Vec<bool>> {
        let mut map = BTreeMap::new();
        map.insert(first.0.to_string(), first.1);
        map.insert(second.0.to_string(), second.1);
        map
    }

    #[test]
    fn reverify_passes_in_isolation_read_as_unconfirmed_churn() {
        let (baseline, patched) = incident_sets();
        let diff = diff_test_failures(&baseline, &patched, &FlakyQuarantine::new());

        // The differing tests pass every isolated re-run: the diff was churn.
        // Nothing holds the patch, and the newly-failing one is identified as
        // a resource-contention flake and quarantined (issue #4178).
        let resolution = resolve_reverification(
            &diff,
            &two_results(
                (NEW, vec![false; ISOLATION_RUNS as usize]),
                (GONE, vec![false; ISOLATION_RUNS as usize]),
            ),
        )
        .expect("both differing tests observed");
        assert_eq!(resolution.outcome, ReverificationOutcome::Unconfirmed);
        assert_eq!(resolution.confirmed(), Vec::<&str>::new());
        assert_eq!(
            resolution.quarantine(),
            vec![(NEW.to_string(), QuarantineMark::Contention)]
        );
        assert_eq!(
            resolution.held_lines(),
            vec![format!(
            "HELD: new test failure -- {NEW} (unconfirmed: passes in isolation, suspect contention)"
        )]
        );
    }

    #[test]
    fn reverify_failures_in_isolation_read_as_confirmed_regression() {
        let (baseline, patched) = incident_sets();
        let diff = diff_test_failures(&baseline, &patched, &FlakyQuarantine::new());

        // The new failure fails every isolated re-run: a regression, now
        // backed by repeated isolated observation (issue #4178).
        let resolution = resolve_reverification(
            &diff,
            &two_results(
                (NEW, vec![true; ISOLATION_RUNS as usize]),
                (GONE, vec![false; ISOLATION_RUNS as usize]),
            ),
        )
        .expect("both differing tests observed");
        assert_eq!(
            resolution.outcome,
            ReverificationOutcome::Regression {
                confirmed: vec![NEW.to_string()],
            }
        );
        assert_eq!(resolution.confirmed(), vec![NEW]);
        assert_eq!(
            resolution.quarantine(),
            Vec::<(String, QuarantineMark)>::new()
        );
        assert_eq!(
            resolution.held_lines(),
            vec![format!(
                "HELD: new test failure -- {NEW} (confirmed: fails 3/3 in isolation)"
            )]
        );
    }

    #[test]
    fn shared_baseline_failures_are_not_evidence_about_the_patch() {
        let (baseline, patched) = incident_sets();
        let diff = diff_test_failures(&baseline, &patched, &FlakyQuarantine::new());

        // Only the reverify set is evidence: the shared failure never enters
        // it, so it needs no observation, and a result for it is ignored.
        let results = BTreeMap::from([
            (SHARED[0].to_string(), vec![true; ISOLATION_RUNS as usize]),
            (NEW.to_string(), vec![false; ISOLATION_RUNS as usize]),
            (GONE.to_string(), vec![false; ISOLATION_RUNS as usize]),
        ]);
        let resolution = resolve_reverification(&diff, &results)
            .expect("reverify tests observed; shared ignored");
        assert_eq!(resolution.outcome, ReverificationOutcome::Unconfirmed);
        assert_eq!(resolution.observations.len(), 2);
        assert!(!resolution.observations.iter().any(|o| o.name == SHARED[0]));
    }

    #[test]
    fn a_single_failing_sample_is_not_confirmation() {
        // Issue #4178, invariant 2: one failing re-run is one more flaky
        // sample, not evidence. `1/1` fails must not charge the patch.
        let (baseline, patched) = incident_sets();
        let diff = diff_test_failures(&baseline, &patched, &FlakyQuarantine::new());

        let resolution = resolve_reverification(
            &diff,
            &two_results(
                (NEW, vec![true]),
                (GONE, vec![false; ISOLATION_RUNS as usize]),
            ),
        )
        .expect("observed");
        assert_eq!(resolution.outcome, ReverificationOutcome::Unconfirmed);
        assert_eq!(resolution.confirmed(), Vec::<&str>::new());
        assert!(!resolution.observations[1].confirmed());
        assert_eq!(
            resolution.quarantine(),
            vec![(NEW.to_string(), QuarantineMark::Flake)]
        );
        assert_eq!(
            resolution.held_lines(),
            vec![format!(
                "HELD: new test failure -- {NEW} (unconfirmed: fails 1/1 in isolation, suspect contention)"
            )]
        );
    }

    #[test]
    fn the_4178_case_passes_in_isolation_is_not_charged() {
        // The observed incident: a ~13s test in a 142-test parallel suite
        // showed up "new" on the patch run, passed 5/5 alone, and the patch
        // was held on its name alone. Re-run in isolation, it passes — and
        // an unconfirmed difference must not reject the patch.
        const FLAKY: &str = "foreground_repeated_restart_observes_one_live_harness_until_merge";
        let baseline: Vec<String> = SHARED.iter().copied().map(String::from).collect();
        let patched: Vec<String> = SHARED
            .iter()
            .copied()
            .chain([FLAKY])
            .map(String::from)
            .collect();
        let diff = diff_test_failures(&baseline, &patched, &FlakyQuarantine::new());

        let resolution = resolve_reverification(
            &diff,
            &BTreeMap::from([(FLAKY.to_string(), vec![false; ISOLATION_RUNS as usize])]),
        )
        .expect("observed");
        assert_eq!(resolution.outcome, ReverificationOutcome::Unconfirmed);
        assert!(!resolution.observations[0].rejects());
        assert_eq!(
            resolution.quarantine(),
            vec![(FLAKY.to_string(), QuarantineMark::Contention)]
        );
    }

    #[test]
    fn a_mixed_isolation_record_is_flaky_not_confirmed() {
        // 2 of 3 isolated runs fail: the test is flaky under isolation as
        // well, so it is quarantined, and the HELD line says unconfirmed.
        let (baseline, patched) = incident_sets();
        let diff = diff_test_failures(&baseline, &patched, &FlakyQuarantine::new());

        let resolution = resolve_reverification(
            &diff,
            &two_results(
                (NEW, vec![true, false, true]),
                (GONE, vec![false; ISOLATION_RUNS as usize]),
            ),
        )
        .expect("observed");
        assert_eq!(resolution.outcome, ReverificationOutcome::Unconfirmed);
        assert_eq!(
            resolution.quarantine(),
            vec![(NEW.to_string(), QuarantineMark::Flake)]
        );
        assert_eq!(
            resolution.held_lines(),
            vec![format!(
                "HELD: new test failure -- {NEW} (unconfirmed: fails 2/3 in isolation, suspect contention)"
            )]
        );
    }

    #[test]
    fn a_gone_failure_failing_in_isolation_is_flaky_not_fixed() {
        // The asymmetry: a test that failed on main and passed in the
        // patched run is not "fixed" if it fails every isolated re-run —
        // the baseline sample was itself a flake. It is identified and
        // quarantined, and the patch is neither charged nor credited.
        let (baseline, patched) = incident_sets();
        let diff = diff_test_failures(&baseline, &patched, &FlakyQuarantine::new());

        let resolution = resolve_reverification(
            &diff,
            &two_results(
                (NEW, vec![false; ISOLATION_RUNS as usize]),
                (GONE, vec![true; ISOLATION_RUNS as usize]),
            ),
        )
        .expect("observed");
        assert_eq!(resolution.outcome, ReverificationOutcome::Unconfirmed);
        assert_eq!(resolution.confirmed(), Vec::<&str>::new());
        // GONE is quarantined as a flake, and the new one as contention; the
        // patch is charged with neither.
        assert_eq!(
            resolution.quarantine(),
            vec![
                (GONE.to_string(), QuarantineMark::Flake),
                (NEW.to_string(), QuarantineMark::Contention),
            ]
        );
        // GONE produced no HELD line: the patch was never charged with it.
        // NEW's unconfirmed line is recorded but rejects nothing.
        assert_eq!(
            resolution.held_lines(),
            vec![format!(
                "HELD: new test failure -- {NEW} (unconfirmed: passes in isolation, suspect contention)"
            )]
        );
    }

    #[test]
    fn a_differing_test_without_isolation_runs_fails_closed() {
        // Issue #4178, invariants 1 and 2: attribution without observation is
        // no attribution. Missing or empty records are an error, not a pass.
        let (baseline, patched) = incident_sets();
        let diff = diff_test_failures(&baseline, &patched, &FlakyQuarantine::new());

        let err = resolve_reverification(&diff, &BTreeMap::new()).expect_err("no observations");
        assert_eq!(
            err,
            MissingReobservation {
                test: GONE.to_string()
            }
        );
        let err = resolve_reverification(
            &diff,
            &two_results((NEW, vec![false; ISOLATION_RUNS as usize]), (GONE, vec![])),
        )
        .expect_err("empty record");
        assert_eq!(
            err,
            MissingReobservation {
                test: GONE.to_string()
            }
        );
        assert!(err.to_string().contains(GONE));
        assert!(err.to_string().contains("--test-threads=1"));
    }

    #[test]
    fn isolation_reverify_args_are_exact_and_single_threaded() {
        // Issue #4178, invariant 1: `--exact` on the full name, one thread.
        assert_eq!(
            isolation_reverify_args("a::b::c"),
            vec![
                "--".to_string(),
                "--exact".to_string(),
                "a::b::c".to_string(),
                "--test-threads=1".to_string()
            ]
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
