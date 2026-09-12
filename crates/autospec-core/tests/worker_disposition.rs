//! Worker-disposition contract (issue #4423).
//!
//! Five workers were cancelled simultaneously and the gateway recorded all
//! five as **crashed** — `"worker crashed (no walltime deadline known)"` on
//! jobs whose `sacct` rows said `CANCELLED by 298907`. The acceptance
//! scenarios:
//!
//! * a vanished worker is classified from scheduler terminal evidence
//!   (`sacct State`), and a cause is never reported when it has not been
//!   established (`sacct` unavailable -> "worker disappeared; cause
//!   unknown", never a crash);
//! * every cancellation logs who/what/why to the shared cancellation file
//!   before the call;
//! * telemetry keeps `cancelled` apart from `crashed`, so the difference
//!   is visible without reading logs.

use autospec_core::worker_disposition::{classify, CancelNotice, Disposition, DispositionCounters};

// -- Acceptance 1: classify from scheduler terminal evidence, and never
//    report a cause that has not been established.

#[test]
fn a_cancelled_job_is_not_a_crash() {
    // The incident row from issue #4423: `CANCELLED by 298907`.
    assert_eq!(
        classify(Some("CANCELLED by 298907")),
        Disposition::Cancelled {
            by_user: Some("298907".to_owned())
        }
    );
    // The bare state is also a cancellation.
    assert_eq!(
        classify(Some("CANCELLED")),
        Disposition::Cancelled { by_user: None }
    );
}

#[test]
fn a_cancellation_and_a_crash_are_opposite_dispositions() {
    // The gateway recorded the incident jobs as crashes; sacct said they
    // were cancelled. The two must stay distinguishable.
    assert_ne!(
        classify(Some("CANCELLED")),
        classify(Some("FAILED")),
        "a cancellation is a deliberate action, a fault is not the same thing"
    );
    assert_eq!(classify(Some("FAILED")), Disposition::Crashed);
    assert_eq!(classify(Some("OUT_OF_MEMORY")), Disposition::Crashed);
    assert_eq!(classify(Some("NODE_FAIL")), Disposition::Crashed);
    assert_eq!(classify(Some("BOOT_FAIL")), Disposition::Crashed);
    assert_eq!(classify(Some("TIMEOUT")), Disposition::Crashed);
}

#[test]
fn no_evidence_is_unknown_never_a_crash() {
    // The gateway's gap: it logged "worker crashed (no walltime deadline
    // known)". Absent evidence the cause is unknown, not a crash.
    assert_eq!(classify(None), Disposition::Unknown);
    assert_eq!(classify(Some("")), Disposition::Unknown);
    assert_eq!(classify(Some("   ")), Disposition::Unknown);
}

#[test]
fn an_unrecognised_state_does_not_guess_a_cause() {
    // A state we cannot classify is not evidence of a crash; report the
    // cause as unestablished. (`CANCELLED_BY_ME` would be a cancellation:
    // the classifier keys on the leading `CANCELLED` state.)
    assert_eq!(classify(Some("RESIZING")), Disposition::Unknown);
    assert_eq!(classify(Some("PENDING")), Disposition::Unknown);
}

#[test]
fn completed_is_the_walltime_deadline_reached_normally() {
    assert_eq!(classify(Some("COMPLETED")), Disposition::Completed);
    assert_eq!(classify(Some("completed")), Disposition::Completed);
}

#[test]
fn the_display_never_names_a_cause_it_has_not_established() {
    let message = Disposition::Unknown.to_string();
    // The exact wording the issue demands when sacct cannot answer.
    assert_eq!(message, "worker disappeared; cause unknown");
    assert!(!message.contains("crash"), "{message}");
}

// -- Acceptance 2: every cancellation logs who/what/why before the call.

#[test]
fn a_cancel_notice_names_who_what_and_why_in_one_line() {
    let notice = CancelNotice {
        job_id: 23026707,
        origin: "reconcile-workers.sh".to_owned(),
        reason: "surplus over declared want".to_owned(),
    };
    let line = notice.log_line();
    // who, what, why all present, so a later reader can attribute the
    // action without guessing.
    assert!(line.contains("reconcile-workers.sh"), "{line}");
    assert!(line.contains("23026707"), "{line}");
    assert!(line.contains("surplus over declared want"), "{line}");
    // The origin is the caller's own name, and the action is named as a
    // cancellation.
    assert!(line.contains("scancel"), "{line}");
    assert!(line.contains("cancels"), "{line}");
}

// -- Acceptance 3: telemetry distinguishes cancelled from crashed.

#[test]
fn counters_keep_cancelled_apart_from_crashed() {
    let mut counters = DispositionCounters::default();
    counters.record(&classify(Some("CANCELLED by 298907")));
    counters.record(&classify(Some("CANCELLED")));
    counters.record(&classify(Some("FAILED")));
    counters.record(&classify(Some("COMPLETED")));
    counters.record(&classify(None));

    assert_eq!(counters.cancelled, 2);
    assert_eq!(counters.crashed, 1);
    assert_eq!(counters.completed, 1);
    assert_eq!(counters.unknown, 1);

    let line = counters.line();
    // The difference is visible on one line without reading logs.
    assert!(line.contains("cancelled=2"), "{line}");
    assert!(line.contains("crashed=1"), "{line}");
    assert!(line.contains("completed=1"), "{line}");
    assert!(line.contains("unknown=1"), "{line}");
}

#[test]
fn an_empty_counter_renders_zeros_not_absence() {
    let counters = DispositionCounters::default();
    let line = counters.line();
    assert!(line.contains("cancelled=0"), "{line}");
    assert!(line.contains("crashed=0"), "{line}");
}
