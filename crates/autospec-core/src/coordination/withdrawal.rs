//! Dispatch withdrawal on a trailing zero-output streak, never on cumulative cost.
//!
//! Ranking issues by the GPU-hours they have consumed in total produced a
//! "stop re-dispatching this" list that was wrong in half its rows:
//!
//! ```text
//! #3805: 5 runs, 21.0h, 3 full-budget timeouts
//! #3192: 5 runs, 15.2h, 2 full-budget timeouts
//! ```
//!
//! Both were blocked as unstartable. `#3192`'s runs, in order, were two
//! `TIMEOUT-NO-OUTPUT` runs and then a run that produced a 45 KB, 6-file patch
//! with 887 tests passing. Its *most recent* attempt had succeeded; blocking
//! it would have stranded a finished patch. `#3805` was the other case: three
//! consecutive terminal timeouts *with the latest run also silent*.
//!
//! **A high cumulative cost means an issue has failed before. It says nothing
//! about whether it is failing now.** An issue can be expensive precisely
//! because it failed twice and then worked — which is the outcome the retry
//! policy exists to produce. Sorting by sum inverts the ranking: an issue that
//! failed twice and then succeeded sorts *above* one that failed twice and
//! stopped being tried, because success runs consume time too.
//!
//! The signal that matters is a run of consecutive zero-output runs *ending at
//! the present*. That is what this module decides on, and cumulative cost —
//! total GPU-hours, total failure count — is deliberately not an input to the
//! decision. It is a reporting metric for where GPU went, not a retry input.
//!
//! Everything here is pure and testable: no I/O, no clock. The caller supplies
//! the per-run outcomes (from job logs, which survive re-dispatch) and reads a
//! verdict.

/// Consecutive zero-output runs (ending at the most recent attempt) at or
/// above which an issue is withdrawn from dispatch.
///
/// This mirrors [`super::review_routing::REVIEW_ROUTING_THRESHOLD`]: the
/// withdrawal streak sharpens the same "two consecutive zero-output runs"
/// rule that routes an issue to review. It is its own constant so the two
/// gates can evolve independently if they ever need to.
pub const WITHDRAWAL_THRESHOLD: usize = 2;

/// The dispatch-withdrawal verdict for one issue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WithdrawalDecision {
    /// Keep offering the issue. The trailing zero-output streak is below the
    /// threshold — including when the most recent run produced output, which
    /// resets the streak to zero regardless of how many earlier runs failed.
    Dispatch { trailing_zero_output_streak: usize },
    /// Withdraw the issue from dispatch. A streak of consecutive zero-output
    /// runs ending at the most recent attempt reached the threshold.
    Withdrawn { trailing_zero_output_streak: usize },
}

impl WithdrawalDecision {
    /// Whether the issue must be withdrawn from dispatch.
    pub fn withdrawn(self) -> bool {
        matches!(self, Self::Withdrawn { .. })
    }

    /// The same decision from just the trailing zero-output streak length, as
    /// maintained by the ready queue's `no_output_streaks` map. Equivalent to
    /// [`dispatch_withdrawal`] over a sequence whose trailing zero-output run
    /// has this length — the two agree because both read only the streak
    /// ending at the present, never a total.
    pub fn from_trailing_streak(streak: usize) -> Self {
        if streak >= WITHDRAWAL_THRESHOLD {
            Self::Withdrawn {
                trailing_zero_output_streak: streak,
            }
        } else {
            Self::Dispatch {
                trailing_zero_output_streak: streak,
            }
        }
    }
}

/// Withdraw an issue from dispatch on a trailing streak of zero-output runs
/// ending at the most recent attempt — **never on cumulative cost**.
///
/// `zero_output` holds one entry per run in chronological order (the most
/// recent run last); `true` marks a run that finished with no output (a
/// full-budget timeout that left no patch), `false` marks a run that produced
/// output.
///
/// The decision reads only two things, both of which end at the present:
///
/// 1. the most recent run's outcome (the final entry), and
/// 2. the length of the trailing run of zero-output runs ending there.
///
/// The total number of earlier failures — and the total GPU-hours the issue
/// has consumed — never enter the decision. A run that produced output
/// truncates the trailing streak to zero, so an issue that failed early and
/// then succeeded stays dispatchable: `[true, true, false]` is `Dispatch`,
/// not `Withdrawn`.
pub fn dispatch_withdrawal(zero_output: &[bool]) -> WithdrawalDecision {
    let streak = zero_output
        .iter()
        .rev()
        .take_while(|was_zero| **was_zero)
        .count();
    WithdrawalDecision::from_trailing_streak(streak)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_issue_with_no_runs_stays_dispatchable() {
        assert_eq!(
            dispatch_withdrawal(&[]),
            WithdrawalDecision::Dispatch {
                trailing_zero_output_streak: 0
            }
        );
    }

    #[test]
    fn a_single_zero_output_run_stays_dispatchable() {
        assert_eq!(
            dispatch_withdrawal(&[true]),
            WithdrawalDecision::Dispatch {
                trailing_zero_output_streak: 1
            }
        );
    }

    #[test]
    fn two_consecutive_zero_output_runs_end_at_present_withdraw() {
        assert_eq!(
            dispatch_withdrawal(&[true, true]),
            WithdrawalDecision::Withdrawn {
                trailing_zero_output_streak: 2
            }
        );
    }

    /// `#3805`: three consecutive terminal timeouts, the latest run silent.
    /// A trailing streak of 3 ends at the present and reaches the threshold.
    #[test]
    fn three_consecutive_zero_output_runs_withdraw() {
        assert_eq!(
            dispatch_withdrawal(&[true, true, true]),
            WithdrawalDecision::Withdrawn {
                trailing_zero_output_streak: 3
            }
        );
    }

    /// `#3192` / the #3793 populated case: two early timeouts followed by a
    /// run that produced a patch. The most recent run succeeded, so the
    /// trailing zero-output streak is zero and the issue remains
    /// dispatchable — its 15.2 cumulative GPU-hours are irrelevant.
    #[test]
    fn early_failures_followed_by_a_success_stay_dispatchable() {
        assert_eq!(
            dispatch_withdrawal(&[true, true, false]),
            WithdrawalDecision::Dispatch {
                trailing_zero_output_streak: 0
            }
        );
    }

    /// The cumulative-total trap, made explicit: many early failures and a
    /// recent success carry a large *total* failure count, but the trailing
    /// streak is zero. The decision must not look at the total.
    #[test]
    fn a_high_total_failure_count_does_not_withdraw_after_a_success() {
        let mut runs = vec![true; 9]; // nine failures in total
        runs.push(false); // then it worked
        assert_eq!(
            dispatch_withdrawal(&runs),
            WithdrawalDecision::Dispatch {
                trailing_zero_output_streak: 0
            }
        );
    }

    /// An interleaved success in the middle breaks the streak: only the runs
    /// after the most recent success count toward the trailing streak.
    #[test]
    fn only_the_runs_after_the_most_recent_success_count() {
        // fail, fail, succeed, fail, fail -> trailing streak is 2.
        assert_eq!(
            dispatch_withdrawal(&[true, true, false, true, true]),
            WithdrawalDecision::Withdrawn {
                trailing_zero_output_streak: 2
            }
        );
        // fail, fail, succeed, fail -> trailing streak is 1.
        assert_eq!(
            dispatch_withdrawal(&[true, true, false, true]),
            WithdrawalDecision::Dispatch {
                trailing_zero_output_streak: 1
            }
        );
    }

    #[test]
    fn the_withdrawn_helper_matches_the_variant() {
        assert!(dispatch_withdrawal(&[true, true]).withdrawn());
        assert!(dispatch_withdrawal(&[true, true, false]).withdrawn() == false);
        assert!(!dispatch_withdrawal(&[]).withdrawn());
    }
}
