// Tests for the blocked-cycle rules.
//
// Split out of blocked_cycle.rs to keep that file inside the size ratchet.

use super::*;

use super::*;
use autospec_core::coordination::{ConductorScope, BLOCKED_BACKLOG_THRESHOLD};

fn blocked_foreground_state(issue: u64, reason: &str) -> ConductorState {
    ConductorState::new("test/repo", ConductorScope::Repository, 3)
        .expect("state")
        .transition(ConductorEvent::ScanFoundWork)
        .expect("scan")
        .transition(ConductorEvent::SafetyReviewed)
        .expect("review")
        .transition(ConductorEvent::Selected {
            issue,
            serialization_reasons: Vec::new(),
        })
        .expect("selected")
        .transition(ConductorEvent::Claimed)
        .expect("claimed")
        .transition(ConductorEvent::DispatchRecorded {
            outcome: ConductorOutcome::Blocked(reason.to_string()),
        })
        .expect("blocked")
}

#[test]
fn blocked_cycle_keeps_the_conductor_looping_instead_of_exiting() {
    let state = blocked_foreground_state(51, "executor_receipt_failed");
    assert_eq!(state.phase(), ConductorPhase::Paused);

    let (state, keep_looping) =
        blocked_cycle_continuation(state).expect("continue after blocked cycle");

    assert!(
        keep_looping,
        "a blocked issue must cost one selection, not the whole conductor"
    );
    assert_eq!(state.phase(), ConductorPhase::Scan);
    assert_eq!(state.selected_issue(), None);
    assert_eq!(state.blocked_backlog_cycles(), 1);
    assert_eq!(state.blocked_backlog_issues(), [51]);
}

/// Re-drive an already-scanning state into the same blocked dispatch, the way
/// a later cycle reloads persisted state and re-selects the same issue. The
/// blocked-backlog counters must survive this, which is what bounds the retries.
fn reblock_foreground_state(state: ConductorState, issue: u64, reason: &str) -> ConductorState {
    state
        .transition(ConductorEvent::ScanFoundWork)
        .expect("scan")
        .transition(ConductorEvent::SafetyReviewed)
        .expect("review")
        .transition(ConductorEvent::Selected {
            issue,
            serialization_reasons: Vec::new(),
        })
        .expect("selected")
        .transition(ConductorEvent::Claimed)
        .expect("claimed")
        .transition(ConductorEvent::DispatchRecorded {
            outcome: ConductorOutcome::Blocked(reason.to_string()),
        })
        .expect("blocked")
}

#[test]
fn repeating_the_same_blocked_issue_seals_the_backlog_and_stays_alive() {
    let mut state = blocked_foreground_state(51, "executor_receipt_failed");
    let mut cycles = 0;
    loop {
        let (next, keep_looping) =
            blocked_cycle_continuation(state).expect("continue after blocked cycle");
        cycles += 1;
        assert!(
            keep_looping,
            "the conductor must stay alive while the backlog seals"
        );
        state = next;
        if state.phase() == ConductorPhase::AllBlocked {
            break;
        }
        assert!(
            cycles < BLOCKED_BACKLOG_THRESHOLD + 2,
            "the governor must seal instead of looping forever"
        );
        state = reblock_foreground_state(state, 51, "executor_receipt_failed");
    }

    assert_eq!(cycles, BLOCKED_BACKLOG_THRESHOLD);
    assert_eq!(state.phase(), ConductorPhase::AllBlocked);
    assert_eq!(state.selected_issue(), None);
    assert_eq!(state.blocked_backlog_cycles(), BLOCKED_BACKLOG_THRESHOLD);
}

#[test]
fn a_different_blocked_issue_resets_the_backlog_governor() {
    let (state, _) =
        blocked_cycle_continuation(blocked_foreground_state(51, "executor_receipt_failed"))
            .expect("first blocked cycle");
    assert_eq!(state.blocked_backlog_cycles(), 1);

    let mut next = blocked_foreground_state(52, "executor_receipt_failed");
    next = next
        .clone()
        .record_blocked_backlog_cycle("executor_receipt_failed", vec![51])
        .expect("carry prior governor");
    let (next, _) = blocked_cycle_continuation(next).expect("second blocked cycle");

    assert_eq!(
        next.blocked_backlog_issues(),
        [52],
        "a new blocked issue set restarts the count"
    );
    assert_eq!(next.blocked_backlog_cycles(), 1);
}

/// Drive a fresh state to a claimed selection, ready for any dispatch outcome.
fn claimed_foreground_state(issue: u64) -> ConductorState {
    ConductorState::new("test/repo", ConductorScope::Repository, 3)
        .expect("state")
        .transition(ConductorEvent::ScanFoundWork)
        .expect("scan")
        .transition(ConductorEvent::SafetyReviewed)
        .expect("review")
        .transition(ConductorEvent::Selected {
            issue,
            serialization_reasons: Vec::new(),
        })
        .expect("selected")
        .transition(ConductorEvent::Claimed)
        .expect("claimed")
}

fn dispatched(issue: u64, outcome: ConductorOutcome) -> ConductorState {
    claimed_foreground_state(issue)
        .transition(ConductorEvent::DispatchRecorded { outcome })
        .expect("dispatch recorded")
}

/// One failed attempt, re-claimed for the next one if a retry remains.
fn retried_once(state: ConductorState) -> ConductorState {
    let state = state
        .transition(ConductorEvent::DispatchRecorded {
            outcome: ConductorOutcome::Retryable("executor timed out".to_string()),
        })
        .expect("retryable dispatch");
    if state.phase() != ConductorPhase::Retry {
        return state;
    }
    state
        .transition(ConductorEvent::RetryScheduled)
        .expect("retry scheduled")
        .transition(ConductorEvent::Claimed)
        .expect("reclaimed for the retry")
}

/// Exhausting the retry limit pauses with a `Retryable` outcome, not a `Blocked`
/// one. This is the most common route into `Paused` and it used to end the run.
#[test]
fn retry_limit_exhaustion_keeps_the_conductor_looping() {
    let mut state = claimed_foreground_state(51);
    for _ in 0..4 {
        state = retried_once(state);
    }
    assert_eq!(state.phase(), ConductorPhase::Paused);
    assert_eq!(state.pause_reason(), Some("retry_limit_exhausted"));

    let (state, keep_looping) =
        blocked_cycle_continuation(state).expect("continue after retry exhaustion");

    assert!(
        keep_looping,
        "an issue that exhausts its retries must cost one selection, not the run"
    );
    assert_eq!(state.phase(), ConductorPhase::Scan);
    assert_eq!(state.selected_issue(), None);
    assert_eq!(state.blocked_backlog_issues(), [51]);
}

/// An unavailable verifier is a transient shortage, not a decision to stop.
#[test]
fn a_verifier_unavailable_pause_keeps_the_conductor_looping() {
    let state = dispatched(
        51,
        ConductorOutcome::VerifierUnavailable {
            reason: "verifier_offline".to_string(),
        },
    );
    assert_eq!(state.phase(), ConductorPhase::Paused);

    let (state, keep_looping) =
        blocked_cycle_continuation(state).expect("continue after verifier outage");

    assert!(keep_looping);
    assert_eq!(state.phase(), ConductorPhase::Scan);
}

/// `AbandonTerminal` rejects this state, so the continuation must retire the
/// selection through the event that admits any paused selection.
#[test]
fn a_free_form_pause_keeps_the_conductor_looping() {
    let state = claimed_foreground_state(51)
        .transition(ConductorEvent::Pause {
            reason: "integration_base_dirty".to_string(),
        })
        .expect("free-form pause");
    assert_eq!(state.phase(), ConductorPhase::Paused);

    let (state, keep_looping) =
        blocked_cycle_continuation(state).expect("continue after a free-form pause");

    assert!(
        keep_looping,
        "a pause reason is a free-form string; an unknown one must not be fatal"
    );
    assert_eq!(state.phase(), ConductorPhase::Scan);
    assert_eq!(
        state.blocked_backlog_reason(),
        Some("integration_base_dirty")
    );
}

#[test]
fn a_deliberate_stop_still_ends_the_run() {
    for outcome in [
        ConductorOutcome::OperatorStop {
            reason: "operator_requested".to_string(),
        },
        ConductorOutcome::ResourcePark {
            reason: "usage_ceiling".to_string(),
        },
    ] {
        let state = dispatched(51, outcome.clone());

        let (state, keep_looping) =
            blocked_cycle_continuation(state).expect("deliberate stop passes through");

        assert!(
            !keep_looping,
            "a deliberate stop must end the run: {outcome:?}"
        );
        assert_eq!(state.phase(), ConductorPhase::Paused);
        assert!(unplanned_exit_reason(&ForegroundCompletion::State(Box::new(state))).is_none());
    }
}

#[test]
fn a_pending_retirement_keeps_its_exact_selection_for_recovery() {
    let ownership = claimed_foreground_state(51)
        .transition(ConductorEvent::BeginOwnershipRetirement)
        .expect("ownership retirement pause");
    let terminal = dispatched(51, ConductorOutcome::Succeeded)
        .transition(ConductorEvent::BeginTerminalRetirement {
            outcome: ConductorOutcome::Succeeded,
        })
        .expect("terminal retirement pause");
    for state in [ownership, terminal] {
        let (retained, keep_looping) =
            blocked_cycle_continuation(state.clone()).expect("preserve recovery state");

        assert!(!keep_looping);
        assert_eq!(retained, state);
        assert_eq!(retained.selected_issue(), Some(51));
    }
}

#[test]
fn an_unloopable_stop_is_reported_rather_than_read_as_a_finish() {
    let state = dispatched(
        51,
        ConductorOutcome::VerifierUnavailable {
            reason: "verifier_offline".to_string(),
        },
    );

    let reason = unplanned_exit_reason(&ForegroundCompletion::State(Box::new(state)))
        .expect("an unloopable pause must be reported");

    assert!(reason.contains("verifier_offline"), "reason was: {reason}");
    assert!(reason.contains("51"), "reason was: {reason}");
}

/// Sealing the backlog under a reserved reason builds an invalid outcome, which
/// poisons the persisted state: every later start seals again and dies the same way.
#[test]
fn a_retry_exhausted_backlog_seals_without_an_invalid_outcome() {
    let mut state = claimed_foreground_state(51);
    for _ in 0..4 {
        state = retried_once(state);
    }
    let reason = pause_governor_reason(&state).expect("a governor key");
    assert_ne!(reason, RETRY_LIMIT_EXHAUSTED_PAUSE);

    let mut sealing = state;
    for _ in 0..BLOCKED_BACKLOG_THRESHOLD {
        sealing = sealing
            .record_blocked_backlog_cycle(reason.clone(), vec![51])
            .expect("the governor must seal without an invalid outcome");
    }

    assert_eq!(sealing.phase(), ConductorPhase::AllBlocked);
}

/// A retry-exhausted pause has no resume phase, so it used to poison every start.
#[test]
fn a_persisted_retry_exhausted_pause_is_retired_instead_of_blocking_startup() {
    let mut state = claimed_foreground_state(51);
    for _ in 0..4 {
        state = retried_once(state);
    }
    assert_eq!(state.pause_reason(), Some("retry_limit_exhausted"));
    assert!(
        state.clone().transition(ConductorEvent::Resume).is_err(),
        "this state is precisely the one resume() cannot recover"
    );
    let path = std::env::temp_dir().join(format!(
        "autospec-exhausted-pause-{}.json",
        std::process::id()
    ));

    // Charge the governor to one below its threshold first, so the retirement seals.
    // Without that the charge stays far from the boundary and a reserved key looks
    // harmless -- which is exactly how the reserved key shipped twice.
    for _ in 0..BLOCKED_BACKLOG_THRESHOLD - 1 {
        state = state
            .record_blocked_backlog_cycle(RETRY_EXHAUSTION_GOVERNOR_KEY, vec![51])
            .expect("charge the governor");
    }

    let state = abandon_exhausted_retries(&path, state).expect("retire the exhausted pause");

    assert_eq!(state.phase(), ConductorPhase::AllBlocked);
    // The governor key must differ from the pause reason: the core reserves
    // `retry_limit_exhausted` for dispatch outcomes and rejects it as an outcome
    // reason, so sealing under that name yields an invalid AllBlocked outcome.
    assert_ne!(RETRY_EXHAUSTION_GOVERNOR_KEY, RETRY_LIMIT_EXHAUSTED_PAUSE);
    assert_eq!(state.selected_issue(), None);
    assert_eq!(state.blocked_backlog_issues(), [51]);
    assert!(path.exists(), "the retired state must be persisted");
    let _ = std::fs::remove_file(path);
}

/// A claim is lost while the state sits in `Claim`, which cannot retire directly.
#[test]
fn a_lost_claim_retires_its_selection_from_the_claim_phase() {
    let state = claimed_foreground_state(51);
    assert_ne!(state.phase(), ConductorPhase::Paused);

    let state = retire_deferred_selection(state).expect("retire a lost claim");

    assert_eq!(state.phase(), ConductorPhase::Scan);
    assert_eq!(
        state.selected_issue(),
        None,
        "the next scan must be free to pick something else"
    );
}

#[test]
fn a_non_blocked_cycle_is_left_untouched_by_the_continuation() {
    let state = ConductorState::new("test/repo", ConductorScope::Repository, 3)
        .expect("state")
        .transition(ConductorEvent::ScanFoundWork)
        .expect("scan");
    let phase = state.phase();

    let (state, keep_looping) =
        blocked_cycle_continuation(state).expect("non-blocked cycle passes through");

    assert!(!keep_looping);
    assert_eq!(state.phase(), phase);
}
