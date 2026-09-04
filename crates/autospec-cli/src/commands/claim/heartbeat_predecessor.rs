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
    // The prior-generation quarantine runs before the evidence probe because it is
    // the only thing that clears a heartbeat left by an older, dead generation of
    // this issue, and that probe answers exactly such a file with a hard error naming
    // the generation mismatch. Probing first made the quarantine unreachable in the
    // one case it exists for, and nothing else closed over the state:
    // `recover-stale-startup` declines too, because the authoritative record reads
    // `released` rather than `claimed`, so there is no stale claim to recover. The
    // issue stayed unacquirable for as long as the file survived.
    //
    // Reaching the quarantine is not sufficient on its own, which is why it now also
    // selects its prior-generation-aware classification on `authorized_prior` being
    // present. It had selected on `heartbeat_lifecycle_step` alone, and a `released`
    // record never reads as one, so it classified the foreign file against its own
    // identity, called it Blocking, and refused. The authorization passed here is the
    // same evidence that branch derives for itself.
    //
    // Both steps stay narrow. `expired_prior_generation_heartbeat` answers `None` for
    // a heartbeat of this generation and for one whose owner is still alive, so a live
    // foreign owner still blocks acquisition and only the dead-foreign-generation case
    // takes the new path. The snapshot is re-read inside the quarantine, so its race
    // guard still compares it against this authorization.
    //
    // "Dead foreign generation" needed one more precondition than that. It also answers
    // `None` while the authoritative record is still inside its own TTL, because a
    // distinct worker's expired heartbeat beside a live record is ordinary lease
    // contention for the lease-timeout path to arbitrate, not this issue's own
    // abandoned garbage (#3505).
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
    if !released_predecessor_heartbeat_evidence_exists(identity)? {
        return Ok(());
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

#[cfg(unix)]
pub(super) fn expired_prior_generation_heartbeat(
    repo_name: &str,
    issue: u64,
    record: &RunStateRecord,
) -> Result<Option<Box<StartupHeartbeatSnapshot>>, CommandFailure> {
    use nix::fcntl::{open, OFlag};
    use nix::sys::stat::Mode;

    // A heartbeat whose worker and claim differ from the authoritative record is only
    // reclaimable as a prior generation once that record has itself been abandoned.
    //
    // Nothing here ties a heartbeat to a *lineage*. Real worker ids are
    // `host:user:harness:pid`, so two concurrent sessions on one machine share every
    // component but the pid, and a prefix test would still admit exactly the collision
    // this guard exists to stop. The one field that would settle it, `session_id`, is
    // carried by the heartbeat but dropped from a released record: a `claimed` record
    // keeps it inside `step`, a `released` one does not. Lineage is therefore not
    // expressible for the records this path sees.
    //
    // Abandonment is expressible, and it separates the two cases the identity test
    // below cannot. A record still inside its own TTL is ordinary lease contention,
    // which the lease-timeout path arbitrates; clearing a distinct worker's heartbeat
    // there discards the only marker that worker has WIP on the branch (#3505). Once
    // the record has aged past its TTL nothing recent holds the issue, and a
    // provably-dead heartbeat left beside it is the garbage that wedged the drain for
    // nine hours (#3503). An unparseable timestamp is not stale, so a malformed record
    // fails closed and keeps its evidence.
    if !server_lease_is_stale(&record.updated_at, record.ttl_seconds) {
        return Ok(None);
    }
    let root_path = heartbeat_root()?;
    let root = match open(
        &root_path,
        OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
        Mode::empty(),
    ) {
        Err(nix::errno::Errno::ENOENT) => return Ok(None),
        Err(error) => {
            return Err(CommandFailure::diagnostic(format!(
                "heartbeat root inspection failed: {error}"
            )))
        }
        Ok(root) => fs::File::from(root),
    };
    private_heartbeat_directory_identity(&root, "prior-generation root")?;
    let repo_key = super::super::autonomous::drain::repository_progress_key(repo_name);
    let Some(repo) = open_optional_heartbeat_directory(&root, Path::new(&repo_key))? else {
        return Ok(None);
    };
    let issue_name = format!("{issue}.json");
    let file = match read_regular_file_at_no_follow(&repo, issue_name.as_ref()) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(
                match classify_retained_prior_generation(
                    &repo,
                    repo_name,
                    issue,
                    record,
                    unix_now()?,
                )? {
                    StartupHeartbeatClassification::ExpiredDead(snapshot) => Some(snapshot),
                    StartupHeartbeatClassification::Absent
                    | StartupHeartbeatClassification::Blocking => None,
                },
            )
        }
        Err(error) => {
            return Err(CommandFailure::diagnostic(format!(
                "prior-generation heartbeat inspection failed: {error}"
            )))
        }
        Ok(file) => file,
    };
    let Some(evidence) = parse_startup_heartbeat(&file.document) else {
        return Ok(None);
    };
    let Some(claim_id) = record.claim_id.as_deref() else {
        return Ok(None);
    };
    if evidence.repo != repo_name
        || evidence.issue != issue.to_string()
        || evidence.branch != record.branch
        || !evidence.pr.is_empty()
        || (evidence.worker_id == record.worker_id && evidence.claim_id == claim_id)
    {
        return Ok(None);
    }
    let expected = StartupHeartbeatExpectation {
        repo: &evidence.repo,
        issue,
        worker_id: &evidence.worker_id,
        branch: &evidence.branch,
        pull_request: &evidence.pr,
        claim_id: &evidence.claim_id,
        step: &evidence.step,
    };
    Ok(
        match classify_startup_heartbeat_snapshot(
            file,
            expected,
            unix_now()?,
            observe_local_startup_pid,
        ) {
            StartupHeartbeatClassification::ExpiredDead(snapshot) => Some(snapshot),
            StartupHeartbeatClassification::Absent | StartupHeartbeatClassification::Blocking => {
                None
            }
        },
    )
}

#[cfg(not(unix))]
pub(super) fn expired_prior_generation_heartbeat(
    _repo: &str,
    _issue: u64,
    _record: &RunStateRecord,
) -> Result<Option<Box<StartupHeartbeatSnapshot>>, CommandFailure> {
    Ok(None)
}
