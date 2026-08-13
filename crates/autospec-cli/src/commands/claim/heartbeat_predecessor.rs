use super::*;

#[cfg(target_os = "linux")]
pub(super) fn retire(
    repo: &str,
    issue: u64,
    prior: Option<&ClaimRefHead>,
) -> Result<(), CommandFailure> {
    let Some(record) = prior
        .map(|head| &head.record)
        .filter(|record| record.state == "released")
    else {
        return Ok(());
    };
    let claim_id = record.claim_id.as_deref().ok_or_else(|| {
        CommandFailure::diagnostic("released predecessor heartbeat has no claim identity")
    })?;
    let identity = ClaimMutationIdentity {
        repo,
        issue,
        worker_id: &record.worker_id,
        branch: &record.branch,
        claim_id,
    };
    if !released_predecessor_heartbeat_evidence_exists(identity)? {
        return Ok(());
    }
    if let Some(prior) = expired_prior_generation_heartbeat(repo, issue, record)? {
        if quarantine_authoritative_stale_heartbeat(
            repo,
            issue,
            record,
            Some(prior.as_ref()),
            &mut || Ok(()),
        )? {
            return Ok(());
        }
    }
    retire_released_startup_heartbeat_with_hook(identity, true, &mut |_, _| Ok(()))
}

#[cfg(not(target_os = "linux"))]
pub(super) fn retire(
    _repo: &str,
    _issue: u64,
    _prior: Option<&ClaimRefHead>,
) -> Result<(), CommandFailure> {
    Err(CommandFailure::diagnostic(
        "predecessor heartbeat retirement requires Linux pidfd ownership",
    ))
}
