//! Issue #3722: a task that times out at maximal context and maximal budget
//! is a decomposition problem, not a budget problem.
//!
//! These tests pin the four rules the monitor applies to a timed-out issue
//! run: escalate once then decompose, compare against the fleet
//! distribution, report the resources in the timeout message, and check the
//! domain before splitting the issue.

use autospec_core::autonomous::timeout_triage::{
    decomposition_candidates, decomposition_threshold_secs, domain_signal, rolling_mean_secs,
    timeout_message, timeout_response, TimeoutResponse, DEFAULT_MULTIPLIER, DEFAULT_WINDOW,
    TOO_LARGE_LABEL,
};

/// The #3722 scenario: an issue times out at the default 7-hour budget, the
/// monitor raises it to the 14-hour reservation, and it times out again.
/// The second timeout must decompose, not re-raise.
#[test]
fn second_timeout_at_reservation_decomposes_instead_of_re_raising() {
    let first = timeout_response(1, 25_200, 50_400).expect("first timeout is valid");
    let raise = TimeoutResponse::RaiseBudget {
        current_secs: 25_200,
        next_secs: 50_400,
    };
    assert_eq!(first, raise);
    assert!(!raise.stop_redispatch());

    let second = timeout_response(2, 50_400, 50_400).expect("second timeout is valid");
    assert_eq!(second, TimeoutResponse::TooLarge);
    assert!(second.stop_redispatch());
    assert_eq!(TOO_LARGE_LABEL, "autospec:too-large");
}

#[test]
fn timeout_message_reports_both_resources() {
    // The exact line from the issue: 25200s at 131072 context.
    assert_eq!(
        timeout_message(25_200, 131_072),
        "TIMEOUT after 25200s at 131072 context"
    );
    // A run that died early with a small context must stay distinguishable.
    assert_eq!(
        timeout_message(3600, 8192),
        "TIMEOUT after 3600s at 8192 context"
    );
}

#[test]
fn rolling_mean_drives_the_decomposition_threshold() {
    // Short fleet with one 7-hour outlier: the outlier sits far above the
    // rolling mean and is the decomposition candidate.
    let values = [900u64, 1_000, 1_200, 25_200];
    assert_eq!(rolling_mean_secs(&values, DEFAULT_WINDOW), Some(7_075));
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
fn rolling_window_ignores_stale_runs() {
    // Old long runs must not inflate the mean of the current window.
    let values: Vec<u64> = std::iter::repeat_n(25_200, 50)
        .chain(std::iter::repeat_n(1_000, 20))
        .collect();
    assert_eq!(
        rolling_mean_secs(&values, DEFAULT_WINDOW),
        Some(1_000),
        "window 20 holds only the recent short runs"
    );
    assert!(decomposition_candidates(&values, DEFAULT_WINDOW, DEFAULT_MULTIPLIER).is_empty());
}

#[test]
fn repeated_timeouts_in_one_domain_flag_the_domain() {
    let domains = [
        "metabolomics",
        "metabolomics",
        "docs",
        "metabolomics",
        "rag",
    ];
    assert_eq!(domain_signal(&domains), vec!["metabolomics"]);
}

#[test]
fn single_timeout_per_domain_is_not_a_domain_signal() {
    assert_eq!(domain_signal(&["a", "b", "c"]), Vec::<String>::new());
    assert_eq!(domain_signal(&[]), Vec::<String>::new());
}

#[test]
fn empty_domain_names_carry_no_evidence() {
    assert_eq!(
        domain_signal(&["", "  ", "metabolomics", "metabolomics"]),
        vec!["metabolomics"]
    );
}

#[test]
fn a_lone_run_has_no_distribution_to_compare_against() {
    assert_eq!(
        decomposition_candidates(&[25_200], DEFAULT_WINDOW, DEFAULT_MULTIPLIER),
        Vec::<usize>::new()
    );
    assert_eq!(
        decomposition_threshold_secs(&[], DEFAULT_WINDOW, DEFAULT_MULTIPLIER),
        None
    );
}
