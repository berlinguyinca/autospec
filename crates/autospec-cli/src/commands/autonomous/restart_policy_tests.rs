// Supervisor restart policy — berlinguyinca/autospec#3012 section 1.
//
// The incident: a conductor exited about a second after launch, the supervisor relaunched it
// every repair interval, and it did that 1017 times over a few hours while logging
// `conductor=running` each cycle. Three properties keep that from recurring, and each is
// asserted below: an immediate exit is counted rather than trusted, repeated ones trip a
// breaker, and the wait grows while restarts keep failing.

use super::supervisor::{RestartPolicy, RESTART_BACKOFF_MAX_SECS, RESTART_FAST_EXIT_LIMIT};

#[test]
fn a_healthy_restart_resets_the_counter_and_keeps_the_configured_cadence() {
    let mut policy = RestartPolicy::default();
    policy.record_restart(false);
    policy.record_restart(false);
    assert_eq!(policy.consecutive_fast_exits, 2);

    policy.record_restart(true);

    assert_eq!(
        policy.consecutive_fast_exits, 0,
        "a survivor clears the streak"
    );
    assert!(policy.may_restart());
    assert_eq!(
        policy.delay_secs(5),
        5,
        "a healthy supervisor keeps its interval"
    );
}

#[test]
fn repeated_immediate_exits_trip_the_breaker_instead_of_restarting_forever() {
    let mut policy = RestartPolicy::default();
    for attempt in 1..RESTART_FAST_EXIT_LIMIT {
        policy.record_restart(false);
        assert!(
            policy.may_restart(),
            "attempt {attempt} is below the limit and should still be retried"
        );
    }
    policy.record_restart(false);

    assert!(
        !policy.may_restart(),
        "after {RESTART_FAST_EXIT_LIMIT} immediate exits the supervisor must stop relaunching"
    );
}

#[test]
fn the_breaker_stays_tripped() {
    let mut policy = RestartPolicy::default();
    for _ in 0..RESTART_FAST_EXIT_LIMIT {
        policy.record_restart(false);
    }
    assert!(!policy.may_restart());

    // A later observation must not silently re-arm restarts: the operator resolves the cause and
    // restarts explicitly. Quarantine that un-quarantines itself is how a storm resumes.
    policy.record_restart(true);

    assert!(
        !policy.may_restart(),
        "quarantine must require operator action to clear"
    );
}

#[test]
fn the_backoff_grows_and_is_capped() {
    let mut policy = RestartPolicy::default();
    let mut previous = policy.delay_secs(5);
    for _ in 0..4 {
        policy.record_restart(false);
        let delay = policy.delay_secs(5);
        assert!(delay > previous, "backoff must grow: {previous} -> {delay}");
        previous = delay;
    }

    let mut runaway = RestartPolicy::default();
    for _ in 0..64 {
        runaway.record_restart(false);
    }
    assert_eq!(
        runaway.delay_secs(300),
        RESTART_BACKOFF_MAX_SECS,
        "the cap keeps a quarantine decision reachable in bounded time"
    );
    assert_eq!(
        runaway.delay_secs(0),
        0,
        "a zero interval cannot be multiplied into a stall"
    );
}

#[test]
fn the_counter_saturates_rather_than_overflowing() {
    let mut policy = RestartPolicy {
        consecutive_fast_exits: u32::MAX,
        quarantined: true,
    };
    policy.record_restart(false);
    assert_eq!(policy.consecutive_fast_exits, u32::MAX);
}
