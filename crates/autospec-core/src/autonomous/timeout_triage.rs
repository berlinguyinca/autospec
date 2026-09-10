//! Timeout triage for timed-out issue runs (#3722): decomposition, not
//! bigger budgets.
//!
//! A task that times out at maximal context and maximal budget is a
//! decomposition problem, not a budget problem. The monitor's response to a
//! timeout is decided here as pure rules; the monitor supplies the recorded
//! numbers (`agent_secs`, the budget, the reservation, the context the run
//! died in) and acts on what these functions return.
//!
//! 1. **Escalate once, then decompose** ([`timeout_response`]). The first
//!    timeout raises the budget, capped at the reservation. A second timeout
//!    on the same issue once the budget has reached the reservation marks the
//!    issue [`TOO_LARGE_LABEL`] and stops re-dispatch — more wall time would
//!    not have finished the task.
//!
//! 2. **Compare against the distribution, not a constant**
//!    ([`decomposition_candidates`]). A run whose duration is at least
//!    [`DEFAULT_MULTIPLIER`] times the rolling mean
//!    ([`rolling_mean_secs`]) of recent fleet runs is a decomposition
//!    candidate, not a "needs more time" issue.
//!
//! 3. **Report the resources in the timeout message**
//!    ([`timeout_message`]). The line carries both the elapsed seconds and
//!    the context the run died in — `TIMEOUT after 25200s at 131072
//!    context` — so "ran out of time" and "ran out of context" stay
//!    distinguishable.
//!
//! 4. **Check the domain before splitting the issue**
//!    ([`domain_signal`]). When two or more timed-out issues share a domain,
//!    the domain is the thing to check first: the split is the wrong fix and
//!    the domain is what is broken.

use std::collections::BTreeMap;

/// Label applied to an issue that timed out twice with the budget at the
/// reservation. Issues carrying it are not re-dispatched.
pub const TOO_LARGE_LABEL: &str = "autospec:too-large";

/// Default size of the rolling window of fleet run durations.
pub const DEFAULT_WINDOW: usize = 20;

/// A run at least this many times the rolling mean is a decomposition
/// candidate.
pub const DEFAULT_MULTIPLIER: u64 = 2;

/// The monitor's response to a timeout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeoutResponse {
    /// Raise the budget and re-dispatch, once.
    RaiseBudget {
        current_secs: u64,
        /// The raised budget, never above the reservation.
        next_secs: u64,
    },
    /// The issue is too large for a single run: label it [`TOO_LARGE_LABEL`]
    /// and stop re-dispatching it.
    TooLarge,
}

impl TimeoutResponse {
    /// Whether the monitor must stop re-dispatching this issue.
    pub fn stop_redispatch(self) -> bool {
        matches!(self, TimeoutResponse::TooLarge)
    }
}

/// Decide the monitor's response to the `attempts`-th timeout of one issue.
///
/// `budget_secs` is the budget the run just timed out at and
/// `reservation_secs` the maximum budget the fleet reserves for an issue.
///
/// * Any timeout below the reservation raises the budget: doubled, capped at
///   the reservation. The raise is what the first timeout is for.
/// * A timeout at the reservation with no headroom left to raise is
///   [`TimeoutResponse::TooLarge`] — re-dispatching would spend another full
///   window and die the same death. This is the second-timeout case in
///   #3722: the budget is already at the generous end, so the task is
///   decomposed, not re-run.
pub fn timeout_response(
    attempts: u32,
    budget_secs: u64,
    reservation_secs: u64,
) -> Result<TimeoutResponse, String> {
    if attempts == 0 {
        return Err("attempts must be at least 1: a response needs a timeout".to_string());
    }
    if budget_secs == 0 {
        return Err("budget_secs must be positive".to_string());
    }
    if reservation_secs == 0 {
        return Err("reservation_secs must be positive".to_string());
    }
    if budget_secs > reservation_secs {
        return Err(format!(
            "budget_secs {budget_secs} exceeds reservation_secs {reservation_secs}"
        ));
    }
    if budget_secs < reservation_secs {
        return Ok(TimeoutResponse::RaiseBudget {
            current_secs: budget_secs,
            next_secs: budget_secs.saturating_mul(2).min(reservation_secs),
        });
    }
    Ok(TimeoutResponse::TooLarge)
}

/// The ceiling mean duration over the most recent `window` run durations, in
/// whole seconds (rounded up, exact integer arithmetic). `None` when there is
/// no window to average over: with no distribution there is no basis for a
/// decomposition claim.
pub fn rolling_mean_secs(values: &[u64], window: usize) -> Option<u64> {
    if window == 0 {
        return None;
    }
    let start = values.len().saturating_sub(window);
    let recent = &values[start..];
    if recent.is_empty() {
        return None;
    }
    let sum: u64 = recent.iter().sum();
    Some(sum.div_ceil(recent.len() as u64))
}

/// The decomposition threshold: a run at or above
/// [`rolling_mean_secs`] * `multiplier` is a candidate. `None` when the
/// window is empty or the multiplier is zero.
pub fn decomposition_threshold_secs(values: &[u64], window: usize, multiplier: u64) -> Option<u64> {
    if multiplier == 0 {
        return None;
    }
    rolling_mean_secs(values, window).and_then(|mean| mean.checked_mul(multiplier))
}

/// Indices of the runs that are decomposition candidates: at least
/// `multiplier` times the rolling mean of the window they sit in.
///
/// Only the same recent window the mean is computed over is evaluated, so a
/// stale outlier outside the window neither inflates the mean nor resurfaces
/// as a candidate. Indices are relative to the full slice. Uniform fleets
/// yield no candidates, and a lone run yields none — a single data point is
/// not a distribution.
pub fn decomposition_candidates(values: &[u64], window: usize, multiplier: u64) -> Vec<usize> {
    if window == 0 {
        return Vec::new();
    }
    let Some(threshold) = decomposition_threshold_secs(values, window, multiplier) else {
        return Vec::new();
    };
    let start = values.len().saturating_sub(window);
    values[start..]
        .iter()
        .enumerate()
        .filter(|(_, value)| **value >= threshold)
        .map(|(offset, _)| start + offset)
        .collect()
}

/// The timeout line the monitor logs and reports, carrying both resources
/// the run died with: `TIMEOUT after 25200s at 131072 context`.
pub fn timeout_message(agent_secs: u64, context_tokens: u64) -> String {
    format!("TIMEOUT after {agent_secs}s at {context_tokens} context")
}

/// Domains shared by two or more timed-out issues, sorted.
///
/// These are the domains to check before splitting any single issue: a
/// repeated timeout inside one domain means the domain is what is broken,
/// and splitting the issue is the wrong fix. Empty domain names are
/// ignored — a run with no recorded domain is not evidence about a domain.
pub fn domain_signal(domains: &[&str]) -> Vec<String> {
    let mut counts: BTreeMap<&str, u32> = BTreeMap::new();
    for domain in domains {
        if domain.trim().is_empty() {
            continue;
        }
        *counts.entry(domain).or_insert(0) += 1;
    }
    counts
        .into_iter()
        .filter(|(_, count)| *count >= 2)
        .map(|(domain, _)| domain.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_timeout_below_reservation_raises_capped_at_reservation() {
        let response = timeout_response(1, 25_200, 50_400).expect("valid inputs");
        assert_eq!(
            response,
            TimeoutResponse::RaiseBudget {
                current_secs: 25_200,
                next_secs: 50_400,
            }
        );
        assert!(!response.stop_redispatch());
    }

    #[test]
    fn raise_is_doubled_and_capped() {
        let response = timeout_response(1, 30_000, 50_400).expect("valid inputs");
        assert_eq!(
            response,
            TimeoutResponse::RaiseBudget {
                current_secs: 30_000,
                next_secs: 50_400,
            }
        );
        let headroom = timeout_response(1, 10_000, 50_400).expect("valid inputs");
        assert_eq!(
            headroom,
            TimeoutResponse::RaiseBudget {
                current_secs: 10_000,
                next_secs: 20_000,
            }
        );
    }

    #[test]
    fn second_timeout_at_reservation_is_too_large() {
        let response = timeout_response(2, 50_400, 50_400).expect("valid inputs");
        assert_eq!(response, TimeoutResponse::TooLarge);
        assert!(response.stop_redispatch());
        assert_eq!(TOO_LARGE_LABEL, "autospec:too-large");
    }

    #[test]
    fn first_timeout_already_at_reservation_has_no_raise_left() {
        let response = timeout_response(1, 50_400, 50_400).expect("valid inputs");
        assert_eq!(response, TimeoutResponse::TooLarge);
        assert!(response.stop_redispatch());
    }

    #[test]
    fn second_timeout_with_headroom_still_raises() {
        let response = timeout_response(2, 25_200, 50_400).expect("valid inputs");
        assert_eq!(
            response,
            TimeoutResponse::RaiseBudget {
                current_secs: 25_200,
                next_secs: 50_400,
            }
        );
    }

    #[test]
    fn invalid_inputs_are_rejected() {
        assert!(timeout_response(0, 25_200, 50_400).is_err());
        assert!(timeout_response(1, 0, 50_400).is_err());
        assert!(timeout_response(1, 25_200, 0).is_err());
        assert!(timeout_response(1, 50_400, 25_200).is_err());
    }

    #[test]
    fn timeout_message_carries_both_resources() {
        assert_eq!(
            timeout_message(25_200, 131_072),
            "TIMEOUT after 25200s at 131072 context"
        );
    }

    #[test]
    fn rolling_mean_uses_the_recent_window_and_rounds_up() {
        assert_eq!(rolling_mean_secs(&[], 20), None);
        assert_eq!(rolling_mean_secs(&[10, 11], 20), Some(11));
        // Window 3 takes the last three values, not the whole slice.
        assert_eq!(rolling_mean_secs(&[1, 1, 1, 1, 100], 3), Some(34));
        assert_eq!(rolling_mean_secs(&[10, 11], 0), None);
    }

    #[test]
    fn outlier_run_is_a_decomposition_candidate() {
        let values = [1_000u64, 1_200, 900, 25_200];
        assert_eq!(
            decomposition_threshold_secs(&values, DEFAULT_WINDOW, DEFAULT_MULTIPLIER),
            Some(14_150)
        );
        assert_eq!(
            decomposition_candidates(&values, DEFAULT_WINDOW, DEFAULT_MULTIPLIER),
            vec![3]
        );
    }

    #[test]
    fn uniform_fleet_yields_no_candidates() {
        let values = [1_000u64, 1_000, 1_000, 1_000];
        assert!(decomposition_candidates(&values, DEFAULT_WINDOW, DEFAULT_MULTIPLIER).is_empty());
    }

    #[test]
    fn a_lone_run_is_not_a_distribution() {
        assert_eq!(
            decomposition_candidates(&[25_200], DEFAULT_WINDOW, DEFAULT_MULTIPLIER),
            Vec::<usize>::new()
        );
        assert_eq!(
            decomposition_threshold_secs(&[25_200], DEFAULT_WINDOW, 0),
            None
        );
    }
}
