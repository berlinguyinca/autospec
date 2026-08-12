//! The conductor pre-check must displace an owner whose lease has expired.
//!
//! A dead worker cannot release its own claim, and `claim release` validates the
//! caller's identity, so an unconditional refusal wedged the issue permanently.

use super::super::lease::conductor_claim_owner_holds_lease;
use autospec_core::claim::RunStateRecord;

pub(super) fn owner_record(updated_at: &str, ttl_seconds: u64, step: &str) -> RunStateRecord {
    RunStateRecord::new(
        "owner/repo",
        42,
        "rust-foreground-conductor-dead-1785286182924880200",
        "claimed",
        "feat/autonomous-issue-42".to_string(),
        "",
        step,
        Vec::new(),
        "2026-07-29T00:49:45Z",
        updated_at,
        ttl_seconds,
    )
}

#[test]
fn an_owner_whose_lease_has_expired_no_longer_holds_it() {
    let record = owner_record("2026-07-29T00:49:45Z", 10800, "verification");

    assert!(
        !conductor_claim_owner_holds_lease(&record),
        "a claim last touched in July must not block a fresh worker forever"
    );
}

#[test]
fn an_owner_still_inside_its_ttl_keeps_the_lease() {
    let updated_at = super::super::utc_now_iso().expect("current timestamp");
    let record = owner_record(&updated_at, 10800, "verification");

    assert!(
        conductor_claim_owner_holds_lease(&record),
        "a live worker one minute into a three-hour lease must not be displaced"
    );
}

#[test]
fn an_owner_mid_heartbeat_publish_keeps_the_lease_even_when_stale() {
    let record = owner_record(
        "2026-07-29T00:49:45Z",
        10800,
        "heartbeat-publishing:verification",
    );

    assert!(
        conductor_claim_owner_holds_lease(&record),
        "a publish in flight is protected regardless of the recorded timestamp"
    );
}

#[test]
fn an_unparseable_timestamp_keeps_the_lease() {
    let record = owner_record("not-a-timestamp", 10800, "verification");

    assert!(
        conductor_claim_owner_holds_lease(&record),
        "an unreadable record must fail closed and keep the owner"
    );
}

mod requeue {
    use super::super::super::lease::{
        claim_is_abandoned, quarantine_abandoned_claim_generation_with,
    };
    use super::super::super::{ClaimRefAdvance, ClaimRefHead};
    use super::owner_record;
    use crate::commands::claim::utc_now_iso;

    fn record(state: &str, updated_at: &str) -> autospec_core::claim::RunStateRecord {
        let mut record = owner_record(updated_at, 10800, "verification");
        record.state = state.to_string();
        record
    }

    fn lose_generation(
        expected: Option<&ClaimRefHead>,
        successor: &autospec_core::claim::RunStateRecord,
    ) -> Result<ClaimRefAdvance, crate::commands::CommandFailure> {
        assert_eq!(
            expected.map(|head| head.oid.as_str()),
            Some("expired-generation")
        );
        assert_eq!(successor.state, "available");
        Ok(ClaimRefAdvance::Lost)
    }

    #[test]
    fn a_missing_claim_record_is_abandoned() {
        assert!(claim_is_abandoned(None, false), "nothing owns the issue");
    }

    #[test]
    fn a_released_claim_is_abandoned() {
        let live = utc_now_iso().expect("timestamp");
        for state in ["available", "released", "retryable", "failed"] {
            assert!(
                claim_is_abandoned(Some(&record(state, &live)), true),
                "{state} leaves the issue unowned even with a fresh timestamp"
            );
        }
    }

    #[test]
    fn an_expired_lease_is_abandoned() {
        assert!(claim_is_abandoned(
            Some(&record("claimed", "2026-07-29T00:49:45Z")),
            false
        ));
    }

    #[test]
    fn a_live_claim_is_left_alone() {
        let live = utc_now_iso().expect("timestamp");

        assert!(
            !claim_is_abandoned(Some(&record("claimed", &live)), true),
            "a worker inside its lease must keep the issue"
        );
    }

    #[test]
    fn a_merged_claim_is_left_alone() {
        assert!(
            !claim_is_abandoned(Some(&record("merged", "2026-07-29T00:49:45Z")), false),
            "merged work is finished, not abandoned"
        );
    }

    #[test]
    fn a_concurrent_lease_renewal_prevents_label_requeue() {
        let selected = ClaimRefHead {
            oid: "expired-generation".to_string(),
            generation: "generation-1".to_string(),
            record: record("claimed", "2026-07-29T00:49:45Z"),
        };
        let quarantined = quarantine_abandoned_claim_generation_with(
            "owner/repo",
            42,
            Some(selected),
            &mut lose_generation,
        )
        .expect("a lost compare-and-swap is not an error");

        assert!(
            quarantined.is_none(),
            "the renewing worker won, so the caller has no authority to mutate labels"
        );
    }

    /// The requeue path and the acquisition path must agree about ownership.
    ///
    /// They diverged once: requeue asked only the TTL clock while acquisition also
    /// asked whether the owner was alive. A dead owner's fresh lease then read as
    /// "owned" to requeue and "takeable" to acquisition, so the conductor was
    /// willing to take the issue but never saw it — the label kept it out of the
    /// candidate pool and it idled for hours beside work it could have done.
    #[test]
    fn an_owner_that_no_longer_holds_the_claim_is_always_abandoned() {
        let live = utc_now_iso().expect("timestamp");
        for (state, owner_holds, expected) in [
            ("claimed", true, false),
            ("claimed", false, true),
            ("merged", false, false),
            ("released", true, true),
        ] {
            assert_eq!(
                claim_is_abandoned(Some(&record(state, &live)), owner_holds),
                expected,
                "state={state} owner_holds={owner_holds} must match what acquisition decides"
            );
        }
    }
}
