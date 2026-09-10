//! Triage of the gate evidence the agent leaves behind (#3715).
//!
//! Before this module, the conversion pass treated every produced patch
//! the same: apply it, then run the full local gate (build, test, fmt)
//! against the current main. The gate is the pass's real job when there
//! is nothing better to go on — but the agent has usually already run
//! that same gate on that same patch, and left the result in the patch's
//! `status.txt`. Re-running it pays the gate's cost again on evidence
//! the pass could have read for free:
//!
//! - a run that **timed out** never reached the gate at all. The local
//!   gate cannot speak about the patch, and the only response is to
//!   re-dispatch;
//! - a run that reported a **fmt or build failure** recorded a
//!   deterministic negative — a property of the submission, not of the
//!   run. A local re-run reproduces the same failure; the patch should
//!   be held with the agent's own evidence;
//! - a run that reported **test failures** recorded an untrusted signal:
//!   tests are flaky, and the agent ran against its own base, which may
//!   have moved. The right response is to gate locally — re-verify
//!   against the current main — which is exactly the pass's real job.
//!
//! This module is the triage policy as pure primitives. It parses the
//! report, decides what the pass should do, and renders the decision
//! line. It runs no gate, dispatches nothing, and moves no files; the
//! caller (the CLI layer) reads the file and executes the decision, the
//! way [`gate`](super::gate) and [`test_diff`](super::test_diff) hand
//! their verdicts to the caller that performs the I/O.
//!
//! 1. **A timed-out run is re-dispatch material, not gate material**
//!    ([`TriageDecision::Redispatch`]). `status=TIMEOUT` and
//!    `status=TIMEOUT-NO-OUTPUT` mean the agent never reached the gate.
//!    A local gate on an untested patch spends the gate's cost saying
//!    nothing about it; a fresh dispatch is the only response. The
//!    timeout outranks every other signal in the file — even a
//!    `fmt_rc != 0` in the same report.
//! 2. **Deterministic negatives are trusted; they hold, they do not
//!    re-run** ([`TriageDecision::Hold`]). `fmt_rc != 0` holds the patch
//!    as [`AgentHoldReason::Unformatted`]
//!    (`AGENT-REPORTED-UNFORMATTED`); `build_rc != 0`, or
//!    `status=BUILD-FAILED` / `TESTS-DO-NOT-COMPILE`, holds it as
//!    [`AgentHoldReason::Unbuilt`] (`AGENT-REPORTED-UNBUILT`).
//!    Formatting and compilation are deterministic: a local re-run
//!    reproduces the failure the agent already paid to observe.
//! 3. **Test negatives are triggers, never holds**
//!    ([`GateBasis::AgentReportedTestFailure`]). `test_rc != 0` or
//!    `status=NEW-TEST-FAILURES` gates locally. The agent's test run was
//!    against its own base on a flaky suite; the pass's real job is to
//!    compare against the current main, and that comparison is the
//!    re-verification. Holding on the agent's negative would let flakiness
//!    kill good patches.
//! 4. **No baseline means gate locally** ([`GateBasis::NoBaseline`]).
//!    `status=UNKNOWN-NO-BASELINE` means the agent's tests failed with
//!    no baseline to attribute to. Attributing against the current main
//!    is exactly what the converter exists to do.
//! 5. **Green is also gated locally** ([`GateBasis::AgentGreen`]). The
//!    agent's `PASS` was against the base the agent ran on, and that
//!    base may have moved. Confirmation is cheap, and it is the pass's
//!    job.
//! 6. **Unreadable fails closed** ([`GateBasis::ReportUnreadable`]). An
//!    empty or unparseable report earns no decision the pass can trust.
//!    The pass gates locally rather than acting on evidence it could
//!    not read; a parse error never re-dispatches and never holds — it
//!    costs one local gate.
//! 7. **The report surfaces in the decision line** ([`decision_line`],
//!    [`held_line`]). A hold must show the agent's own evidence — the
//!    rc, the file count, the status: `HELD: agent reported
//!    fmt_rc=1 (32 files), status=TIMEOUT`.

/// One agent run's gate report, as read from the patch's `status.txt`.
///
/// Every field is `Option` because the file is written by a process
/// that may have died before writing everything. The triage must be
/// able to tell "not recorded" apart from "recorded as zero".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AgentReport {
    /// The terminal status the run recorded (for example `TIMEOUT`,
    /// `BUILD-FAILED`, `NEW-TEST-FAILURES`, `PASS`), if any.
    pub status: Option<String>,
    /// The build stage's exit code, if any.
    pub build_rc: Option<i32>,
    /// The test stage's exit code, if any.
    pub test_rc: Option<i32>,
    /// The fmt stage's exit code, if any.
    pub fmt_rc: Option<i32>,
    /// The number of files the fmt stage listed, if recorded
    /// (`fmt-files.txt: 32 entries`).
    pub fmt_files: Option<usize>,
}

impl AgentReport {
    /// Whether the file recorded nothing the triage can use. An empty
    /// report is not a green report ([`triage`]).
    pub fn is_empty(&self) -> bool {
        self.status.is_none()
            && self.build_rc.is_none()
            && self.test_rc.is_none()
            && self.fmt_rc.is_none()
            && self.fmt_files.is_none()
    }
}

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
            // Gate shape: every token on the line is `key=value`.
            for token in line.split_whitespace() {
                let (key, value) = token
                    .split_once('=')
                    .ok_or_else(|| format!("line {line_number}: not 'key=value': {token}"))?;
                set(&mut report, key.trim(), value.trim())
                    .map_err(|error| format!("line {line_number}: {error}"))?;
            }
        } else {
            // Fleet shape: one `key: value` per line.
            let (key, value) = line.split_once(':').ok_or_else(|| {
                format!("line {line_number}: not 'key=value' or 'key: value': {line}")
            })?;
            set(&mut report, key.trim(), value.trim())
                .map_err(|error| format!("line {line_number}: {error}"))?;
        }
    }
    Ok(report)
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
        _ => {}
    }
    Ok(())
}

fn parse_rc(key: &str, value: &str) -> Result<i32, String> {
    value
        .parse::<i32>()
        .map_err(|_| format!("{key} expects an integer exit code, got {value}"))
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

/// The three responses the pass can make to an agent report (#3715).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TriageDecision {
    /// Re-dispatch the issue; do not gate locally (rule 1).
    Redispatch {
        /// The status that triggered the re-dispatch (`TIMEOUT` or
        /// `TIMEOUT-NO-OUTPUT`).
        status: String,
    },
    /// Hold the patch on the agent's own deterministic negative (rule 2).
    Hold {
        /// Which deterministic signal held the patch.
        reason: AgentHoldReason,
    },
    /// Gate locally against the current main (rules 3–5, 6).
    GateLocally {
        /// Why the local gate is running; the basis goes in the run
        /// summary so the gate reads as re-verification or first
        /// verification.
        basis: GateBasis,
    },
}

/// The deterministic signal that held a patch on the agent's own report
/// (rule 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentHoldReason {
    /// The agent's fmt stage failed (`fmt_rc != 0`).
    Unformatted,
    /// The agent's build stage failed (`build_rc != 0`,
    /// `status=BUILD-FAILED`, or `status=TESTS-DO-NOT-COMPILE`).
    Unbuilt,
}

impl AgentHoldReason {
    /// The token the pass records in the hold line and the retirement.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unformatted => "AGENT-REPORTED-UNFORMATTED",
            Self::Unbuilt => "AGENT-REPORTED-UNBUILT",
        }
    }
}

/// Why the local gate runs although the agent already reported (rules
/// 3–6). The basis goes in the run summary: the gate is a
/// re-verification or a first verification, and the summary says which.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateBasis {
    /// The agent reported test failures (rule 3): `test_rc != 0` or
    /// `status=NEW-TEST-FAILURES`. The gate re-verifies against the
    /// current main; the agent's negative is a trigger, never a hold.
    AgentReportedTestFailure {
        /// The status the run recorded, if any.
        status: Option<String>,
    },
    /// The agent's tests failed with no baseline to attribute to (rule
    /// 4): `status=UNKNOWN-NO-BASELINE`. Attributing against the
    /// current main is the converter's real job.
    NoBaseline,
    /// The agent reported everything green (rule 5): the gate confirms
    /// the result against a main that may have moved since the run.
    AgentGreen,
    /// The report was empty or unparseable (rule 6): the gate runs
    /// because there is no evidence to act on.
    ReportUnreadable {
        /// Why the report could not be read (the parser's error, or
        /// "status file is empty").
        detail: String,
    },
}

/// Decide what the pass should do with a patch whose agent left the
/// given report (#3715). Pure: no I/O, no gate execution — the caller
/// reads the file, calls this, and executes the decision.
///
/// The precedence is the contract, most decisive evidence first:
///
/// 1. `TIMEOUT` / `TIMEOUT-NO-OUTPUT` → [`TriageDecision::Redispatch`].
///    The run never reached the gate, so nothing the pass runs locally
///    can say about the patch. The timeout outranks every other signal
///    in the report.
/// 2. `fmt_rc != 0` → [`TriageDecision::Hold`] as
///    [`AgentHoldReason::Unformatted`]. Deterministic; a local re-run
///    reproduces it.
/// 3. `build_rc != 0` or `BUILD-FAILED` / `TESTS-DO-NOT-COMPILE` →
///    [`TriageDecision::Hold`] as [`AgentHoldReason::Unbuilt`].
///    Deterministic; same reason.
/// 4. `UNKNOWN-NO-BASELINE` → [`TriageDecision::GateLocally`] as
///    [`GateBasis::NoBaseline`]. The converter's real job is to
///    compare against the current main, and there is no baseline to do
///    it with until this pass runs.
/// 5. `test_rc != 0` or `NEW-TEST-FAILURES` →
///    [`TriageDecision::GateLocally`] as
///    [`GateBasis::AgentReportedTestFailure`]. Tests are flaky and the
///    agent ran against its own base; re-verify, never hold.
/// 6. Everything else — green, or a status this triage does not
///    consume — → [`TriageDecision::GateLocally`] as
///    [`GateBasis::AgentGreen`]. The agent's result was against the
///    base the agent ran on; confirming against the current main is the
///    pass's job.
///
/// An empty report (nothing recorded) is not a green report: it gates
/// locally as [`GateBasis::ReportUnreadable`] (rule 6 of the module).
pub fn triage(report: &AgentReport) -> TriageDecision {
    if report.is_empty() {
        return TriageDecision::GateLocally {
            basis: GateBasis::ReportUnreadable {
                detail: "status file is empty".to_string(),
            },
        };
    }
    let status = report.status.as_deref();
    // 1. The timeout outranks everything: the run never reached the
    //    gate, so no local gate can speak about the patch.
    if matches!(status, Some("TIMEOUT") | Some("TIMEOUT-NO-OUTPUT")) {
        return TriageDecision::Redispatch {
            status: status.unwrap().to_string(),
        };
    }
    // 2. fmt: a deterministic negative; trust the agent's.
    if matches!(report.fmt_rc, Some(rc) if rc != 0) {
        return TriageDecision::Hold {
            reason: AgentHoldReason::Unformatted,
        };
    }
    // 3. build: a deterministic negative; trust the agent's.
    if matches!(report.build_rc, Some(rc) if rc != 0)
        || status == Some("BUILD-FAILED")
        || status == Some("TESTS-DO-NOT-COMPILE")
    {
        return TriageDecision::Hold {
            reason: AgentHoldReason::Unbuilt,
        };
    }
    // 4. No baseline: the converter's real job is to supply one.
    if status == Some("UNKNOWN-NO-BASELINE") {
        return TriageDecision::GateLocally {
            basis: GateBasis::NoBaseline,
        };
    }
    // 5. Test negatives: a trigger, never a hold.
    if matches!(report.test_rc, Some(rc) if rc != 0) || status == Some("NEW-TEST-FAILURES") {
        return TriageDecision::GateLocally {
            basis: GateBasis::AgentReportedTestFailure {
                status: report.status.clone(),
            },
        };
    }
    // 6. Green, or a status this triage does not consume: confirm
    //    against the current main.
    TriageDecision::GateLocally {
        basis: GateBasis::AgentGreen,
    }
}

/// Parse a `status.txt` body and triage it in one step. A file that
/// cannot be parsed gates locally as [`GateBasis::ReportUnreadable`]
/// (rule 6): a parse error never re-dispatches and never holds.
pub fn triage_report(content: &str) -> TriageDecision {
    match parse_agent_report(content) {
        Ok(report) => triage(&report),
        Err(detail) => TriageDecision::GateLocally {
            basis: GateBasis::ReportUnreadable { detail },
        },
    }
}

/// The hold line the pass prints for an agent-reported hold (rule 7):
/// the agent's own evidence, not just the hold's reason.
///
/// `held_line(&report, AgentHoldReason::Unformatted)` for a report with
/// `fmt_rc=1`, `fmt_files=Some(32)`, `status=TIMEOUT` prints
/// `HELD: agent reported fmt_rc=1 (32 files), status=TIMEOUT`.
pub fn held_line(report: &AgentReport, reason: AgentHoldReason) -> String {
    let evidence = match reason {
        AgentHoldReason::Unformatted => match (report.fmt_rc, report.fmt_files) {
            (Some(rc), Some(files)) => format!("fmt_rc={rc} ({files} files)"),
            (Some(rc), None) => format!("fmt_rc={rc}"),
            (None, Some(files)) => format!("fmt_files={files}"),
            (None, None) => "fmt failure".to_string(),
        },
        AgentHoldReason::Unbuilt => match report.build_rc {
            Some(rc) => format!("build_rc={rc}"),
            None => "build failure".to_string(),
        },
    };
    match report.status.as_deref() {
        Some(status) => format!("HELD: agent reported {evidence}, status={status}"),
        None => format!("HELD: agent reported {evidence}"),
    }
}

/// The one line the pass prints for any triage decision (rule 7): what
/// the pass is doing, and the report evidence it is acting on.
pub fn decision_line(decision: &TriageDecision, report: &AgentReport) -> String {
    match decision {
        TriageDecision::Redispatch { status } => {
            format!("RE-DISPATCH: agent reported status={status}; the run never reached the gate")
        }
        TriageDecision::Hold { reason } => held_line(report, *reason),
        TriageDecision::GateLocally { basis } => {
            format!("GATE-LOCALLY: {}", basis_line(basis, report))
        }
    }
}

fn basis_line(basis: &GateBasis, report: &AgentReport) -> String {
    match basis {
        GateBasis::AgentReportedTestFailure { status } => match status {
            Some(status) => format!(
                "agent reported test failure (status={status}); re-verifying against current main"
            ),
            None => match report.test_rc {
                Some(rc) => format!(
                    "agent reported test failure (test_rc={rc}); re-verifying against current main"
                ),
                None => "agent reported test failure; re-verifying against current main"
                    .to_string(),
            },
        },
        GateBasis::NoBaseline => {
            "status=UNKNOWN-NO-BASELINE; no baseline to attribute the failures — gating against current main"
                .to_string()
        }
        GateBasis::AgentGreen => "agent reported green; confirming against current main".to_string(),
        GateBasis::ReportUnreadable { detail } => {
            format!("status file unreadable ({detail}); gating against current main")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(
        status: Option<&str>,
        build_rc: Option<i32>,
        test_rc: Option<i32>,
        fmt_rc: Option<i32>,
        fmt_files: Option<usize>,
    ) -> AgentReport {
        AgentReport {
            status: status.map(str::to_string),
            build_rc,
            test_rc,
            fmt_rc,
            fmt_files,
        }
    }

    // --- parsing -----------------------------------------------------

    #[test]
    fn parses_the_gate_shape_single_line() {
        let parsed = parse_agent_report("status=PASS build_rc=0 test_rc=0 fmt_rc=0").unwrap();
        assert_eq!(
            parsed,
            report(Some("PASS"), Some(0), Some(0), Some(0), None)
        );
    }

    #[test]
    fn parses_the_fleet_shape_multi_line() {
        let content = "\
# agent gate record
status: TIMEOUT
build_rc: 0
test_rc: 0
fmt_rc: 1

worker: gpu-4090-03
";
        let parsed = parse_agent_report(content).unwrap();
        assert_eq!(
            parsed,
            report(Some("TIMEOUT"), Some(0), Some(0), Some(1), None)
        );
    }

    #[test]
    fn parses_the_fmt_files_sidecar_count() {
        let parsed = parse_agent_report("fmt-files.txt: 32 entries").unwrap();
        assert_eq!(parsed.fmt_files, Some(32));
        let bare = parse_agent_report("fmt-files.txt: 7").unwrap();
        assert_eq!(bare.fmt_files, Some(7));
    }

    #[test]
    fn ignores_unknown_keys_comments_and_blanks() {
        let parsed =
            parse_agent_report("status: PASS\nagent_secs: 60\n# note\n\ngpu: 4090\n").unwrap();
        assert_eq!(parsed.status.as_deref(), Some("PASS"));
        assert!(parsed.build_rc.is_none());
        assert!(parsed.fmt_files.is_none());
    }

    #[test]
    fn duplicate_keys_take_the_last_value() {
        let parsed = parse_agent_report("status: TIMEOUT\nstatus: PASS").unwrap();
        assert_eq!(parsed.status.as_deref(), Some("PASS"));
    }

    #[test]
    fn mixes_both_shapes_across_lines() {
        // The issue's own example mixes `key=value` and `key: value`.
        let parsed =
            parse_agent_report("status=TIMEOUT\nfmt_rc=1\nfmt-files.txt: 32 entries").unwrap();
        assert_eq!(
            parsed,
            report(Some("TIMEOUT"), None, None, Some(1), Some(32))
        );
    }

    #[test]
    fn a_non_integer_rc_fails_the_file_naming_the_line() {
        let error = parse_agent_report("status: PASS\nbuild_rc: abc").unwrap_err();
        assert!(error.contains("line 2"), "{error}");
        assert!(error.contains("build_rc"), "{error}");
    }

    #[test]
    fn an_empty_value_fails_the_file() {
        let error = parse_agent_report("status:").unwrap_err();
        assert!(error.contains("line 1"), "{error}");
    }

    #[test]
    fn a_line_without_any_separator_fails_the_file() {
        let error = parse_agent_report("status=PASS\ngarbage").unwrap_err();
        assert!(error.contains("line 2"), "{error}");
    }

    #[test]
    fn a_gate_line_with_a_bare_token_fails_the_file() {
        let error = parse_agent_report("status=PASS garbage").unwrap_err();
        assert!(error.contains("line 1"), "{error}");
    }

    #[test]
    fn an_empty_file_is_a_report_not_an_error() {
        let parsed = parse_agent_report("").unwrap();
        assert!(parsed.is_empty());
    }

    // --- triage ------------------------------------------------------

    #[test]
    fn a_timeout_re_dispatches() {
        assert_eq!(
            triage(&report(Some("TIMEOUT"), None, None, None, None)),
            TriageDecision::Redispatch {
                status: "TIMEOUT".to_string()
            }
        );
        assert_eq!(
            triage(&report(Some("TIMEOUT-NO-OUTPUT"), None, None, None, None)),
            TriageDecision::Redispatch {
                status: "TIMEOUT-NO-OUTPUT".to_string()
            }
        );
    }

    #[test]
    fn a_timeout_outranks_every_other_signal() {
        let noisy = report(Some("TIMEOUT"), Some(1), Some(1), Some(1), Some(32));
        assert_eq!(
            triage(&noisy),
            TriageDecision::Redispatch {
                status: "TIMEOUT".to_string()
            }
        );
    }

    #[test]
    fn a_fmt_failure_holds_unformatted() {
        assert_eq!(
            triage(&report(None, Some(0), Some(0), Some(1), Some(32))),
            TriageDecision::Hold {
                reason: AgentHoldReason::Unformatted
            }
        );
    }

    #[test]
    fn a_zero_fmt_rc_does_not_hold() {
        assert_eq!(
            triage(&report(Some("PASS"), Some(0), Some(0), Some(0), None)),
            TriageDecision::GateLocally {
                basis: GateBasis::AgentGreen
            }
        );
    }

    #[test]
    fn a_build_failure_holds_unbuilt() {
        assert_eq!(
            triage(&report(None, Some(101), Some(0), Some(0), None)),
            TriageDecision::Hold {
                reason: AgentHoldReason::Unbuilt
            }
        );
        // The status alone says the same thing.
        assert_eq!(
            triage(&report(Some("BUILD-FAILED"), None, None, None, None)),
            TriageDecision::Hold {
                reason: AgentHoldReason::Unbuilt
            }
        );
        assert_eq!(
            triage(&report(
                Some("TESTS-DO-NOT-COMPILE"),
                None,
                None,
                None,
                None
            )),
            TriageDecision::Hold {
                reason: AgentHoldReason::Unbuilt
            }
        );
    }

    #[test]
    fn fmt_outranks_build_when_both_failed() {
        let report = report(None, Some(101), None, Some(1), None);
        assert_eq!(
            triage(&report),
            TriageDecision::Hold {
                reason: AgentHoldReason::Unformatted
            }
        );
    }

    #[test]
    fn a_test_failure_re_verifies_never_holds() {
        assert_eq!(
            triage(&report(None, Some(0), Some(101), Some(0), None)),
            TriageDecision::GateLocally {
                basis: GateBasis::AgentReportedTestFailure { status: None }
            }
        );
        assert_eq!(
            triage(&report(Some("NEW-TEST-FAILURES"), None, None, None, None)),
            TriageDecision::GateLocally {
                basis: GateBasis::AgentReportedTestFailure {
                    status: Some("NEW-TEST-FAILURES".to_string())
                }
            }
        );
    }

    #[test]
    fn no_baseline_gates_locally() {
        assert_eq!(
            triage(&report(Some("UNKNOWN-NO-BASELINE"), None, None, None, None)),
            TriageDecision::GateLocally {
                basis: GateBasis::NoBaseline
            }
        );
    }

    #[test]
    fn no_baseline_outranks_the_test_failure_it_implies() {
        let report = report(Some("UNKNOWN-NO-BASELINE"), None, Some(101), None, None);
        assert_eq!(
            triage(&report),
            TriageDecision::GateLocally {
                basis: GateBasis::NoBaseline
            }
        );
    }

    #[test]
    fn green_gates_locally_to_confirm() {
        assert_eq!(
            triage(&report(Some("PASS"), Some(0), Some(0), Some(0), None)),
            TriageDecision::GateLocally {
                basis: GateBasis::AgentGreen
            }
        );
    }

    #[test]
    fn an_unconsumed_status_still_gates_locally() {
        // A fleet terminal status this triage does not act on: confirm
        // against the current main, the pass's default job.
        assert_eq!(
            triage(&report(Some("VERIFIED"), None, None, None, None)),
            TriageDecision::GateLocally {
                basis: GateBasis::AgentGreen
            }
        );
    }

    #[test]
    fn an_empty_report_fails_closed() {
        assert_eq!(
            triage(&AgentReport::default()),
            TriageDecision::GateLocally {
                basis: GateBasis::ReportUnreadable {
                    detail: "status file is empty".to_string()
                }
            }
        );
    }

    // --- triage_report ------------------------------------------------

    #[test]
    fn triage_report_parses_and_triages() {
        assert_eq!(
            triage_report("status=TIMEOUT fmt_rc=1"),
            TriageDecision::Redispatch {
                status: "TIMEOUT".to_string()
            }
        );
        assert_eq!(
            triage_report("status: PASS\nbuild_rc: 0\ntest_rc: 0\nfmt_rc: 0"),
            TriageDecision::GateLocally {
                basis: GateBasis::AgentGreen
            }
        );
    }

    #[test]
    fn an_unparseable_file_fails_closed_to_a_local_gate() {
        let decision = triage_report("build_rc: abc");
        match decision {
            TriageDecision::GateLocally {
                basis: GateBasis::ReportUnreadable { detail },
            } => {
                assert!(detail.contains("line 1"), "{detail}");
                assert!(detail.contains("build_rc"), "{detail}");
            }
            other => panic!("expected a failed-closed local gate, got {other:?}"),
        }
    }

    // --- rendering -----------------------------------------------------

    #[test]
    fn the_held_line_carries_the_agents_own_evidence() {
        let report = report(Some("TIMEOUT"), None, None, Some(1), Some(32));
        assert_eq!(
            held_line(&report, AgentHoldReason::Unformatted),
            "HELD: agent reported fmt_rc=1 (32 files), status=TIMEOUT"
        );
    }

    #[test]
    fn the_held_line_degrades_gracefully_when_fields_are_missing() {
        assert_eq!(
            held_line(
                &report(None, None, None, Some(1), None),
                AgentHoldReason::Unformatted
            ),
            "HELD: agent reported fmt_rc=1"
        );
        assert_eq!(
            held_line(
                &report(None, None, None, None, Some(32)),
                AgentHoldReason::Unformatted
            ),
            "HELD: agent reported fmt_files=32"
        );
        assert_eq!(
            held_line(
                &report(None, None, None, None, None),
                AgentHoldReason::Unformatted
            ),
            "HELD: agent reported fmt failure"
        );
        assert_eq!(
            held_line(
                &report(Some("TESTS-DO-NOT-COMPILE"), None, None, None, None),
                AgentHoldReason::Unbuilt
            ),
            "HELD: agent reported build failure, status=TESTS-DO-NOT-COMPILE"
        );
        assert_eq!(
            held_line(
                &report(Some("BUILD-FAILED"), Some(101), None, None, None),
                AgentHoldReason::Unbuilt
            ),
            "HELD: agent reported build_rc=101, status=BUILD-FAILED"
        );
    }

    #[test]
    fn the_decision_line_renders_every_branch() {
        let timeout = report(Some("TIMEOUT"), None, None, None, None);
        assert_eq!(
            decision_line(
                &TriageDecision::Redispatch {
                    status: "TIMEOUT".to_string()
                },
                &timeout
            ),
            "RE-DISPATCH: agent reported status=TIMEOUT; the run never reached the gate"
        );

        let unformatted = report(Some("TIMEOUT"), None, None, Some(1), Some(32));
        assert_eq!(
            decision_line(
                &TriageDecision::Hold {
                    reason: AgentHoldReason::Unformatted
                },
                &unformatted
            ),
            "HELD: agent reported fmt_rc=1 (32 files), status=TIMEOUT"
        );

        assert_eq!(
            decision_line(
                &TriageDecision::GateLocally {
                    basis: GateBasis::AgentReportedTestFailure {
                        status: Some("NEW-TEST-FAILURES".to_string())
                    }
                },
                &report(Some("NEW-TEST-FAILURES"), None, None, None, None)
            ),
            "GATE-LOCALLY: agent reported test failure (status=NEW-TEST-FAILURES); re-verifying against current main"
        );
        assert_eq!(
            decision_line(
                &TriageDecision::GateLocally {
                    basis: GateBasis::AgentReportedTestFailure { status: None }
                },
                &report(None, None, Some(101), None, None)
            ),
            "GATE-LOCALLY: agent reported test failure (test_rc=101); re-verifying against current main"
        );
        assert_eq!(
            decision_line(
                &TriageDecision::GateLocally {
                    basis: GateBasis::NoBaseline
                },
                &report(Some("UNKNOWN-NO-BASELINE"), None, None, None, None)
            ),
            "GATE-LOCALLY: status=UNKNOWN-NO-BASELINE; no baseline to attribute the failures — gating against current main"
        );
        assert_eq!(
            decision_line(
                &TriageDecision::GateLocally {
                    basis: GateBasis::AgentGreen
                },
                &report(Some("PASS"), Some(0), Some(0), Some(0), None)
            ),
            "GATE-LOCALLY: agent reported green; confirming against current main"
        );
        assert_eq!(
            decision_line(
                &TriageDecision::GateLocally {
                    basis: GateBasis::ReportUnreadable {
                        detail: "line 2: build_rc expects an integer exit code, got abc"
                            .to_string()
                    }
                },
                &AgentReport::default()
            ),
            "GATE-LOCALLY: status file unreadable (line 2: build_rc expects an integer exit code, got abc); gating against current main"
        );
    }

    #[test]
    fn the_hold_reason_tokens_are_the_recorded_strings() {
        assert_eq!(
            AgentHoldReason::Unformatted.as_str(),
            "AGENT-REPORTED-UNFORMATTED"
        );
        assert_eq!(AgentHoldReason::Unbuilt.as_str(), "AGENT-REPORTED-UNBUILT");
    }
}
