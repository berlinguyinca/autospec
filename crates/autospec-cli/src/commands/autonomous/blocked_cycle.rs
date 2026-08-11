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
