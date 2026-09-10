//! Regrade: a failing grade is re-run once, and the verdict is a function
//! of two runs, not one (issue #4080).
//!
//! Timing-sensitive assertions — 92 in this repository run against 10 ms
//! deadlines — fail under inference load, and a single failing verdict used
//! to reject the patch. The regrade is the fix at the verdict level: the
//! failing tests are re-run exactly once, and the two runs are compared
//! test-by-test.
//!
//! 1. **A test that fails on both runs is a persistent failure** and is the
//!    only kind of failure that blocks ([`RegradeOutcome::blocks`]).
//! 2. **A test that fails on one run only is FLAKY.** It is recorded with
//!    its per-run results ([`FlakyTest`]) and never blocks.
//! 3. **A single failing verdict does not block on its own.** Blocking is a
//!    property of the regrade, not of one run — and it is independent of
//!    the host the grade ran on ([`RegradeOutcome`]).
//! 4. **The verdict is reported with the host conditions it ran under** —
//!    load average and concurrent-agent count ([`HostConditions`]).
//!
//! Everything here is pure — no I/O, no clock, no subprocess. The caller
//! measures the host and the two runs; this module only compares.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

/// The host the grade ran under, reported with the verdict (AC3).
///
/// Recorded at grading time, never a trust condition on its own: a
/// verdict graded under heavy load is exactly as valid as one graded
/// under an idle host. The load figure explains the flaky set; it does
/// not excuse or condemn a failure.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct HostConditions {
    /// The host load average at grading time (1-minute).
    pub load_average: f64,
    /// How many agents were running on the host at grading time.
    pub concurrent_agents: u64,
}

impl HostConditions {
    /// The report line appended to a verdict line.
    pub fn line(&self) -> String {
        format!(
            "host load {:.1}, {} concurrent agent(s)",
            self.load_average, self.concurrent_agents
        )
    }
}

/// One FLAKY test from a regrade: it failed on exactly one of the two
/// runs, so its failure is timing, not the patch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlakyTest {
    /// The test name.
    pub test: String,
    /// Failed on the first (original) run.
    pub failed_in_first: bool,
    /// Failed on the second (re-run) run.
    pub failed_in_second: bool,
}

/// The verdict of a regrade: the comparison of two runs of the failing
/// tests.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegradeOutcome {
    /// Tests that failed on both runs. These — and only these — block.
    pub failures: Vec<String>,
    /// Tests that failed on one run only. Recorded, never blocking.
    pub flaky: Vec<FlakyTest>,
}

impl RegradeOutcome {
    /// Whether the regrade blocks the patch. A single failing verdict does
    /// not block on its own: only failures that persist across the re-run
    /// do (AC4).
    pub fn blocks(&self) -> bool {
        !self.failures.is_empty()
    }

    /// The report line, in the `line()` style of the other verdict
    /// modules.
    pub fn line(&self) -> String {
        if self.failures.is_empty() {
            let flaky = self
                .flaky
                .iter()
                .map(FlakyTest::line)
                .collect::<Vec<_>>()
                .join(", ");
            if flaky.is_empty() {
                "regrade: no failures on either run".to_string()
            } else {
                format!("regrade: no persistent failures; flaky: {flaky}")
            }
        } else {
            format!(
                "regrade: {} persistent failure(s): {}; flaky: {}",
                self.failures.len(),
                self.failures.join(", "),
                self.flaky
                    .iter()
                    .map(FlakyTest::line)
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
    }
}

impl FlakyTest {
    /// The report fragment for one flaky test.
    pub fn line(&self) -> String {
        match (self.failed_in_first, self.failed_in_second) {
            (true, false) => format!("{} (first run only)", self.test),
            (false, true) => format!("{} (second run only)", self.test),
            _ => self.test.clone(),
        }
    }
}

/// Compare two runs of the failing tests (AC1).
///
/// `first_failed` is the set of tests that failed on the original run;
/// `second_failed` the set that failed on the single re-run of those same
/// tests. A test failing on both runs is a persistent failure and blocks;
/// a test failing on one run only is FLAKY and is recorded, never blocking.
/// The result is sorted and deduplicated: both input sets are
/// [`BTreeSet`]s, and the union iterates in sorted order.
pub fn regrade(
    first_failed: &BTreeSet<String>,
    second_failed: &BTreeSet<String>,
) -> RegradeOutcome {
    let mut failures = Vec::new();
    let mut flaky = Vec::new();
    for name in first_failed.union(second_failed) {
        let failed_first = first_failed.contains(name);
        let failed_second = second_failed.contains(name);
        if failed_first && failed_second {
            failures.push(name.clone());
        } else {
            flaky.push(FlakyTest {
                test: name.clone(),
                failed_in_first: failed_first,
                failed_in_second: failed_second,
            });
        }
    }
    RegradeOutcome { failures, flaky }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|name| (*name).to_string()).collect()
    }

    /// AC1: a test that fails on both runs is a persistent failure that
    /// blocks.
    #[test]
    fn a_test_failing_on_both_runs_is_a_persistent_failure_that_blocks() {
        let outcome = regrade(&set(&["a::test", "b::test"]), &set(&["a::test"]));
        assert_eq!(outcome.failures, vec!["a::test".to_string()]);
        assert!(outcome.blocks(), "a persistent failure blocks");
    }

    /// AC1: a test that fails on one run only is FLAKY and does not block.
    #[test]
    fn a_test_failing_on_one_run_only_is_flaky_and_does_not_block() {
        let outcome = regrade(&set(&["a::test"]), &set(&["b::test"]));
        assert!(outcome.failures.is_empty());
        assert!(!outcome.blocks(), "a flaky-only regrade must not block");
        assert_eq!(
            outcome.flaky,
            vec![
                FlakyTest {
                    test: "a::test".to_string(),
                    failed_in_first: true,
                    failed_in_second: false,
                },
                FlakyTest {
                    test: "b::test".to_string(),
                    failed_in_first: false,
                    failed_in_second: true,
                },
            ],
            "the flaky set is sorted and carries per-run results"
        );
    }

    /// AC4: no consumer treats a single failing verdict as rejection —
    /// blocking is a property of the regrade, not of one run.
    #[test]
    fn a_single_failing_verdict_does_not_block_on_its_own() {
        // The original run failed two tests; the re-run failed none.
        let outcome = regrade(&set(&["a::test", "b::test"]), &BTreeSet::new());
        assert!(!outcome.blocks());
        assert_eq!(outcome.flaky.len(), 2);

        // And the reverse: the re-run alone fails a test the original
        // run passed. Still flaky, still non-blocking.
        let outcome = regrade(&BTreeSet::new(), &set(&["a::test"]));
        assert!(!outcome.blocks());
        assert_eq!(outcome.flaky[0].line(), "a::test (second run only)");
    }

    /// The outcome is sorted and deduplicated for the same two runs in any
    /// input order.
    #[test]
    fn regrade_sorts_and_deduplicates() {
        let first = set(&["c::test", "a::test", "b::test"]);
        let second = set(&["b::test", "a::test"]);
        let outcome = regrade(&first, &second);
        assert_eq!(outcome.failures, vec!["a::test", "b::test"]);
        assert_eq!(
            outcome.flaky,
            vec![FlakyTest {
                test: "c::test".to_string(),
                failed_in_first: true,
                failed_in_second: false,
            }]
        );
    }

    /// A regrade with nothing failing on either run reports exactly that.
    #[test]
    fn an_empty_regrade_reports_no_failures() {
        let outcome = regrade(&BTreeSet::new(), &BTreeSet::new());
        assert!(!outcome.blocks());
        assert_eq!(outcome.line(), "regrade: no failures on either run");
    }

    /// The report line names persistent and flaky sets (AC1 + AC3 tone).
    #[test]
    fn the_report_line_names_persistent_and_flaky_sets() {
        let outcome = regrade(&set(&["a::test", "f::test"]), &set(&["a::test", "g::test"]));
        assert_eq!(
            outcome.line(),
            "regrade: 1 persistent failure(s): a::test; flaky: f::test (first run only), g::test (second run only)"
        );
    }

    /// AC3: the host line reports load and concurrent agents.
    #[test]
    fn host_conditions_line_reports_load_and_agents() {
        let host = HostConditions {
            load_average: 4.25,
            concurrent_agents: 3,
        };
        assert_eq!(host.line(), "host load 4.2, 3 concurrent agent(s)");
    }

    /// The outcome round-trips through serde so it can ride along in a
    /// recorded verdict or a report.
    #[test]
    fn regrade_outcome_round_trips_through_serde() {
        let outcome = regrade(&set(&["a::test"]), &set(&["a::test", "b::test"]));
        let text = serde_json::to_string(&outcome).expect("serializable");
        let decoded: RegradeOutcome = serde_json::from_str(&text).expect("deserializable");
        assert_eq!(decoded, outcome);
        assert!(decoded.blocks());
    }

    /// A flaky test carries its per-run results (AC1).
    #[test]
    fn a_flaky_test_carries_its_per_run_results() {
        let outcome = regrade(&set(&["only_second"]), &set(&["only_second"]));
        // Failing on both runs: persistent, not flaky.
        assert_eq!(outcome.failures, vec!["only_second".to_string()]);
        assert!(outcome.flaky.is_empty());
    }
}
