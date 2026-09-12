//! A measurement must assert that it measured (issue #4462).
//!
//! Each test is one of the four measurement bugs from the incident,
//! reproduced in miniature and shown to be refused: a stale file is not a
//! fresh response, a broken counter is not a total failure, a killed gate
//! is not zero failures, and a single flat sample is not a wedge.

use autospec_core::gate_verdict::GateVerdict;
use autospec_core::measurement_assertion::{
    capture_verdict, marked_run, tally_contradiction, CaptureEvidence, CaptureVerdict,
    PersistenceGate, Tally, TallyArityError,
};

// --- 1. clean-file capture: a stale file is not a fresh response ----------

#[test]
fn a_file_left_by_the_previous_iteration_is_stale_not_fresh() {
    // The probe for the new model timed out and wrote nothing, so the file
    // on disk is the PREVIOUS model's response with an mtime from before
    // this invocation.
    let evidence = CaptureEvidence {
        existed_before: true,
        present: true,
        mtime_after_start: Some(false),
    };
    assert_eq!(capture_verdict(&evidence), CaptureVerdict::Stale);
}

#[test]
fn a_request_that_wrote_nothing_is_missing_not_a_previous_response() {
    // http=000 on a fresh path: there is nothing to parse.
    let evidence = CaptureEvidence {
        existed_before: false,
        present: false,
        mtime_after_start: None,
    };
    assert_eq!(capture_verdict(&evidence), CaptureVerdict::Missing);
}

#[test]
fn removal_before_the_invocation_proves_freshness_without_an_mtime() {
    let evidence = CaptureEvidence {
        existed_before: false,
        present: true,
        mtime_after_start: None,
    };
    assert_eq!(capture_verdict(&evidence), CaptureVerdict::Fresh);
}

#[test]
fn an_mtime_strictly_after_start_proves_a_rewritten_file_fresh() {
    let evidence = CaptureEvidence {
        existed_before: true,
        present: true,
        mtime_after_start: Some(true),
    };
    assert_eq!(capture_verdict(&evidence), CaptureVerdict::Fresh);
}

#[test]
fn an_unreadable_mtime_is_stale_not_fresh() {
    // Fail-closed: the freshness proof is missing, so the file is treated
    // as the previous iteration's.
    let evidence = CaptureEvidence {
        existed_before: true,
        present: true,
        mtime_after_start: None,
    };
    assert_eq!(capture_verdict(&evidence), CaptureVerdict::Stale);
}

// --- 2. arity-checked tally: a broken counter is not a total failure ------

#[test]
fn a_tally_that_cannot_account_for_every_attempt_is_rejected() {
    // The per-request status reads failed, so both counters undercount:
    // 0 + 0 != 10. Printing `ok=0 failed=0` for ten attempts is not a
    // measurement.
    let err = Tally::new(10, 0, 0).unwrap_err();
    assert_eq!(
        err,
        TallyArityError {
            attempted: 10,
            ok: 0,
            failed: 0
        }
    );
}

#[test]
fn a_complete_tally_is_accepted_and_renders_its_arity() {
    let tally = Tally::new(10, 7, 3).unwrap();
    assert_eq!(tally.line(), "ok=7 failed=3 (attempted=10)");
}

#[test]
fn two_tallies_over_the_same_set_that_disagree_are_a_contradiction() {
    // The incident: `ok=0 failed=10` from the broken status-file reads
    // beside `non-empty content: 10/10` from the bodies, in the same
    // output. Both pass the arity check; only the cross-field check
    // catches it.
    let from_status_files = Tally::new(10, 0, 10).unwrap();
    let from_bodies = Tally::new(10, 10, 0).unwrap();
    assert!(tally_contradiction(&from_status_files, &from_bodies));
    // Symmetric: the contradiction is in the pair, not in an ordering.
    assert!(tally_contradiction(&from_bodies, &from_status_files));
}

#[test]
fn agreeing_tallies_and_disjoint_sets_are_not_contradictions() {
    let a = Tally::new(10, 7, 3).unwrap();
    let a_again = Tally::new(10, 7, 3).unwrap();
    assert!(!tally_contradiction(&a, &a_again));
    // Different attempted sets: nothing to compare.
    let other = Tally::new(4, 4, 0).unwrap();
    assert!(!tally_contradiction(&a, &other));
    // Zero-attempt tallies: no sample to contradict.
    let none_a = Tally::new(0, 0, 0).unwrap();
    let none_b = Tally::new(0, 0, 0).unwrap();
    assert!(!tally_contradiction(&none_a, &none_b));
}

// --- 3. completion-marked run: a killed gate is not zero failures ---------

#[test]
fn a_killed_run_with_zero_failures_is_unmeasured_not_pass() {
    // The background run was terminated when its tool call timed out,
    // leaving `suites=0 FAILED=0`. The counters say pass; the missing
    // marker says nothing was measured.
    let verdict = marked_run(false, true);
    assert!(verdict.is_unmeasured());
    assert!(!verdict.is_pass());
    // The rendered line must not be readable as a pass.
    let line = verdict.to_string();
    assert!(line.starts_with("NOT MEASURED"), "rendered: {line}");
    assert!(
        line.contains("completion marker missing"),
        "rendered: {line}"
    );
}

#[test]
fn a_completed_run_keeps_its_pass_or_fail_verdict() {
    assert!(marked_run(true, true).is_pass());
    assert!(matches!(marked_run(true, false), GateVerdict::Fail { .. }));
}

// --- 4. persistence-gated verdict: one window is never sufficient ---------

#[test]
fn a_single_sample_can_never_gate_a_verdict() {
    // `required < 2` would let one window decide, which is the defect.
    assert!(PersistenceGate::new(0).is_none());
    assert!(PersistenceGate::new(1).is_none());
    assert!(PersistenceGate::new(2).is_some());
}

#[test]
fn a_single_flat_sample_is_not_a_wedge() {
    // One window showed the worker flat; the next showed decode advancing
    // by 890 tokens and a direct generation returning in seconds.
    let mut gate = PersistenceGate::new(2).unwrap();
    assert!(!gate.record(true));
    assert!(!gate.is_actionable());
    assert_eq!(gate.streak(), 1);
    assert_eq!(gate.required(), 2);
    assert_eq!(gate.line(), "1 of 2 consecutive samples -- not actionable");
}

#[test]
fn persistence_across_consecutive_samples_makes_the_verdict_actionable() {
    let mut gate = PersistenceGate::new(3).unwrap();
    assert!(!gate.record(true));
    assert!(!gate.record(true));
    assert!(gate.record(true));
    assert!(gate.is_actionable());
    assert_eq!(gate.line(), "3 of 3 consecutive samples -- actionable");
}

#[test]
fn one_sample_against_the_verdict_resets_the_streak() {
    let mut gate = PersistenceGate::new(3).unwrap();
    assert!(!gate.record(true));
    assert!(!gate.record(true));
    assert!(!gate.record(false));
    assert_eq!(gate.streak(), 0);
    assert!(!gate.is_actionable());
    // The streak rebuilds from zero, not from where it broke.
    assert!(!gate.record(true));
    assert!(!gate.is_actionable());
}
