//! Reading an agent's `status.txt` into an [`AgentReport`] (#3715, #4665).
//!
//! The reader is deliberately asymmetric: lenient to *growth*, strict to
//! *value*. A harness that learns a new field must not break the pass, so
//! unknown keys are ignored; but a known key with an unreadable value fails the
//! whole file, naming the line, because a silently dropped rc turns a hold into
//! a gate. The counts added for #4665 follow the same rule — `test_passed` and
//! `tests_total` decide whether a `VERIFIED` is entitled to exist, so a
//! non-integer there is refused rather than read as zero.

use super::AgentReport;

/// Parse a `status.txt` body into an [`AgentReport`].
///
/// The reader is lenient in one direction and strict in the other, the
/// same way the fleet cost reader (`autospec_core::cost::record`)
/// behaves:
///
/// - **Lenient to growth**: unknown keys are ignored, blank lines and
///   `#` comments are skipped, and a duplicate key takes its last
///   value. The harness can add lines without breaking the pass.
/// - **Strict about what it reads**: a known key with an unreadable
///   value (a non-integer rc, an empty value, a line with no
///   separator) fails the whole file, naming the offending line. A
///   silently dropped rc would turn a hold into a gate.
///
/// Two on-disk shapes are accepted, and they may be mixed:
///
/// - the gate shape: whitespace-separated `key=value` tokens, one line
///   (`status=PASS build_rc=0 test_rc=0 fmt_rc=0`);
/// - the fleet shape: one `key: value` per line, including the
///   `fmt-files.txt: 32 entries` line that records how many files the
///   fmt stage listed.
pub fn parse_agent_report(content: &str) -> Result<AgentReport, String> {
    let mut report = AgentReport::default();
    for (index, raw) in content.lines().enumerate() {
        let line_number = index + 1;
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.contains('=') {
            record_tokens(&mut report, line, line_number)?;
        } else {
            record_pair(&mut report, line, line_number)?;
        }
    }
    Ok(report)
}

/// Read the gate shape: whitespace-separated `key=value` tokens, one line.
fn record_tokens(report: &mut AgentReport, line: &str, line_number: usize) -> Result<(), String> {
    for token in line.split_whitespace() {
        let (key, value) = token
            .split_once('=')
            .ok_or_else(|| format!("line {line_number}: not 'key=value': {token}"))?;
        set(report, key.trim(), value.trim())
            .map_err(|error| format!("line {line_number}: {error}"))?;
    }
    Ok(())
}

/// Read the fleet shape: one `key: value` per line, including the
/// `fmt-files.txt: 32 entries` line.
fn record_pair(report: &mut AgentReport, line: &str, line_number: usize) -> Result<(), String> {
    let (key, value) = line
        .split_once(':')
        .ok_or_else(|| format!("line {line_number}: not 'key=value' or 'key: value': {line}"))?;
    set(report, key.trim(), value.trim()).map_err(|error| format!("line {line_number}: {error}"))
}

/// Record one `key=value` pair. Unknown keys are ignored (the harness
/// may grow the file); known keys are parsed strictly.
fn set(report: &mut AgentReport, key: &str, value: &str) -> Result<(), String> {
    if value.is_empty() {
        return Err(format!("value for `{key}` is empty"));
    }
    match key {
        "status" => report.status = Some(value.to_string()),
        "build_rc" => report.build_rc = Some(parse_rc("build_rc", value)?),
        "test_rc" => report.test_rc = Some(parse_rc("test_rc", value)?),
        "fmt_rc" => report.fmt_rc = Some(parse_rc("fmt_rc", value)?),
        // The fmt stage lists the files it touched in a sidecar file
        // and reports the count here.
        "fmt-files.txt" => report.fmt_files = Some(parse_fmt_files(value)?),
        // Both of these are written by the runner today and dropped by the
        // reader today (#4651): the fleet record is
        // `status=NO-OUTPUT agent_rc=143 agent_secs=2761 changed_files=0`.
        "signal" => report.signal = Some(value.to_string()),
        "agent_rc" => report.agent_rc = Some(parse_rc("agent_rc", value)?),
        // Coverage counters (#4665). Read strictly: `test_passed + test_failed`
        // versus `tests_total` decides whether a `VERIFIED` is entitled to
        // exist, so a value that does not parse is refused rather than
        // defaulted to zero — a silent zero would read as a run that executed
        // nothing, or as a suite of nothing.
        "test_passed" => report.test_passed = Some(parse_count("test_passed", value)?),
        "test_failed" => report.test_failed = Some(parse_count("test_failed", value)?),
        "tests_total" => report.tests_total = Some(parse_count("tests_total", value)?),
        _ => {}
    }
    Ok(())
}

fn parse_rc(key: &str, value: &str) -> Result<i32, String> {
    value
        .parse::<i32>()
        .map_err(|_| format!("{key} expects an integer exit code, got {value}"))
}

fn parse_count(key: &str, value: &str) -> Result<u64, String> {
    value
        .parse::<u64>()
        .map_err(|_| format!("{key} expects a non-negative test count, got {value}"))
}

fn parse_fmt_files(value: &str) -> Result<usize, String> {
    // The recorded shape is "32 entries"; a bare count is accepted.
    let count = value
        .strip_suffix("entries")
        .map(str::trim)
        .unwrap_or(value);
    count
        .parse::<usize>()
        .map_err(|_| format!("fmt-files.txt expects a file count, got {value}"))
}
