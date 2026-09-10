//! Conversion-pipeline gate contract (issue #3748).
//!
//! Agent-generated patches pass through a build gate before they may be
//! emitted as conversion candidates: the gate runs a build stage, a test
//! stage, and a fmt check, records each stage's exit code in a status file,
//! and the status file's classification decides whether the patch enters the
//! conversion queue.
//!
//! The incident this module encodes: a gate whose build stage was
//! `cargo build` reported `build_rc=0` while the test stage failed to
//! compile the patch's tests (rustc errors such as `cannot find function
//! ... in this scope`). The classifier saw a green build and classified the
//! run `UNKNOWN-NO-BASELINE` — a status that asserts *no baseline exists* —
//! even though the actual evidence was that the tests do not compile at all.
//! The patch was emitted as a conversion candidate on the strength of a
//! build stage that had never looked at the test targets.
//!
//! Three consequences are now load-bearing:
//!
//! 1. **A gate's scope, not just its outcome, is part of its contract.**
//!    `cargo build` compiles lib and bins; it does not compile test targets,
//!    so its exit code is no evidence about the tests. The gate's build stage
//!    is [`build_gate_command`] (`cargo check --all-targets`), and
//!    [`gate_scope_violation`] rejects commands whose scope is narrower than
//!    what the test stage will build.
//!
//! 2. **`TESTS-DO-NOT-COMPILE` is a terminal status of its own.** When the
//!    test stage failed to build, that *is* the classification. It requires
//!    no baseline and holds unconditionally: a baseline can justify (attribute)
//!    a test failure, but no baseline can justify a test that does not
//!    compile. `UNKNOWN-NO-BASELINE` holds only when the test stage actually
//!    built and failed at run time with no baseline to attribute the failures
//!    to.
//!
//! 3. **Contradictory signals are flagged at write time.** A status file that
//!    claims `build_rc=0` while recording that the test stage failed to build
//!    is self-contradictory — it is the signature of a gate whose build stage
//!    never saw the test targets. [`render_status_file`] and
//!    [`write_status_file`] mark such files with a `contradiction=` token at
//!    write time, and [`parse_status_file`] surfaces the flag (re-deriving it
//!    for files written before the flag existed) so downstream consumers see
//!    the `build_rc=0` claim for what it is.
//!
//! 4. **An absolute-green run is its own status (issue #3798).** A baseline
//!    answers the weaker question — did the patch make things *worse*? — and
//!    a run whose gates all ran and returned zero does not need it. Two
//!    incompatible situations used to collapse into one label: "we could not
//!    measure the code" and "we measured everything; we could not compare
//!    the delta". They are now separate statuses. `VERIFIED-ABSOLUTE`
//!    records that every gate ran and returned zero with no baseline to
//!    compare against, and it admits to the conversion queue exactly as
//!    `PASS` does. `UNKNOWN-NO-BASELINE` holds only when the test stage
//!    failed at run time with no baseline to attribute the failures to — a
//!    report that the harness did not prepare a baseline, not a judgement
//!    about the patch. A status file that claims `UNKNOWN-NO-BASELINE` while
//!    recording every gate green is mislabeled: its recorded rcs are
//!    positive evidence that every gate ran and passed, and
//!    [`StatusFile::effective_status`] re-derives the status the evidence
//!    supports, so such a green artifact is never terminal.

use std::fs;
use std::io;
use std::path::Path;

/// The build stage of the gate.
///
/// `cargo check --all-targets` compiles every target the test stage will
/// build — lib, bins, tests, benches, examples — without linking, so a green
/// exit code is genuine evidence that the patch's tests compile.
pub const BUILD_GATE_COMMAND: [&str; 3] = ["cargo", "check", "--all-targets"];

/// The gate's build-stage command.
pub fn build_gate_command() -> Vec<String> {
    BUILD_GATE_COMMAND.map(str::to_string).to_vec()
}

/// A gate command that does not compile every target the test stage will
/// build is lying about its scope, whatever its exit code says.
///
/// Accepts `cargo check --all-targets` (the canonical build gate) and
/// `cargo test ...` (which compiles all targets and then runs them, so its
/// scope is covered even though it is slower). Rejects `cargo build`, bare
/// `cargo check`, and anything that is not a cargo target build at all.
pub fn gate_scope_violation(command: &[String]) -> Option<String> {
    let Some(cargo) = command.first() else {
        return Some("gate command is empty".to_string());
    };
    let name = cargo.rsplit('/').next().unwrap_or(cargo);
    if name != "cargo" {
        return Some(format!("gate build stage must run cargo, not {cargo}"));
    }
    let rest = &command[1..];
    let Some(sub) = rest.first() else {
        return Some("bare cargo runs no gate stage".to_string());
    };
    match sub.as_str() {
        "build" => Some(
            "cargo build compiles lib and bins but not test targets; use cargo check \
             --all-targets"
                .to_string(),
        ),
        "check" => {
            if rest.iter().any(|arg| arg == "--all-targets") {
                None
            } else {
                Some(
                    "cargo check without --all-targets skips the test, bench, and \
                     example targets the test stage will build"
                        .to_string(),
                )
            }
        }
        "test" => None,
        other => Some(format!(
            "cargo {other:?} is not an accepted gate build stage"
        )),
    }
}

/// Raw evidence from one gate run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateRun {
    /// Exit code of the build stage.
    pub build_rc: i32,
    /// Exit code of the test stage.
    pub test_rc: i32,
    /// `true` when the test stage failed *before running any test* — the
    /// patch's test targets do not compile. A test that panics at run time
    /// leaves this `false` (the targets built fine; a test failed).
    pub test_build_failed: bool,
    /// Exit code of the fmt check. Recorded as evidence; fmt is gated
    /// separately from build/test admission.
    pub fmt_rc: i32,
    /// A baseline of pre-change test results exists and was consulted.
    pub has_baseline: bool,
    /// The node the gate ran on.
    ///
    /// Recorded with every verdict (issue #3725). The scheduler assigns the
    /// node per run, so a verdict and its node belong together: a reader
    /// comparing two runs' failures needs to know whether they ran in the
    /// same world. Always written to the status file.
    pub node: String,
    /// Whether the declared test-database dependency was reachable at run
    /// time.
    ///
    /// `true` when no test-database dependency is declared, or when the
    /// declared one is reachable. `false` when a declared dependency is
    /// unreachable: the test stage's *runtime* results are void (the targets
    /// never ran against a live database), so the run classifies as
    /// [`GateStatus::NoTestDb`] — a distinct state from "tests failed" — and
    /// the failures do not count against the patch (issue #3725).
    pub test_db_reachable: bool,
}

/// The classification of a gate run, as written to the status file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateStatus {
    /// The build stage itself failed; nothing downstream is evidence.
    BuildFailed,
    /// The test stage failed to build. Terminal and unconditional: no
    /// baseline can justify a test that does not compile.
    TestsDoNotCompile,
    /// The declared test-database dependency was unreachable, so the test
    /// stage's runtime results are void. Terminal on the evidence at hand:
    /// the targets did not run, so the failures are a property of the node,
    /// not of the patch — a non-result rather than a regression (issue
    /// #3725).
    NoTestDb,
    /// The test stage built and failed at run time; the baseline attributes
    /// the failures to this change.
    NewTestFailures,
    /// The test stage built and failed at run time, with no baseline to
    /// attribute the failures to. Holds only when the tests actually built:
    /// a green run with no baseline is [`GateStatus::VerifiedAbsolute`],
    /// not this. A missing baseline is a harness fault (the run was not
    /// prepared with one), not a property of the patch (issue #3798).
    UnknownNoBaseline,
    /// Build and test stages ran and returned zero, with no baseline to
    /// compare against: absolute green (issue #3798). A baseline only
    /// answers the weaker question of whether the patch made things worse;
    /// "every gate passed" is meaningful without one. Admits to the
    /// conversion queue as `PASS` does.
    VerifiedAbsolute,
    /// Build and test stages passed; the patch may enter the conversion queue.
    Pass,
}

impl GateStatus {
    /// The wire form written to the status file.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BuildFailed => "BUILD-FAILED",
            Self::TestsDoNotCompile => "TESTS-DO-NOT-COMPILE",
            Self::NoTestDb => "NO-TEST-DB",
            Self::NewTestFailures => "NEW-TEST-FAILURES",
            Self::UnknownNoBaseline => "UNKNOWN-NO-BASELINE",
            Self::VerifiedAbsolute => "VERIFIED-ABSOLUTE",
            Self::Pass => "PASS",
        }
    }

    /// Parses the wire form.
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "BUILD-FAILED" => Ok(Self::BuildFailed),
            "TESTS-DO-NOT-COMPILE" => Ok(Self::TestsDoNotCompile),
            "NO-TEST-DB" => Ok(Self::NoTestDb),
            "NEW-TEST-FAILURES" => Ok(Self::NewTestFailures),
            "UNKNOWN-NO-BASELINE" => Ok(Self::UnknownNoBaseline),
            "VERIFIED-ABSOLUTE" => Ok(Self::VerifiedAbsolute),
            "PASS" => Ok(Self::Pass),
            other => Err(format!("unknown gate status: {other}")),
        }
    }

    /// Terminal statuses are final on the evidence at hand: re-running
    /// downstream stages or consulting a baseline cannot demote them.
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::BuildFailed | Self::TestsDoNotCompile | Self::NoTestDb
        )
    }

    /// Only a green build *and* a green test stage admit a patch to the
    /// conversion queue — with or without a baseline: `PASS` (green, a
    /// baseline existed and was compared) and `VERIFIED-ABSOLUTE` (green,
    /// no baseline to compare against). `TESTS-DO-NOT-COMPILE`,
    /// `UNKNOWN-NO-BASELINE`, `NEW-TEST-FAILURES`, and `BUILD-FAILED`
    /// never do.
    pub const fn admits_to_conversion_queue(self) -> bool {
        matches!(self, Self::Pass | Self::VerifiedAbsolute)
    }
}

/// Classifies a gate run.
///
/// Precedence, most specific evidence first:
///
/// 1. `test_build_failed` — the test stage failed to build. This is
///    unconditional: it holds regardless of `build_rc`, `test_rc`, or
///    `has_baseline`, because a test that does not compile is a fact about
///    the patch, not about the baseline.
/// 2. `build_rc != 0` — the build stage failed; downstream evidence is void.
/// 3. `!test_db_reachable` — the declared test-database dependency was
///    unreachable, so the test stage never ran against a live database and
///    its runtime exit code is void. Classifies as `NO-TEST-DB`, a
///    non-result rather than a regression (issue #3725).
/// 4. `test_rc != 0` — the tests built and failed at run time; a baseline
///    attributes the failures (`NEW-TEST-FAILURES`), its absence does not
///    (`UNKNOWN-NO-BASELINE`).
/// 5. otherwise — every gate ran and returned zero: `PASS` when a baseline
///    existed and was compared, `VERIFIED-ABSOLUTE` when it did not (issue
///    #3798). Absolute green needs no baseline — a baseline only answers
///    the weaker question of whether the patch made things worse, and a
///    green run has already answered the stronger one.
pub fn classify_gate_run(run: &GateRun) -> GateStatus {
    if run.test_build_failed {
        return GateStatus::TestsDoNotCompile;
    }
    if run.build_rc != 0 {
        return GateStatus::BuildFailed;
    }
    if !run.test_db_reachable {
        return GateStatus::NoTestDb;
    }
    if run.test_rc != 0 {
        return if run.has_baseline {
            GateStatus::NewTestFailures
        } else {
            GateStatus::UnknownNoBaseline
        };
    }
    if run.has_baseline {
        GateStatus::Pass
    } else {
        GateStatus::VerifiedAbsolute
    }
}

/// The write-time contradiction check.
///
/// `build_rc=0` claims that the build stage verified the code it was scoped
/// to, and `test_build_failed` records that the test stage could not build
/// the same code. Both can only be true if the build stage never saw the
/// test targets — exactly the incident this module prevents. Such a run must
/// be flagged in the status file, not silently averaged into a status.
pub fn contradictory_signals(run: &GateRun) -> Option<&'static str> {
    (run.build_rc == 0 && run.test_build_failed).then_some("build_ok_but_tests_do_not_compile")
}

/// Renders the status file line for a gate run, flagging contradictory
/// signals at write time.
///
/// Format: `status=<STATUS> node=<NODE> build_rc=<n> test_rc=<n>
/// fmt_rc=<n>` followed by ` contradiction=<reason>` when the signals
/// contradict each other.
pub fn render_status_file(run: &GateRun) -> String {
    let status = classify_gate_run(run);
    let mut line = format!(
        "status={} node={} build_rc={} test_rc={} fmt_rc={}",
        status.as_str(),
        run.node,
        run.build_rc,
        run.test_rc,
        run.fmt_rc,
    );
    if let Some(reason) = contradictory_signals(run) {
        line.push_str(&format!(" contradiction={reason}"));
    }
    line
}

/// Writes the status file for a gate run, flagging contradictory signals at
/// write time, and returns the classification written.
pub fn write_status_file(path: &Path, run: &GateRun) -> io::Result<GateStatus> {
    let status = classify_gate_run(run);
    fs::write(path, format!("{}\n", render_status_file(run)))?;
    Ok(status)
}

/// A parsed status file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusFile {
    pub status: GateStatus,
    pub build_rc: i32,
    pub test_rc: i32,
    pub fmt_rc: i32,
    /// The node the run happened on, as recorded in the file. Empty for files
    /// written before the `node=` token existed.
    pub node: String,
    /// The `contradiction=` token as written, if any.
    pub contradiction: Option<String>,
}

impl StatusFile {
    /// True when the file carries the write-time flag, or when the flag can
    /// be re-derived from the recorded signals — which catches files written
    /// before the flag existed.
    pub fn is_contradictory(&self) -> bool {
        self.contradiction.is_some()
            || (self.build_rc == 0 && self.status == GateStatus::TestsDoNotCompile)
    }

    /// True when the file claims `UNKNOWN-NO-BASELINE` while every recorded
    /// gate returned zero — the mislabel from issue #3798, where a fully
    /// green run was labelled as if it had been unmeasurable. The recorded
    /// rcs are positive evidence that every gate ran and passed; the status
    /// line is the one field that is wrong.
    pub fn is_mislabeled_no_baseline(&self) -> bool {
        self.status == GateStatus::UnknownNoBaseline
            && self.build_rc == 0
            && self.test_rc == 0
            && self.fmt_rc == 0
    }

    /// The status the recorded evidence actually supports (issue #3798).
    /// A mislabeled no-baseline file with every gate green is
    /// [`GateStatus::VerifiedAbsolute`] and admits to the conversion queue;
    /// every other file keeps its written status.
    pub fn effective_status(&self) -> GateStatus {
        if self.is_mislabeled_no_baseline() {
            GateStatus::VerifiedAbsolute
        } else {
            self.status
        }
    }
}

/// Parses a status file produced by [`render_status_file`].
///
/// Accepts the canonical single-line form and multi-line files; whitespace
/// separates `key=value` tokens. The `node=` token is optional: files written
/// before it existed parse with an empty [`StatusFile::node`].
pub fn parse_status_file(text: &str) -> Result<StatusFile, String> {
    let mut status: Option<GateStatus> = None;
    let mut build_rc: Option<i32> = None;
    let mut test_rc: Option<i32> = None;
    let mut fmt_rc: Option<i32> = None;
    let mut node: Option<String> = None;
    let mut contradiction: Option<String> = None;

    for token in text.split_whitespace() {
        let (key, value) = token
            .split_once('=')
            .ok_or_else(|| format!("status file token is not key=value: {token}"))?;
        match key {
            "status" => {
                if status.is_some() {
                    return Err("status file declares status twice".to_string());
                }
                status = Some(GateStatus::parse(value)?);
            }
            "build_rc" => build_rc = Some(parse_rc(key, value)?),
            "test_rc" => test_rc = Some(parse_rc(key, value)?),
            "fmt_rc" => fmt_rc = Some(parse_rc(key, value)?),
            "node" => {
                if node.is_some() {
                    return Err("status file declares node twice".to_string());
                }
                node = Some(value.to_string());
            }
            "contradiction" => {
                if contradiction.is_some() {
                    return Err("status file declares contradiction twice".to_string());
                }
                if value.is_empty() {
                    return Err("contradiction token has no reason".to_string());
                }
                contradiction = Some(value.to_string());
            }
            other => return Err(format!("unknown status file key: {other}")),
        }
    }

    Ok(StatusFile {
        status: status.ok_or("status file has no status")?,
        build_rc: build_rc.ok_or("status file has no build_rc")?,
        test_rc: test_rc.ok_or("status file has no test_rc")?,
        fmt_rc: fmt_rc.ok_or("status file has no fmt_rc")?,
        node: node.unwrap_or_default(),
        contradiction,
    })
}

fn parse_rc(key: &str, value: &str) -> Result<i32, String> {
    value
        .parse::<i32>()
        .map_err(|_| format!("status file {key} is not an integer: {value}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// The exact evidence from the incident: green `cargo build`, test stage
    /// failed to compile, fmt check failed, no baseline.
    fn incident_run() -> GateRun {
        GateRun {
            build_rc: 0,
            test_rc: 101,
            test_build_failed: true,
            fmt_rc: 1,
            has_baseline: false,
            node: "hive-as-11-2-54".to_string(),
            test_db_reachable: true,
        }
    }

    // --- gate scope -------------------------------------------------------

    #[test]
    fn the_gate_compiles_all_targets() {
        assert_eq!(
            build_gate_command(),
            vec![
                "cargo".to_string(),
                "check".to_string(),
                "--all-targets".to_string()
            ]
        );
        assert_eq!(gate_scope_violation(&build_gate_command()), None);
    }

    #[test]
    fn bare_cargo_build_is_a_scope_violation() {
        let cmd = vec!["cargo".to_string(), "build".to_string()];
        let violation = gate_scope_violation(&cmd).expect("cargo build must be rejected");
        assert!(violation.contains("not test targets"), "{violation}");
    }

    #[test]
    fn bare_cargo_check_is_a_scope_violation() {
        let cmd = vec!["cargo".to_string(), "check".to_string()];
        assert!(gate_scope_violation(&cmd).is_some());
    }

    #[test]
    fn cargo_test_covers_the_scope() {
        let cmd = vec![
            "cargo".to_string(),
            "test".to_string(),
            "--no-run".to_string(),
        ];
        assert_eq!(gate_scope_violation(&cmd), None);
    }

    #[test]
    fn non_cargo_commands_are_not_gates() {
        let cmd = vec!["make".to_string(), "all".to_string()];
        assert!(gate_scope_violation(&cmd).is_some());
        assert!(gate_scope_violation(&[]).is_some());
    }

    // --- classification ---------------------------------------------------

    #[test]
    fn the_incident_classifies_as_tests_do_not_compile() {
        let status = classify_gate_run(&incident_run());
        assert_eq!(status, GateStatus::TestsDoNotCompile);
        assert_eq!(status.as_str(), "TESTS-DO-NOT-COMPILE");
    }

    #[test]
    fn tests_do_not_compile_holds_even_with_a_baseline() {
        let mut run = incident_run();
        run.has_baseline = true;
        assert_eq!(classify_gate_run(&run), GateStatus::TestsDoNotCompile);
    }

    #[test]
    fn tests_do_not_compile_is_terminal() {
        assert!(GateStatus::TestsDoNotCompile.is_terminal());
        assert!(!GateStatus::UnknownNoBaseline.is_terminal());
        assert!(!GateStatus::NewTestFailures.is_terminal());
        assert!(GateStatus::BuildFailed.is_terminal());
    }

    #[test]
    fn unknown_no_baseline_holds_only_when_the_tests_built() {
        let run = GateRun {
            build_rc: 0,
            test_rc: 101,
            test_build_failed: false,
            fmt_rc: 0,
            has_baseline: false,
            node: "hive-as-11-2-54".to_string(),
            test_db_reachable: true,
        };
        assert_eq!(classify_gate_run(&run), GateStatus::UnknownNoBaseline);
    }

    #[test]
    fn a_baseline_attributes_run_time_failures() {
        let run = GateRun {
            build_rc: 0,
            test_rc: 101,
            test_build_failed: false,
            fmt_rc: 0,
            has_baseline: true,
            node: "hive-as-11-2-54".to_string(),
            test_db_reachable: true,
        };
        assert_eq!(classify_gate_run(&run), GateStatus::NewTestFailures);
    }

    #[test]
    fn a_failed_build_stage_is_its_own_status() {
        let run = GateRun {
            build_rc: 101,
            test_rc: 101,
            test_build_failed: false,
            fmt_rc: 0,
            has_baseline: false,
            node: "hive-as-11-2-54".to_string(),
            test_db_reachable: true,
        };
        assert_eq!(classify_gate_run(&run), GateStatus::BuildFailed);
    }

    #[test]
    fn a_green_run_with_a_baseline_passes() {
        let run = GateRun {
            build_rc: 0,
            test_rc: 0,
            test_build_failed: false,
            fmt_rc: 0,
            has_baseline: true,
            node: "hive-as-11-3-51".to_string(),
            test_db_reachable: true,
        };
        assert_eq!(classify_gate_run(&run), GateStatus::Pass);
    }

    #[test]
    fn a_green_run_without_a_baseline_is_verified_absolute() {
        // Issue #3798: "all gates pass absolutely" is strictly more
        // information than a baseline comparison, not less. A green run
        // with no baseline is its own status, and it admits.
        let run = GateRun {
            build_rc: 0,
            test_rc: 0,
            test_build_failed: false,
            fmt_rc: 0,
            has_baseline: false,
            node: "hive-as-11-3-51".to_string(),
            test_db_reachable: true,
        };
        let status = classify_gate_run(&run);
        assert_eq!(status, GateStatus::VerifiedAbsolute);
        assert_eq!(status.as_str(), "VERIFIED-ABSOLUTE");
        assert!(status.admits_to_conversion_queue());
        assert!(!status.is_terminal());
    }

    // --- conversion queue admission ----------------------------------------

    #[test]
    fn patches_whose_tests_do_not_compile_never_reach_the_conversion_queue() {
        assert!(!GateStatus::TestsDoNotCompile.admits_to_conversion_queue());
        assert!(!GateStatus::UnknownNoBaseline.admits_to_conversion_queue());
        assert!(!GateStatus::NewTestFailures.admits_to_conversion_queue());
        assert!(!GateStatus::BuildFailed.admits_to_conversion_queue());
        assert!(GateStatus::Pass.admits_to_conversion_queue());
        assert!(GateStatus::VerifiedAbsolute.admits_to_conversion_queue());
        assert!(!classify_gate_run(&incident_run()).admits_to_conversion_queue());
    }

    // --- status file and write-time flag ------------------------------------

    #[test]
    fn contradictory_signals_are_flagged_at_write_time() {
        let run = incident_run();
        let line = render_status_file(&run);
        assert!(line.starts_with("status=TESTS-DO-NOT-COMPILE"), "{line}");
        assert!(line.contains("build_rc=0"), "{line}");
        assert!(line.contains("test_rc=101"), "{line}");
        assert!(line.contains("fmt_rc=1"), "{line}");
        assert!(
            line.contains("contradiction=build_ok_but_tests_do_not_compile"),
            "{line}"
        );
    }

    #[test]
    fn a_consistent_run_is_not_flagged() {
        let run = GateRun {
            build_rc: 101,
            test_rc: 101,
            test_build_failed: true,
            fmt_rc: 0,
            has_baseline: false,
            node: "hive-as-11-2-54".to_string(),
            test_db_reachable: true,
        };
        // The build stage itself reports the failure, so its nonzero rc and
        // the test-stage build failure agree: no contradiction.
        let line = render_status_file(&run);
        assert!(!line.contains("contradiction="), "{line}");
    }

    #[test]
    fn write_status_file_round_trips_through_the_parser() {
        let path = PathBuf::from(std::env::temp_dir())
            .join(format!("conversion-gate-status-{}.txt", std::process::id()));
        let run = incident_run();
        let written = write_status_file(&path, &run).expect("write succeeds");
        assert_eq!(written, GateStatus::TestsDoNotCompile);

        let text = fs::read_to_string(&path).expect("read back");
        let parsed = parse_status_file(&text).expect("parses");
        let _ = fs::remove_file(&path);

        assert_eq!(parsed.status, GateStatus::TestsDoNotCompile);
        assert_eq!(parsed.node, "hive-as-11-2-54");
        assert_eq!(parsed.build_rc, 0);
        assert_eq!(parsed.test_rc, 101);
        assert_eq!(parsed.fmt_rc, 1);
        assert_eq!(
            parsed.contradiction.as_deref(),
            Some("build_ok_but_tests_do_not_compile")
        );
        assert!(parsed.is_contradictory());
    }

    #[test]
    fn the_parser_rederives_the_flag_for_files_written_before_it_existed() {
        // A legacy file: the pre-fix classifier swallowed the test-build
        // failure into a status that carries no contradiction token.
        let legacy = "status=TESTS-DO-NOT-COMPILE build_rc=0 test_rc=101 fmt_rc=1\n";
        let parsed = parse_status_file(legacy).expect("parses");
        assert!(parsed.contradiction.is_none());
        assert!(parsed.is_contradictory());
    }

    #[test]
    fn statuses_round_trip_through_their_wire_forms() {
        for status in [
            GateStatus::BuildFailed,
            GateStatus::TestsDoNotCompile,
            GateStatus::NoTestDb,
            GateStatus::NewTestFailures,
            GateStatus::UnknownNoBaseline,
            GateStatus::VerifiedAbsolute,
            GateStatus::Pass,
        ] {
            assert_eq!(
                GateStatus::parse(status.as_str()).expect("round trip"),
                status
            );
        }
        assert!(GateStatus::parse("MAYBE").is_err());
    }

    #[test]
    fn the_parser_rejects_malformed_files() {
        assert!(parse_status_file("").is_err());
        assert!(parse_status_file("status=PASS").is_err()); // missing rcs
        assert!(parse_status_file("status=NOPE build_rc=0 test_rc=0 fmt_rc=0").is_err());
        assert!(parse_status_file("status=PASS build_rc=x test_rc=0 fmt_rc=0").is_err());
        assert!(parse_status_file("notkeyvalue").is_err());
        assert!(
            parse_status_file("status=PASS status=PASS build_rc=0 test_rc=0 fmt_rc=0").is_err()
        );
    }

    // --- NO-TEST-DB (issue #3725) ------------------------------------------

    /// A run on a node whose declared test database is unreachable: the build
    /// stage is green and the test stage "failed" (connection refused -> rc
    /// 101), but the targets never ran against a live database, so the runtime
    /// exit code is void.
    fn no_test_db_run() -> GateRun {
        GateRun {
            build_rc: 0,
            test_rc: 101,
            test_build_failed: false,
            fmt_rc: 0,
            has_baseline: true,
            node: "hive-as-11-2-54".to_string(),
            test_db_reachable: false,
        }
    }

    #[test]
    fn an_unreachable_test_db_is_a_distinct_status_not_a_failure() {
        // A baseline exists and the test stage "failed" (rc 101). Without the
        // DB-awareness this would classify as NEW-TEST-FAILURES — a regression
        // attributed to the patch. With it, the honest report is NO-TEST-DB:
        // the targets did not run. It holds with or without a baseline.
        let run = no_test_db_run();
        assert_eq!(classify_gate_run(&run), GateStatus::NoTestDb);
        assert_eq!(classify_gate_run(&run).as_str(), "NO-TEST-DB");
        assert_eq!(
            classify_gate_run(&GateRun {
                has_baseline: false,
                ..run
            }),
            GateStatus::NoTestDb
        );
    }

    #[test]
    fn no_test_db_is_terminal() {
        // Re-running the test stage on the same node or consulting a baseline
        // cannot demote it: the database was unreachable, full stop.
        assert!(GateStatus::NoTestDb.is_terminal());
        let run = no_test_db_run();
        let flipped_baseline = GateRun {
            has_baseline: !run.has_baseline,
            ..run
        };
        assert_eq!(classify_gate_run(&flipped_baseline), GateStatus::NoTestDb);
    }

    #[test]
    fn no_test_db_does_not_admit_to_the_conversion_queue() {
        // No positive test evidence exists, so the patch cannot be admitted —
        // but the non-admission is a non-result, not a counted failure.
        assert!(!GateStatus::NoTestDb.admits_to_conversion_queue());
    }

    #[test]
    fn a_build_or_compile_failure_outranks_no_test_db() {
        // The database may be unreachable, but a patch that does not build, or
        // whose tests do not compile, is a fact about the patch — it wins.
        assert_eq!(
            classify_gate_run(&GateRun {
                build_rc: 101,
                test_db_reachable: false,
                ..no_test_db_run()
            }),
            GateStatus::BuildFailed
        );
        assert_eq!(
            classify_gate_run(&GateRun {
                test_build_failed: true,
                test_db_reachable: false,
                ..no_test_db_run()
            }),
            GateStatus::TestsDoNotCompile
        );
    }

    #[test]
    fn a_reachable_db_leaves_the_existing_classification_intact() {
        // test_rc != 0 with the database reachable is still a real runtime
        // failure (a baseline is present, so it is attributed).
        assert_eq!(
            classify_gate_run(&GateRun {
                test_db_reachable: true,
                ..no_test_db_run()
            }),
            GateStatus::NewTestFailures
        );
    }

    #[test]
    fn the_node_is_recorded_with_the_verdict() {
        let line = render_status_file(&no_test_db_run());
        assert!(line.starts_with("status=NO-TEST-DB"), "{line}");
        assert!(line.contains("node=hive-as-11-2-54"), "{line}");
    }

    #[test]
    fn the_node_round_trips_through_the_status_file() {
        let line = render_status_file(&no_test_db_run());
        let parsed = parse_status_file(&line).expect("parses");
        assert_eq!(parsed.status, GateStatus::NoTestDb);
        assert_eq!(parsed.node, "hive-as-11-2-54");
    }

    #[test]
    fn a_legacy_status_file_without_a_node_parses_with_an_empty_node() {
        let legacy = "status=PASS build_rc=0 test_rc=0 fmt_rc=0\n";
        let parsed = parse_status_file(legacy).expect("parses");
        assert_eq!(parsed.status, GateStatus::Pass);
        assert_eq!(parsed.node, "");
    }

    #[test]
    fn the_parser_rejects_a_duplicate_node_token() {
        assert!(
            parse_status_file("status=PASS node=a node=b build_rc=0 test_rc=0 fmt_rc=0").is_err()
        );
    }

    // --- mislabeled no-baseline green (issue #3798) ----------------------

    /// The exact evidence shape from the incident: a fully green run labelled
    /// `UNKNOWN-NO-BASELINE` because no baseline existed to compare against.
    #[test]
    fn a_mislabeled_no_baseline_green_file_is_effectively_verified_absolute() {
        let text = "status=UNKNOWN-NO-BASELINE build_rc=0 test_rc=0 fmt_rc=0\n";
        let parsed = parse_status_file(text).expect("parses");
        // The written status is preserved; the recorded evidence is what
        // the effective status is derived from.
        assert_eq!(parsed.status, GateStatus::UnknownNoBaseline);
        assert!(parsed.is_mislabeled_no_baseline());
        assert_eq!(parsed.effective_status(), GateStatus::VerifiedAbsolute);
        assert!(parsed.effective_status().admits_to_conversion_queue());
    }

    #[test]
    fn a_genuine_unknown_no_baseline_file_is_not_mislabeled() {
        // The test stage failed at run time with no baseline: the label is
        // what the evidence says, and the run does not admit.
        let text = "status=UNKNOWN-NO-BASELINE build_rc=0 test_rc=101 fmt_rc=0\n";
        let parsed = parse_status_file(text).expect("parses");
        assert!(!parsed.is_mislabeled_no_baseline());
        assert_eq!(parsed.effective_status(), GateStatus::UnknownNoBaseline);
        assert!(!parsed.effective_status().admits_to_conversion_queue());
    }

    #[test]
    fn a_no_baseline_file_with_a_nonzero_fmt_rc_is_not_absolute_green() {
        // Every gate must have returned zero: a failing fmt stage is a
        // recorded negative, not a missing comparison.
        let text = "status=UNKNOWN-NO-BASELINE build_rc=0 test_rc=0 fmt_rc=1\n";
        let parsed = parse_status_file(text).expect("parses");
        assert!(!parsed.is_mislabeled_no_baseline());
        assert_eq!(parsed.effective_status(), GateStatus::UnknownNoBaseline);
    }

    #[test]
    fn files_written_after_the_split_round_trip_their_status() {
        let run = GateRun {
            build_rc: 0,
            test_rc: 0,
            test_build_failed: false,
            fmt_rc: 0,
            has_baseline: false,
            node: "hive-as-11-3-51".to_string(),
            test_db_reachable: true,
        };
        let line = render_status_file(&run);
        assert!(line.starts_with("status=VERIFIED-ABSOLUTE"), "{line}");
        let parsed = parse_status_file(&line).expect("parses");
        assert_eq!(parsed.status, GateStatus::VerifiedAbsolute);
        assert!(!parsed.is_mislabeled_no_baseline());
        assert_eq!(parsed.effective_status(), GateStatus::VerifiedAbsolute);
    }
}
