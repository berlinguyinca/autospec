//! Review routing after consecutive zero-output runs.
//!
//! A run that completes without producing a verifiable artifact is either a
//! transient environment failure or a task the model cannot do. One such run
//! earns a re-dispatch; a second consecutive one suspends dispatch and routes
//! the issue to review instead of burning more compute on an impossible run.

/// Consecutive zero-output runs before a task routes to review.
pub const REVIEW_ROUTING_THRESHOLD: usize = 2;

/// The dispatch decision for an issue with a zero-output streak.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewRouting {
    /// Below threshold: the task may be re-dispatched.
    Redispatch { zero_output_streak: usize },
    /// At or above threshold: dispatch is suspended pending review.
    Review { zero_output_streak: usize },
}

/// Route an issue by its consecutive zero-output run count.
pub fn review_routing(zero_output_streak: usize) -> ReviewRouting {
    if zero_output_streak >= REVIEW_ROUTING_THRESHOLD {
        ReviewRouting::Review { zero_output_streak }
    } else {
        ReviewRouting::Redispatch { zero_output_streak }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_issue_redispatches() {
        assert_eq!(
            review_routing(0),
            ReviewRouting::Redispatch {
                zero_output_streak: 0
            }
        );
    }

    #[test]
    fn single_zero_output_run_still_redispatches() {
        assert_eq!(
            review_routing(1),
            ReviewRouting::Redispatch {
                zero_output_streak: 1
            }
        );
    }

    #[test]
    fn two_consecutive_zero_output_runs_route_to_review() {
        assert_eq!(
            review_routing(2),
            ReviewRouting::Review {
                zero_output_streak: 2
            }
        );
    }

    #[test]
    fn longer_streaks_stay_in_review() {
        assert_eq!(
            review_routing(7),
            ReviewRouting::Review {
                zero_output_streak: 7
            }
        );
    }
}
