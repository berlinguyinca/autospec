//! Keeping a continuous conductor alive across a blocked cycle.
//!
//! Extracted from `autonomous.rs` so the blocked-cycle rules read on their own
//! and the parent module shrinks rather than grows.

use autospec_core::coordination::{
    ConductorEvent, ConductorOutcome, ConductorPhase, ConductorState,
};

use super::{
    foreground_scope, foreground_state_path, no_ready_selection_pause, persist_foreground_state,
    ForegroundCompletion, Options, RunLayout,
};

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

/// Keep a continuous conductor alive across a blocked cycle.
///
/// A blocked dispatch pauses the state with a `Blocked` outcome. Left alone that
/// state is not loopable, so `run_foreground_cycles` returns and the process
/// exits — and a supervisor immediately restarts it into the identical failure.
/// One issue that cannot clear its gate then costs the whole conductor instead
/// of costing its own selection.
///
/// Count the cycle through the persisted blocked-backlog governor, abandon the
/// selection so the next scan moves on, and report that the loop may continue.
/// Repeating the same reason and issue set seals the state as `AllBlocked` at
/// `BLOCKED_BACKLOG_THRESHOLD`, which bounds the retries without ending the run.
/// A cycle that is not a blocked pause passes through untouched.
fn blocked_cycle_continuation(state: ConductorState) -> Result<(ConductorState, bool), String> {
    if state.phase() != ConductorPhase::Paused {
        return Ok((state, false));
    }
    let Some(ConductorOutcome::Blocked(reason)) = state.last_outcome().cloned() else {
        return Ok((state, false));
    };
    let Some(issue) = state.selected_issue() else {
        return Ok((state, false));
    };
    let state = state.record_blocked_backlog_cycle(reason, vec![issue])?;
    if state.phase() == ConductorPhase::AllBlocked {
        return Ok((state, true));
    }
    let state = state.transition(ConductorEvent::AbandonTerminal)?;
    Ok((state, true))
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
