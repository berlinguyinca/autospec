//! Gate results are captured, not narrated (#4007).
//!
//! The failure this module exists to close: one PR body put two gate figures
//! in one paragraph. `2901 passed, 0 failed` for `cargo test` was counted
//! from that run's `test result:` lines. `69/69 pass` for a bats suite was
//! read off a tail that went `ok 67`, `ok 68`, `ok 69` — and was wrong: the
//! suite ran 69 cases, 63 passed, 6 failed. bats prints each failure inline
//! and carries on, so the last line of a run names the last case executed,
//! never the verdict. Exit status was captured for neither run, so both
//! figures sat in the same format with the same apparent confidence and
//! nothing downstream could tell the measurement from the guess.
//!
//! Output order carries no information about the verdict, and a model reads
//! the end. Three primitives close the gap; all of them are pure — no I/O,
//! no subprocess, no clock. The caller runs the command and hands the output
//! here; the verdict is computed from the record.
//!
//! 1. **Count something** ([`capture`], [`GateRecord`]). The record is the
//!    exit status plus the runner's own counts ([`TestSummary`]) plus the
//!    count of its failure markers ([`GateDefinition::failure_marker`]). The
//!    counts are parsed from the whole output, so a run that fails 6 tests
//!    and then passes 490 reports 6 failures whoever reads it.
//! 2. **The runner's shape is declared, not improvised** ([`GateDefinition`]).
//!    Where the verdict lives (`test result:` lines, TAP result lines, or no
//!    summary at all) and what names a failing case are properties of the
//!    gate, fixed when the gate is defined. A run cannot pick its own marker,
//!    which is how "green" gets read off a tail.
//! 3. **The verdict is generated from the record** ([`GateRecord::claim`],
//!    [`render_gate_section`]) **and re-checked at review** ([`review`]).
//!    A passing claim with a non-zero exit status, or with a failure count
//!    above zero, is rejected; so is a passing claim whose exit status was
//!    never recorded — an unmeasured gate is refused, never trusted.
//!
//! The report carries the raw evidence ([`GateRecord::evidence`]) so a reader
//! can tell a measurement from an estimate without re-running anything.
//!
//! # What this module does not do
//!
//! It does not run commands, and it does not judge a *reported-failure*
//! claim against a clean record. Understating success fails closed: the work
//! is held rather than released, which is the safe direction and costs a
//! re-run instead of a bad merge.

use std::collections::BTreeMap;

/// Where a runner states its own pass/fail counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SummaryFormat {
    /// `cargo test` and friends: one
    /// `test result: ok. 1871 passed; 0 failed; …` line per test binary.
    /// The counts are summed across every line, because one `cargo test`
    /// invocation runs many binaries and only the sum describes the run.
    CargoTest,
    /// TAP (`bats`, `prove`): no summary line at all — every test is one
    /// `ok N` / `not ok N` result line, so the counts are those line counts.
    Tap,
    /// The runner prints no per-test verdict: the gate's failure marker is
    /// the only count available, and it is the one that is used.
    MarkersOnly,
}

impl SummaryFormat {
    /// The stable name used in records and reports.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CargoTest => "cargo-test",
            Self::Tap => "tap",
            Self::MarkersOnly => "markers-only",
        }
    }

    /// The failure marker this runner uses to name a failing case:
    /// `FAILED` for `cargo test`, `not ok` for TAP.
    ///
    /// `MarkersOnly` has no default — a runner with no summary needs the
    /// gate author to say what failure looks like ([`GateDefinition::markers_only`]).
    pub fn default_failure_marker(self) -> Option<&'static str> {
        match self {
            Self::CargoTest => Some("FAILED"),
            Self::Tap => Some("not ok"),
            Self::MarkersOnly => None,
        }
    }

    /// Whether `marker` on `line` names a failing case for this runner.
    ///
    /// TAP markers are anchored to the start of the (trimmed) line, because
    /// `ok 12 - parses the not ok directive` passes and must not be counted
    /// as a failure. Other runners state failure inside or at the end of the
    /// line, so their marker is matched anywhere in it.
    fn marker_matches(self, line: &str, marker: &str) -> bool {
        let trimmed = line.trim();
        match self {
            Self::Tap => starts_as_word(trimmed, marker),
            Self::CargoTest | Self::MarkersOnly => trimmed.contains(marker),
        }
    }

    /// The runner's own counts, read from the whole output.
    ///
    /// `None` when the output states no counts at all ([`SummaryFormat::MarkersOnly`],
    /// or a run whose output never reached its summary). Callers fall back to
    /// the failure-marker count rather than to the tail ([`GateRecord::failed`]).
    pub fn parse(self, output: &str) -> Option<TestSummary> {
        match self {
            Self::MarkersOnly => None,
            Self::CargoTest => parse_cargo_test(output),
            Self::Tap => parse_tap(output),
        }
    }

    /// Whether `line` is part of the evidence for this runner's counts.
    fn states_counts(self, line: &str) -> bool {
        match self {
            Self::CargoTest => line.contains("test result:"),
            Self::Tap => matches!(tap_line_kind(line), Some(TapLine::Ok | TapLine::NotOk)),
            Self::MarkersOnly => false,
        }
    }
}

/// A runner's own pass/fail counts, summed over its whole output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TestSummary {
    /// Tests the runner reported as passing.
    pub passed: u64,
    /// Tests the runner reported as failing.
    pub failed: u64,
}

impl TestSummary {
    /// The counts the runner stated.
    pub fn new(passed: u64, failed: u64) -> Self {
        Self { passed, failed }
    }

    /// The runner's summary line, in the shape a report carries.
    pub fn render(self) -> String {
        format!("{} passed, {} failed", self.passed, self.failed)
    }
}

/// One gate: what it runs, where its verdict is stated, and what names a
/// failing case.
///
/// These are properties of the gate, fixed when the gate is defined, so no
/// run gets to choose them. A marker chosen per run is the mechanism by
/// which a green tail becomes a green verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateDefinition {
    /// Stable gate name, e.g. `rust-tests`, `bats-suite`.
    pub name: String,
    /// The exact command the gate runs, carried into the record so the
    /// evidence names what was measured (#3915).
    pub command: String,
    /// Where the runner states its own counts.
    pub summary: SummaryFormat,
    /// The pattern naming a failing case in this runner's output.
    pub failure_marker: String,
}

impl GateDefinition {
    /// A gate whose runner states its counts. The failure marker is the
    /// runner's own ([`SummaryFormat::default_failure_marker`]).
    ///
    /// Rejected: an empty name or command, and [`SummaryFormat::MarkersOnly`]
    /// — a runner with no summary must declare its marker through
    /// [`GateDefinition::markers_only`], because there is no default to fall
    /// back to and an empty marker would count zero failures for every run.
    pub fn new(
        name: impl Into<String>,
        command: impl Into<String>,
        summary: SummaryFormat,
    ) -> Result<Self, String> {
        let marker = summary.default_failure_marker().ok_or_else(|| {
            format!(
                "gate {:?} prints no summary line: declare its failure marker with \
                 GateDefinition::markers_only — a gate with no marker counts nothing",
                summary.as_str()
            )
        })?;
        Self::build(name, command, summary, marker)
    }

    /// A gate whose runner states no counts: `failure_marker` naming a
    /// failing case is the only measurement the record can carry.
    pub fn markers_only(
        name: impl Into<String>,
        command: impl Into<String>,
        failure_marker: impl Into<String>,
    ) -> Result<Self, String> {
        Self::build(name, command, SummaryFormat::MarkersOnly, failure_marker)
    }

    /// The workspace Rust suite, the gate every patch in this repository is
    /// landed by.
    pub fn cargo_test(name: impl Into<String>, command: impl Into<String>) -> Self {
        Self::new(name, command, SummaryFormat::CargoTest)
            .expect("cargo-test has a default failure marker and the fields are validated")
    }

    /// A TAP runner (`bats`, `prove`).
    pub fn tap(name: impl Into<String>, command: impl Into<String>) -> Self {
        Self::new(name, command, SummaryFormat::Tap)
            .expect("tap has a default failure marker and the fields are validated")
    }

    fn build(
        name: impl Into<String>,
        command: impl Into<String>,
        summary: SummaryFormat,
        failure_marker: impl Into<String>,
    ) -> Result<Self, String> {
        let name = name.into();
        let command = command.into();
        let failure_marker = failure_marker.into();
        if name.trim().is_empty() {
            return Err(
                "a gate has an empty name: every gate must be nameable so a report can say \
                 which result it is describing"
                    .to_string(),
            );
        }
        if command.trim().is_empty() {
            return Err(format!(
                "gate {name} has an empty command: a gate nobody can run has no result to \
                 capture"
            ));
        }
        if failure_marker.trim().is_empty() {
            return Err(format!(
                "gate {name} has an empty failure marker: an empty marker matches every \
                 line, which counts the whole run as failing or nothing at all"
            ));
        }
        Ok(Self {
            name,
            command,
            summary,
            failure_marker,
        })
    }
}

/// The result of one gate run, as captured.
///
/// Every field is a measurement: the exit status the process returned, the
/// counts the runner itself printed, the number of failure markers in the
/// output, and the raw lines those numbers were read from. Nothing here is a
/// reading of the output's shape or order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateRecord {
    /// The gate that ran, by its definition name.
    pub gate: String,
    /// The command that ran.
    pub command: String,
    /// The exit status the process returned. Recorded, never inferred:
    /// a run whose exit status was not captured is not a green run.
    pub exit_status: i32,
    /// The runner's own counts, when it stated any.
    pub summary: Option<TestSummary>,
    /// How many failure markers appear in the whole output.
    pub failure_markers: u64,
    /// The raw output lines the counts came from, in output order. This is
    /// what lets a reader tell a measurement from an estimate.
    pub evidence: Vec<String>,
}

impl GateRecord {
    /// The failures this run reports: the runner's own count when it stated
    /// one, otherwise the failure-marker count.
    ///
    /// Never the tail, and never `0` by default: a runner with no summary and
    /// no marker in its output reported zero failures because nobody counted.
    pub fn failed(&self) -> u64 {
        self.summary.map_or(self.failure_markers, |s| s.failed)
    }

    /// The passes this run reports, when the runner stated counts.
    pub fn passed(&self) -> Option<u64> {
        self.summary.map(|s| s.passed)
    }

    /// The mechanical verdict: exit status 0 and no failures.
    ///
    /// Both halves are required. `cargo test --workspace` exits 101 when a
    /// later binary fails after earlier ones reported `ok`; `bats` prints
    /// `ok N` for every test that passed, in order, so a run that ends green
    /// is green on the tail and red everywhere else.
    pub fn is_green(&self) -> bool {
        self.exit_status == 0 && self.failed() == 0
    }

    /// What this record entitles a report to claim about the gate.
    ///
    /// Generated from the record, so a report assembled from these claims
    /// cannot say "passed" about a run the record does not support (AC1).
    pub fn claim(&self) -> GateClaim {
        GateClaim {
            gate: self.gate.clone(),
            reported: if self.is_green() {
                Reported::Pass
            } else {
                Reported::Fail
            },
            exit_status: Some(self.exit_status),
            passed: self.passed(),
            failed: Some(self.failed()),
        }
    }

    /// The evidence line a report carries for this run: verdict, exit
    /// status, counts, marker count, and the raw lines behind them.
    pub fn evidence_line(&self) -> String {
        let verdict = if self.is_green() { "GREEN" } else { "RED" };
        let counts = match self.summary {
            Some(summary) => format!(
                "summary {} ({} failure markers counted)",
                summary.render(),
                self.failure_markers
            ),
            None => format!(
                "no summary line ({} failure markers counted)",
                self.failure_markers
            ),
        };
        format!(
            "`{}`: {verdict} — exit {}, {}",
            self.command, self.exit_status, counts
        )
    }
}

/// Captures one gate run: the exit status the process returned and the
/// output it produced.
///
/// The failure marker comes from the gate definition, never from the call
/// site, and the counts are read from the whole output rather than from any
/// window on it.
pub fn capture(definition: &GateDefinition, exit_status: i32, output: &str) -> GateRecord {
    let summary = definition.summary.parse(output);
    let mut failure_markers = 0u64;
    let mut evidence = Vec::new();
    for line in output.lines() {
        let is_failure = definition
            .summary
            .marker_matches(line, &definition.failure_marker);
        if is_failure {
            failure_markers += 1;
        }
        if is_failure || definition.summary.states_counts(line) {
            evidence.push(line.to_string());
        }
    }
    GateRecord {
        gate: definition.name.clone(),
        command: definition.command.clone(),
        exit_status,
        summary,
        failure_markers,
        evidence,
    }
}

/// What a report says about one gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reported {
    /// The gate is reported as passing.
    Pass,
    /// The gate is reported as failing.
    Fail,
}

impl Reported {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Fail => "fail",
        }
    }
}

/// A claim about one gate: what the report says, and what it recorded.
///
/// The recorded fields are `Option` because a claim written from prose may
/// record nothing. A passing claim with nothing recorded is the case review
/// exists for ([`review`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateClaim {
    /// The gate the claim is about, matched to a record by name.
    pub gate: String,
    /// The verdict the report states.
    pub reported: Reported,
    /// The exit status the report recorded, if any.
    pub exit_status: Option<i32>,
    /// The pass count the report recorded, if any.
    pub passed: Option<u64>,
    /// The failure count the report recorded, if any.
    pub failed: Option<u64>,
}

impl GateClaim {
    /// A claim that a gate passed, with nothing recorded. Rejected on sight
    /// by [`review`]: a gate nobody measured is not a gate that passed.
    pub fn asserted_pass(gate: impl Into<String>) -> Self {
        Self {
            gate: gate.into(),
            reported: Reported::Pass,
            exit_status: None,
            passed: None,
            failed: None,
        }
    }

    /// A claim that a gate failed, with nothing recorded.
    pub fn asserted_fail(gate: impl Into<String>) -> Self {
        Self {
            gate: gate.into(),
            reported: Reported::Fail,
            exit_status: None,
            passed: None,
            failed: None,
        }
    }

    /// Records the exit status the report has.
    pub fn with_exit_status(mut self, exit_status: i32) -> Self {
        self.exit_status = Some(exit_status);
        self
    }

    /// Records the counts the report has.
    pub fn with_counts(mut self, passed: u64, failed: u64) -> Self {
        self.passed = Some(passed);
        self.failed = Some(failed);
        self
    }

    /// Records only a failure count (a marker-only runner).
    pub fn with_failed(mut self, failed: u64) -> Self {
        self.failed = Some(failed);
        self
    }
}

/// The review rules, with stable ids (report-facing; do not renumber).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CaptureRule {
    /// GC-001: a gate is reported as passing with no exit status recorded.
    /// Refused, not trusted — the #4007 run had no exit status at all.
    ExitStatusUnrecorded,
    /// GC-002: a gate is reported as passing with a non-zero exit status.
    PassWithNonZeroExit,
    /// GC-003: a gate is reported as passing with a failure count above zero.
    PassWithFailures,
    /// GC-004: a gate is reported as passing with no failure count recorded
    /// by either the runner's summary or its failure markers.
    FailureCountUnrecorded,
    /// GC-005: the numbers in the claim are not the numbers captured for the
    /// run, so the claim was written from something other than the record.
    NumbersNotCaptured,
}

impl CaptureRule {
    /// Stable rule id, `GC-001` … `GC-005`.
    pub fn id(self) -> &'static str {
        match self {
            Self::ExitStatusUnrecorded => "GC-001",
            Self::PassWithNonZeroExit => "GC-002",
            Self::PassWithFailures => "GC-003",
            Self::FailureCountUnrecorded => "GC-004",
            Self::NumbersNotCaptured => "GC-005",
        }
    }

    /// Short rule name for log lines and reports.
    pub fn name(self) -> &'static str {
        match self {
            Self::ExitStatusUnrecorded => "exit-status-unrecorded",
            Self::PassWithNonZeroExit => "pass-with-nonzero-exit",
            Self::PassWithFailures => "pass-with-failures",
            Self::FailureCountUnrecorded => "failure-count-unrecorded",
            Self::NumbersNotCaptured => "numbers-not-captured",
        }
    }
}

/// One review finding: a passing claim the record does not support.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureFinding {
    /// The rule that fired.
    pub rule: CaptureRule,
    /// The gate the claim is about.
    pub gate: String,
    /// Factual explanation, naming the values that disagree.
    pub message: String,
}

/// Reviews one claim about one gate against the record captured for it.
///
/// `captured` is the [`GateRecord`] when the run was captured, `None` when
/// the claim arrived on its own. The record is authoritative where it exists;
/// a claim that contradicts it is rejected rather than reconciled.
///
/// Only passing claims are reviewed. A claim that a gate *failed* is never
/// rejected here: it holds work instead of releasing it, which fails closed.
pub fn review(claim: &GateClaim, captured: Option<&GateRecord>) -> Vec<CaptureFinding> {
    let mut findings = Vec::new();
    if claim.reported != Reported::Pass {
        return findings;
    }

    // The record is the measurement; the claim's own numbers are only used
    // when nothing was captured. Where both exist they must agree.
    let disagreements = captured.map_or_else(Vec::new, |record| {
        [
            disagreement("exit status", claim.exit_status, Some(record.exit_status)),
            disagreement("passed", claim.passed, record.passed()),
            disagreement("failed", claim.failed, Some(record.failed())),
        ]
        .into_iter()
        .flatten()
        .collect()
    });
    if !disagreements.is_empty() {
        findings.push(CaptureFinding {
            rule: CaptureRule::NumbersNotCaptured,
            gate: claim.gate.clone(),
            message: format!(
                "{} is reported as passing with numbers that are not the numbers captured for \
                 the run: {} — the claim was written from the output, not from the record",
                claim.gate,
                disagreements.join("; ")
            ),
        });
    }

    let exit_status = captured.map(|r| r.exit_status).or(claim.exit_status);
    let failed = captured.map(|r| r.failed()).or(claim.failed);

    match exit_status {
        None => findings.push(CaptureFinding {
            rule: CaptureRule::ExitStatusUnrecorded,
            gate: claim.gate.clone(),
            message: format!(
                "{} is reported as passing with no exit status recorded: a gate whose exit \
                 status was never captured has no verdict, and an unmeasured gate is refused \
                 rather than trusted",
                claim.gate
            ),
        }),
        Some(0) => {}
        Some(status) => findings.push(CaptureFinding {
            rule: CaptureRule::PassWithNonZeroExit,
            gate: claim.gate.clone(),
            message: format!(
                "{} is reported as passing with exit status {status}: the runner said the run \
                 did not succeed, and the exit status is the verdict",
                claim.gate
            ),
        }),
    }

    match failed {
        None => findings.push(CaptureFinding {
            rule: CaptureRule::FailureCountUnrecorded,
            gate: claim.gate.clone(),
            message: format!(
                "{} is reported as passing with no failure count recorded — neither the \
                 runner's summary nor a count of its failure markers: count something, never \
                 read the end",
                claim.gate
            ),
        }),
        Some(0) => {}
        Some(count) => findings.push(CaptureFinding {
            rule: CaptureRule::PassWithFailures,
            gate: claim.gate.clone(),
            message: format!(
                "{} is reported as passing with {count} failing: the run's own count names \
                 the failures, whatever the last lines of output look like",
                claim.gate
            ),
        }),
    }

    findings
}

/// One number the report states against the number that was captured.
///
/// A number the report never stated is not a disagreement: the captured value
/// stands and the gap is reported by the unrecorded-count rules instead.
fn disagreement<T: PartialEq + std::fmt::Display>(
    label: &str,
    claimed: Option<T>,
    captured: Option<T>,
) -> Option<String> {
    match (claimed, captured) {
        (Some(claimed), Some(captured)) if claimed != captured => Some(format!(
            "{label} {} (claim) vs {} (captured)",
            claimed, captured
        )),
        _ => None,
    }
}

/// Reviews several claims at once. Findings are returned for every claim;
/// review never stops at the first rejection, because a report with two
/// fabricated gates must name both.
pub fn review_all(claims: &[GateClaim], records: &[GateRecord]) -> Vec<CaptureFinding> {
    let by_gate: BTreeMap<&str, &GateRecord> = records
        .iter()
        .map(|record| (record.gate.as_str(), record))
        .collect();
    claims
        .iter()
        .flat_map(|claim| review(claim, by_gate.get(claim.gate.as_str()).copied()))
        .collect()
}

/// Renders the gate section of a PR body from the captured records.
///
/// The section is generated: it states each run's verdict, exit status and
/// counts exactly as captured, and quotes the raw evidence lines behind them
/// ([`MAX_EVIDENCE_LINES`] per gate at most, with the remainder counted).
/// There is no prose input to get wrong.
pub fn render_gate_section(records: &[GateRecord]) -> String {
    let red: Vec<&GateRecord> = records.iter().filter(|r| !r.is_green()).collect();
    let mut out = String::from("## Gate evidence (captured)\n\n");
    out.push_str(&format!(
        "Verdict: {}. Generated from the captured exit status and the runners' own counts; \
         no result below is transcribed from the end of an output.\n\n",
        verdict_line(records.len(), red.len())
    ));
    for record in records {
        out.push_str(&format!("- {}\n", record.evidence_line()));
        for line in record.evidence.iter().take(MAX_EVIDENCE_LINES) {
            out.push_str(&format!("  - evidence: `{}`\n", line.trim_end()));
        }
        let overflow = record.evidence.len().saturating_sub(MAX_EVIDENCE_LINES);
        if overflow > 0 {
            out.push_str(&format!(
                "  - evidence: … {overflow} more counted line(s), all read, none quoted\n"
            ));
        }
    }
    out
}

/// The maximum number of raw evidence lines quoted per gate in a rendered
/// report; every counted line is counted whether or not it is quoted.
pub const MAX_EVIDENCE_LINES: usize = 5;

fn verdict_line(total: usize, red: usize) -> String {
    match (total, red) {
        (0, _) => "NO GATES RECORDED — nothing here proves anything".to_string(),
        (total, 0) => format!("PASSED — {total} gate(s) captured, all green"),
        (total, red) => format!("FAILED — {red} of {total} gate(s) captured are red"),
    }
}

/// A TAP result line's verdict.
enum TapLine {
    Ok,
    NotOk,
}

/// Classifies a TAP result line, ignoring plan lines (`1..69`), diagnostics
/// (`# …`) and anything else that is not a result.
fn tap_line_kind(line: &str) -> Option<TapLine> {
    let trimmed = line.trim();
    if starts_as_word(trimmed, "not ok") {
        Some(TapLine::NotOk)
    } else if starts_as_word(trimmed, "ok") {
        Some(TapLine::Ok)
    } else {
        None
    }
}

/// Whether `haystack` begins with `needle` as a whole token: the character
/// after the match is whitespace or the end of the line, so a TAP result
/// keyword or marker never matches inside `not okay`.
fn starts_as_word(haystack: &str, needle: &str) -> bool {
    haystack
        .strip_prefix(needle)
        .is_some_and(|rest| rest.chars().next().is_none_or(char::is_whitespace))
}

/// Reads the two counts out of a single `test result: …` line. A line missing
/// either count is not a result that can be summed, so it yields `None` rather
/// than a partial count.
fn cargo_result_counts(line: &str) -> Option<(u64, u64)> {
    let (_, rest) = line.split_once("test result:")?;
    let mut counts: [Option<u64>; 2] = [None, None];
    for segment in rest.split(';') {
        let mut previous = "";
        for token in segment.split_whitespace() {
            match token {
                "passed" => counts[0] = previous.parse::<u64>().ok(),
                "failed" => counts[1] = previous.parse::<u64>().ok(),
                _ => {}
            }
            previous = token;
        }
    }
    Some((counts[0]?, counts[1]?))
}

/// Sums `test result: …` lines across a whole `cargo test` output.
fn parse_cargo_test(output: &str) -> Option<TestSummary> {
    let mut passed = 0u64;
    let mut failed = 0u64;
    let mut seen = false;
    for line in output.lines() {
        if let Some((line_passed, line_failed)) = cargo_result_counts(line) {
            seen = true;
            passed += line_passed;
            failed += line_failed;
        }
    }
    seen.then_some(TestSummary { passed, failed })
}

/// Counts TAP result lines across the whole output — TAP has no summary line
/// to read, so the count of its own result lines is its summary.
fn parse_tap(output: &str) -> Option<TestSummary> {
    let mut passed = 0u64;
    let mut failed = 0u64;
    let mut seen = false;
    for line in output.lines() {
        match tap_line_kind(line) {
            Some(TapLine::Ok) => {
                seen = true;
                passed += 1;
            }
            Some(TapLine::NotOk) => {
                seen = true;
                failed += 1;
            }
            None => {}
        }
    }
    seen.then_some(TestSummary { passed, failed })
}
