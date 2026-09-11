//! The backlog is held patches, not conversion capacity (issue #4366).
//!
//! The regression tests run in the configuration the incident required:
//! 116 open `auto-implement` issues with no PR, of which 109 held
//! conversion (56 of them conflicts), while the selector that drives the
//! work offered 11 candidates — and the first report of the backlog
//! counted 121 "patches awaiting conversion" from disk and proposed
//! parallelising the converter. The controls prove the checks do not
//! false-positive on a healthy, throughput-bound backlog.

use autospec_core::held_backlog::{
    breakdown_line, conflict_without_rebase_finding, fix_population, held_counted_as_queued,
    reason_counts_line, response_for, separate_count_finding, stall_cause,
    wrong_bottleneck_finding, BacklogBreakdown, BacklogState, Bottleneck, HeldReasons, HoldReason,
    ProposedFix, RebaseAttempt, Response,
};

/// The open `auto-implement` issues that never had a PR.
const INCIDENT_OPEN_TOTAL: usize = 116;

/// The "awaiting conversion" count computed from `changes.patch` on disk:
/// a patch on disk means an agent finished, not that conversion was
/// pending.
const INCIDENT_CLAIM: usize = 121;

fn incident_breakdown() -> BacklogBreakdown {
    BacklogBreakdown {
        held: 109,
        running: 5,
        never_implemented: 2,
        candidates: 11,
    }
}

fn incident_reasons() -> HeldReasons {
    HeldReasons {
        conflicts: 56,
        bats: 17,
        test: 9,
        build: 6,
        misc: 8,
    }
}

// --- The incident breakdown ----------------------------------------------

#[test]
fn incident_breakdown_reconciles() {
    let breakdown = incident_breakdown();
    assert!(breakdown.reconciles(INCIDENT_OPEN_TOTAL));
    // The per-reason counts cover 96 of the 109 held; 13 carry no reason.
    let reasons = incident_reasons();
    assert!(reasons.reconciles(breakdown.held));
    assert_eq!(reasons.total(), 96);
    assert_eq!(reasons.unrecorded(breakdown.held), 13);
    assert_eq!(breakdown.stalled(), 111);
}

#[test]
fn incident_breakdown_line_names_every_bucket() {
    let line = breakdown_line(&incident_breakdown(), &incident_reasons());
    assert_eq!(
        line,
        "backlog: held=109 (conflicts=56 bats=17 test=9 build=6 misc=8; unrecorded=13) running=5 not_implemented=2 candidates=11"
    );
}

#[test]
fn reason_counts_line_is_in_fixed_order() {
    assert_eq!(
        reason_counts_line(&incident_reasons()),
        "conflicts=56 bats=17 test=9 build=6 misc=8"
    );
}

#[test]
fn hold_reason_labels_match_the_report() {
    assert_eq!(HoldReason::Conflict.label(), "conflicts");
    assert_eq!(HoldReason::BatsFailure.label(), "bats");
    assert_eq!(HoldReason::TestFailure.label(), "test");
    assert_eq!(HoldReason::BuildFailure.label(), "build");
    assert_eq!(HoldReason::Other.label(), "misc");
}

#[test]
fn reasons_cannot_exceed_the_held_count() {
    let reasons = HeldReasons {
        conflicts: 110,
        ..Default::default()
    };
    assert!(!reasons.reconciles(109));
    assert_eq!(reasons.unrecorded(109), 0); // saturating: no underflow
}

#[test]
fn breakdown_that_misses_issues_does_not_reconcile() {
    let breakdown = BacklogBreakdown {
        held: 109,
        running: 5,
        never_implemented: 1, // one issue unaccounted for
        candidates: 11,
    };
    assert!(!breakdown.reconciles(INCIDENT_OPEN_TOTAL));
}

// --- Invariant 3: measure the queue with the selector ---------------------

#[test]
fn the_separate_count_was_the_wrong_one() {
    // 121 from disk vs 11 from the selector: the separate count is wrong.
    let findings = separate_count_finding(INCIDENT_CLAIM, 11);
    assert_eq!(findings.len(), 1);
    assert!(findings[0].starts_with("SEPARATE_COUNT:"));
    assert!(findings[0].contains("121"));
    assert!(findings[0].contains("11"));
}

#[test]
fn an_agreeing_count_is_not_a_finding() {
    assert!(separate_count_finding(11, 11).is_empty());
}

// --- Invariant 4: a held item is not a queued item -------------------------

#[test]
fn held_and_queued_need_opposite_responses() {
    assert_eq!(response_for(BacklogState::Held), Response::JudgementOrFix);
    assert_eq!(response_for(BacklogState::Queued), Response::Capacity);
}

#[test]
fn the_incident_claim_counted_held_as_queued() {
    let findings = held_counted_as_queued(INCIDENT_CLAIM, &incident_breakdown());
    assert_eq!(findings.len(), 1);
    assert!(findings[0].starts_with("HELD_COUNTED_AS_QUEUED:"));
    // 121 - 11 candidates = 110 non-candidates in the claim.
    assert!(findings[0].contains("110 of the claim are not candidates"));
    assert!(findings[0].contains("109 of the backlog is held"));
}

#[test]
fn a_claim_at_or_below_the_selector_count_is_not_a_finding() {
    assert!(held_counted_as_queued(11, &incident_breakdown()).is_empty());
    assert!(held_counted_as_queued(3, &incident_breakdown()).is_empty());
}

// --- Invariant 2: attempt a rebase before declaring a conflict -------------

#[test]
fn a_conflict_declared_without_a_rebase_is_a_finding() {
    let findings = conflict_without_rebase_finding(RebaseAttempt::NotAttempted);
    assert_eq!(findings.len(), 1);
    assert!(findings[0].starts_with("CONFLICT_WITHOUT_REBASE:"));
}

#[test]
fn a_rebase_attempt_clears_the_finding_either_way() {
    // Rescued: the apply failure was context against a moved base.
    assert!(conflict_without_rebase_finding(RebaseAttempt::Rescued).is_empty());
    // Genuine conflict: the attempt happened; the hold is a real one.
    assert!(conflict_without_rebase_finding(RebaseAttempt::GenuineConflict).is_empty());
}

// --- Invariant 1: staleness, not throughput, is the stall -------------------

#[test]
fn the_incident_stall_is_staleness_not_throughput() {
    let cause = stall_cause(&incident_breakdown(), &incident_reasons());
    assert_eq!(cause, Bottleneck::Staleness);
}

#[test]
fn a_capacity_fix_reaches_only_the_candidates() {
    let breakdown = incident_breakdown();
    let reasons = incident_reasons();
    // "Parallelising the converter speeds up the 11 and does nothing for
    // the 109."
    assert_eq!(
        fix_population(ProposedFix::MoreCapacity, &breakdown, &reasons),
        11
    );
    assert_eq!(
        fix_population(ProposedFix::RebaseOnArrival, &breakdown, &reasons),
        56
    );
    assert_eq!(
        fix_population(ProposedFix::ShorterWindow, &breakdown, &reasons),
        56
    );
}

#[test]
fn the_incident_proposed_fix_is_the_wrong_bottleneck() {
    let breakdown = incident_breakdown();
    let reasons = incident_reasons();
    let cause = stall_cause(&breakdown, &reasons);
    let findings = wrong_bottleneck_finding(cause, ProposedFix::MoreCapacity, &breakdown, &reasons);
    assert_eq!(findings.len(), 1);
    assert!(findings[0].starts_with("WRONG_BOTTLENECK:"));
    assert!(findings[0].contains("11 of 111 stalled items"));
    assert!(findings[0].contains("56 are held on base staleness"));
}

#[test]
fn a_staleness_fix_for_a_staleness_stall_is_not_a_finding() {
    let breakdown = incident_breakdown();
    let reasons = incident_reasons();
    let cause = stall_cause(&breakdown, &reasons);
    assert!(
        wrong_bottleneck_finding(cause, ProposedFix::RebaseOnArrival, &breakdown, &reasons)
            .is_empty()
    );
    assert!(
        wrong_bottleneck_finding(cause, ProposedFix::ShorterWindow, &breakdown, &reasons)
            .is_empty()
    );
}

#[test]
fn a_throughput_stall_rejects_a_staleness_fix() {
    // Control: a healthy backlog with no held conflicts and 3 candidates
    // waiting. The stall is throughput; rebasing on arrival fixes nothing.
    let breakdown = BacklogBreakdown {
        held: 0,
        running: 1,
        never_implemented: 0,
        candidates: 3,
    };
    let reasons = HeldReasons::default();
    assert_eq!(stall_cause(&breakdown, &reasons), Bottleneck::Throughput);
    let findings = wrong_bottleneck_finding(
        Bottleneck::Throughput,
        ProposedFix::RebaseOnArrival,
        &breakdown,
        &reasons,
    );
    assert_eq!(findings.len(), 1);
    assert!(findings[0].starts_with("WRONG_BOTTLENECK:"));
    assert!(findings[0].contains("3 candidates"));
    // And the capacity fix for this stall is clean.
    assert!(wrong_bottleneck_finding(
        Bottleneck::Throughput,
        ProposedFix::MoreCapacity,
        &breakdown,
        &reasons
    )
    .is_empty());
}

#[test]
fn an_empty_backlog_is_not_a_stall() {
    let breakdown = BacklogBreakdown {
        held: 0,
        running: 0,
        never_implemented: 0,
        candidates: 0,
    };
    let reasons = HeldReasons::default();
    assert_eq!(stall_cause(&breakdown, &reasons), Bottleneck::NoStall);
    // No population is stalled, so no fix is misdirected.
    assert!(wrong_bottleneck_finding(
        Bottleneck::NoStall,
        ProposedFix::MoreCapacity,
        &breakdown,
        &reasons
    )
    .is_empty());
}

// --- The incident end-to-end -----------------------------------------------

#[test]
fn the_incident_end_to_end() {
    let breakdown = incident_breakdown();
    let reasons = incident_reasons();

    // The miscount: 121 "awaiting conversion" computed from disk, while
    // the selector's accounting line — on screen in the same session —
    // offered 11.
    assert!(!separate_count_finding(INCIDENT_CLAIM, breakdown.candidates).is_empty());
    assert!(!held_counted_as_queued(INCIDENT_CLAIM, &breakdown).is_empty());

    // The real answer: the stall is staleness, and the fix the first
    // report proposed reaches 11 of 111 stalled items.
    let cause = stall_cause(&breakdown, &reasons);
    assert_eq!(cause, Bottleneck::Staleness);
    let findings = wrong_bottleneck_finding(cause, ProposedFix::MoreCapacity, &breakdown, &reasons);
    assert_eq!(findings.len(), 1);

    // And the 56 conflicts may never have had a rebase attempt: declaring
    // them from the failed apply alone is a finding.
    assert!(!conflict_without_rebase_finding(RebaseAttempt::NotAttempted).is_empty());

    // The line the backlog should have been reported in.
    let line = breakdown_line(&breakdown, &reasons);
    assert!(line.starts_with("backlog: held=109 (conflicts=56"));
    assert!(line.ends_with("candidates=11"));
}
