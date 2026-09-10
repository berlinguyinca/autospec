//! Gates over tool output that require positive evidence.
//!
//! The rule enforced here: **a gate must require positive evidence, not the
//! absence of negative evidence.** `0 failed` is not "it passed" — it is
//! "nothing failed", which is also true of a run that never started, a binary
//! that did not build, a filter that matched no tests, and a command that
//! died before emitting output. Every one of those reaches the same string,
//! so a predicate that matches on absence passes all of them.
//!
//! Three gates live here:
//!
//! * [`judge_test_run`] — the pass/fail predicate for `cargo test`-style
//!   output. It sums every `test result:` line (one per target), requires
//!   `passed > 0` as well as `failed == 0`, reports the aggregate with the
//!   target count, and returns the distinct [`TestRunOutcome::NoTestsRan`]
//!   outcome for a run that executed zero tests.
//! * [`judge_completion`] — the same rule for the other gates that grep for
//!   absence (build, clippy, fmt): passing requires positive evidence the
//!   command ran to completion, not merely that no error text was printed.
//! * [`record_gate`] / [`review_gate_claim`] — the mechanical record of a
//!   gate execution (the exit status, the parsed summary, the
//!   failure-marker count, all taken from the full output) and the review
//!   that rejects a reported outcome disagreeing with that record: a gate
//!   reported as passing with a non-zero exit status, or with a failure
//!   count above zero, is a defect, and a claim with no recorded exit
//!   status is rejected rather than trusted (issue #4007).
//!
//! The same doctrine applies to *failing* runs: a count from a run that
//! aborted is a **lower bound**, not a measurement. Cargo's default is
//! fail-fast — the run stops at the first failing target, and every target
//! after it never ran, so `2 failed` can mean "the suite has at least two
//! failures" while looking identical to a completed run's "exactly two". A
//! build failure truncates even earlier. [`TestRunVerdict::completed`] says
//! which kind a run was: the only positive evidence that every target ran is
//! the `--no-fail-fast` summary (`error: N targets failed:`), emitted after
//! the last target. A truncated verdict's [`TestRunVerdict::evidence`]
//! labels the aggregate as a lower bound, and
//! [`failure_count_delta`] refuses to compare two runs when either was
//! truncated, so a baseline captured from an aborted run cannot silently
//! omit the failures that come after the first failing target.

/// Counts summed across every `test result:` line in a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TestRunAggregate {
    pub passed: u64,
    pub failed: u64,
    pub ignored: u64,
    pub measured: u64,
    pub filtered: u64,
    /// How many targets reported a `test result:` line.
    pub targets: usize,
}

impl TestRunAggregate {
    /// The evidence line a report records for this run: the aggregate and
    /// the target count, so a reader can see the run had scope.
    pub fn evidence(&self) -> String {
        format!(
            "{} passed; {} failed across {} {}",
            self.passed,
            self.failed,
            self.targets,
            if self.targets == 1 {
                "target"
            } else {
                "targets"
            }
        )
    }
}

/// Parse every `test result:` line in `output` and sum the counts.
///
/// A target is one line whose trimmed form starts with `test result:`; a
/// workspace run emits one per test binary, and summing all of them is what
/// a `tail -1` over the last line gets wrong when the last target is empty.
pub fn parse_test_run(output: &str) -> TestRunAggregate {
    let mut aggregate = TestRunAggregate::default();
    for line in output.lines() {
        let rest = match line.trim_start().strip_prefix("test result:") {
            Some(rest) => rest,
            None => continue,
        };
        aggregate.targets += 1;
        add_counts(rest, &mut aggregate);
    }
    aggregate
}

fn add_counts(summary: &str, aggregate: &mut TestRunAggregate) {
    for field in summary.split(';') {
        let tokens: Vec<&str> = field.split_whitespace().collect();
        // The first field leads with the status word (`ok.` / `FAILED.`), so
        // the count is paired with the keyword that follows it, wherever the
        // pair sits in the field.
        for pair in tokens.windows(2) {
            let Some(count) = pair[0].parse::<u64>().ok() else {
                continue;
            };
            match pair[1] {
                "passed" => aggregate.passed += count,
                "failed" => aggregate.failed += count,
                "ignored" => aggregate.ignored += count,
                "measured" => aggregate.measured += count,
                "filtered" => aggregate.filtered += count,
                _ => {}
            }
        }
    }
}

/// The outcome of a run judged on positive evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestRunOutcome {
    /// Every target reported zero failures and at least one test ran.
    Passed,
    /// At least one test failed, or the output reports a hard error (a
    /// target that never built has no `test result:` line at all).
    Failed,
    /// Zero tests were executed. A distinct outcome, never folded into
    /// [`Passed`]: its cause is usually a bad scope argument rather than a
    /// clean crate.
    NoTestsRan,
}

impl TestRunOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Passed => "PASSED",
            Self::Failed => "FAILED",
            Self::NoTestsRan => "NO-TESTS-RAN",
        }
    }
}

/// The verdict of [`judge_test_run`]: the outcome plus what was measured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TestRunVerdict {
    pub outcome: TestRunOutcome,
    pub aggregate: TestRunAggregate,
    /// Whether the run is known to have executed every target.
    ///
    /// `false` means the run aborted (fail-fast at the first failing target,
    /// or a build failure) and `aggregate` is a **lower bound** on the
    /// failure count: the targets after the abort never ran. The only
    /// positive evidence of completeness on a failing run is the
    /// `--no-fail-fast` summary (`error: N targets failed:`).
    pub completed: bool,
}

impl TestRunVerdict {
    /// The evidence a completion report must record for this verdict.
    ///
    /// States the aggregate and the target count so a reader can see the run
    /// had scope; the zero-tests outcome carries its distinct label; a
    /// truncated run labels its aggregate as a lower bound, because a count
    /// from an aborted run must not read as a measurement.
    pub fn evidence(&self) -> String {
        let aggregate = self.aggregate.evidence();
        if self.outcome == TestRunOutcome::NoTestsRan {
            format!("NO-TESTS-RAN — {aggregate}")
        } else if self.completed {
            aggregate
        } else {
            format!(
                "{aggregate} (lower bound — run aborted before all targets ran; re-run without fail-fast for the full failure set)"
            )
        }
    }

    /// Whether this verdict may be reported as a green run.
    ///
    /// Only [`TestRunOutcome::Passed`] qualifies: `NoTestsRan` and `Failed`
    /// both refuse, so a zero-scope run cannot be reported as a green one.
    pub fn is_passed(&self) -> bool {
        self.outcome == TestRunOutcome::Passed
    }
}

/// Judge a `cargo test`-style run on positive evidence.
///
/// Requires `failed == 0` across **all** targets as well as `passed > 0`.
/// Reading only the last `test result:` line, or only the absence of
/// failures, is how a run that executed zero tests gets recorded as a pass.
///
/// The verdict also reports whether the run is known to have executed every
/// target ([`TestRunVerdict::completed`]). A failing run is complete only
/// when the `--no-fail-fast` summary is present; anything else is a lower
/// bound, because cargo's fail-fast default stops at the first failing
/// target and a build failure stops it before any later target runs.
pub fn judge_test_run(output: &str) -> TestRunVerdict {
    let aggregate = parse_test_run(output);
    let outcome = if aggregate.failed > 0 || reports_hard_error(output) {
        TestRunOutcome::Failed
    } else if aggregate.passed == 0 {
        TestRunOutcome::NoTestsRan
    } else {
        TestRunOutcome::Passed
    };
    TestRunVerdict {
        outcome,
        aggregate,
        completed: run_completed(&aggregate, output),
    }
}

/// Whether the run is known to have executed every target, i.e. whether the
/// aggregate is a measurement rather than a lower bound.
///
/// Positive evidence only: a run without a hard error and without failures
/// reached its end (fail-fast stops only on failure); a run with a hard
/// error is complete only if the `--no-fail-fast` summary is present, since
/// that summary is printed only after the last target has run. A failing
/// run with no summary is not *known* to have run every target, and
/// unknown scope is treated as truncated: the count is labelled a lower
/// bound rather than allowed to masquerade as a measurement.
fn run_completed(aggregate: &TestRunAggregate, output: &str) -> bool {
    if reports_hard_error(output) {
        reports_all_targets_summary(output)
    } else {
        aggregate.failed == 0
    }
}

/// Whether the output carries the `--no-fail-fast` completion summary:
/// `error: N targets failed:` (or `error: 1 target failed:`), which cargo
/// prints only after every target has run and lists the failing ones.
///
/// The singular `error: test failed, to rerun pass …` line is *not* this
/// marker: it is printed per failing target in both modes, and in fail-fast
/// mode it is the last line of the output. The numbered summary is.
fn reports_all_targets_summary(output: &str) -> bool {
    output.lines().any(|line| {
        let rest = match line.trim_start().strip_prefix("error:") {
            Some(rest) => rest.trim_start(),
            None => return false,
        };
        let Some((count, remainder)) = rest.split_once(' ') else {
            return false;
        };
        count.parse::<u64>().is_ok()
            && matches!(remainder.trim_end(), "target failed:" | "targets failed:")
    })
}

/// Why two test runs cannot be compared: one or both were truncated, so
/// their counts are lower bounds rather than measurements of the same set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IncomparableRuns {
    /// The baseline count is a lower bound, not a measurement.
    pub baseline_truncated: bool,
    /// The current count is a lower bound, not a measurement.
    pub current_truncated: bool,
}

impl std::fmt::Display for IncomparableRuns {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let which = if self.baseline_truncated && self.current_truncated {
            "both runs were truncated"
        } else if self.baseline_truncated {
            "the baseline run was truncated"
        } else {
            "the current run was truncated"
        };
        write!(
            f,
            "a count from a truncated run is a lower bound, not a measurement; {which}"
        )
    }
}

impl std::error::Error for IncomparableRuns {}

/// The difference in failure counts between two runs: `current - baseline`.
///
/// Rejected when either run was truncated ([`IncomparableRuns`]). A count
/// from an aborted run is a lower bound, so the delta between a lower bound
/// and a measurement (or between two lower bounds) measures nothing: fixing
/// the first failure of a truncated baseline reveals the failures that were
/// always there, and reading that as `current - baseline > 0` is how a
/// correct fix presents as a regression.
pub fn failure_count_delta(
    baseline: &TestRunVerdict,
    current: &TestRunVerdict,
) -> Result<i64, IncomparableRuns> {
    let error = IncomparableRuns {
        baseline_truncated: !baseline.completed,
        current_truncated: !current.completed,
    };
    if error.baseline_truncated || error.current_truncated {
        return Err(error);
    }
    // Count values fit in an `i64`; a run that exceeds i64::MAX failures
    // has already defeated every other gate in this module.
    Ok(i64::try_from(current.aggregate.failed).unwrap_or(i64::MAX)
        - i64::try_from(baseline.aggregate.failed).unwrap_or(i64::MAX))
}

/// Whether the output carries a hard error: a line starting with `error:`
/// or `error[` (a compile failure, or `error: test failed, to rerun pass`).
///
/// A target that did not build emits no `test result:` line, so without
/// this check its death is indistinguishable from a run that simply had no
/// tests in scope.
fn reports_hard_error(output: &str) -> bool {
    output.lines().any(|line| {
        let trimmed = line.trim_start();
        trimmed.starts_with("error:") || trimmed.starts_with("error[")
    })
}

/// The evidence a completion gate holds about one command run.
#[derive(Debug, Clone, Copy)]
pub struct CompletionEvidence<'a> {
    /// The process's own exit status, or `None` when the command never ran
    /// (no binary, no shell, a wrapper that swallowed the process).
    pub exit_code: Option<i32>,
    /// Everything the command printed.
    pub output: &'a str,
    /// The marker the tool prints when it ran to completion (`Finished` for
    /// `cargo build` and `cargo clippy`). `None` for tools that are silent
    /// on success — `cargo fmt` — where the exit status is the evidence of a
    /// run.
    pub completion_marker: Option<&'a str>,
}

/// The outcome of a completion gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionOutcome {
    /// The command ran to completion and reported no defect.
    RanClean,
    /// The command ran and reported a defect (non-zero exit status).
    Failed,
    /// No positive evidence the command ran to completion: no exit status,
    /// or the expected completion marker is missing from the output.
    NoRunEvidence,
}

impl CompletionOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::RanClean => "RAN-CLEAN",
            Self::Failed => "FAILED",
            Self::NoRunEvidence => "NO-RUN-EVIDENCE",
        }
    }
}

/// The verdict of [`judge_completion`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompletionVerdict {
    pub outcome: CompletionOutcome,
    pub exit_code: Option<i32>,
}

impl CompletionVerdict {
    /// Whether this verdict may be reported as a green gate.
    pub fn is_passed(&self) -> bool {
        self.outcome == CompletionOutcome::RanClean
    }
}

/// Gate a tool (build, clippy, fmt) on positive evidence that it ran.
///
/// "No errors printed" and "ran successfully" are different claims: a dead
/// wrapper prints no errors either. Passing therefore requires the exit
/// status to be 0 **and**, when the tool has a completion marker, that
/// marker to be present in the output.
pub fn judge_completion(evidence: &CompletionEvidence) -> CompletionVerdict {
    let outcome = match evidence.exit_code {
        None => CompletionOutcome::NoRunEvidence,
        Some(code) if code != 0 => CompletionOutcome::Failed,
        Some(_) => {
            let marker_missing = matches!(
                evidence.completion_marker,
                Some(marker) if !evidence.output.contains(marker)
            );
            if marker_missing {
                CompletionOutcome::NoRunEvidence
            } else {
                CompletionOutcome::RanClean
            }
        }
    };
    CompletionVerdict {
        outcome,
        exit_code: evidence.exit_code,
    }
}

mod gate_record;

pub use gate_record::*;
#[cfg(test)]
mod tests;
