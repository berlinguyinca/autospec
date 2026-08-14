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
    heartbeat_portable::retire_released(ClaimMutationIdentity {
        repo,
        issue,
        worker_id: &record.worker_id,
        branch: &record.branch,
        claim_id,
    })
}

#[cfg(all(test, not(target_os = "linux")))]
mod tests {
    use super::*;

    fn predecessor(state: &str) -> ClaimRefHead {
        ClaimRefHead {
            oid: "oid".to_string(),
            generation: "generation".to_string(),
            record: RunStateRecord::new(
                "owner/repo",
                42,
                "worker-a",
                state,
                "feat/worker-a",
                "",
                state,
                Vec::new(),
                "2026-08-13T00:00:00Z",
                "2026-08-13T00:00:00Z",
                300,
            )
            .with_claim_id("claim-a"),
        }
    }

    #[test]
    fn fresh_acquisition_without_predecessor_needs_no_linux_retirement() {
        retire("owner/repo", 42, None).expect("fresh acquisition has nothing to retire");
    }

    #[test]
    fn released_predecessor_without_local_evidence_needs_no_retirement() {
        let prior = predecessor("released");
        retire("owner/repo", 42, Some(&prior)).expect("missing heartbeat is already retired");
    }
}
