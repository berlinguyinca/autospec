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
//! 3. **A hold's class comes from exit codes, its detail from output (issue
//!    #3747).** The same gate also *held* patches that compile: it grepped
//!    `^error:` in the captured output to decide "build error", but cargo
//!    writes test-run failures behind the very same prefix
//!    (`error: test failed, to rerun pass ...`). A patch whose build stage
//!    exited 0 and whose tests then failed was therefore held as a build
//!    error, and the hold named no failing test at all. [`classify_hold`] and
//!    [`hold_for_run`] pick the class from the stage exit codes and use the
//!    output only for detail; [`compile_failure_line`] and
//!    [`test_failure_line`] are the discriminating shapes for the text that
//!    detail is pulled from. Every test hold carries the failing test names
//!    ([`failing_test_names`]), and when the output yielded none the hold says
//!    so in words instead of falling back to a stage label the exit codes do
//!    not support ([`GateHold::line`]).
//!
//!    The test stage's verdict compares **failing test names against the
//!    recorded baseline** ([`compare_tests`]), the way the validate gate does
//!    (#3715, #3727) — never a failing count against zero, which reads an
//!    absent baseline as a clean one and calls an equal-count, entirely
//!    different failure set unchanged. And the trunk revision written next to
//!    a hold is the revision the gate actually tested: a sha captured *before*
//!    the per-patch fetch is one merge behind and is never recorded as `base=`
//!    ([`BaseRevision`]).
//!
//! 4. **Contradictory signals are flagged at write time.** A status file that
//!    claims `build_rc=0` while recording that the test stage failed to build
//!    is self-contradictory — it is the signature of a gate whose build stage
//!    never saw the test targets. [`render_status_file`] and
//!    [`write_status_file`] mark such files with a `contradiction=` token at
//!    write time, and [`parse_status_file`] surfaces the flag (re-deriving it
//!    for files written before the flag existed) so downstream consumers see
//!    the `build_rc=0` claim for what it is.

use std::collections::BTreeSet;
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
    /// attribute the failures to. Holds only when the tests actually built.
    UnknownNoBaseline,
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
    /// conversion queue. `TESTS-DO-NOT-COMPILE`, `UNKNOWN-NO-BASELINE`,
    /// `NEW-TEST-FAILURES`, and `BUILD-FAILED` never do.
    pub const fn admits_to_conversion_queue(self) -> bool {
        matches!(self, Self::Pass)
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
/// 5. otherwise — `PASS`.
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
    GateStatus::Pass
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

// ---------------------------------------------------------------------------
// Issue #3747: exit codes classify a hold, output only names it
// ---------------------------------------------------------------------------

/// Which stage of the gate failed.
///
/// Decided by exit codes and nothing else. Output is evidence *about* the
/// stage the exit codes already condemned; it never condemns a stage the
/// exit codes report green, because cargo writes test-run failures behind the
/// same `error:` prefix it writes compile failures behind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailedStage {
    /// The build stage exited nonzero.
    Build,
    /// The build stage exited 0 and the test stage exited nonzero.
    Test,
}

/// The stage whose exit code says the run failed; `None` when both are green.
pub fn failed_stage(build_rc: i32, test_rc: i32) -> Option<FailedStage> {
    if build_rc != 0 {
        Some(FailedStage::Build)
    } else if test_rc != 0 {
        Some(FailedStage::Test)
    } else {
        None
    }
}

/// A cargo diagnostic line that names a **compile** failure.
///
/// Two shapes, anchored at the start of the line (cargo indents
/// sub-diagnostics, so leading whitespace is trimmed):
///
/// * `error[E0425]: cannot find function ... in this scope` — a rustc
///   diagnostic, recognised by its uppercase error code;
/// * `error: could not compile <target> ...` — cargo's own compile summary.
///
/// A bare `error:` prefix is deliberately **not** a match: see
/// [`test_failure_line`] for the test-run failures cargo writes behind it.
pub fn compile_failure_line(line: &str) -> bool {
    let Some(rest) = line.trim_start().strip_prefix("error") else {
        return false;
    };
    if let Some(code) = rest.strip_prefix('[') {
        return code.chars().next().is_some_and(|c| c.is_ascii_uppercase());
    }
    rest.strip_prefix(": could not compile")
        .is_some_and(|tail| tail.is_empty() || tail.starts_with(' '))
}

/// A cargo diagnostic line that names a **test-run** failure.
///
/// * `error: test failed, to rerun pass ...` — a test binary reported a
///   failure;
/// * `error: 1 target failed:` / `error: 2 targets failed, ...` — cargo's
///   summary over failing test targets.
///
/// These are the lines that made a `^error:` grep hold a patch that compiles:
/// they arrive with `build_rc=0`.
pub fn test_failure_line(line: &str) -> bool {
    let Some(rest) = line.trim_start().strip_prefix("error: ") else {
        return false;
    };
    if rest == "test failed" || rest.starts_with("test failed ") || rest.starts_with("test failed,")
    {
        return true;
    }
    targets_failed_summary(rest)
}

/// The `<n> target(s) failed` shape of cargo's test-stage summary.
fn targets_failed_summary(rest: &str) -> bool {
    let digits = rest.len() - rest.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    if digits == 0 {
        return false;
    }
    let after = &rest[digits..];
    after.starts_with(" target failed") || after.starts_with(" targets failed")
}

/// Whether any line of captured output names a compile failure.
pub fn output_reports_compile_failure(output: &str) -> bool {
    output.lines().any(compile_failure_line)
}

/// Whether any line of captured output names a test-run failure.
pub fn output_reports_test_failure(output: &str) -> bool {
    output.lines().any(test_failure_line)
}

/// The failing test names in cargo test output, deduplicated and sorted.
///
/// Two sources, both present on a failing run:
///
/// * the progress lines, `test <path> ... FAILED`;
/// * the `failures:` block, whose indented entries repeat those paths (and
///   occasionally add ones the progress lines scrolled).
///
/// An empty result means the output truly named nothing — a killed test
/// binary, a truncated log. It is reported as such by [`GateHold::line`],
/// never reinterpreted as a build failure.
pub fn failing_test_names(output: &str) -> Vec<String> {
    let mut names = BTreeSet::new();
    let mut in_failures_block = false;
    for line in output.lines() {
        if line.trim_end() == "failures:" {
            in_failures_block = true;
            continue;
        }
        if let Some(name) = progress_failed_name(line) {
            names.insert(name);
            continue;
        }
        if !in_failures_block {
            continue;
        }
        let entry = line.trim();
        if entry.is_empty() {
            continue;
        }
        if !line.starts_with(char::is_whitespace) {
            // Back to column 0: the block is over.
            in_failures_block = false;
            continue;
        }
        if is_test_path(entry) {
            names.insert(entry.to_string());
        }
    }
    names.into_iter().collect()
}

/// `test <path> ... FAILED` — the progress line, without its status.
fn progress_failed_name(line: &str) -> Option<String> {
    let rest = line.trim_start().strip_prefix("test ")?;
    let (name, status) = rest.rsplit_once(" ... ")?;
    let status = status.trim();
    (!name.is_empty() && status == "FAILED").then(|| name.to_string())
}

/// Whether an indented `failures:` entry looks like a test path rather than
/// the prose cargo prints between blocks.
fn is_test_path(candidate: &str) -> bool {
    !candidate.is_empty()
        && !candidate.contains(char::is_whitespace)
        && candidate
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '_' | ':' | '.' | '-'))
}

/// Why the gate held a patch (#3747).
///
/// The three classes are the three things the exit-code evidence can say.
/// A hold never carries a class the exit codes do not support, and a test
/// hold never degrades into a count.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateHold {
    /// The **build stage** exited nonzero. `detail` is the compile diagnostic
    /// when the output carried one, the exit code when it did not.
    Build {
        /// The diagnostic that explains the failing build stage.
        detail: String,
    },
    /// The test stage failed to *build* (#3748): nothing ran, so there are no
    /// failing test names to report and no baseline that could attribute them.
    TestsDoNotCompile {
        /// The compile diagnostic naming the target that did not build.
        detail: String,
    },
    /// The build stage was green and the test stage exited nonzero. Tests
    /// ran; these are the ones that did not pass.
    Tests {
        /// The failing test names, empty only when the output named none.
        failing_tests: Vec<String>,
        /// The test stage's exit code, kept so a nameless hold is still
        /// traceable to the run that produced it.
        test_rc: i32,
    },
}

impl GateHold {
    /// The machine-readable class, for status files and logs.
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Build { .. } => "build_error",
            Self::TestsDoNotCompile { .. } => "tests_do_not_compile",
            Self::Tests { .. } => "test_failure",
        }
    }

    /// The failing test names this hold records, if it has any.
    pub fn failing_tests(&self) -> &[String] {
        match self {
            Self::Tests { failing_tests, .. } => failing_tests,
            _ => &[],
        }
    }

    /// The one line the pass records for the hold.
    ///
    /// A test hold always states the failing tests by name, and says in words
    /// when the output yielded none. It never says "build error" unless the
    /// build stage itself failed.
    pub fn line(&self) -> String {
        match self {
            Self::Build { detail } => format!("HELD: build error -- {detail}"),
            Self::TestsDoNotCompile { detail } => format!("HELD: tests do not compile -- {detail}"),
            Self::Tests {
                failing_tests,
                test_rc: _,
            } if !failing_tests.is_empty() => format!(
                "HELD: test failure -- {} failing tests: {}",
                failing_tests.len(),
                failing_tests.join(", ")
            ),
            Self::Tests { test_rc, .. } => {
                format!("HELD: test failure -- failing test names unavailable (test_rc={test_rc})")
            }
        }
    }
}

/// Classify one gate run into the hold the pass records.
///
/// The **exit codes** pick the class ([`failed_stage`]); the output only
/// supplies the detail. A line that merely starts with `error:` is not read
/// as a build failure — that grep is what held compiling patches as "build
/// error" (#3747). Returns `None` when both stages exited 0: there is nothing
/// to hold.
pub fn classify_hold(build_rc: i32, test_rc: i32, output: &str) -> Option<GateHold> {
    match failed_stage(build_rc, test_rc) {
        None => None,
        Some(FailedStage::Build) => Some(GateHold::Build {
            detail: compile_detail(output)
                .unwrap_or_else(|| format!("build stage exit code {build_rc}")),
        }),
        Some(FailedStage::Test) => Some(GateHold::Tests {
            failing_tests: failing_test_names(output),
            test_rc,
        }),
    }
}

/// The hold for a gate run, given the output its stages produced.
///
/// `test_build_failed` outranks the exit codes, exactly as it outranks them
/// in [`classify_gate_run`]: a test stage that failed to build produced no
/// test results, so its hold is `tests do not compile`, not a test failure
/// with an empty name list.
pub fn hold_for_run(run: &GateRun, output: &str) -> Option<GateHold> {
    if run.test_build_failed {
        return Some(GateHold::TestsDoNotCompile {
            detail: compile_detail(output).unwrap_or_else(|| {
                format!("test targets did not compile (test_rc={})", run.test_rc)
            }),
        });
    }
    classify_hold(run.build_rc, run.test_rc, output)
}

/// The first compile diagnostic in the output, trimmed.
fn compile_detail(output: &str) -> Option<String> {
    output
        .lines()
        .find(|line| compile_failure_line(line))
        .map(str::trim)
        .map(str::to_string)
}

/// The comparison between one patched run's failing tests and the recorded
/// baseline (#3747).
///
/// Comparison is **by name, never by count**. A count compared against zero
/// makes an absent baseline look like a clean one, and an equal count makes an
/// entirely different failure set look unchanged. `baseline: None` means no
/// baseline was recorded — the comparison never happened, and the run fails
/// closed to a hold rather than to "no new failures".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TestComparison {
    /// The test stage named no failing test.
    Passed,
    /// Every failing name is already in the baseline: the patch introduced
    /// nothing new. Per #3727 this is a re-verify trigger, not a verdict, and
    /// it does not hold the patch on its own.
    PreExisting {
        /// The failing names, all of them in the baseline.
        failing: Vec<String>,
    },
    /// Names the baseline does not carry. These are what hold the patch, and
    /// they are named individually.
    NewFailures {
        /// Failing names absent from the baseline.
        new: Vec<String>,
        /// Failing names the baseline already carried.
        pre_existing: Vec<String>,
    },
    /// No baseline was recorded, so nothing was attributed.
    NoBaseline {
        /// The failing names, held on the strength of having no comparison.
        failing: Vec<String>,
    },
}

impl TestComparison {
    /// Whether the comparison holds the patch. `PRE-EXISTING` and `PASSED` do
    /// not; new failures and the absence of a baseline do.
    pub fn holds(&self) -> bool {
        matches!(self, Self::NewFailures { .. } | Self::NoBaseline { .. })
    }

    /// The failing test names that hold the patch, empty when it is not held.
    pub fn blocking_tests(&self) -> &[String] {
        match self {
            Self::NewFailures { new, .. } | Self::NoBaseline { failing: new } => new,
            _ => &[],
        }
    }

    /// The one line recording the comparison, always naming the tests it
    /// compared.
    pub fn line(&self) -> String {
        match self {
            Self::Passed => "tests: no failures".to_string(),
            Self::PreExisting { failing } => format!(
                "tests: {} failing, all pre-existing in baseline: {}",
                failing.len(),
                failing.join(", ")
            ),
            Self::NewFailures { new, pre_existing } => {
                let new_part = format!("tests: {} new failing: {}", new.len(), new.join(", "));
                if pre_existing.is_empty() {
                    new_part
                } else {
                    format!(
                        "{} ({} pre-existing: {})",
                        new_part,
                        pre_existing.len(),
                        pre_existing.join(", ")
                    )
                }
            }
            Self::NoBaseline { failing } => format!(
                "tests: {} failing, no baseline recorded: {}",
                failing.len(),
                failing.join(", ")
            ),
        }
    }
}

/// Compare a patched run's failing test names against the baseline (#3747).
///
/// `baseline` is `Some` only when one was actually recorded; an empty
/// baseline is a *recorded baseline with nothing failing*, and every failing
/// name against it is new. That distinction is the whole of the count-vs-zero
/// bug.
pub fn compare_tests(failing: &[String], baseline: Option<&[String]>) -> TestComparison {
    let failing: BTreeSet<String> = failing.iter().cloned().collect();
    if failing.is_empty() {
        return TestComparison::Passed;
    }
    let Some(baseline) = baseline else {
        return TestComparison::NoBaseline {
            failing: failing.into_iter().collect(),
        };
    };
    let baseline: BTreeSet<String> = baseline.iter().cloned().collect();
    let mut new = Vec::new();
    let mut pre_existing = Vec::new();
    for name in failing {
        if baseline.contains(&name) {
            pre_existing.push(name);
        } else {
            new.push(name);
        }
    }
    if new.is_empty() {
        TestComparison::PreExisting {
            failing: pre_existing,
        }
    } else {
        TestComparison::NewFailures { new, pre_existing }
    }
}

/// The trunk revision a gate verdict is about (#3747).
///
/// The pass fetches trunk before each patch. A sha captured *before* that
/// fetch names a trunk the gate never tested — typically one merge behind —
/// and a hold that records it as `base=` misattributes later failures to a
/// revision that was not under review. Such a sha is kept for diagnostics
/// under `pre_fetch_sha=` and never as the tested base.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BaseRevision {
    /// Captured after the fetch: it names the revision the gate tested.
    Current {
        /// The fetched trunk sha.
        sha: String,
    },
    /// Captured before the fetch: not the base the run tested against.
    PreFetch {
        /// The stale sha, kept for diagnostics only.
        sha: String,
    },
    /// Nothing was captured.
    Unknown,
}

impl BaseRevision {
    /// Record a captured sha against whether the fetch had happened. An empty
    /// sha records nothing rather than recording a blank base.
    pub fn capture(sha: &str, fetch_completed: bool) -> Self {
        let sha = sha.trim();
        if sha.is_empty() {
            return Self::Unknown;
        }
        if fetch_completed {
            Self::Current {
                sha: sha.to_string(),
            }
        } else {
            Self::PreFetch {
                sha: sha.to_string(),
            }
        }
    }

    /// The sha a verdict may call its base: a post-fetch capture only.
    pub fn tested_base(&self) -> Option<&str> {
        match self {
            Self::Current { sha } => Some(sha),
            Self::PreFetch { .. } | Self::Unknown => None,
        }
    }

    /// The `base=` field for the run's log line. A stale capture is recorded
    /// as unrecorded, with the stale sha kept alongside for diagnosis.
    pub fn field(&self) -> String {
        match self {
            Self::Current { sha } => format!("base={sha}"),
            Self::PreFetch { sha } => format!("base=unrecorded pre_fetch_sha={sha}"),
            Self::Unknown => "base=unknown".to_string(),
        }
    }
}

/// The hold and the revision it is about, as one logged record.
pub fn hold_log_line(hold: &GateHold, base: &BaseRevision) -> String {
    format!("{} {}", hold.line(), base.field())
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
    fn a_green_run_passes() {
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

    // --- conversion queue admission ----------------------------------------

    #[test]
    fn patches_whose_tests_do_not_compile_never_reach_the_conversion_queue() {
        assert!(!GateStatus::TestsDoNotCompile.admits_to_conversion_queue());
        assert!(!GateStatus::UnknownNoBaseline.admits_to_conversion_queue());
        assert!(!GateStatus::NewTestFailures.admits_to_conversion_queue());
        assert!(!GateStatus::BuildFailed.admits_to_conversion_queue());
        assert!(GateStatus::Pass.admits_to_conversion_queue());
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

    // --- hold classification (issue #3747) ---------------------------------

    /// The #3747 evidence: the build stage exited 0, the test stage exited
    /// 101, and the output's last line starts with `error:`.
    const TEST_FAILURE_OUTPUT: &str = "\
test core::tests::parses_input ... ok
test core::tests::rejects_negative ... FAILED

failures:

core::tests::rejects_negative

---- core::tests::rejects_negative stdout ----
thread 'core::tests::rejects_negative' panicked at src/core.rs:42:9

failures:
    core::tests::rejects_negative

test result: FAILED. 12 passed; 1 failed; 0 ignored

error: test failed, to rerun pass `-p core --lib`
";

    const COMPILE_OUTPUT: &str = "\
   Compiling autospec-core v0.1.0 (/repo/crates/autospec-core)
error[E0425]: cannot find function `classify_gate_run` in this scope
  --> crates/autospec-core/src/conversion_gate.rs:12:9
error: could not compile `autospec-core` (lib) due to 1 previous error
";

    #[test]
    fn exit_codes_pick_the_failing_stage() {
        assert_eq!(failed_stage(0, 0), None);
        assert_eq!(failed_stage(101, 101), Some(FailedStage::Build));
        assert_eq!(failed_stage(0, 101), Some(FailedStage::Test));
    }

    #[test]
    fn a_test_failure_is_never_held_as_a_build_error() {
        // The #3747 incident: build green, tests red, output ending in a
        // line that starts with `error:`.
        let hold = classify_hold(0, 101, TEST_FAILURE_OUTPUT).expect("test stage failed");
        assert_eq!(hold.kind(), "test_failure");
        let line = hold.line();
        assert!(!line.contains("build error"), "{line}");
        assert!(line.contains("test failure"), "{line}");
        assert!(line.contains("core::tests::rejects_negative"), "{line}");
    }

    #[test]
    fn a_compile_failure_is_held_as_one_with_its_diagnostic() {
        let hold = classify_hold(101, 101, COMPILE_OUTPUT).expect("build stage failed");
        assert_eq!(hold.kind(), "build_error");
        let line = hold.line();
        assert!(
            line.contains("error[E0425]: cannot find function"),
            "{line}"
        );
    }

    #[test]
    fn discriminating_patterns_separate_the_two_error_shapes() {
        assert!(compile_failure_line("error[E0425]: cannot find function x"));
        assert!(compile_failure_line(
            "error: could not compile `autospec-core` (lib) due to 1 previous error"
        ));
        assert!(test_failure_line(
            "error: test failed, to rerun pass `-p core --lib`"
        ));
        assert!(test_failure_line("error: 1 target failed:"));
        assert!(test_failure_line("error: 2 targets failed, passing 3"));

        // The bare prefix both shapes share classifies as neither.
        assert!(!compile_failure_line("error: test failed, to rerun pass"));
        assert!(!test_failure_line("error[E0425]: cannot find function x"));
        assert!(!compile_failure_line("error: something else entirely"));
        assert!(!test_failure_line("error: something else entirely"));
        assert!(!compile_failure_line("errorish"));
        assert!(!compile_failure_line("warning: unused import"));
    }

    #[test]
    fn output_reports_each_failure_shape_independently() {
        assert!(output_reports_test_failure(TEST_FAILURE_OUTPUT));
        assert!(!output_reports_compile_failure(TEST_FAILURE_OUTPUT));
        assert!(output_reports_compile_failure(COMPILE_OUTPUT));
        assert!(!output_reports_test_failure(COMPILE_OUTPUT));
    }

    #[test]
    fn failing_test_names_are_deduplicated_and_sorted() {
        let names = failing_test_names(TEST_FAILURE_OUTPUT);
        assert_eq!(names, vec!["core::tests::rejects_negative".to_string()]);
    }

    #[test]
    fn failing_test_names_collect_progress_and_failures_block() {
        let output = "\
test a::one ... FAILED
test a::two ... ok
test b::three ... FAILED

failures:
    b::three
    c::four

test result: FAILED. 1 passed; 2 failed

error: test failed, to rerun pass `--lib`
";
        assert_eq!(
            failing_test_names(output),
            vec![
                "a::one".to_string(),
                "b::three".to_string(),
                "c::four".to_string()
            ]
        );
    }

    #[test]
    fn a_green_run_holds_nothing() {
        assert_eq!(classify_hold(0, 0, TEST_FAILURE_OUTPUT), None);
        assert_eq!(classify_hold(0, 0, ""), None);
    }

    #[test]
    fn a_nameless_test_hold_says_so_instead_of_blaming_the_build() {
        // A killed test binary leaves the exit code and no names at all.
        let hold = classify_hold(0, 101, "error: test failed, to rerun pass").expect("held");
        assert_eq!(hold.kind(), "test_failure");
        assert!(hold.failing_tests().is_empty());
        let line = hold.line();
        assert!(line.contains("failing test names unavailable"), "{line}");
        assert!(line.contains("test_rc=101"), "{line}");
        assert!(!line.contains("build error"), "{line}");
    }

    #[test]
    fn a_build_hold_without_output_names_its_exit_code() {
        let hold = classify_hold(101, 0, "").expect("held");
        assert_eq!(hold.kind(), "build_error");
        assert!(hold.line().contains("build stage exit code 101"));
    }

    #[test]
    fn the_run_level_hold_keeps_tests_that_do_not_compile_distinct() {
        // #3748's evidence seen through #3747: the compile diagnostics are in
        // the *test* stage, so the hold is not a build error and not a test
        // failure with an empty name list.
        let hold = hold_for_run(&incident_run(), COMPILE_OUTPUT).expect("held");
        assert_eq!(hold.kind(), "tests_do_not_compile");
        let line = hold.line();
        assert!(
            line.starts_with("HELD: tests do not compile -- error[E0425]"),
            "{line}"
        );
        assert!(!line.contains("build error"), "{line}");
    }

    #[test]
    fn every_test_hold_names_its_failing_tests() {
        // Acceptance: whichever branch produced the hold, the tests are named.
        let via_rc = classify_hold(0, 101, TEST_FAILURE_OUTPUT).expect("held");
        let via_run = hold_for_run(
            &GateRun {
                build_rc: 0,
                test_rc: 101,
                test_build_failed: false,
                fmt_rc: 0,
                has_baseline: true,
                node: "hive-as-11-2-54".to_string(),
                test_db_reachable: true,
            },
            TEST_FAILURE_OUTPUT,
        )
        .expect("held");
        assert_eq!(via_rc, via_run);
        for line in [via_rc.line(), via_run.line()] {
            assert!(line.contains("core::tests::rejects_negative"), "{line}");
        }
    }

    // --- baseline comparison by name (issue #3747) -------------------------

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|n| n.to_string()).collect()
    }

    #[test]
    fn an_equal_count_with_different_names_is_still_a_new_failure() {
        // The count-vs-zero comparison saw 1 == 1 and called the run clean.
        let baseline = names(&["core::tests::flaky_socket"]);
        let comparison = compare_tests(&names(&["core::tests::parses_input"]), Some(&baseline));
        assert_eq!(
            comparison,
            TestComparison::NewFailures {
                new: names(&["core::tests::parses_input"]),
                pre_existing: vec![],
            }
        );
        assert!(comparison.holds());
        assert!(comparison.line().contains("core::tests::parses_input"));
    }

    #[test]
    fn failures_already_in_the_baseline_do_not_hold() {
        let baseline = names(&["core::tests::flaky_socket", "core::tests::slow_db"]);
        let comparison = compare_tests(&names(&["core::tests::flaky_socket"]), Some(&baseline));
        assert_eq!(
            comparison,
            TestComparison::PreExisting {
                failing: names(&["core::tests::flaky_socket"]),
            }
        );
        assert!(!comparison.holds());
        assert!(comparison.blocking_tests().is_empty());
    }

    #[test]
    fn a_partially_overlapping_set_holds_only_its_new_names() {
        let comparison =
            compare_tests(&names(&["a::known", "b::new"]), Some(&names(&["a::known"])));
        assert_eq!(
            comparison,
            TestComparison::NewFailures {
                new: names(&["b::new"]),
                pre_existing: names(&["a::known"]),
            }
        );
        assert_eq!(comparison.blocking_tests(), &["b::new".to_string()]);
        let line = comparison.line();
        assert!(line.contains("1 new failing: b::new"), "{line}");
        assert!(line.contains("1 pre-existing: a::known"), "{line}");
    }

    #[test]
    fn an_empty_baseline_is_a_baseline_and_every_failure_is_new() {
        // The count-vs-zero bug's other half: a recorded baseline of nothing
        // failing is not the same as no baseline at all.
        let comparison = compare_tests(&names(&["a::one"]), Some(&vec![]));
        assert_eq!(
            comparison,
            TestComparison::NewFailures {
                new: names(&["a::one"]),
                pre_existing: vec![],
            }
        );
    }

    #[test]
    fn no_baseline_holds_and_names_the_tests_it_could_not_attribute() {
        let comparison = compare_tests(&names(&["a::one", "b::two"]), None);
        assert_eq!(
            comparison,
            TestComparison::NoBaseline {
                failing: names(&["a::one", "b::two"]),
            }
        );
        assert!(comparison.holds());
        let line = comparison.line();
        assert!(line.contains("no baseline recorded"), "{line}");
        assert!(line.contains("a::one, b::two"), "{line}");
    }

    #[test]
    fn a_run_that_named_no_failures_passed() {
        assert_eq!(compare_tests(&[], None), TestComparison::Passed);
        assert_eq!(
            compare_tests(&[], Some(&names(&["a::one"]))),
            TestComparison::Passed
        );
        assert!(!TestComparison::Passed.holds());
    }

    // --- the base sha a hold is about (issue #3747) ------------------------

    #[test]
    fn a_base_sha_captured_after_the_fetch_is_the_tested_base() {
        let base = BaseRevision::capture("abc1234", true);
        assert_eq!(base.tested_base(), Some("abc1234"));
        assert_eq!(base.field(), "base=abc1234");
    }

    #[test]
    fn a_base_sha_captured_before_the_fetch_is_never_the_base() {
        // The #3747 staleness: `mainsha` read before the per-patch fetch is
        // one merge behind the trunk the gate actually tested.
        let base = BaseRevision::capture("abc1234", false);
        assert_eq!(base.tested_base(), None);
        let field = base.field();
        assert!(field.starts_with("base=unrecorded"), "{field}");
        assert!(field.contains("pre_fetch_sha=abc1234"), "{field}");
    }

    #[test]
    fn an_empty_sha_records_no_base_at_all() {
        let base = BaseRevision::capture("  ", true);
        assert_eq!(base, BaseRevision::Unknown);
        assert_eq!(base.tested_base(), None);
        assert_eq!(base.field(), "base=unknown");
    }

    #[test]
    fn the_hold_record_carries_names_and_a_fresh_base() {
        let hold = classify_hold(0, 101, TEST_FAILURE_OUTPUT).expect("held");
        let fresh = hold_log_line(&hold, &BaseRevision::capture("def5678", true));
        assert!(
            fresh.starts_with(
                "HELD: test failure -- 1 failing tests: core::tests::rejects_negative base=def5678"
            ),
            "{fresh}"
        );

        let stale = hold_log_line(&hold, &BaseRevision::capture("abc1234", false));
        assert!(stale.contains("core::tests::rejects_negative"), "{stale}");
        assert!(!stale.contains("base=abc1234"), "{stale}");
    }
}
