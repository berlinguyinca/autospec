use super::*;

#[cfg(unix)]
pub(super) fn completed_handoff(repo: &fs::File, identity: ClaimMutationIdentity<'_>) -> bool {
    heartbeat_receipt_retry_decision(
        repo,
        StartupHeartbeatExpectation {
            repo: identity.repo,
            issue: identity.issue,
            worker_id: identity.worker_id,
            branch: identity.branch,
            pull_request: "",
            claim_id: identity.claim_id,
            step: "claimed",
        },
    ) == HeartbeatReceiptDecision::Completed
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
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

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
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

/// Retire the local heartbeat and its session binding for a claim the caller is
/// terminating itself.
///
/// [`retire`] exists for the *next* acquirer cleaning up after a predecessor, so
/// it insists the owning process is dead. Here the owning process is the caller,
/// so liveness is not a precondition. Without this, `claim release` left the
/// session binding on disk forever and the same session could never claim
/// another issue: the create-once binding still named the finished issue.
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) fn retire_terminal(identity: ClaimMutationIdentity<'_>) -> Result<(), CommandFailure> {
    // Probe before opening for write. The write path creates the repository
    // directory when it is absent, and a freshly created directory inherits the
    // umask, so it then fails its own 0700 private-directory check. Releases of
    // claims that never published a local heartbeat must stay silent.
    if !released_predecessor_heartbeat_evidence_exists(identity)? {
        // No issue heartbeat. Either this claim never published one -- in which
        // case there is nothing to do and staying silent is right -- or the
        // watchdog collected it and left the create-once session binding
        // behind, which wedges the session for good (#3356). Only the second
        // case has anything to retire, and it names itself.
        return retire_orphaned_session_bindings(identity, &mut |_, _| Ok(()));
    }
    retire_released_startup_heartbeat_with_hook(identity, false, &mut |_, _| Ok(()))?;
    retire_orphaned_session_bindings(identity, &mut |_, _| Ok(()))
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(super) fn retire_terminal(identity: ClaimMutationIdentity<'_>) -> Result<(), CommandFailure> {
    heartbeat_portable::retire_released(identity)
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
    fn released_predecessor_without_local_evidence_needs_no_retirement() {
        const CHILD: &str = "AUTOSPEC_TEST_RELEASED_PREDECESSOR_MISSING_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let root = std::env::temp_dir().join(format!(
                "autospec-released-predecessor-missing-{}",
                std::process::id()
            ));
            std::fs::create_dir(&root).expect("private heartbeat root");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))
                    .expect("private heartbeat root permissions");
            }
            let output = std::process::Command::new(std::env::current_exe().expect("test binary"))
                .args([
                    "--exact",
                    "commands::claim::heartbeat_predecessor::tests::released_predecessor_without_local_evidence_needs_no_retirement",
                    "--nocapture",
                ])
                .env(CHILD, "1")
                .env("AUTOSPEC_HEARTBEAT_DIR", &root)
                .output()
                .expect("isolated predecessor retirement test");
            std::fs::remove_dir_all(&root).expect("remove heartbeat root");
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        let prior = predecessor("released");
        retire("owner/repo", 42, Some(&prior)).expect("missing heartbeat is already retired");
    }
}
