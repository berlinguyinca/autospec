//! Gates over tool output that require positive evidence.
//!
//! The rule enforced here: **a gate must require positive evidence, not the
//! absence of negative evidence.** `0 failed` is not "it passed" — it is
//! "nothing failed", which is also true of a run that never started, a binary
//! that did not build, a filter that matched no tests, and a command that
//! died before emitting output. Every one of those reaches the same string,
//! so a predicate that matches on absence passes all of them.
//!
//! Two gates live here:
//!
//! * [`judge_test_run`] — the pass/fail predicate for `cargo test`-style
//!   output. It sums every `test result:` line (one per target), requires
//!   `passed > 0` as well as `failed == 0`, reports the aggregate with the
//!   target count, and returns the distinct [`TestRunOutcome::NoTestsRan`]
//!   outcome for a run that executed zero tests.
//! * [`judge_completion`] — the same rule for the other gates that grep for
//!   absence (build, clippy, fmt): passing requires positive evidence the
//!   command ran to completion, not merely that no error text was printed.

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
}

impl TestRunVerdict {
    /// The evidence a completion report must record for this verdict.
    ///
    /// States the aggregate and the target count so a reader can see the run
    /// had scope; the zero-tests outcome carries its distinct label.
    pub fn evidence(&self) -> String {
        let aggregate = self.aggregate.evidence();
        if self.outcome == TestRunOutcome::NoTestsRan {
            format!("NO-TESTS-RAN — {aggregate}")
        } else {
            aggregate
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
pub fn judge_test_run(output: &str) -> TestRunVerdict {
    let aggregate = parse_test_run(output);
    let outcome = if aggregate.failed > 0 || reports_hard_error(output) {
        TestRunOutcome::Failed
    } else if aggregate.passed == 0 {
        TestRunOutcome::NoTestsRan
    } else {
        TestRunOutcome::Passed
    };
    TestRunVerdict { outcome, aggregate }
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

#[cfg(test)]
mod tests;
