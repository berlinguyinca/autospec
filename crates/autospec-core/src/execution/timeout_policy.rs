//! Timeout escalation for a work item whose runtime distribution is bimodal (#3722).
//!
//! Two runs burned their full 7-hour budget (25200s) at maximal context
//! (131072 tokens) and still produced unfinished work. A timeout that consumed
//! the whole budget is not a scheduling accident the way a 5-minute stall is:
//! it is evidence that the work item is larger than what one agent run can
//! finish, and the distribution of runtimes carries that evidence — a run is
//! "abnormally large" relative to what comparable work has actually taken, not
//! relative to a constant somebody chose once.
//!
//! The old behavior re-dispatched such an item with a larger budget and tried
//! again: a third 7-hour attempt, a fourth, each consuming a full reservation
//! and finishing unfinished, because nothing distinguished "cut off early"
//! from "given everything and still not done".
//!
//! Four rules, each a pure primitive, each the mirror image of one of those
//! failures:
//!
//! 1. **Escalate once, then mark too-large** ([`redispatch_after_timeout`]).
//!    The first timeout below the ceiling earns exactly one escalation to the
//!    ceiling. Any timeout after that — a second attempt, or a first attempt
//!    that was already at the ceiling — is terminal: the item is labelled
//!    [`TOO_LARGE_LABEL`] and re-dispatching stops. "One more 7 hours" is
//!    never the answer twice.
//! 2. **Compare against the rolling mean, not a constant**
//!    ([`SizeHistory`] + [`decomposition_candidate`]). Whether a run is
//!    abnormally large is judged against the mean of recent `agent_secs`, not
//!    a fixed 600s threshold that is simultaneously "too small" for the big
//!    mode and "too large" for the small one.
//! 3. **A timeout message reports the resources it spent**
//!    ([`timeout_message`]). `TIMEOUT after 25200s at 131072 context` — the
//!    elapsed wallclock and the context window the run held. A bare
//!    `TIMEOUT` line cannot tell a full-budget, full-context exhaustion from
//!    an early stall, and the disposition rule 1 needs exactly that
//!    distinction.
//! 4. **Check domain aggregation before splitting**
//!    ([`decomposition_target`]). Two timeouts in the same domain say the
//!    problem is in the domain — its spec, its codebase region — not in one
//!    issue. Splitting that issue again re-splits the same saturated work;
//!    the decomposition unit becomes the domain.
//!
//! Like [`super::patch_pipeline`], this module performs no I/O and knows
//! nothing about the runner: the runner observes the kill, reads the context
//! counter, and hands the numbers here. Keeping the policy here is the point —
//! it is the part that was wrong, and it is now testable.

use std::collections::VecDeque;

/// The label a terminally timed-out work item receives. Issues carrying it are
/// excluded from the re-dispatch queue: rule 1's "stop" is a state the queue
/// can see, not a promise the runner keeps.
pub const TOO_LARGE_LABEL: &str = "autospec:too-large";

/// Timeouts in one domain before the domain, not the issue, is the
/// decomposition unit (rule 4). Two is the smallest count that can distinguish
/// "this issue is hard" from "this domain is saturated".
pub const DOMAIN_TIMEOUT_THRESHOLD: usize = 2;

/// What the last attempt at a work item looked like, as observed after its
/// timeout kill.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeoutAttempt {
    /// The wallclock the timed-out attempt was given.
    previous_budget_secs: u64,
    /// The largest budget a dispatch of this item may be given: the ceiling
    /// the escalation can reach and nothing beyond.
    ceiling_secs: u64,
    /// The timed-out attempt was already the escalated one.
    previous_was_escalated: bool,
}

impl TimeoutAttempt {
    /// Record a timed-out attempt.
    ///
    /// Validation is part of the policy: a budget of zero seconds is a
    /// misconfiguration, not a timeout of real work, and a budget above the
    /// ceiling means the numbers come from two different cages. Both are
    /// refused with the numbers in the message rather than fed into a
    /// disposition that looks confident.
    pub fn new(
        previous_budget_secs: u64,
        ceiling_secs: u64,
        previous_was_escalated: bool,
    ) -> Result<Self, TimeoutPolicyError> {
        if ceiling_secs == 0 {
            return Err(TimeoutPolicyError::ZeroCeiling);
        }
        if previous_budget_secs == 0 || previous_budget_secs > ceiling_secs {
            return Err(TimeoutPolicyError::BudgetOutOfRange {
                budget_secs: previous_budget_secs,
                ceiling_secs,
            });
        }
        Ok(Self {
            previous_budget_secs,
            ceiling_secs,
            previous_was_escalated,
        })
    }

    pub fn previous_budget_secs(&self) -> u64 {
        self.previous_budget_secs
    }

    pub fn ceiling_secs(&self) -> u64 {
        self.ceiling_secs
    }

    pub fn previous_was_escalated(&self) -> bool {
        self.previous_was_escalated
    }
}

/// What the work item does after a timeout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeoutDisposition {
    /// The first timeout, below the ceiling: the item gets exactly one
    /// escalated attempt at the ceiling.
    Escalate { budget_secs: u64 },
    /// The escalation is used or was unnecessary: the item is labelled
    /// [`TOO_LARGE_LABEL`], re-dispatching stops, and the disposition is
    /// terminal — deciding it again yields the same answer.
    TooLarge { previous_budget_secs: u64 },
}

impl TimeoutDisposition {
    /// Does this disposition end the re-dispatch loop?
    pub fn stop_redispatching(self) -> bool {
        matches!(self, Self::TooLarge { .. })
    }

    /// The label to apply, if any: [`TOO_LARGE_LABEL`] for a terminal item.
    pub fn label(self) -> Option<&'static str> {
        match self {
            Self::Escalate { .. } => None,
            Self::TooLarge { .. } => Some(TOO_LARGE_LABEL),
        }
    }

    /// The wallclock the next attempt gets, if there is one.
    pub fn next_budget_secs(self) -> Option<u64> {
        match self {
            Self::Escalate { budget_secs } => Some(budget_secs),
            Self::TooLarge { .. } => None,
        }
    }
}

/// Rule 1: decide the next attempt from the timed-out one.
///
/// One escalation, then done. The incident shape — a run given the full
/// ceiling, holding maximal context, still unfinished at the kill — lands on
/// `TooLarge` on its *first* timeout, because there is nothing left to
/// escalate to; the label and the stopped re-dispatch are the answer, not a
/// third reservation.
pub fn redispatch_after_timeout(attempt: &TimeoutAttempt) -> TimeoutDisposition {
    if !attempt.previous_was_escalated && attempt.previous_budget_secs < attempt.ceiling_secs {
        return TimeoutDisposition::Escalate {
            budget_secs: attempt.ceiling_secs,
        };
    }
    TimeoutDisposition::TooLarge {
        previous_budget_secs: attempt.previous_budget_secs,
    }
}

/// Recent `agent_secs` observations for comparable work, kept in a bounded
/// window (rule 2). The window is the distribution: the rolling mean of what
/// runs have actually taken, which the candidate test compares against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SizeHistory {
    entries: VecDeque<u64>,
    capacity: usize,
}

impl SizeHistory {
    /// Keep at most `capacity` most-recent observations. Zero is refused: a
    /// history that holds nothing is not a distribution, it is no rule at all,
    /// and the caller should say so instead of pretending.
    pub fn new(capacity: usize) -> Result<Self, TimeoutPolicyError> {
        if capacity == 0 {
            return Err(TimeoutPolicyError::EmptyHistoryWindow);
        }
        Ok(Self {
            entries: VecDeque::with_capacity(capacity),
            capacity,
        })
    }

    /// Record one completed run's wallclock, evicting the oldest observation
    /// once the window is full.
    pub fn record(&mut self, agent_secs: u64) {
        if self.entries.len() == self.capacity {
            self.entries.pop_front();
        }
        self.entries.push_back(agent_secs);
    }

    /// The rolling mean of the windowed observations, floored.
    ///
    /// `None` when the window is empty — or when its sum overflows, which at
    /// realistic values of `agent_secs` is undecidable anyway, and an
    /// undecidable mean is treated as no mean rather than a wrong one.
    pub fn rolling_mean_secs(&self) -> Option<u64> {
        if self.entries.is_empty() {
            return None;
        }
        let sum = self
            .entries
            .iter()
            .try_fold(0u64, |acc, secs| acc.checked_add(*secs));
        sum.map(|sum| sum / self.entries.len() as u64)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Whether one run is abnormally large relative to the distribution (rule 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SizeSignal {
    /// The run strictly exceeds `multiplier × rolling mean`: it belongs to the
    /// large mode and is a candidate for decomposition before it is given
    /// another full attempt.
    Candidate {
        agent_secs: u64,
        threshold_secs: u64,
    },
    /// The run is within the distribution: the timeout (if any) is a
    /// scheduling matter, not a size matter.
    Ordinary {
        agent_secs: u64,
        threshold_secs: u64,
    },
    /// No trustworthy mean to compare against: the window is empty. The
    /// caller may fall back to whatever it did before, but this module does
    /// not invent a constant threshold to stand in for the distribution.
    NoHistory { agent_secs: u64 },
}

/// Rule 2: judge one run's wallclock against the rolling mean.
///
/// `multiplier` must be at least 2: a threshold below twice the mean would
/// flag ordinary variance, which is the constant-threshold mistake in new
/// clothing. The comparison is strict — a run *at* `multiplier × mean` is the
/// edge of the distribution, not beyond it.
pub fn decomposition_candidate(
    agent_secs: u64,
    history: &SizeHistory,
    multiplier: u64,
) -> Result<SizeSignal, TimeoutPolicyError> {
    if multiplier < 2 {
        return Err(TimeoutPolicyError::MultiplierBelowTwo { multiplier });
    }
    let Some(mean) = history.rolling_mean_secs() else {
        return Ok(SizeSignal::NoHistory { agent_secs });
    };
    let threshold = multiplier.saturating_mul(mean);
    if agent_secs > threshold {
        Ok(SizeSignal::Candidate {
            agent_secs,
            threshold_secs: threshold,
        })
    } else {
        Ok(SizeSignal::Ordinary {
            agent_secs,
            threshold_secs: threshold,
        })
    }
}

/// Rule 3: the timeout line, with the resources the run spent.
///
/// Both numbers are always present — a message with one of them is a message
/// that cannot be acted on: `elapsed` alone cannot say whether the run hit a
/// small budget early or a full budget at the end, and `context` alone cannot
/// say how long the exhaustion took.
pub fn timeout_message(elapsed_secs: u64, context_tokens: u64) -> String {
    format!("TIMEOUT after {elapsed_secs}s at {context_tokens} context")
}

/// Which unit decomposition should start from (rule 4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecompositionTarget {
    /// No aggregation in the domain: the timed-out issue is the unit to split.
    Issue { domain: String },
    /// The domain has timed out at least [`DOMAIN_TIMEOUT_THRESHOLD`] times
    /// recently: the unit is the domain. Splitting this issue again would
    /// re-split work the domain has already shown it cannot absorb.
    Domain { domain: String, timeouts: usize },
}

/// Rule 4: check domain aggregation before splitting.
///
/// `recent_timeout_domains` is the runner's recent timeout log and *includes
/// the timeout just observed* — the count is how many times the domain has
/// timed out, current one included. One timeout names an issue; two name a
/// domain.
pub fn decomposition_target(
    domain: &str,
    recent_timeout_domains: &[String],
) -> Result<DecompositionTarget, TimeoutPolicyError> {
    let domain = domain.trim();
    if domain.is_empty() {
        return Err(TimeoutPolicyError::EmptyDomain {
            domain: domain.to_string(),
        });
    }
    for entry in recent_timeout_domains {
        if entry.trim().is_empty() {
            return Err(TimeoutPolicyError::EmptyDomain {
                domain: entry.clone(),
            });
        }
    }
    let timeouts = recent_timeout_domains
        .iter()
        .filter(|entry| entry.trim() == domain)
        .count();
    if timeouts >= DOMAIN_TIMEOUT_THRESHOLD {
        Ok(DecompositionTarget::Domain {
            domain: domain.to_string(),
            timeouts,
        })
    } else {
        Ok(DecompositionTarget::Issue {
            domain: domain.to_string(),
        })
    }
}

/// Every way a timeout-policy input can be wrong, with the numbers that show
/// it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimeoutPolicyError {
    /// No ceiling was supplied: there is nothing an escalation could reach and
    /// no wall a run could be observed against.
    ZeroCeiling,
    /// The timed-out attempt's budget is zero or exceeds the ceiling: the two
    /// numbers cannot describe one cage.
    BudgetOutOfRange { budget_secs: u64, ceiling_secs: u64 },
    /// A history window of zero: a distribution that holds nothing.
    EmptyHistoryWindow,
    /// A candidate multiplier below 2 would flag ordinary variance.
    MultiplierBelowTwo { multiplier: u64 },
    /// A domain name that names nothing.
    EmptyDomain { domain: String },
}

impl std::fmt::Display for TimeoutPolicyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ZeroCeiling => write!(
                f,
                "timeout policy has no ceiling: an escalation needs a budget to \
                 reach and a run needs a wall to be observed against"
            ),
            Self::BudgetOutOfRange {
                budget_secs,
                ceiling_secs,
            } => write!(
                f,
                "attempt budget {budget_secs}s is outside its ceiling of \
                 {ceiling_secs}s: the numbers describe two different cages"
            ),
            Self::EmptyHistoryWindow => {
                write!(
                    f,
                    "size history window is empty: a zero-capacity window holds no distribution"
                )
            }
            Self::MultiplierBelowTwo { multiplier } => write!(
                f,
                "decomposition multiplier {multiplier} is below 2: a threshold \
                 under twice the rolling mean flags ordinary variance"
            ),
            Self::EmptyDomain { domain } => write!(
                f,
                "timeout domain {domain:?} names nothing: domain aggregation \
                 needs a name to count against"
            ),
        }
    }
}

impl std::error::Error for TimeoutPolicyError {}

#[cfg(test)]
mod tests {
    use super::*;

    /// The incident number: a 7-hour budget.
    const SEVEN_HOURS: u64 = 7 * 3600;
    /// The incident context: maximal.
    const MAX_CONTEXT: u64 = 131_072;

    // ---- rule 1: escalate once, then too-large ----

    #[test]
    fn a_first_timeout_below_the_ceiling_escalates_exactly_to_the_ceiling() {
        let attempt = TimeoutAttempt::new(3600, SEVEN_HOURS, false).expect("valid attempt");
        assert_eq!(
            redispatch_after_timeout(&attempt),
            TimeoutDisposition::Escalate {
                budget_secs: SEVEN_HOURS
            },
            "the first cut-off attempt earns the whole ceiling, and only that"
        );
    }

    #[test]
    fn a_first_timeout_at_the_ceiling_is_too_large_with_nothing_left_to_escalate_to() {
        // The incident: the run had the full 7-hour budget, held maximal
        // context, and was still unfinished at the kill. There is no larger
        // budget, so the answer is the label, not a third reservation.
        let attempt = TimeoutAttempt::new(SEVEN_HOURS, SEVEN_HOURS, false).expect("valid attempt");
        let disposition = redispatch_after_timeout(&attempt);
        assert_eq!(
            disposition,
            TimeoutDisposition::TooLarge {
                previous_budget_secs: SEVEN_HOURS
            }
        );
        assert!(disposition.stop_redispatching());
    }

    #[test]
    fn a_second_timeout_after_the_escalation_is_terminal_even_below_the_ceiling() {
        // The one escalation was used; a sub-ceiling budget on the second
        // attempt changes nothing — the item has had its "one more".
        let attempt = TimeoutAttempt::new(3600, SEVEN_HOURS, true).expect("valid attempt");
        let disposition = redispatch_after_timeout(&attempt);
        assert_eq!(
            disposition,
            TimeoutDisposition::TooLarge {
                previous_budget_secs: 3600
            }
        );
        assert!(disposition.stop_redispatching());
    }

    #[test]
    fn the_too_large_disposition_carries_the_label_and_stops_the_queue() {
        let disposition = TimeoutDisposition::TooLarge {
            previous_budget_secs: SEVEN_HOURS,
        };
        assert_eq!(disposition.label(), Some(TOO_LARGE_LABEL));
        assert_eq!(TOO_LARGE_LABEL, "autospec:too-large");
        assert!(disposition.stop_redispatching());
        assert_eq!(disposition.next_budget_secs(), None);
        assert!(!TimeoutDisposition::Escalate {
            budget_secs: SEVEN_HOURS
        }
        .stop_redispatching());
    }

    #[test]
    fn the_terminal_disposition_is_idempotent() {
        // Deciding "stop" again must keep saying stop: the disposition is a
        // state, so a runner that re-queries cannot resurrect the queue.
        let attempt = TimeoutAttempt::new(SEVEN_HOURS, SEVEN_HOURS, true).expect("valid attempt");
        assert_eq!(
            redispatch_after_timeout(&attempt),
            redispatch_after_timeout(&attempt)
        );
    }

    #[test]
    fn an_attempt_with_no_budget_is_a_misconfiguration_not_a_timeout() {
        let error = TimeoutAttempt::new(0, SEVEN_HOURS, false).unwrap_err();
        assert!(matches!(error, TimeoutPolicyError::BudgetOutOfRange { .. }));
        assert!(
            error.to_string().contains("0s") && error.to_string().contains("25200s"),
            "both numbers, because a message with one of them cannot be acted on: {error}"
        );
    }

    #[test]
    fn an_attempt_whose_budget_exceeds_the_ceiling_is_refused_with_both_numbers() {
        let error = TimeoutAttempt::new(SEVEN_HOURS, 3600, false).unwrap_err();
        assert!(matches!(error, TimeoutPolicyError::BudgetOutOfRange { .. }));
        assert!(
            error.to_string().contains("25200s") && error.to_string().contains("3600s"),
            "{error}"
        );
    }

    #[test]
    fn a_zero_ceiling_is_refused() {
        assert!(matches!(
            TimeoutAttempt::new(3600, 0, false).unwrap_err(),
            TimeoutPolicyError::ZeroCeiling
        ));
    }

    // ---- rule 2: the rolling mean, not a constant ----

    #[test]
    fn a_run_beyond_the_multiplier_of_the_rolling_mean_is_a_candidate() {
        let mut history = SizeHistory::new(8).expect("window");
        for secs in [900, 1000, 1100] {
            history.record(secs);
        }
        // mean 1000, multiplier 3 -> threshold 3000; 3001 is beyond it.
        assert_eq!(
            decomposition_candidate(3001, &history, 3).expect("decides"),
            SizeSignal::Candidate {
                agent_secs: 3001,
                threshold_secs: 3000
            }
        );
    }

    #[test]
    fn a_run_at_the_threshold_is_the_edge_of_the_distribution_not_beyond_it() {
        let mut history = SizeHistory::new(8).expect("window");
        for secs in [900, 1000, 1100] {
            history.record(secs);
        }
        // Exactly 3x the mean: the comparison is strict, so this is ordinary.
        assert_eq!(
            decomposition_candidate(3000, &history, 3).expect("decides"),
            SizeSignal::Ordinary {
                agent_secs: 3000,
                threshold_secs: 3000
            }
        );
    }

    #[test]
    fn an_empty_history_is_no_history_not_a_constant_in_disguise() {
        let history = SizeHistory::new(8).expect("window");
        assert_eq!(
            decomposition_candidate(SEVEN_HOURS, &history, 3).expect("decides"),
            SizeSignal::NoHistory {
                agent_secs: SEVEN_HOURS
            },
            "without observations there is no distribution and no invented \
             600s constant to stand in for it"
        );
    }

    #[test]
    fn the_window_keeps_the_recent_observations_and_evicts_the_old() {
        let mut history = SizeHistory::new(3).expect("window");
        for secs in [100, 200, 300, 400] {
            history.record(secs);
        }
        assert_eq!(history.len(), 3);
        // 100 is gone: the mean of 200, 300, 400 is 300, not 250.
        assert_eq!(history.rolling_mean_secs(), Some(300));
    }

    #[test]
    fn the_rolling_mean_is_floored() {
        let mut history = SizeHistory::new(2).expect("window");
        history.record(1000);
        history.record(1001);
        assert_eq!(history.rolling_mean_secs(), Some(1000));
    }

    #[test]
    fn the_distribution_not_the_constant_decides_what_is_large() {
        // A 2000s run is 3x the old 600s constant, but against a distribution
        // whose mean is 900s with multiplier 3 the threshold is 2700s: this
        // run is ordinary variance, not the large mode.
        let mut history = SizeHistory::new(4).expect("window");
        for secs in [800, 900, 900, 1000] {
            history.record(secs);
        }
        assert_eq!(
            decomposition_candidate(2000, &history, 3).expect("decides"),
            SizeSignal::Ordinary {
                agent_secs: 2000,
                threshold_secs: 2700
            }
        );
    }

    #[test]
    fn a_sub_double_multiplier_is_refused() {
        let history = SizeHistory::new(4).expect("window");
        let error = decomposition_candidate(100, &history, 1).unwrap_err();
        assert!(matches!(
            error,
            TimeoutPolicyError::MultiplierBelowTwo { multiplier: 1 }
        ));
        assert!(decomposition_candidate(100, &history, 2).is_ok());
    }

    #[test]
    fn a_zero_capacity_window_is_refused() {
        assert!(matches!(
            SizeHistory::new(0).unwrap_err(),
            TimeoutPolicyError::EmptyHistoryWindow
        ));
    }

    // ---- rule 3: the message carries the resources ----

    #[test]
    fn the_timeout_message_names_the_wallclock_and_the_context() {
        assert_eq!(
            timeout_message(SEVEN_HOURS, MAX_CONTEXT),
            "TIMEOUT after 25200s at 131072 context",
            "the incident line: both numbers, because a message with one of \
             them cannot be acted on"
        );
    }

    #[test]
    fn the_timeout_message_keeps_both_numbers_for_small_runs() {
        assert_eq!(
            timeout_message(605, 8192),
            "TIMEOUT after 605s at 8192 context",
            "an early stall is distinguishable from a full-budget exhaustion \
             by the same two fields, not by a different line format"
        );
    }

    // ---- rule 4: domain aggregation before splitting ----

    #[test]
    fn a_single_domain_timeout_splits_the_issue_not_the_domain() {
        // The list includes the timeout just observed: one entry for this
        // domain means one timeout means the issue is the unit.
        let target =
            decomposition_target("metabolite-lookup", &[String::from("metabolite-lookup")])
                .expect("decides");
        assert_eq!(
            target,
            DecompositionTarget::Issue {
                domain: String::from("metabolite-lookup")
            }
        );
    }

    #[test]
    fn a_second_timeout_in_the_domain_makes_the_domain_the_unit() {
        // Two timeouts in the same domain: splitting the issue again would
        // re-split work the domain has already shown it cannot absorb.
        let target = decomposition_target(
            "metabolite-lookup",
            &[
                String::from("metabolite-lookup"),
                String::from("metabolite-lookup"),
            ],
        )
        .expect("decides");
        assert_eq!(
            target,
            DecompositionTarget::Domain {
                domain: String::from("metabolite-lookup"),
                timeouts: 2
            }
        );
    }

    #[test]
    fn other_domains_do_not_aggregate_into_this_one() {
        let target = decomposition_target(
            "metabolite-lookup",
            &[
                String::from("metabolite-lookup"),
                String::from("report-export"),
                String::from("report-export"),
                String::from("report-export"),
            ],
        )
        .expect("decides");
        assert_eq!(
            target,
            DecompositionTarget::Issue {
                domain: String::from("metabolite-lookup")
            },
            "saturated elsewhere is not saturated here"
        );
    }

    #[test]
    fn an_unnamed_domain_is_refused() {
        let error = decomposition_target("", &[]).unwrap_err();
        assert!(matches!(error, TimeoutPolicyError::EmptyDomain { .. }));
        let error = decomposition_target(
            "metabolite-lookup",
            &["metabolite-lookup".to_string(), String::from("  ")],
        )
        .unwrap_err();
        assert!(matches!(error, TimeoutPolicyError::EmptyDomain { .. }));
    }
}
