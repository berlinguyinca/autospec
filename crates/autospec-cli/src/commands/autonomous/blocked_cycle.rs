//! Keeping a continuous conductor alive across a blocked cycle.
//!
//! Extracted from `autonomous.rs` so the blocked-cycle rules read on their own
//! and the parent module shrinks rather than grows.

use autospec_core::coordination::{
    ConductorEvent, ConductorOutcome, ConductorPhase, ConductorState,
};

use super::{
    foreground_scope, foreground_state_path, persist_foreground_state, ForegroundCompletion,
    Options, RunLayout, NO_READY_ISSUE_PAUSE,
};

/// Whether a pause is the benign "nothing was ready after review" one, which the
/// continuous loop resumes from rather than treating as a stop.
pub(super) fn no_ready_selection_pause(state: &ConductorState) -> Result<bool, String> {
    if state.pause_reason() != Some(NO_READY_ISSUE_PAUSE) {
        return Ok(false);
    }
    if state.phase() != ConductorPhase::Paused || state.selected_issue().is_some() {
        return Err("no-ready foreground pause has incompatible state".to_string());
    }
    let resumed = state
        .clone()
        .transition(ConductorEvent::Resume)
        .map_err(|error| format!("cannot inspect no-ready foreground pause: {error}"))?;
    if resumed.phase() != ConductorPhase::Select {
        return Err("no-ready foreground pause must resume from Select".to_string());
    }
    Ok(true)
}

/// Whether a finished cycle leaves the conductor in a phase the continuous loop
/// may run again. `AllBlocked` counts: a sealed backlog stops Tier 1 selection,
/// not the conductor itself.
pub(super) fn foreground_cycle_is_loopable(
    completion: &ForegroundCompletion,
) -> Result<bool, String> {
    let ForegroundCompletion::State(state) = completion else {
        return Ok(false);
    };
    if matches!(
        state.phase(),
        ConductorPhase::Scan | ConductorPhase::AllBlocked
    ) {
        return Ok(true);
    }
    no_ready_selection_pause(state)
}

/// Apply [`blocked_cycle_continuation`] to a finished cycle and persist the result
/// so the bounded blocked-backlog count survives process death.
pub(super) fn continue_after_blocked_cycle(
    layout: &RunLayout,
    options: &Options,
    completion: ForegroundCompletion,
) -> Result<ForegroundCompletion, String> {
    let ForegroundCompletion::State(state) = completion else {
        return Ok(completion);
    };
    let (state, continued) = blocked_cycle_continuation(*state)?;
    if continued {
        let scope = foreground_scope(options, layout);
        persist_foreground_state(&foreground_state_path(layout, scope), &state)?;
    }
    Ok(ForegroundCompletion::State(Box::new(state)))
}

/// Whether a paused cycle is a deliberate stop that must end the run.
///
/// A resource park and an operator stop are decisions, not failures: the run is
/// meant to end. Every other pause costs one selection and nothing more.
fn deliberate_stop(state: &ConductorState) -> bool {
    matches!(
        state.last_outcome(),
        Some(ConductorOutcome::ResourcePark { .. }) | Some(ConductorOutcome::OperatorStop { .. })
    ) || matches!(
        state.phase(),
        ConductorPhase::ResourcePark | ConductorPhase::OperatorStop
    )
}

/// The key a continued pause is charged to in the blocked-backlog governor.
///
/// `pause_reason` comes first because it is the stable one: a blocked dispatch
/// records the blocked reason there verbatim, and the coarser routes record a
/// fixed label such as `retry_limit_exhausted`. An outcome reason can carry
/// per-attempt detail, and a key that varies per attempt never repeats, so the
/// governor would count to one forever and never seal.
fn pause_governor_reason(state: &ConductorState) -> Option<String> {
    let outcome_reason = match state.last_outcome() {
        Some(ConductorOutcome::Blocked(reason)) | Some(ConductorOutcome::Retryable(reason)) => {
            Some(reason.clone())
        }
        Some(ConductorOutcome::AllBlocked { reason, .. })
        | Some(ConductorOutcome::VerifierUnavailable { reason }) => Some(reason.clone()),
        _ => None,
    };
    state
        .pause_reason()
        .map(str::to_string)
        .or(outcome_reason)
        .filter(|reason| !reason.trim().is_empty())
}

/// Keep a continuous conductor alive across any pause that is not a deliberate stop.
///
/// A pause leaves a state the loop cannot run again, so `run_foreground_cycles`
/// returns and the process exits — and a supervisor immediately restarts it into
/// the identical failure. One issue that cannot clear its gate then costs the
/// whole conductor instead of costing its own selection.
///
/// Every route into `Paused` is covered, not just a `Blocked` outcome: retry-limit
/// exhaustion, verifier-unavailable, terminal and ownership retirement, and any
/// free-form `pause` reason. Pause reasons are open-ended `String`s, so matching a
/// known set here would leave each new reason silently fatal.
///
/// Count the cycle through the persisted blocked-backlog governor, abandon the
/// selection so the next scan moves on, and report that the loop may continue.
/// Repeating the same reason and issue set seals the state as `AllBlocked` at
/// `BLOCKED_BACKLOG_THRESHOLD`, which bounds the retries without ending the run.
///
/// Retirement goes through `RetireObsoleteSelection` rather than `AbandonTerminal`
/// because the latter admits only a blocked outcome, terminal retirement, or retry
/// exhaustion; both raise the identical `abandon_exhausted`.
fn blocked_cycle_continuation(state: ConductorState) -> Result<(ConductorState, bool), String> {
    if state.phase() != ConductorPhase::Paused || deliberate_stop(&state) {
        return Ok((state, false));
    }
    let (Some(issue), Some(reason)) = (state.selected_issue(), pause_governor_reason(&state))
    else {
        return Ok((state, false));
    };
    let state = state.record_blocked_backlog_cycle(reason, vec![issue])?;
    if state.phase() == ConductorPhase::AllBlocked {
        return Ok((state, true));
    }
    let state = state.transition(ConductorEvent::RetireObsoleteSelection)?;
    Ok((state, true))
}

/// Fail a continuous run that stopped on a state it cannot loop.
///
/// Kept here so the cycle loop stays flat: the caller asks once and either
/// continues or propagates.
pub(super) fn reject_unplanned_exit(
    continuous: bool,
    loopable_state: bool,
    completion: &ForegroundCompletion,
) -> Result<(), String> {
    if !continuous || loopable_state {
        return Ok(());
    }
    match unplanned_exit_reason(completion) {
        Some(reason) => Err(reason),
        None => Ok(()),
    }
}

/// Why a continuous run stopped, when it stopped on a state it cannot loop.
///
/// Such a stop is a defect, not a finish, and it used to be invisible: the loop
/// returned `Ok` and the process exited zero, exactly like a deliberate stop.
/// A live conductor died this way and the exit was read as a clean convergence
/// stop, because nothing in the logs distinguished the two.
///
/// A lifecycle decision, a terminal phase, and a deliberate stop are real
/// finishes and report nothing.
pub(super) fn unplanned_exit_reason(completion: &ForegroundCompletion) -> Option<String> {
    let ForegroundCompletion::State(state) = completion else {
        return None;
    };
    if matches!(
        state.phase(),
        ConductorPhase::AllDone | ConductorPhase::SliceComplete
    ) || deliberate_stop(state)
    {
        return None;
    }
    Some(format!(
        "continuous conductor stopped on a phase it cannot loop: phase={:?} pause_reason={} selected_issue={:?}",
        state.phase(),
        state.pause_reason().unwrap_or("none"),
        state.selected_issue()
    ))
}

#[cfg(test)]
mod tests {
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
}
