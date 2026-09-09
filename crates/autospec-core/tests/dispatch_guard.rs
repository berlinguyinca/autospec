//! Dispatch guard against unconverted output (#3764).
//!
//! The dispatch path destroys the issue's output directory before re-dispatch
//! — `rm -rf out/issue-N` — and the guard is the only thing standing between
//! that and a finished, unconverted patch. These tests pin the two invariants
//! that make the original failure impossible: the dispatch precondition is
//! "no unconverted output", not "no running process", and a check that cannot
//! answer is unsafe, not clear.

use autospec_core::dispatch_guard::{
    decide, CheckId, CheckOutcome, CheckReport, GuardDecision, GuardReason,
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
