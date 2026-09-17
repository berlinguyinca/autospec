//! How much of a suite a recorded run actually executed (issue #4665).
//!
//! The incident: a run came back `VERIFIED` with its own counters beside it:
//!
//! ```text
//! status=VERIFIED test_passed=1120 test_failed=30
//! new_failing_tests=0 fixed_baseline_failures=120
//! ```
//!
//! 1120 + 30 = **1150 tests of a 10 073-test suite**: the run covered 11 % and
//! stopped, because `cargo test` aborts after the first failing test *binary*
//! and the runner had no `--no-fail-fast`. Two numbers were wrong in opposite
//! directions from the same cause. `new_failing_tests=0` did not mean the patch
//! broke nothing; it meant nothing broke in the 11 % that ran, and a regression
//! in any binary scheduled after the abort was invisible while the issue was
//! queued for merge. `fixed_baseline_failures=120` was worse because it was
//! confidently wrong: the baseline holds 149 known failures, a test that never
//! ran cannot fail, so absence from the failure list was scored as a repair. A
//! patch to one module does not fix 120 unrelated tests; that number counts
//! tests the run never reached.
//!
//! The invariants this module holds:
//!
//! 1. **A pass needs a complete run; a failure does not**
//!    ([`entitled`]). A run that executed a prefix of the suite can still
//!    *prove* a failure — a test that failed, failed — but it cannot prove
//!    every test it never ran would have passed. So only a claim of passing is
//!    downgraded, and the downgrade is not a failure verdict: it is the honest
//!    absence of one.
//! 2. **Absence from a failure list is not a repair** ([`tally_fixes`]).
//!    "Fixed a baseline failure" means the test ran and passed. Entries the run
//!    did not observe are reported as unobserved, never counted as fixed.
//! 3. **Coverage belongs in the verdict, not in the reader's arithmetic**
//!    ([`Coverage::line`]). `status=VERIFIED` and `tests_run=1150 of 10073`
//!    must not be able to appear together without the first being downgraded,
//!    and nobody should have to divide two numbers to notice.
//! 4. **A record with no declared suite size keeps its verdict**
//!    ([`Coverage::Unrecorded`]). Every record written before the total was
//!    recorded would otherwise become ungradeable, which is a different bug and
//!    a worse one: it converts an unknown into a claim. The gap is reported on
//!    the line so it accumulates visibly rather than silently.
//!
//! Pure: it reads counters a record already carries and runs no suite.

use crate::run_status::Status;
use std::collections::BTreeSet;

/// How much of the suite a run's own counters account for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Coverage {
    /// The record declares no suite size. Not [`Coverage::Partial`]: the run
    /// may have covered everything, and inventing a shortfall would replace a
    /// missing fact with a false one (invariant 4).
    Unrecorded,
    /// The counters account for the whole declared suite.
    Complete {
        /// tests the run accounted for.
        ran: u64,
    },
    /// The run accounted for fewer tests than the suite holds.
    Partial {
        /// tests the run accounted for.
        ran: u64,
        /// the suite's declared size.
        total: u64,
    },
}

impl Coverage {
    /// Derive coverage from the counters a run records: `test_passed`,
    /// `test_failed`, and the suite size it was measured against.
    ///
    /// A missing counter counts as zero executed, but a missing *total* makes
    /// the question unanswerable rather than answered badly — the two are
    /// deliberately not collapsed.
    pub fn from_counts(passed: Option<u64>, failed: Option<u64>, total: Option<u64>) -> Self {
        let ran = passed.unwrap_or(0).saturating_add(failed.unwrap_or(0));
        match total {
            // A zero-sized suite is not a suite: it is a capture that measured
            // nothing, and claiming `Complete { ran: 0 }` over it would let an
            // empty run certify a patch.
            Some(0) => Self::Unrecorded,
            Some(total) if ran >= total => Self::Complete { ran },
            Some(total) => Self::Partial { ran, total },
            None => Self::Unrecorded,
        }
    }

    /// Whether the run fell short of the suite.
    pub fn is_partial(self) -> bool {
        matches!(self, Self::Partial { .. })
    }

    /// The executed test count, when the record supports one.
    pub fn ran(self) -> Option<u64> {
        match self {
            Self::Unrecorded => None,
            Self::Complete { ran } => Some(ran),
            Self::Partial { ran, .. } => Some(ran),
        }
    }

    /// The coverage phrase, in the shape the status file already uses.
    ///
    /// Integer percent: the repository's architecture gate forbids `f64` in
    /// `crates/**`, and a two-digit percentage needs no floating point.
    pub fn line(self) -> String {
        match self {
            Self::Unrecorded => "coverage unrecorded (suite size not declared)".to_string(),
            Self::Complete { ran } => format!("tests_run={ran} of {ran}"),
            Self::Partial { ran, total } => {
                let percent = ran.saturating_mul(100) / total.max(1);
                format!("tests_run={ran} of {total} ({percent}%)")
            }
        }
    }
}

/// Whether a recorded label claims the patch passes.
///
/// Only `VERIFIED` does. `NEW-TEST-FAILURES` reports an observed failure, which
/// a short run can establish perfectly well; `UNKNOWN-NO-BASELINE` and the
/// timeout statuses already disclaim a verdict, so downgrading them would say
/// nothing the record has not already said.
fn claims_a_pass(label: Option<Status>) -> bool {
    label == Some(Status::Verified)
}

/// The status a record is entitled to claim, given what it executed (#4665).
///
/// This is the same judgement [`crate::execution::verification`] makes about a
/// `VERIFIED` enumeration: the record states counters, and the counters
/// determine what the label may say. A `VERIFIED` written over 11 % of the suite
/// is rewritten to [`Status::PartialCoverage`] — which is *not* a failing status
/// and must not route the patch to the rejected path, because the run did not
/// show the patch is broken; it showed the run cannot say either way.
pub fn entitled(label: Option<Status>, coverage: Coverage) -> Option<Status> {
    if claims_a_pass(label) && coverage.is_partial() {
        return Some(Status::PartialCoverage);
    }
    label
}

/// What a run can honestly say about the failures its baseline listed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixTally {
    /// baseline entries the run observed passing.
    pub fixed: Vec<String>,
    /// baseline entries the run observed failing again.
    pub still_failing: Vec<String>,
    /// baseline entries the run never observed, in either direction.
    pub unobserved: Vec<String>,
}

impl FixTally {
    /// The count the record may publish as `fixed_baseline_failures`.
    pub fn fixed_count(&self) -> usize {
        self.fixed.len()
    }

    /// The line that reports the tally without laundering the unobserved ones
    /// into fixes (invariant 2).
    pub fn line(&self) -> String {
        let fixed = self.fixed_count();
        let still_failing = self.still_failing.len();
        match self.unobserved.is_empty() {
            true => format!("fixed_baseline_failures={fixed} still_failing={still_failing}"),
            false => format!(
                "fixed_baseline_failures={fixed} still_failing={still_failing} unobserved={} (not scored: a test that never ran cannot be fixed)",
                self.unobserved.len()
            ),
        }
    }
}

/// Score a baseline's known failures against what this run actually observed.
///
/// The rule is that absence is not evidence of repair. The incident scored 120
/// baseline failures as fixed by a run that covered 11 % of the suite, because
/// the implementation asked "is this entry in the failure list?" and a test the
/// runner never reached is absent from every list. So membership is tested
/// against *both* observed sets, and an entry in neither is reported as
/// unobserved — a number the reader can act on — rather than being silently
/// added to the fixed column.
pub fn tally_fixes(baseline: &[&str], passed: &[&str], failed: &[&str]) -> FixTally {
    let passed: BTreeSet<&str> = passed.iter().copied().collect();
    let failed: BTreeSet<&str> = failed.iter().copied().collect();
    let mut tally = FixTally {
        fixed: Vec::new(),
        still_failing: Vec::new(),
        unobserved: Vec::new(),
    };
    for entry in baseline {
        if passed.contains(entry) {
            tally.fixed.push((*entry).to_string());
        } else if failed.contains(entry) {
            tally.still_failing.push((*entry).to_string());
        } else {
            tally.unobserved.push((*entry).to_string());
        }
    }
    tally
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_incident_run_is_partial_not_verified() {
        // 1120 + 30 of a 10 073-test suite: 11 %, and the run stopped there.
        let coverage = Coverage::from_counts(Some(1120), Some(30), Some(10073));
        assert_eq!(
            coverage,
            Coverage::Partial {
                ran: 1150,
                total: 10073
            }
        );
        assert!(coverage.is_partial());
        let line = coverage.line();
        assert!(line.contains("1150 of 10073"), "{line}");
        assert!(line.contains("11%"), "{line}");
        assert_eq!(
            entitled(Some(Status::Verified), coverage),
            Some(Status::PartialCoverage)
        );
    }

    #[test]
    fn a_complete_run_keeps_its_verdict() {
        let coverage = Coverage::from_counts(Some(10073), Some(0), Some(10073));
        assert_eq!(coverage, Coverage::Complete { ran: 10073 });
        assert!(!coverage.is_partial());
        assert_eq!(
            entitled(Some(Status::Verified), coverage),
            Some(Status::Verified)
        );
    }

    #[test]
    fn a_missing_suite_size_is_not_a_shortfall() {
        // Invariant 4: every pre-existing record lacks the total. Reading it as
        // partial would make every historical VERIFIED ungradeable — replacing
        // a missing fact with a false one.
        let coverage = Coverage::from_counts(Some(1120), Some(30), None);
        assert_eq!(coverage, Coverage::Unrecorded);
        assert!(!coverage.is_partial());
        assert_eq!(
            entitled(Some(Status::Verified), coverage),
            Some(Status::Verified)
        );
        assert!(coverage.line().contains("unrecorded"));
        // A zero total is a capture that measured nothing, not a suite.
        assert_eq!(
            Coverage::from_counts(Some(0), Some(0), Some(0)),
            Coverage::Unrecorded
        );
    }

    #[test]
    fn a_short_run_can_still_prove_a_failure() {
        // Only a claim of passing is downgraded: a test that failed, failed,
        // however much of the suite the run skipped.
        let coverage = Coverage::Partial {
            ran: 1150,
            total: 10073,
        };
        assert_eq!(
            entitled(Some(Status::NewTestFailures), coverage),
            Some(Status::NewTestFailures)
        );
        assert_eq!(
            entitled(Some(Status::UnknownNoBaseline), coverage),
            Some(Status::UnknownNoBaseline)
        );
        assert_eq!(entitled(None, coverage), None);
    }

    #[test]
    fn an_absent_test_is_not_a_fixed_test() {
        // The 120: entries the run never reached, scored as repairs because the
        // implementation asked only "did this fail?".
        let baseline = ["a::t1", "a::t2", "a::t3", "b::slow_test"];
        let passed = ["a::t1"];
        let failed = ["a::t2"];
        let tally = tally_fixes(&baseline, &passed, &failed);
        assert_eq!(tally.fixed, vec!["a::t1".to_string()]);
        assert_eq!(tally.still_failing, vec!["a::t2".to_string()]);
        assert_eq!(
            tally.unobserved,
            vec!["a::t3".to_string(), "b::slow_test".to_string()]
        );
        // One honest fix, not three.
        assert_eq!(tally.fixed_count(), 1);
        let line = tally.line();
        assert!(line.contains("fixed_baseline_failures=1"), "{line}");
        assert!(line.contains("unobserved=2"), "{line}");
    }

    #[test]
    fn a_full_observation_reports_no_unobserved_column() {
        let tally = tally_fixes(&["a::t1", "a::t2"], &["a::t1"], &["a::t2"]);
        assert!(tally.unobserved.is_empty());
        assert!(!tally.line().contains("unobserved"), "{}", tally.line());
    }

    #[test]
    fn partial_coverage_is_a_routed_name_in_the_vocabulary() {
        // An undeclared name resolves to None, and `None` reads as green.
        use crate::run_status::canonical_status;
        assert_eq!(
            canonical_status("PARTIAL-COVERAGE"),
            Some(Status::PartialCoverage)
        );
        assert_eq!(Status::PartialCoverage.as_str(), "PARTIAL-COVERAGE");
    }
}
