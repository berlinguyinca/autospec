//! Lease and liveness behaviour (#3563).
//!
//! A claim must expire unless progress renews it, and a stall must be decided by
//! liveness rather than by the absence of output.

use autospec_core::stall::{
    progress_revision, IssueLease, LeasePolicy, Liveness, LivenessMonitor, LivenessSample, Renewal,
};

fn policy() -> LeasePolicy {
    LeasePolicy {
        duration_secs: 900,
        renewal_interval_secs: 120,
    }
}

fn lease() -> IssueLease {
    IssueLease::take(
        3289,
        "worker-7",
        "feat/issue-3289",
        "Qwen3.8-27B Q8",
        1_000,
        policy(),
    )
    .expect("valid policy takes a lease")
}

#[test]
fn lease_expires_when_nothing_renews_it() {
    let lease = lease();
    assert_eq!(lease.remaining_secs(1_000), 900);
    assert!(!lease.is_expired_at(1_799));
    // 903s into the run — the interval the gateway#7 attempt ran before it was
    // killed — the lease is still held.
    assert!(!lease.is_expired_at(1_899));
    assert!(lease.is_expired_at(1_900));
    assert_eq!(lease.held_secs(1_900), 900);
}

#[test]
fn observed_progress_renews_the_lease() {
    let mut lease = lease();
    let renewal = lease.observe_progress(1_500, 42);
    assert_eq!(renewal, Renewal::Renewed { expires_at: 2_400 });
    assert_eq!(lease.expires_at, 2_400);
    assert!(!lease.is_expired_at(1_900));
}

#[test]
fn a_lease_is_not_renewed_without_progress() {
    let mut lease = lease();
    // Same revision twice: the agent has moved nowhere, so the expiry stands.
    assert_eq!(
        lease.observe_progress(1_500, 7),
        Renewal::Renewed { expires_at: 2_400 }
    );
    assert_eq!(lease.observe_progress(1_600, 7), Renewal::NoProgress);
    assert_eq!(lease.expires_at, 2_400);
    assert_eq!(lease.observe_progress(2_399, 7), Renewal::NoProgress);
    assert!(lease.is_expired_at(2_400));
}

#[test]
fn renewal_is_rate_limited_by_the_interval() {
    let mut lease = lease();
    assert_eq!(lease.observe_progress(1_060, 5), Renewal::TooSoon);
    assert_eq!(lease.expires_at, 1_900);
    // Progress seen while rate-limited is not lost: it renews the moment the
    // interval elapses.
    assert_eq!(
        lease.observe_progress(1_120, 5),
        Renewal::Renewed { expires_at: 2_020 }
    );
    assert_eq!(lease.observe_progress(1_200, 9), Renewal::TooSoon);
    assert_eq!(
        lease.observe_progress(1_240, 9),
        Renewal::Renewed { expires_at: 2_140 }
    );
}

#[test]
fn an_expired_lease_cannot_be_renewed() {
    let mut lease = lease();
    assert_eq!(lease.observe_progress(5_000, 99), Renewal::Expired);
    assert_eq!(lease.expires_at, 1_900);
}

#[test]
fn commits_count_as_progress() {
    // The check that reported a committing agent as producing nothing must not
    // be reproduced here: a commit moves the revision even with a quiet tree.
    let quiet = progress_revision(0, 4_000, 0);
    let committing = progress_revision(1, 4_000, 0);
    assert!(committing > quiet);
    let transcript_growth = progress_revision(0, 9_000, 0);
    assert!(transcript_growth > quiet);
}

#[test]
fn an_unrenewable_policy_is_rejected_at_claim_time() {
    let long_interval = LeasePolicy {
        duration_secs: 60,
        renewal_interval_secs: 60,
    };
    assert!(long_interval.validate().is_err());
    assert!(IssueLease::take(
        1,
        "w",
        "b",
        "m",
        0,
        LeasePolicy {
            duration_secs: 0,
            renewal_interval_secs: 1,
        }
    )
    .is_err());
    assert!(IssueLease::take(0, "w", "b", "m", 0, policy()).is_err());
    assert!(policy().validate().is_ok());
}

#[test]
fn lease_policy_reads_its_environment() {
    std::env::set_var("AUTOSPEC_ISSUE_LEASE_SECS", "1234");
    std::env::set_var("AUTOSPEC_ISSUE_LEASE_RENEW_SECS", "300");
    assert_eq!(
        LeasePolicy::from_env(),
        LeasePolicy {
            duration_secs: 1234,
            renewal_interval_secs: 300
        }
    );
    std::env::remove_var("AUTOSPEC_ISSUE_LEASE_SECS");
    std::env::remove_var("AUTOSPEC_ISSUE_LEASE_RENEW_SECS");
    assert_eq!(LeasePolicy::from_env(), LeasePolicy::default());
}

fn sample(at: u64, transcript: u64, output: u64) -> LivenessSample {
    LivenessSample::new(at, transcript, output)
}

#[test]
fn a_growing_transcript_is_deliberation_not_a_stall() {
    let mut monitor = LivenessMonitor::new(1_800, sample(0, 1_000, 60));
    // Reading for a long time before the first edit: no output at all, and not
    // a stall, because the transcript keeps growing.
    for (at, transcript) in [
        (600, 2_000),
        (1_200, 5_000),
        (1_800, 9_000),
        (3_600, 40_000),
    ] {
        assert_eq!(
            monitor.observe(sample(at, transcript, 60)),
            Liveness::Deliberating,
            "transcript growth at {at}s must read as liveness"
        );
    }
    assert!(monitor.last_sample().transcript_bytes > 1_000);
}

#[test]
fn output_growth_alone_counts_as_liveness() {
    let mut monitor = LivenessMonitor::new(1_800, sample(0, 1_000, 0));
    assert_eq!(
        monitor.observe(sample(600, 1_000, 120)),
        Liveness::Producing
    );
    assert!(monitor.silence_secs(600) == 0);
}

#[test]
fn silence_on_both_signals_is_quiet_then_hung() {
    let mut monitor = LivenessMonitor::new(1_800, sample(0, 1_000, 60));
    assert_eq!(monitor.observe(sample(900, 1_000, 60)), Liveness::Quiet);
    assert!(!monitor.observe(sample(900, 1_000, 60)).is_stalled());
    // The gateway#7 hang: transcript frozen, output frozen at 60 bytes.
    assert_eq!(monitor.observe(sample(1_800, 1_000, 60)), Liveness::Hung);
    assert!(monitor.observe(sample(2_400, 1_000, 60)).is_stalled());
    assert_eq!(monitor.silence_secs(2_400), 2_400);
}

#[test]
fn liveness_predicates_agree_with_the_verdict() {
    assert!(Liveness::Deliberating.is_live());
    assert!(Liveness::Producing.is_live());
    assert!(Liveness::Quiet.is_live());
    assert!(!Liveness::Hung.is_live());
    assert!(Liveness::Hung.is_stalled());
}
