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
//! - a run that reported a **fmt failure, alone** recorded a
//!   deterministic negative the pass can repair: formatting is
//!   deterministic and mechanical, so the pass formats the patch
//!   itself and re-checks it locally before judging. The recorded
//!   `FMT-DIRTY` verdict never short-circuits the local check — the
//!   local result is authoritative;
//! - a run that reported a **build failure** (or a fmt failure
//!   together with one) recorded a deterministic negative no
//!   normalisation can repair — a property of the submission, not of
//!   the run. A local re-run reproduces the same failure; the patch is
//!   held with the agent's own evidence;
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
//!    ([`TriageDecision::Redispatch`]). `status=TIMEOUT` means the agent
//!    never reached the gate. A local gate on an untested patch spends
//!    the gate's cost saying nothing about it; a fresh dispatch is the
//!    only response. The timeout outranks every other signal in the file
//!    — even a `fmt_rc != 0` in the same report.
//! 1a. **A timed-out run with no output is review material, not
//!    re-dispatch material** ([`TriageDecision::RaiseForReview`]).
//!    `status=TIMEOUT-NO-OUTPUT` is the empty-output case: the agent
//!    produced nothing, so re-dispatching the same prompt reproduces the
//!    same emptiness (#3936). It outranks every other signal just as the
//!    plain timeout does, but the response is to raise it for review —
//!    a human or the monitor looks at why nothing came out — not to
//!    silently re-queue it.
//! 2. **Deterministic negatives are trusted; a repairable one is
//!    repaired, not held** ([`TriageDecision::FormatAndRecheck`],
//!    [`TriageDecision::Hold`]). A fmt failure alone (`fmt_rc != 0` or
//!    `status=FMT-DIRTY`, with a green build) formats the patch and
//!    re-checks it locally before judging: a stage that consults the
//!    recorded verdict and runs its own local check for the same
//!    property must not let the recorded verdict win — the local result
//!    is authoritative. The re-check verifies that formatting did not
//!    widen the changed-file set ([`format_scope_check`]) and decides
//!    on the local exit code ([`post_format_decision`]). A fmt failure
//!    together with a build failure, or a build failure alone
//!    (`build_rc != 0`, or `status=BUILD-FAILED` /
//!    `TESTS-DO-NOT-COMPILE`), holds the patch: no formatter repairs a
//!    broken build. A fmt failure holds as
//!    [`AgentHoldReason::Unformatted`] (`AGENT-REPORTED-UNFORMATTED`);
//!    a build failure holds as [`AgentHoldReason::Unbuilt`]
//!    (`AGENT-REPORTED-UNBUILT`). Formatting and compilation are
//!    deterministic: the pass re-runs only what it can repair, and the
//!    agent's report still saves the re-run for the rest.
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

use crate::run_status::Status;

/// Reading the record the runner left behind: the `key=value` / `key: value`
/// parsing that turns a `status.txt` into an [`AgentReport`], separated from the
/// policy that consumes one (#4665).
pub mod record;
pub use record::parse_agent_report;
/// How much of the suite a run actually executed, and what its counters are
/// therefore entitled to claim (#4665).
pub mod coverage;

/// How a decision is written out, separated from the policy that makes it:
/// the rendering grows every time the vocabulary gains a case, the
/// precedence rules rarely move (#4651).
mod render;
/// The termination a recorded exit code describes — the runner's own
/// bounding `timeout`, or a signal from somewhere that has not said who it
/// is (#4651).
pub mod signal;
pub use render::{decision_line, held_line};

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
    /// The signal the runner says ended the agent, if it named one
    /// (`signal=SIGTERM`). A killed run reached no stage, so this outranks
    /// the status label (#4651).
    pub signal: Option<String>,
    /// The agent process's own exit code, if recorded (`agent_rc=143`).
    /// This is the only field that separates the runner's own `timeout`
    /// (124) from a signal sent by something else (128 + N) — the
    /// distinction #4651 lost three runs over.
    pub agent_rc: Option<i32>,
    /// `test_passed`: tests the run saw pass (#4665).
    pub test_passed: Option<u64>,
    /// `test_failed`: tests the run saw fail (#4665).
    pub test_failed: Option<u64>,
    /// The size of the suite the run was measured against, if declared
    /// (`tests_total=10073`). Its absence does not imply a shortfall — it
    /// means coverage cannot be judged (#4665 invariant 4).
    pub tests_total: Option<u64>,
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
            && self.signal.is_none()
            && self.agent_rc.is_none()
            && self.test_passed.is_none()
            && self.test_failed.is_none()
            && self.tests_total.is_none()
    }

    /// How much of the suite this run accounted for (#4665).
    pub fn coverage(&self) -> coverage::Coverage {
        coverage::Coverage::from_counts(self.test_passed, self.test_failed, self.tests_total)
    }
}

/// The responses the pass can make to an agent report (#3715, #4651).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TriageDecision {
    /// Re-dispatch the issue; do not gate locally (rule 1).
    Redispatch {
        /// The status that triggered the re-dispatch (`TIMEOUT`).
        status: String,
    },
    /// Raise the issue for review instead of re-dispatching it (rule 1a).
    RaiseForReview {
        /// The status that triggered the review (`TIMEOUT-NO-OUTPUT`): the
        /// run timed out with no output, so re-dispatch would reproduce the
        /// same emptiness (#3936).
        status: String,
    },
    /// Refuse the patch: the agent was terminated by a signal (rule 1b).
    ///
    /// Not a `Hold`, which asserts a deterministic property of the
    /// submission a killed agent never established; not a `Redispatch`,
    /// which walks into the same supervisor; not a `GateLocally`, which
    /// spends a full gate on a tree whose agent is gone. No verdict about
    /// the patch exists and none can be derived from the record (#4651).
    Signalled {
        /// The signal the run recorded, if it named one.
        signal: Option<String>,
        /// The status the refusal is recorded on (`SIGNALLED`).
        status: String,
    },
    /// Hold the patch on the agent's own deterministic negative (rule 2).
    Hold {
        /// Which deterministic signal held the patch.
        reason: AgentHoldReason,
    },
    /// Format the patch and re-check it locally before judging (rule 2).
    ///
    /// The agent's report recorded a fmt failure — `fmt_rc != 0` or a
    /// `FMT-DIRTY` status — with no build failure in the same report.
    /// The recorded verdict does not short-circuit the local check: the
    /// pass formats the patch, verifies that formatting did not widen
    /// the changed-file set ([`format_scope_check`]), and re-runs the
    /// fmt check; the local result is authoritative
    /// ([`post_format_decision`]). The decision is an action for the
    /// caller, like [`TriageDecision::Redispatch`] — this module
    /// performs no I/O, and the pass runs the formatter.
    FormatAndRecheck,
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
    /// The run's own counters show it never reached the whole suite, so a
    /// green label beside them is not evidence (#4665). This is not a
    /// duplicated gate: the local run *is* the measurement the agent's run
    /// skipped when it stopped at the first failing test binary.
    PartialCoverage {
        /// what the run's counters say it covered.
        coverage: coverage::Coverage,
    },
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
/// Every status name is resolved through
/// [`crate::run_status::canonical_status`] first, so this match names each
/// status once and in its canonical spelling; a legacy spelling means what
/// the vocabulary says it means (#4206). A name the vocabulary does not
/// declare resolves to `None` and is handled by the last rule.
///
/// The precedence is the contract, most decisive evidence first:
///
/// 1. `TIMEOUT` → [`TriageDecision::Redispatch`]. The run never reached
///    the gate, so nothing the pass runs locally can say about the
///    patch. The timeout outranks every other signal in the report.
/// 1a. `TIMEOUT-NO-OUTPUT` → [`TriageDecision::RaiseForReview`]. The
///    empty-output case (#3936): re-dispatching the same prompt
///    reproduces the same emptiness, so the run is raised for review
///    rather than re-queued. It outranks every other signal just as the
///    plain timeout does.
/// 1b. A **signalled agent** → [`TriageDecision::Signalled`] (#4651). The
///    label `SIGNALLED`, a recorded `signal=`, or an `agent_rc` that decodes
///    to `128 + N` all mean the process was killed rather than finished, and
///    the exit code outranks the label for the same reason `test_rc` does
///    (#4206): a runner that dies before writing its verdict leaves only the
///    code. It outranks the stage fields because a killed run reached no
///    stage. `UNKNOWN-NO-BASELINE` is not a substitute: it is a claim about
///    the *baseline*, and a killed agent never had one.
///
///    Rule 1 still outranks this one, and that is the same principle rather
///    than an exception: a `TIMEOUT` label is the runner *asserting* its own
///    limit fired, and an assertion outranks an inference. The exit code
///    cannot settle it either way — a harness that reports the child's
///    death-signal records `143` where `timeout` itself would exit `124` —
///    which is why #4651 asks the runner to say whether its own timeout
///    fired instead of leaving the reader to guess from `128 + N`.
/// 2. `fmt_rc != 0` or `FMT-DIRTY`, with no build failure in the same
///    report → [`TriageDecision::FormatAndRecheck`]. The recorded
///    verdict does not short-circuit the local check: the pass formats
///    the patch, verifies the changed-file set did not widen
///    ([`format_scope_check`]), and re-checks locally; the local result
///    is authoritative ([`post_format_decision`]). The status counts on
///    its own because a fleet run that stops at formatting records the
///    name and no `fmt_rc`. A fmt failure together with a build
///    failure → [`TriageDecision::Hold`] as
///    [`AgentHoldReason::Unformatted`]: a formatter cannot repair a
///    broken build, and the fmt evidence leads the hold line.
/// 3. `build_rc != 0` or `BUILD-FAIL` → [`TriageDecision::Hold`] as
///    [`AgentHoldReason::Unbuilt`]. Deterministic; same reason. The
///    legacy spellings `BUILD-FAILED` and `TESTS-DO-NOT-COMPILE` reach
///    this rule through the vocabulary, not through a second literal.
/// 4. `NO-OUTPUT` / `NO-TEST-DB` → [`TriageDecision::RaiseForReview`].
///    Nothing was measured, so neither re-dispatch nor a local gate can
///    say anything the harness did not already say.
/// 5. `UNKNOWN-NO-BASELINE` → [`TriageDecision::GateLocally`] as
///    [`GateBasis::NoBaseline`]. The converter's real job is to
///    compare against the current main, and there is no baseline to do
///    it with until this pass runs.
/// 6. `test_rc != 0`, `NEW-TEST-FAILURES` or `TEST-TIMEOUT` →
///    [`TriageDecision::GateLocally`] as
///    [`GateBasis::AgentReportedTestFailure`]. Tests are flaky and the
///    agent ran against its own base; re-verify, never hold. The exit
///    code outranks the label: a report that says `VERIFIED` and carries
///    `test_rc=101` is triaged on the code, not the word (#4206).
/// 6b. A green label over a run that did not finish the suite →
///    [`TriageDecision::GateLocally`] as
///    [`GateBasis::PartialCoverage`] (#4665). `test_passed + test_failed`
///    short of the declared suite size means the run covered a prefix of it,
///    and a prefix cannot certify the whole: `cargo test` aborts at the first
///    failing test binary, so the label is computed from whatever ran before
///    the abort. Only a claim of *passing* is downgraded — an observed failure
///    is still an observed failure.
/// 7. Everything else — green, or a status this triage does not
///    consume — → [`TriageDecision::GateLocally`] as
///    [`GateBasis::AgentGreen`]. The agent's result was against the
///    base the agent ran on; confirming against the current main is the
///    pass's job.
///
/// An empty report (nothing recorded) is not a green report: it gates
/// locally as [`GateBasis::ReportUnreadable`] (rule 6 of the module).
///
/// The last match is exhaustive over [`Status`], so adding a status to
/// the vocabulary without saying what triage does with it is a compile
/// error rather than a rule that quietly stops matching (#4206).
pub fn triage(report: &AgentReport) -> TriageDecision {
    if report.is_empty() {
        return TriageDecision::GateLocally {
            basis: GateBasis::ReportUnreadable {
                detail: "status file is empty".to_string(),
            },
        };
    }
    // Resolve the recorded name through the shared vocabulary once. Every
    // rule below compares against a canonical `Status`, never against a
    // spelling this file invented (#4206: a match list written beside the
    // vocabulary, rather than against it, silently stops matching).
    let recorded = report.status.as_deref();
    let canonical = recorded.and_then(crate::run_status::canonical_status);
    // 1. The timeout outranks everything: the run never reached the
    //    gate, so no local gate can speak about the patch. A plain
    //    timeout re-dispatches; a timeout with no output raises for
    //    review instead, because re-dispatch would reproduce the same
    //    emptiness (#3936).
    if canonical == Some(Status::Timeout) {
        return TriageDecision::Redispatch {
            status: Status::Timeout.as_str().to_string(),
        };
    }
    if canonical == Some(Status::TimeoutNoOutput) {
        return TriageDecision::RaiseForReview {
            status: Status::TimeoutNoOutput.as_str().to_string(),
        };
    }
    // 1b. A killed agent outranks every stage field in the same record,
    //     exactly as a timeout does: the process was terminated, so an
    //     `fmt_rc` beside it is debris from a half-finished edit, not a
    //     verdict. `iw-87` died mid-edit with `agent_rc=143 fmt_rc=1
    //     test_rc=101` and triaged as a repairable fmt failure — the pass
    //     formatted 2000 lines of half-applied change and offered it to the
    //     conversion queue (#4651).
    if signal::is_signalled(recorded, report.signal.as_deref(), report.agent_rc) {
        return TriageDecision::Signalled {
            signal: report.signal.clone(),
            status: Status::Signalled.as_str().to_string(),
        };
    }
    // 2. fmt: a deterministic negative the pass can repair. Trust the
    //    agent's exit code, and trust the name when the run recorded
    //    only the name. The recorded verdict does not short-circuit the
    //    local check: the pass formats and re-checks locally before
    //    judging, and the local result is authoritative (#4099). A fmt
    //    failure together with a build failure in the same report is
    //    not repairable by a formatter — it holds, with the fmt
    //    evidence first, as before.
    let fmt_failed =
        matches!(report.fmt_rc, Some(rc) if rc != 0) || canonical == Some(Status::FmtDirty);
    let build_failed = matches!(report.build_rc, Some(rc) if rc != 0)
        || matches!(canonical, Some(Status::BuildFail));
    if fmt_failed {
        return if build_failed {
            TriageDecision::Hold {
                reason: AgentHoldReason::Unformatted,
            }
        } else {
            TriageDecision::FormatAndRecheck
        };
    }
    // 3. build: a deterministic negative no normalisation repairs; trust
    //    the agent's.
    if build_failed {
        return TriageDecision::Hold {
            reason: AgentHoldReason::Unbuilt,
        };
    }
    // 4-7. The label decides, and the match is exhaustive over the
    //      vocabulary so a new status cannot be added without saying what
    //      triage does with it. The failing `test_rc` is folded into the
    //      green arm, so a `VERIFIED` written over a failing run is
    //      triaged on the failure and not the word (#4206: 183 reports,
    //      every one `status=VERIFIED test_rc=101`), while a status that
    //      already explains itself (`UNKNOWN-NO-BASELINE`) keeps the
    //      precedence its own rule has.
    let test_rc_failed = matches!(report.test_rc, Some(rc) if rc != 0);
    match canonical {
        // 1-3. Unreachable by construction: the same `canonical` value was
        //      matched by the early returns above. The arm exists so this
        //      match stays exhaustive over `Status`.
        Some(
            Status::Timeout
            | Status::TimeoutNoOutput
            | Status::Signalled
            | Status::FmtDirty
            | Status::BuildFail,
        ) => {
            unreachable!("the timeout, signal, fmt and build rules above return first")
        }
        // 4. Nothing was measured: no gate, local or remote, has evidence
        //    to work with, and re-dispatch reproduces the emptiness.
        Some(Status::NoOutput | Status::NoTestDb) => TriageDecision::RaiseForReview {
            status: recorded.unwrap_or_default().to_string(),
        },
        // 5. No baseline: the converter's real job is to supply one.
        Some(Status::UnknownNoBaseline) => TriageDecision::GateLocally {
            basis: GateBasis::NoBaseline,
        },
        // 6. Test negatives: a trigger, never a hold.
        Some(Status::NewTestFailures | Status::TestTimeout) => TriageDecision::GateLocally {
            basis: GateBasis::AgentReportedTestFailure {
                status: report.status.clone(),
            },
        },
        // 6a. Green by label, failing by exit code: the code wins.
        Some(Status::Verified) | None if test_rc_failed => TriageDecision::GateLocally {
            basis: GateBasis::AgentReportedTestFailure {
                status: report.status.clone(),
            },
        },
        // 7. Green, or a status this triage does not consume: confirm against
        //    the current main — unless the run's own counters show it stopped
        //    short of the suite, which the basis states rather than leaving the
        //    reader to divide out of two numbers (#4665: `test_passed=1120
        //    test_failed=30` of 10 073, labelled `VERIFIED`). A short run can
        //    prove a failure and cannot prove a pass, so this routes to the
        //    local gate rather than to a hold or a rejection.
        Some(Status::Verified | Status::PartialCoverage) | None => TriageDecision::GateLocally {
            basis: green_basis(report, canonical),
        },
    }
}

/// The basis reported for a run that reached its gate and claims it passed.
///
/// A green claim over a prefix of the suite is not green (#4665), and a record
/// that *declares* partial coverage without carrying counters still disclaims
/// its own pass: the claim is the finding, and the missing counts are detail on
/// top of it.
fn green_basis(report: &AgentReport, canonical: Option<Status>) -> GateBasis {
    let coverage = report.coverage();
    match coverage {
        coverage::Coverage::Partial { .. } => GateBasis::PartialCoverage { coverage },
        other if canonical == Some(Status::PartialCoverage) => {
            GateBasis::PartialCoverage { coverage: other }
        }
        _ => GateBasis::AgentGreen,
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

/// What the local fmt re-check means after the pass formatted the patch
/// ([`TriageDecision::FormatAndRecheck`], #4099).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PostFormatDecision {
    /// The local fmt check passed and formatting did not widen the
    /// changed-file set: proceed to the gate. The local result outranks
    /// the recorded `FMT-DIRTY` verdict.
    Gate,
    /// The local fmt check still fails after the pass formatted the
    /// patch: hold it. The formatter did not clean the tree, so this is
    /// not a formatting defect the pass can repair.
    HoldStillDirty,
    /// The local fmt check passes, but the formatter touched files the
    /// patch never changed: hold it, naming the widened files.
    HoldWidened {
        /// The files the formatter touched that were not in the patch's
        /// changed set.
        widened: Vec<String>,
    },
}

/// Decide what the local fmt re-check means after the pass formatted
/// the patch (#4099). The local result is authoritative: a recorded
/// `FMT-DIRTY` verdict never holds a patch the local check clears, and
/// a local check that still fails holds a patch the recorded verdict
/// would have passed.
///
/// `local_fmt_rc` is the exit code of the local fmt check the pass ran
/// after formatting; `widened` is the widened-file list from
/// [`format_scope_check`] (empty when the changed-file set did not
/// widen).
pub fn post_format_decision(local_fmt_rc: i32, widened: Vec<String>) -> PostFormatDecision {
    if local_fmt_rc != 0 {
        return PostFormatDecision::HoldStillDirty;
    }
    if widened.is_empty() {
        PostFormatDecision::Gate
    } else {
        PostFormatDecision::HoldWidened { widened }
    }
}

/// Verify that formatting did not widen the changed-file set (#4099).
///
/// `changed_before` is the patch's changed files as recorded before the
/// pass formatted; `changed_after` is the changed files after. The
/// check passes when every file touched after formatting was already
/// touched before — `changed_after ⊆ changed_before`. It fails with the
/// files the formatter introduced (sorted, deduplicated), so the hold
/// line can name them. Order and duplicates do not matter: this is a
/// set inclusion.
pub fn format_scope_check(
    changed_before: &[String],
    changed_after: &[String],
) -> Result<(), Vec<String>> {
    use std::collections::BTreeSet;
    let before: BTreeSet<&String> = changed_before.iter().collect();
    let widened: BTreeSet<String> = changed_after
        .iter()
        .filter(|path| !before.contains(path))
        .cloned()
        .collect();
    if widened.is_empty() {
        Ok(())
    } else {
        Err(widened.into_iter().collect())
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
            ..AgentReport::default()
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
    }

    #[test]
    fn a_timeout_with_no_output_raises_for_review() {
        assert_eq!(
            triage(&report(Some("TIMEOUT-NO-OUTPUT"), None, None, None, None)),
            TriageDecision::RaiseForReview {
                status: "TIMEOUT-NO-OUTPUT".to_string()
            }
        );
    }

    #[test]
    fn a_timeout_with_no_output_outranks_every_other_signal() {
        let noisy = report(
            Some("TIMEOUT-NO-OUTPUT"),
            Some(1),
            Some(1),
            Some(1),
            Some(32),
        );
        assert_eq!(
            triage(&noisy),
            TriageDecision::RaiseForReview {
                status: "TIMEOUT-NO-OUTPUT".to_string()
            }
        );
    }

    #[test]
    fn the_raise_for_review_decision_line_names_the_status_and_reason() {
        let report = report(Some("TIMEOUT-NO-OUTPUT"), None, None, None, None);
        let decision = triage(&report);
        let line = decision_line(&decision, &report);
        assert!(line.starts_with("RAISE-FOR-REVIEW:"), "{line}");
        assert!(line.contains("status=TIMEOUT-NO-OUTPUT"), "{line}");
        assert!(line.contains("no output"), "{line}");
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
    fn a_fmt_only_failure_formats_and_rechecks() {
        // The recorded verdict does not short-circuit the local check:
        // the pass formats and re-checks before judging (#4099).
        assert_eq!(
            triage(&report(None, Some(0), Some(0), Some(1), Some(32))),
            TriageDecision::FormatAndRecheck
        );
        // The status alone says the same thing.
        assert_eq!(
            triage(&report(Some("FMT-DIRTY"), Some(0), Some(0), None, None)),
            TriageDecision::FormatAndRecheck
        );
    }

    #[test]
    fn a_fmt_failure_with_a_build_failure_still_holds() {
        // A formatter cannot repair a broken build: the same report
        // holds, with the fmt evidence first, as before.
        assert_eq!(
            triage(&report(None, Some(101), Some(0), Some(1), Some(32))),
            TriageDecision::Hold {
                reason: AgentHoldReason::Unformatted
            }
        );
        assert_eq!(
            triage(&report(Some("FMT-DIRTY"), Some(101), None, Some(1), None)),
            TriageDecision::Hold {
                reason: AgentHoldReason::Unformatted
            }
        );
    }

    #[test]
    fn a_recorded_fmt_dirty_verdict_yields_to_the_local_recheck() {
        // AC3 (a): the run recorded FMT-DIRTY and the local re-check
        // passes without widening the changed-file set — the local
        // result is authoritative; the patch goes to the gate.
        let report = report(Some("FMT-DIRTY"), Some(0), Some(0), Some(1), Some(32));
        assert_eq!(triage(&report), TriageDecision::FormatAndRecheck);
        assert!(format_scope_check(&["src/a.rs".into()], &["src/a.rs".into()]).is_ok());
        assert_eq!(
            post_format_decision(0, Vec::new()),
            PostFormatDecision::Gate
        );
    }

    #[test]
    fn a_recorded_fmt_dirty_verdict_holds_when_formatting_widens() {
        // AC3 (b): the run recorded FMT-DIRTY and the local re-check
        // passes, but the formatter touched a file the patch never
        // changed — the patch is held, naming the widened file.
        let report = report(Some("FMT-DIRTY"), Some(0), Some(0), Some(1), Some(32));
        assert_eq!(triage(&report), TriageDecision::FormatAndRecheck);
        let widened = format_scope_check(
            &["src/a.rs".into()],
            &["src/a.rs".into(), "src/b.rs".into()],
        )
        .unwrap_err();
        assert_eq!(widened, vec!["src/b.rs".to_string()]);
        assert_eq!(
            post_format_decision(0, widened),
            PostFormatDecision::HoldWidened {
                widened: vec!["src/b.rs".to_string()]
            }
        );
    }

    #[test]
    fn a_local_fmt_failure_after_formatting_holds_still_dirty() {
        // The formatter did not clean the tree: the local re-check
        // still fails, and the patch is held.
        assert_eq!(
            post_format_decision(1, Vec::new()),
            PostFormatDecision::HoldStillDirty
        );
        // A still-dirty tree holds even if it also widened — the
        // formatter failure is the primary defect.
        assert_eq!(
            post_format_decision(1, vec!["src/b.rs".to_string()]),
            PostFormatDecision::HoldStillDirty
        );
    }

    #[test]
    fn the_format_scope_check_is_a_set_inclusion() {
        let before = vec!["src/b.rs".to_string(), "src/a.rs".to_string()];
        // A subset in any order passes.
        assert!(format_scope_check(&before, &["src/a.rs".to_string()]).is_ok());
        // Duplicates on either side do not matter.
        assert!(format_scope_check(
            &before,
            &[
                "src/a.rs".to_string(),
                "src/a.rs".to_string(),
                "src/b.rs".to_string()
            ]
        )
        .is_ok());
        // Any file the formatter introduced fails, sorted and deduplicated.
        let widened = format_scope_check(
            &before,
            &["src/c.rs".into(), "src/c.rs".into(), "src/a.rs".into()],
        )
        .unwrap_err();
        assert_eq!(widened, vec!["src/c.rs".to_string()]);
        let widened =
            format_scope_check(&before, &["src/d.rs".into(), "src/c.rs".into()]).unwrap_err();
        assert_eq!(
            widened,
            vec!["src/c.rs".to_string(), "src/d.rs".to_string()]
        );
        // An empty after-set passes: formatting can only narrow.
        assert!(format_scope_check(&before, &[]).is_ok());
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

        let fmt_dirty = report(Some("FMT-DIRTY"), Some(0), Some(0), Some(1), Some(32));
        assert_eq!(
            decision_line(&TriageDecision::FormatAndRecheck, &fmt_dirty),
            "FORMAT-AND-RECHECK: agent reported fmt_rc=1 (32 files), status=FMT-DIRTY; formatting and re-checking locally before judging"
        );
        assert_eq!(
            decision_line(
                &TriageDecision::FormatAndRecheck,
                &report(None, Some(0), Some(0), Some(1), None)
            ),
            "FORMAT-AND-RECHECK: agent reported fmt_rc=1; formatting and re-checking locally before judging"
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
