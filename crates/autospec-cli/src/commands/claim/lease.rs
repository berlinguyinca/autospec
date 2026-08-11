//! Whether a recorded claim owner still holds its lease.

use autospec_core::claim::RunStateRecord;

use super::{parse_iso_timestamp, unix_now};

/// Whether the recorded owner still holds its lease, and so must not be displaced.
///
/// This is the same test `acquire_record` applies before refusing a claim: an owner
/// mid heartbeat-publish is protected, and otherwise the lease must not have aged
/// past its TTL. Without it the conductor pre-check refused every recorded owner
/// outright, including one whose process was long dead, and the issue stayed wedged
/// forever — a single failed GitHub call was enough to strand it, because the dead
/// owner could never release its own claim.
pub(super) fn conductor_claim_owner_holds_lease(record: &RunStateRecord) -> bool {
    record.step.starts_with("heartbeat-publishing:")
        || !server_lease_is_stale(&record.updated_at, record.ttl_seconds)
}

/// Whether a server-recorded lease is still inside its TTL.
pub(super) fn server_lease_is_fresh(server_timestamp: &str, ttl_seconds: u64) -> bool {
    let Some(updated_at) = parse_iso_timestamp(server_timestamp) else {
        return false;
    };
    unix_now()
        .map(|now| now.saturating_sub(updated_at) <= ttl_seconds)
        .unwrap_or(false)
}

/// Whether a server-recorded lease has aged past its TTL. An unreadable timestamp
/// is not stale, so a malformed record fails closed and keeps its owner.
pub(super) fn server_lease_is_stale(server_timestamp: &str, ttl_seconds: u64) -> bool {
    let Some(updated_at) = parse_iso_timestamp(server_timestamp) else {
        return false;
    };
    unix_now()
        .map(|now| now.saturating_sub(updated_at) > ttl_seconds)
        .unwrap_or(false)
}
