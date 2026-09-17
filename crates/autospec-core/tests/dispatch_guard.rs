//! Dispatch guard against unconverted output (#3764).
//!
//! The dispatch path destroys the issue's output directory before re-dispatch
//! — `rm -rf out/issue-N` — and the guard is the only thing standing between
//! that and a finished, unconverted patch. These tests pin the two invariants
//! that make the original failure impossible: the dispatch precondition is
//! "no unconverted output", not "no running process", and a check that cannot
//! answer is unsafe, not clear.

use autospec_core::dispatch_guard::{
    classified_hold, classify_artifact_outcome, decide, ArtifactOutcome, CheckId, CheckOutcome,
    CheckReport, GuardDecision, GuardReason, FAILED_RUN_STATUSES,
};

fn patch_check(outcome: CheckOutcome, evidence: Option<&str>) -> CheckReport {
    match outcome {
        CheckOutcome::Clear => {
            CheckReport::clear(CheckId::UnconvertedPatch, evidence.map(String::from))
        }
        CheckOutcome::Dangerous => {
            CheckReport::dangerous(CheckId::UnconvertedPatch, evidence.map(String::from))
        }
        other => CheckReport {
            check: CheckId::UnconvertedPatch,
            outcome: other,
            evidence: None,
        },
    }
}

fn patch_clear() -> CheckReport {
    patch_check(CheckOutcome::Clear, Some("out/issue-1234/changes.patch"))
}

fn patch_dangerous() -> CheckReport {
    patch_check(
        CheckOutcome::Dangerous,
        Some("out/issue-1234/changes.patch"),
    )
}

fn process_check(outcome: CheckOutcome) -> CheckReport {
    match outcome {
        CheckOutcome::Clear => CheckReport::clear(CheckId::RunningProcess, None),
        CheckOutcome::Dangerous => CheckReport::dangerous(CheckId::RunningProcess, None),
        other => CheckReport {
            check: CheckId::RunningProcess,
            outcome: other,
            evidence: None,
        },
    }
}

fn process_clear() -> CheckReport {
    process_check(CheckOutcome::Clear)
}

fn process_dangerous() -> CheckReport {
    process_check(CheckOutcome::Dangerous)
}

fn patch_failed(detail: &str) -> CheckReport {
    CheckReport::failed(
        CheckId::UnconvertedPatch,
        format!("ssh: Could not resolve hostname hive: {detail}"),
    )
}

fn process_failed(detail: &str) -> CheckReport {
    CheckReport::failed(CheckId::RunningProcess, detail)
}

// ── invariant 1: the precondition is "no unconverted output" ───────────────

#[test]
fn dispatch_authorized_when_no_unconverted_output_and_no_runner() {
    let report = decide("1234", &[patch_clear(), process_clear()]);

    assert_eq!(report.decision, GuardDecision::Dispatch);
    assert!(!report.held());
    assert_eq!(report.issue, "1234");
    assert_eq!(report.checks.len(), 2);
}

#[test]
fn a_finished_runner_leaves_the_patch_behind_and_holds_the_dispatch() {
    // The #3764 shape: the process check (the old precondition) passes, but
    // the runner finished and left its unconverted patch in the output
    // directory. Dispatch must not proceed — the patch would be destroyed.
    let report = decide("1234", &[patch_dangerous(), process_clear()]);

    assert_eq!(
        report.decision,
        GuardDecision::Hold {
            reason: GuardReason::UnconvertedPatch {
                evidence: Some("out/issue-1234/changes.patch".to_string())
            }
        }
    );
    assert!(report.held());
}

#[test]
fn a_running_process_holds_the_dispatch_even_without_a_patch() {
    let report = decide("1234", &[patch_clear(), process_dangerous()]);

    assert_eq!(
        report.decision,
        GuardDecision::Hold {
            reason: GuardReason::RunningProcess
        }
    );
}

#[test]
fn an_unconverted_patch_is_reported_before_a_running_process() {
    // Both dangers present: the patch is the correct precondition and the
    // one that dispatch would destroy, so it is named.
    let report = decide("1234", &[patch_dangerous(), process_dangerous()]);

    assert!(
        matches!(
            report.decision,
            GuardDecision::Hold {
                reason: GuardReason::UnconvertedPatch { .. }
            }
        ),
        "{:?}",
        report.decision
    );
}

// ── invariant 2: a check that cannot answer is unsafe ──────────────────────

#[test]
fn an_errored_patch_check_holds_the_dispatch_fail_closed() {
    // The original bug: the patch check went through an unreachable ssh alias
    // and the error was read as "no file". A check error must instead be an
    // unsafe answer — the dispatch holds.
    let report = decide(
        "1234",
        &[patch_failed("name does not resolve"), process_clear()],
    );

    assert_eq!(
        report.decision,
        GuardDecision::Hold {
            reason: GuardReason::CheckFailed {
                check: CheckId::UnconvertedPatch,
                detail: "ssh: Could not resolve hostname hive: name does not resolve".to_string()
            }
        }
    );
}

#[test]
fn an_errored_process_check_holds_the_dispatch_fail_closed() {
    let report = decide("1234", &[patch_clear(), process_failed("pgrep exited 2")]);

    assert_eq!(
        report.decision,
        GuardDecision::Hold {
            reason: GuardReason::CheckFailed {
                check: CheckId::RunningProcess,
                detail: "pgrep exited 2".to_string()
            }
        }
    );
}

#[test]
fn a_missing_patch_check_cannot_authorize_dispatch() {
    // A guard that only ever ran the process check is the pre-#3764 guard:
    // it cannot verify the precondition, so it cannot authorize.
    let report = decide("1234", &[process_clear()]);

    assert_eq!(
        report.decision,
        GuardDecision::Hold {
            reason: GuardReason::CheckMissing {
                check: CheckId::UnconvertedPatch
            }
        }
    );
}

#[test]
fn a_seen_danger_is_reported_before_a_failed_check() {
    // Evidence first, ambiguity second: the patch was *seen*, so the hold
    // names the patch, not the process check's error.
    let report = decide(
        "1234",
        &[patch_dangerous(), process_failed("pgrep exited 2")],
    );

    assert!(
        matches!(
            report.decision,
            GuardDecision::Hold {
                reason: GuardReason::UnconvertedPatch { .. }
            }
        ),
        "{:?}",
        report.decision
    );
}

// ── reporting ───────────────────────────────────────────────────────────────

#[test]
fn the_line_names_the_issue_the_verdict_and_the_reason() {
    let hold = decide("1234", &[patch_dangerous(), process_clear()]);
    let line = hold.line();
    assert!(line.contains("issue 1234"), "{line}");
    assert!(line.contains("HELD"), "{line}");
    assert!(line.contains("out/issue-1234/changes.patch"), "{line}");
    assert!(line.contains("would destroy"), "{line}");

    let go = decide("1234", &[patch_clear(), process_clear()]);
    let line = go.line();
    assert!(line.contains("authorized"), "{line}");
    assert!(line.contains("unconverted_patch=clear"), "{line}");
    assert!(line.contains("running_process=clear"), "{line}");
}

#[test]
fn a_failed_check_line_carries_the_error_detail() {
    let report = decide("1234", &[patch_failed("name does not resolve")]);
    let line = report.line();
    assert!(line.contains("FAILED") || line.contains("failed"), "{line}");
    assert!(line.contains("name does not resolve"), "{line}");
    assert!(line.contains("unsafe"), "{line}");
}

#[test]
fn the_dry_run_report_lists_every_check_before_the_verdict() {
    let report = decide(
        "1234",
        &[patch_dangerous(), process_failed("pgrep exited 2")],
    );
    let lines = report.lines();

    assert_eq!(lines.len(), 3);
    assert!(lines[0].contains("check unconverted_patch"), "{:?}", lines);
    assert!(lines[0].contains("DANGEROUS"), "{:?}", lines);
    assert!(
        lines[0].contains("out/issue-1234/changes.patch"),
        "{:?}",
        lines
    );
    assert!(lines[1].contains("check running_process"), "{:?}", lines);
    assert!(lines[1].contains("FAILED"), "{:?}", lines);
    assert!(lines[2].contains("HELD"), "{:?}", lines);
}

#[test]
fn the_report_round_trips_through_json() {
    let report = decide("1234", &[patch_dangerous(), process_clear()]);
    let json = report.to_json();

    assert!(json.contains("\"unconverted_patch\""), "{json}");
    assert!(json.contains("\"dangerous\""), "{json}");
    assert!(json.contains("out/issue-1234/changes.patch"), "{json}");

    let back: autospec_core::dispatch_guard::GuardReport =
        serde_json::from_str(&json).expect("guard report deserializes");
    assert_eq!(back, report);
}

// --- #3784: classify_artifact_outcome regression tests ---

#[test]
fn classify_missing_status_is_unrecorded() {
    let outcome = classify_artifact_outcome(None);
    match &outcome {
        ArtifactOutcome::Unrecorded { detail } => {
            assert!(detail.contains("no terminal status"), "detail: {detail}");
        }
        other => panic!("expected Unrecorded, got {other:?}"),
    }
}

#[test]
fn classify_each_failed_status_is_failed_run() {
    for status in FAILED_RUN_STATUSES {
        let outcome = classify_artifact_outcome(Some(status));
        match &outcome {
            ArtifactOutcome::FailedRun { status: s } => {
                assert_eq!(s, status);
            }
            other => panic!("status {status}: expected FailedRun, got {other:?}"),
        }
    }
}

#[test]
fn classify_fmt_dirty_is_convertible_not_archivable() {
    let outcome = classify_artifact_outcome(Some("FMT-DIRTY"));
    assert_eq!(
        outcome,
        ArtifactOutcome::Convertible {
            status: "FMT-DIRTY".to_string(),
        }
    );
}

#[test]
fn classify_unknown_status_defaults_to_convertible() {
    let outcome = classify_artifact_outcome(Some("SOME-NEW-STATUS"));
    assert_eq!(
        outcome,
        ArtifactOutcome::Convertible {
            status: "SOME-NEW-STATUS".to_string(),
        }
    );
}

#[test]
fn classify_pass_is_convertible() {
    let outcome = classify_artifact_outcome(Some("PASS"));
    assert_eq!(
        outcome,
        ArtifactOutcome::Convertible {
            status: "PASS".to_string(),
        }
    );
}

#[test]
fn classify_new_test_failures_is_convertible() {
    let outcome = classify_artifact_outcome(Some("NEW-TEST-FAILURES"));
    assert_eq!(
        outcome,
        ArtifactOutcome::Convertible {
            status: "NEW-TEST-FAILURES".to_string(),
        }
    );
}

#[test]
fn classified_hold_convertible_renders_status() {
    let check = CheckReport::dangerous(
        CheckId::UnconvertedPatch,
        Some("out/issue-42/changes.patch".to_string()),
    );
    let outcome = ArtifactOutcome::Convertible {
        status: "FMT-DIRTY".to_string(),
    };
    let report = classified_hold("42", check, outcome);
    assert!(report.held());
    let line = report.line();
    assert!(line.contains("unconverted patch exists"), "line: {line}");
    assert!(line.contains("FMT-DIRTY"), "line: {line}");
}

#[test]
fn classified_hold_unrecorded_renders_detail() {
    let check = CheckReport::dangerous(
        CheckId::UnconvertedPatch,
        Some("out/issue-99/changes.patch".to_string()),
    );
    let outcome = ArtifactOutcome::Unrecorded {
        detail: "status.txt is missing".to_string(),
    };
    let report = classified_hold("99", check, outcome);
    assert!(report.held());
    let line = report.line();
    assert!(line.contains("unconverted patch exists"), "line: {line}");
    assert!(line.contains("status.txt is missing"), "line: {line}");
}

#[test]
fn classified_hold_failed_run_is_unreachable_but_exhaustive() {
    // The CLI archives before calling classified_hold, so this variant
    // should never be reached in practice. We test it for exhaustiveness.
    let check = CheckReport::dangerous(
        CheckId::UnconvertedPatch,
        Some("out/issue-7/changes.patch".to_string()),
    );
    let outcome = ArtifactOutcome::FailedRun {
        status: "BUILD-FAIL".to_string(),
    };
    let report = classified_hold("7", check, outcome);
    assert!(report.held());
    let line = report.line();
    assert!(line.contains("unconverted patch exists"), "line: {line}");
}

#[test]
fn artifact_outcome_round_trips_through_json() {
    let outcomes = vec![
        ArtifactOutcome::FailedRun {
            status: "TIMEOUT".to_string(),
        },
        ArtifactOutcome::Convertible {
            status: "FMT-DIRTY".to_string(),
        },
        ArtifactOutcome::Unrecorded {
            detail: "no status".to_string(),
        },
    ];
    for outcome in outcomes {
        let json = serde_json::to_string(&outcome).expect("serialize");
        let back: ArtifactOutcome = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, outcome);
    }
}

// ---------------------------------------------------------------------------
// A signalled run is a failed run whatever its label claims (#4651).
// ---------------------------------------------------------------------------

use autospec_core::dispatch_guard::classify_report;
use autospec_core::execution::status_triage::AgentReport;

fn record(status: Option<&str>, agent_rc: Option<i32>, signal: Option<&str>) -> AgentReport {
    AgentReport {
        status: status.map(String::from),
        agent_rc,
        signal: signal.map(String::from),
        ..AgentReport::default()
    }
}

#[test]
fn a_killed_agent_is_a_failed_run_under_a_healthy_label() {
    // iw-87: `UNKNOWN-NO-BASELINE` with agent_rc=143. Read as a label alone,
    // the guard held the artifact as convertible while the conversion pass
    // refused it, so the dispatch slot never freed (#4651).
    assert_eq!(
        classify_report(&record(Some("UNKNOWN-NO-BASELINE"), Some(143), None)),
        ArtifactOutcome::FailedRun {
            status: "SIGNALLED".to_string()
        }
    );
}

#[test]
fn a_signal_with_no_label_at_all_is_still_a_failed_run() {
    // A runner killed before it wrote its verdict leaves only the signal.
    assert_eq!(
        classify_report(&record(None, None, Some("SIGKILL"))),
        ArtifactOutcome::FailedRun {
            status: "SIGNALLED".to_string()
        }
    );
}

#[test]
fn the_runners_own_timeout_is_its_own_failure_not_an_unattributed_kill() {
    // 124 is a known sender: the artifact archives as the timeout it is,
    // because conflating the two would hide every limit expiry inside
    // "something killed it".
    assert_eq!(
        classify_report(&record(Some("TIMEOUT"), Some(124), None)),
        ArtifactOutcome::FailedRun {
            status: "TIMEOUT".to_string()
        }
    );
    // A runner that asserts its own limit fired keeps that name even over an
    // ambiguous code, so the archived artifact is not mislabelled as an
    // unattributed kill.
    assert_eq!(
        classify_report(&record(Some("TIMEOUT"), Some(143), None)),
        ArtifactOutcome::FailedRun {
            status: "TIMEOUT".to_string()
        }
    );
}

#[test]
fn an_ordinary_record_classifies_exactly_as_the_label_alone_did() {
    // The gate must not start seeing kills where it saw none: for any report
    // that names no termination, the report classifier agrees with the status
    // classifier it replaced.
    for status in [
        None,
        Some("VERIFIED"),
        Some("NO-OUTPUT"),
        Some("UNKNOWN-NO-BASELINE"),
        Some("BUILD-FAIL"),
        Some("TIMEOUT-NO-OUTPUT"),
        Some("FMT-DIRTY"),
    ] {
        let report = record(status, Some(0), None);
        assert_eq!(
            classify_report(&report),
            classify_artifact_outcome(report.status.as_deref()),
            "{status:?} changed classification"
        );
    }
}

#[test]
fn signalled_is_a_terminal_failure_the_guard_archives() {
    assert!(FAILED_RUN_STATUSES.contains(&"SIGNALLED"));
}

/// A graded record that carries its own coverage counters (#4665).
fn graded(status: &str, passed: u64, failed: u64, total: Option<u64>) -> AgentReport {
    AgentReport {
        status: Some(status.to_string()),
        test_passed: Some(passed),
        test_failed: Some(failed),
        tests_total: total,
        ..AgentReport::default()
    }
}

#[test]
fn a_verified_over_a_prefix_of_the_suite_is_not_reported_as_verified() {
    // The guard's `status` exists for operator visibility, so it must say what
    // the record's own counters support: 1150 of 10 073 tests is not a verdict
    // on a 10 073-test suite (#4665).
    assert_eq!(
        classify_report(&graded("VERIFIED", 1120, 30, Some(10073))),
        ArtifactOutcome::Convertible {
            status: "PARTIAL-COVERAGE".to_string()
        }
    );
}

#[test]
fn a_run_that_finished_the_suite_is_reported_as_it_claims() {
    assert_eq!(
        classify_report(&graded("VERIFIED", 10073, 0, Some(10073))),
        ArtifactOutcome::Convertible {
            status: "VERIFIED".to_string()
        }
    );
    // No declared total: the shortfall cannot be established, so the record is
    // reported as written rather than demoted on a guess.
    assert_eq!(
        classify_report(&graded("VERIFIED", 1120, 30, None)),
        ArtifactOutcome::Convertible {
            status: "VERIFIED".to_string()
        }
    );
}

#[test]
fn a_short_run_is_not_a_failed_run() {
    // The distinction matters operationally: a failed run is archived and its
    // dispatch slot frees, while a run that never graded the suite has said
    // nothing about the patch at all. Its artifact stays for the pass, which
    // is the measurement the run skipped.
    let outcome = classify_report(&graded("VERIFIED", 1120, 30, Some(10073)));
    assert!(
        !matches!(outcome, ArtifactOutcome::FailedRun { .. }),
        "{outcome:?}"
    );
    // And the label on its own is not one of the failures either.
    assert_eq!(
        classify_artifact_outcome(Some("PARTIAL-COVERAGE")),
        ArtifactOutcome::Convertible {
            status: "PARTIAL-COVERAGE".to_string()
        }
    );
}

#[test]
fn a_kill_still_outranks_a_short_run() {
    // Precedence: #4651's signal wins over #4665's coverage, because a killed
    // process left no graded record at all — there is nothing to measure.
    let mut report = graded("VERIFIED", 1120, 30, Some(10073));
    report.agent_rc = Some(143);
    assert_eq!(
        classify_report(&report),
        ArtifactOutcome::FailedRun {
            status: "SIGNALLED".to_string()
        }
    );
}
