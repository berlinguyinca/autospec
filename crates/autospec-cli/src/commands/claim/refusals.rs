//! The refusal constructors for `claim acquire`.
//!
//! Every one of these returns `ConductorClaimError::Deferred` with exit code 2 and a
//! single-line JSON object naming a `reason`. They live together because the set is the
//! command's machine-readable refusal vocabulary -- a caller matches on `reason`, so the
//! shapes have to stay consistent with each other -- and because `claim.rs` is past the
//! size ratchet and may not grow.

use super::{json_escape, ConductorClaimError};

pub(super) fn unavailable_claim<T>(
    issue: u64,
    repo: &str,
    worker_id: Option<&str>,
    reason: &str,
) -> Result<T, ConductorClaimError> {
    let worker_id = worker_id
        .map(|value| format!(",\"worker_id\":\"{}\"", json_escape(value)))
        .unwrap_or_default();
    Err(ConductorClaimError::Deferred {
        json: format!(
            "{{\"claimed\":false,\"issue\":{issue},\"repo\":\"{}\"{worker_id},\"reason\":\"{}\"}}",
            json_escape(repo),
            json_escape(reason),
        ),
        exit_code: 2,
    })
}

pub(super) fn heartbeat_publication_deferred<T>(
    issue: u64,
    repo: &str,
    worker_id: &str,
    claim_id: &str,
) -> Result<T, ConductorClaimError> {
    Err(ConductorClaimError::Deferred {
        json: format!(
            "{{\"claimed\":false,\"issue\":{issue},\"repo\":\"{}\",\"worker_id\":\"{}\",\"claim_id\":\"{}\",\"reason\":\"heartbeat_write_failed\"}}",
            json_escape(repo),
            json_escape(worker_id),
            json_escape(claim_id),
        ),
        exit_code: 2,
    })
}

/// Refuse an acquire whose predecessor heartbeat could not be retired, naming
/// the predecessor's claim id.
///
/// Recovery is `claim release --claim-id <predecessor>`, so withholding the id
/// forces the operator to scrape it out of the run-state comment history.
pub(super) fn unavailable_predecessor_claim<T>(
    issue: u64,
    repo: &str,
    worker_id: &str,
    claim_id: &str,
) -> Result<T, ConductorClaimError> {
    Err(ConductorClaimError::Deferred {
        json: format!(
            "{{\"claimed\":false,\"issue\":{issue},\"repo\":\"{}\",\"worker_id\":\"{}\",\"claim_id\":\"{}\",\"reason\":\"predecessor_heartbeat_retirement_failed\"}}",
            json_escape(repo),
            json_escape(worker_id),
            json_escape(claim_id),
        ),
        exit_code: 2,
    })
}

pub(super) fn unavailable_safety_claim<T>(
    issue: u64,
    repo: &str,
    worker_id: &str,
    safety_reason: &str,
) -> Result<T, ConductorClaimError> {
    Err(ConductorClaimError::Deferred {
        json: format!(
            "{{\"claimed\":false,\"issue\":{issue},\"repo\":\"{}\",\"worker_id\":\"{}\",\"reason\":\"safety_gate_failed\",\"safety_gate\":{{\"ok\":false,\"reason\":\"{}\"}}}}",
            json_escape(repo),
            json_escape(worker_id),
            json_escape(safety_reason),
        ),
        exit_code: 2,
    })
}

pub(super) fn unavailable_claim_with_observed_owner<T>(
    issue: u64,
    repo: &str,
    worker_id: &str,
    observed_owner: &str,
) -> Result<T, ConductorClaimError> {
    Err(ConductorClaimError::Deferred {
        json: format!(
            "{{\"claimed\":false,\"issue\":{issue},\"repo\":\"{}\",\"worker_id\":\"{}\",\"reason\":\"claim_lost\",\"observed_owner\":\"{}\"}}",
            json_escape(repo),
            json_escape(worker_id),
            json_escape(observed_owner),
        ),
        exit_code: 2,
    })
}
