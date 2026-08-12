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

/// Run an idempotent `gh` read, retrying a failed attempt before giving up.
///
/// Every un-retried `gh` invocation is a single point of failure for the conductor:
/// one handshake that comes back unusable under concurrency kills the process, and
/// the claim it was holding then wedges the issue. Reads only — retrying a mutation
/// would not be safe.
pub(super) fn read_gh_with_retry(
    arguments: &[&str],
    action: &str,
) -> Result<std::process::Output, super::CommandFailure> {
    let attempts = claim_retry_attempts();
    let sleep_ms = claim_retry_sleep_ms();
    let mut last_error = String::new();
    for attempt in 0..attempts {
        let output = std::process::Command::new("gh")
            .args(arguments)
            .output()
            .map_err(|error| {
                super::CommandFailure::transient(format!("could not {action}: {error}"))
            })?;
        if output.status.success() {
            return Ok(output);
        }
        last_error = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if attempt + 1 < attempts {
            std::thread::sleep(std::time::Duration::from_millis(sleep_ms));
        }
    }
    Err(super::CommandFailure::transient(format!(
        "{action} failed after {attempts} attempts: {last_error}"
    )))
}

fn env_u64(name: &str, fallback: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(fallback)
}

/// How many times an idempotent GitHub call is attempted before it is a failure.
pub(super) fn claim_retry_attempts() -> u64 {
    env_u64("AUTOSPEC_GH_API_RETRIES", 3)
}

/// How long to wait between those attempts.
pub(super) fn claim_retry_sleep_ms() -> u64 {
    env_u64("AUTOSPEC_CLAIM_RETRY_SLEEP_MS", 1_000)
}
