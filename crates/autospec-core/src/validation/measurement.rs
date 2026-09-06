//! Counts that remember whether they were ever taken.
//!
//! Every false verdict in the session that produced #3535 was a *fabricated zero*: a
//! measurement that never happened, rendered as `0`, and then read as a pass. `gofmt -l
//! … | wc -l` printed `0` because `gofmt` was not on `PATH`; `grep -c '^--- PASS'`
//! printed `0` because `go test` had been invoked without `-v`, so it never emitted a
//! single `PASS` line. Both reports were indistinguishable from a genuinely clean run,
//! and both were trusted.
//!
//! [`Measurement`] makes that confusion unrepresentable. It has no `Default`, no
//! `unwrap_or(0)`, and no `From<u64>`; the only way to read a number out of one is
//! [`Measurement::count`], which hands back an `Option` and so forces every consumer to
//! decide what an unmeasured value means for it. `Unknown` carries the reason, because a
//! verdict that cannot say *why* it has no number is barely better than a fabricated one.

use std::path::{Path, PathBuf};

/// A count, or an explanation of why no count exists.
///
/// Construct with [`Measurement::measured`] when a number was genuinely observed, or via
/// [`Measurement::problems_reported`] / [`Measurement::records_parsed`] to derive one
/// from a command's output without inventing a zero.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Measurement {
    /// A number that was actually observed.
    Measured(u64),
    /// No number was observed, and this is why.
    Unknown(String),
}

impl Measurement {
    pub fn measured(count: u64) -> Self {
        Self::Measured(count)
    }

    pub fn unknown(reason: impl Into<String>) -> Self {
        Self::Unknown(reason.into())
    }

    /// Counts the lines of `stdout` a tool flagged as problems.
    ///
    /// Silence is a legitimate answer here — `gofmt -l` prints nothing when every file is
    /// formatted — so an empty stream is `Measured(0)`, but *only* once the tool has
    /// demonstrably run to completion. `exit_code` of `None` means the process never
    /// reached an exit status (it was absent from `PATH`, or was killed), and that is the
    /// case the historical `gofmt` verdict got wrong.
    pub fn problems_reported(
        tool: &str,
        exit_code: Option<i32>,
        stdout: &str,
        flagged: impl Fn(&str) -> bool,
    ) -> Self {
        if exit_code.is_none() {
            return Self::unknown(format!(
                "{tool} did not run to completion, so no count was measured"
            ));
        }
        Self::Measured(stdout.lines().filter(|line| flagged(line)).count() as u64)
    }

    /// Counts the result records a tool emitted.
    ///
    /// Unlike [`Measurement::problems_reported`], silence is *not* an answer: a parser
    /// that recognised no records at all has measured nothing, whatever the exit status
    /// says. `grep -c '^--- PASS'` over a `go test` run without `-v` matches zero lines
    /// not because zero tests passed but because the format never carried the answer.
    pub fn records_parsed(
        tool: &str,
        exit_code: Option<i32>,
        stdout: &str,
        record: impl Fn(&str) -> bool,
    ) -> Self {
        if exit_code.is_none() {
            return Self::unknown(format!(
                "{tool} did not run to completion, so no records were parsed"
            ));
        }
        let parsed = stdout.lines().filter(|line| record(line)).count() as u64;
        if parsed == 0 {
            return Self::unknown(format!(
                "{tool} produced no parseable result records, so nothing was measured"
            ));
        }
        Self::Measured(parsed)
    }

    /// The observed count, or `None` when nothing was measured.
    ///
    /// Deliberately an `Option`: there is no accessor that substitutes a number, because
    /// substituting one is the bug this type exists to prevent.
    pub fn count(&self) -> Option<u64> {
        match self {
            Self::Measured(count) => Some(*count),
            Self::Unknown(_) => None,
        }
    }

    pub fn is_unknown(&self) -> bool {
        matches!(self, Self::Unknown(_))
    }

    /// Why nothing was measured, or `None` when a count exists.
    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::Measured(_) => None,
            Self::Unknown(reason) => Some(reason),
        }
    }

    /// Renders as a JSON number, or `null` — never as `0` for an unmeasured value.
    pub fn to_json(&self) -> String {
        match self {
            Self::Measured(count) => count.to_string(),
            Self::Unknown(_) => "null".to_string(),
        }
    }

    /// Renders for a human report, where `unknown` must not read as a quantity.
    pub fn as_display(&self) -> String {
        match self {
            Self::Measured(count) => count.to_string(),
            Self::Unknown(_) => "unknown".to_string(),
        }
    }
}

/// Fails when `program` is not an executable on `PATH`, naming the tool that is missing.
///
/// Call this *before* measuring anything with the tool. The point is not to duplicate the
/// spawn error — [`std::process::Command`] already reports `NotFound` — but to let a gate
/// refuse to report at all rather than publish a report whose numbers came from a tool
/// that was never there.
pub fn require_tool(program: &str) -> Result<PathBuf, String> {
    resolve_tool(program).ok_or_else(|| {
        format!("required tool {program} is not on PATH; nothing can be measured with it")
    })
}

/// Resolves `program` against `PATH`, or `None` when it is absent.
pub fn resolve_tool(program: &str) -> Option<PathBuf> {
    let candidate = Path::new(program);
    if candidate.components().count() > 1 {
        return candidate.is_file().then(|| candidate.to_path_buf());
    }
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|directory| directory.join(program))
        .find(|path| path.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_absent_tool_measures_nothing_rather_than_zero_problems() {
        let measurement = Measurement::problems_reported("gofmt", None, "", |_| true);

        assert!(measurement.is_unknown());
        assert_eq!(measurement.count(), None);
        assert_eq!(measurement.to_json(), "null");
        assert!(measurement
            .reason()
            .is_some_and(|reason| reason.contains("gofmt")));
    }

    #[test]
    fn a_tool_that_ran_and_flagged_nothing_measures_zero_problems() {
        let measurement = Measurement::problems_reported("gofmt", Some(0), "", |_| true);

        assert_eq!(measurement, Measurement::Measured(0));
        assert_eq!(measurement.to_json(), "0");
        assert_eq!(measurement.as_display(), "0");
    }

    #[test]
    fn output_without_a_single_result_record_measures_nothing() {
        let measurement =
            Measurement::records_parsed("go test", Some(0), "ok  \tpkg\t0.10s\n", |line| {
                line.starts_with("--- PASS")
            });

        assert!(measurement.is_unknown());
        assert_eq!(measurement.as_display(), "unknown");
    }

    #[test]
    fn parsed_result_records_are_counted() {
        let stdout = "--- PASS: TestOne (0.00s)\n--- PASS: TestTwo (0.00s)\n";
        let measurement = Measurement::records_parsed("go test", Some(0), stdout, |line| {
            line.starts_with("--- PASS")
        });

        assert_eq!(measurement.count(), Some(2));
    }

    #[test]
    fn a_missing_tool_is_named_in_the_error() {
        let error = require_tool("autospec-tool-that-does-not-exist")
            .expect_err("an absent tool must not resolve");

        assert!(
            error.contains("autospec-tool-that-does-not-exist"),
            "{error}"
        );
    }
}
