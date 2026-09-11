//! The dispatcher re-checks the live tracker at dispatch time (issue #3774).
//!
//! The regression tests run in the configuration the latent defect
//! required: an issue closed on the tracker without producing a patch,
//! sitting in the dispatcher's worklist, passing every existing
//! enqueue-time guard. Before the fix, such an issue was dispatched
//! again on every cycle — the queue never converged, and the only guard
//! that happened to cover it (the `changes.patch` guard) was checking a
//! different property for a different reason.

use autospec_core::dispatch_recheck::{
    CycleReport, EntryGuards, GuardFailure, Recheck, Removal, TrackerState, Worklist,
    CLOSED_AT_DISPATCH, MIN_SPEC_BYTES,
};

/// An issue that passes every existing enqueue-time guard: not in flight,
/// no patch, not on the hold list, staged spec over the 800-byte floor.
fn clear_guards() -> EntryGuards {
    EntryGuards {
        in_flight: false,
        has_patch: false,
        on_hold: false,
        spec_bytes: MIN_SPEC_BYTES + 1,
    }
}

#[test]
fn a_closed_issue_without_a_patch_is_not_dispatched_and_leaves_on_the_first_cycle() {
    // The incident shape: closed without producing a patch (closed as a
    // duplicate, fixed by a human, wontfix). Every existing guard is
    // clear, so the dispatcher would have launched a seven-hour job for
    // it.
    let guards = clear_guards();
    assert!(guards.passes());
    assert_eq!(guards.failure(), None);

    let mut recheck = Recheck::new(Worklist::new([3774, 3775]));
    recheck.guards(3774, guards);
    recheck.guards(3775, guards);
    recheck.live_state(3774, TrackerState::Closed);
    recheck.live_state(3775, TrackerState::Open);

    // First cycle: the closed issue is not dispatched and leaves the
    // worklist; the removal is logged with the issue number and reason.
    let first: CycleReport = recheck.run();
    assert_eq!(first.dispatched(), &[3775]);
    assert!(first.held().is_empty());
    assert_eq!(
        first.removed(),
        &[Removal {
            issue: 3774,
            reason: CLOSED_AT_DISPATCH,
        }]
    );
    assert_eq!(recheck.worklist().entries(), &[3775]);
    let lines = first.lines();
    assert!(
        lines
            .iter()
            .any(|line| line.contains("#3774") && line.contains(CLOSED_AT_DISPATCH)),
        "removal must be logged with the issue number and the reason: {lines:?}"
    );

    // Second cycle: the queue has converged — nothing left to remove, the
    // worklist is stable.
    let second = recheck.run();
    assert!(second.removed().is_empty());
    assert_eq!(second.dispatched(), &[3775]);
    assert_eq!(recheck.worklist().entries(), &[3775]);
}

#[test]
fn closure_is_terminal_even_when_the_patch_guard_would_have_skipped_it() {
    // The guard that happened to cover closed issues was checking a
    // different property for a different reason. With the re-check, a
    // closed issue with a patch is dropped, not skipped: the worklist
    // shrinks instead of the entry sitting behind the patch guard
    // forever.
    let mut recheck = Recheck::new(Worklist::new([42]));
    recheck.guards(
        42,
        EntryGuards {
            has_patch: true,
            ..clear_guards()
        },
    );
    recheck.live_state(42, TrackerState::Closed);

    let report = recheck.run();

    assert!(report.dispatched().is_empty());
    assert!(report.skipped().is_empty());
    assert_eq!(
        report.removed(),
        &[Removal {
            issue: 42,
            reason: CLOSED_AT_DISPATCH,
        }]
    );
    assert!(recheck.worklist().is_empty());
}

#[test]
fn an_unreadable_tracker_state_holds_the_entry_fail_closed() {
    // The check cannot answer: the tracker was unreachable. The entry is
    // held — never dispatched, and never dropped, because closure was
    // not established.
    let mut recheck = Recheck::new(Worklist::new([55]));
    recheck.guards(55, clear_guards());
    recheck.live_state(55, TrackerState::Unknown);

    let report = recheck.run();

    assert!(report.dispatched().is_empty());
    assert!(report.removed().is_empty());
    assert_eq!(report.held(), &[55]);
    assert_eq!(recheck.worklist().entries(), &[55]);
}

#[test]
fn an_open_issue_still_faces_the_existing_guards() {
    // The re-check adds the tracker state; it does not replace the
    // enqueue-time guards. An open issue is dispatched only when every
    // existing guard is clear.
    let mut recheck = Recheck::new(Worklist::new([61, 62]));
    recheck.guards(
        61,
        EntryGuards {
            in_flight: true,
            ..clear_guards()
        },
    );
    recheck.guards(62, clear_guards());
    recheck.live_state(61, TrackerState::Open);
    recheck.live_state(62, TrackerState::Open);

    let report = recheck.run();

    assert_eq!(report.dispatched(), &[62]);
    assert_eq!(report.skipped(), &[(61, GuardFailure::InFlight)],);
    assert!(report.removed().is_empty());
}
