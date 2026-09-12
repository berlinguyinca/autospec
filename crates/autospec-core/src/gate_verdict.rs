//! A gate verdict that can say "I did not measure" (issue #4434).
//!
//! # Why this exists
//!
//! The same defect was written four times in one session, each time in fresh
//! tooling, each time *after* filing an invariant against it:
//!
//! | where | what happened |
//! |---|---|
//! | conversion gate | a patch that failed to compile produced an empty failure set; a set-difference read it as "no new failures" and PASSED it |
//! | worker-list parser | an unmatched JSON shape returned 0, reported as "the gateway knows 0 workers" rather than "I could not parse this" |
//! | dependency scan | patterns matched nothing, reported as "zero issues declare dependencies"; 133 did |
//! | gate loop | a `continue` skipped one crate's tests but left the ok flag set, reporting PASS without testing |
//!
//! Every one failed **open**: absence of evidence became evidence of success.
//! Writing the invariant down did not prevent the next occurrence, because the
//! invariant lived in an issue tracker and the code was written from scratch
//! minutes later.
//!
//! So the rule is expressed as a type instead. [`GateVerdict`] has no `bool`
//! reading: [`GateVerdict::is_pass`] is true only for [`GateVerdict::Pass`],
//! and `NotMeasured` is a distinct state that callers must handle. A forgotten
//! case fails closed.

use std::collections::BTreeSet;
use std::fmt;

/// The outcome of a gate, including the case where nothing was measured.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateVerdict {
    /// The gate ran and the subject is acceptable.
    Pass,
    /// The gate ran and the subject is not acceptable.
    Fail { reasons: Vec<String> },
    /// The gate did not produce a usable measurement. This is NOT a pass.
    ///
    /// A build failure, an empty result set, a skipped step, an unparsed
    /// response: all of these are "I do not know", and "I do not know" must
    /// never be reported as "fine".
    NotMeasured { why: String },
}

impl GateVerdict {
    /// True only for [`GateVerdict::Pass`].
    ///
    /// Deliberately not `From<GateVerdict> for bool`: the whole point is that
    /// there is no silent coercion, so a caller cannot accidentally treat
    /// `NotMeasured` as success by writing `if verdict.into()`.
    pub fn is_pass(&self) -> bool {
        matches!(self, GateVerdict::Pass)
    }

    /// True when the gate could not decide. Callers must not proceed as if
    /// the subject were acceptable.
    pub fn is_unmeasured(&self) -> bool {
        matches!(self, GateVerdict::NotMeasured { .. })
    }
}

impl fmt::Display for GateVerdict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GateVerdict::Pass => write!(f, "PASS"),
            GateVerdict::Fail { reasons } => {
                write!(f, "FAIL: {}", reasons.join("; "))
            }
            GateVerdict::NotMeasured { why } => write!(
                f,
                "NOT MEASURED: {why} -- this is not a pass; the gate produced no usable result"
            ),
        }
    }
}

/// One observed test run: what the runner reported.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TestRun {
    /// Process exit status.
    pub exit_code: i32,
    /// How many `test result:` lines the runner emitted. Zero means the suite
    /// did not run -- a compile error, a missing binary, a crashed harness.
    pub result_lines: usize,
    /// Names of failing tests.
    pub failures: BTreeSet<String>,
}

impl TestRun {
    /// Whether this run produced a usable measurement at all.
    pub fn measured(&self) -> bool {
        self.result_lines > 0
    }
}

/// Judges a run against an absolute expectation: everything must pass.
///
/// Use where the subject's suite is known green. Where it is not, use
/// [`differential`], which is the only correct form against a red baseline.
pub fn absolute(run: &TestRun) -> GateVerdict {
    if !run.measured() {
        return GateVerdict::NotMeasured {
            why: format!(
                "the suite emitted no result lines (exit {}), so no test was observed to run",
                run.exit_code
            ),
        };
    }
    if run.failures.is_empty() && run.exit_code == 0 {
        return GateVerdict::Pass;
    }
    let mut reasons: Vec<String> = run.failures.iter().cloned().collect();
    if reasons.is_empty() {
        reasons.push(format!(
            "non-zero exit ({}) with no named failures",
            run.exit_code
        ));
    }
    GateVerdict::Fail { reasons }
}

/// Judges a candidate against a baseline: it must introduce no NEW failures.
///
/// This is the only correct gate against a repository whose mainline is not
/// green. It refuses to compare when either side is unmeasured -- comparing a
/// measurement against a non-measurement is how an empty failure set came to
/// read as an improvement.
pub fn differential(baseline: &TestRun, candidate: &TestRun) -> GateVerdict {
    if !baseline.measured() {
        return GateVerdict::NotMeasured {
            why: "the BASELINE produced no result lines, so there is nothing to compare against"
                .to_string(),
        };
    }
    if !candidate.measured() {
        return GateVerdict::NotMeasured {
            why: format!(
                "the candidate produced no result lines (exit {}) -- an empty failure set is not \
                 an improvement, it means the suite never ran",
                candidate.exit_code
            ),
        };
    }
    // A candidate that runs materially fewer tests than the baseline has not
    // been judged either: it may have compiled a subset, or a harness may have
    // died partway.
    if candidate.result_lines < baseline.result_lines {
        return GateVerdict::NotMeasured {
            why: format!(
                "the candidate emitted {} result line(s) against the baseline's {} -- fewer tests \
                 ran, so 'no new failures' would be measuring less, not improving",
                candidate.result_lines, baseline.result_lines
            ),
        };
    }
    let new: Vec<String> = candidate
        .failures
        .difference(&baseline.failures)
        .cloned()
        .collect();
    if new.is_empty() {
        GateVerdict::Pass
    } else {
        GateVerdict::Fail { reasons: new }
    }
}
