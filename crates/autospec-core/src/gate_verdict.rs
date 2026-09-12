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
//! A later session found the same defect in the *instrument* rather than the
//! predicate (#4457): a gate killed by a timeout produced a log that read
//! exactly like a green run (zero failures, and possibly zero suites), and a
//! `git diff --stat` over the unstaged view missed every new file. So the
//! gate's own completion marker is now part of [`TestRun`] -- a run without it
//! is [`GateVerdict::NotMeasured`], a zero-suite result is a contradiction
//! rather than a pass, and the change size a PR body states is measured from
//! the staged view ([`DiffStat`], [`parse_diff_stat`]).
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
    /// Whether the gate stated the run is complete: its completion marker is
    /// present in the log.
    ///
    /// A gate that was killed before finishing (a timeout killed the process
    /// group, exit 143) stops mid-run and never prints the marker. Its log
    /// then reads exactly like a clean run: zero failures, and -- if the kill
    /// landed before any suite finished -- zero suites too. The marker is the
    /// gate's own statement that the log is whole, so its absence makes every
    /// count in the log unmeasured: never a pass, never a fail (#4457).
    ///
    /// Defaults to `false` so a `TestRun` that is not fully populated fails
    /// closed as unmeasured rather than reading as a pass.
    pub completed: bool,
}

impl TestRun {
    /// Whether this run produced a usable measurement at all: at least one
    /// suite reported a `test result:` line.
    pub fn measured(&self) -> bool {
        self.result_lines > 0
    }

    /// The reason this run is unmeasured because the gate did not state it
    /// completed, or `None` when the completion marker is present.
    ///
    /// A gate result is only interpretable if the gate stated that it
    /// completed. A log without the marker may have been truncated mid-run, so
    /// its counts -- including a zero-failure count -- are a partial
    /// measurement, not a result a reader may convert on.
    fn incomplete_why(&self) -> Option<String> {
        if self.completed {
            return None;
        }
        Some(format!(
            "the gate did not emit its completion marker (exit {}) -- the log is incomplete, so \
             its {} suite(s) and {} failure(s) are a partial measurement, not a pass or a fail",
            self.exit_code,
            self.result_lines,
            self.failures.len()
        ))
    }

    /// The one-line gate summary a report records: the suite count alongside
    /// the failure count.
    ///
    /// The two numbers travel together. A log with zero suites and zero
    /// failures is the shape of a run killed before any suite ran, and reading
    /// only the failure count is how it was mistaken for a clean run (#4457).
    pub fn summary(&self) -> String {
        format!(
            "suites: {}   failed: {}",
            self.result_lines,
            self.failures.len()
        )
    }
}

/// Judges a run against an absolute expectation: everything must pass.
///
/// Use where the subject's suite is known green. Where it is not, use
/// [`differential`], which is the only correct form against a red baseline.
pub fn absolute(run: &TestRun) -> GateVerdict {
    if let Some(why) = run.incomplete_why() {
        return GateVerdict::NotMeasured { why };
    }
    if !run.measured() {
        return GateVerdict::NotMeasured {
            why: format!(
                "the gate completed but reported zero suites run (exit {}): a count of zero \
                 failures from a log with zero suites is a contradiction, not a pass -- no test \
                 was observed to run (a compile error, a missing binary, a crashed harness)",
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
    // A gate result is only interpretable if the gate stated it completed. A
    // log without its completion marker may have been truncated, so neither
    // side's counts can be trusted -- check this before reading any count.
    if let Some(why) = baseline.incomplete_why() {
        return GateVerdict::NotMeasured {
            why: format!("the BASELINE: {why}"),
        };
    }
    if let Some(why) = candidate.incomplete_why() {
        return GateVerdict::NotMeasured {
            why: format!("the candidate: {why}"),
        };
    }
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

/// Change size for review, measured from the **staged** view (#4457).
///
/// `git diff` is blind to untracked files: a patch that adds new files reports
/// a tiny fraction of its real size from the unstaged view. The incident: a
/// patch applied as 1973 insertions showed `2 files changed, 18 insertions(+)`
/// from `git diff --stat` -- eight of the ten files were new, so the unstaged
/// view understated the change by two orders of magnitude. The staged view
/// (`git add -A && git diff --cached --stat`) counts them. A PR body's line
/// counts must come from that same staged view, never the unstaged one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DiffStat {
    /// Files changed.
    pub files: usize,
    /// Lines added.
    pub insertions: usize,
    /// Lines removed.
    pub deletions: usize,
}

impl DiffStat {
    /// The change-size line a PR body states: the file and line counts from
    /// the staged view, named as the staged view, so a reader can tell the
    /// number was measured where new files are counted (#4457).
    pub fn pr_body_line(&self) -> String {
        format!(
            "Reviewed {} file(s): {} insertion(s), {} deletion(s) (staged view).",
            self.files, self.insertions, self.deletions
        )
    }
}

/// Parse the trailing summary line of `git diff --stat` output:
/// `N files changed, M insertions(+), K deletions(-)`.
///
/// `git diff --stat` prints one bar line per file, then a single summary line
/// that is the only line carrying a `changed` total; parse from that. Any of
/// the three counts may be absent (a pure-addition diff has no deletions). The
/// staged and unstaged forms share this shape, so the caller is responsible
/// for feeding the staged view -- this function cannot tell which one it was
/// given, which is why [`verify_change_size`] exists.
///
/// Returns `None` when the output carries no summary line: a diff stat with
/// nothing to report is not a measurement, for the same reason a zero-suite
/// log is not a pass.
pub fn parse_diff_stat(output: &str) -> Option<DiffStat> {
    let line = output.lines().rev().find(|l| l.contains("changed"))?;
    let tokens: Vec<&str> = line.split_whitespace().collect();
    let mut stat = DiffStat::default();
    let mut found = false;
    for pair in tokens.windows(2) {
        let Ok(count) = pair[0].parse::<usize>() else {
            continue;
        };
        match pair[1] {
            "files" | "file" | "files," | "file," => {
                stat.files = count;
                found = true;
            }
            "insertions(+)" | "insertions(+)," => {
                stat.insertions = count;
                found = true;
            }
            "deletions(-)" | "deletions(-)," => {
                stat.deletions = count;
                found = true;
            }
            _ => {}
        }
    }
    if found {
        Some(stat)
    } else {
        None
    }
}

/// The error a review measurement must produce when it understates the change
/// it was asked to measure (#4457).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnderstatedChange {
    /// The change size the patch applied as.
    pub declared_insertions: usize,
    /// The size the diff view measured.
    pub measured_insertions: usize,
}

impl std::fmt::Display for UnderstatedChange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "the diff measured {} insertion(s) but the patch applied as {} -- the view missed \
             new (untracked) files; measure the staged view (stage all changes, then read the \
             staged diff)",
            self.measured_insertions, self.declared_insertions
        )
    }
}

impl std::error::Error for UnderstatedChange {}

/// Check that a measured change size accounts for the change as applied.
///
/// `declared_insertions` is the size `git apply` reported when the patch was
/// applied; `measured` is the size a `git diff --stat` view produced. They
/// must agree: a measurement that falls short of the applied size missed
/// files (the unstaged view is blind to untracked ones), and a PR body built
/// from it understates the change by orders of magnitude (#4457).
pub fn verify_change_size(
    declared_insertions: usize,
    measured: &DiffStat,
) -> Result<(), UnderstatedChange> {
    if measured.insertions < declared_insertions {
        return Err(UnderstatedChange {
            declared_insertions,
            measured_insertions: measured.insertions,
        });
    }
    Ok(())
}
