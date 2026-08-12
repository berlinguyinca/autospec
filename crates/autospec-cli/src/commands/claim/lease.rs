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

/// Return an issue whose worker died to the candidate pool.
///
/// Claiming swaps `auto-implement` off and `in-progress-by-bot` on, and the ready
/// queue builds its candidates from `auto-implement` alone. A worker that dies
/// without a clean release therefore strands its issue outside the pool forever:
/// the claim record says nobody owns it, the label says it is in flight, and the
/// conductor idles beside work it can never see.
///
/// Requeue only when the claim is genuinely abandoned — no record, a released
/// state, or an owner that no longer holds it. A `merged` record is finished, and
/// an owner still holding its claim is working.
///
/// `owner_holds` comes from [`owner_still_holds`] so this agrees with the claim
/// acquisition path by construction. When the two disagreed, a dead owner's fresh
/// lease made this say "owned" while acquisition said "takeable": the conductor was
/// willing to take the issue but never saw it, because the label kept it out of the
/// candidate pool.
pub(super) fn claim_is_abandoned(record: Option<&RunStateRecord>, owner_holds: bool) -> bool {
    match record {
        None => true,
        Some(record) if record.state == "merged" => false,
        Some(record) => {
            matches!(
                record.state.as_str(),
                "available" | "released" | "retryable" | "failed"
            ) || !owner_holds
        }
    }
}

pub(super) fn acquisition_blocking_owner(record: &RunStateRecord) -> Option<&str> {
    (!matches!(
        record.state.as_str(),
        "available" | "released" | "retryable" | "failed"
    ) && (heartbeat_publication_in_flight(&record.step)
        || conductor_claim_owner_holds_lease(record)))
    .then_some(record.worker_id.as_str())
}

pub(super) fn heartbeat_publication_in_flight(step: &str) -> bool {
    step.starts_with("heartbeat-pending:") || step.starts_with("heartbeat-publishing:")
}

/// Whether the recorded owner still holds its claim.
///
/// Both the TTL and the owner's liveness must agree. The clock alone is not enough:
/// a worker that died seconds ago keeps a valid lease for its full TTL.
pub(super) fn owner_still_holds(
    repo: &str,
    issue: u64,
    record: &RunStateRecord,
) -> Result<bool, super::CommandFailure> {
    if heartbeat_publication_in_flight(&record.step) {
        return Ok(true);
    }
    if !conductor_claim_owner_holds_lease(record) {
        return Ok(false);
    }
    current_owner_heartbeat_holds(repo, issue, record)
}

#[cfg(target_os = "linux")]
fn open_current_owner_repo(
    repo_name: &str,
) -> Result<Option<std::fs::File>, super::CommandFailure> {
    use nix::fcntl::{open, OFlag};
    use nix::sys::stat::Mode;

    let root_path = super::heartbeat_root()?;
    let root = match open(
        &root_path,
        OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
        Mode::empty(),
    ) {
        Err(nix::errno::Errno::ENOENT) => return Ok(None),
        Err(error) => {
            return Err(super::CommandFailure::diagnostic(format!(
                "heartbeat root inspection failed: {error}"
            )))
        }
        Ok(root) => std::fs::File::from(root),
    };
    super::private_heartbeat_directory_identity(&root, "current-owner root")?;
    let repo_key = super::super::autonomous::drain::repository_progress_key(repo_name);
    super::open_optional_heartbeat_directory(&root, std::path::Path::new(&repo_key))
}

#[cfg(target_os = "linux")]
fn current_owner_heartbeat_holds(
    repo_name: &str,
    issue: u64,
    record: &RunStateRecord,
) -> Result<bool, super::CommandFailure> {
    let Some(claim_id) = record.claim_id.as_deref() else {
        return Ok(true);
    };
    let Some(repo) = open_current_owner_repo(repo_name)? else {
        return Ok(false);
    };
    let expected = super::StartupHeartbeatExpectation {
        repo: repo_name,
        issue,
        worker_id: &record.worker_id,
        branch: &record.branch,
        pull_request: "",
        claim_id,
        step: "claimed",
    };
    let issue_name = format!("{issue}.json");
    match super::classify_startup_heartbeat_at(
        &repo,
        issue_name.as_ref(),
        expected,
        super::unix_now()?,
        super::observe_local_startup_pid,
    ) {
        super::StartupHeartbeatClassification::ExpiredDead(_) => Ok(false),
        super::StartupHeartbeatClassification::Blocking => Ok(true),
        super::StartupHeartbeatClassification::Absent => {
            let no_receipt = matches!(
                super::heartbeat_receipt_retry_decision(&repo, expected),
                super::HeartbeatReceiptDecision::Absent
            );
            if !no_receipt {
                return Ok(true);
            }
            let retained = super::classify_retained_prior_generation(
                &repo,
                repo_name,
                issue,
                record,
                super::unix_now()?,
            )?;
            Ok(!matches!(
                retained,
                super::StartupHeartbeatClassification::Absent
            ))
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn current_owner_heartbeat_holds(
    _repo: &str,
    _issue: u64,
    _record: &RunStateRecord,
) -> Result<bool, super::CommandFailure> {
    Ok(true)
}

pub(crate) fn requeue_abandoned_active_issue(
    repo: &str,
    issue: u64,
) -> Result<bool, super::CommandFailure> {
    let selected = super::read_claim_ref(repo, issue)?;
    let Some(head) = quarantine_abandoned_claim_generation_with(
        repo,
        issue,
        selected,
        &mut |expected, successor| super::advance_claim_ref(repo, issue, expected, successor),
    )?
    else {
        return Ok(false);
    };
    super::project_claim_ref_to_comments(repo, &head);
    relabel_abandoned_active_issue(repo, issue)?;
    Ok(true)
}

fn relabel_abandoned_active_issue(repo: &str, issue: u64) -> Result<(), super::CommandFailure> {
    super::run_gh_with_retry(
        &[
            "issue".to_string(),
            "edit".to_string(),
            issue.to_string(),
            "--repo".to_string(),
            repo.to_string(),
            "--remove-label".to_string(),
            "in-progress-by-bot".to_string(),
            "--add-label".to_string(),
            "auto-implement".to_string(),
        ],
        "requeue an abandoned active issue",
    )
}

pub(super) fn quarantine_abandoned_claim_generation_with<Advance>(
    repo: &str,
    issue: u64,
    selected: Option<super::ClaimRefHead>,
    advance: &mut Advance,
) -> Result<Option<Box<super::ClaimRefHead>>, super::CommandFailure>
where
    Advance: FnMut(
        Option<&super::ClaimRefHead>,
        &RunStateRecord,
    ) -> Result<super::ClaimRefAdvance, super::CommandFailure>,
{
    let owner_holds = match selected.as_ref().map(|head| &head.record) {
        Some(record) if record.state == "claimed" => owner_still_holds(repo, issue, record)?,
        None => false,
        Some(_) => false,
    };
    if !claim_is_abandoned(selected.as_ref().map(|head| &head.record), owner_holds) {
        return Ok(None);
    }
    if let Some(record) = selected
        .as_ref()
        .map(|head| &head.record)
        .filter(|record| record.state == "claimed")
    {
        if !super::quarantine_authoritative_stale_heartbeat(repo, issue, record, None, &mut || {
            Ok(())
        })? {
            return Ok(None);
        }
    }
    let selected = match selected {
        Some(selected) => selected,
        None => {
            let now = super::utc_now_iso()?;
            let identity = super::unique_operation_id("abandoned-requeue")?;
            let quarantine = RunStateRecord::new(
                repo,
                issue,
                format!("autospec-{identity}"),
                "claimed",
                "autospec/requeue-abandoned",
                "",
                "abandoned_requeue_quarantine",
                Vec::new(),
                &now,
                &now,
                1,
            )
            .with_claim_id(identity);
            match advance(None, &quarantine)? {
                super::ClaimRefAdvance::Won(head) => *head,
                super::ClaimRefAdvance::Lost => return Ok(None),
            }
        }
    };
    let mut available = selected.record.clone();
    available.state = "available".to_string();
    available.step = "abandoned_requeued".to_string();
    available.updated_at = super::utc_now_iso()?;
    let head = match advance(Some(&selected), &available)? {
        super::ClaimRefAdvance::Won(head) => head,
        super::ClaimRefAdvance::Lost => return Ok(None),
    };
    Ok(Some(head))
}

/// The owner a fresh worker must yield to, if any.
///
/// An owner yields when its lease has aged past its TTL, and also when its startup
/// heartbeat is expired-dead. The TTL alone is not enough: a worker that died
/// seconds ago holds a valid lease for the full three hours, so every successor
/// lost the claim to a dead process and exited, over and over.
pub(super) fn contesting_claim_owner(
    repo: &str,
    issue: u64,
    worker_id: &str,
    branch: &str,
) -> Result<Option<String>, super::CommandFailure> {
    let Some(head) = super::read_claim_ref(repo, issue)? else {
        return Ok(None);
    };
    let contested = head.record.state == "claimed"
        && (head.record.worker_id != worker_id || head.record.branch != branch);
    if !contested || !owner_still_holds(repo, issue, &head.record)? {
        return Ok(None);
    }
    Ok(Some(head.record.worker_id))
}
