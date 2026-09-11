//! Regression tests for the merge-gate invariants (issue #4307).
//!
//! Incident configuration: the `rust-suites` workflow runs six jobs —
//! `build-test`, `main-builds`, `macos-test`, `windows-test`, `freebsd-test`
//! and `audit`. The converter's local gate covers three of them on a single
//! Linux host. A macOS-gated test file (#4306) with five compile errors sat
//! merged and hidden, the merge record named no unchecked job, and the
//! workflow's pass rate on `main` was 0/40 — a red gate read as a green one.
//!
//! A full-coverage, green-base, 40/40 control cannot see the defect: the
//! incident tests below are the ones that must fail if any of the four
//! invariants regresses.

use autospec_core::merge_gate::{
    decide, record_names_unchecked, BaseBranchStatus, CiStatus, Coverage, GateResult,
    MergeDecision, PassRate,
};

fn workflow_jobs() -> Vec<String> {
    [
        "build-test",
        "main-builds",
        "macos-test",
        "windows-test",
        "freebsd-test",
        "audit",
    ]
    .into_iter()
    .map(String::from)
    .collect()
}

fn local_gate_jobs() -> Vec<String> {
    ["build-test", "main-builds", "audit"]
        .into_iter()
        .map(String::from)
        .collect()
}

fn incident_coverage() -> Coverage {
    Coverage::new(&workflow_jobs(), &local_gate_jobs())
}

#[test]
fn incident_coverage_names_the_three_unchecked_jobs() {
    let coverage = incident_coverage();
    assert_eq!(
        coverage.unchecked,
        vec![
            "macos-test".to_string(),
            "windows-test".to_string(),
            "freebsd-test".to_string()
        ],
        "workflow order is preserved for the unchecked gap"
    );
    assert!(!coverage.complete());
    assert_eq!(
        coverage.line_with_unchecked(),
        "local gate covers 3/6 CI jobs; unchecked: macos-test, windows-test, freebsd-test"
    );
}

#[test]
fn incident_record_that_names_nothing_is_refused() {
    let coverage = incident_coverage();
    // The record the converter actually wrote: the gate passed, no gap named.
    assert!(
        !record_names_unchecked("local gate passed (fmt, build, clippy, test)", &coverage),
        "a record silent about the gap is refused, not defaulted"
    );
    assert!(
        !record_names_unchecked("local gate passed; macos-test also green", &coverage),
        "naming one of the three is not naming all three"
    );
}

#[test]
fn incident_record_that_names_every_gap_passes() {
    let coverage = incident_coverage();
    assert!(record_names_unchecked(
        "local gate covers 3/6 CI jobs; unchecked: macos-test, windows-test, freebsd-test",
        &coverage
    ));
}

#[test]
fn control_full_coverage_needs_no_unchecked_names() {
    let coverage = Coverage::new(&workflow_jobs(), &workflow_jobs());
    assert!(coverage.complete());
    assert_eq!(
        coverage.line_with_unchecked(),
        "local gate covers 6/6 CI jobs"
    );
    // With no gap, an empty record is fine.
    assert!(record_names_unchecked("merged", &coverage));
}

#[test]
fn incident_local_pass_never_approves_alone() {
    // Invariant 1: the local gate is a pre-filter, not the gate.
    assert_eq!(
        decide(
            GateResult::Passed,
            CiStatus::Pending,
            BaseBranchStatus::Green
        ),
        MergeDecision::CiNotPassed {
            status: CiStatus::Pending
        }
    );
    assert_eq!(
        decide(
            GateResult::Passed,
            CiStatus::Failed,
            BaseBranchStatus::Green
        ),
        MergeDecision::CiNotPassed {
            status: CiStatus::Failed
        }
    );
    assert_eq!(
        decide(
            GateResult::Passed,
            CiStatus::NotRun,
            BaseBranchStatus::Green
        ),
        MergeDecision::CiNotPassed {
            status: CiStatus::NotRun
        }
    );
}

#[test]
fn incident_red_base_refuses_even_with_green_everywhere_else() {
    // Invariant 3: rust-suites was 0/40 on main while the converter merged.
    let failing = workflow_jobs();
    let base = BaseBranchStatus::Red {
        failing_jobs: failing.clone(),
    };
    let decision = decide(GateResult::Passed, CiStatus::Passed, base);
    assert_eq!(
        decision,
        MergeDecision::BaseBranchRed {
            failing_jobs: failing
        }
    );
    assert!(!decision.approved());
    assert!(
        decision.line().contains("red on base branch"),
        "the refusal line names the red base: {}",
        decision.line()
    );
}

#[test]
fn local_gate_failure_is_a_prefilter_rejection_not_a_ci_verdict() {
    let decision = decide(
        GateResult::Failed,
        CiStatus::Passed,
        BaseBranchStatus::Green,
    );
    assert_eq!(decision, MergeDecision::LocalGateFailed);
    assert!(!decision.approved());
    assert!(
        decision.line().contains("pre-filter"),
        "the local result is recorded as a pre-filter, never as the gate: {}",
        decision.line()
    );
}

#[test]
fn control_all_green_approves() {
    let decision = decide(
        GateResult::Passed,
        CiStatus::Passed,
        BaseBranchStatus::Green,
    );
    assert_eq!(decision, MergeDecision::Approved);
    assert!(decision.approved());
}

#[test]
fn incident_pass_rate_zero_over_forty_is_flagged_and_surfaced() {
    // Invariant 4: the 0/40 the monitor never printed.
    let mut rate = PassRate::new("rust-suites", "main");
    for _ in 0..40 {
        rate.record(false);
    }
    assert!(rate.all_failing());
    assert_eq!(rate.line(), "rust-suites on main: 0/40 passed (0.0%)");
}

#[test]
fn control_pass_rate_all_green_is_not_flagged() {
    let mut rate = PassRate::new("rust-suites", "main");
    for _ in 0..40 {
        rate.record(true);
    }
    assert!(!rate.all_failing());
    assert_eq!(rate.line(), "rust-suites on main: 40/40 passed (100.0%)");
    assert!((rate.ratio() - 1.0).abs() < f64::EPSILON);
}

#[test]
fn pass_rate_with_no_runs_is_not_all_failing() {
    let rate = PassRate::new("rust-suites", "main");
    assert!(!rate.all_failing(), "an empty ledger is unknown, not red");
    assert_eq!(rate.ratio(), 0.0);
}
