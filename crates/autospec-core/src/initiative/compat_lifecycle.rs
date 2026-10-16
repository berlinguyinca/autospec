//! Adapter from the work-protocol lifecycle to the canonical task lifecycle.
//!
//! `resilience::work_protocol::{WorkState, WorkStateNonHappy}` map onto
//! `initiative::task::TaskState` exactly as `docs/contracts/initiative-dag-model-mapping.md`
//! section "Task lifecycles" tabulates. Every match is exhaustive with no
//! wildcard arm, so a new variant on either side is a compile error here
//! rather than a silent default.
//!
//! Nothing unknown becomes done: only `WorkState::Merged` with no non-happy
//! state maps to a state that satisfies dependents.

use super::task::TaskState;
use crate::resilience::work_protocol::{WorkState, WorkStateNonHappy};

/// The canonical task state for a work item.
///
/// A non-happy state outranks the happy state: a `Merged` item that is also
/// `Unknown` is `Blocked`, not `Verified`.
pub fn task_state_from_work(state: WorkState, non_happy: Option<WorkStateNonHappy>) -> TaskState {
    match non_happy {
        Some(WorkStateNonHappy::Blocked) => TaskState::Blocked,
        Some(WorkStateNonHappy::Failed) => TaskState::FailedTerminal,
        Some(WorkStateNonHappy::Abandoned) => TaskState::FailedRetryable,
        Some(WorkStateNonHappy::RetryPending) => TaskState::FailedRetryable,
        Some(WorkStateNonHappy::NeedsHuman) => TaskState::Blocked,
        Some(WorkStateNonHappy::Superseded) => TaskState::Superseded,
        Some(WorkStateNonHappy::Cancelled) => TaskState::FailedTerminal,
        Some(WorkStateNonHappy::Unknown) => TaskState::Blocked,
        None => match state {
            WorkState::Created => TaskState::Defined,
            WorkState::Assigned => TaskState::Ready,
            WorkState::Delivered => TaskState::Leased,
            WorkState::Claimed => TaskState::Leased,
            WorkState::Running => TaskState::Running,
            WorkState::Completed => TaskState::AwaitingTest,
            WorkState::Validated => TaskState::AwaitingReview,
            WorkState::Reviewed => TaskState::AwaitingReview,
            WorkState::Merged => TaskState::Verified,
        },
    }
}

/// The work-protocol state for a canonical task state, never further along
/// than the task is.
///
/// A `TaskState` that records a failure or a hold does not record how far the
/// work had got, so those map to the least advanced `WorkState`,
/// `Created`, carrying the non-happy state. `ChangesRequested` has no
/// counterpart (a review round is a new attempt in the work protocol) and
/// maps to `None`.
pub fn work_state_from_task(state: TaskState) -> Option<(WorkState, Option<WorkStateNonHappy>)> {
    match state {
        TaskState::Defined => Some((WorkState::Created, None)),
        TaskState::Ready => Some((WorkState::Assigned, None)),
        TaskState::Leased => Some((WorkState::Delivered, None)),
        TaskState::Running => Some((WorkState::Running, None)),
        TaskState::AwaitingTest => Some((WorkState::Completed, None)),
        TaskState::AwaitingReview => Some((WorkState::Validated, None)),
        TaskState::Verified => Some((WorkState::Merged, None)),
        TaskState::Blocked => Some((WorkState::Created, Some(WorkStateNonHappy::Blocked))),
        TaskState::FailedRetryable => {
            Some((WorkState::Created, Some(WorkStateNonHappy::RetryPending)))
        }
        TaskState::FailedTerminal => Some((WorkState::Created, Some(WorkStateNonHappy::Failed))),
        TaskState::Superseded => Some((WorkState::Created, Some(WorkStateNonHappy::Superseded))),
        TaskState::ChangesRequested => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HAPPY: [WorkState; 9] = [
        WorkState::Created,
        WorkState::Assigned,
        WorkState::Delivered,
        WorkState::Claimed,
        WorkState::Running,
        WorkState::Completed,
        WorkState::Validated,
        WorkState::Reviewed,
        WorkState::Merged,
    ];

    const NON_HAPPY: [WorkStateNonHappy; 8] = [
        WorkStateNonHappy::Blocked,
        WorkStateNonHappy::Failed,
        WorkStateNonHappy::Abandoned,
        WorkStateNonHappy::RetryPending,
        WorkStateNonHappy::NeedsHuman,
        WorkStateNonHappy::Superseded,
        WorkStateNonHappy::Cancelled,
        WorkStateNonHappy::Unknown,
    ];

    #[test]
    fn unknown_maps_to_blocked() {
        assert_eq!(
            task_state_from_work(WorkState::Running, Some(WorkStateNonHappy::Unknown)),
            TaskState::Blocked
        );
    }

    #[test]
    fn only_merged_without_a_non_happy_state_satisfies_dependents() {
        for state in HAPPY {
            let satisfied = task_state_from_work(state, None).satisfies_dependents();
            assert_eq!(satisfied, state == WorkState::Merged, "{state:?}");
            for non_happy in NON_HAPPY {
                assert!(
                    !task_state_from_work(state, Some(non_happy)).satisfies_dependents(),
                    "{state:?} with {non_happy:?}"
                );
            }
        }
    }

    #[test]
    fn a_non_happy_state_outranks_a_merged_one() {
        assert_eq!(
            task_state_from_work(WorkState::Merged, Some(WorkStateNonHappy::Failed)),
            TaskState::FailedTerminal
        );
    }

    #[test]
    fn every_happy_state_survives_the_round_trip_never_moving_ahead() {
        for state in HAPPY {
            let task = task_state_from_work(state, None);
            let (back, non_happy) = work_state_from_task(task).expect("happy states map back");
            assert_eq!(non_happy, None, "{state:?}");
            assert!(back <= state, "{state:?} came back ahead as {back:?}");
            assert_eq!(task_state_from_work(back, None), task, "{state:?}");
        }
    }

    #[test]
    fn seven_happy_states_return_exactly_and_two_share_a_task_state() {
        let exact: Vec<WorkState> = HAPPY
            .into_iter()
            .filter(|state| {
                work_state_from_task(task_state_from_work(*state, None)) == Some((*state, None))
            })
            .collect();
        assert_eq!(exact.len(), 7);
        assert!(!exact.contains(&WorkState::Claimed));
        assert!(!exact.contains(&WorkState::Reviewed));
    }

    #[test]
    fn non_happy_task_states_come_back_as_their_non_happy_state() {
        for non_happy in NON_HAPPY {
            let task = task_state_from_work(WorkState::Running, Some(non_happy));
            let (_, back) = work_state_from_task(task).expect("maps back");
            let back = back.expect("carries a non-happy state");
            assert_eq!(
                task_state_from_work(WorkState::Created, Some(back)),
                task,
                "{non_happy:?}"
            );
        }
    }

    #[test]
    fn changes_requested_has_no_work_state() {
        assert_eq!(work_state_from_task(TaskState::ChangesRequested), None);
    }
}
