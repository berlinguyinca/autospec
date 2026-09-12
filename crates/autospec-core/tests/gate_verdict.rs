//! Regression tests for #4434. Each case is a real incident from the session
//! that produced the type, not an invented example.

use autospec_core::gate_verdict::{absolute, differential, GateVerdict, TestRun};
use std::collections::BTreeSet;

fn run(exit: i32, lines: usize, failures: &[&str]) -> TestRun {
    TestRun {
        exit_code: exit,
        result_lines: lines,
        failures: failures
            .iter()
            .map(|s| s.to_string())
            .collect::<BTreeSet<_>>(),
    }
}

/// INCIDENT: patch #4148 failed to compile (19 x E0596). It produced an empty
/// failure set, and a set-difference against a 2-failure baseline read it as
/// "no new failures" and PASSED it.
#[test]
fn a_build_failure_is_not_an_improvement() {
    let baseline = run(101, 1, &["known_a", "known_b"]);
    let did_not_build = run(101, 0, &[]);
    let v = differential(&baseline, &did_not_build);
    assert!(!v.is_pass(), "a patch that never ran must not pass: {v}");
    assert!(
        v.is_unmeasured(),
        "and it must be reported as unmeasured, not as a failure: {v}"
    );
    assert!(
        v.to_string().contains("never ran"),
        "the message must say why, or the next reader repeats the mistake: {v}"
    );
}

/// INCIDENT: the gate loop `continue`d past one crate's suite but left its ok
/// flag set, reporting PASS without testing.
#[test]
fn a_skipped_suite_is_not_a_pass() {
    let skipped = run(0, 0, &[]);
    let v = absolute(&skipped);
    assert!(!v.is_pass(), "exit 0 with no tests run is not a pass: {v}");
    assert!(v.is_unmeasured());
}

/// A candidate that runs FEWER tests than the baseline has not been judged.
/// "No new failures" while measuring less is measuring less, not improving.
#[test]
fn measuring_less_is_not_improving() {
    let baseline = run(101, 175, &["known"]);
    let partial = run(101, 40, &["known"]);
    let v = differential(&baseline, &partial);
    assert!(v.is_unmeasured(), "a partial run must not pass: {v}");
    assert!(v.to_string().contains("fewer tests"), "{v}");
}

#[test]
fn an_unmeasured_baseline_refuses_to_judge() {
    // Comparing against a baseline that never ran is comparing against
    // nothing; the honest answer is "I do not know".
    let v = differential(&run(101, 0, &[]), &run(0, 100, &[]));
    assert!(v.is_unmeasured(), "{v}");
    assert!(v.to_string().contains("BASELINE"), "{v}");
}

/// The real autospec-cli case: 2 pre-existing failures on main, a patch that
/// introduces none must pass.
#[test]
fn inherited_failures_do_not_block_a_clean_patch() {
    let baseline = run(
        101,
        1,
        &["stale_startup_recovery", "integrated_inactive_local_branch"],
    );
    let candidate = run(
        101,
        1,
        &["stale_startup_recovery", "integrated_inactive_local_branch"],
    );
    assert_eq!(differential(&baseline, &candidate), GateVerdict::Pass);
}

#[test]
fn a_genuinely_new_failure_is_named() {
    let baseline = run(101, 1, &["known"]);
    let candidate = run(101, 1, &["known", "brand_new"]);
    match differential(&baseline, &candidate) {
        GateVerdict::Fail { reasons } => {
            assert_eq!(reasons, vec!["brand_new".to_string()]);
        }
        other => panic!("expected Fail naming the new test, got {other}"),
    }
}

#[test]
fn absolute_passes_a_green_suite_and_fails_a_red_one() {
    assert_eq!(absolute(&run(0, 175, &[])), GateVerdict::Pass);
    assert!(!absolute(&run(101, 175, &["boom"])).is_pass());
}

/// Non-zero exit with no named failures is still a failure, not a pass: the
/// harness itself may have died after reporting results.
#[test]
fn non_zero_exit_without_named_failures_still_fails() {
    let v = absolute(&run(101, 10, &[]));
    assert!(!v.is_pass(), "{v}");
    assert!(
        !v.is_unmeasured(),
        "results were observed, so this is a real failure: {v}"
    );
}

/// There is deliberately no `bool` conversion: a caller cannot coerce
/// NotMeasured into success. This test documents that as intent.
#[test]
fn not_measured_never_reads_as_pass() {
    for v in [
        GateVerdict::NotMeasured {
            why: "anything".into(),
        },
        GateVerdict::Fail {
            reasons: vec!["x".into()],
        },
    ] {
        assert!(!v.is_pass(), "{v}");
    }
    assert!(GateVerdict::Pass.is_pass());
}
