use std::fs;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use autospec_core::autonomous_lifecycle::{
    ClaimBranch, ClaimContext, ClaimEvidence, IssueNumber, LeaseFreshness, RepositoryScope,
    WorkerId, ABANDONED_LEASE_SECS, STALE_LEASE_SECS,
};
use autospec_core::claim::{
    executor_result_evidence_exists, find_reconcilable_pull_request,
    is_executor_result_pull_request, is_reconcilable_pull_request, lowest_marked_comment,
    parse_claim_issue_json, parse_open_pull_requests_json, parse_paths_argument,
    parse_remote_comments_json, parse_run_state_comment, select_run_state,
    terminal_merged_comment_exists, ExecutorResultEvidence, RunStateRecord,
    RUN_TERMINAL_BEGIN_MARKER, RUN_TERMINAL_END_MARKER,
};
use autospec_core::coordination::ConductorOutcome;

use super::lint::claim_safety_with_config;
use super::CommandFailure;

static EXECUTOR_RESULT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(crate) enum ConductorClaimError {
    Diagnostic(CommandFailure),
    Deferred { json: String, exit_code: i32 },
}

impl From<CommandFailure> for ConductorClaimError {
    fn from(error: CommandFailure) -> Self {
        Self::Diagnostic(error)
    }
}

impl ConductorClaimError {
    fn into_command_failure(self) -> CommandFailure {
        match self {
            Self::Diagnostic(error) => error,
            Self::Deferred { json, exit_code } => {
                println!("{json}");
                CommandFailure::status(String::new(), exit_code)
            }
        }
    }
}

pub fn run(args: &[String]) -> Result<(), CommandFailure> {
    match args {
        [] => Err(CommandFailure::diagnostic(
            "autospec claim requires a subcommand",
        )),
        [flag] if matches!(flag.as_str(), "--help" | "-h") => {
            print_help();
            Ok(())
        }
        [command, rest @ ..] if command == "state" => run_state(rest),
        [command, rest @ ..] if command == "acquire" => acquire(rest),
        [command, rest @ ..] if command == "release" => release(rest),
        [command, ..] => Err(CommandFailure::diagnostic(format!(
            "unknown autospec claim command: {command}"
        ))),
    }
}

fn run_state(args: &[String]) -> Result<(), CommandFailure> {
    match args {
        [] => Err(CommandFailure::diagnostic(
            "autospec claim state requires a subcommand",
        )),
        [flag] if matches!(flag.as_str(), "--help" | "-h") => {
            print_state_help();
            Ok(())
        }
        [command, rest @ ..] if command == "read" => read(rest),
        [command, rest @ ..] if command == "upsert" => upsert(rest),
        [command, rest @ ..] if command == "clear" => clear(rest),
        [command, rest @ ..] if command == "reconcile-linked-pr" => reconcile_linked_pr(rest),
        [command, rest @ ..] if command == "recover-stale-startup" => recover_stale_startup(rest),
        [command, ..] => Err(CommandFailure::diagnostic(format!(
            "unknown autospec claim state command: {command}"
        ))),
    }
}

fn release(args: &[String]) -> Result<(), CommandFailure> {
    let options = parse_release_options(args)?;
    let repo = match options.repo {
        Some(repo) => repo,
        None => infer_repo()?,
    };
    let worker_id = options.worker_id.unwrap_or_else(default_worker_id);
    let comments = list_comments(&repo, options.issue)?;
    let now = utc_now_iso()?;
    let claimed_at = select_run_state(&comments, &repo, options.issue)
        .map(|state| state.record.claimed_at)
        .unwrap_or_else(|| now.clone());
    let record = RunStateRecord::new(
        &repo,
        options.issue,
        &worker_id,
        &options.state,
        &options.branch,
        &options.pr,
        &options.state,
        Vec::new(),
        claimed_at,
        &now,
        10_800,
    );
    if options.state == "merged" && !terminal_merged_exists(&comments) {
        let body = format!(
            "{RUN_TERMINAL_BEGIN_MARKER}\n{{\"schema\":1,\"repo\":\"{}\",\"issue\":{},\"worker_id\":\"{}\",\"state\":\"merged\",\"branch\":\"{}\",\"pr\":\"{}\",\"finalized_at\":\"{}\"}}\n{RUN_TERMINAL_END_MARKER}",
            json_escape(&repo),
            options.issue,
            json_escape(&worker_id),
            json_escape(&options.branch),
            json_escape(&options.pr),
            json_escape(&now),
        );
        create_comment(&repo, options.issue, &body)?;
    }
    upsert_record(&repo, &comments, &record)?;
    let mut arguments = vec![
        "issue".to_string(),
        "edit".to_string(),
        options.issue.to_string(),
        "--repo".to_string(),
        repo.clone(),
        "--remove-label".to_string(),
        "in-progress-by-bot".to_string(),
    ];
    if options.state != "merged" {
        arguments.push("--add-label".to_string());
        arguments.push("auto-implement".to_string());
    }
    run_gh_with_retry(&arguments, "transition issue claim labels")?;
    println!(
        "{{\"released\":true,\"issue\":{},\"repo\":\"{}\",\"worker_id\":\"{}\",\"state\":\"{}\"}}",
        options.issue,
        json_escape(&repo),
        json_escape(&worker_id),
        json_escape(&options.state),
    );
    Ok(())
}

fn acquire(args: &[String]) -> Result<(), CommandFailure> {
    let lease = acquire_record(parse_acquire_options(args)?)
        .map_err(ConductorClaimError::into_command_failure)?;
    println!(
        "{{\"claimed\":true,\"issue\":{},\"repo\":\"{}\",\"worker_id\":\"{}\",\"branch\":\"{}\"}}",
        lease.issue,
        json_escape(&lease.repo),
        json_escape(&lease.worker_id),
        json_escape(&lease.branch),
    );
    Ok(())
}

pub(crate) fn acquire_for_conductor(
    repo: &str,
    issue: u64,
    worker_id: &str,
    branch: &str,
) -> Result<ClaimLease, ConductorClaimError> {
    acquire_record(AcquireOptions {
        issue,
        repo: Some(repo.to_string()),
        worker_id: Some(worker_id.to_string()),
        branch: branch.to_string(),
    })
}

/// Read the claim linearization point before the autonomous conductor mutates
/// labels, heartbeats, or comments. The lifecycle policy receives the observed
/// owner rather than a synthetic claim request, so it can reject terminal,
/// foreign, stale, and abandoned state before acquisition has side effects.
pub(crate) fn lifecycle_claim_evidence(
    repo: &str,
    issue: u64,
    requested_worker: &str,
    requested_branch: &str,
) -> Result<ClaimEvidence, CommandFailure> {
    let scope = RepositoryScope::try_from(repo)
        .map_err(|reason| CommandFailure::diagnostic(format!("invalid claim repo: {reason}")))?;
    let issue =
        IssueNumber::new(issue).ok_or_else(|| CommandFailure::diagnostic("invalid claim issue"))?;
    let requested_worker = WorkerId::try_from(requested_worker).map_err(|reason| {
        CommandFailure::diagnostic(format!("invalid requested worker: {reason}"))
    })?;
    let requested_branch = ClaimBranch::try_from(requested_branch).map_err(|reason| {
        CommandFailure::diagnostic(format!("invalid requested branch: {reason}"))
    })?;
    let comments = list_comments(repo, issue.get())?;
    if terminal_merged_exists(&comments) {
        return Ok(ClaimEvidence::Observed(ClaimContext::terminal(
            scope,
            issue,
            requested_worker,
            requested_branch,
        )));
    }
    let Some(selected) = select_run_state(&comments, repo, issue.get()) else {
        return if lowest_marked_comment(&comments).is_some() {
            Ok(ClaimEvidence::Malformed)
        } else {
            Ok(ClaimEvidence::Observed(ClaimContext::active(
                scope,
                issue,
                requested_worker,
                requested_branch,
                LeaseFreshness::Fresh,
            )))
        };
    };
    let worker = WorkerId::try_from(selected.record.worker_id.as_str()).map_err(|reason| {
        CommandFailure::diagnostic(format!("invalid recorded claim worker: {reason}"))
    })?;
    let branch = ClaimBranch::try_from(selected.record.branch.as_str()).map_err(|reason| {
        CommandFailure::diagnostic(format!("invalid recorded claim branch: {reason}"))
    })?;
    if selected.record.state == "merged" {
        return Ok(ClaimEvidence::Observed(ClaimContext::terminal(
            scope, issue, worker, branch,
        )));
    }
    Ok(ClaimEvidence::Observed(ClaimContext::active(
        scope,
        issue,
        worker,
        branch,
        lifecycle_lease_freshness(&selected.server_updated_at),
    )))
}

fn lifecycle_lease_freshness(server_timestamp: &str) -> LeaseFreshness {
    let Some(updated_at) = parse_iso_timestamp(server_timestamp) else {
        return LeaseFreshness::Stale;
    };
    let Ok(now) = unix_now() else {
        return LeaseFreshness::Stale;
    };
    let age = now.saturating_sub(updated_at);
    if age > ABANDONED_LEASE_SECS {
        LeaseFreshness::Abandoned
    } else if age > STALE_LEASE_SECS {
        LeaseFreshness::Stale
    } else {
        LeaseFreshness::Fresh
    }
}

fn acquire_record(options: AcquireOptions) -> Result<ClaimLease, ConductorClaimError> {
    let repo = match options.repo {
        Some(repo) => repo,
        None => infer_repo()?,
    };
    let worker_id = options.worker_id.unwrap_or_else(default_worker_id);
    let issue = load_claim_issue(&repo, options.issue)?;
    if !issue.labels.iter().any(|label| label == "auto-implement") {
        return unavailable_claim(options.issue, &repo, Some(&worker_id), "not_auto_implement");
    }
    let safety = claim_safety_with_config(&issue.safety_input())?;
    if !safety.allowed {
        return unavailable_safety_claim(options.issue, &repo, &worker_id, safety.reason);
    }
    if write_startup_heartbeat(&repo, options.issue, &options.branch).is_err() {
        return unavailable_claim(
            options.issue,
            &repo,
            Some(&worker_id),
            "heartbeat_write_failed",
        );
    }
    let label_create = [
        "label".to_string(),
        "create".to_string(),
        "in-progress-by-bot".to_string(),
        "--repo".to_string(),
        repo.clone(),
        "--color".to_string(),
        "ededed".to_string(),
        "--force".to_string(),
    ];
    let _ = run_gh_with_retry(&label_create, "ensure in-progress-by-bot label");
    let label_move = [
        "issue".to_string(),
        "edit".to_string(),
        options.issue.to_string(),
        "--repo".to_string(),
        repo.clone(),
        "--remove-label".to_string(),
        "auto-implement".to_string(),
        "--add-label".to_string(),
        "in-progress-by-bot".to_string(),
    ];
    if run_gh_with_retry(&label_move, "mark issue in progress").is_err() {
        cleanup_startup_heartbeat(&repo, options.issue);
        return unavailable_claim(
            options.issue,
            &repo,
            Some(&worker_id),
            "label_mutation_failed",
        );
    }

    let ttl_seconds = claim_ttl_seconds();
    let comments = list_comments(&repo, options.issue)?;
    if terminal_merged_exists(&comments) {
        cleanup_startup_heartbeat(&repo, options.issue);
        let remove_active = [
            "issue".to_string(),
            "edit".to_string(),
            options.issue.to_string(),
            "--repo".to_string(),
            repo.clone(),
            "--remove-label".to_string(),
            "in-progress-by-bot".to_string(),
        ];
        let _ = run_gh_with_retry(&remove_active, "remove stale active label");
        return unavailable_claim(options.issue, &repo, Some(&worker_id), "already_merged");
    }
    let selected = select_run_state(&comments, &repo, options.issue);
    let foreign_fresh_owner = selected.as_ref().and_then(|selected| {
        (selected.record.worker_id != worker_id
            && !server_lease_is_stale(&selected.server_updated_at, ttl_seconds))
        .then_some(selected.record.worker_id.as_str())
    });
    if let Some(owner) = foreign_fresh_owner {
        cleanup_own_marked_comments(&repo, options.issue, &worker_id, &comments);
        cleanup_startup_heartbeat(&repo, options.issue);
        return unavailable_claim_with_observed_owner(options.issue, &repo, &worker_id, owner);
    }

    let now = utc_now_iso()?;
    let claimed_at = selected
        .as_ref()
        .map(|state| state.record.claimed_at.clone())
        .unwrap_or_else(|| now.clone());
    let record = RunStateRecord::new(
        &repo,
        options.issue,
        &worker_id,
        "claimed",
        &options.branch,
        "",
        "claimed",
        Vec::new(),
        claimed_at,
        now,
        ttl_seconds,
    );
    if upsert_record(&repo, &comments, &record).is_err() {
        cleanup_startup_heartbeat(&repo, options.issue);
        let restore = [
            "issue".to_string(),
            "edit".to_string(),
            options.issue.to_string(),
            "--repo".to_string(),
            repo.clone(),
            "--remove-label".to_string(),
            "in-progress-by-bot".to_string(),
            "--add-label".to_string(),
            "auto-implement".to_string(),
        ];
        let _ = run_gh_with_retry(&restore, "restore claim label after state failure");
        let reason = if lowest_marked_comment(&comments).is_some() {
            "run_state_upsert_failed"
        } else {
            "run_state_create_failed"
        };
        return unavailable_claim(options.issue, &repo, Some(&worker_id), reason);
    }

    for confirmation in 0..claim_confirm_reads() {
        if confirmation > 0 {
            sleep_claim_settle_interval();
        }
        let observed_comments = list_comments(&repo, options.issue)?;
        if terminal_merged_exists(&observed_comments) {
            cleanup_startup_heartbeat(&repo, options.issue);
            return unavailable_claim(options.issue, &repo, Some(&worker_id), "already_merged");
        }
        let observed = select_run_state(&observed_comments, &repo, options.issue);
        let labels = load_claim_issue(&repo, options.issue)?.labels;
        if observed.as_ref().is_none_or(|state| {
            state.record.worker_id != worker_id || state.record.state != "claimed"
        }) || !labels.iter().any(|label| label == "in-progress-by-bot")
        {
            cleanup_own_marked_comments(&repo, options.issue, &worker_id, &observed_comments);
            cleanup_startup_heartbeat(&repo, options.issue);
            let owner = observed
                .as_ref()
                .map(|state| state.record.worker_id.as_str())
                .unwrap_or_default();
            return unavailable_claim_with_observed_owner(options.issue, &repo, &worker_id, owner);
        }
    }
    Ok(ClaimLease {
        issue: options.issue,
        repo,
        worker_id,
        branch: options.branch,
    })
}

fn read(args: &[String]) -> Result<(), CommandFailure> {
    let options = parse_read_options(args)?;
    let repo = match options.repo {
        Some(repo) => repo,
        None => infer_repo()?,
    };
    let comments = list_comments(&repo, options.issue)?;
    if let Some(selected) = select_run_state(&comments, &repo, options.issue) {
        println!("{}", selected.record.to_json());
    }
    Ok(())
}

fn upsert(args: &[String]) -> Result<(), CommandFailure> {
    let options = parse_upsert_options(args)?;
    let repo = match options.repo {
        Some(repo) => repo,
        None => infer_repo()?,
    };
    let comments = list_comments(&repo, options.issue)?;
    let now = utc_now_iso()?;
    let claimed_at = select_run_state(&comments, &repo, options.issue)
        .map(|state| state.record.claimed_at)
        .unwrap_or_else(|| now.clone());
    let record = RunStateRecord::new(
        &repo,
        options.issue,
        options.worker_id,
        options.state.clone(),
        options.branch,
        options.pr,
        options.step.unwrap_or(options.state),
        options.paths,
        claimed_at,
        now,
        options.ttl_seconds,
    );
    upsert_record(&repo, &comments, &record)?;
    emit_claim_telemetry(
        if lowest_marked_comment(&comments).is_some() {
            "session.step"
        } else {
            "session.started"
        },
        &repo,
        options.issue,
        &record.step,
    );
    println!("{}", record.to_json());
    Ok(())
}

fn upsert_record(
    repo: &str,
    comments: &[autospec_core::claim::RemoteComment],
    record: &RunStateRecord,
) -> Result<(), CommandFailure> {
    let body = record.to_marked_comment();
    if let Some(comment) = lowest_marked_comment(comments) {
        patch_comment(repo, comment.id, &body)?;
        for duplicate in comments.iter().filter(|candidate| {
            candidate.id != comment.id
                && candidate
                    .body
                    .contains(autospec_core::claim::RUN_STATE_BEGIN_MARKER)
                && candidate
                    .body
                    .contains(autospec_core::claim::RUN_STATE_END_MARKER)
        }) {
            let _ = delete_comment(repo, duplicate.id);
        }
    } else {
        create_comment(repo, record.issue, &body)?;
    }
    Ok(())
}

fn clear(args: &[String]) -> Result<(), CommandFailure> {
    let options = parse_read_options(args)?;
    let repo = match options.repo {
        Some(repo) => repo,
        None => infer_repo()?,
    };
    let comments = list_comments(&repo, options.issue)?;
    clear_marked_state(&repo, &comments)?;
    emit_claim_telemetry("session.terminal", &repo, options.issue, "");
    Ok(())
}

fn recover_stale_startup(args: &[String]) -> Result<(), CommandFailure> {
    let options = parse_recover_options(args)?;
    let repo = match options.repo {
        Some(repo) => repo,
        None => infer_repo()?,
    };
    let outcome = recover_stale_startup_record(&repo, options.issue, options.timeout_seconds)?;
    print_recovery_result(outcome.recovered, options.issue, &repo, &outcome.reason);
    Ok(())
}

pub(crate) fn recover_active_issue(
    repo: &str,
    issue: u64,
    timeout_seconds: u64,
) -> Result<bool, CommandFailure> {
    recover_stale_startup_record(repo, issue, timeout_seconds).map(|outcome| outcome.recovered)
}

/// A newly-created claim without a heartbeat, branch, or PR is kept during its
/// startup grace period but must not consume a worker slot yet. Read failures
/// and malformed records remain counted so the queue fails closed.
pub(crate) fn active_issue_counts_toward_worker_capacity(
    repo: &str,
    issue: u64,
    timeout_seconds: u64,
) -> Result<bool, CommandFailure> {
    let comments = list_comments(repo, issue)?;
    let Some(selected) = select_run_state(&comments, repo, issue) else {
        return Ok(true);
    };
    if !selected.record.pr.is_empty()
        || startup_heartbeat_exists(repo, issue)
        || branch_ref_exists(&selected.record.branch)
    {
        return Ok(true);
    }
    let Some(updated_at) = parse_iso_timestamp(&selected.record.updated_at) else {
        return Ok(true);
    };
    Ok(unix_now()
        .map(|now| now.saturating_sub(updated_at) > timeout_seconds)
        .unwrap_or(true))
}

struct RecoveryOutcome {
    recovered: bool,
    reason: String,
}

fn recover_stale_startup_record(
    repo: &str,
    issue: u64,
    timeout_seconds: u64,
) -> Result<RecoveryOutcome, CommandFailure> {
    let comments = list_comments(repo, issue)?;
    let Some(selected) = select_run_state(&comments, repo, issue) else {
        return Ok(RecoveryOutcome {
            recovered: false,
            reason: "missing_run_state".to_string(),
        });
    };
    if !selected.record.pr.is_empty() {
        return Ok(RecoveryOutcome {
            recovered: false,
            reason: "claim_has_pr".to_string(),
        });
    }
    if startup_heartbeat_exists(repo, issue)
        || branch_ref_exists(&selected.record.branch)
        || !server_lease_is_stale(&selected.server_updated_at, timeout_seconds)
        || !server_lease_is_stale(&selected.record.updated_at, timeout_seconds)
    {
        return Ok(RecoveryOutcome {
            recovered: false,
            reason: "claim_evidence_or_fresh_state".to_string(),
        });
    }
    let release_labels = [
        "issue".to_string(),
        "edit".to_string(),
        issue.to_string(),
        "--repo".to_string(),
        repo.to_string(),
        "--remove-label".to_string(),
        "in-progress-by-bot".to_string(),
        "--add-label".to_string(),
        "auto-implement".to_string(),
    ];
    run_gh_with_retry(&release_labels, "release stale startup claim labels")?;
    if let Err(error) = clear_marked_state(repo, &comments) {
        let restore_labels = [
            "issue".to_string(),
            "edit".to_string(),
            issue.to_string(),
            "--repo".to_string(),
            repo.to_string(),
            "--remove-label".to_string(),
            "auto-implement".to_string(),
            "--add-label".to_string(),
            "in-progress-by-bot".to_string(),
        ];
        let _ = run_gh_with_retry(
            &restore_labels,
            "restore active label after stale recovery failure",
        );
        return Err(error);
    }
    emit_claim_telemetry("session.terminal", repo, issue, "stale_startup_released");
    Ok(RecoveryOutcome {
        recovered: true,
        reason: "released_stale_startup_claim".to_string(),
    })
}

fn clear_marked_state(
    repo: &str,
    comments: &[autospec_core::claim::RemoteComment],
) -> Result<(), CommandFailure> {
    for comment in comments.iter().filter(|comment| {
        comment
            .body
            .contains(autospec_core::claim::RUN_STATE_BEGIN_MARKER)
            && comment
                .body
                .contains(autospec_core::claim::RUN_STATE_END_MARKER)
    }) {
        delete_comment(repo, comment.id)?;
    }
    Ok(())
}

fn reconcile_linked_pr(args: &[String]) -> Result<(), CommandFailure> {
    let options = parse_reconcile_options(args)?;
    let repo = match options.repo {
        Some(repo) => repo,
        None => infer_repo()?,
    };
    let outcome = reconcile_linked_pr_record(&repo, options.issue, options.worker_id.as_deref())?;
    print_reconcile_result(
        outcome.reconciled,
        options.issue,
        &repo,
        outcome.pr.as_deref(),
        &outcome.reason,
    );
    Ok(())
}

pub(crate) fn reconcile_active_issue(repo: &str, issue: u64) -> Result<(), CommandFailure> {
    let _ = reconcile_linked_pr_record(repo, issue, None)?;
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExecutorResultRecord {
    Recorded,
    EvidenceUnavailable,
    OwnershipLost,
}

pub(crate) fn record_executor_result(
    repo: &str,
    issue: u64,
    worker_id: &str,
    branch: &str,
    outcome: &ConductorOutcome,
    pull_request: Option<u64>,
) -> Result<ExecutorResultRecord, CommandFailure> {
    let comments = list_comments(repo, issue)?;
    let Some(selected) = select_run_state(&comments, repo, issue) else {
        return Ok(ExecutorResultRecord::OwnershipLost);
    };
    if !has_active_executor_claim(
        &comments,
        &selected.record,
        &selected.server_updated_at,
        worker_id,
        branch,
    ) {
        return Ok(ExecutorResultRecord::OwnershipLost);
    }

    let pull_request = match outcome {
        ConductorOutcome::Succeeded => {
            let Some(pull_request) = pull_request else {
                return Ok(ExecutorResultRecord::EvidenceUnavailable);
            };
            let pull_requests = list_open_pull_requests(repo)?;
            if !pull_requests.iter().any(|candidate| {
                candidate.number == pull_request
                    && is_executor_result_pull_request(candidate, issue, branch)
            }) {
                return Ok(ExecutorResultRecord::EvidenceUnavailable);
            }
            Some(pull_request)
        }
        ConductorOutcome::Blocked(_) | ConductorOutcome::Retryable(_) => {
            if pull_request.is_some() {
                return Ok(ExecutorResultRecord::EvidenceUnavailable);
            }
            None
        }
    };

    let receipt_id = executor_result_receipt_id()?;
    let evidence = ExecutorResultEvidence::new(
        repo,
        issue,
        worker_id,
        branch,
        executor_result_outcome_name(outcome),
        pull_request,
        executor_result_step(outcome),
        receipt_id,
    );
    create_comment(repo, issue, &evidence.to_marked_comment())?;

    let confirmed_comments = list_comments(repo, issue)?;
    if !executor_result_evidence_exists(&confirmed_comments, &evidence) {
        return Err(CommandFailure::diagnostic(
            "executor result evidence was not persisted",
        ));
    }
    let Some(confirmed) = select_run_state(&confirmed_comments, repo, issue) else {
        return Ok(ExecutorResultRecord::OwnershipLost);
    };
    if !has_active_executor_claim(
        &confirmed_comments,
        &confirmed.record,
        &confirmed.server_updated_at,
        worker_id,
        branch,
    ) {
        return Ok(ExecutorResultRecord::OwnershipLost);
    }
    Ok(ExecutorResultRecord::Recorded)
}

pub(crate) fn record_executor_outcome(
    repo: &str,
    issue: u64,
    worker_id: &str,
    branch: &str,
    outcome: &str,
) -> Result<(), CommandFailure> {
    if outcome.trim().is_empty() {
        return Err(CommandFailure::diagnostic(
            "executor outcome must not be empty",
        ));
    }
    match record_executor_result_with_step(
        repo,
        issue,
        worker_id,
        branch,
        &ConductorOutcome::Blocked(outcome.to_string()),
        None,
        outcome,
    )? {
        ExecutorResultRecord::Recorded => Ok(()),
        ExecutorResultRecord::EvidenceUnavailable => Err(CommandFailure::diagnostic(
            "executor outcome evidence became unavailable",
        )),
        ExecutorResultRecord::OwnershipLost => Err(CommandFailure::diagnostic(
            "claim ownership changed while recording executor outcome",
        )),
    }
}

fn record_executor_result_with_step(
    repo: &str,
    issue: u64,
    worker_id: &str,
    branch: &str,
    outcome: &ConductorOutcome,
    pull_request: Option<u64>,
    step: &str,
) -> Result<ExecutorResultRecord, CommandFailure> {
    let comments = list_comments(repo, issue)?;
    let Some(selected) = select_run_state(&comments, repo, issue) else {
        return Ok(ExecutorResultRecord::OwnershipLost);
    };
    if !has_executor_claim_owner(&selected.record, worker_id, branch) {
        return Ok(ExecutorResultRecord::OwnershipLost);
    }

    let verified_pr = match outcome {
        ConductorOutcome::Succeeded => {
            let Some(pull_request) = pull_request else {
                return Ok(ExecutorResultRecord::EvidenceUnavailable);
            };
            let pull_requests = list_open_pull_requests(repo)?;
            if !pull_requests.iter().any(|candidate| {
                candidate.number == pull_request && is_reconcilable_pull_request(candidate, issue)
            }) {
                return Ok(ExecutorResultRecord::EvidenceUnavailable);
            }
            pull_request.to_string()
        }
        ConductorOutcome::Blocked(_) | ConductorOutcome::Retryable(_) => {
            if pull_request.is_some() {
                return Ok(ExecutorResultRecord::EvidenceUnavailable);
            }
            selected.record.pr.clone()
        }
    };

    let mut record = selected.record;
    record.state = "claimed".to_string();
    record.step = step.to_string();
    record.pr = verified_pr.clone();
    record.updated_at = utc_now_iso()?;
    upsert_record(repo, &comments, &record)?;
    let confirmed_comments = list_comments(repo, issue)?;
    let Some(confirmed) = select_run_state(&confirmed_comments, repo, issue) else {
        return Ok(ExecutorResultRecord::OwnershipLost);
    };
    if !has_executor_claim_owner(&confirmed.record, worker_id, branch)
        || confirmed.record.step != step
        || confirmed.record.pr != verified_pr
    {
        return Ok(ExecutorResultRecord::OwnershipLost);
    }
    Ok(ExecutorResultRecord::Recorded)
}

fn has_executor_claim_owner(record: &RunStateRecord, worker_id: &str, branch: &str) -> bool {
    record.worker_id == worker_id && record.branch == branch && record.state == "claimed"
}

fn has_active_executor_claim(
    comments: &[autospec_core::claim::RemoteComment],
    record: &RunStateRecord,
    server_updated_at: &str,
    worker_id: &str,
    branch: &str,
) -> bool {
    !terminal_merged_exists(comments)
        && has_executor_claim_owner(record, worker_id, branch)
        && server_lease_is_fresh(server_updated_at, claim_ttl_seconds())
}

fn server_lease_is_fresh(server_timestamp: &str, ttl_seconds: u64) -> bool {
    let Some(updated_at) = parse_iso_timestamp(server_timestamp) else {
        return false;
    };
    unix_now()
        .map(|now| now.saturating_sub(updated_at) <= ttl_seconds)
        .unwrap_or(false)
}

fn executor_result_step(outcome: &ConductorOutcome) -> &'static str {
    match outcome {
        ConductorOutcome::Succeeded => "executor_succeeded",
        ConductorOutcome::Blocked(_) => "executor_blocked",
        ConductorOutcome::Retryable(_) => "executor_retryable",
    }
}

fn executor_result_outcome_name(outcome: &ConductorOutcome) -> &'static str {
    match outcome {
        ConductorOutcome::Succeeded => "succeeded",
        ConductorOutcome::Blocked(_) => "blocked",
        ConductorOutcome::Retryable(_) => "retryable",
    }
}

fn executor_result_receipt_id() -> Result<String, CommandFailure> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| {
            CommandFailure::diagnostic(format!(
                "cannot generate executor result receipt id: {error}"
            ))
        })?
        .as_nanos();
    let sequence = EXECUTOR_RESULT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    Ok(format!(
        "executor-result-{}-{nanos}-{sequence}",
        std::process::id()
    ))
}

struct ReconcileOutcome {
    reconciled: bool,
    pr: Option<String>,
    reason: String,
}

fn reconcile_linked_pr_record(
    repo: &str,
    issue: u64,
    worker_id: Option<&str>,
) -> Result<ReconcileOutcome, CommandFailure> {
    let comments = list_comments(repo, issue)?;
    let Some(selected) = select_run_state(&comments, repo, issue) else {
        return Ok(ReconcileOutcome {
            reconciled: false,
            pr: None,
            reason: "missing_run_state".to_string(),
        });
    };
    if !selected.record.pr.is_empty() {
        return Ok(ReconcileOutcome {
            reconciled: false,
            pr: Some(selected.record.pr),
            reason: "pr_already_recorded".to_string(),
        });
    }
    let pull_requests = list_open_pull_requests(repo)?;
    let Some(pull_request) = find_reconcilable_pull_request(&pull_requests, issue) else {
        return Ok(ReconcileOutcome {
            reconciled: false,
            pr: None,
            reason: "no_linked_pr_with_one_closeout".to_string(),
        });
    };
    let mut record = selected.record;
    record.worker_id = worker_id.map_or(record.worker_id, ToOwned::to_owned);
    record.state = "claimed".to_string();
    record.step = "post_pr_handoff_failed".to_string();
    record.pr = pull_request.number.to_string();
    record.updated_at = utc_now_iso()?;
    upsert_record(repo, &comments, &record)?;

    let marker = format!(
        "<!-- autospec-linked-pr-run-state-reconcile:pr:{} -->",
        pull_request.number
    );
    if !comments
        .iter()
        .any(|comment| comment.body.contains(&marker))
    {
        let body = format!(
            "{marker}\nAutospec run-state reconciliation found linked PR #{} with one Closeout report while issue #{} was still in `claimed` state with no recorded PR. Resume post-PR handoff from PR #{}: run review/merge gates or comment the blocking gate failure, then release or merge the claim.",
            pull_request.number, issue, pull_request.number
        );
        create_comment(repo, issue, &body)?;
    }
    Ok(ReconcileOutcome {
        reconciled: true,
        pr: Some(pull_request.number.to_string()),
        reason: String::new(),
    })
}

#[derive(Debug, PartialEq, Eq)]
struct ReadOptions {
    issue: u64,
    repo: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
struct UpsertOptions {
    issue: u64,
    repo: Option<String>,
    worker_id: String,
    state: String,
    step: Option<String>,
    branch: String,
    pr: String,
    paths: Vec<String>,
    ttl_seconds: u64,
}

#[derive(Debug, PartialEq, Eq)]
struct ReconcileOptions {
    issue: u64,
    repo: Option<String>,
    worker_id: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
struct RecoverOptions {
    issue: u64,
    repo: Option<String>,
    timeout_seconds: u64,
}

#[derive(Debug, PartialEq, Eq)]
struct ReleaseOptions {
    issue: u64,
    repo: Option<String>,
    worker_id: Option<String>,
    state: String,
    branch: String,
    pr: String,
}

#[derive(Debug, PartialEq, Eq)]
struct AcquireOptions {
    issue: u64,
    repo: Option<String>,
    worker_id: Option<String>,
    branch: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ClaimLease {
    pub issue: u64,
    pub repo: String,
    pub worker_id: String,
    pub branch: String,
}

fn parse_read_options(args: &[String]) -> Result<ReadOptions, CommandFailure> {
    let mut issue = None;
    let mut repo = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--issue" => {
                let value = argument_value(args, &mut index, "--issue")?;
                let parsed = value
                    .parse::<u64>()
                    .ok()
                    .filter(|value| *value > 0)
                    .ok_or_else(|| {
                        CommandFailure::diagnostic("--issue must be a positive integer")
                    })?;
                if issue.replace(parsed).is_some() {
                    return Err(CommandFailure::diagnostic(
                        "--issue accepts exactly one issue number",
                    ));
                }
            }
            "--repo" => {
                let value = argument_value(args, &mut index, "--repo")?;
                if repo.replace(value).is_some() {
                    return Err(CommandFailure::diagnostic(
                        "--repo accepts exactly one repository",
                    ));
                }
            }
            option => {
                return Err(CommandFailure::diagnostic(format!(
                    "unknown autospec claim state read option: {option}"
                )))
            }
        }
        index += 1;
    }
    let issue = issue.ok_or_else(|| CommandFailure::diagnostic("--issue is required"))?;
    Ok(ReadOptions { issue, repo })
}

fn parse_recover_options(args: &[String]) -> Result<RecoverOptions, CommandFailure> {
    let mut issue = None;
    let mut repo = None;
    let mut timeout_seconds = 300;
    let mut timeout_seen = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--issue" => {
                let value = argument_value(args, &mut index, "--issue")?;
                let parsed = value
                    .parse::<u64>()
                    .ok()
                    .filter(|value| *value > 0)
                    .ok_or_else(|| {
                        CommandFailure::diagnostic("--issue must be a positive integer")
                    })?;
                if issue.replace(parsed).is_some() {
                    return Err(CommandFailure::diagnostic(
                        "--issue accepts exactly one issue number",
                    ));
                }
            }
            "--repo" => {
                let value = argument_value(args, &mut index, "--repo")?;
                if repo.replace(value).is_some() {
                    return Err(CommandFailure::diagnostic(
                        "--repo accepts exactly one repository",
                    ));
                }
            }
            "--timeout-seconds" => {
                let value = argument_value(args, &mut index, "--timeout-seconds")?;
                let parsed = value
                    .parse::<u64>()
                    .ok()
                    .filter(|value| *value > 0)
                    .ok_or_else(|| {
                        CommandFailure::diagnostic("--timeout-seconds must be a positive integer")
                    })?;
                if timeout_seen {
                    return Err(CommandFailure::diagnostic(
                        "--timeout-seconds accepts exactly one value",
                    ));
                }
                timeout_seen = true;
                timeout_seconds = parsed;
            }
            option => {
                return Err(CommandFailure::diagnostic(format!(
                    "unknown autospec claim state recover-stale-startup option: {option}"
                )));
            }
        }
        index += 1;
    }
    Ok(RecoverOptions {
        issue: issue.ok_or_else(|| CommandFailure::diagnostic("--issue is required"))?,
        repo,
        timeout_seconds,
    })
}

fn parse_upsert_options(args: &[String]) -> Result<UpsertOptions, CommandFailure> {
    let mut issue = None;
    let mut repo = None;
    let mut worker_id = None;
    let mut state = None;
    let mut step = None;
    let mut branch = String::new();
    let mut branch_seen = false;
    let mut pr = String::new();
    let mut pr_seen = false;
    let mut paths = Vec::new();
    let mut paths_seen = false;
    let mut ttl_seconds = 10_800;
    let mut index = 0;
    while index < args.len() {
        let option = args[index].as_str();
        let value = match option {
            "--issue" | "--repo" | "--worker-id" | "--state" | "--step" | "--branch" | "--pr"
            | "--paths" | "--ttl-seconds" => argument_value(args, &mut index, option)?,
            _ => {
                return Err(CommandFailure::diagnostic(format!(
                    "unknown autospec claim state upsert option: {option}"
                )))
            }
        };
        match option {
            "--issue" => {
                let parsed = value
                    .parse::<u64>()
                    .ok()
                    .filter(|value| *value > 0)
                    .ok_or_else(|| {
                        CommandFailure::diagnostic("--issue must be a positive integer")
                    })?;
                set_once(
                    &mut issue,
                    parsed,
                    "--issue accepts exactly one issue number",
                )?;
            }
            "--repo" => set_once(&mut repo, value, "--repo accepts exactly one repository")?,
            "--worker-id" => set_once(
                &mut worker_id,
                value,
                "--worker-id accepts exactly one worker identifier",
            )?,
            "--state" => set_once(&mut state, value, "--state accepts exactly one state")?,
            "--step" => set_once(&mut step, value, "--step accepts exactly one step")?,
            "--branch" => {
                if branch_seen {
                    return Err(CommandFailure::diagnostic(
                        "--branch accepts exactly one branch",
                    ));
                }
                branch_seen = true;
                branch = value;
            }
            "--pr" => {
                if pr_seen {
                    return Err(CommandFailure::diagnostic(
                        "--pr accepts exactly one pull request",
                    ));
                }
                pr_seen = true;
                pr = value;
            }
            "--paths" => {
                if paths_seen {
                    return Err(CommandFailure::diagnostic(
                        "--paths accepts exactly one path list",
                    ));
                }
                paths_seen = true;
                paths = parse_paths_argument(&value).map_err(|error| {
                    CommandFailure::diagnostic(format!("could not parse --paths: {error}"))
                })?;
            }
            "--ttl-seconds" => {
                ttl_seconds = value.parse::<u64>().map_err(|_| {
                    CommandFailure::diagnostic("--ttl-seconds must be a non-negative integer")
                })?;
            }
            _ => unreachable!("options were matched before parsing"),
        }
        index += 1;
    }
    Ok(UpsertOptions {
        issue: issue.ok_or_else(|| CommandFailure::diagnostic("--issue is required"))?,
        repo,
        worker_id: worker_id
            .ok_or_else(|| CommandFailure::diagnostic("--worker-id is required"))?,
        state: state.ok_or_else(|| CommandFailure::diagnostic("--state is required"))?,
        step,
        branch,
        pr,
        paths,
        ttl_seconds,
    })
}

fn parse_reconcile_options(args: &[String]) -> Result<ReconcileOptions, CommandFailure> {
    let mut issue = None;
    let mut repo = None;
    let mut worker_id = None;
    let mut index = 0;
    while index < args.len() {
        let option = args[index].as_str();
        let value = match option {
            "--issue" | "--repo" | "--worker-id" => argument_value(args, &mut index, option)?,
            _ => {
                return Err(CommandFailure::diagnostic(format!(
                    "unknown autospec claim state reconcile-linked-pr option: {option}"
                )))
            }
        };
        match option {
            "--issue" => {
                let parsed = value
                    .parse::<u64>()
                    .ok()
                    .filter(|value| *value > 0)
                    .ok_or_else(|| {
                        CommandFailure::diagnostic("--issue must be a positive integer")
                    })?;
                set_once(
                    &mut issue,
                    parsed,
                    "--issue accepts exactly one issue number",
                )?;
            }
            "--repo" => set_once(&mut repo, value, "--repo accepts exactly one repository")?,
            "--worker-id" => set_once(
                &mut worker_id,
                value,
                "--worker-id accepts exactly one worker identifier",
            )?,
            _ => unreachable!("options were matched before parsing"),
        }
        index += 1;
    }
    Ok(ReconcileOptions {
        issue: issue.ok_or_else(|| CommandFailure::diagnostic("--issue is required"))?,
        repo,
        worker_id,
    })
}

fn parse_release_options(args: &[String]) -> Result<ReleaseOptions, CommandFailure> {
    let mut issue = None;
    let mut repo = None;
    let mut worker_id = None;
    let mut state = "released".to_string();
    let mut state_seen = false;
    let mut branch = String::new();
    let mut branch_seen = false;
    let mut pr = String::new();
    let mut pr_seen = false;
    let mut index = 0;
    while index < args.len() {
        let option = args[index].as_str();
        let value = match option {
            "--issue" | "--repo" | "--worker-id" | "--state" | "--branch" | "--pr" => {
                argument_value(args, &mut index, option)?
            }
            _ => {
                return Err(CommandFailure::diagnostic(format!(
                    "unknown autospec claim release option: {option}"
                )))
            }
        };
        match option {
            "--issue" => {
                let parsed = value
                    .parse::<u64>()
                    .ok()
                    .filter(|value| *value > 0)
                    .ok_or_else(|| {
                        CommandFailure::diagnostic("--issue must be a positive integer")
                    })?;
                set_once(
                    &mut issue,
                    parsed,
                    "--issue accepts exactly one issue number",
                )?;
            }
            "--repo" => set_once(&mut repo, value, "--repo accepts exactly one repository")?,
            "--worker-id" => set_once(
                &mut worker_id,
                value,
                "--worker-id accepts exactly one worker identifier",
            )?,
            "--state" => {
                if state_seen {
                    return Err(CommandFailure::diagnostic(
                        "--state accepts exactly one state",
                    ));
                }
                if !matches!(value.as_str(), "released" | "failed" | "merged") {
                    return Err(CommandFailure::diagnostic(
                        "--state must be released, failed, or merged",
                    ));
                }
                state_seen = true;
                state = value;
            }
            "--branch" => {
                if branch_seen {
                    return Err(CommandFailure::diagnostic(
                        "--branch accepts exactly one branch",
                    ));
                }
                branch_seen = true;
                branch = value;
            }
            "--pr" => {
                if pr_seen {
                    return Err(CommandFailure::diagnostic(
                        "--pr accepts exactly one pull request",
                    ));
                }
                pr_seen = true;
                pr = value;
            }
            _ => unreachable!("options were matched before parsing"),
        }
        index += 1;
    }
    Ok(ReleaseOptions {
        issue: issue.ok_or_else(|| CommandFailure::diagnostic("--issue is required"))?,
        repo,
        worker_id,
        state,
        branch,
        pr,
    })
}

fn parse_acquire_options(args: &[String]) -> Result<AcquireOptions, CommandFailure> {
    let mut issue = None;
    let mut repo = None;
    let mut worker_id = None;
    let mut branch = String::new();
    let mut branch_seen = false;
    let mut index = 0;
    while index < args.len() {
        let option = args[index].as_str();
        let value = match option {
            "--issue" | "--repo" | "--worker-id" | "--branch" => {
                argument_value(args, &mut index, option)?
            }
            _ => {
                return Err(CommandFailure::diagnostic(format!(
                    "unknown autospec claim acquire option: {option}"
                )))
            }
        };
        match option {
            "--issue" => {
                let parsed = value
                    .parse::<u64>()
                    .ok()
                    .filter(|value| *value > 0)
                    .ok_or_else(|| {
                        CommandFailure::diagnostic("--issue must be a positive integer")
                    })?;
                set_once(
                    &mut issue,
                    parsed,
                    "--issue accepts exactly one issue number",
                )?;
            }
            "--repo" => set_once(&mut repo, value, "--repo accepts exactly one repository")?,
            "--worker-id" => set_once(
                &mut worker_id,
                value,
                "--worker-id accepts exactly one worker identifier",
            )?,
            "--branch" => {
                if branch_seen {
                    return Err(CommandFailure::diagnostic(
                        "--branch accepts exactly one branch",
                    ));
                }
                branch_seen = true;
                branch = value;
            }
            _ => unreachable!("options were matched before parsing"),
        }
        index += 1;
    }
    Ok(AcquireOptions {
        issue: issue.ok_or_else(|| CommandFailure::diagnostic("--issue is required"))?,
        repo,
        worker_id,
        branch,
    })
}

fn argument_value(
    args: &[String],
    index: &mut usize,
    option: &str,
) -> Result<String, CommandFailure> {
    *index += 1;
    let Some(value) = args.get(*index) else {
        return Err(CommandFailure::diagnostic(format!(
            "{option} requires an argument"
        )));
    };
    if value.is_empty() || value.starts_with('-') {
        return Err(CommandFailure::diagnostic(format!(
            "{option} requires an argument"
        )));
    }
    Ok(value.clone())
}

fn set_once<T>(slot: &mut Option<T>, value: T, message: &str) -> Result<(), CommandFailure> {
    if slot.replace(value).is_some() {
        return Err(CommandFailure::diagnostic(message));
    }
    Ok(())
}

fn infer_repo() -> Result<String, CommandFailure> {
    let output = Command::new("gh")
        .args([
            "repo",
            "view",
            "--json",
            "nameWithOwner",
            "--jq",
            ".nameWithOwner",
        ])
        .output()
        .map_err(|error| {
            CommandFailure::diagnostic(format!("could not run gh repo view: {error}"))
        })?;
    if !output.status.success() {
        return Err(CommandFailure::diagnostic(
            "--repo is required when gh cannot infer it",
        ));
    }
    let repo = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if repo.is_empty() {
        Err(CommandFailure::diagnostic(
            "--repo is required when gh cannot infer it",
        ))
    } else {
        Ok(repo)
    }
}

fn list_comments(
    repo: &str,
    issue: u64,
) -> Result<Vec<autospec_core::claim::RemoteComment>, CommandFailure> {
    let endpoint = format!("repos/{repo}/issues/{issue}/comments");
    let output = Command::new("gh")
        .args([
            "api",
            endpoint.as_str(),
            "--jq",
            "[.[] | {id,body,updated_at}]",
        ])
        .output()
        .map_err(|error| {
            CommandFailure::diagnostic(format!("could not run gh api issue comments: {error}"))
        })?;
    if !output.status.success() {
        return Err(CommandFailure::diagnostic(format!(
            "gh api issue comments failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    parse_remote_comments_json(&String::from_utf8_lossy(&output.stdout)).map_err(|error| {
        CommandFailure::diagnostic(format!("could not parse GitHub issue comments: {error}"))
    })
}

fn load_claim_issue(
    repo: &str,
    issue: u64,
) -> Result<autospec_core::claim::ClaimIssueSnapshot, CommandFailure> {
    let output = Command::new("gh")
        .args([
            "issue",
            "view",
            &issue.to_string(),
            "--repo",
            repo,
            "--json",
            "labels,body,title,author",
            "--jq",
            "{labels:[.labels[].name],body:(.body // \"\"),title:(.title // \"\"),author:(.author.login // \"\")}",
        ])
        .output()
        .map_err(|error| CommandFailure::diagnostic(format!("could not run gh issue view: {error}")))?;
    if !output.status.success() {
        return Err(CommandFailure::diagnostic(format!(
            "gh issue view failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    parse_claim_issue_json(&String::from_utf8_lossy(&output.stdout)).map_err(|error| {
        CommandFailure::diagnostic(format!("could not parse GitHub claim issue: {error}"))
    })
}

fn list_open_pull_requests(
    repo: &str,
) -> Result<Vec<autospec_core::claim::OpenPullRequest>, CommandFailure> {
    let output = Command::new("gh")
        .args([
            "pr",
            "list",
            "--repo",
            repo,
            "--state",
            "open",
            "--limit",
            "100",
            "--json",
            "number,body,headRefName",
        ])
        .output()
        .map_err(|error| {
            CommandFailure::diagnostic(format!("could not run gh pr list: {error}"))
        })?;
    if !output.status.success() {
        return Err(CommandFailure::diagnostic(format!(
            "gh pr list failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    parse_open_pull_requests_json(&String::from_utf8_lossy(&output.stdout)).map_err(|error| {
        CommandFailure::diagnostic(format!(
            "could not parse GitHub open pull requests: {error}"
        ))
    })
}

fn patch_comment(repo: &str, comment_id: u64, body: &str) -> Result<(), CommandFailure> {
    let endpoint = format!("repos/{repo}/issues/comments/{comment_id}");
    let arguments = vec![
        "api".to_string(),
        endpoint,
        "-X".to_string(),
        "PATCH".to_string(),
        "-f".to_string(),
        format!("body={body}"),
    ];
    run_gh_with_retry(&arguments, "patch run-state comment")
}

fn delete_comment(repo: &str, comment_id: u64) -> Result<(), CommandFailure> {
    let endpoint = format!("repos/{repo}/issues/comments/{comment_id}");
    run_gh_with_retry(
        &[
            "api".to_string(),
            endpoint,
            "-X".to_string(),
            "DELETE".to_string(),
        ],
        "delete duplicate run-state comment",
    )
}

fn create_comment(repo: &str, issue: u64, body: &str) -> Result<(), CommandFailure> {
    run_gh_with_retry(
        &[
            "issue".to_string(),
            "comment".to_string(),
            issue.to_string(),
            "--repo".to_string(),
            repo.to_string(),
            "--body".to_string(),
            body.to_string(),
        ],
        "create run-state comment",
    )
}

fn run_gh_with_retry(arguments: &[String], action: &str) -> Result<(), CommandFailure> {
    let attempts = std::env::var("AUTOSPEC_GH_API_RETRIES")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(3);
    let sleep_ms = std::env::var("AUTOSPEC_CLAIM_RETRY_SLEEP_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(1_000);
    for attempt in 0..attempts {
        let output = Command::new("gh")
            .args(arguments)
            .output()
            .map_err(|error| CommandFailure::diagnostic(format!("could not {action}: {error}")))?;
        if output.status.success() {
            return Ok(());
        }
        if attempt + 1 < attempts {
            std::thread::sleep(std::time::Duration::from_millis(sleep_ms));
            continue;
        }
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(CommandFailure::diagnostic(if detail.is_empty() {
            format!("could not {action}: gh exited with {}", output.status)
        } else {
            format!("could not {action}: {detail}")
        }));
    }
    unreachable!("retry attempts are always positive")
}

fn utc_now_iso() -> Result<String, CommandFailure> {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| {
            CommandFailure::diagnostic(format!("system clock is before the Unix epoch: {error}"))
        })?
        .as_secs();
    let days = (seconds / 86_400) as i64;
    let seconds_of_day = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    Ok(format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        seconds_of_day / 3_600,
        (seconds_of_day % 3_600) / 60,
        seconds_of_day % 60
    ))
}

fn default_worker_id() -> String {
    if let Ok(worker_id) = std::env::var("AUTOSPEC_WORKER_ID") {
        if !worker_id.trim().is_empty() {
            return worker_id;
        }
    }
    let host = std::env::var("HOSTNAME").unwrap_or_else(|_| "unknown-host".to_string());
    let user = std::env::var("USER").unwrap_or_else(|_| "unknown-user".to_string());
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    format!("{host}:{user}:rust:{}:{timestamp}", std::process::id())
}

fn terminal_merged_exists(comments: &[autospec_core::claim::RemoteComment]) -> bool {
    terminal_merged_comment_exists(comments)
}

fn write_startup_heartbeat(repo: &str, issue: u64, branch: &str) -> Result<(), CommandFailure> {
    let root = heartbeat_root()?;
    let directory = root.join(repo.replace('/', "__"));
    fs::create_dir_all(&directory).map_err(|error| {
        CommandFailure::diagnostic(format!(
            "could not create claim heartbeat directory {}: {error}",
            directory.display()
        ))
    })?;
    let timestamp = unix_now()?;
    let body = format!(
        "{{\"issue\":\"{issue}\",\"branch\":\"{}\",\"step\":\"claimed\",\"ts\":{timestamp},\"pr\":\"\",\"repo\":\"{}\"}}\n",
        json_escape(branch),
        json_escape(repo),
    );
    fs::write(directory.join(format!("{issue}.json")), body).map_err(|error| {
        CommandFailure::diagnostic(format!("could not write claim startup heartbeat: {error}"))
    })
}

fn cleanup_startup_heartbeat(repo: &str, issue: u64) {
    let Ok(root) = heartbeat_root() else {
        return;
    };
    let _ = fs::remove_file(
        root.join(repo.replace('/', "__"))
            .join(format!("{issue}.json")),
    );
}

fn heartbeat_root() -> Result<std::path::PathBuf, CommandFailure> {
    if let Ok(value) = std::env::var("AUTOSPEC_HEARTBEAT_DIR") {
        if !value.is_empty() {
            return Ok(value.into());
        }
    }
    if let Ok(value) = std::env::var("AUTOSPEC_WATCHDOG_DIR") {
        if !value.is_empty() {
            return Ok(std::path::PathBuf::from(value).join("process-heartbeats"));
        }
    }
    let home = std::env::var("HOME").map_err(|_| {
        CommandFailure::diagnostic("could not resolve heartbeat directory: HOME is not set")
    })?;
    Ok(std::path::PathBuf::from(home)
        .join(".autospec")
        .join("process-heartbeats"))
}

fn startup_heartbeat_exists(repo: &str, issue: u64) -> bool {
    heartbeat_root().is_ok_and(|root| {
        root.join(repo.replace('/', "__"))
            .join(format!("{issue}.json"))
            .is_file()
    })
}

fn branch_ref_exists(branch: &str) -> bool {
    if branch.trim().is_empty() {
        return false;
    }
    for reference in [
        format!("refs/heads/{branch}"),
        format!("refs/remotes/origin/{branch}"),
    ] {
        match Command::new("git")
            .args(["show-ref", "--verify", "--quiet", &reference])
            .status()
        {
            Ok(status) if status.success() => return true,
            Ok(_) => {}
            Err(_) => return true,
        }
    }
    match Command::new("git")
        .args(["ls-remote", "--heads", "origin", branch])
        .output()
    {
        Ok(output) if output.status.success() => !output.stdout.is_empty(),
        Ok(_) | Err(_) => true,
    }
}

fn cleanup_own_marked_comments(
    repo: &str,
    issue: u64,
    worker_id: &str,
    comments: &[autospec_core::claim::RemoteComment],
) {
    let lowest = lowest_marked_comment(comments).map(|comment| comment.id);
    let own = comments
        .iter()
        .filter_map(|comment| {
            parse_run_state_comment(&comment.body)
                .ok()
                .filter(|record| record.worker_id == worker_id)
                .map(|_| comment.id)
        })
        .max();
    if let Some(comment_id) = own.filter(|comment_id| Some(*comment_id) != lowest) {
        let _ = delete_comment(repo, comment_id);
    }
    cleanup_startup_heartbeat(repo, issue);
}

fn claim_ttl_seconds() -> u64 {
    std::env::var("AUTOSPEC_CLAIM_LEASE_SECONDS")
        .ok()
        .or_else(|| std::env::var("AUTOSPEC_WATCHDOG_RECLAIM_SECS").ok())
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(10_800)
}

fn claim_confirm_reads() -> u64 {
    std::env::var("AUTOSPEC_CLAIM_CONFIRM_READS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(5)
}

fn sleep_claim_settle_interval() {
    let millis = claim_settle_millis(
        std::env::var("AUTOSPEC_CLAIM_SETTLE_MILLIS")
            .ok()
            .as_deref(),
        std::env::var("AUTOSPEC_CLAIM_SETTLE_SECONDS")
            .ok()
            .as_deref(),
    )
    .unwrap_or(200);
    if millis > 0 {
        std::thread::sleep(std::time::Duration::from_millis(millis));
    }
}

fn claim_settle_millis(millis: Option<&str>, seconds: Option<&str>) -> Option<u64> {
    millis
        .and_then(|value| value.parse::<u64>().ok())
        .or_else(|| seconds.and_then(decimal_seconds_to_millis))
}

fn decimal_seconds_to_millis(value: &str) -> Option<u64> {
    let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
    if (whole.is_empty() && fraction.is_empty())
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    let whole_millis = if whole.is_empty() {
        0
    } else {
        whole.parse::<u64>().ok()?.checked_mul(1_000)?
    };
    let fraction_millis = fraction
        .bytes()
        .take(3)
        .fold(0_u64, |millis, byte| millis * 10 + u64::from(byte - b'0'));
    let fraction_millis = fraction_millis
        .checked_mul(10_u64.pow(3_u32.saturating_sub(fraction.len().min(3) as u32)))?;
    whole_millis.checked_add(fraction_millis)
}

/// Keep telemetry strictly observational: state transitions never depend on
/// the optional autospec-db binary, its configuration, or its exit status.
fn emit_claim_telemetry(kind: &str, repo: &str, issue: u64, step: &str) {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let configured = std::env::var("AUTOSPEC_DB_DSN")
        .ok()
        .is_some_and(|value| !value.is_empty())
        || home
            .as_ref()
            .is_some_and(|path| path.join(".autospec/db.env").is_file());
    if !configured {
        return;
    }

    let binary = if program_on_path("autospec-db") {
        std::path::PathBuf::from("autospec-db")
    } else if let Some(path) = home
        .as_ref()
        .map(|path| path.join(".autospec/bin/autospec-db"))
        .filter(|path| path.is_file())
    {
        path
    } else {
        return;
    };
    let _ = Command::new(binary)
        .args([
            "emit",
            kind,
            &format!("repo={repo}"),
            &format!("issue={issue}"),
            &format!("step={step}"),
        ])
        .status();
}

fn program_on_path(program: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths).any(|directory| directory.join(program).is_file())
    })
}

fn server_lease_is_stale(server_timestamp: &str, ttl_seconds: u64) -> bool {
    let Some(updated_at) = parse_iso_timestamp(server_timestamp) else {
        return false;
    };
    unix_now()
        .map(|now| now.saturating_sub(updated_at) > ttl_seconds)
        .unwrap_or(false)
}

fn unix_now() -> Result<u64, CommandFailure> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|error| {
            CommandFailure::diagnostic(format!("system clock is before the Unix epoch: {error}"))
        })
}

fn parse_iso_timestamp(value: &str) -> Option<u64> {
    let bytes = value.as_bytes();
    if bytes.len() != 20
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
        || bytes[19] != b'Z'
    {
        return None;
    }
    let year = parse_decimal(&bytes[0..4])? as i64;
    let month = parse_decimal(&bytes[5..7])?;
    let day = parse_decimal(&bytes[8..10])?;
    let hour = parse_decimal(&bytes[11..13])?;
    let minute = parse_decimal(&bytes[14..16])?;
    let second = parse_decimal(&bytes[17..19])?;
    if !(1..=12).contains(&month)
        || day == 0
        || day > days_in_month(year, month)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return None;
    }
    let days = days_from_civil(year, month, day);
    u64::try_from(days)
        .ok()?
        .checked_mul(86_400)?
        .checked_add(u64::from(hour) * 3_600 + u64::from(minute) * 60 + u64::from(second))
}

fn parse_decimal(bytes: &[u8]) -> Option<u32> {
    bytes.iter().try_fold(0_u32, |value, byte| {
        byte.is_ascii_digit()
            .then(|| value * 10 + u32::from(*byte - b'0'))
    })
}

fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        _ => 0,
    }
}

fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = year - i64::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month_prime = i64::from(month) + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * month_prime + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let days = days + 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    let year = year + i64::from(month <= 2);
    (year, month as u32, day as u32)
}

fn print_reconcile_result(
    reconciled: bool,
    issue: u64,
    repo: &str,
    pr: Option<&str>,
    reason: &str,
) {
    let pr = pr
        .map(|value| format!("\"{}\"", json_escape(value)))
        .unwrap_or_else(|| "null".to_string());
    let reason = if reason.is_empty() {
        String::new()
    } else {
        format!(",\"reason\":\"{}\"", json_escape(reason))
    };
    println!(
        "{{\"reconciled\":{reconciled},\"issue\":{issue},\"repo\":\"{}\",\"pr\":{pr}{reason}}}",
        json_escape(repo)
    );
}

fn print_recovery_result(recovered: bool, issue: u64, repo: &str, reason: &str) {
    println!(
        "{{\"recovered\":{recovered},\"issue\":{issue},\"repo\":\"{}\",\"reason\":\"{}\"}}",
        json_escape(repo),
        json_escape(reason),
    );
}

fn json_escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            character if character.is_control() => {
                escaped.push_str(&format!("\\u{:04x}", character as u32));
            }
            character => escaped.push(character),
        }
    }
    escaped
}

fn unavailable_claim<T>(
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

fn unavailable_safety_claim<T>(
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

fn unavailable_claim_with_observed_owner<T>(
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

fn print_help() {
    println!(
        "autospec claim\n\nUSAGE:\n    autospec claim state read --issue <N> [--repo OWNER/REPO]\n    autospec claim state upsert --issue <N> --worker-id <ID> --state <STATE> [OPTIONS]\n    autospec claim acquire --issue <N> [--repo OWNER/REPO] [--worker-id ID] [--branch NAME]\n    autospec claim release --issue <N> [--repo OWNER/REPO] [--state released|failed|merged]\n\nCOMMANDS:\n    state          Read and update GitHub-backed claim state\n    acquire        Validate issue eligibility before acquiring an issue lease\n    release        Release, fail, or terminally merge an issue lease"
    );
}

fn print_state_help() {
    println!(
        "autospec claim state\n\nUSAGE:\n    autospec claim state read --issue <N> [--repo OWNER/REPO]\n    autospec claim state upsert --issue <N> --worker-id <ID> --state <STATE> [OPTIONS]\n    autospec claim state clear --issue <N> [--repo OWNER/REPO]\n    autospec claim state reconcile-linked-pr --issue <N> [--repo OWNER/REPO] [--worker-id ID]\n    autospec claim state recover-stale-startup --issue <N> [--repo OWNER/REPO] [--timeout-seconds 300]\n\nCOMMANDS:\n    read                   Read the lowest-ID authoritative run-state comment\n    upsert                 Patch or create the lowest-ID authoritative run-state comment\n    clear                  Delete marked run-state comments\n    reconcile-linked-pr    Record a linked PR before post-PR handoff recovery\n    recover-stale-startup  Release only an evidenceless stale startup claim"
    );
}

#[cfg(test)]
mod tests {
    use super::claim_settle_millis;

    #[test]
    fn claim_settle_millis_preserves_decimal_second_configuration() {
        assert_eq!(claim_settle_millis(None, Some("0.2")), Some(200));
        assert_eq!(claim_settle_millis(None, Some("1.2349")), Some(1_234));
        assert_eq!(claim_settle_millis(None, Some("0")), Some(0));
    }

    #[test]
    fn claim_settle_millis_prefers_explicit_milliseconds_and_rejects_invalid_seconds() {
        assert_eq!(claim_settle_millis(Some("17"), Some("0.2")), Some(17));
        assert_eq!(claim_settle_millis(None, Some("-0.2")), None);
        assert_eq!(claim_settle_millis(None, Some("not-a-duration")), None);
    }
}
