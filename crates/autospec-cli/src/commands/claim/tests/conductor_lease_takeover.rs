//! The conductor pre-check must displace an owner whose lease has expired.
//!
//! A dead worker cannot release its own claim, and `claim release` validates the
//! caller's identity, so an unconditional refusal wedged the issue permanently.

use super::super::{conductor_claim_owner_holds_lease, RunStateRecord};

fn owner_record(updated_at: &str, ttl_seconds: u64, step: &str) -> RunStateRecord {
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
