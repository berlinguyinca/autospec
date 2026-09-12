//! A gate set built entirely from "nothing regressed" checks cannot
//! distinguish untested new code from correct new code (issue #4003).
//!
//! An agent produced 291 lines of new routing logic — a routing filter that
//! decides whether inference traffic reaches a worker, a suspension state
//! machine, a background recovery loop — and no test file. It passed the
//! entire gate set: format clean, build ok, vet ok, suite green. Every gate
//! asked *"did anything break?"* and nothing did. The new code was simply
//! never executed; the suite would have been equally green had the filter
//! been written to return the wrong workers.
//!
//! The property is systematic: coverage of a change is inversely related to
//! how new the change is, which is precisely backwards. Total project
//! coverage stays flat while new code goes uncovered, so it cannot be the
//! measurement — the check must report coverage of the *diff*.
//!
//! The second half of the defect is equally mechanical. The missing tests
//! were written before merging and each was verified to fail with the
//! change disabled, then to pass with it restored. Without that mutation
//! step the tests would themselves have been unverified: a test that has
//! never failed is in exactly the position of the code it is supposed to be
//! checking.
//!
//! Four invariants, each a primitive here:
//!
//! 1. **A patch adding executable lines must have those lines covered by a
//!    test.** [`diff_coverage`] reports coverage of the diff, not of the
//!    project, and names every uncovered line.
//! 2. **Patches that add no executable lines are exempt automatically,
//!    without a human deciding.** [`is_executable`] classifies each added
//!    line — blank, comment, and doc/spec text are not executable — and a
//!    patch with none gets [`GateVerdict::Exempt`].
//! 3. **A new test must be shown to fail without the change.** The
//!    implementer records the pre-change run ([`PreChangeRun`]);
//!    [`vacuous_tests`] names the ones that passed there. A test that
//!    passes without the change under review is a defect in the test, not
//!    accepted as a pass.
//! 4. **The enforcement is mechanical.** [`gate`] combines both halves and
//!    refuses — naming the uncovered lines and the vacuous tests — instead
//!    of leaving the rule to a review convention. A rule with no
//!    enforcement is a preference.
//!
//! This module is pure in-memory: the caller parses the patch into
//! [`AddedLine`]s (extracting the enclosing symbol where it can), runs the
//! suite against the patched tree and hands back the executed
//! `(file, line)` set, and runs the new tests against the pre-change tree
//! and hands back the records. The classification and the verdict happen
//! here, so a shell or CLI caller adopts them as the single source of
//! truth.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

/// A line added by the patch. `line_no` is the line number in the new
/// (post-patch) version of the file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddedLine {
    pub file: String,
    pub line_no: usize,
    pub text: String,
    /// The enclosing function or method, when the patch parser knows it.
    /// The refusal names it (`router.go:42 (route_to_worker)`); a line
    /// without a known symbol is named by position only.
    pub symbol: Option<String>,
}

/// File extensions that are never executable: documentation and spec text
/// (invariant 2). The check is mechanical, not a human decision.
pub const DOC_EXTENSIONS: &[&str] = &["md", "markdown", "txt", "rst", "adoc"];

/// True when the file's extension is not a doc/spec extension (invariant 2).
fn is_code_file(file: &str) -> bool {
    match file.rsplit_once('.') {
        Some((_, ext)) => !DOC_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()),
        // No extension: treat as code. Over-classifying is the safe
        // direction — it demands coverage where a test may not exist, it
        // never waives one.
        None => true,
    }
}

/// True when a trimmed line is a comment in the languages this pipeline
/// handles: `//` and block comments (`/*`, `*…`, `*/`), and `#` (shell,
/// YAML, TOML, …).
fn is_comment_line(trimmed: &str) -> bool {
    trimmed.starts_with("//")
        || trimmed.starts_with("/*")
        || trimmed.starts_with("*/")
        || trimmed.starts_with('*')
        || trimmed.starts_with('#')
}

/// True when a trimmed line carries no statement — pure structural
/// punctuation such as `}`, `);` or `},`.
fn is_structure_only(trimmed: &str) -> bool {
    !trimmed.is_empty()
        && trimmed
            .chars()
            .all(|c| matches!(c, '{' | '}' | '(' | ')' | ';' | ','))
}

/// Classify one added line (invariant 2, the mechanical half). Blank
/// lines, comments, structure-only lines, and lines in doc/spec files are
/// not executable; everything else is. Over-flagging (demanding coverage
/// of a line that is not really executable) is the safe direction;
/// under-flagging is how untested code slips through.
pub fn is_executable(line: &AddedLine) -> bool {
    is_code_file(&line.file) && {
        let trimmed = line.text.trim();
        !trimmed.is_empty() && !is_comment_line(trimmed) && !is_structure_only(trimmed)
    }
}

/// Where an uncovered line is: the form the refusal names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineRef {
    pub file: String,
    pub line_no: usize,
    pub symbol: Option<String>,
}

impl LineRef {
    /// `internal/gateway/router.go:42 (route_to_worker)`, or
    /// `internal/gateway/router.go:42` without a known symbol.
    pub fn loc(&self) -> String {
        match &self.symbol {
            Some(symbol) => format!("{}:{} ({symbol})", self.file, self.line_no),
            None => format!("{}:{}", self.file, self.line_no),
        }
    }
}

/// Coverage of the change, not of the repository (invariant 1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffCoverage {
    /// Executable lines the patch added.
    pub executable: usize,
    /// Of those, how many the test run against the patched tree executed.
    pub covered: usize,
    /// The uncovered lines, in patch order, each named.
    pub uncovered: Vec<LineRef>,
}

impl DiffCoverage {
    /// The line a gate record must carry. Names the uncovered lines — never
    /// a bare project-coverage percentage, which stays flat while new code
    /// goes uncovered.
    pub fn line(&self) -> String {
        let base = format!(
            "diff coverage: {}/{} executable lines covered",
            self.covered, self.executable
        );
        if self.uncovered.is_empty() {
            base
        } else {
            format!(
                "{base} — uncovered: {}",
                self.uncovered
                    .iter()
                    .map(LineRef::loc)
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
    }
}

/// Compute coverage of the diff (invariant 1). `executed` is the set of
/// `(file, post-patch line)` pairs the test run against the patched tree
/// executed — the caller runs the suite and hands back the coverage report;
/// this primitive only diffs it against the patch's added lines. Lines the
/// patch did not add are outside the measurement by construction.
pub fn diff_coverage(patch: &[AddedLine], executed: &[(String, usize)]) -> DiffCoverage {
    let executed: BTreeSet<(String, usize)> = executed.iter().cloned().collect();
    let mut coverage = DiffCoverage {
        executable: 0,
        covered: 0,
        uncovered: Vec::new(),
    };
    for line in patch {
        if !is_executable(line) {
            continue;
        }
        coverage.executable += 1;
        if executed.contains(&(line.file.clone(), line.line_no)) {
            coverage.covered += 1;
        } else {
            coverage.uncovered.push(LineRef {
                file: line.file.clone(),
                line_no: line.line_no,
                symbol: line.symbol.clone(),
            });
        }
    }
    coverage
}

/// The implementer's record of running one new test against the pre-change
/// tree (invariant 3: "the implementer runs the new test against the
/// pre-change tree and records that it fails there").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreChangeRun {
    /// The test's name, as the runner reports it.
    pub test: String,
    /// The test failed against the pre-change tree, as the invariant
    /// requires. A recorded `false` is a vacuous test (invariant 3).
    pub failed: bool,
}

/// The new tests whose recorded pre-change run passed (invariant 3). A test
/// that passes without the change under review is a defect in the test, not
/// accepted as a pass — otherwise the pipeline just moves the untested
/// artifact one level up, from untested code to an untested test.
pub fn vacuous_tests(runs: &[PreChangeRun]) -> Vec<String> {
    runs.iter()
        .filter(|run| !run.failed)
        .map(|run| run.test.clone())
        .collect()
}

/// The gate's verdict over the patch (invariant 4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GateVerdict {
    /// No executable lines added and no vacuous test recorded: coverage was
    /// not demanded, and no human decided it (invariant 2).
    Exempt,
    /// Every executable line the patch added was executed by the test run
    /// against the patched tree, and every new test was shown to fail on
    /// the pre-change tree.
    Passed { coverage: DiffCoverage },
    /// Refused: one or more uncovered executable lines (each named) and/or
    /// one or more vacuous tests (each named).
    Refused {
        coverage: DiffCoverage,
        uncovered: Vec<LineRef>,
        vacuous: Vec<String>,
    },
}

impl GateVerdict {
    /// `Exempt` and `Passed` both accept; only `Refused` blocks.
    pub fn passed(&self) -> bool {
        !matches!(self, GateVerdict::Refused { .. })
    }

    /// One line for the gate record and the monitor log. Carries the
    /// diff-coverage line and, on refusal, names every uncovered line and
    /// every vacuous test — never a bare suite total.
    pub fn line(&self) -> String {
        match self {
            GateVerdict::Exempt => "diff coverage: no executable lines added — exempt".into(),
            GateVerdict::Passed { coverage } => format!("{} — gate passed", coverage.line()),
            GateVerdict::Refused {
                coverage,
                uncovered,
                vacuous,
            } => {
                let mut reasons = Vec::new();
                if !uncovered.is_empty() {
                    reasons.push(format!("{} uncovered line(s)", uncovered.len()));
                }
                if !vacuous.is_empty() {
                    reasons.push(format!(
                        "{} vacuous test(s): {}",
                        vacuous.len(),
                        vacuous.join(", ")
                    ));
                }
                format!("{} — gate refused: {}", coverage.line(), reasons.join("; "))
            }
        }
    }
}

/// Run the gate (invariant 4): the standard becomes mechanical here, or it
/// is not enforced at all. Refuses — naming every uncovered executable line
/// and every vacuous test — otherwise.
pub fn gate(
    patch: &[AddedLine],
    executed: &[(String, usize)],
    pre_change: &[PreChangeRun],
) -> GateVerdict {
    let coverage = diff_coverage(patch, executed);
    let vacuous = vacuous_tests(pre_change);
    if coverage.executable == 0 && vacuous.is_empty() {
        GateVerdict::Exempt
    } else if coverage.uncovered.is_empty() && vacuous.is_empty() {
        GateVerdict::Passed { coverage }
    } else {
        let uncovered = coverage.uncovered.clone();
        GateVerdict::Refused {
            coverage,
            uncovered,
            vacuous,
        }
    }
}
