use super::*;

#[cfg(unix)]
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

#[cfg(not(unix))]
pub(super) fn retire(
    _repo: &str,
    _issue: u64,
    prior: Option<&ClaimRefHead>,
) -> Result<(), CommandFailure> {
    if !prior.is_some_and(|head| head.record.state == "released") {
        return Ok(());
    }
    Err(CommandFailure::diagnostic(
        "predecessor heartbeat retirement requires Unix descriptor operations",
    ))
}

#[cfg(all(test, not(unix)))]
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
    fn released_predecessor_requires_linux_pidfd_retirement() {
        let prior = predecessor("released");

        let error = retire("owner/repo", 42, Some(&prior))
            .expect_err("released predecessor retirement must fail closed");

        assert_eq!(
            error.message,
            "predecessor heartbeat retirement requires Linux pidfd ownership"
        );
    }
}
