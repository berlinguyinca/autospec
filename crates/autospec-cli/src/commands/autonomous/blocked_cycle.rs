//! Keeping a continuous conductor alive across a blocked cycle.
//!
//! Extracted from `autonomous.rs` so the blocked-cycle rules read on their own
//! and the parent module shrinks rather than grows.

use autospec_core::coordination::{
    ConductorEvent, ConductorOutcome, ConductorPhase, ConductorState,
};

use super::{
    foreground_scope, foreground_state_path, load_foreground_state, persist_foreground_state,
    CommandFailure, ForegroundCompletion, ForegroundFailure, Options, RunLayout,
    NO_READY_ISSUE_PAUSE, OWNERSHIP_RETIREMENT_PAUSE, TERMINAL_RETIREMENT_PAUSE,
};

/// The pause a conductor cannot resume from, and the only one that poisons startup.
const RETRY_LIMIT_EXHAUSTED_PAUSE: &str = "retry_limit_exhausted";

/// The governor key that pause is charged under.
///
/// It must differ from the pause reason: the core reserves `retry_limit_exhausted`
/// to dispatch outcomes, and `validate_named_outcome_reason` rejects it, so sealing
/// the backlog under that name produces an invalid `AllBlocked` outcome.
const RETRY_EXHAUSTION_GOVERNOR_KEY: &str = "retry_exhaustion_recovered";

/// Retire a persisted pause the conductor can never resume.
///
/// `record_retryable_dispatch` clears `resume_phase` when the retry limit is
/// exhausted, so `resume()` fails with "paused conductor state requires explicit
/// recovery". Persisted to disk, that state poisons every subsequent start: the
/// process loads it, dies, and a supervisor restarts it into the identical failure.
/// Observed live at 158 restarts, unrecoverable without deleting the file by hand.
///
/// `AbandonExhausted` exists for exactly this state -- `can_abandon_exhausted`
/// admits it and it needs no resume phase. Charging the governor first keeps the
/// retries bounded, so a genuinely stuck issue still seals rather than cycling.
///
/// Only this pause reason is touched. Terminal and ownership retirement also clear
/// `resume_phase`, but they have their own recovery that settles claim receipts,
/// and preempting it would strand those receipts.
pub(super) fn abandon_exhausted_retries(
    path: &std::path::Path,
    state: ConductorState,
) -> Result<ConductorState, String> {
    if state.phase() != ConductorPhase::Paused
        || state.pause_reason() != Some(RETRY_LIMIT_EXHAUSTED_PAUSE)
    {
        return Ok(state);
    }
    let state = match state.selected_issue() {
        Some(issue) => {
            state.record_blocked_backlog_cycle(RETRY_EXHAUSTION_GOVERNOR_KEY, vec![issue])?
        }
        None => state,
    };
    let state = if state.phase() == ConductorPhase::AllBlocked {
        state
    } else {
        state.transition(ConductorEvent::AbandonExhausted)?
    };
    persist_foreground_state(path, &state)?;
    Ok(state)
}

/// The governor key a lost claim is charged to.
///
/// Deliberately coarse. The deferral JSON names the observed owner, which changes
/// with every competing generation; keying on that would never repeat, so the
/// backlog would count to one forever and never seal.
const DEFERRED_CANDIDATE_REASON: &str = "claim_deferred";

/// Turn a claim the conductor did not win into a finished cycle it can loop past.
///
/// Losing a claim race is one candidate's problem. It used to exit the process with
/// code 2, and a supervisor restarted straight into the same race: 139 restarts in
/// under two hours, no work done, while the issue was re-wedged on every pass.
///
/// Only `CandidateDeferred` is contained, and only in a continuous run. `Deferred`
/// still ends the run, which matters because that is how a lifecycle lease reject
/// arrives -- containing it would leave two conductors mutating one repository.
pub(super) fn deferred_candidate_cycle(
    layout: &RunLayout,
    options: &Options,
    failure: ForegroundFailure,
    continuous: bool,
) -> Result<ForegroundCompletion, ForegroundFailure> {
    let ForegroundFailure::CandidateDeferred { json, exit_code } = failure else {
        return Err(failure);
    };
    if !continuous {
        return Err(ForegroundFailure::CandidateDeferred { json, exit_code });
    }
    println!("{json}");
    let scope = foreground_scope(options, layout);
    let path = foreground_state_path(layout, scope);
    let state = load_foreground_state(&path, layout, scope).map_err(diagnostic)?;
    let Some(issue) = state.selected_issue() else {
        return Ok(ForegroundCompletion::State(Box::new(state)));
    };
    let state = state
        .record_blocked_backlog_cycle(DEFERRED_CANDIDATE_REASON, vec![issue])
        .map_err(diagnostic)?;
    let state = if state.phase() == ConductorPhase::AllBlocked {
        state
    } else {
        retire_deferred_selection(state).map_err(diagnostic)?
    };
    persist_foreground_state(&path, &state).map_err(diagnostic)?;
    Ok(ForegroundCompletion::State(Box::new(state)))
}

/// Drop the selection so the next scan moves on.
///
/// A claim is lost from `Claim`, which cannot retire directly, so pause first and
/// then retire -- `RetireObsoleteSelection` admits any paused selection.
fn retire_deferred_selection(state: ConductorState) -> Result<ConductorState, String> {
    let state = if state.phase() == ConductorPhase::Paused {
        state
    } else {
        state.transition(ConductorEvent::Pause {
            reason: DEFERRED_CANDIDATE_REASON.to_string(),
        })?
    };
    state.transition(ConductorEvent::RetireObsoleteSelection)
}

fn diagnostic(reason: String) -> ForegroundFailure {
    ForegroundFailure::Diagnostic(CommandFailure::diagnostic(reason))
}

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
        .map(governor_safe_reason)
}

/// Keep a governor key out of the reason space the core reserves.
///
/// `validate_named_outcome_reason` rejects `retry_limit_exhausted`, which is
/// recorded only from a dispatch outcome. Charging the backlog under that name
/// looks harmless until the count reaches `BLOCKED_BACKLOG_THRESHOLD`: sealing
/// then builds an `AllBlocked` outcome with that reason, validation fails, and
/// the persisted state seals and dies identically on every later start.
fn governor_safe_reason(reason: String) -> String {
    if reason == RETRY_LIMIT_EXHAUSTED_PAUSE {
        return RETRY_EXHAUSTION_GOVERNOR_KEY.to_string();
    }
    reason
}

/// Keep a continuous conductor alive across any pause that is not a deliberate stop.
///
/// A pause leaves a state the loop cannot run again, so `run_foreground_cycles`
/// returns and the process exits — and a supervisor immediately restarts it into
/// the identical failure. One issue that cannot clear its gate then costs the
/// whole conductor instead of costing its own selection.
///
/// Every disposable route into `Paused` is covered, not just a `Blocked` outcome:
/// retry-limit exhaustion, verifier-unavailable, and free-form `pause` reasons.
/// Ownership and terminal retirement pauses are intentionally retained because
/// their selected issue and acquisition receipt are durable recovery identity.
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
    if state.phase() != ConductorPhase::Paused
        || deliberate_stop(&state)
        || matches!(
            state.pause_reason(),
            Some(OWNERSHIP_RETIREMENT_PAUSE) | Some(TERMINAL_RETIREMENT_PAUSE)
        )
    {
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
mod tests;
