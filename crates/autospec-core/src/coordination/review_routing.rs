//! Review routing after consecutive zero-output runs.
//!
//! A run that completes without producing a verifiable artifact is either a
//! transient environment failure or a task the model cannot do. One such run
//! earns a re-dispatch; a second consecutive one suspends dispatch and routes
//! the issue to review instead of burning more compute on an impossible run.
//!
//! The decision reads two things and nothing else: the outcome of the **most
//! recent** run, and the length of the **trailing** zero-output streak — the
//! consecutive zero-output runs that end at that most recent run (#3983). A
//! cumulative total is the wrong input because hours and run counts keep
//! climbing for an issue that went on to succeed: the work that was later
//! merged and deployed had spent 129.1 h and 79 runs by the time it was
//! believed impossible, and an issue with two early timeouts followed by a
//! recent run that produced files is still a work issue, not a stuck one.
//! Cumulative hours are a reporting metric ([`crate::cost`]), never a gate.

/// Consecutive zero-output runs before a task routes to review.
pub const REVIEW_ROUTING_THRESHOLD: usize = 2;

/// How one finished run ended, as far as the retry decision can tell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunOutcome {
    /// The run left a verifiable artifact behind (files changed, a PR).
    ProducedOutput,
    /// The run finished without a verifiable artifact.
    ZeroOutput,
}

/// The outcome of the most recent run of an issue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LatestRunOutcome {
    /// No finished run has been observed for the issue.
    NoRuns,
    /// The most recent finished run produced a verifiable artifact.
    ProducedOutput,
    /// The most recent finished run ended without one.
    ZeroOutput,
}

impl LatestRunOutcome {
    /// Derive the latest outcome from a trailing streak, for callers that
    /// carry only the streak counter (the queue frontier's state file).
    ///
    /// A non-zero trailing streak means the chain ends on a zero-output run.
    /// A zero streak means the chain is broken — the most recent run produced,
    /// or nothing has run yet — and both cases are dispatchable.
    pub fn from_trailing_streak(trailing_zero_output_streak: usize) -> Self {
        if trailing_zero_output_streak == 0 {
            Self::ProducedOutput
        } else {
            Self::ZeroOutput
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoRuns => "none",
            Self::ProducedOutput => "produced",
            Self::ZeroOutput => "zero-output",
        }
    }
}

/// Count the **trailing** zero-output runs: the consecutive `ZeroOutput`
/// entries at the end of an oldest-first run history. An older zero-output run
/// followed by a producing run contributes nothing — the producing run ended
/// the streak, which is the whole point of measuring the trailing one.
///
/// `outcomes` is ordered oldest first, matching a per-run history read from
/// job logs (`out/issue-*/`, which survive a re-dispatch).
pub fn trailing_zero_output_streak(outcomes: &[RunOutcome]) -> usize {
    outcomes
        .iter()
        .rev()
        .take_while(|outcome| **outcome == RunOutcome::ZeroOutput)
        .count()
}

/// Count **every** zero-output run in the history. This is the number that
/// must never gate a dispatch (#3983): it is monotonic in time and in retries,
/// so it keeps growing for an issue whose later runs succeeded. Reporting
/// metric only.
pub fn total_zero_output_runs(outcomes: &[RunOutcome]) -> usize {
    outcomes
        .iter()
        .filter(|outcome| **outcome == RunOutcome::ZeroOutput)
        .count()
}

/// The retry decision for one issue, with the two inputs it read kept visible
/// so a reviewer can see that no total was consulted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryDecision {
    /// Where the issue goes next.
    pub routing: ReviewRouting,
    /// The most recent run's outcome, as read.
    pub latest_outcome: LatestRunOutcome,
    /// Length of the trailing zero-output streak, as read. Always the trailing
    /// streak, never a cumulative count.
    pub trailing_zero_output_streak: usize,
}

impl RetryDecision {
    pub fn dispatchable(&self) -> bool {
        matches!(self.routing, ReviewRouting::Redispatch { .. })
    }
}

/// Decide whether to re-dispatch an issue.
///
/// The rule is read off the most recent run and the trailing streak:
///
/// - A most recent run that **produced** an artifact (or no run at all) is
///   dispatchable, whatever the issue's history or accumulated hours say. A
///   trailing streak handed in alongside a producing latest run is a stale
///   total, not a streak: it is reported as the streak the decision actually
///   used (0) rather than routing the issue to review.
/// - A most recent run that ended **zero-output** routes by the trailing
///   streak: below [`REVIEW_ROUTING_THRESHOLD`] re-dispatch, at or above it
///   hold for review.
pub fn retry_decision(
    latest_outcome: LatestRunOutcome,
    trailing_zero_output_streak: usize,
) -> RetryDecision {
    if matches!(latest_outcome, LatestRunOutcome::ProducedOutput) {
        return RetryDecision {
            routing: ReviewRouting::Redispatch {
                zero_output_streak: 0,
            },
            latest_outcome,
            trailing_zero_output_streak: 0,
        };
    }
    RetryDecision {
        routing: review_routing(trailing_zero_output_streak),
        latest_outcome,
        trailing_zero_output_streak,
    }
}

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

    // ── the retry decision reads the trailing streak, not a total (#3983) ──

    use RunOutcome::{ProducedOutput, ZeroOutput};

    #[test]
    fn trailing_streak_stops_at_the_first_producing_run() {
        // Two early zero-output runs, then a run that produced files: the
        // trailing streak is 0 even though the total is 2.
        let history = [ZeroOutput, ZeroOutput, ProducedOutput];
        assert_eq!(trailing_zero_output_streak(&history), 0);
        assert_eq!(total_zero_output_runs(&history), 2);
    }

    #[test]
    fn trailing_streak_counts_only_the_run_at_the_end() {
        assert_eq!(trailing_zero_output_streak(&[]), 0);
        assert_eq!(trailing_zero_output_streak(&[ProducedOutput]), 0);
        assert_eq!(trailing_zero_output_streak(&[ZeroOutput]), 1);
        assert_eq!(
            trailing_zero_output_streak(&[ProducedOutput, ZeroOutput, ZeroOutput]),
            2
        );
        assert_eq!(
            trailing_zero_output_streak(&[ZeroOutput, ZeroOutput, ProducedOutput, ZeroOutput]),
            1
        );
    }

    #[test]
    fn early_zero_output_runs_with_a_recent_success_stay_dispatchable() {
        // The #3793 shape: issue-3192 ran six times, two early timeouts and a
        // recent run that produced 6 files. The decision reads the recent run.
        let history = [ZeroOutput, ZeroOutput, ProducedOutput];
        let decision = retry_decision(
            LatestRunOutcome::ProducedOutput,
            trailing_zero_output_streak(&history),
        );
        assert!(decision.dispatchable(), "decision: {decision:?}");
        assert_eq!(
            decision.routing,
            ReviewRouting::Redispatch {
                zero_output_streak: 0
            }
        );
        assert_eq!(decision.latest_outcome, LatestRunOutcome::ProducedOutput);
    }

    #[test]
    fn two_trailing_zero_output_runs_route_to_review() {
        let history = [ProducedOutput, ZeroOutput, ZeroOutput];
        let decision = retry_decision(
            LatestRunOutcome::ZeroOutput,
            trailing_zero_output_streak(&history),
        );
        assert!(!decision.dispatchable(), "decision: {decision:?}");
        assert_eq!(
            decision.routing,
            ReviewRouting::Review {
                zero_output_streak: 2
            }
        );
    }

    #[test]
    fn one_trailing_zero_output_run_is_still_a_redispatch() {
        let decision = retry_decision(LatestRunOutcome::ZeroOutput, 1);
        assert!(decision.dispatchable(), "decision: {decision:?}");
        assert_eq!(
            decision.routing,
            ReviewRouting::Redispatch {
                zero_output_streak: 1
            }
        );
    }

    #[test]
    fn a_producing_latest_run_outranks_a_stale_streak() {
        // A caller that hands in a cumulative count with a producing latest run
        // gets the producing run's verdict, and the decision reports the
        // trailing streak it used rather than echoing the total back.
        let decision = retry_decision(LatestRunOutcome::ProducedOutput, 79);
        assert!(decision.dispatchable(), "decision: {decision:?}");
        assert_eq!(decision.trailing_zero_output_streak, 0);
    }

    #[test]
    fn no_runs_yet_is_dispatchable() {
        let decision = retry_decision(LatestRunOutcome::NoRuns, 0);
        assert!(decision.dispatchable(), "decision: {decision:?}");
        assert_eq!(
            decision.routing,
            ReviewRouting::Redispatch {
                zero_output_streak: 0
            }
        );
    }

    #[test]
    fn latest_outcome_resolves_from_a_trailing_streak() {
        assert_eq!(
            LatestRunOutcome::from_trailing_streak(0),
            LatestRunOutcome::ProducedOutput
        );
        assert_eq!(
            LatestRunOutcome::from_trailing_streak(2),
            LatestRunOutcome::ZeroOutput
        );
    }
}
