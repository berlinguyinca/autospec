//! Fail-loud verification primitives (issue #3535).
//!
//! Five separate false verdicts occurred in one session. Not one invented a
//! problem — every single one hid work or hid breakage: a formatter that was
//! not on `PATH` was reported as "0 unformatted" because empty output counted
//! as success; pre-existing test failures were reported as new ones because no
//! baseline was compared; a suite that never ran (0/0) was reported as a
//! regression; `go test` without `-v` printed no `PASS` lines and the missing
//! lines were reported as a zero pass count; and a gate that checked only
//! working-tree dirtiness failed an agent that had committed its work.
//!
//! A harness that fails toward "fine" is worse than none, because it is
//! trusted. This module is the shared encoding of the doctrine that prevents
//! each of those verdicts:
//!
//! - every measurement asserts its tools exist **before** measuring — an
//!   absent tool is a hard error that names the tool, never a silent empty
//!   result ([`assert_tool_available`], [`measure`]);
//! - unmeasured renders as `unknown`, never `0` or pass — a fabricated zero
//!   is indistinguishable from a measured one ([`Metric`]);
//! - the real exit status is captured; success is the captured status, never
//!   the absence of output ([`SuiteOutput`]);
//! - "no result lines parsed at all" is `unknown`, not "zero failures"
//!   ([`SuiteOutput::result_counts`]);
//! - pre-existing failures and a missing baseline are `Unmeasured`, never a
//!   clean verdict ([`compare_failures`]);
//! - produced work is uncommitted changes **or** commits ahead of the base
//!   ([`produced_work`]);
//! - a status record can express `unknown` distinctly from `0` for every
//!   numeric field ([`StatusRecord`]).
//!
//! Everything here is pure: callers perform the `command -v` probe and the
//! process spawn, then hand the observations in. That keeps the doctrine
//! testable without a toolchain and identical on Linux and macOS.

use std::collections::BTreeMap;
use std::fmt;

use crate::state::json::{JsonParser, JsonValue};

/// An error a verification step reports instead of a fabricated verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerificationError {
    /// A required tool was not found. The name is the tool the caller asked
    /// for, so the operator can install or fix `PATH` directly.
    MissingTool { tool: String },
    /// A status record or field name is malformed.
    MalformedRecord(String),
}

impl fmt::Display for VerificationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingTool { tool } => write!(
                f,
                "verification tool not found on PATH: {tool} (assert it before measuring; an absent tool is not a clean result)"
            ),
            Self::MalformedRecord(detail) => write!(f, "malformed verification record: {detail}"),
        }
    }
}

impl std::error::Error for VerificationError {}

/// Why a measurement could not be taken. Carried by [`Metric::Unknown`] so a
/// report can say *why* a field is unknown, not just that it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnknownReason {
    /// The tool needed for the measurement was not present.
    ToolMissing,
    /// The command never ran.
    CommandNotRun,
    /// The command ran but produced no parseable result lines.
    NoResultLines,
    /// The comparison needs a baseline that was never recorded.
    BaselineMissing,
}

impl UnknownReason {
    pub const ALL: [Self; 4] = [
        Self::ToolMissing,
        Self::CommandNotRun,
        Self::NoResultLines,
        Self::BaselineMissing,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::ToolMissing => "tool-missing",
            Self::CommandNotRun => "command-not-run",
            Self::NoResultLines => "no-result-lines",
            Self::BaselineMissing => "baseline-missing",
        }
    }
}

/// One numeric field of a verification status: either a value the verifier
/// actually observed, or the reason it could not be measured.
///
/// `Unknown` and `Measured(0)` are distinct. Rendering an `Unknown` as `0`
/// makes a fabricated zero indistinguishable from a measured one, which is
/// the defect this module exists to remove.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Metric {
    Measured(u64),
    Unknown(UnknownReason),
}

impl Metric {
    pub const fn measured(value: u64) -> Self {
        Self::Measured(value)
    }

    pub const fn unknown(reason: UnknownReason) -> Self {
        Self::Unknown(reason)
    }

    pub fn is_measured(&self) -> bool {
        matches!(self, Self::Measured(_))
    }

    pub fn is_unknown(&self) -> bool {
        matches!(self, Self::Unknown(_))
    }

    /// The observed value, or `None` when the field was never measured.
    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Self::Measured(value) => Some(*value),
            Self::Unknown(_) => None,
        }
    }

    /// True only for a *measured* zero. A measured zero is evidence; an
    /// unknown is not, and must never be reported where a zero would be
    /// trusted.
    pub fn is_measured_zero(&self) -> bool {
        matches!(self, Self::Measured(0))
    }

    /// Render the field for a human report. Unmeasured values render as the
    /// literal string `unknown`, never `0` and never a pass.
    pub fn render(&self) -> String {
        match self {
            Self::Measured(value) => value.to_string(),
            Self::Unknown(reason) => format!("unknown ({})", reason.as_str()),
        }
    }

    /// JSON encoding: a bare number when measured, the string `"unknown"`
    /// when not. The two stay distinct after a round trip.
    pub fn to_json(&self) -> String {
        match self {
            Self::Measured(value) => value.to_string(),
            Self::Unknown(_) => "\"unknown\"".to_string(),
        }
    }

    /// Parse the JSON encoding produced by [`Metric::to_json`].
    ///
    /// The JSON form is binary: a number or the string `"unknown"`. The
    /// human-facing detail (which [`UnknownReason`] applied) is intentionally
    /// not encoded; a round trip preserves the measured/unknown distinction,
    /// which is the part that must never collapse into a fabricated zero.
    pub fn parse_json(value: &JsonValue) -> Result<Self, String> {
        match value {
            JsonValue::Number(raw) => {
                let parsed = raw
                    .parse::<u64>()
                    .map_err(|_| "metric must be a non-negative JSON number")?;
                Ok(Self::Measured(parsed))
            }
            JsonValue::String(raw) if raw == "unknown" => {
                Ok(Self::Unknown(UnknownReason::CommandNotRun))
            }
            other => Err(format!(
                "metric must be a JSON number or \"unknown\", got {other:?}"
            )),
        }
    }
}

/// Assert a tool exists **before** measuring. The caller performs the
/// `command -v <tool> >/dev/null` probe (or the equivalent) and passes the
/// answer; an absent tool is a hard error that names the tool, never a silent
/// empty measurement.
pub fn assert_tool_available(tool: &str, available: bool) -> Result<(), VerificationError> {
    if tool.trim().is_empty() {
        return Err(VerificationError::MalformedRecord(
            "tool name must not be empty".to_string(),
        ));
    }
    if !available {
        return Err(VerificationError::MissingTool {
            tool: tool.to_string(),
        });
    }
    Ok(())
}

/// Run a measurement only after the tool preflight passes. The probe never
/// runs when the tool is absent, so no empty output can be mistaken for a
/// clean result.
pub fn measure(
    tool: &str,
    available: bool,
    probe: impl FnOnce() -> Metric,
) -> Result<Metric, VerificationError> {
    assert_tool_available(tool, available)?;
    Ok(probe())
}

/// Captured output of a verification command. `exit_status` is the real
/// process exit status (`${PIPESTATUS[0]}` or equivalent); `None` means the
/// command never ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuiteOutput {
    pub exit_status: Option<i32>,
    pub stdout: String,
}

impl SuiteOutput {
    pub fn new(exit_status: Option<i32>, stdout: impl Into<String>) -> Self {
        Self {
            exit_status,
            stdout: stdout.into(),
        }
    }

    /// Success is the captured exit status, never the absence of output.
    /// An empty stdout from a command that never ran is not a pass.
    pub fn exited_ok(&self) -> bool {
        self.exit_status == Some(0)
    }

    /// `(passed, failed)` counts parsed from result lines.
    ///
    /// A run that produced no parseable result lines at all — no `PASS`, no
    /// `FAIL`, nothing — yields `(unknown, unknown)`, not `(0, 0)`. `go test`
    /// without `-v` prints no `PASS` lines; counting the absence of lines as
    /// a zero pass count is the historical parse artifact this prevents.
    pub fn result_counts(&self, pass_marker: &str, fail_marker: &str) -> (Metric, Metric) {
        if self.exit_status.is_none() {
            return (
                Metric::unknown(UnknownReason::CommandNotRun),
                Metric::unknown(UnknownReason::CommandNotRun),
            );
        }
        let passed = self
            .stdout
            .lines()
            .filter(|line| !pass_marker.is_empty() && line.contains(pass_marker))
            .count() as u64;
        let failed = self
            .stdout
            .lines()
            .filter(|line| !fail_marker.is_empty() && line.contains(fail_marker))
            .count() as u64;
        if passed + failed == 0 {
            (
                Metric::unknown(UnknownReason::NoResultLines),
                Metric::unknown(UnknownReason::NoResultLines),
            )
        } else {
            (Metric::measured(passed), Metric::measured(failed))
        }
    }
}

/// The outcome of comparing current failures against a baseline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegressionVerdict {
    /// Measured failures strictly above the baseline: a real regression.
    Regressed,
    /// Measured, with no failures above the baseline. Pre-existing failures
    /// in the baseline do not make the current run a regression.
    Clean,
    /// The current run or the baseline was never measured. This is rendered
    /// as unknown — it is never rendered as clean.
    Unmeasured,
}

impl RegressionVerdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Regressed => "regressed",
            Self::Clean => "clean",
            Self::Unmeasured => "unmeasured",
        }
    }
}

/// Compare current test failures against the baseline. Reporting failures
/// that the baseline already had as new is the historical
/// `UNIT-TESTS-FAIL` false verdict; a missing baseline is `Unmeasured`,
/// never `Clean`.
pub fn compare_failures(current: Metric, baseline: Metric) -> RegressionVerdict {
    match (current.as_u64(), baseline.as_u64()) {
        (Some(current), Some(baseline)) if current > baseline => RegressionVerdict::Regressed,
        (Some(_), Some(_)) => RegressionVerdict::Clean,
        _ => RegressionVerdict::Unmeasured,
    }
}

/// The outcome of an acceptance suite against its committed total.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcceptanceVerdict {
    /// Every committed acceptance test ran and passed.
    Met,
    /// The suite ran and fewer tests passed than the committed total.
    Regressed,
    /// The suite never ran (0/0), or was never measured. 0/0 is not a
    /// regression — a suite that was never committed produced no evidence in
    /// either direction.
    Unmeasured,
}

impl AcceptanceVerdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Met => "met",
            Self::Regressed => "regressed",
            Self::Unmeasured => "unmeasured",
        }
    }
}

/// Judge an acceptance suite from its measured pass/total counts.
pub fn acceptance_verdict(passed: Metric, total: Metric) -> AcceptanceVerdict {
    match (passed.as_u64(), total.as_u64()) {
        (Some(passed), Some(total)) if total > 0 => {
            if passed >= total {
                AcceptanceVerdict::Met
            } else {
                AcceptanceVerdict::Regressed
            }
        }
        _ => AcceptanceVerdict::Unmeasured,
    }
}

/// A gate that checks for produced work counts **both** uncommitted changes
/// and commits ahead of the base. Checking only working-tree dirtiness fails
/// an agent that committed its work — the historical `GATE: FAIL` false
/// verdict.
pub fn produced_work(uncommitted_changes: u64, commits_ahead: u64) -> bool {
    uncommitted_changes > 0 || commits_ahead > 0
}

/// A verification status record. Every numeric field is a [`Metric`], so an
/// unmeasured field round-trips as `unknown` and stays distinct from `0`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StatusRecord {
    fields: BTreeMap<String, Metric>,
}

impl StatusRecord {
    pub fn new() -> Self {
        Self {
            fields: BTreeMap::new(),
        }
    }

    pub fn with(mut self, name: &str, value: Metric) -> Result<Self, VerificationError> {
        if name.trim().is_empty() {
            return Err(VerificationError::MalformedRecord(
                "field name must not be empty".to_string(),
            ));
        }
        self.fields.insert(name.to_string(), value);
        Ok(self)
    }

    pub fn get(&self, name: &str) -> Option<&Metric> {
        self.fields.get(name)
    }

    pub fn field_count(&self) -> usize {
        self.fields.len()
    }

    /// JSON encoding. Field order is deterministic; measured fields are bare
    /// numbers, unmeasured fields are the string `"unknown"`.
    pub fn to_json(&self) -> String {
        let fields = self
            .fields
            .iter()
            .map(|(name, metric)| format!("\"{}\":{}", escape_json(name), metric.to_json()))
            .collect::<Vec<_>>()
            .join(",");
        format!("{{{fields}}}")
    }

    /// Parse the JSON encoding produced by [`StatusRecord::to_json`].
    pub fn from_json(document: &str) -> Result<Self, String> {
        let fields = JsonParser::new(document)
            .parse()?
            .into_object("status record")?;
        let mut record = Self::new();
        for (name, value) in fields {
            if name.trim().is_empty() {
                return Err("status record field name must not be empty".to_string());
            }
            record.fields.insert(name, Metric::parse_json(&value)?);
        }
        Ok(record)
    }

    /// Render the record for a human report. Unmeasured fields show as
    /// `unknown (reason)`, never as `0`.
    pub fn render(&self) -> String {
        self.fields
            .iter()
            .map(|(name, metric)| format!("{name}: {}", metric.render()))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn escape_json(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            character if character.is_control() => {
                escaped.push_str(&format!("\\u{:04x}", character as u32))
            }
            character => escaped.push(character),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record_with(pairs: &[(&str, Metric)]) -> StatusRecord {
        let mut record = StatusRecord::new();
        for (name, metric) in pairs {
            record = record
                .with(name, *metric)
                .expect("valid field name in test");
        }
        record
    }

    // ── Historical failure 1: `gofmt` was not on PATH and the empty output
    //    was reported as "0 unformatted". ────────────────────────────────────

    #[test]
    fn an_absent_tool_fails_loud_and_names_the_tool() {
        let error = assert_tool_available("gofmt", false).unwrap_err();
        match &error {
            VerificationError::MissingTool { tool } => assert_eq!(tool, "gofmt"),
            other => panic!("expected MissingTool, got {other:?}"),
        }
        assert!(
            error.to_string().contains("gofmt"),
            "the error must name the missing tool: {}",
            error
        );
    }

    #[test]
    fn a_measurement_never_runs_when_its_tool_is_absent() {
        let mut probe_ran = false;
        let result = measure("gofmt", false, || {
            probe_ran = true;
            Metric::measured(0)
        });
        assert!(matches!(result, Err(VerificationError::MissingTool { .. })));
        assert!(!probe_ran, "no empty measurement may be produced");
    }

    // ── Historical failure 4: `go test` without `-v` printed no `PASS` lines
    //    and the absence was reported as `PASS=0`. ──────────────────────────

    #[test]
    fn output_without_result_lines_is_unknown_not_zero() {
        let output = SuiteOutput::new(Some(0), "");
        let (passed, failed) = output.result_counts("PASS", "FAIL");
        assert!(passed.is_unknown());
        assert!(failed.is_unknown());
        assert_eq!(passed.as_u64(), None);
        assert_eq!(passed.render(), "unknown (no-result-lines)");
        assert_ne!(passed.to_json(), "0", "a fabricated zero is forbidden");
    }

    #[test]
    fn a_command_that_never_ran_is_unknown_not_zero() {
        let output = SuiteOutput::new(None, "PASS: something stale\n");
        let (passed, failed) = output.result_counts("PASS", "FAIL");
        assert_eq!(passed, Metric::unknown(UnknownReason::CommandNotRun));
        assert_eq!(failed, Metric::unknown(UnknownReason::CommandNotRun));
        assert!(!output.exited_ok());
    }

    #[test]
    fn real_result_lines_are_measured() {
        let output = SuiteOutput::new(
            Some(1),
            "=== RUN TestA\n--- PASS: TestA\n--- FAIL: TestB\nFAIL\n",
        );
        let (passed, failed) = output.result_counts("PASS", "FAIL");
        assert_eq!(passed, Metric::measured(1));
        assert_eq!(failed, Metric::measured(2));
        assert!(!output.exited_ok());
    }

    // ── Historical failure 2: the 3 unit-test failures were pre-existing and
    //    were reported as new. ───────────────────────────────────────────────

    #[test]
    fn preexisting_failures_are_not_a_regression() {
        assert_eq!(
            compare_failures(Metric::measured(3), Metric::measured(3)),
            RegressionVerdict::Clean
        );
    }

    #[test]
    fn new_failures_above_the_baseline_are_a_regression() {
        assert_eq!(
            compare_failures(Metric::measured(4), Metric::measured(3)),
            RegressionVerdict::Regressed
        );
    }

    #[test]
    fn a_missing_baseline_is_unmeasured_never_clean() {
        assert_eq!(
            compare_failures(
                Metric::measured(3),
                Metric::unknown(UnknownReason::BaselineMissing)
            ),
            RegressionVerdict::Unmeasured
        );
        assert_eq!(
            compare_failures(
                Metric::unknown(UnknownReason::NoResultLines),
                Metric::measured(3)
            ),
            RegressionVerdict::Unmeasured
        );
    }

    // ── Historical failure 3: the acceptance suite was never committed, 0
    //    tests ran, and 0/0 was read as a regression. ───────────────────────

    #[test]
    fn zero_for_zero_is_unmeasured_not_regressed() {
        assert_eq!(
            acceptance_verdict(Metric::measured(0), Metric::measured(0)),
            AcceptanceVerdict::Unmeasured
        );
    }

    #[test]
    fn a_measured_acceptance_suite_is_judged() {
        assert_eq!(
            acceptance_verdict(Metric::measured(5), Metric::measured(5)),
            AcceptanceVerdict::Met
        );
        assert_eq!(
            acceptance_verdict(Metric::measured(4), Metric::measured(5)),
            AcceptanceVerdict::Regressed
        );
    }

    // ── Historical failure 5: the gate checked working-tree dirt and failed
    //    an agent that had committed its work. ──────────────────────────────

    #[test]
    fn committed_work_satisfies_the_gate() {
        assert!(produced_work(0, 2), "commits ahead of the base are work");
        assert!(produced_work(1, 0), "uncommitted changes are work");
        assert!(!produced_work(0, 0), "no changes and no commits is no work");
    }

    // ── Status records: `unknown` must stay distinct from `0`. ─────────────

    #[test]
    fn a_status_record_round_trips_unknown_distinct_from_zero() {
        let record = record_with(&[
            (
                "gofmt_unformatted",
                Metric::unknown(UnknownReason::ToolMissing),
            ),
            ("unit_failures", Metric::measured(0)),
        ]);

        let json = record.to_json();
        assert!(
            json.contains("\"gofmt_unformatted\":\"unknown\""),
            "unmeasured fields must render as unknown: {json}"
        );
        assert!(
            json.contains("\"unit_failures\":0"),
            "measured zero stays a number: {json}"
        );

        let parsed = StatusRecord::from_json(&json).unwrap();
        let unformatted = parsed.get("gofmt_unformatted").expect("field present");
        assert!(
            unformatted.is_unknown(),
            "unknown must round-trip as unknown"
        );
        assert_ne!(
            unformatted.as_u64(),
            Some(0),
            "a round-tripped unknown must not become a zero"
        );
        assert!(parsed.get("unit_failures").unwrap().is_measured_zero());
        assert_eq!(parsed.field_count(), 2);
        assert_eq!(parsed.to_json(), json, "the encoding is idempotent");
    }

    #[test]
    fn a_status_record_rejects_malformed_fields() {
        assert!(StatusRecord::new().with("", Metric::measured(0)).is_err());
        assert!(StatusRecord::from_json("{\"a\":true}").is_err());
        assert!(StatusRecord::from_json("[]").is_err());
    }

    #[test]
    fn a_metric_round_trips_through_json() {
        let measured = Metric::measured(7);
        let unknown = Metric::unknown(UnknownReason::NoResultLines);
        let doc = format!(
            "{{\"measured\":{},\"unknown\":{}}}",
            measured.to_json(),
            unknown.to_json()
        );
        let fields = JsonParser::new(&doc)
            .parse()
            .unwrap()
            .into_object("t")
            .unwrap();
        assert_eq!(Metric::parse_json(&fields["measured"]).unwrap(), measured);
        let parsed_unknown = Metric::parse_json(&fields["unknown"]).unwrap();
        assert!(parsed_unknown.is_unknown());
        assert_ne!(parsed_unknown, measured);
    }

    #[test]
    fn rendered_report_never_shows_unknown_as_zero() {
        let record = record_with(&[
            ("formatting", Metric::unknown(UnknownReason::ToolMissing)),
            ("failures", Metric::measured(0)),
        ]);
        let rendered = record.render();
        assert!(rendered.contains("formatting: unknown (tool-missing)"));
        assert!(rendered.contains("failures: 0"));
    }
}
