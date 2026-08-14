use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

use autospec_core::autonomous_lifecycle::{
    ClaimBranch, ClaimContext, ClaimEvidence, IssueNumber, LeaseFreshness, RepositoryScope,
    WorkerId, ABANDONED_LEASE_SECS, STALE_LEASE_SECS,
};
use autospec_core::claim::{
    evaluate_merge_ready_claim_recovery, executor_result_evidence_exists,
    find_reconcilable_pull_request, is_executor_result_pull_request, is_reconcilable_pull_request,
    parse_claim_issue_json, parse_open_pull_requests_json, parse_paths_argument,
    parse_required_checks_json, parse_run_state_comment,
    successful_executor_result_for_pull_request, ClaimRecoveryDecision, ExecutorResultEvidence,
    RemoteComment, RunStateRecord, SelectedRunState, RUN_STATE_BEGIN_MARKER, RUN_STATE_END_MARKER,
    RUN_TERMINAL_BEGIN_MARKER, RUN_TERMINAL_END_MARKER,
};
use autospec_core::coordination::ConductorOutcome;
use autospec_core::safety::redact_secrets;
use autospec_core::state::json::JsonParser;

use super::lint::claim_safety_with_config;
use super::CommandFailure;

static UNIQUE_ID_SEQUENCE: AtomicU64 = AtomicU64::new(0);
const WAIT_FAILURE_MARKER: &str = "<!-- autospec-implementer-wait-failed -->";
const RUN_STATE_LINK_PREFIX: &str = "<!-- autospec-run-state-link parent=";
const RUN_STATE_LINK_SUFFIX: &str = " -->";
const CLAIM_REF_MESSAGE_HEADER: &str = "autospec-claim-ledger-v1";
const BRIDGE_TERMINAL_PREPARED_PREFIX: &str = "terminal_prepared:";

#[derive(Debug, Clone, PartialEq, Eq)]
struct ClaimRefHead {
    oid: String,
    generation: String,
    record: RunStateRecord,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ClaimRefAdvance {
    Won(Box<ClaimRefHead>),
    Lost,
}

fn claim_ref_name(issue: u64) -> String {
    format!("refs/autospec/claims/issue-{issue}")
}

fn run_git_in(
    git_program: &Path,
    workdir: &Path,
    arguments: &[&str],
    action: &str,
) -> Result<std::process::Output, CommandFailure> {
    Command::new(git_program)
        .args(arguments)
        .current_dir(workdir)
        .output()
        .map_err(|error| CommandFailure::diagnostic(format!("{action}: {error}")))
}

fn claim_remote_arguments(remote_url: &str, command_arguments: &[&str]) -> Vec<String> {
    let mut arguments = Vec::new();
    if remote_url.starts_with("https://") || remote_url.starts_with("http://") {
        arguments.extend([
            "-c".to_string(),
            "credential.helper=!gh auth git-credential".to_string(),
            "-c".to_string(),
            "credential.useHttpPath=true".to_string(),
        ]);
    }
    arguments.extend(
        command_arguments
            .iter()
            .map(|argument| (*argument).to_string()),
    );
    arguments
}

fn resolve_remote_url_in(
    git_program: &Path,
    workdir: &Path,
    remote: &str,
) -> Result<String, CommandFailure> {
    if remote.contains("://")
        || remote.contains(':')
        || Path::new(remote).is_absolute()
        || Path::new(remote).exists()
    {
        return Ok(remote.to_string());
    }
    let output = run_git_in(
        git_program,
        workdir,
        &["remote", "get-url", remote],
        "resolve claim remote URL",
    )?;
    if !output.status.success() {
        return Err(CommandFailure::diagnostic(format!(
            "resolve claim remote URL: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn push_claim_ref_in(
    git_program: &Path,
    workdir: &Path,
    remote: &str,
    refspec: &str,
) -> Result<std::process::Output, CommandFailure> {
    let remote_url = resolve_remote_url_in(git_program, workdir, remote)?;
    let arguments = claim_remote_arguments(&remote_url, &["push", remote, refspec]);
    let arguments = arguments.iter().map(String::as_str).collect::<Vec<_>>();
    run_git_in(git_program, workdir, &arguments, "advance claim ref")
        .map_err(CommandFailure::into_transient)
}

fn parse_claim_ref_message(
    oid: String,
    message: &str,
    repo: &str,
    issue: u64,
) -> Result<ClaimRefHead, CommandFailure> {
    let mut lines = message.lines();
    if lines.next() != Some(CLAIM_REF_MESSAGE_HEADER) {
        return Err(CommandFailure::diagnostic(
            "claim ref commit has an unsupported message header",
        ));
    }
    let generation = lines
        .next()
        .and_then(|line| line.strip_prefix("generation="))
        .filter(|generation| !generation.trim().is_empty())
        .ok_or_else(|| CommandFailure::diagnostic("claim ref commit has no generation"))?
        .to_string();
    let record = parse_run_state_comment(message).map_err(|error| {
        CommandFailure::diagnostic(format!("invalid claim ref record: {error}"))
    })?;
    if record.repo != repo || record.issue != issue {
        return Err(CommandFailure::diagnostic(
            "claim ref record does not match the requested repository and issue",
        ));
    }
    Ok(ClaimRefHead {
        oid,
        generation,
        record,
    })
}

fn read_claim_ref_in(
    git_program: &Path,
    workdir: &Path,
    remote: &str,
    repo: &str,
    issue: u64,
) -> Result<Option<ClaimRefHead>, CommandFailure> {
    let reference = claim_ref_name(issue);
    let remote_url = resolve_remote_url_in(git_program, workdir, remote)?;
    let arguments =
        claim_remote_arguments(&remote_url, &["ls-remote", "--refs", remote, &reference]);
    let arguments = arguments.iter().map(String::as_str).collect::<Vec<_>>();
    let output = run_git_in(git_program, workdir, &arguments, "read claim ref")
        .map_err(CommandFailure::into_transient)?;
    if !output.status.success() {
        return Err(CommandFailure::transient(format!(
            "read claim ref: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let Some(oid) = stdout.split_whitespace().next() else {
        return Ok(None);
    };
    if !matches!(oid.len(), 40 | 64) || !oid.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(CommandFailure::diagnostic(
            "read claim ref returned an invalid object ID",
        ));
    }
    let arguments =
        claim_remote_arguments(&remote_url, &["fetch", "--no-tags", remote, &reference]);
    let arguments = arguments.iter().map(String::as_str).collect::<Vec<_>>();
    let fetch = run_git_in(git_program, workdir, &arguments, "fetch claim ref")
        .map_err(CommandFailure::into_transient)?;
    if !fetch.status.success() {
        return Err(CommandFailure::transient(format!(
            "fetch claim ref: {}",
            String::from_utf8_lossy(&fetch.stderr).trim()
        )));
    }
    let message = run_git_in(
        git_program,
        workdir,
        &["show", "-s", "--format=%B", oid],
        "read claim ref commit",
    )?;
    if !message.status.success() {
        return Err(CommandFailure::diagnostic(format!(
            "read claim ref commit: {}",
            String::from_utf8_lossy(&message.stderr).trim()
        )));
    }
    parse_claim_ref_message(
        oid.to_string(),
        &String::from_utf8_lossy(&message.stdout),
        repo,
        issue,
    )
    .map(Some)
}

fn claim_transition_identity_is_valid(
    parent: Option<&ClaimRefHead>,
    record: &RunStateRecord,
) -> bool {
    let Some(parent) = parent else {
        return record.state == "claimed";
    };
    if !matches!(
        record.state.as_str(),
        "merged" | "released" | "failed" | "available"
    ) {
        return true;
    }
    parent.record.worker_id == record.worker_id
        && parent.record.branch == record.branch
        && parent.record.claim_id == record.claim_id
}

fn create_claim_ref_commit(
    git_program: &Path,
    workdir: &Path,
    parent: Option<&ClaimRefHead>,
    generation: &str,
    record: &RunStateRecord,
) -> Result<String, CommandFailure> {
    let tree = run_git_in(git_program, workdir, &["mktree"], "create claim ref tree")?;
    if !tree.status.success() {
        return Err(CommandFailure::diagnostic(format!(
            "create claim ref tree: {}",
            String::from_utf8_lossy(&tree.stderr).trim()
        )));
    }
    let tree = String::from_utf8_lossy(&tree.stdout).trim().to_string();
    let mut command = Command::new(git_program);
    command
        .arg("commit-tree")
        .arg(&tree)
        .current_dir(workdir)
        .env("GIT_AUTHOR_NAME", "Autospec Claim Ledger")
        .env("GIT_AUTHOR_EMAIL", "autospec-claim@localhost")
        .env("GIT_COMMITTER_NAME", "Autospec Claim Ledger")
        .env("GIT_COMMITTER_EMAIL", "autospec-claim@localhost")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(parent) = parent {
        command.args(["-p", &parent.oid]);
    }
    let mut child = command
        .spawn()
        .map_err(|error| CommandFailure::diagnostic(format!("create claim ref commit: {error}")))?;
    let message = format!(
        "{CLAIM_REF_MESSAGE_HEADER}\ngeneration={generation}\n\n{}\n",
        record.to_marked_comment()
    );
    child
        .stdin
        .take()
        .ok_or_else(|| CommandFailure::diagnostic("claim ref commit stdin was unavailable"))?
        .write_all(message.as_bytes())
        .map_err(|error| {
            CommandFailure::diagnostic(format!("write claim ref commit message: {error}"))
        })?;
    let output = child
        .wait_with_output()
        .map_err(|error| CommandFailure::diagnostic(format!("finish claim ref commit: {error}")))?;
    if !output.status.success() {
        return Err(CommandFailure::diagnostic(format!(
            "create claim ref commit: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn advance_claim_ref_in(
    git_program: &Path,
    workdir: &Path,
    remote: &str,
    repo: &str,
    issue: u64,
    expected: Option<&ClaimRefHead>,
    record: &RunStateRecord,
) -> Result<ClaimRefAdvance, CommandFailure> {
    if record.repo != repo || record.issue != issue {
        return Err(CommandFailure::diagnostic(
            "claim transition record does not match its repository and issue",
        ));
    }
    if !claim_transition_identity_is_valid(expected, record) {
        return Err(CommandFailure::diagnostic(
            "terminal claim transition does not match worker, branch, and claim ID",
        ));
    }
    let observed = read_claim_ref_in(git_program, workdir, remote, repo, issue)?;
    if observed.as_ref().map(|head| &head.oid) != expected.map(|head| &head.oid) {
        return Ok(ClaimRefAdvance::Lost);
    }
    let generation = unique_operation_id("claim-ref")?;
    let oid = create_claim_ref_commit(git_program, workdir, expected, &generation, record)?;
    let reference = claim_ref_name(issue);
    let refspec = format!("{oid}:{reference}");
    let push = push_claim_ref_in(git_program, workdir, remote, &refspec)?;
    if push.status.success() {
        return Ok(ClaimRefAdvance::Won(Box::new(ClaimRefHead {
            oid,
            generation,
            record: record.clone(),
        })));
    }
    // A transport failure is ambiguous. Re-read once: if receive-pack committed
    // our exact object, this operation won. Never retry the mutation.
    let reread = read_claim_ref_in(git_program, workdir, remote, repo, issue)?;
    if reread.as_ref().is_some_and(|head| head.oid == oid) {
        Ok(ClaimRefAdvance::Won(Box::new(
            reread.expect("checked claim ref"),
        )))
    } else if reread.as_ref().map(|head| &head.oid) != expected.map(|head| &head.oid) {
        Ok(ClaimRefAdvance::Lost)
    } else {
        Err(CommandFailure::transient(format!(
            "advance claim ref failed without changing authoritative ownership: {}",
            String::from_utf8_lossy(&push.stderr).trim()
        )))
    }
}

fn claim_git_program() -> std::path::PathBuf {
    std::env::var_os("AUTOSPEC_CLAIM_GIT_BIN")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("git"))
}

fn resolve_claim_remote(repo: &str) -> Result<String, CommandFailure> {
    if let Ok(remote) = std::env::var("AUTOSPEC_CLAIM_GIT_REMOTE") {
        if !remote.trim().is_empty() {
            if remote.starts_with("https://") {
                return validated_claim_remote(repo, &remote);
            }
            if Path::new(&remote).exists() {
                return Ok(remote);
            }
            return Err(CommandFailure::diagnostic(
                "AUTOSPEC_CLAIM_GIT_REMOTE must be a matching HTTPS clone URL or an existing local test remote",
            ));
        }
    }
    // Idempotent read: retry it. A single failed handshake here used to kill the
    // conductor before it could claim anything, and the dead worker's claim then
    // wedged the issue.
    let output = read_gh_with_retry(
        &["repo", "view", repo, "--json", "url", "--jq", ".url"],
        &format!("resolve authenticated claim repository {repo}"),
    )?;
    validated_claim_remote(repo, String::from_utf8_lossy(&output.stdout).trim())
}

fn validated_claim_remote(repo: &str, url: &str) -> Result<String, CommandFailure> {
    let invalid = || {
        CommandFailure::diagnostic(
            "authenticated claim repository URL does not match the requested owner/repo",
        )
    };
    let (expected_owner, expected_repo) = repo.split_once('/').ok_or_else(invalid)?;
    if expected_owner.is_empty()
        || expected_repo.is_empty()
        || expected_repo.contains('/')
        || !safe_repository_segment(expected_owner)
        || !safe_repository_segment(expected_repo)
    {
        return Err(invalid());
    }
    let remainder = url.strip_prefix("https://").ok_or_else(invalid)?;
    let (authority, path) = remainder.split_once('/').ok_or_else(invalid)?;
    if authority.is_empty()
        || authority.contains(['@', '?', '#'])
        || authority.chars().any(char::is_whitespace)
        || path.contains(['?', '#'])
    {
        return Err(invalid());
    }
    let mut segments = path.split('/');
    let owner = segments.next().ok_or_else(invalid)?;
    let repository = segments.next().ok_or_else(invalid)?;
    if segments.next().is_some() {
        return Err(invalid());
    }
    let repository = repository.strip_suffix(".git").unwrap_or(repository);
    if owner != expected_owner
        || repository != expected_repo
        || !safe_repository_segment(owner)
        || !safe_repository_segment(repository)
    {
        return Err(invalid());
    }
    Ok(format!("https://{authority}/{owner}/{repository}.git"))
}

fn safe_repository_segment(segment: &str) -> bool {
    !segment.is_empty()
        && !matches!(segment, "." | "..")
        && segment
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

struct PrivateClaimGitDir {
    path: std::path::PathBuf,
}

impl Drop for PrivateClaimGitDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn private_claim_git_dir(repo: &str) -> Result<PrivateClaimGitDir, CommandFailure> {
    let root = if let Ok(root) = std::env::var("AUTOSPEC_CLAIM_GIT_STATE_DIR") {
        std::path::PathBuf::from(root)
    } else {
        let home = std::env::var("HOME").map_err(|_| {
            CommandFailure::diagnostic("HOME is required for private claim Git state")
        })?;
        std::path::PathBuf::from(home).join(".autospec/state/claim-git")
    };
    private_claim_git_dir_in(&root, repo)
}

fn private_claim_git_dir_in(root: &Path, repo: &str) -> Result<PrivateClaimGitDir, CommandFailure> {
    fs::create_dir_all(root).map_err(|error| {
        CommandFailure::diagnostic(format!("create private claim Git state: {error}"))
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(root, fs::Permissions::from_mode(0o700)).map_err(|error| {
            CommandFailure::diagnostic(format!("protect private claim Git state: {error}"))
        })?;
    }
    let repo_key = repo.replace('/', "__");
    let path = root.join(format!("{repo_key}-{}", unique_operation_id("git")?));
    fs::create_dir(&path).map_err(|error| {
        CommandFailure::diagnostic(format!("create private claim Git operation: {error}"))
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).map_err(|error| {
            CommandFailure::diagnostic(format!("protect private claim Git operation: {error}"))
        })?;
    }
    let init = run_git_in(
        &claim_git_program(),
        &path,
        &["init", "--bare", "."],
        "initialize private claim Git operation",
    )?;
    if !init.status.success() {
        return Err(CommandFailure::diagnostic(format!(
            "initialize private claim Git operation: {}",
            String::from_utf8_lossy(&init.stderr).trim()
        )));
    }
    Ok(PrivateClaimGitDir { path })
}

fn read_claim_ref(repo: &str, issue: u64) -> Result<Option<ClaimRefHead>, CommandFailure> {
    let private = private_claim_git_dir(repo)?;
    read_claim_ref_in(
        &claim_git_program(),
        &private.path,
        &resolve_claim_remote(repo)?,
        repo,
        issue,
    )
}

fn advance_claim_ref(
    repo: &str,
    issue: u64,
    expected: Option<&ClaimRefHead>,
    record: &RunStateRecord,
) -> Result<ClaimRefAdvance, CommandFailure> {
    let private = private_claim_git_dir(repo)?;
    advance_claim_ref_in(
        &claim_git_program(),
        &private.path,
        &resolve_claim_remote(repo)?,
        repo,
        issue,
        expected,
        record,
    )
}

#[cfg(test)]
pub(crate) fn advance_claim_ref_for_test(
    workdir: &Path,
    record: &RunStateRecord,
) -> Result<bool, CommandFailure> {
    let expected = read_claim_ref_in(
        &claim_git_program(),
        workdir,
        "origin",
        &record.repo,
        record.issue,
    )?;
    advance_claim_ref_in(
        &claim_git_program(),
        workdir,
        "origin",
        &record.repo,
        record.issue,
        expected.as_ref(),
        record,
    )
    .map(|result| matches!(result, ClaimRefAdvance::Won(_)))
}

fn project_claim_ref_to_comments(repo: &str, head: &ClaimRefHead) {
    let result = list_comments(repo, head.record.issue).and_then(|comments| {
        if comments.iter().any(|comment| {
            parse_run_state_comment(&comment.body).is_ok_and(|record| record == head.record)
        }) {
            return Ok(());
        }
        let parent = authoritative_run_state_comment(&comments);
        create_comment(
            repo,
            head.record.issue,
            &linked_run_state_comment(&head.record, parent, &head.generation),
        )
    });
    if let Err(error) = result {
        eprintln!(
            "WARN: claim ref advanced but GitHub audit projection failed: {}",
            error.message.trim()
        );
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WaitFailureRecovery {
    Requeued,
    AlreadyRecovered,
    OwnershipLost,
}

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
        [command, rest @ ..] if command == "refresh" => refresh(rest),
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
    let Some(selected) = read_claim_ref(&repo, options.issue)? else {
        return Err(CommandFailure::status(
            "claim release lost ownership: no authoritative claim ref",
            2,
        ));
    };
    if !has_exact_claim_generation(
        &selected.record,
        &worker_id,
        &options.claim_id,
        &options.branch,
    ) {
        return Err(CommandFailure::status(
            "claim release lost ownership: worker, branch, or claim ID changed",
            2,
        ));
    }
    let now = utc_now_iso()?;
    let record = RunStateRecord::new(
        &repo,
        options.issue,
        &worker_id,
        &options.state,
        &options.branch,
        &options.pr,
        &options.state,
        Vec::new(),
        &selected.record.claimed_at,
        &now,
        selected.record.ttl_seconds,
    )
    .with_claim_id(&options.claim_id);
    let head = match advance_claim_ref(&repo, options.issue, Some(&selected), &record)? {
        ClaimRefAdvance::Won(head) => head,
        ClaimRefAdvance::Lost => {
            return Err(CommandFailure::status(
                "claim release lost ownership during terminal transition",
                2,
            ))
        }
    };
    project_claim_ref_to_comments(&repo, &head);
    if options.state == "merged" {
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
        "{{\"claimed\":true,\"issue\":{},\"repo\":\"{}\",\"worker_id\":\"{}\",\"branch\":\"{}\",\"claim_id\":\"{}\",\"session_id\":{}}}",
        lease.issue,
        json_escape(&lease.repo),
        json_escape(&lease.worker_id),
        json_escape(&lease.branch),
        json_escape(&lease.claim_id),
        lease.session_id.as_ref().map_or_else(
            || "null".to_string(),
            |session_id| format!("\"{}\"", json_escape(session_id)),
        ),
    );
    Ok(())
}

pub(crate) fn acquire_for_conductor(
    repo: &str,
    issue: u64,
    worker_id: &str,
    branch: &str,
    base_branch: &str,
) -> Result<ClaimLease, ConductorClaimError> {
    let recovered = recover_active_issue_against(repo, issue, 300, Some(base_branch))?;
    if !recovered {
        if let Some(owner) = lease::active_contesting_owner(repo, issue, worker_id, branch)? {
            return unavailable_claim_with_observed_owner(issue, repo, worker_id, &owner);
        }
    }
    acquire_record(AcquireOptions {
        issue,
        repo: Some(repo.to_string()),
        worker_id: Some(worker_id.to_string()),
        branch: branch.to_string(),
        session_id: None,
    })
}

pub(crate) fn recover_for_conductor(
    repo: &str,
    issue: u64,
    expected: &ClaimLease,
) -> Result<Option<ClaimLease>, CommandFailure> {
    let Some(selected) = read_claim_ref(repo, issue)? else {
        return Ok(None);
    };
    let record = selected.record;
    if record.repo != repo
        || record.issue != issue
        || !matches!(
            record.state.as_str(),
            "claimed"
                | "merged"
                | "released"
                | "failed"
                | "retryable"
                | "needs-human"
                | "available"
        )
    {
        return Err(CommandFailure::diagnostic(
            "foreground recovery claim is not authoritative for the selected issue",
        ));
    }
    if record.state != "claimed" {
        return Ok(None);
    }
    let claim_id = record.claim_id.ok_or_else(|| {
        CommandFailure::diagnostic("foreground recovery claim has no generation identity")
    })?;
    if expected.repo != repo
        || expected.issue != issue
        || record.worker_id != expected.worker_id
        || record.branch != expected.branch
        || claim_id != expected.claim_id
    {
        return Err(CommandFailure::diagnostic(
            "foreground recovery claim does not match the durable local acquisition",
        ));
    }
    Ok(Some(ClaimLease {
        issue,
        repo: repo.to_string(),
        worker_id: record.worker_id,
        branch: record.branch,
        claim_id,
        session_id: None,
    }))
}

pub(crate) fn recover_terminal_for_conductor(
    repo: &str,
    issue: u64,
) -> Result<Option<ClaimLease>, CommandFailure> {
    let Some(selected) = read_claim_ref(repo, issue)? else {
        return Ok(None);
    };
    let record = selected.record;
    if record.repo != repo || record.issue != issue {
        return Err(CommandFailure::diagnostic(
            "foreground terminal recovery claim is not authoritative for the selected issue",
        ));
    }
    if !matches!(
        record.state.as_str(),
        "merged" | "released" | "failed" | "retryable" | "needs-human"
    ) {
        return Ok(None);
    }
    let claim_id = record.claim_id.ok_or_else(|| {
        CommandFailure::diagnostic("foreground terminal recovery claim has no generation identity")
    })?;
    Ok(Some(ClaimLease {
        issue,
        repo: repo.to_string(),
        worker_id: record.worker_id,
        branch: record.branch,
        claim_id,
        session_id: None,
    }))
}

pub(crate) fn authoritative_lease_for_legacy_migration(
    repo: &str,
    issue: u64,
) -> Result<Option<ClaimLease>, CommandFailure> {
    let Some(selected) = read_claim_ref(repo, issue)? else {
        return Ok(None);
    };
    let record = selected.record;
    if record.repo != repo
        || record.issue != issue
        || !matches!(
            record.state.as_str(),
            "claimed" | "merged" | "released" | "failed" | "retryable" | "needs-human"
        )
    {
        return Err(CommandFailure::diagnostic(
            "legacy migration claim is not authoritative for the selected issue",
        ));
    }
    let claim_id = record.claim_id.ok_or_else(|| {
        CommandFailure::diagnostic("legacy migration claim has no generation identity")
    })?;
    Ok(Some(ClaimLease {
        issue,
        repo: repo.to_string(),
        worker_id: record.worker_id,
        branch: record.branch,
        claim_id,
        session_id: None,
    }))
}

pub(crate) fn conductor_claim_is_terminal(repo: &str, issue: u64) -> Result<bool, CommandFailure> {
    recover_terminal_for_conductor(repo, issue).map(|lease| lease.is_some())
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
    let Some(selected) = read_claim_ref(repo, issue.get())? else {
        return Ok(ClaimEvidence::Observed(ClaimContext::active(
            scope,
            issue,
            requested_worker,
            requested_branch,
            LeaseFreshness::Fresh,
        )));
    };
    lifecycle_claim_evidence_from_record(
        scope,
        issue,
        requested_worker,
        requested_branch,
        &selected.record,
    )
}

fn lifecycle_claim_evidence_from_record(
    scope: RepositoryScope,
    issue: IssueNumber,
    requested_worker: WorkerId,
    requested_branch: ClaimBranch,
    record: &RunStateRecord,
) -> Result<ClaimEvidence, CommandFailure> {
    let worker = WorkerId::try_from(record.worker_id.as_str()).map_err(|reason| {
        CommandFailure::diagnostic(format!("invalid recorded claim worker: {reason}"))
    })?;
    let branch = ClaimBranch::try_from(record.branch.as_str()).map_err(|reason| {
        CommandFailure::diagnostic(format!("invalid recorded claim branch: {reason}"))
    })?;
    record.claim_id.as_deref().ok_or_else(|| {
        CommandFailure::diagnostic("recorded lifecycle claim has no generation identity")
    })?;
    if matches!(record.state.as_str(), "released" | "retryable") {
        return Ok(ClaimEvidence::Observed(ClaimContext::active(
            scope,
            issue,
            requested_worker,
            requested_branch,
            LeaseFreshness::Fresh,
        )));
    }
    if record.state == "merged" {
        return Ok(ClaimEvidence::Observed(ClaimContext::terminal(
            scope, issue, worker, branch,
        )));
    }
    Ok(ClaimEvidence::Observed(ClaimContext::active(
        scope,
        issue,
        worker,
        branch,
        lifecycle_lease_freshness(&record.updated_at),
    )))
}

/// Confirm that premerge evidence belongs to the one active claim generation.
/// This is intentionally observational: it reads the deterministic claim
/// linearization point without refreshing or otherwise mutating the lease.
pub(crate) fn active_claim_generation_matches(
    repo: &str,
    issue: u64,
    worker_id: &str,
    claim_id: &str,
    branch: &str,
) -> Result<bool, CommandFailure> {
    let Some(selected) = read_claim_ref(repo, issue)? else {
        return Ok(false);
    };
    Ok(selected.record.repo == repo
        && selected.record.issue == issue
        && selected.record.worker_id == worker_id
        && selected.record.claim_id.as_deref() == Some(claim_id)
        && selected.record.branch == branch
        && selected.record.state == "claimed"
        && server_lease_is_fresh(&selected.record.updated_at, selected.record.ttl_seconds))
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

fn heartbeat_claim_step(phase: &str, session_id: Option<&str>) -> String {
    let session = session_id
        .map(heartbeat_session_key)
        .unwrap_or_else(|| "none".to_string());
    format!("heartbeat-{phase}:{session}")
}

fn heartbeat_lifecycle_step(step: &str) -> bool {
    let session = |value: &str| {
        value == "none"
            || (!value.is_empty()
                && value.len().is_multiple_of(2)
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f')))
    };
    match step.split(':').collect::<Vec<_>>().as_slice() {
        ["heartbeat-pending" | "heartbeat-ready", value] => session(value),
        ["heartbeat-publishing", value, attempt] => session(value) && !attempt.is_empty(),
        _ => false,
    }
}

fn resumable_heartbeat_phase(
    head: &ClaimRefHead,
    worker_id: &str,
    branch: &str,
    session_id: Option<&str>,
) -> Option<&'static str> {
    if head.record.state != "claimed"
        || head.record.worker_id != worker_id
        || head.record.branch != branch
        || head.record.claim_id.is_none()
    {
        return None;
    }
    ["pending", "ready"]
        .into_iter()
        .find(|phase| head.record.step == heartbeat_claim_step(phase, session_id))
        .or_else(|| {
            let publishing = heartbeat_claim_step("publishing", session_id) + ":";
            (head.record.step.starts_with(&publishing)
                && server_lease_is_stale(&head.record.updated_at, head.record.ttl_seconds))
            .then_some("pending")
        })
}

fn acquire_record(options: AcquireOptions) -> Result<ClaimLease, ConductorClaimError> {
    let repo = match options.repo {
        Some(repo) => repo,
        None => infer_repo()?,
    };
    let worker_id = options.worker_id.unwrap_or_else(default_worker_id);
    let prior = read_claim_ref(&repo, options.issue)?;
    let resume = prior.as_ref().and_then(|head| {
        resumable_heartbeat_phase(
            head,
            &worker_id,
            &options.branch,
            options.session_id.as_deref(),
        )
    });
    let issue = load_claim_issue(&repo, options.issue)?;
    if resume.is_none() && !issue.labels.iter().any(|label| label == "auto-implement") {
        return unavailable_claim(options.issue, &repo, Some(&worker_id), "not_auto_implement");
    }
    let safety = claim_safety_with_config(&issue.safety_input())?;
    if !safety.allowed {
        return unavailable_safety_claim(options.issue, &repo, &worker_id, safety.reason);
    }
    if prior
        .as_ref()
        .is_some_and(|head| head.record.state == "merged")
    {
        return unavailable_claim(options.issue, &repo, Some(&worker_id), "already_merged");
    }
    if resume.is_none() {
        if let Some(owner) = prior
            .as_ref()
            .and_then(|head| lease::acquisition_blocking_owner(&head.record))
        {
            return unavailable_claim_with_observed_owner(options.issue, &repo, &worker_id, owner);
        }
        if let Err(error) = heartbeat_predecessor::retire(&repo, options.issue, prior.as_ref()) {
            eprintln!(
                "WARN: predecessor heartbeat retirement deferred: {}",
                error.message
            );
            return unavailable_claim(
                options.issue,
                &repo,
                Some(&worker_id),
                "predecessor_heartbeat_retirement_failed",
            );
        }
    }

    let (claim_id, mut head, phase) = if let Some(phase) = resume {
        let head = prior.clone().expect("resume requires a claim ref");
        let claim_id = head
            .record
            .claim_id
            .clone()
            .expect("resumable claim has an ID");
        (claim_id, Box::new(head), phase)
    } else {
        let claim_id = claim_generation_id()?;
        let now = utc_now_iso()?;
        let record = RunStateRecord::new(
            &repo,
            options.issue,
            &worker_id,
            "claimed",
            &options.branch,
            "",
            heartbeat_claim_step("pending", options.session_id.as_deref()),
            Vec::new(),
            &now,
            &now,
            claim_ttl_seconds(),
        )
        .with_claim_id(claim_id.clone());
        let head = match advance_claim_ref(&repo, options.issue, prior.as_ref(), &record)? {
            ClaimRefAdvance::Won(head) => head,
            ClaimRefAdvance::Lost => {
                let owner = read_claim_ref(&repo, options.issue)?
                    .map(|head| head.record.worker_id)
                    .unwrap_or_default();
                return unavailable_claim_with_observed_owner(
                    options.issue,
                    &repo,
                    &worker_id,
                    &owner,
                );
            }
        };
        project_claim_ref_to_comments(&repo, &head);
        (claim_id, head, "pending")
    };

    if phase == "pending" {
        let mut publishing = head.record.clone();
        publishing.step = format!(
            "{}:{}",
            heartbeat_claim_step("publishing", options.session_id.as_deref()),
            claim_generation_id()?
        );
        publishing.updated_at = utc_now_iso()?;
        head = match advance_claim_ref(&repo, options.issue, Some(&head), &publishing)? {
            ClaimRefAdvance::Won(head) => head,
            ClaimRefAdvance::Lost => {
                return unavailable_claim(
                    options.issue,
                    &repo,
                    Some(&worker_id),
                    "heartbeat_publish_transition_lost",
                )
            }
        };
        if write_startup_heartbeat(
            &repo,
            options.issue,
            &worker_id,
            &options.branch,
            &claim_id,
            options.session_id.as_deref(),
        )
        .is_err()
        {
            let mut pending = head.record.clone();
            pending.step = heartbeat_claim_step("pending", options.session_id.as_deref());
            pending.updated_at = utc_now_iso()?;
            if let ClaimRefAdvance::Won(pending) =
                advance_claim_ref(&repo, options.issue, Some(&head), &pending)?
            {
                project_claim_ref_to_comments(&repo, &pending);
            }
            return unavailable_claim(
                options.issue,
                &repo,
                Some(&worker_id),
                "heartbeat_write_failed",
            );
        }
        let mut ready = head.record.clone();
        ready.step = heartbeat_claim_step("ready", options.session_id.as_deref());
        ready.updated_at = utc_now_iso()?;
        head = match advance_claim_ref(&repo, options.issue, Some(&head), &ready)? {
            ClaimRefAdvance::Won(head) => head,
            ClaimRefAdvance::Lost => {
                return unavailable_claim(
                    options.issue,
                    &repo,
                    Some(&worker_id),
                    "heartbeat_ready_transition_lost",
                )
            }
        };
        project_claim_ref_to_comments(&repo, &head);
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
    if !issue
        .labels
        .iter()
        .any(|label| label == "in-progress-by-bot")
        && run_gh_with_retry(&label_move, "mark issue in progress").is_err()
    {
        return unavailable_claim(
            options.issue,
            &repo,
            Some(&worker_id),
            "label_mutation_failed",
        );
    }

    for confirmation in 0..claim_confirm_reads() {
        if confirmation > 0 {
            sleep_claim_settle_interval();
        }
        let observed = read_claim_ref(&repo, options.issue)?;
        let labels = load_claim_issue(&repo, options.issue)?.labels;
        if observed.as_ref().is_none_or(|head| {
            head.record.worker_id != worker_id
                || head.record.state != "claimed"
                || head.record.claim_id.as_deref() != Some(claim_id.as_str())
                || head.record.branch != options.branch
                || head.record.step != heartbeat_claim_step("ready", options.session_id.as_deref())
        }) || !labels.iter().any(|label| label == "in-progress-by-bot")
        {
            let owner = observed
                .as_ref()
                .map(|head| head.record.worker_id.as_str())
                .unwrap_or_default();
            return unavailable_claim_with_observed_owner(options.issue, &repo, &worker_id, owner);
        }
    }
    let mut claimed = head.record.clone();
    claimed.step = "claimed".to_string();
    claimed.updated_at = utc_now_iso()?;
    let head = match advance_claim_ref(&repo, options.issue, Some(&head), &claimed)? {
        ClaimRefAdvance::Won(head) => head,
        ClaimRefAdvance::Lost => {
            return unavailable_claim(
                options.issue,
                &repo,
                Some(&worker_id),
                "claim_confirmation_transition_lost",
            )
        }
    };
    project_claim_ref_to_comments(&repo, &head);
    emit_claim_telemetry("session.started", &repo, options.issue, "claimed");
    Ok(ClaimLease {
        issue: options.issue,
        repo,
        worker_id,
        branch: options.branch,
        claim_id,
        session_id: options.session_id,
    })
}

#[cfg(target_os = "linux")]
fn released_predecessor_heartbeat_evidence_exists(
    identity: ClaimMutationIdentity<'_>,
) -> Result<bool, CommandFailure> {
    use nix::fcntl::{open, OFlag};
    use nix::sys::stat::Mode;
    use std::os::fd::AsRawFd;

    let root_path = heartbeat_root()?;
    let root = match open(
        &root_path,
        OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
        Mode::empty(),
    ) {
        Err(nix::errno::Errno::ENOENT) => return Ok(false),
        Err(error) => {
            return Err(CommandFailure::diagnostic(format!(
                "heartbeat root inspection failed: {error}"
            )))
        }
        Ok(root) => fs::File::from(root),
    };
    private_heartbeat_directory_identity(&root, "predecessor root")?;
    let repo_name = super::autonomous::drain::repository_progress_key(identity.repo);
    let Some(repo) = open_optional_heartbeat_directory(&root, Path::new(&repo_name))? else {
        return Ok(false);
    };
    let issue_name = format!("{}.json", identity.issue);
    match read_regular_file_at_no_follow(&repo, issue_name.as_ref()) {
        Ok(_) => return Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(CommandFailure::diagnostic(format!(
                "predecessor heartbeat inspection failed: {error}"
            )))
        }
    }
    let Some(quarantine) = open_optional_heartbeat_directory(&repo, Path::new("quarantine"))?
    else {
        return Ok(false);
    };
    let Some(handoff) =
        open_optional_heartbeat_directory(&quarantine, Path::new("startup-heartbeat-handoffs"))?
    else {
        return Ok(false);
    };
    let directory = format!("/proc/self/fd/{}", handoff.as_raw_fd());
    for entry in fs::read_dir(directory).map_err(|error| {
        CommandFailure::diagnostic(format!(
            "predecessor heartbeat handoff scan failed: {error}"
        ))
    })? {
        let name = entry
            .map_err(|error| {
                CommandFailure::diagnostic(format!(
                    "predecessor heartbeat handoff entry failed: {error}"
                ))
            })?
            .file_name()
            .into_string()
            .map_err(|_| CommandFailure::diagnostic("predecessor heartbeat handoff is unsafe"))?;
        let pending = format!("pending-{}-", identity.issue);
        let completed = format!("completed-{}-", identity.issue);
        if (name.starts_with(&pending) || name.starts_with(&completed))
            && (name.ends_with(".receipt") || name.ends_with(".json"))
        {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(target_os = "linux")]
fn open_optional_heartbeat_directory(
    parent: &fs::File,
    descendant: &Path,
) -> Result<Option<fs::File>, CommandFailure> {
    use nix::fcntl::AtFlags;
    use nix::sys::stat::fstatat;

    match fstatat(parent, descendant, AtFlags::AT_SYMLINK_NOFOLLOW) {
        Err(nix::errno::Errno::ENOENT) => Ok(None),
        Err(error) => Err(CommandFailure::diagnostic(format!(
            "heartbeat directory inspection failed: {error}"
        ))),
        Ok(_) => open_heartbeat_directory_beneath(parent, descendant).map(Some),
    }
}

fn read(args: &[String]) -> Result<(), CommandFailure> {
    let options = parse_read_options(args)?;
    let repo = match options.repo {
        Some(repo) => repo,
        None => infer_repo()?,
    };
    if let Some(selected) = read_claim_ref(&repo, options.issue)? {
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
    let now = utc_now_iso()?;
    let selected = read_claim_ref(&repo, options.issue)?.ok_or_else(|| {
        CommandFailure::diagnostic("claim state upsert requires an active claim ref")
    })?;
    if !has_exact_claim_generation(
        &selected.record,
        &options.worker_id,
        &options.claim_id,
        &options.branch,
    ) {
        return Err(CommandFailure::diagnostic(
            "claim state upsert lost exact worker, branch, or claim ID ownership",
        ));
    }
    let claimed_at = selected.record.claimed_at.clone();
    let mut record = RunStateRecord::new(
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
    record.claim_id = Some(options.claim_id.clone());
    upsert_record(&repo, &[], &options.claim_id, &record)?;
    emit_claim_telemetry("session.step", &repo, options.issue, &record.step);
    println!("{}", record.to_json());
    Ok(())
}

fn refresh(args: &[String]) -> Result<(), CommandFailure> {
    let options = parse_refresh_options(args)?;
    let repo = match options.repo {
        Some(repo) => repo,
        None => infer_repo()?,
    };
    match refresh_claim_generation(
        &repo,
        options.issue,
        &options.worker_id,
        &options.claim_id,
        &options.branch,
        &options.step,
        options.pr.as_deref(),
    )? {
        ClaimRefreshResult::Refreshed(record) => {
            println!("{}", record.to_json());
            Ok(())
        }
        ClaimRefreshResult::OwnershipLost => {
            println!(
                "{{\"refreshed\":false,\"repo\":\"{}\",\"issue\":{},\"reason\":\"ownership_lost\"}}",
                json_escape(&repo),
                options.issue
            );
            Err(CommandFailure::status(String::new(), 2))
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ClaimRefreshResult {
    Refreshed(Box<RunStateRecord>),
    OwnershipLost,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RunStateLink {
    parent: Option<u64>,
    parent_generation: Option<String>,
    generation: String,
}

fn run_state_link(body: &str) -> Option<RunStateLink> {
    let line = body
        .lines()
        .find(|line| line.starts_with(RUN_STATE_LINK_PREFIX))?;
    let fields = line
        .strip_prefix(RUN_STATE_LINK_PREFIX)?
        .strip_suffix(RUN_STATE_LINK_SUFFIX)?;
    let mut fields = fields.split_whitespace();
    let parent = match fields.next()? {
        "none" => None,
        value => Some(value.parse().ok()?),
    };
    let mut parent_generation = None;
    let mut generation = None;
    for field in fields {
        let (name, value) = field.split_once('=')?;
        match name {
            "parent_generation" => parent_generation = Some(value.to_string()),
            "generation" => generation = Some(value.to_string()),
            _ => return None,
        }
    }
    let generation = generation?;
    let valid_token = |value: &str| {
        !value.is_empty()
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    };
    if !valid_token(&generation)
        || parent_generation
            .as_deref()
            .is_some_and(|value| !valid_token(value))
        || parent.is_none()
            && parent_generation
                .as_deref()
                .is_some_and(|value| value != "none")
    {
        return None;
    }
    if parent.is_some() && parent_generation.is_none() {
        parent_generation = Some("legacy".to_string());
    }
    Some(RunStateLink {
        parent,
        parent_generation,
        generation,
    })
}

fn run_state_parent(body: &str) -> Option<Option<u64>> {
    let line = body
        .lines()
        .find(|line| line.starts_with(RUN_STATE_LINK_PREFIX))?;
    let parent = line
        .strip_prefix(RUN_STATE_LINK_PREFIX)?
        .split_once(' ')
        .map(|(parent, _)| parent)?;
    match parent {
        "none" => Some(None),
        value => value.parse().ok().map(Some),
    }
}

fn marked_run_state(comment: &RemoteComment) -> bool {
    comment.body.contains(RUN_STATE_BEGIN_MARKER) && comment.body.contains(RUN_STATE_END_MARKER)
}

fn authoritative_run_state_comment(comments: &[RemoteComment]) -> Option<&RemoteComment> {
    let mut current = comments
        .iter()
        .filter(|comment| {
            marked_run_state(comment)
                && run_state_parent(&comment.body).is_none_or(|parent| parent.is_none())
        })
        .min_by_key(|comment| comment.id)?;
    loop {
        let child = comments
            .iter()
            .filter(|comment| {
                comment.id > current.id
                    && marked_run_state(comment)
                    && run_state_parent(&comment.body)
                        .is_some_and(|parent| parent == Some(current.id))
            })
            .min_by_key(|comment| comment.id);
        match child {
            Some(child) => current = child,
            None => return Some(current),
        }
    }
}

fn select_run_state(
    comments: &[RemoteComment],
    repo: &str,
    issue: u64,
) -> Option<SelectedRunState> {
    let comment = authoritative_run_state_comment(comments)?;
    if comment.body.contains(RUN_STATE_LINK_PREFIX) && run_state_link(&comment.body).is_none() {
        return None;
    }
    let record = parse_run_state_comment(&comment.body).ok()?;
    if record.repo != repo || record.issue != issue {
        return None;
    }
    Some(SelectedRunState {
        comment_id: comment.id,
        server_updated_at: comment.updated_at.clone(),
        record,
    })
}

fn linked_run_state_comment(
    record: &RunStateRecord,
    parent: Option<&RemoteComment>,
    generation: &str,
) -> String {
    let (parent, parent_generation) = parent.map_or_else(
        || ("none".to_string(), "none".to_string()),
        |comment| {
            (
                comment.id.to_string(),
                run_state_link(&comment.body)
                    .map(|link| link.generation)
                    .unwrap_or_else(|| "legacy".to_string()),
            )
        },
    );
    format!(
        "{RUN_STATE_LINK_PREFIX}{parent} parent_generation={parent_generation} generation={generation}{RUN_STATE_LINK_SUFFIX}\n{}",
        record.to_marked_comment()
    )
}

/// Renew one exact claim generation without changing its immutable identity.
///
/// The dedicated Git claim ref is the linearization point. An expired
/// heartbeat may still be renewed when that exact commit remains authoritative;
/// any changed owner, branch, claim ID, or post-write takeover makes the caller
/// inert.
pub(crate) fn refresh_claim_generation(
    repo: &str,
    issue: u64,
    worker_id: &str,
    claim_id: &str,
    branch: &str,
    step: &str,
    pull_request: Option<&str>,
) -> Result<ClaimRefreshResult, CommandFailure> {
    let Some(selected) = read_claim_ref(repo, issue)? else {
        return Ok(ClaimRefreshResult::OwnershipLost);
    };
    if !has_exact_claim_generation(&selected.record, worker_id, claim_id, branch) {
        return Ok(ClaimRefreshResult::OwnershipLost);
    }

    let mut refreshed = selected.record.clone();
    refreshed.updated_at = utc_now_iso()?;
    refreshed.step = step.to_string();
    if let Some(pull_request) = pull_request {
        refreshed.pr = pull_request.to_string();
    }
    match advance_claim_ref(repo, issue, Some(&selected), &refreshed)? {
        ClaimRefAdvance::Won(head) => {
            project_claim_ref_to_comments(repo, &head);
            Ok(ClaimRefreshResult::Refreshed(Box::new(head.record)))
        }
        ClaimRefAdvance::Lost => Ok(ClaimRefreshResult::OwnershipLost),
    }
}

fn has_exact_claim_generation(
    record: &RunStateRecord,
    worker_id: &str,
    claim_id: &str,
    branch: &str,
) -> bool {
    record.state == "claimed"
        && record.worker_id == worker_id
        && record.claim_id.as_deref() == Some(claim_id)
        && record.branch == branch
}

fn upsert_record(
    repo: &str,
    _comments: &[autospec_core::claim::RemoteComment],
    expected_claim_id: &str,
    record: &RunStateRecord,
) -> Result<(), CommandFailure> {
    let parent = read_claim_ref(repo, record.issue)?
        .ok_or_else(|| CommandFailure::diagnostic("run-state mutation requires a claim ref"))?;
    if parent.record.worker_id != record.worker_id
        || parent.record.branch != record.branch
        || parent.record.claim_id.as_deref() != Some(expected_claim_id)
        || record.claim_id.as_deref() != Some(expected_claim_id)
    {
        return Err(CommandFailure::diagnostic(
            "run-state mutation lost exact worker, branch, or claim ID ownership",
        ));
    }
    match advance_claim_ref(repo, record.issue, Some(&parent), record)? {
        ClaimRefAdvance::Won(head) => {
            project_claim_ref_to_comments(repo, &head);
            Ok(())
        }
        ClaimRefAdvance::Lost => Err(CommandFailure::diagnostic(
            "run-state mutation lost the authoritative ref race",
        )),
    }
}

fn clear(args: &[String]) -> Result<(), CommandFailure> {
    let options = parse_clear_options(args)?;
    let repo = match options.repo {
        Some(repo) => repo,
        None => infer_repo()?,
    };
    let Some(selected) = read_claim_ref(&repo, options.issue)? else {
        return Err(CommandFailure::diagnostic(
            "claim state clear requires an authoritative claim ref",
        ));
    };
    if !has_exact_claim_generation(
        &selected.record,
        &options.worker_id,
        &options.claim_id,
        &options.branch,
    ) {
        return Err(CommandFailure::diagnostic(
            "claim state clear lost exact worker, branch, or claim ID ownership",
        ));
    }
    let mut released = selected.record.clone();
    released.state = "released".to_string();
    released.step = "cleared".to_string();
    released.updated_at = utc_now_iso()?;
    match advance_claim_ref(&repo, options.issue, Some(&selected), &released)? {
        ClaimRefAdvance::Won(head) => project_claim_ref_to_comments(&repo, &head),
        ClaimRefAdvance::Lost => {
            return Err(CommandFailure::diagnostic(
                "claim state clear lost the authoritative ref race",
            ))
        }
    }
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

struct RecoveryOutcome {
    recovered: bool,
    reason: String,
}

pub(crate) fn recover_active_issue(
    repo: &str,
    issue: u64,
    timeout_seconds: u64,
) -> Result<bool, CommandFailure> {
    recover_active_issue_against(repo, issue, timeout_seconds, None)
}

fn recover_active_issue_against(
    repo: &str,
    issue: u64,
    timeout_seconds: u64,
    base_branch: Option<&str>,
) -> Result<bool, CommandFailure> {
    let Some(selected) = read_claim_ref(repo, issue)? else {
        return Ok(false);
    };
    recover_authoritative_stale_startup(repo, issue, timeout_seconds, selected, base_branch)
        .map(|outcome| outcome.recovered)
}

/// A newly-created claim without a heartbeat, branch, or PR is kept during its
/// startup grace period but must not consume a worker slot yet. Read failures
/// and malformed records remain counted so the queue fails closed.
pub(crate) fn active_issue_counts_toward_worker_capacity(
    repo: &str,
    issue: u64,
    timeout_seconds: u64,
) -> Result<bool, CommandFailure> {
    if let Some(selected) = read_claim_ref(repo, issue)? {
        return active_record_counts_toward_worker_capacity(
            repo,
            issue,
            &selected.record,
            timeout_seconds,
        );
    }
    let comments = list_comments(repo, issue)?;
    let Some(selected) = select_run_state(&comments, repo, issue) else {
        return Ok(true);
    };
    active_record_counts_toward_worker_capacity(repo, issue, &selected.record, timeout_seconds)
}

fn active_record_counts_toward_worker_capacity(
    repo: &str,
    issue: u64,
    record: &RunStateRecord,
    timeout_seconds: u64,
) -> Result<bool, CommandFailure> {
    if record.state != "claimed" {
        return Ok(true);
    }
    if !record.pr.is_empty()
        || startup_heartbeat_exists(repo, issue)
        || branch_ref_exists(&record.branch)
    {
        return Ok(true);
    }
    let Some(updated_at) = parse_iso_timestamp(&record.updated_at) else {
        return Ok(true);
    };
    Ok(unix_now()
        .map(|now| now.saturating_sub(updated_at) > timeout_seconds)
        .unwrap_or(true))
}

fn recover_stale_startup_record(
    repo: &str,
    issue: u64,
    timeout_seconds: u64,
) -> Result<RecoveryOutcome, CommandFailure> {
    if let Some(selected) = read_claim_ref(repo, issue)? {
        return recover_authoritative_stale_startup(repo, issue, timeout_seconds, selected, None);
    }
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
    release_stale_startup_labels(repo, issue)?;
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

fn recover_authoritative_stale_startup(
    repo: &str,
    issue: u64,
    timeout_seconds: u64,
    selected: ClaimRefHead,
    base_branch: Option<&str>,
) -> Result<RecoveryOutcome, CommandFailure> {
    if selected.record.state == "available" && selected.record.step == "stale_startup_recovered" {
        release_stale_startup_labels(repo, issue)?;
        emit_claim_telemetry("session.terminal", repo, issue, "stale_startup_recovered");
        return Ok(RecoveryOutcome {
            recovered: true,
            reason: "released_stale_startup_claim".to_string(),
        });
    }
    if selected.record.state != "claimed"
        || !selected.record.pr.is_empty()
        || (heartbeat_lifecycle_step(&selected.record.step)
            && selected.record.step != "heartbeat-pending:none")
        || !server_lease_is_stale(&selected.record.updated_at, timeout_seconds)
    {
        return Ok(RecoveryOutcome {
            recovered: false,
            reason: "claim_evidence_or_fresh_state".to_string(),
        });
    }
    let branch_blocked = branch_blocks_stale_recovery(&selected.record.branch, base_branch);
    let authorized_prior = if branch_blocked {
        expired_prior_generation_heartbeat(repo, issue, &selected.record)?
    } else {
        None
    };
    if branch_blocked && authorized_prior.is_none() {
        return Ok(RecoveryOutcome {
            recovered: false,
            reason: "claim_evidence_or_fresh_state".to_string(),
        });
    }
    if !quarantine_authoritative_stale_heartbeat(
        repo,
        issue,
        &selected.record,
        authorized_prior.as_deref(),
        &mut || Ok(()),
    )? {
        return Ok(RecoveryOutcome {
            recovered: false,
            reason: "claim_evidence_or_fresh_state".to_string(),
        });
    }
    let mut available = selected.record.clone();
    available.state = "available".to_string();
    available.step = "stale_startup_recovered".to_string();
    available.updated_at = utc_now_iso()?;
    let head = match advance_claim_ref(repo, issue, Some(&selected), &available)? {
        ClaimRefAdvance::Won(head) => head,
        ClaimRefAdvance::Lost => {
            return Ok(RecoveryOutcome {
                recovered: false,
                reason: "ownership_lost".to_string(),
            })
        }
    };
    project_claim_ref_to_comments(repo, &head);
    release_stale_startup_labels(repo, issue)?;
    emit_claim_telemetry("session.terminal", repo, issue, "stale_startup_recovered");
    Ok(RecoveryOutcome {
        recovered: true,
        reason: "released_stale_startup_claim".to_string(),
    })
}

#[cfg(target_os = "linux")]
fn expired_prior_generation_heartbeat(
    repo_name: &str,
    issue: u64,
    record: &RunStateRecord,
) -> Result<Option<Box<StartupHeartbeatSnapshot>>, CommandFailure> {
    use nix::fcntl::{open, OFlag};
    use nix::sys::stat::Mode;

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
    let repo_key = super::autonomous::drain::repository_progress_key(repo_name);
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

#[cfg(not(target_os = "linux"))]
fn expired_prior_generation_heartbeat(
    _repo: &str,
    _issue: u64,
    _record: &RunStateRecord,
) -> Result<Option<Box<StartupHeartbeatSnapshot>>, CommandFailure> {
    Ok(None)
}

#[cfg(target_os = "linux")]
fn quarantine_authoritative_stale_heartbeat(
    repo_name: &str,
    issue: u64,
    record: &RunStateRecord,
    authorized_prior: Option<&StartupHeartbeatSnapshot>,
    boundary: &mut impl FnMut() -> Result<(), CommandFailure>,
) -> Result<bool, CommandFailure> {
    use nix::fcntl::AtFlags;
    use nix::sys::stat::fstatat;

    let root_path = heartbeat_root()?;
    let root = open_private_heartbeat_directory(&root_path)?;
    let repo_key = super::autonomous::drain::repository_progress_key(repo_name);
    let repo_path = Path::new(&repo_key);
    let repo = match fstatat(&root, repo_path, AtFlags::AT_SYMLINK_NOFOLLOW) {
        Err(nix::errno::Errno::ENOENT) => return Ok(true),
        Err(error) => {
            return Err(CommandFailure::diagnostic(format!(
                "heartbeat repository inspection failed: {error}"
            )))
        }
        Ok(_) => open_heartbeat_directory_beneath(&root, repo_path)?,
    };
    let Some(claim_id) = record.claim_id.as_deref() else {
        return Ok(false);
    };
    boundary()?;
    let expected = StartupHeartbeatExpectation {
        repo: repo_name,
        issue,
        worker_id: &record.worker_id,
        branch: &record.branch,
        pull_request: "",
        claim_id,
        step: "claimed",
    };
    let issue_name = format!("{issue}.json");
    let classified = if heartbeat_lifecycle_step(&record.step) {
        match read_regular_file_at_no_follow(&repo, issue_name.as_ref()) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                StartupHeartbeatClassification::Absent
            }
            Err(_) => StartupHeartbeatClassification::Blocking,
            Ok(file) => {
                let evidence = parse_startup_heartbeat(&file.document);
                match evidence {
                    Some(evidence)
                        if (evidence.worker_id != record.worker_id
                            || evidence.claim_id != claim_id)
                            && evidence.repo == repo_name
                            && evidence.issue == issue.to_string()
                            && evidence.branch == record.branch
                            && evidence.pr.is_empty() =>
                    {
                        let prior_generation = StartupHeartbeatExpectation {
                            repo: &evidence.repo,
                            issue,
                            worker_id: &evidence.worker_id,
                            branch: &evidence.branch,
                            pull_request: &evidence.pr,
                            claim_id: &evidence.claim_id,
                            step: &evidence.step,
                        };
                        classify_startup_heartbeat_snapshot(
                            file,
                            prior_generation,
                            unix_now()?,
                            observe_local_startup_pid,
                        )
                    }
                    _ => classify_startup_heartbeat_snapshot(
                        file,
                        expected,
                        unix_now()?,
                        observe_local_startup_pid,
                    ),
                }
            }
        }
    } else {
        classify_startup_heartbeat_at(
            &repo,
            issue_name.as_ref(),
            expected,
            unix_now()?,
            observe_local_startup_pid,
        )
    };
    let snapshot = match classified {
        StartupHeartbeatClassification::Blocking => return Ok(false),
        StartupHeartbeatClassification::ExpiredDead(snapshot) => Some(snapshot),
        StartupHeartbeatClassification::Absent => {
            match heartbeat_receipt_retry_decision(&repo, expected) {
                HeartbeatReceiptDecision::Absent => {
                    match classify_retained_prior_generation(
                        &repo,
                        repo_name,
                        issue,
                        record,
                        unix_now()?,
                    )? {
                        StartupHeartbeatClassification::ExpiredDead(snapshot) => Some(snapshot),
                        StartupHeartbeatClassification::Absent
                            if lease::heartbeat_publication_in_flight(&record.step) =>
                        {
                            return Ok(false)
                        }
                        StartupHeartbeatClassification::Absent => None,
                        StartupHeartbeatClassification::Blocking => return Ok(false),
                    }
                }
                HeartbeatReceiptDecision::Pending | HeartbeatReceiptDecision::Completed => {
                    let snapshot = terminal_heartbeat_snapshot(
                        &repo,
                        issue_name.as_ref(),
                        ClaimMutationIdentity {
                            repo: repo_name,
                            issue,
                            worker_id: &record.worker_id,
                            branch: &record.branch,
                            claim_id,
                        },
                    )?;
                    match classify_startup_heartbeat_snapshot(
                        snapshot.file.clone(),
                        expected,
                        unix_now()?,
                        observe_local_startup_pid,
                    ) {
                        StartupHeartbeatClassification::ExpiredDead(snapshot) => Some(snapshot),
                        StartupHeartbeatClassification::Absent
                        | StartupHeartbeatClassification::Blocking => return Ok(false),
                    }
                }
                HeartbeatReceiptDecision::Blocking => return Ok(false),
            }
        }
    };
    if authorized_prior.is_some_and(|authorized| snapshot.as_deref() != Some(authorized)) {
        return Err(CommandFailure::diagnostic(
            "authorized prior-generation heartbeat changed before quarantine",
        ));
    }
    if let Some(snapshot) = snapshot {
        handoff_retained_heartbeat(
            &root_path.join(&repo_key),
            &repo,
            issue_name.as_ref(),
            &snapshot,
            |_, _, _, _| {},
            |_| Ok(()),
        )?;
    } else if !matches!(
        fstatat(
            &repo,
            std::ffi::OsStr::new(&issue_name),
            AtFlags::AT_SYMLINK_NOFOLLOW,
        ),
        Err(nix::errno::Errno::ENOENT)
    ) {
        return Ok(false);
    }
    let opened = private_heartbeat_directory_identity(&repo, "recovery repository")?;
    if private_heartbeat_name_identity(&root, repo_path, "recovery repository binding")? != opened {
        return Err(CommandFailure::diagnostic(
            "heartbeat recovery repository binding changed",
        ));
    }
    Ok(true)
}

#[cfg(not(target_os = "linux"))]
fn quarantine_authoritative_stale_heartbeat(
    _repo: &str,
    _issue: u64,
    _record: &RunStateRecord,
    _authorized_prior: Option<&StartupHeartbeatSnapshot>,
    _boundary: &mut impl FnMut() -> Result<(), CommandFailure>,
) -> Result<bool, CommandFailure> {
    Ok(false)
}

fn release_stale_startup_labels(repo: &str, issue: u64) -> Result<(), CommandFailure> {
    run_gh_with_retry(
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
        "release stale startup claim labels",
    )
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
    let outcome = reconcile_linked_pr_record(
        &repo,
        options.issue,
        &options.worker_id,
        &options.branch,
        &options.claim_id,
    )?;
    print_reconcile_result(
        outcome.reconciled,
        options.issue,
        &repo,
        outcome.pr.as_deref(),
        &outcome.reason,
    );
    Ok(())
}

pub(crate) fn reconcile_active_issue(
    repo: &str,
    issue: u64,
    worker_id: &str,
    branch: &str,
    expected_claim_id: &str,
) -> Result<(), CommandFailure> {
    let _ = reconcile_linked_pr_record(repo, issue, worker_id, branch, expected_claim_id)?;
    Ok(())
}

pub(crate) fn reconcile_authoritative_active_issue(
    repo: &str,
    issue: u64,
) -> Result<(), CommandFailure> {
    let Some(selected) = read_claim_ref(repo, issue)? else {
        return Ok(());
    };
    reconcile_active_issue(
        repo,
        issue,
        &selected.record.worker_id,
        &selected.record.branch,
        selected
            .record
            .claim_id
            .as_deref()
            .ok_or_else(|| CommandFailure::diagnostic("active claim has no generation identity"))?,
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExecutorResultRecord {
    Recorded,
    EvidenceUnavailable,
    OwnershipLost,
}

pub(crate) struct ExecutorSuccessBinding<'a> {
    pub(crate) claim_id: &'a str,
    pub(crate) commit: &'a str,
    pub(crate) premerge_receipt: &'a str,
}

#[derive(Clone, Copy)]
pub(crate) struct ExecutorResultAuthorityBinding<'a> {
    pub(crate) pull_request: u64,
    pub(crate) commit: &'a str,
    pub(crate) premerge_receipt: &'a str,
    pub(crate) receipt_id: &'a str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BridgeClaimDisposition {
    Merged,
    Retryable,
    NeedsHuman,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BridgeClaimTransition {
    Transitioned,
    OwnershipLost,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BridgeTerminalMode {
    Fresh,
    Prepared,
    Complete,
    Lost,
}

fn bridge_terminal_mode(
    record: &RunStateRecord,
    identity: ClaimMutationIdentity<'_>,
    desired_state: &str,
    desired_step: &str,
    desired_pr: &str,
    prepared_step: &str,
) -> BridgeTerminalMode {
    if record.worker_id != identity.worker_id
        || record.claim_id.as_deref() != Some(identity.claim_id)
        || record.branch != identity.branch
    {
        return BridgeTerminalMode::Lost;
    }
    if record.state == desired_state && record.step == desired_step && record.pr == desired_pr {
        BridgeTerminalMode::Complete
    } else if record.state == "claimed" && record.step == prepared_step && record.pr == desired_pr {
        BridgeTerminalMode::Prepared
    } else if record.state == "claimed" && !record.step.starts_with(BRIDGE_TERMINAL_PREPARED_PREFIX)
    {
        BridgeTerminalMode::Fresh
    } else {
        BridgeTerminalMode::Lost
    }
}

#[derive(Clone, Copy)]
pub(crate) struct ClaimMutationIdentity<'a> {
    pub(crate) repo: &'a str,
    pub(crate) issue: u64,
    pub(crate) worker_id: &'a str,
    pub(crate) branch: &'a str,
    pub(crate) claim_id: &'a str,
}

pub(crate) fn transition_bridge_claim(
    identity: ClaimMutationIdentity<'_>,
    pull_request: Option<u64>,
    disposition: BridgeClaimDisposition,
) -> Result<BridgeClaimTransition, CommandFailure> {
    let Some(selected) = read_claim_ref(identity.repo, identity.issue)? else {
        return Ok(BridgeClaimTransition::OwnershipLost);
    };
    if disposition == BridgeClaimDisposition::Merged && pull_request.is_none() {
        return Err(CommandFailure::diagnostic(
            "merged bridge claim transition requires a pull request",
        ));
    }
    let desired_state = match disposition {
        BridgeClaimDisposition::Merged => "merged",
        BridgeClaimDisposition::Retryable => "released",
        BridgeClaimDisposition::NeedsHuman => "failed",
    };
    let desired_step = match disposition {
        BridgeClaimDisposition::Merged => "merged",
        BridgeClaimDisposition::Retryable => "retryable_released",
        BridgeClaimDisposition::NeedsHuman => "needs_human",
    };
    let prepared_step = format!("{BRIDGE_TERMINAL_PREPARED_PREFIX}{desired_step}");
    let desired_pr = pull_request.map_or_else(String::new, |number| number.to_string());
    let mode = bridge_terminal_mode(
        &selected.record,
        identity,
        desired_state,
        desired_step,
        &desired_pr,
        &prepared_step,
    );
    if mode == BridgeTerminalMode::Lost {
        return Ok(BridgeClaimTransition::OwnershipLost);
    }
    if mode == BridgeTerminalMode::Complete {
        project_bridge_terminal_audit(
            identity,
            pull_request,
            disposition,
            &selected.record,
            &selected,
        )?;
        emit_claim_telemetry(
            "session.terminal",
            identity.repo,
            identity.issue,
            selected.record.step.as_str(),
        );
        return Ok(BridgeClaimTransition::Transitioned);
    }
    let prepared = if mode == BridgeTerminalMode::Prepared {
        Box::new(selected)
    } else {
        let mut record = selected.record.clone();
        record.state = "claimed".to_string();
        record.step = prepared_step;
        record.pr = desired_pr;
        record.updated_at = utc_now_iso()?;
        match advance_claim_ref(identity.repo, identity.issue, Some(&selected), &record)? {
            ClaimRefAdvance::Won(head) => head,
            ClaimRefAdvance::Lost => return Ok(BridgeClaimTransition::OwnershipLost),
        }
    };
    if disposition == BridgeClaimDisposition::Retryable {
        retire_released_startup_heartbeat(identity)?;
    }
    let mut arguments = vec![
        "issue".to_string(),
        "edit".to_string(),
        identity.issue.to_string(),
        "--repo".to_string(),
        identity.repo.to_string(),
        "--remove-label".to_string(),
        "in-progress-by-bot".to_string(),
    ];
    match disposition {
        BridgeClaimDisposition::Merged => {}
        BridgeClaimDisposition::Retryable => {
            arguments.extend([
                "--remove-label".to_string(),
                "autospec:needs-human".to_string(),
                "--add-label".to_string(),
                "auto-implement".to_string(),
            ]);
        }
        BridgeClaimDisposition::NeedsHuman => {
            arguments.extend([
                "--remove-label".to_string(),
                "auto-implement".to_string(),
                "--add-label".to_string(),
                "autospec:needs-human".to_string(),
            ]);
        }
    }
    run_gh_with_retry(&arguments, "transition autonomous bridge claim labels")?;
    let mut record = prepared.record.clone();
    record.state = desired_state.to_string();
    record.step = desired_step.to_string();
    record.updated_at = utc_now_iso()?;
    let head = match advance_claim_ref(identity.repo, identity.issue, Some(&prepared), &record)? {
        ClaimRefAdvance::Won(head) => head,
        ClaimRefAdvance::Lost => return Ok(BridgeClaimTransition::OwnershipLost),
    };
    project_bridge_terminal_audit(identity, pull_request, disposition, &record, &head)?;
    emit_claim_telemetry(
        "session.terminal",
        identity.repo,
        identity.issue,
        record.step.as_str(),
    );
    Ok(BridgeClaimTransition::Transitioned)
}

pub(crate) fn observe_terminal_bridge_claim(
    identity: ClaimMutationIdentity<'_>,
    pull_request: Option<u64>,
    disposition: BridgeClaimDisposition,
) -> Result<bool, CommandFailure> {
    let Some(selected) = read_claim_ref(identity.repo, identity.issue)? else {
        return Ok(false);
    };
    let expected_state = match disposition {
        BridgeClaimDisposition::Merged => "merged",
        BridgeClaimDisposition::Retryable => "released",
        BridgeClaimDisposition::NeedsHuman => "failed",
    };
    let expected_step = match disposition {
        BridgeClaimDisposition::Merged => "merged",
        BridgeClaimDisposition::Retryable => "retryable_released",
        BridgeClaimDisposition::NeedsHuman => "needs_human",
    };
    let expected_pr = pull_request.map_or_else(String::new, |number| number.to_string());
    Ok(selected.record.worker_id == identity.worker_id
        && selected.record.claim_id.as_deref() == Some(identity.claim_id)
        && selected.record.branch == identity.branch
        && selected.record.state == expected_state
        && selected.record.step == expected_step
        && selected.record.pr == expected_pr)
}

pub(crate) fn recover_released_bridge_claim(
    identity: ClaimMutationIdentity<'_>,
) -> Result<bool, CommandFailure> {
    let Some(selected) = read_claim_ref(identity.repo, identity.issue)? else {
        return Ok(false);
    };
    let exact = selected.record.worker_id == identity.worker_id
        && selected.record.claim_id.as_deref() == Some(identity.claim_id)
        && selected.record.branch == identity.branch
        && selected.record.state == "released"
        && selected.record.step == "released"
        && selected.record.pr.is_empty();
    if exact {
        retire_released_startup_heartbeat(identity)?;
    }
    Ok(exact)
}

#[cfg(target_os = "linux")]
pub(crate) fn with_released_bridge_predecessor_authority<T>(
    identity: ClaimMutationIdentity<'_>,
    operation: impl FnOnce() -> Result<T, CommandFailure>,
) -> Result<Option<T>, CommandFailure> {
    let exact_release = read_claim_ref(identity.repo, identity.issue)?.is_some_and(|selected| {
        selected.record.worker_id == identity.worker_id
            && selected.record.claim_id.as_deref() == Some(identity.claim_id)
            && selected.record.branch == identity.branch
            && selected.record.state == "released"
            && selected.record.step == "released"
            && selected.record.pr.is_empty()
    });
    if exact_release {
        retire_released_startup_heartbeat(identity)?;
        return operation().map(Some);
    }
    with_retained_bridge_predecessor_authority(
        identity,
        observe_local_startup_pid,
        || {},
        operation,
    )
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn with_released_bridge_predecessor_authority<T>(
    identity: ClaimMutationIdentity<'_>,
    operation: impl FnOnce() -> Result<T, CommandFailure>,
) -> Result<Option<T>, CommandFailure> {
    if recover_released_bridge_claim(identity)? {
        operation().map(Some)
    } else {
        Ok(None)
    }
}

#[cfg(all(test, not(target_os = "linux")))]
#[allow(dead_code)]
fn with_retained_bridge_predecessor_authority<T>(
    _identity: ClaimMutationIdentity<'_>,
    _observe_pid: impl FnOnce(&str, u32, &str, &str, &str) -> StartupPidLiveness,
    _boundary: impl FnOnce(),
    _operation: impl FnOnce() -> Result<T, CommandFailure>,
) -> Result<Option<T>, CommandFailure> {
    Ok(None)
}

fn project_bridge_terminal_audit(
    identity: ClaimMutationIdentity<'_>,
    pull_request: Option<u64>,
    disposition: BridgeClaimDisposition,
    record: &RunStateRecord,
    head: &ClaimRefHead,
) -> Result<(), CommandFailure> {
    project_claim_ref_to_comments(identity.repo, head);
    if disposition != BridgeClaimDisposition::Merged {
        return Ok(());
    }
    let body = format!(
        "{RUN_TERMINAL_BEGIN_MARKER}\n{{\"schema\":1,\"repo\":\"{}\",\"issue\":{},\"worker_id\":\"{}\",\"state\":\"merged\",\"branch\":\"{}\",\"pr\":\"{}\",\"finalized_at\":\"{}\"}}\n{RUN_TERMINAL_END_MARKER}",
        json_escape(identity.repo),
        identity.issue,
        json_escape(identity.worker_id),
        json_escape(identity.branch),
        pull_request.expect("validated merged pull request"),
        json_escape(&record.updated_at),
    );
    if !list_comments(identity.repo, identity.issue)?
        .iter()
        .any(|comment| comment.body == body)
    {
        create_comment(identity.repo, identity.issue, &body)?;
    }
    Ok(())
}

pub(crate) fn record_executor_result(
    identity: ClaimMutationIdentity<'_>,
    outcome: &ConductorOutcome,
    pull_request: Option<u64>,
    success: Option<ExecutorSuccessBinding<'_>>,
) -> Result<ExecutorResultRecord, CommandFailure> {
    record_executor_result_with_receipt(identity, outcome, pull_request, success, None)
}

pub(crate) fn record_executor_result_with_receipt(
    identity: ClaimMutationIdentity<'_>,
    outcome: &ConductorOutcome,
    pull_request: Option<u64>,
    success: Option<ExecutorSuccessBinding<'_>>,
    stable_receipt_id: Option<&str>,
) -> Result<ExecutorResultRecord, CommandFailure> {
    let ClaimMutationIdentity {
        repo,
        issue,
        worker_id,
        branch,
        claim_id: expected_claim_id,
    } = identity;
    let Some(selected) = read_claim_ref(repo, issue)? else {
        return Ok(ExecutorResultRecord::OwnershipLost);
    };
    if !has_active_executor_claim(&selected.record, worker_id, branch)
        || selected.record.claim_id.as_deref() != Some(expected_claim_id)
    {
        return Ok(ExecutorResultRecord::OwnershipLost);
    }
    if matches!(outcome, ConductorOutcome::Succeeded)
        && success.as_ref().map(|binding| binding.claim_id) != Some(expected_claim_id)
    {
        return Ok(ExecutorResultRecord::OwnershipLost);
    }

    let pull_request = match outcome {
        ConductorOutcome::Succeeded => {
            let Some(success) = success.as_ref() else {
                return Ok(ExecutorResultRecord::EvidenceUnavailable);
            };
            let Some(pull_request) = pull_request else {
                return Ok(ExecutorResultRecord::EvidenceUnavailable);
            };
            let pull_requests = list_open_pull_requests(repo)?;
            if !pull_requests.iter().any(|candidate| {
                candidate.number == pull_request
                    && is_executor_result_pull_request(candidate, issue, branch)
                    && candidate.head_ref_oid == success.commit
            }) {
                return Ok(ExecutorResultRecord::EvidenceUnavailable);
            }
            Some(pull_request)
        }
        ConductorOutcome::Blocked(_)
        | ConductorOutcome::AllBlocked { .. }
        | ConductorOutcome::VerifierUnavailable { .. }
        | ConductorOutcome::ResourcePark { .. }
        | ConductorOutcome::OperatorStop { .. }
        | ConductorOutcome::Retryable(_) => {
            if pull_request.is_some() {
                return Ok(ExecutorResultRecord::EvidenceUnavailable);
            }
            None
        }
    };

    let receipt_id = match stable_receipt_id {
        Some(receipt_id) if !receipt_id.trim().is_empty() => receipt_id.to_string(),
        Some(_) => {
            return Err(CommandFailure::diagnostic(
                "executor result stable receipt ID must not be empty",
            ))
        }
        None => executor_result_receipt_id()?,
    };
    let evidence = ExecutorResultEvidence::new(
        repo,
        issue,
        worker_id,
        branch,
        executor_result_outcome_name(outcome),
        pull_request,
        executor_result_step(outcome),
        receipt_id,
        Some(expected_claim_id.to_string()),
        success.as_ref().map(|binding| binding.commit.to_string()),
        success
            .as_ref()
            .map(|binding| binding.premerge_receipt.to_string()),
    );
    let existing_comments = list_comments(repo, issue)?;
    let mut renewed = selected.record.clone();
    renewed.updated_at = utc_now_iso()?;
    renewed.step = executor_result_step(outcome).to_string();
    match advance_claim_ref(repo, issue, Some(&selected), &renewed)? {
        ClaimRefAdvance::Won(_) => {}
        ClaimRefAdvance::Lost => return Ok(ExecutorResultRecord::OwnershipLost),
    }
    if !executor_result_evidence_exists(&existing_comments, &evidence) {
        create_comment(repo, issue, &evidence.to_marked_comment())?;
    }

    let confirmed_comments = list_comments(repo, issue)?;
    if !executor_result_evidence_exists(&confirmed_comments, &evidence) {
        return Err(CommandFailure::diagnostic(
            "executor result evidence was not persisted",
        ));
    }
    let Some(confirmed) = read_claim_ref(repo, issue)? else {
        return Ok(ExecutorResultRecord::OwnershipLost);
    };
    if !has_active_executor_claim(&confirmed.record, worker_id, branch)
        || confirmed.record.claim_id.as_deref() != Some(expected_claim_id)
    {
        return Ok(ExecutorResultRecord::OwnershipLost);
    }
    if matches!(outcome, ConductorOutcome::Succeeded)
        && confirmed.record.claim_id.as_deref() != success.as_ref().map(|binding| binding.claim_id)
    {
        return Ok(ExecutorResultRecord::OwnershipLost);
    }
    Ok(ExecutorResultRecord::Recorded)
}

pub(crate) fn authoritative_executor_result(
    identity: ClaimMutationIdentity<'_>,
    binding: ExecutorResultAuthorityBinding<'_>,
) -> Result<Option<ExecutorResultEvidence>, CommandFailure> {
    let Some(selected) = read_claim_ref(identity.repo, identity.issue)? else {
        return Ok(None);
    };
    if !has_exact_claim_generation(
        &selected.record,
        identity.worker_id,
        identity.claim_id,
        identity.branch,
    ) {
        return Ok(None);
    }
    Ok(exact_successful_executor_result(
        &list_comments(identity.repo, identity.issue)?,
        identity,
        binding,
    ))
}

fn exact_successful_executor_result(
    comments: &[RemoteComment],
    identity: ClaimMutationIdentity<'_>,
    binding: ExecutorResultAuthorityBinding<'_>,
) -> Option<ExecutorResultEvidence> {
    let mut matching = comments
        .iter()
        .filter_map(|comment| {
            successful_executor_result_for_pull_request(
                std::slice::from_ref(comment),
                binding.pull_request,
            )
        })
        .filter(|evidence| {
            evidence.repo == identity.repo
                && evidence.issue == identity.issue
                && evidence.worker_id == identity.worker_id
                && evidence.branch == identity.branch
                && evidence.step == "executor_succeeded"
                && evidence.claim_id.as_deref() == Some(identity.claim_id)
                && evidence.commit.as_deref() == Some(binding.commit)
                && evidence.premerge_receipt.as_deref() == Some(binding.premerge_receipt)
                && evidence.receipt_id == binding.receipt_id
        })
        .collect::<Vec<_>>();
    matching.dedup();
    if matching.len() == 1 {
        matching.pop()
    } else {
        None
    }
}

#[allow(dead_code)]
pub(crate) fn record_executor_outcome(
    identity: ClaimMutationIdentity<'_>,
    outcome: &str,
) -> Result<(), CommandFailure> {
    if outcome.trim().is_empty() {
        return Err(CommandFailure::diagnostic(
            "executor outcome must not be empty",
        ));
    }
    match record_executor_result_with_step(
        identity,
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

pub(crate) fn recover_implementer_wait_failure(
    repo: &str,
    issue: u64,
    worker_id: &str,
    branch: &str,
    expected_claim_id: &str,
    session_id: &str,
    diagnostic: &str,
) -> Result<WaitFailureRecovery, CommandFailure> {
    let Some(selected) = read_claim_ref(repo, issue)? else {
        return Ok(WaitFailureRecovery::OwnershipLost);
    };
    let already_transitioned = selected.record.worker_id == worker_id
        && selected.record.branch == branch
        && selected.record.claim_id.as_deref() == Some(expected_claim_id)
        && selected.record.state == "available"
        && selected.record.step == "implementer_wait_failed";
    if selected.record.claim_id.as_deref() != Some(expected_claim_id) {
        return Ok(WaitFailureRecovery::OwnershipLost);
    }
    if !already_transitioned && !has_active_executor_claim(&selected.record, worker_id, branch) {
        return Ok(WaitFailureRecovery::OwnershipLost);
    }

    let head = if already_transitioned {
        selected
    } else {
        let mut available = selected.record.clone();
        available.state = "available".to_string();
        available.step = "implementer_wait_failed".to_string();
        available.updated_at = utc_now_iso()?;
        match advance_claim_ref(repo, issue, Some(&selected), &available)? {
            ClaimRefAdvance::Won(head) => {
                wait_recovery_failpoint("after_claim_cas")?;
                *head
            }
            ClaimRefAdvance::Lost => return Ok(WaitFailureRecovery::OwnershipLost),
        }
    };
    ensure_claim_ref_projection(repo, &head)?;
    wait_recovery_failpoint("after_ref_projection")?;

    let comments = list_comments(repo, issue)?;
    let evidence = wait_failure_evidence(
        repo,
        issue,
        worker_id,
        branch,
        expected_claim_id,
        session_id,
    );
    let already = executor_result_evidence_exists(&comments, &evidence);
    if !already {
        create_comment(repo, issue, &evidence.to_marked_comment())?;
    }
    let confirmed_comments = list_comments(repo, issue)?;
    if !executor_result_evidence_exists(&confirmed_comments, &evidence) {
        return Err(CommandFailure::diagnostic(
            "implementer wait-failure evidence was not persisted",
        ));
    }
    wait_recovery_failpoint("after_evidence_projection")?;

    requeue_wait_failure(repo, issue)?;
    wait_recovery_failpoint("after_label_requeue")?;
    upsert_wait_failure_comment(repo, issue, worker_id, branch, session_id, diagnostic)?;
    Ok(if already_transitioned || already {
        WaitFailureRecovery::AlreadyRecovered
    } else {
        WaitFailureRecovery::Requeued
    })
}

fn ensure_claim_ref_projection(repo: &str, head: &ClaimRefHead) -> Result<(), CommandFailure> {
    let comments = list_comments(repo, head.record.issue)?;
    if comments.iter().any(|comment| {
        run_state_link(&comment.body).is_some_and(|link| link.generation == head.generation)
            && parse_run_state_comment(&comment.body).is_ok_and(|record| record == head.record)
    }) {
        return Ok(());
    }
    let parent = authoritative_run_state_comment(&comments);
    create_comment(
        repo,
        head.record.issue,
        &linked_run_state_comment(&head.record, parent, &head.generation),
    )
}

fn wait_recovery_failpoint(point: &str) -> Result<(), CommandFailure> {
    if std::env::var("AUTOSPEC_WAIT_RECOVERY_FAILPOINT").as_deref() == Ok(point) {
        return Err(CommandFailure::diagnostic(format!(
            "implementer wait-failure recovery stopped at failpoint {point}"
        )));
    }
    Ok(())
}

fn wait_failure_evidence(
    repo: &str,
    issue: u64,
    worker_id: &str,
    branch: &str,
    claim_id: &str,
    session_id: &str,
) -> ExecutorResultEvidence {
    ExecutorResultEvidence::new(
        repo,
        issue,
        worker_id,
        branch,
        "failed",
        None,
        "implementer_wait_failed",
        format!("implementer-wait-failed:{claim_id}:{session_id}"),
        Some(claim_id.to_string()),
        None,
        None,
    )
}

fn requeue_wait_failure(repo: &str, issue: u64) -> Result<(), CommandFailure> {
    run_gh_with_retry(
        &[
            "issue".into(),
            "edit".into(),
            issue.to_string(),
            "--repo".into(),
            repo.into(),
            "--remove-label".into(),
            "in-progress-by-bot".into(),
            "--add-label".into(),
            "auto-implement".into(),
        ],
        "requeue implementer wait failure",
    )
}

fn upsert_wait_failure_comment(
    repo: &str,
    issue: u64,
    worker_id: &str,
    branch: &str,
    session_id: &str,
    diagnostic: &str,
) -> Result<(), CommandFailure> {
    let comments = list_comments(repo, issue)?;
    let body = format!("{WAIT_FAILURE_MARKER}\nAutospec requeued issue #{issue} after `implementer_wait_failed`. Session: `{}`. Worker: `{}`. Branch: `{}`. Diagnostic: `{}`. Recovery: start a fresh implementer from the restored `auto-implement` queue.", comment_field(session_id), comment_field(worker_id), comment_field(branch), safe_wait_diagnostic(diagnostic));
    if let Some(comment) = comments
        .iter()
        .find(|comment| comment.body.contains(WAIT_FAILURE_MARKER))
    {
        patch_comment(repo, comment.id, &body)?;
    } else {
        create_comment(repo, issue, &body)?;
    }
    Ok(())
}

fn comment_field(value: &str) -> String {
    value.replace(['`', '\n', '\r'], " ")
}

fn safe_wait_diagnostic(value: &str) -> String {
    redact_secrets(value).chars().take(512).collect()
}

#[allow(dead_code)]
fn record_executor_result_with_step(
    identity: ClaimMutationIdentity<'_>,
    outcome: &ConductorOutcome,
    pull_request: Option<u64>,
    step: &str,
) -> Result<ExecutorResultRecord, CommandFailure> {
    let ClaimMutationIdentity {
        repo,
        issue,
        worker_id,
        branch,
        claim_id: expected_claim_id,
    } = identity;
    let comments = list_comments(repo, issue)?;
    let Some(selected) = read_claim_ref(repo, issue)? else {
        return Ok(ExecutorResultRecord::OwnershipLost);
    };
    if !has_exact_claim_generation(&selected.record, worker_id, expected_claim_id, branch) {
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
        ConductorOutcome::Blocked(_)
        | ConductorOutcome::AllBlocked { .. }
        | ConductorOutcome::VerifierUnavailable { .. }
        | ConductorOutcome::ResourcePark { .. }
        | ConductorOutcome::OperatorStop { .. }
        | ConductorOutcome::Retryable(_) => {
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
    upsert_record(repo, &comments, expected_claim_id, &record)?;
    let Some(confirmed) = read_claim_ref(repo, issue)? else {
        return Ok(ExecutorResultRecord::OwnershipLost);
    };
    if !has_exact_claim_generation(&confirmed.record, worker_id, expected_claim_id, branch)
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

fn has_active_executor_claim(record: &RunStateRecord, worker_id: &str, branch: &str) -> bool {
    has_executor_claim_owner(record, worker_id, branch)
        && server_lease_is_fresh(&record.updated_at, record.ttl_seconds)
}

fn executor_result_step(outcome: &ConductorOutcome) -> &'static str {
    match outcome {
        ConductorOutcome::Succeeded => "executor_succeeded",
        ConductorOutcome::Blocked(_)
        | ConductorOutcome::AllBlocked { .. }
        | ConductorOutcome::VerifierUnavailable { .. }
        | ConductorOutcome::ResourcePark { .. }
        | ConductorOutcome::OperatorStop { .. } => "executor_blocked",
        ConductorOutcome::Retryable(_) => "executor_retryable",
    }
}

fn executor_result_outcome_name(outcome: &ConductorOutcome) -> &'static str {
    match outcome {
        ConductorOutcome::Succeeded => "succeeded",
        ConductorOutcome::Blocked(_)
        | ConductorOutcome::AllBlocked { .. }
        | ConductorOutcome::VerifierUnavailable { .. }
        | ConductorOutcome::ResourcePark { .. }
        | ConductorOutcome::OperatorStop { .. } => "blocked",
        ConductorOutcome::Retryable(_) => "retryable",
    }
}

fn executor_result_receipt_id() -> Result<String, CommandFailure> {
    unique_operation_id("executor-result")
}

fn claim_generation_id() -> Result<String, CommandFailure> {
    unique_operation_id("claim")
}

fn unique_operation_id(prefix: &str) -> Result<String, CommandFailure> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| {
            CommandFailure::diagnostic(format!("cannot generate unique operation id: {error}"))
        })?
        .as_nanos();
    let sequence = UNIQUE_ID_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    Ok(format!(
        "{prefix}-{}-{nanos}-{sequence}",
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
    worker_id: &str,
    branch: &str,
    expected_claim_id: &str,
) -> Result<ReconcileOutcome, CommandFailure> {
    let comments = list_comments(repo, issue)?;
    let Some(selected) = read_claim_ref(repo, issue)? else {
        return Ok(ReconcileOutcome {
            reconciled: false,
            pr: None,
            reason: "missing_run_state".to_string(),
        });
    };
    if !has_exact_claim_generation(&selected.record, worker_id, expected_claim_id, branch) {
        return Ok(ReconcileOutcome {
            reconciled: false,
            pr: None,
            reason: "identity_mismatch".to_string(),
        });
    }
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
    let Some(evidence) =
        successful_executor_result_for_pull_request(&comments, pull_request.number)
    else {
        return Ok(ReconcileOutcome {
            reconciled: false,
            pr: Some(pull_request.number.to_string()),
            reason: "missing_exact_executor_result".to_string(),
        });
    };
    let required_checks = list_required_checks(repo, pull_request.number)?;
    let lease_is_live =
        server_lease_is_fresh(&selected.record.updated_at, selected.record.ttl_seconds);
    match evaluate_merge_ready_claim_recovery(
        &selected.record,
        &evidence,
        pull_request,
        &required_checks,
        lease_is_live,
    ) {
        ClaimRecoveryDecision::Recover { .. } => {}
        ClaimRecoveryDecision::Blocked(reason) => {
            return Ok(ReconcileOutcome {
                reconciled: false,
                pr: Some(pull_request.number.to_string()),
                reason: reason.as_str().to_string(),
            });
        }
    }
    let mut record = selected.record;
    record.state = "claimed".to_string();
    record.step = "post_pr_handoff_failed".to_string();
    record.pr = pull_request.number.to_string();
    record.updated_at = utc_now_iso()?;
    upsert_record(repo, &comments, expected_claim_id, &record)?;

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
struct ClearOptions {
    issue: u64,
    repo: Option<String>,
    worker_id: String,
    claim_id: String,
    branch: String,
}

#[derive(Debug, PartialEq, Eq)]
struct UpsertOptions {
    issue: u64,
    repo: Option<String>,
    worker_id: String,
    claim_id: String,
    state: String,
    step: Option<String>,
    branch: String,
    pr: String,
    paths: Vec<String>,
    ttl_seconds: u64,
}

#[derive(Debug, PartialEq, Eq)]
struct RefreshOptions {
    issue: u64,
    repo: Option<String>,
    worker_id: String,
    claim_id: String,
    branch: String,
    step: String,
    pr: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
struct ReconcileOptions {
    issue: u64,
    repo: Option<String>,
    worker_id: String,
    claim_id: String,
    branch: String,
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
    claim_id: String,
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
    session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ClaimLease {
    pub issue: u64,
    pub repo: String,
    pub worker_id: String,
    pub branch: String,
    pub claim_id: String,
    pub session_id: Option<String>,
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

fn parse_clear_options(args: &[String]) -> Result<ClearOptions, CommandFailure> {
    let mut issue = None;
    let mut repo = None;
    let mut worker_id = None;
    let mut claim_id = None;
    let mut branch = None;
    let mut index = 0;
    while index < args.len() {
        let option = args[index].as_str();
        let value = match option {
            "--issue" | "--repo" | "--worker-id" | "--claim-id" | "--branch" => {
                argument_value(args, &mut index, option)?
            }
            _ => {
                return Err(CommandFailure::diagnostic(format!(
                    "unknown autospec claim state clear option: {option}"
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
            "--claim-id" => set_once(
                &mut claim_id,
                value,
                "--claim-id accepts exactly one claim generation",
            )?,
            "--branch" => set_once(&mut branch, value, "--branch accepts exactly one branch")?,
            _ => unreachable!("options were matched before parsing"),
        }
        index += 1;
    }
    Ok(ClearOptions {
        issue: issue.ok_or_else(|| CommandFailure::diagnostic("--issue is required"))?,
        repo,
        worker_id: worker_id
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| CommandFailure::diagnostic("--worker-id is required"))?,
        claim_id: claim_id
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| CommandFailure::diagnostic("--claim-id is required"))?,
        branch: branch
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| CommandFailure::diagnostic("--branch is required"))?,
    })
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
    let mut claim_id = None;
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
            "--issue" | "--repo" | "--worker-id" | "--claim-id" | "--state" | "--step"
            | "--branch" | "--pr" | "--paths" | "--ttl-seconds" => {
                argument_value(args, &mut index, option)?
            }
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
            "--claim-id" => set_once(
                &mut claim_id,
                value,
                "--claim-id accepts exactly one claim generation",
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
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| CommandFailure::diagnostic("--worker-id is required"))?,
        claim_id: claim_id
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| CommandFailure::diagnostic("--claim-id is required"))?,
        state: state.ok_or_else(|| CommandFailure::diagnostic("--state is required"))?,
        step,
        branch: (!branch.trim().is_empty())
            .then_some(branch)
            .ok_or_else(|| CommandFailure::diagnostic("--branch is required"))?,
        pr,
        paths,
        ttl_seconds,
    })
}

fn parse_refresh_options(args: &[String]) -> Result<RefreshOptions, CommandFailure> {
    let mut issue = None;
    let mut repo = None;
    let mut worker_id = None;
    let mut claim_id = None;
    let mut branch = None;
    let mut step = None;
    let mut pr = None;
    let mut index = 0;
    while index < args.len() {
        let option = args[index].as_str();
        let value = match option {
            "--issue" | "--repo" | "--worker-id" | "--claim-id" | "--branch" | "--step"
            | "--pr" => argument_value(args, &mut index, option)?,
            _ => {
                return Err(CommandFailure::diagnostic(format!(
                    "unknown autospec claim state refresh option: {option}"
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
            "--claim-id" => set_once(
                &mut claim_id,
                value,
                "--claim-id accepts exactly one claim generation",
            )?,
            "--branch" => set_once(&mut branch, value, "--branch accepts exactly one branch")?,
            "--step" => set_once(&mut step, value, "--step accepts exactly one step")?,
            "--pr" => set_once(&mut pr, value, "--pr accepts exactly one pull request")?,
            _ => unreachable!("options were matched before parsing"),
        }
        index += 1;
    }
    Ok(RefreshOptions {
        issue: issue.ok_or_else(|| CommandFailure::diagnostic("--issue is required"))?,
        repo,
        worker_id: worker_id
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| CommandFailure::diagnostic("--worker-id is required"))?,
        claim_id: claim_id
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| CommandFailure::diagnostic("--claim-id is required"))?,
        branch: branch
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| CommandFailure::diagnostic("--branch is required"))?,
        step: step
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| CommandFailure::diagnostic("--step is required"))?,
        pr,
    })
}

fn parse_reconcile_options(args: &[String]) -> Result<ReconcileOptions, CommandFailure> {
    let mut issue = None;
    let mut repo = None;
    let mut worker_id = None;
    let mut claim_id = None;
    let mut branch = None;
    let mut index = 0;
    while index < args.len() {
        let option = args[index].as_str();
        let value = match option {
            "--issue" | "--repo" | "--worker-id" | "--claim-id" | "--branch" => {
                argument_value(args, &mut index, option)?
            }
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
            "--claim-id" => set_once(
                &mut claim_id,
                value,
                "--claim-id accepts exactly one claim generation",
            )?,
            "--branch" => set_once(&mut branch, value, "--branch accepts exactly one branch")?,
            _ => unreachable!("options were matched before parsing"),
        }
        index += 1;
    }
    Ok(ReconcileOptions {
        issue: issue.ok_or_else(|| CommandFailure::diagnostic("--issue is required"))?,
        repo,
        worker_id: worker_id
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| CommandFailure::diagnostic("--worker-id is required"))?,
        claim_id: claim_id
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| CommandFailure::diagnostic("--claim-id is required"))?,
        branch: branch
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| CommandFailure::diagnostic("--branch is required"))?,
    })
}

fn parse_release_options(args: &[String]) -> Result<ReleaseOptions, CommandFailure> {
    let mut issue = None;
    let mut repo = None;
    let mut worker_id = None;
    let mut claim_id = None;
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
            "--issue" | "--repo" | "--worker-id" | "--claim-id" | "--state" | "--branch"
            | "--pr" => argument_value(args, &mut index, option)?,
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
            "--claim-id" => set_once(
                &mut claim_id,
                value,
                "--claim-id accepts exactly one claim generation",
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
        claim_id: claim_id.ok_or_else(|| CommandFailure::diagnostic("--claim-id is required"))?,
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
    let mut session_id = None;
    let mut index = 0;
    while index < args.len() {
        let option = args[index].as_str();
        let value = match option {
            "--issue" | "--repo" | "--worker-id" | "--branch" | "--session-id" => {
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
            "--session-id" => set_once(
                &mut session_id,
                value,
                "--session-id accepts exactly one session identifier",
            )?,
            _ => unreachable!("options were matched before parsing"),
        }
        index += 1;
    }
    Ok(AcquireOptions {
        issue: issue.ok_or_else(|| CommandFailure::diagnostic("--issue is required"))?,
        repo,
        worker_id,
        branch,
        session_id,
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
    let Ok(output) = lease::read_gh_with_retry(
        &[
            "repo",
            "view",
            "--json",
            "nameWithOwner",
            "--jq",
            ".nameWithOwner",
        ],
        "infer the repository",
    ) else {
        return Err(CommandFailure::diagnostic(
            "--repo is required when gh cannot infer it",
        ));
    };
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
    let output = lease::read_gh_with_retry(
        &["api", endpoint.as_str(), "--paginate", "--slurp"],
        "read issue comments",
    )?;
    parse_paginated_comments_json(&String::from_utf8_lossy(&output.stdout)).map_err(|error| {
        CommandFailure::diagnostic(format!("could not parse GitHub issue comments: {error}"))
    })
}

/// Flatten raw `gh api --paginate --slurp` pages into the typed comment boundary.
///
/// GitHub owns the raw object shape and may add unrelated fields, so this parser
/// intentionally projects only `id`, `body`, and `updated_at`. The downstream
/// typed value remains exact without coupling Autospec to every GitHub field.
pub(crate) fn parse_paginated_comments_json(input: &str) -> Result<Vec<RemoteComment>, String> {
    let value: serde_json::Value = serde_json::from_str(input)
        .map_err(|error| format!("GitHub issue comments are not valid JSON: {error}"))?;
    let pages = value
        .as_array()
        .ok_or_else(|| "GitHub issue comments are not an array".to_string())?;
    let nested = pages.iter().any(serde_json::Value::is_array);
    if nested && !pages.iter().all(serde_json::Value::is_array) {
        return Err("GitHub issue comment pages mix arrays with comments".to_string());
    }
    let values = if nested {
        pages
            .iter()
            .flat_map(|page| page.as_array().into_iter().flatten())
            .collect::<Vec<_>>()
    } else {
        pages.iter().collect::<Vec<_>>()
    };
    values
        .into_iter()
        .enumerate()
        .map(|(index, value)| {
            let context = format!("GitHub issue comments[{index}]");
            let object = value
                .as_object()
                .ok_or_else(|| format!("{context} is not an object"))?;
            let id = object
                .get("id")
                .and_then(serde_json::Value::as_u64)
                .ok_or_else(|| format!("{context} id is not an unsigned integer"))?;
            let optional_string = |field: &str| match object.get(field) {
                None | Some(serde_json::Value::Null) => Ok(String::new()),
                Some(serde_json::Value::String(value)) => Ok(value.clone()),
                Some(_) => Err(format!("{context} {field} is not a string or null")),
            };
            Ok(RemoteComment::new(
                id,
                optional_string("body")?,
                optional_string("updated_at")?,
            ))
        })
        .collect()
}

fn load_claim_issue(
    repo: &str,
    issue: u64,
) -> Result<autospec_core::claim::ClaimIssueSnapshot, CommandFailure> {
    let issue = issue.to_string();
    let output = lease::read_gh_with_retry(
        &[
            "issue",
            "view",
            issue.as_str(),
            "--repo",
            repo,
            "--json",
            "labels,body,title,author",
            "--jq",
            "{labels:[.labels[].name],body:(.body // \"\"),title:(.title // \"\"),author:(.author.login // \"\")}",
        ],
        "read a claim issue",
    )?;
    parse_claim_issue_json(&String::from_utf8_lossy(&output.stdout)).map_err(|error| {
        CommandFailure::diagnostic(format!("could not parse GitHub claim issue: {error}"))
    })
}

fn list_open_pull_requests(
    repo: &str,
) -> Result<Vec<autospec_core::claim::OpenPullRequest>, CommandFailure> {
    let output = lease::read_gh_with_retry(
        &[
            "pr",
            "list",
            "--repo",
            repo,
            "--state",
            "open",
            "--limit",
            "100",
            "--json",
            "number,body,headRefName,headRefOid,isDraft,baseRefName",
        ],
        "list open pull requests",
    )?;
    parse_open_pull_requests_json(&String::from_utf8_lossy(&output.stdout)).map_err(|error| {
        CommandFailure::diagnostic(format!(
            "could not parse GitHub open pull requests: {error}"
        ))
    })
}

fn list_required_checks(
    repo: &str,
    pull_request: u64,
) -> Result<Vec<autospec_core::claim::RequiredCheck>, CommandFailure> {
    let pull_request = pull_request.to_string();
    let output = lease::read_gh_with_retry(
        &[
            "pr",
            "checks",
            pull_request.as_str(),
            "--repo",
            repo,
            "--required",
            "--json",
            "name,state",
        ],
        "read required checks",
    )?;
    if output.stdout.is_empty() {
        return Err(CommandFailure::transient(format!(
            "gh pr checks returned no required-check evidence: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    parse_required_checks_json(&String::from_utf8_lossy(&output.stdout)).map_err(|error| {
        CommandFailure::diagnostic(format!("could not parse GitHub required checks: {error}"))
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
    let attempts = claim_retry_attempts();
    let sleep_ms = claim_retry_sleep_ms();
    for attempt in 0..attempts {
        let output = Command::new("gh")
            .args(arguments)
            .output()
            .map_err(|error| CommandFailure::transient(format!("could not {action}: {error}")))?;
        if output.status.success() {
            return Ok(());
        }
        if attempt + 1 < attempts {
            std::thread::sleep(std::time::Duration::from_millis(sleep_ms));
            continue;
        }
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(CommandFailure::transient(if detail.is_empty() {
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

fn write_startup_heartbeat(
    repo: &str,
    issue: u64,
    worker_id: &str,
    branch: &str,
    claim_id: &str,
    session_id: Option<&str>,
) -> Result<(), CommandFailure> {
    let _root = heartbeat_root()?;
    let timestamp = unix_now()?;
    let ttl_seconds = claim_ttl_seconds().max(1);
    let pid = std::process::id();
    let nonce = startup_heartbeat_nonce(repo, issue, claim_id);
    let (host, boot_id, process_start) = startup_process_identity(pid)?;
    let session_field = session_id.map_or_else(String::new, |session_id| {
        format!(",\"session_id\":\"{}\"", json_escape(session_id))
    });
    let _body = format!(
        "{{\"issue\":\"{issue}\",\"branch\":\"{}\",\"step\":\"claimed\",\"ts\":{timestamp},\"ttl_seconds\":{ttl_seconds},\"pid\":{pid},\"nonce\":\"{}\",\"host\":\"{}\",\"boot_id\":\"{}\",\"process_start\":\"{}\",\"pr\":\"\",\"repo\":\"{}\",\"worker_id\":\"{}\",\"claim_id\":\"{}\"{session_field}}}\n",
        json_escape(branch),
        json_escape(&nonce),
        json_escape(&host),
        json_escape(&boot_id),
        json_escape(&process_start),
        json_escape(repo),
        json_escape(worker_id),
        json_escape(claim_id),
    );
    #[cfg(target_os = "linux")]
    return publish_startup_heartbeat_transaction_with_hook(
        &_root,
        repo,
        issue,
        session_id,
        _body.as_bytes(),
        &mut |_, _| Ok(()),
    );
    #[cfg(not(target_os = "linux"))]
    heartbeat_portable::publish(&_root, repo, issue, session_id, _body.as_bytes())
}

#[cfg(test)]
pub(crate) fn write_startup_heartbeat_for_test(
    repo: &str,
    issue: u64,
    worker_id: &str,
    branch: &str,
    claim_id: &str,
    session_id: Option<&str>,
) -> Result<(), CommandFailure> {
    write_startup_heartbeat(repo, issue, worker_id, branch, claim_id, session_id)
}

#[cfg(target_os = "linux")]
fn open_private_heartbeat_directory(path: &Path) -> Result<fs::File, CommandFailure> {
    use nix::fcntl::{open, OFlag};
    use nix::sys::stat::Mode;

    prepare_heartbeat_root_parent_with_hook(&path.join(".publication-anchor"), |_| Ok(()))?;
    let directory = fs::File::from(
        open(
            path,
            OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
            Mode::empty(),
        )
        .map_err(|error| CommandFailure::diagnostic(format!("heartbeat dir open: {error}")))?,
    );
    private_heartbeat_directory_identity(&directory, "publication directory")?;
    Ok(directory)
}

#[cfg(target_os = "linux")]
fn inspect_heartbeat_target(
    directory: &fs::File,
    name: &str,
    expected: &[u8],
) -> Result<Option<HeartbeatPublication>, CommandFailure> {
    use nix::fcntl::{openat, OFlag};
    use nix::sys::stat::{fchmod, fstat, Mode, SFlag};

    let blocking = || CommandFailure::diagnostic("heartbeat publication target conflicts");
    let descriptor = match openat(
        directory,
        name,
        OFlag::O_RDWR | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC | OFlag::O_NONBLOCK,
        Mode::empty(),
    ) {
        Err(nix::errno::Errno::ENOENT) => return Ok(None),
        Ok(descriptor) => descriptor,
        Err(_) => return Err(blocking()),
    };
    let file = fs::File::from(descriptor);
    let stat = fstat(&file).map_err(|_| blocking())?;
    if SFlag::from_bits_truncate(stat.st_mode) & SFlag::S_IFMT != SFlag::S_IFREG
        || stat.st_uid != nix::unistd::geteuid().as_raw()
        || stat.st_nlink != 1
    {
        return Err(blocking());
    }
    let snapshot = file
        .try_clone()
        .and_then(read_regular_file)
        .map_err(|_| blocking())?;
    if !same_startup_heartbeat_generation(&snapshot.document, expected) {
        return Err(blocking());
    }
    if stat.st_mode & 0o7777 != 0o600
        && (fchmod(&file, Mode::from_bits_truncate(0o600)).is_err() || file.sync_all().is_err())
    {
        return Err(blocking());
    }
    let current = fstat(&file).map_err(|_| blocking())?;
    if current.st_mode & 0o7777 != 0o600 {
        return Err(blocking());
    }
    if heartbeat_final_binding(&file, directory, name, (stat.st_dev, stat.st_ino)).ok()
        == Some((HeartbeatFinalBinding::Exact, 1))
    {
        Ok(Some(HeartbeatPublication {
            file,
            device: stat.st_dev,
            inode: stat.st_ino,
            durability: HeartbeatPublicationDurability::Unconfirmed,
        }))
    } else {
        Err(blocking())
    }
}

#[cfg(target_os = "linux")]
fn heartbeat_publication_error(error: HeartbeatPublicationFailure) -> CommandFailure {
    match error {
        HeartbeatPublicationFailure::PreCommit(error)
        | HeartbeatPublicationFailure::PostCommit { error, .. } => error,
    }
}

#[cfg(target_os = "linux")]
fn publish_startup_heartbeat_transaction_with_hook(
    root: &Path,
    repo: &str,
    issue: u64,
    session_id: Option<&str>,
    document: &[u8],
    boundary: &mut impl FnMut(&str, &str) -> Result<(), CommandFailure>,
) -> Result<(), CommandFailure> {
    let root_directory = open_private_heartbeat_directory(root)?;
    let repo_name = super::autonomous::drain::repository_progress_key(repo);
    let repo_path = root.join(&repo_name);
    drop(open_private_heartbeat_directory(&repo_path)?);
    let repo = open_heartbeat_directory_beneath(&root_directory, Path::new(&repo_name))?;
    let issue_name = format!("{issue}.json");
    let mut issue_guard = inspect_heartbeat_target(&repo, &issue_name, document)?;
    let sessions = session_id
        .map(|_| {
            drop(open_private_heartbeat_directory(
                &repo_path.join("sessions"),
            )?);
            open_heartbeat_directory_beneath(&repo, Path::new("sessions"))
        })
        .transpose()?;
    let session_name = session_id.map(|value| format!("{}.json", heartbeat_session_key(value)));
    let mut session_guard = match (&sessions, &session_name) {
        (Some(directory), Some(name)) => inspect_heartbeat_target(directory, name, document)?,
        _ => None,
    };
    let issue_file = issue_guard
        .is_none()
        .then(|| prepare_private_heartbeat_file(&repo, document, "issue", boundary))
        .transpose()
        .map_err(heartbeat_publication_error)?;
    let session_file = (session_guard.is_none() && sessions.is_some())
        .then(|| {
            prepare_private_heartbeat_file(
                sessions.as_ref().expect("session directory"),
                document,
                "session",
                boundary,
            )
        })
        .transpose()
        .map_err(heartbeat_publication_error)?;
    if let Some(prepared) = issue_file {
        issue_guard = Some(
            publish_prepared_heartbeat_file(&repo, &issue_name, prepared, "issue", boundary)
                .map_err(heartbeat_publication_error)?,
        );
    }
    if let (Some(directory), Some(name), Some(prepared)) =
        (sessions.as_ref(), session_name.as_ref(), session_file)
    {
        session_guard = Some(
            publish_prepared_heartbeat_file(directory, name, prepared, "session", boundary)
                .map_err(heartbeat_publication_error)?,
        );
    }
    boundary("issue", "transaction-fsync")?;
    nix::unistd::fsync(&repo)
        .map_err(|error| CommandFailure::diagnostic(format!("heartbeat repo fsync: {error}")))?;
    if let Some(directory) = sessions.as_ref() {
        boundary("session", "transaction-fsync")?;
        nix::unistd::fsync(directory)
            .map_err(|error| CommandFailure::diagnostic(format!("session fsync: {error}")))?;
    }
    let exact = |directory: &fs::File, name: &str, guard: &HeartbeatPublication| {
        let metadata = guard.file.metadata().ok();
        let content = guard.file.try_clone().and_then(read_regular_file).ok();
        metadata.is_some_and(|value| {
            value.is_file()
                && value.uid() == nix::unistd::geteuid().as_raw()
                && value.mode() & 0o7777 == 0o600
        }) && content
            .is_some_and(|value| same_startup_heartbeat_generation(&value.document, document))
            && heartbeat_final_binding(&guard.file, directory, name, (guard.device, guard.inode))
                .ok()
                == Some((HeartbeatFinalBinding::Exact, 1))
    };
    if !exact(
        &repo,
        &issue_name,
        issue_guard.as_ref().expect("issue guard"),
    ) || sessions
        .as_ref()
        .zip(session_name.as_deref())
        .zip(session_guard.as_ref())
        .is_some_and(|((directory, name), guard)| !exact(directory, name, guard))
    {
        return Err(CommandFailure::diagnostic("heartbeat binding changed"));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn same_startup_heartbeat_generation(left: &[u8], right: &[u8]) -> bool {
    let normalize = |document: &[u8]| {
        parse_startup_heartbeat(document).map(|mut evidence| {
            evidence.ts = 0;
            evidence.pid = 0;
            evidence.process_start.clear();
            evidence
        })
    };
    normalize(left) == normalize(right)
}

fn startup_heartbeat_nonce(repo: &str, issue: u64, claim_id: &str) -> String {
    let mut identity = b"autospec-startup-heartbeat-nonce-v1".to_vec();
    for field in [repo, &issue.to_string(), claim_id] {
        identity.extend_from_slice(&(field.len() as u64).to_be_bytes());
        identity.extend_from_slice(field.as_bytes());
    }
    autospec_core::autonomous::waterfall::sha256_hex(&identity)
}

#[cfg(target_os = "linux")]
fn startup_process_identity(pid: u32) -> Result<(String, String, String), CommandFailure> {
    let host = fs::read_to_string("/proc/sys/kernel/hostname")
        .map_err(|error| CommandFailure::diagnostic(format!("read heartbeat host: {error}")))?
        .trim()
        .to_string();
    let (boot_id, process_start) = super::autonomous::process_birth_identity(pid)
        .map_err(|error| CommandFailure::diagnostic(format!("read heartbeat process: {error}")))?
        .ok_or_else(|| CommandFailure::diagnostic("heartbeat process identity disappeared"))?;
    if host.is_empty() || boot_id.is_empty() || process_start.is_empty() {
        return Err(CommandFailure::diagnostic(
            "heartbeat process identity is incomplete",
        ));
    }
    Ok((host, boot_id, process_start))
}

#[cfg(not(target_os = "linux"))]
fn startup_process_identity(pid: u32) -> Result<(String, String, String), CommandFailure> {
    heartbeat_portable::process_identity(pid)
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
struct SessionBindingIdentity {
    session_id: String,
    issue: String,
    worker_id: String,
    branch: String,
    claim_id: String,
}

#[allow(dead_code)]
impl SessionBindingIdentity {
    fn new(session_id: &str, issue: u64, worker_id: &str, branch: &str, claim_id: &str) -> Self {
        Self {
            session_id: session_id.to_string(),
            issue: issue.to_string(),
            worker_id: worker_id.to_string(),
            branch: branch.to_string(),
            claim_id: claim_id.to_string(),
        }
    }

    fn from_document(document: &[u8]) -> Result<Self, CommandFailure> {
        let value: serde_json::Value = serde_json::from_slice(document).map_err(|error| {
            CommandFailure::diagnostic(format!(
                "existing claim session binding is malformed: {error}"
            ))
        })?;
        let field = |name: &str| {
            value
                .get(name)
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .ok_or_else(|| {
                    CommandFailure::diagnostic(format!(
                        "existing claim session binding has invalid {name}"
                    ))
                })
        };
        Ok(Self {
            session_id: field("session_id")?,
            issue: field("issue")?,
            worker_id: field("worker_id")?,
            branch: value
                .get("branch")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| {
                    CommandFailure::diagnostic(
                        "existing claim session binding has invalid branch".to_string(),
                    )
                })?,
            claim_id: field("claim_id")?,
        })
    }
}

#[allow(dead_code)]
fn publish_session_binding(
    path: &Path,
    document: &[u8],
    expected: &SessionBindingIdentity,
) -> Result<bool, CommandFailure> {
    let mut file = match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let existing = fs::read(path).map_err(|read_error| {
                CommandFailure::diagnostic(format!(
                    "could not read existing claim session binding: {read_error}"
                ))
            })?;
            let observed = SessionBindingIdentity::from_document(&existing)?;
            if observed == *expected {
                return Ok(false);
            }
            return Err(CommandFailure::diagnostic(
                "claim session binding identity conflict".to_string(),
            ));
        }
        Err(error) => {
            return Err(CommandFailure::diagnostic(format!(
                "could not create claim session binding: {error}"
            )))
        }
    };
    if let Err(error) = file.write_all(document).and_then(|()| file.sync_all()) {
        return Err(CommandFailure::diagnostic(format!(
            "could not persist claim session binding: {error}"
        )));
    }
    Ok(true)
}

#[cfg(unix)]
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HeartbeatPublicationDurability {
    Unconfirmed,
    Durable,
}

#[cfg(unix)]
#[allow(dead_code)]
#[derive(Debug)]
struct HeartbeatPublication {
    file: fs::File,
    device: u64,
    inode: u64,
    durability: HeartbeatPublicationDurability,
}

#[cfg(target_os = "linux")]
#[derive(Debug)]
struct PreparedHeartbeat {
    file: fs::File,
    identity: (u64, u64),
}

#[cfg(unix)]
#[allow(dead_code)]
#[derive(Debug)]
enum HeartbeatPublicationFailure {
    PreCommit(CommandFailure),
    PostCommit {
        publication: HeartbeatPublication,
        error: CommandFailure,
    },
}

#[cfg(unix)]
fn validate_heartbeat_final_name(final_name: &str) -> Result<(), HeartbeatPublicationFailure> {
    use std::path::Component;

    let mut components = Path::new(final_name).components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(name)), None) if name == std::ffi::OsStr::new(final_name) => Ok(()),
        _ => Err(HeartbeatPublicationFailure::PreCommit(
            CommandFailure::diagnostic("heartbeat final name must be one normal component"),
        )),
    }
}

#[cfg(target_os = "linux")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HeartbeatFinalBinding {
    Missing,
    Exact,
    Other,
}

#[cfg(target_os = "linux")]
fn heartbeat_final_binding(
    file: &fs::File,
    directory: &impl std::os::fd::AsFd,
    final_name: &str,
    identity: (u64, u64),
) -> Result<(HeartbeatFinalBinding, u64), CommandFailure> {
    use nix::fcntl::AtFlags;
    use nix::sys::stat::{fstat, fstatat};

    let links = fstat(file)
        .map_err(|error| CommandFailure::diagnostic(format!("heartbeat fstat: {error}")))?
        .st_nlink;
    let binding = match fstatat(directory, final_name, AtFlags::AT_SYMLINK_NOFOLLOW) {
        Err(nix::errno::Errno::ENOENT) => HeartbeatFinalBinding::Missing,
        Ok(stat) if (stat.st_dev, stat.st_ino) == identity => HeartbeatFinalBinding::Exact,
        Ok(_) => HeartbeatFinalBinding::Other,
        Err(error) => {
            return Err(CommandFailure::diagnostic(format!(
                "could not inspect heartbeat final binding: {error}"
            )))
        }
    };
    Ok((binding, links))
}

#[cfg(target_os = "linux")]
fn post_commit_failure(
    file: fs::File,
    identity: (u64, u64),
    durability: HeartbeatPublicationDurability,
    error: CommandFailure,
) -> HeartbeatPublicationFailure {
    HeartbeatPublicationFailure::PostCommit {
        publication: HeartbeatPublication {
            file,
            device: identity.0,
            inode: identity.1,
            durability,
        },
        error,
    }
}

#[cfg(target_os = "linux")]
fn prepare_private_heartbeat_file(
    directory: &impl std::os::fd::AsFd,
    document: &[u8],
    role: &str,
    boundary: &mut impl FnMut(&str, &str) -> Result<(), CommandFailure>,
) -> Result<PreparedHeartbeat, HeartbeatPublicationFailure> {
    use nix::fcntl::{openat, OFlag};
    use nix::sys::stat::{fchmod, fstat, Mode};

    let pre = HeartbeatPublicationFailure::PreCommit;
    private_heartbeat_directory_identity(directory, "publication").map_err(pre)?;
    let descriptor = openat(
        directory,
        ".",
        OFlag::O_TMPFILE | OFlag::O_RDWR | OFlag::O_CLOEXEC,
        Mode::from_bits_truncate(0o600),
    )
    .map_err(|error| {
        pre(CommandFailure::diagnostic(format!(
            "anonymous heartbeat staging is unavailable: {error}"
        )))
    })?;
    let mut file = fs::File::from(descriptor);
    let fail = |message| pre(CommandFailure::diagnostic(message));
    boundary(role, "chmod").map_err(pre)?;
    fchmod(&file, Mode::from_bits_truncate(0o600))
        .map_err(|error| fail(format!("heartbeat chmod: {error}")))?;
    boundary(role, "write").map_err(pre)?;
    file.write_all(document)
        .map_err(|error| fail(format!("heartbeat write: {error}")))?;
    boundary(role, "file-fsync").map_err(pre)?;
    file.sync_all()
        .map_err(|error| fail(format!("heartbeat fsync: {error}")))?;
    let stat = fstat(&file).map_err(|error| fail(format!("heartbeat fstat: {error}")))?;
    boundary(role, "before-link").map_err(pre)?;
    Ok(PreparedHeartbeat {
        file,
        identity: (stat.st_dev as u64, stat.st_ino as u64),
    })
}

#[cfg(target_os = "linux")]
fn publish_prepared_heartbeat_file(
    directory: &impl std::os::fd::AsFd,
    final_name: &str,
    prepared: PreparedHeartbeat,
    role: &str,
    boundary: &mut impl FnMut(&str, &str) -> Result<(), CommandFailure>,
) -> Result<HeartbeatPublication, HeartbeatPublicationFailure> {
    use nix::fcntl::{AtFlags, AT_FDCWD};
    use nix::unistd::{fsync, linkat};
    use std::os::fd::AsRawFd;

    validate_heartbeat_final_name(final_name)?;
    let PreparedHeartbeat { file, identity } = prepared;
    let mut link_error = linkat(&file, "", directory, final_name, AtFlags::AT_EMPTY_PATH).err();
    let (mut binding, mut link_count) =
        match heartbeat_final_binding(&file, directory, final_name, identity) {
            Ok(state) => state,
            Err(error) => {
                return Err(post_commit_failure(
                    file,
                    identity,
                    HeartbeatPublicationDurability::Unconfirmed,
                    error,
                ))
            }
        };
    let unavailable = matches!(
        link_error,
        Some(
            nix::errno::Errno::EPERM
                | nix::errno::Errno::EINVAL
                | nix::errno::Errno::ENOENT
                | nix::errno::Errno::EOPNOTSUPP
        )
    );
    if unavailable && binding == HeartbeatFinalBinding::Missing && link_count == 0 {
        let proc_path = format!("/proc/self/fd/{}", file.as_raw_fd());
        link_error = linkat(
            AT_FDCWD,
            proc_path.as_str(),
            directory,
            final_name,
            AtFlags::AT_SYMLINK_FOLLOW,
        )
        .err();
        (binding, link_count) =
            match heartbeat_final_binding(&file, directory, final_name, identity) {
                Ok(state) => state,
                Err(error) => {
                    return Err(post_commit_failure(
                        file,
                        identity,
                        HeartbeatPublicationDurability::Unconfirmed,
                        error,
                    ))
                }
            };
    }
    if (binding, link_count) != (HeartbeatFinalBinding::Exact, 1) {
        let error = CommandFailure::diagnostic(format!(
            "heartbeat link failed or lost identity: {}",
            link_error.map_or_else(
                || "final binding changed".to_string(),
                |error| error.to_string()
            )
        ));
        if link_error.is_none() || binding == HeartbeatFinalBinding::Exact || link_count > 0 {
            return Err(post_commit_failure(
                file,
                identity,
                HeartbeatPublicationDurability::Unconfirmed,
                error,
            ));
        }
        return Err(HeartbeatPublicationFailure::PreCommit(error));
    }

    if let Err(error) = boundary(role, "directory-fsync").and_then(|()| {
        fsync(directory).map_err(|error| CommandFailure::diagnostic(error.to_string()))
    }) {
        return Err(post_commit_failure(
            file,
            identity,
            HeartbeatPublicationDurability::Unconfirmed,
            error,
        ));
    }
    if let Err(error) = boundary(role, "revalidate").and_then(|()| {
        (heartbeat_final_binding(&file, directory, final_name, identity)?
            == (HeartbeatFinalBinding::Exact, 1))
            .then_some(())
            .ok_or_else(|| CommandFailure::diagnostic("heartbeat final identity changed"))
    }) {
        return Err(post_commit_failure(
            file,
            identity,
            HeartbeatPublicationDurability::Unconfirmed,
            error,
        ));
    }
    Ok(HeartbeatPublication {
        file,
        device: identity.0,
        inode: identity.1,
        durability: HeartbeatPublicationDurability::Durable,
    })
}

#[cfg(target_os = "linux")]
#[allow(dead_code)]
fn publish_private_heartbeat_file(
    directory: &impl std::os::fd::AsFd,
    final_name: &str,
    document: &[u8],
    role: &str,
    boundary: &mut impl FnMut(&str, &str) -> Result<(), CommandFailure>,
) -> Result<HeartbeatPublication, HeartbeatPublicationFailure> {
    validate_heartbeat_final_name(final_name)?;
    let prepared = prepare_private_heartbeat_file(directory, document, role, boundary)?;
    publish_prepared_heartbeat_file(directory, final_name, prepared, role, boundary)
}

#[cfg(all(unix, not(target_os = "linux")))]
#[allow(dead_code)]
fn publish_private_heartbeat_file(
    _directory: &impl std::os::fd::AsFd,
    final_name: &str,
    _document: &[u8],
    _role: &str,
    _boundary: &mut impl FnMut(&str, &str) -> Result<(), CommandFailure>,
) -> Result<HeartbeatPublication, HeartbeatPublicationFailure> {
    validate_heartbeat_final_name(final_name)?;
    Err(HeartbeatPublicationFailure::PreCommit(
        CommandFailure::diagnostic(
            "identity-bound heartbeat publication is unavailable on this platform",
        ),
    ))
}

fn heartbeat_session_key(session_id: &str) -> String {
    session_id
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(crate) fn heartbeat_root() -> Result<std::path::PathBuf, CommandFailure> {
    let root = if let Ok(value) = std::env::var("AUTOSPEC_HEARTBEAT_DIR") {
        (!value.is_empty()).then(|| value.into())
    } else {
        None
    }
    .or_else(|| {
        std::env::var("AUTOSPEC_WATCHDOG_DIR")
            .ok()
            .filter(|value| !value.is_empty())
            .map(|value| std::path::PathBuf::from(value).join("process-heartbeats"))
    })
    .map(Ok)
    .unwrap_or_else(|| {
        std::env::var("HOME")
            .map(|home| std::path::PathBuf::from(home).join(".autospec/process-heartbeats"))
            .map_err(|_| {
                CommandFailure::diagnostic("could not resolve heartbeat directory: HOME is not set")
            })
    })?;
    #[cfg(target_os = "linux")]
    prepare_heartbeat_root_parent_with_hook(&root, |_| Ok(()))?;
    Ok(root)
}

#[cfg(target_os = "linux")]
fn terminal_heartbeat_snapshot(
    directory: &fs::File,
    name: &std::ffi::OsStr,
    identity: ClaimMutationIdentity<'_>,
) -> Result<StartupHeartbeatSnapshot, CommandFailure> {
    let mut archive = None;
    let file = match read_regular_file_at_no_follow(directory, name) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let expected = StartupHeartbeatExpectation {
                repo: identity.repo,
                issue: identity.issue,
                worker_id: identity.worker_id,
                branch: identity.branch,
                pull_request: "",
                claim_id: identity.claim_id,
                step: "claimed",
            };
            let completed = heartbeat_receipt_names(expected).1;
            let archive_name = completed.replace(".receipt", ".json");
            let handoff = open_receipt_anchors_with_hook(directory, |_| {})?;
            let file = read_regular_file_at_no_follow(&handoff, archive_name.as_ref())
                .map_err(|error| CommandFailure::diagnostic(format!("retained read: {error}")))?;
            archive = Some((handoff, archive_name));
            file
        }
        Err(error) => {
            return Err(CommandFailure::diagnostic(format!(
                "terminal heartbeat read: {error}"
            )))
        }
    };
    let evidence = parse_startup_heartbeat(&file.document)
        .ok_or_else(|| CommandFailure::diagnostic("terminal heartbeat evidence is invalid"))?;
    let snapshot = StartupHeartbeatSnapshot { file, evidence };
    let (source, observed) = archive
        .as_ref()
        .map_or((directory, name), |(handoff, name)| {
            (handoff, name.as_ref())
        });
    held_heartbeat_at(source, observed, &snapshot)?
        .ok_or_else(|| CommandFailure::diagnostic("terminal heartbeat evidence is absent"))?;
    Ok(snapshot)
}
#[cfg(target_os = "linux")]
fn handoff_terminal_heartbeat(
    state_root: &Path,
    directory: &fs::File,
    name: &std::ffi::OsStr,
    snapshot: &StartupHeartbeatSnapshot,
    role: &str,
    boundary: &mut impl FnMut(&str, &str) -> Result<(), CommandFailure>,
) -> Result<(), CommandFailure> {
    handoff_retained_heartbeat(
        state_root,
        directory,
        name,
        snapshot,
        |_, _, _, _| {},
        |sync| {
            boundary(
                role,
                match sync {
                    HeartbeatHandoffSyncBoundary::Source => "source-fsync",
                    HeartbeatHandoffSyncBoundary::Handoff => "archive-fsync",
                    HeartbeatHandoffSyncBoundary::Cleanup => "cleanup",
                    HeartbeatHandoffSyncBoundary::RestoreRoot => "restore-source",
                    HeartbeatHandoffSyncBoundary::RestoreUnlink => "restore-unlink",
                    HeartbeatHandoffSyncBoundary::RestoreHandoff => "restore-archive",
                },
            )
        },
    )
    .map(|_| ())
}
#[cfg(target_os = "linux")]
fn retire_released_startup_heartbeat_with_hook(
    identity: ClaimMutationIdentity<'_>,
    require_dead_process: bool,
    boundary: &mut impl FnMut(&str, &str) -> Result<(), CommandFailure>,
) -> Result<(), CommandFailure> {
    use nix::fcntl::{open, OFlag};
    use nix::sys::stat::Mode;
    let root_path = heartbeat_root()?;
    let root = fs::File::from(
        open(
            &root_path,
            OFlag::O_PATH | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
            Mode::empty(),
        )
        .map_err(|error| CommandFailure::diagnostic(format!("heartbeat root open: {error}")))?,
    );
    private_heartbeat_directory_identity(&root, "terminal root")?;
    let repo_name = super::autonomous::drain::repository_progress_key(identity.repo);
    let repo = open_heartbeat_directory_beneath(&root, Path::new(&repo_name))?;
    let issue_name = format!("{}.json", identity.issue);
    let issue = terminal_heartbeat_snapshot(&repo, issue_name.as_ref(), identity)?;
    let evidence = &issue.evidence;
    let exact = evidence.repo == identity.repo
        && evidence.issue == identity.issue.to_string()
        && evidence.worker_id == identity.worker_id
        && evidence.branch == identity.branch
        && evidence.claim_id == identity.claim_id
        && evidence.step == "claimed"
        && evidence.pr.is_empty()
        && evidence.ttl_seconds > 0
        && evidence.pid > 0
        && evidence.pid <= i32::MAX as u32
        && evidence.nonce
            == startup_heartbeat_nonce(identity.repo, identity.issue, identity.claim_id)
        && !evidence.host.is_empty()
        && !evidence.boot_id.is_empty()
        && evidence
            .session_id
            .as_ref()
            .is_none_or(|value| !value.is_empty())
        && evidence
            .process_start
            .parse::<u64>()
            .ok()
            .filter(|value| *value > 0)
            .is_some_and(|value| value.to_string() == evidence.process_start);
    if !exact {
        return Err(CommandFailure::diagnostic(
            "terminal heartbeat does not match prepared claim",
        ));
    }
    let liveness = observe_local_startup_pid(
        &evidence.worker_id,
        evidence.pid,
        &evidence.host,
        &evidence.boot_id,
        &evidence.process_start,
    );
    if liveness == StartupPidLiveness::Unknown
        || (require_dead_process
            && liveness != StartupPidLiveness::Dead
            && evidence.pid != std::process::id())
    {
        return Err(CommandFailure::diagnostic(
            "terminal heartbeat process identity is ambiguous",
        ));
    }
    let repo_path = root_path.join(&repo_name);
    if let Some(session) = &evidence.session_id {
        let sessions = open_heartbeat_directory_beneath(&repo, Path::new("sessions"))?;
        let session_name = format!("{}.json", heartbeat_session_key(session));
        let session = terminal_heartbeat_snapshot(&sessions, session_name.as_ref(), identity)?;
        if session.evidence != issue.evidence {
            return Err(CommandFailure::diagnostic(
                "terminal session heartbeat does not match issue evidence",
            ));
        }
        handoff_terminal_heartbeat(
            &repo_path.join("sessions"),
            &sessions,
            session_name.as_ref(),
            &session,
            "session",
            boundary,
        )?;
    }
    handoff_terminal_heartbeat(
        &repo_path,
        &repo,
        issue_name.as_ref(),
        &issue,
        "issue",
        boundary,
    )
}
#[cfg(target_os = "linux")]
fn retire_released_startup_heartbeat(
    identity: ClaimMutationIdentity<'_>,
) -> Result<(), CommandFailure> {
    retire_released_startup_heartbeat_with_hook(identity, false, &mut |_, _| Ok(()))
}
#[cfg(not(target_os = "linux"))]
fn retire_released_startup_heartbeat(
    identity: ClaimMutationIdentity<'_>,
) -> Result<(), CommandFailure> {
    heartbeat_portable::retire_released(identity)
}
#[cfg(target_os = "linux")]
fn prepare_heartbeat_root_parent_with_hook(
    root: &Path,
    mut after_sync: impl FnMut(&'static str) -> Result<(), CommandFailure>,
) -> Result<(), CommandFailure> {
    use nix::fcntl::{open, openat, renameat2, AtFlags, OFlag, RenameFlags};
    use nix::sys::stat::{fstatat, mkdirat, Mode};

    let invalid = || CommandFailure::diagnostic("heartbeat root must have a named parent");
    let parent_path = root.parent().ok_or_else(&invalid)?;
    let ancestor_path = parent_path.parent().ok_or_else(&invalid)?;
    let component = parent_path.file_name().ok_or_else(invalid)?;
    let flags = OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC;
    let ancestor = open(ancestor_path, flags, Mode::empty()).map_err(|error| {
        CommandFailure::diagnostic(format!("could not open heartbeat parent ancestor: {error}"))
    })?;
    let anchor_flags = OFlag::O_PATH | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC;
    match openat(&ancestor, component, anchor_flags, Mode::empty()) {
        Ok(anchor) => {
            let expected = normalize_heartbeat_root_parent(&anchor, false, &mut after_sync)?;
            sync_heartbeat_parent_ancestor(&ancestor, &mut after_sync)?;
            let current =
                fstatat(&ancestor, component, AtFlags::AT_SYMLINK_NOFOLLOW).map_err(|error| {
                    CommandFailure::diagnostic(format!(
                        "could not revalidate heartbeat root parent: {error}"
                    ))
                })?;
            if (current.st_dev, current.st_ino) != (expected.device, expected.inode) {
                return Err(CommandFailure::diagnostic(
                    "heartbeat root parent identity changed during preparation",
                ));
            }
            return Ok(());
        }
        Err(nix::errno::Errno::ENOENT) => {}
        Err(error) => {
            return Err(CommandFailure::diagnostic(format!(
                "could not open heartbeat root parent: {error}"
            )))
        }
    }

    let staging = format!(
        ".autospec-heartbeat-stage-{}-{}",
        std::process::id(),
        UNIQUE_ID_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    );
    mkdirat(&ancestor, staging.as_str(), Mode::from_bits_truncate(0o700)).map_err(|error| {
        CommandFailure::diagnostic(format!("could not stage heartbeat root parent: {error}"))
    })?;
    let anchor = match openat(&ancestor, staging.as_str(), anchor_flags, Mode::empty()) {
        Ok(anchor) => anchor,
        Err(error) => {
            return Err(cleanup_staged_heartbeat_parent(
                &ancestor,
                &staging,
                CommandFailure::diagnostic(format!(
                    "could not open staged heartbeat root parent: {error}"
                )),
                &mut after_sync,
            ));
        }
    };
    let mut published = false;
    let prepared = (|| {
        let expected = normalize_heartbeat_root_parent(&anchor, true, &mut after_sync)?;
        after_sync("before-publish")?;
        renameat2(
            &ancestor,
            staging.as_str(),
            &ancestor,
            component,
            RenameFlags::RENAME_NOREPLACE,
        )
        .map_err(|error| {
            CommandFailure::diagnostic(format!("could not publish heartbeat root parent: {error}"))
        })?;
        published = true;
        sync_heartbeat_parent_ancestor(&ancestor, &mut after_sync)?;
        let current =
            fstatat(&ancestor, component, AtFlags::AT_SYMLINK_NOFOLLOW).map_err(|error| {
                CommandFailure::diagnostic(format!(
                    "could not revalidate heartbeat root parent: {error}"
                ))
            })?;
        if (current.st_dev, current.st_ino) != (expected.device, expected.inode) {
            return Err(CommandFailure::diagnostic(
                "heartbeat root parent identity changed during publication",
            ));
        }
        Ok(())
    })();
    if let Err(error) = prepared {
        if published {
            return Err(error);
        }
        drop(anchor);
        return Err(cleanup_staged_heartbeat_parent(
            &ancestor,
            &staging,
            error,
            &mut after_sync,
        ));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn sync_heartbeat_parent_ancestor(
    ancestor: &impl std::os::fd::AsFd,
    boundary: &mut impl FnMut(&'static str) -> Result<(), CommandFailure>,
) -> Result<(), CommandFailure> {
    boundary("ancestor-fsync")?;
    nix::unistd::fsync(ancestor).map_err(|error| {
        CommandFailure::diagnostic(format!("could not sync heartbeat parent ancestor: {error}"))
    })?;
    boundary("ancestor")
}

#[cfg(target_os = "linux")]
fn cleanup_staged_heartbeat_parent(
    ancestor: &impl std::os::fd::AsFd,
    staging: &str,
    original: CommandFailure,
    boundary: &mut impl FnMut(&'static str) -> Result<(), CommandFailure>,
) -> CommandFailure {
    use nix::unistd::{unlinkat, UnlinkatFlags};

    if let Err(error) = unlinkat(ancestor, staging, UnlinkatFlags::RemoveDir) {
        return CommandFailure::diagnostic(format!(
            "{original}; could not remove staged heartbeat root parent: {error}"
        ));
    }
    if let Err(error) = nix::unistd::fsync(ancestor) {
        return CommandFailure::diagnostic(format!(
            "{original}; could not sync staged heartbeat cleanup: {error}"
        ));
    }
    if let Err(error) = boundary("cleanup-ancestor") {
        return CommandFailure::diagnostic(format!(
            "{original}; staged heartbeat cleanup boundary failed: {error}"
        ));
    }
    original
}

#[cfg(target_os = "linux")]
fn normalize_heartbeat_root_parent(
    anchor: &impl std::os::fd::AsFd,
    created: bool,
    boundary: &mut impl FnMut(&'static str) -> Result<(), CommandFailure>,
) -> Result<HeartbeatDirectoryIdentity, CommandFailure> {
    use nix::fcntl::{openat, OFlag};
    use nix::sys::stat::{fchmod, fstat, Mode, SFlag};
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::PermissionsExt;

    let before = fstat(anchor).map_err(|error| {
        CommandFailure::diagnostic(format!("could not inspect heartbeat root parent: {error}"))
    })?;
    if !SFlag::from_bits_truncate(before.st_mode).contains(SFlag::S_IFDIR)
        || before.st_uid != nix::unistd::geteuid().as_raw()
    {
        return Err(CommandFailure::diagnostic(
            "heartbeat root parent must be owned by the effective user",
        ));
    }
    if created || before.st_mode & 0o7777 != 0o700 {
        boundary("chmod")?;
        fs::set_permissions(
            format!("/proc/self/fd/{}", anchor.as_fd().as_raw_fd()),
            fs::Permissions::from_mode(0o700),
        )
        .map_err(|error| {
            CommandFailure::diagnostic(format!(
                "could not make heartbeat root parent accessible: {error}"
            ))
        })?;
    }
    let parent = openat(
        anchor,
        ".",
        OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
        Mode::empty(),
    )
    .map_err(|error| {
        CommandFailure::diagnostic(format!("could not reopen heartbeat root parent: {error}"))
    })?;
    if created || before.st_mode & 0o7777 != 0o700 {
        fchmod(&parent, Mode::from_bits_truncate(0o700)).map_err(|error| {
            CommandFailure::diagnostic(format!("could not protect heartbeat root parent: {error}"))
        })?;
        nix::unistd::fsync(&parent).map_err(|error| {
            CommandFailure::diagnostic(format!("could not sync heartbeat root parent: {error}"))
        })?;
        boundary("parent")?;
    }
    private_heartbeat_directory_identity(&parent, "root parent")
}

#[cfg(target_os = "linux")]
use open_beneath::open_heartbeat_directory_beneath;
#[cfg(all(test, unix))]
use open_beneath::open_heartbeat_directory_portable_unix_with_hook;
#[cfg(unix)]
use open_beneath::private_heartbeat_directory_identity;
#[cfg(all(test, target_os = "linux"))]
use open_beneath::{heartbeat_openat2_resolve_flags, open_heartbeat_directory_beneath_with_hook};
#[cfg(target_os = "linux")]
use open_beneath::{private_heartbeat_name_identity, HeartbeatDirectoryIdentity};

// These primitives remain inert until the guarded recovery activation slice.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
struct StartupHeartbeatExpectation<'a> {
    repo: &'a str,
    issue: u64,
    worker_id: &'a str,
    branch: &'a str,
    pull_request: &'a str,
    claim_id: &'a str,
    step: &'a str,
}
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
struct StartupHeartbeatEvidence {
    repo: String,
    issue: String,
    worker_id: String,
    branch: String,
    pr: String,
    claim_id: String,
    step: String,
    ts: u64,
    ttl_seconds: u64,
    pid: u32,
    nonce: String,
    host: String,
    boot_id: String,
    process_start: String,
    session_id: Option<String>,
}
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
struct RegularFileIdentity {
    device: u64,
    inode: u64,
    length: u64,
    modified_seconds: i64,
    modified_nanos: i64,
}
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
struct RegularFileSnapshot {
    document: Vec<u8>,
    identity: RegularFileIdentity,
}
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
struct StartupHeartbeatSnapshot {
    file: RegularFileSnapshot,
    evidence: StartupHeartbeatEvidence,
}
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StartupPidLiveness {
    Live,
    Dead,
    Unknown,
}
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
enum StartupHeartbeatClassification {
    Absent,
    Blocking,
    /// Owner proven dead. Expiry is not required: liveness alone decides.
    ExpiredDead(Box<StartupHeartbeatSnapshot>),
}
#[cfg(unix)]
#[allow(dead_code)]
fn file_identity(metadata: &fs::Metadata) -> RegularFileIdentity {
    RegularFileIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
        length: metadata.len(),
        modified_seconds: metadata.mtime(),
        modified_nanos: metadata.mtime_nsec(),
    }
}
#[cfg(unix)]
#[allow(dead_code)]
fn read_regular_file(mut file: fs::File) -> std::io::Result<RegularFileSnapshot> {
    std::io::Seek::rewind(&mut file)?;
    let before = file.metadata()?;
    if !before.is_file() {
        return Err(std::io::Error::other("heartbeat is not a regular file"));
    }
    let before = file_identity(&before);
    let mut document = Vec::new();
    file.read_to_end(&mut document)?;
    let after = file_identity(&file.metadata()?);
    if before != after || after.length != document.len() as u64 {
        return Err(std::io::Error::other(
            "heartbeat changed while it was being read",
        ));
    }
    Ok(RegularFileSnapshot {
        document,
        identity: after,
    })
}
#[cfg(unix)]
#[allow(dead_code)]
fn read_regular_file_no_follow(path: &Path) -> std::io::Result<RegularFileSnapshot> {
    let mut options = OpenOptions::new();
    options
        .read(true)
        .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_CLOEXEC | nix::libc::O_NONBLOCK);
    read_regular_file(options.open(path)?)
}
#[cfg(unix)]
#[allow(dead_code)]
fn read_regular_file_at_no_follow(
    directory: &impl std::os::fd::AsFd,
    name: &std::ffi::OsStr,
) -> std::io::Result<RegularFileSnapshot> {
    use nix::fcntl::{openat, OFlag};
    use nix::sys::stat::Mode;
    let descriptor = openat(
        directory,
        name,
        OFlag::O_RDONLY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC | OFlag::O_NONBLOCK,
        Mode::empty(),
    )
    .map_err(std::io::Error::from)?;
    read_regular_file(fs::File::from(descriptor))
}
#[cfg(not(unix))]
#[allow(dead_code)]
fn read_regular_file_no_follow(path: &Path) -> std::io::Result<RegularFileSnapshot> {
    match fs::symlink_metadata(path) {
        Err(error) => Err(error),
        Ok(_) => Err(std::io::Error::other("heartbeat snapshots require Unix")),
    }
}
#[allow(dead_code)]
fn parse_startup_heartbeat(document: &[u8]) -> Option<StartupHeartbeatEvidence> {
    let mut fields = JsonParser::new(std::str::from_utf8(document).ok()?)
        .parse()
        .ok()?
        .into_object("startup heartbeat")
        .ok()?;
    let (repo, issue, worker_id, branch, pr, claim_id, step, nonce, host, boot_id, process_start) = {
        let mut string = |name| fields.remove(name)?.into_string(name).ok();
        (
            string("repo")?,
            string("issue")?,
            string("worker_id")?,
            string("branch")?,
            string("pr")?,
            string("claim_id")?,
            string("step")?,
            string("nonce")?,
            string("host")?,
            string("boot_id")?,
            string("process_start")?,
        )
    };
    let (ts, ttl_seconds, pid) = {
        let mut number = |name| fields.remove(name)?.into_number(name).ok();
        (
            number("ts")?,
            number("ttl_seconds")?,
            u32::try_from(number("pid")?).ok()?,
        )
    };
    let session_id = fields
        .remove("session_id")
        .map(|value| value.into_string("session_id"))
        .transpose()
        .ok()?;
    fields.is_empty().then_some(StartupHeartbeatEvidence {
        repo,
        issue,
        worker_id,
        branch,
        pr,
        claim_id,
        step,
        ts,
        ttl_seconds,
        pid,
        nonce,
        host,
        boot_id,
        process_start,
        session_id,
    })
}
#[allow(dead_code)]
fn classify_startup_heartbeat(
    path: &Path,
    expected: StartupHeartbeatExpectation<'_>,
    now: u64,
    observe_pid: impl FnOnce(&str, u32, &str, &str, &str) -> StartupPidLiveness,
) -> StartupHeartbeatClassification {
    let file = match read_regular_file_no_follow(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return StartupHeartbeatClassification::Absent;
        }
        Err(_) => return StartupHeartbeatClassification::Blocking,
    };
    classify_startup_heartbeat_snapshot(file, expected, now, observe_pid)
}

#[cfg(target_os = "linux")]
fn classify_startup_heartbeat_at(
    directory: &impl std::os::fd::AsFd,
    name: &std::ffi::OsStr,
    expected: StartupHeartbeatExpectation<'_>,
    now: u64,
    observe_pid: impl FnOnce(&str, u32, &str, &str, &str) -> StartupPidLiveness,
) -> StartupHeartbeatClassification {
    let file = match read_regular_file_at_no_follow(directory, name) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return StartupHeartbeatClassification::Absent;
        }
        Err(_) => return StartupHeartbeatClassification::Blocking,
    };
    classify_startup_heartbeat_snapshot(file, expected, now, observe_pid)
}

fn classify_startup_heartbeat_snapshot(
    file: RegularFileSnapshot,
    expected: StartupHeartbeatExpectation<'_>,
    _now: u64,
    observe_pid: impl FnOnce(&str, u32, &str, &str, &str) -> StartupPidLiveness,
) -> StartupHeartbeatClassification {
    let Some(evidence) = parse_startup_heartbeat(&file.document) else {
        return StartupHeartbeatClassification::Blocking;
    };
    let exact = evidence.repo == expected.repo
        && evidence.issue == expected.issue.to_string()
        && evidence.worker_id == expected.worker_id
        && evidence.branch == expected.branch
        && evidence.pr == expected.pull_request
        && evidence.claim_id == expected.claim_id
        && evidence.step == expected.step
        && evidence.ttl_seconds > 0
        && evidence.pid > 0
        && evidence.pid <= i32::MAX as u32
        && evidence.nonce
            == startup_heartbeat_nonce(expected.repo, expected.issue, expected.claim_id)
        && !evidence.host.is_empty()
        && !evidence.boot_id.is_empty()
        && evidence
            .process_start
            .parse::<u64>()
            .ok()
            .filter(|start| *start > 0)
            .is_some_and(|start| start.to_string() == evidence.process_start);
    // Liveness decides this, not the clock: a crash loop rewrites the heartbeat and
    // pushes expiry out forever. `exact` plus observe_pid's host/boot/start match is
    // what keeps the takeover safe.
    if !exact || !cfg!(unix) {
        return StartupHeartbeatClassification::Blocking;
    }
    if observe_pid(
        &evidence.worker_id,
        evidence.pid,
        &evidence.host,
        &evidence.boot_id,
        &evidence.process_start,
    ) != StartupPidLiveness::Dead
    {
        return StartupHeartbeatClassification::Blocking;
    }
    StartupHeartbeatClassification::ExpiredDead(Box::new(StartupHeartbeatSnapshot {
        file,
        evidence,
    }))
}

// These descriptor-only primitives remain inert until guarded recovery integrates them.
#[cfg(target_os = "linux")]
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HeartbeatReceiptDecision {
    Absent,
    Blocking,
    Pending,
    Completed,
}
#[cfg(target_os = "linux")]
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HeartbeatReceiptEntry {
    Missing,
    Exact,
    Unsafe,
}
#[cfg(target_os = "linux")]
#[allow(dead_code)]
struct HeartbeatReceiptTransaction {
    handoff: fs::File,
    pending: String,
    completed: String,
}
#[cfg(target_os = "linux")]
#[allow(dead_code)]
fn heartbeat_receipt_names(expected: StartupHeartbeatExpectation<'_>) -> (String, String) {
    let issue = expected.issue.to_string();
    let mut identity = Vec::new();
    for field in [
        expected.repo,
        &issue,
        expected.worker_id,
        expected.branch,
        expected.pull_request,
        expected.claim_id,
        expected.step,
    ] {
        identity.extend_from_slice(&(field.len() as u64).to_be_bytes());
        identity.extend_from_slice(field.as_bytes());
    }
    let digest = autospec_core::autonomous::waterfall::sha256_hex(&identity);
    (
        format!("pending-{}-{digest}.receipt", expected.issue),
        format!("completed-{}-{digest}.receipt", expected.issue),
    )
}
#[cfg(target_os = "linux")]
fn open_receipt_anchors_with_hook(
    trusted_repo: &fs::File,
    mut after_open: impl FnMut(&str),
) -> Result<fs::File, CommandFailure> {
    let identity = private_heartbeat_directory_identity(trusted_repo, "receipt repo")?;
    after_open("repo");
    if private_heartbeat_directory_identity(trusted_repo, "receipt repo")? != identity {
        return Err(CommandFailure::diagnostic(
            "heartbeat receipt repo descriptor identity drift",
        ));
    }
    let quarantine = open_heartbeat_directory_beneath(trusted_repo, Path::new("quarantine"))?;
    after_open("quarantine");
    let handoff =
        open_heartbeat_directory_beneath(&quarantine, Path::new("startup-heartbeat-handoffs"))?;
    after_open("handoff");
    private_heartbeat_directory_identity(trusted_repo, "receipt repo")?;
    private_heartbeat_directory_identity(&quarantine, "receipt quarantine")?;
    private_heartbeat_directory_identity(&handoff, "receipt handoff")?;
    Ok(handoff)
}
#[cfg(target_os = "linux")]
fn ensure_receipt_directory(parent: &fs::File, name: &str) -> Result<fs::File, CommandFailure> {
    use nix::sys::stat::{mkdirat, Mode};
    let created = match mkdirat(parent, name, Mode::from_bits_truncate(0o700)) {
        Ok(()) => true,
        Err(nix::errno::Errno::EEXIST) => false,
        Err(error) => {
            return Err(CommandFailure::diagnostic(format!(
                "could not create heartbeat receipt directory: {error}"
            )))
        }
    };
    let directory = open_heartbeat_directory_beneath(parent, Path::new(name))?;
    if created {
        nix::unistd::fsync(parent).map_err(|error| {
            CommandFailure::diagnostic(format!(
                "could not sync heartbeat receipt ancestry: {error}"
            ))
        })?;
    }
    Ok(directory)
}
#[cfg(target_os = "linux")]
#[allow(dead_code)]
fn inspect_heartbeat_receipt(
    directory: &fs::File,
    name: &std::ffi::OsStr,
) -> HeartbeatReceiptEntry {
    use nix::fcntl::AtFlags;
    use nix::sys::stat::{fstatat, SFlag};

    let stat = match fstatat(directory, name, AtFlags::AT_SYMLINK_NOFOLLOW) {
        Ok(stat) => stat,
        Err(nix::errno::Errno::ENOENT) => return HeartbeatReceiptEntry::Missing,
        Err(_) => return HeartbeatReceiptEntry::Unsafe,
    };
    if SFlag::from_bits_truncate(stat.st_mode) & SFlag::S_IFMT == SFlag::S_IFREG
        && stat.st_uid == nix::unistd::geteuid().as_raw()
        && stat.st_mode & 0o7777 == 0o600
        && stat.st_nlink == 1
        && stat.st_size == 0
    {
        HeartbeatReceiptEntry::Exact
    } else {
        HeartbeatReceiptEntry::Unsafe
    }
}
#[cfg(target_os = "linux")]
#[allow(dead_code)]
fn heartbeat_receipt_retry_decision(
    trusted_repo: &fs::File,
    expected: StartupHeartbeatExpectation<'_>,
) -> HeartbeatReceiptDecision {
    heartbeat_receipt_retry_decision_with_hook(trusted_repo, expected, |_| {})
}
#[cfg(target_os = "linux")]
fn heartbeat_receipt_retry_decision_with_hook(
    trusted_repo: &fs::File,
    expected: StartupHeartbeatExpectation<'_>,
    after_open: impl FnMut(&str),
) -> HeartbeatReceiptDecision {
    use nix::fcntl::AtFlags;
    use nix::sys::stat::fstatat;

    let quarantine = match fstatat(trusted_repo, "quarantine", AtFlags::AT_SYMLINK_NOFOLLOW) {
        Err(nix::errno::Errno::ENOENT) => return HeartbeatReceiptDecision::Absent,
        Err(_) => return HeartbeatReceiptDecision::Blocking,
        Ok(_) => match open_heartbeat_directory_beneath(trusted_repo, Path::new("quarantine")) {
            Ok(directory) => directory,
            Err(_) => return HeartbeatReceiptDecision::Blocking,
        },
    };
    match fstatat(
        &quarantine,
        "startup-heartbeat-handoffs",
        AtFlags::AT_SYMLINK_NOFOLLOW,
    ) {
        Err(nix::errno::Errno::ENOENT) => return HeartbeatReceiptDecision::Absent,
        Err(_) => return HeartbeatReceiptDecision::Blocking,
        Ok(_) => {}
    }
    let Ok(handoff) = open_receipt_anchors_with_hook(trusted_repo, after_open) else {
        return HeartbeatReceiptDecision::Blocking;
    };
    let (pending, completed) = heartbeat_receipt_names(expected);
    match (
        inspect_heartbeat_receipt(&handoff, pending.as_ref()),
        inspect_heartbeat_receipt(&handoff, completed.as_ref()),
    ) {
        (HeartbeatReceiptEntry::Missing, HeartbeatReceiptEntry::Missing) => {
            HeartbeatReceiptDecision::Absent
        }
        (HeartbeatReceiptEntry::Exact, HeartbeatReceiptEntry::Missing) => {
            HeartbeatReceiptDecision::Pending
        }
        (HeartbeatReceiptEntry::Missing, HeartbeatReceiptEntry::Exact) => {
            HeartbeatReceiptDecision::Completed
        }
        _ => HeartbeatReceiptDecision::Blocking,
    }
}

#[cfg(target_os = "linux")]
fn classify_retained_prior_generation(
    trusted_repo: &fs::File,
    repo_name: &str,
    issue: u64,
    record: &RunStateRecord,
    now: u64,
) -> Result<StartupHeartbeatClassification, CommandFailure> {
    use nix::fcntl::AtFlags;
    use nix::sys::stat::fstatat;
    use std::os::fd::AsRawFd;

    let quarantine = match fstatat(trusted_repo, "quarantine", AtFlags::AT_SYMLINK_NOFOLLOW) {
        Err(nix::errno::Errno::ENOENT) => return Ok(StartupHeartbeatClassification::Absent),
        Err(_) => return Ok(StartupHeartbeatClassification::Blocking),
        Ok(_) => open_heartbeat_directory_beneath(trusted_repo, Path::new("quarantine"))?,
    };
    match fstatat(
        &quarantine,
        "startup-heartbeat-handoffs",
        AtFlags::AT_SYMLINK_NOFOLLOW,
    ) {
        Err(nix::errno::Errno::ENOENT) => return Ok(StartupHeartbeatClassification::Absent),
        Err(_) => return Ok(StartupHeartbeatClassification::Blocking),
        Ok(_) => {}
    }
    let handoff = open_receipt_anchors_with_hook(trusted_repo, |_| {})?;
    let directory = format!("/proc/self/fd/{}", handoff.as_raw_fd());
    let entries = fs::read_dir(directory)
        .map_err(|error| CommandFailure::diagnostic(format!("retained scan: {error}")))?;
    let pending_prefix = format!("pending-{issue}-");
    let completed_prefix = format!("completed-{issue}-");
    let mut relevant = 0usize;
    let mut candidate = None;
    for entry in entries {
        let entry = entry
            .map_err(|error| CommandFailure::diagnostic(format!("retained entry: {error}")))?;
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            return Ok(StartupHeartbeatClassification::Blocking);
        };
        if (name.starts_with(&pending_prefix) || name.starts_with(&completed_prefix))
            && name.ends_with(".receipt")
        {
            relevant += 1;
            continue;
        }
        if !name.starts_with(&completed_prefix) || !name.ends_with(".json") {
            continue;
        }
        relevant += 1;
        let Ok(file) = read_regular_file_at_no_follow(&handoff, name.as_ref()) else {
            return Ok(StartupHeartbeatClassification::Blocking);
        };
        let Some(evidence) = parse_startup_heartbeat(&file.document) else {
            return Ok(StartupHeartbeatClassification::Blocking);
        };
        if evidence.repo != repo_name
            || evidence.issue != issue.to_string()
            || evidence.branch != record.branch
            || !evidence.pr.is_empty()
            || (evidence.worker_id == record.worker_id
                && evidence.claim_id.as_str() == record.claim_id.as_deref().unwrap_or_default())
        {
            return Ok(StartupHeartbeatClassification::Blocking);
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
        if heartbeat_receipt_names(expected)
            .1
            .replace(".receipt", ".json")
            != name
            || !matches!(
                heartbeat_receipt_retry_decision(trusted_repo, expected),
                HeartbeatReceiptDecision::Pending | HeartbeatReceiptDecision::Completed
            )
        {
            return Ok(StartupHeartbeatClassification::Blocking);
        }
        match classify_startup_heartbeat_snapshot(file, expected, now, observe_local_startup_pid) {
            StartupHeartbeatClassification::ExpiredDead(snapshot) if candidate.is_none() => {
                candidate = Some(snapshot)
            }
            _ => return Ok(StartupHeartbeatClassification::Blocking),
        }
    }
    Ok(match candidate {
        Some(snapshot) if relevant == 2 => StartupHeartbeatClassification::ExpiredDead(snapshot),
        Some(_) => StartupHeartbeatClassification::Blocking,
        None if relevant > 0 => StartupHeartbeatClassification::Blocking,
        None => StartupHeartbeatClassification::Absent,
    })
}
#[cfg(target_os = "linux")]
#[allow(dead_code)]
fn begin_heartbeat_receipt(
    trusted_repo: &fs::File,
    expected: StartupHeartbeatExpectation<'_>,
) -> Result<HeartbeatReceiptTransaction, CommandFailure> {
    begin_heartbeat_receipt_with_hook(trusted_repo, expected, || {})
}
#[cfg(target_os = "linux")]
fn begin_heartbeat_receipt_with_hook(
    trusted_repo: &fs::File,
    expected: StartupHeartbeatExpectation<'_>,
    after_pending_create: impl FnOnce(),
) -> Result<HeartbeatReceiptTransaction, CommandFailure> {
    use nix::fcntl::{openat, OFlag};
    use nix::sys::stat::{fchmod, Mode};

    private_heartbeat_directory_identity(trusted_repo, "receipt repo")?;
    let quarantine = ensure_receipt_directory(trusted_repo, "quarantine")?;
    let handoff = ensure_receipt_directory(&quarantine, "startup-heartbeat-handoffs")?;
    private_heartbeat_directory_identity(trusted_repo, "receipt repo")?;
    private_heartbeat_directory_identity(&quarantine, "receipt quarantine")?;
    private_heartbeat_directory_identity(&handoff, "receipt handoff")?;
    let (pending, completed) = heartbeat_receipt_names(expected);
    if inspect_heartbeat_receipt(&handoff, completed.as_ref()) != HeartbeatReceiptEntry::Missing {
        return Err(CommandFailure::diagnostic(
            "completed heartbeat receipt blocks pending creation",
        ));
    }
    let descriptor = openat(
        &handoff,
        pending.as_str(),
        OFlag::O_WRONLY | OFlag::O_CREAT | OFlag::O_EXCL | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
        Mode::from_bits_truncate(0o600),
    )
    .map_err(|error| {
        CommandFailure::diagnostic(format!("could not create pending receipt: {error}"))
    })?;
    fchmod(&descriptor, Mode::from_bits_truncate(0o600)).map_err(|error| {
        CommandFailure::diagnostic(format!("could not protect pending receipt: {error}"))
    })?;
    fs::File::from(descriptor).sync_all().map_err(|error| {
        CommandFailure::diagnostic(format!("could not sync pending receipt: {error}"))
    })?;
    after_pending_create();
    private_heartbeat_directory_identity(trusted_repo, "receipt repo")?;
    private_heartbeat_directory_identity(&quarantine, "receipt quarantine")?;
    private_heartbeat_directory_identity(&handoff, "receipt handoff")?;
    if inspect_heartbeat_receipt(&handoff, completed.as_ref()) != HeartbeatReceiptEntry::Missing {
        return Err(CommandFailure::diagnostic(
            "completed heartbeat receipt won pending creation",
        ));
    }
    nix::unistd::fsync(&handoff).map_err(|error| {
        CommandFailure::diagnostic(format!("could not sync pending receipt directory: {error}"))
    })?;
    Ok(HeartbeatReceiptTransaction {
        handoff,
        pending,
        completed,
    })
}
#[cfg(target_os = "linux")]
#[allow(dead_code)]
fn retire_heartbeat_receipt_with_sync(
    transaction: HeartbeatReceiptTransaction,
    sync: impl FnOnce(&fs::File) -> Result<(), CommandFailure>,
) -> Result<(), CommandFailure> {
    nix::fcntl::renameat2(
        &transaction.handoff,
        transaction.pending.as_str(),
        &transaction.handoff,
        transaction.completed.as_str(),
        nix::fcntl::RenameFlags::RENAME_NOREPLACE,
    )
    .map_err(|error| {
        CommandFailure::diagnostic(format!("could not retire heartbeat receipt: {error}"))
    })?;
    sync(&transaction.handoff)
}
#[cfg(unix)]
#[allow(dead_code)]
fn revalidate_heartbeat_snapshot(
    observed: &RegularFileSnapshot,
    expected: &RegularFileSnapshot,
) -> Result<(), CommandFailure> {
    if observed.identity != expected.identity {
        return Err(CommandFailure::diagnostic(
            "startup heartbeat identity drift",
        ));
    }
    if observed.document != expected.document {
        return Err(CommandFailure::diagnostic(
            "startup heartbeat document drift",
        ));
    }
    Ok(())
}

#[cfg(unix)]
#[allow(dead_code)]
fn open_private_quarantine_component(
    parent: &std::os::fd::OwnedFd,
    component: &str,
    after_open: &mut impl FnMut(&str),
) -> Result<std::os::fd::OwnedFd, CommandFailure> {
    use nix::fcntl::{openat, OFlag};
    use nix::sys::stat::{fchmod, fstat, mkdirat, Mode};

    let private = Mode::from_bits_truncate(0o700);
    let created = match mkdirat(parent, component, private) {
        Ok(()) => true,
        Err(nix::errno::Errno::EEXIST) => false,
        Err(error) => {
            return Err(CommandFailure::diagnostic(format!(
                "could not create private heartbeat quarantine: {error}"
            )))
        }
    };
    let flags = OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC;
    let child = openat(parent, component, flags, Mode::empty()).map_err(|_| {
        CommandFailure::diagnostic(format!(
            "heartbeat quarantine ancestor {component} is not a safe directory"
        ))
    })?;
    fchmod(&child, private).map_err(|error| {
        CommandFailure::diagnostic(format!(
            "could not protect heartbeat quarantine ancestor {component}: {error}"
        ))
    })?;
    after_open(component);
    let verified = openat(parent, component, flags, Mode::empty()).map_err(|_| {
        CommandFailure::diagnostic(format!(
            "heartbeat quarantine ancestor {component} changed during creation"
        ))
    })?;
    let opened = fstat(&child).map_err(|error| {
        CommandFailure::diagnostic(format!("could not inspect heartbeat quarantine: {error}"))
    })?;
    let current = fstat(&verified).map_err(|error| {
        CommandFailure::diagnostic(format!("could not recheck heartbeat quarantine: {error}"))
    })?;
    if (opened.st_dev, opened.st_ino) != (current.st_dev, current.st_ino) {
        return Err(CommandFailure::diagnostic(format!(
            "heartbeat quarantine ancestor {component} changed during creation"
        )));
    }
    if created {
        nix::unistd::fsync(parent).map_err(|error| {
            CommandFailure::diagnostic(format!(
                "could not sync heartbeat quarantine ancestry: {error}"
            ))
        })?;
    }
    Ok(child)
}

#[cfg(unix)]
#[allow(dead_code)]
fn persist_heartbeat_copy(
    state_root: &Path,
    source: &Path,
    snapshot: &StartupHeartbeatSnapshot,
) -> Result<std::path::PathBuf, CommandFailure> {
    persist_heartbeat_copy_with_hooks(state_root, source, snapshot, |_| {}, |_| Ok(()))
}

#[cfg(unix)]
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HeartbeatCopySyncBoundary {
    File,
    Directory,
}

#[cfg(unix)]
fn sync_heartbeat_copy(
    file: &fs::File,
    directory: &std::os::fd::OwnedFd,
    after_sync: &mut impl FnMut(HeartbeatCopySyncBoundary) -> Result<(), CommandFailure>,
) -> Result<(), CommandFailure> {
    file.sync_all().map_err(|error| {
        CommandFailure::diagnostic(format!("could not sync heartbeat quarantine copy: {error}"))
    })?;
    after_sync(HeartbeatCopySyncBoundary::File)?;
    nix::unistd::fsync(directory).map_err(|error| {
        CommandFailure::diagnostic(format!(
            "could not sync heartbeat quarantine directory: {error}"
        ))
    })?;
    after_sync(HeartbeatCopySyncBoundary::Directory)
}

#[cfg(unix)]
#[allow(dead_code)]
fn persist_heartbeat_copy_with_hooks(
    state_root: &Path,
    source: &Path,
    snapshot: &StartupHeartbeatSnapshot,
    mut after_open: impl FnMut(&str),
    mut after_sync: impl FnMut(HeartbeatCopySyncBoundary) -> Result<(), CommandFailure>,
) -> Result<std::path::PathBuf, CommandFailure> {
    use nix::fcntl::{open, openat, OFlag};
    use nix::sys::stat::{fchmod, fstat, Mode};

    let observed = read_regular_file_no_follow(source).map_err(|error| {
        CommandFailure::diagnostic(format!("could not revalidate startup heartbeat: {error}"))
    })?;
    revalidate_heartbeat_snapshot(&observed, &snapshot.file)?;
    let directory_flags =
        OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC;
    let root = open(state_root, directory_flags, Mode::empty()).map_err(|_| {
        CommandFailure::diagnostic("heartbeat private state root is not a safe directory")
    })?;
    let root_stat = fstat(&root).map_err(|error| {
        CommandFailure::diagnostic(format!("could not inspect private state root: {error}"))
    })?;
    if root_stat.st_uid != nix::unistd::geteuid().as_raw() || root_stat.st_mode & 0o7777 != 0o700 {
        return Err(CommandFailure::diagnostic(
            "heartbeat private state root must be owned by the effective user with mode 0700",
        ));
    }
    let quarantine = open_private_quarantine_component(&root, "quarantine", &mut after_open)?;
    let copies =
        open_private_quarantine_component(&quarantine, "startup-heartbeats", &mut after_open)?;
    let filename = format!(
        "{}-{}.json",
        snapshot.evidence.issue,
        heartbeat_session_key(&snapshot.evidence.nonce)
    );
    let target = state_root
        .join("quarantine")
        .join("startup-heartbeats")
        .join(&filename);
    let flags =
        OFlag::O_WRONLY | OFlag::O_CREAT | OFlag::O_EXCL | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC;
    let descriptor = openat(
        &copies,
        filename.as_str(),
        flags,
        Mode::from_bits_truncate(0o600),
    )
    .map_err(|error| {
        CommandFailure::diagnostic(if error == nix::errno::Errno::EEXIST {
            "heartbeat quarantine copy already exists".to_string()
        } else {
            format!("could not create heartbeat quarantine copy: {error}")
        })
    })?;
    fchmod(&descriptor, Mode::from_bits_truncate(0o600)).map_err(|error| {
        CommandFailure::diagnostic(format!(
            "could not protect heartbeat quarantine copy: {error}"
        ))
    })?;
    let mut file = fs::File::from(descriptor);
    file.write_all(&snapshot.file.document).map_err(|error| {
        CommandFailure::diagnostic(format!(
            "could not persist heartbeat quarantine copy: {error}"
        ))
    })?;
    sync_heartbeat_copy(&file, &copies, &mut after_sync)?;
    Ok(target)
}

#[cfg(unix)]
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HeartbeatHandoffSyncBoundary {
    Source,
    Handoff,
    Cleanup,
    RestoreRoot,
    RestoreUnlink,
    RestoreHandoff,
}

#[cfg(unix)]
fn rename_heartbeat_no_clobber(
    source: &std::os::fd::OwnedFd,
    source_name: &std::ffi::OsStr,
    target: &std::os::fd::OwnedFd,
    target_name: &str,
) -> Result<(), CommandFailure> {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    {
        nix::fcntl::renameat2(
            source,
            source_name,
            target,
            target_name,
            nix::fcntl::RenameFlags::RENAME_NOREPLACE,
        )
        .map_err(|error| {
            CommandFailure::diagnostic(format!("could not hand off startup heartbeat: {error}"))
        })
    }
    #[cfg(not(all(target_os = "linux", target_env = "gnu")))]
    {
        let _ = (source, source_name, target, target_name);
        Err(CommandFailure::diagnostic(
            "identity-bound heartbeat handoff is unavailable on this platform",
        ))
    }
}

#[cfg(unix)]
#[allow(dead_code)]
fn restore_moved_heartbeat(
    root: &impl std::os::fd::AsFd,
    live_name: &std::ffi::OsStr,
    handoff: &impl std::os::fd::AsFd,
    moved_name: &str,
    after_sync: &mut impl FnMut(HeartbeatHandoffSyncBoundary) -> Result<(), CommandFailure>,
) -> Result<(), CommandFailure> {
    use nix::fcntl::AtFlags;
    use nix::unistd::{linkat, unlinkat, UnlinkatFlags};
    match linkat(handoff, moved_name, root, live_name, AtFlags::empty()) {
        Err(nix::errno::Errno::EEXIST) => Ok(()),
        Err(error) => Err(CommandFailure::diagnostic(format!(
            "could not restore moved startup heartbeat: {error}"
        ))),
        Ok(()) => {
            nix::unistd::fsync(root).map_err(|error| {
                CommandFailure::diagnostic(format!(
                    "could not sync restored startup heartbeat: {error}"
                ))
            })?;
            after_sync(HeartbeatHandoffSyncBoundary::RestoreRoot)?;
            unlinkat(handoff, moved_name, UnlinkatFlags::NoRemoveDir).map_err(|error| {
                CommandFailure::diagnostic(format!(
                    "could not remove restored heartbeat handoff: {error}"
                ))
            })?;
            after_sync(HeartbeatHandoffSyncBoundary::RestoreUnlink)?;
            nix::unistd::fsync(handoff).map_err(|error| {
                CommandFailure::diagnostic(format!(
                    "could not sync restored heartbeat handoff: {error}"
                ))
            })?;
            after_sync(HeartbeatHandoffSyncBoundary::RestoreHandoff)
        }
    }
}

#[cfg(target_os = "linux")]
fn persist_heartbeat_copy_anchored(
    state_root: &Path,
    repo: &fs::File,
    snapshot: &StartupHeartbeatSnapshot,
) -> Result<(std::path::PathBuf, fs::File, String), CommandFailure> {
    use nix::fcntl::{openat, OFlag};
    use nix::sys::stat::{fchmod, Mode};

    let quarantine = ensure_receipt_directory(repo, "quarantine")?;
    let copies = ensure_receipt_directory(&quarantine, "startup-heartbeats")?;
    let filename = format!(
        "{}-{}.json",
        snapshot.evidence.issue,
        heartbeat_session_key(&snapshot.evidence.nonce)
    );
    let descriptor = openat(
        &copies,
        filename.as_str(),
        OFlag::O_WRONLY | OFlag::O_CREAT | OFlag::O_EXCL | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
        Mode::from_bits_truncate(0o600),
    )
    .map_err(|error| {
        CommandFailure::diagnostic(format!("could not create heartbeat copy: {error}"))
    })?;
    fchmod(&descriptor, Mode::from_bits_truncate(0o600)).map_err(|error| {
        CommandFailure::diagnostic(format!("could not protect heartbeat copy: {error}"))
    })?;
    let mut file = fs::File::from(descriptor);
    file.write_all(&snapshot.file.document)
        .and_then(|()| file.sync_all())
        .map_err(|error| {
            CommandFailure::diagnostic(format!("could not persist heartbeat copy: {error}"))
        })?;
    nix::unistd::fsync(&copies).map_err(|error| {
        CommandFailure::diagnostic(format!("could not sync heartbeat copies: {error}"))
    })?;
    Ok((
        state_root
            .join("quarantine/startup-heartbeats")
            .join(&filename),
        copies,
        filename,
    ))
}

#[cfg(target_os = "linux")]
#[allow(dead_code)]
fn handoff_stale_heartbeat(
    state_root: &Path,
    repo: &fs::File,
    live_name: &std::ffi::OsStr,
    snapshot: &StartupHeartbeatSnapshot,
    mut after_move: impl FnMut(&fs::File, &fs::File, &str),
    mut after_sync: impl FnMut(HeartbeatHandoffSyncBoundary) -> Result<(), CommandFailure>,
    mut cleanup_sync: impl FnMut(&fs::File) -> Result<(), CommandFailure>,
) -> Result<std::path::PathBuf, CommandFailure> {
    use nix::fcntl::{renameat2, AtFlags, RenameFlags};
    use nix::sys::stat::fstatat;
    use nix::unistd::{fsync, linkat, unlinkat, UnlinkatFlags};

    let (copy, copies, copy_name) = persist_heartbeat_copy_anchored(state_root, repo, snapshot)?;
    let issue = snapshot.evidence.issue.parse::<u64>().map_err(|_| {
        CommandFailure::diagnostic("startup heartbeat issue is not an unsigned integer")
    })?;
    let expected = StartupHeartbeatExpectation {
        repo: &snapshot.evidence.repo,
        issue,
        worker_id: &snapshot.evidence.worker_id,
        branch: &snapshot.evidence.branch,
        pull_request: &snapshot.evidence.pr,
        claim_id: &snapshot.evidence.claim_id,
        step: &snapshot.evidence.step,
    };
    let transaction = begin_heartbeat_receipt(repo, expected)?;
    let handoff = &transaction.handoff;
    let observed = read_regular_file_at_no_follow(repo, live_name).map_err(|error| {
        CommandFailure::diagnostic(format!("could not revalidate startup heartbeat: {error}"))
    })?;
    revalidate_heartbeat_snapshot(&observed, &snapshot.file)?;
    let moved_name = format!(
        "{}-{}-{}.json",
        snapshot.evidence.issue,
        heartbeat_session_key(&snapshot.evidence.nonce),
        unique_operation_id("handoff")?
    );
    renameat2(
        repo,
        live_name,
        handoff,
        moved_name.as_str(),
        RenameFlags::RENAME_NOREPLACE,
    )
    .map_err(|error| CommandFailure::diagnostic(format!("could not move heartbeat: {error}")))?;
    let source_sync = fsync(repo)
        .map_err(|error| {
            CommandFailure::diagnostic(format!("could not sync heartbeat source: {error}"))
        })
        .and_then(|()| after_sync(HeartbeatHandoffSyncBoundary::Source));
    if let Err(error) = source_sync {
        restore_moved_heartbeat(repo, live_name, handoff, &moved_name, &mut after_sync)?;
        return Err(error);
    }
    let handoff_sync = fsync(handoff)
        .map_err(|error| {
            CommandFailure::diagnostic(format!("could not sync heartbeat handoff: {error}"))
        })
        .and_then(|()| after_sync(HeartbeatHandoffSyncBoundary::Handoff));
    if let Err(error) = handoff_sync {
        restore_moved_heartbeat(repo, live_name, handoff, &moved_name, &mut after_sync)?;
        return Err(error);
    }
    after_move(repo, handoff, &moved_name);
    let validation = read_regular_file_at_no_follow(handoff, moved_name.as_ref())
        .map_err(|error| CommandFailure::diagnostic(format!("moved heartbeat: {error}")))
        .and_then(|moved| revalidate_heartbeat_snapshot(&moved, &snapshot.file));
    if let Err(error) = validation {
        restore_moved_heartbeat(repo, live_name, handoff, &moved_name, &mut after_sync)?;
        return Err(error);
    }
    if !matches!(
        fstatat(repo, live_name, AtFlags::AT_SYMLINK_NOFOLLOW),
        Err(nix::errno::Errno::ENOENT)
    ) {
        restore_moved_heartbeat(repo, live_name, handoff, &moved_name, &mut after_sync)?;
        return Err(CommandFailure::diagnostic(
            "startup heartbeat live name reappeared",
        ));
    }
    unlinkat(handoff, moved_name.as_str(), UnlinkatFlags::NoRemoveDir).map_err(|error| {
        CommandFailure::diagnostic(format!("could not remove moved heartbeat: {error}"))
    })?;
    let cleanup =
        cleanup_sync(handoff).and_then(|()| after_sync(HeartbeatHandoffSyncBoundary::Cleanup));
    if let Err(error) = cleanup {
        match linkat(
            &copies,
            copy_name.as_str(),
            handoff,
            moved_name.as_str(),
            AtFlags::empty(),
        ) {
            Ok(()) | Err(nix::errno::Errno::EEXIST) => {}
            Err(restore) => {
                return Err(CommandFailure::diagnostic(format!(
                    "{error}; could not restore cleanup identity: {restore}"
                )))
            }
        }
        fsync(handoff).map_err(|restore| {
            CommandFailure::diagnostic(format!("{error}; restored identity sync: {restore}"))
        })?;
        return Err(error);
    }
    retire_heartbeat_receipt_with_sync(transaction, |directory| {
        fsync(directory)
            .map_err(|error| CommandFailure::diagnostic(format!("receipt sync: {error}")))
    })?;
    Ok(copy)
}

#[cfg(target_os = "linux")]
fn held_heartbeat_at(
    directory: &fs::File,
    name: &std::ffi::OsStr,
    expected: &StartupHeartbeatSnapshot,
) -> Result<Option<fs::File>, CommandFailure> {
    use nix::fcntl::{openat, OFlag};
    use nix::sys::stat::{fstat, Mode, SFlag};
    let descriptor = match openat(
        directory,
        name,
        OFlag::O_RDONLY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC | OFlag::O_NONBLOCK,
        Mode::empty(),
    ) {
        Err(nix::errno::Errno::ENOENT) => return Ok(None),
        Ok(descriptor) => descriptor,
        Err(_) => return Err(CommandFailure::diagnostic("heartbeat entry is unsafe")),
    };
    let file = fs::File::from(descriptor);
    let stat = fstat(&file)
        .map_err(|error| CommandFailure::diagnostic(format!("heartbeat fstat: {error}")))?;
    if SFlag::from_bits_truncate(stat.st_mode) & SFlag::S_IFMT != SFlag::S_IFREG
        || stat.st_uid != nix::unistd::geteuid().as_raw()
        || stat.st_mode & 0o7777 != 0o600
        || stat.st_nlink != 1
    {
        return Err(CommandFailure::diagnostic("heartbeat entry is unsafe"));
    }
    let observed = file
        .try_clone()
        .and_then(read_regular_file)
        .map_err(|error| CommandFailure::diagnostic(format!("heartbeat read: {error}")))?;
    revalidate_heartbeat_snapshot(&observed, &expected.file)?;
    if parse_startup_heartbeat(&observed.document).as_ref() != Some(&expected.evidence)
        || heartbeat_final_binding(
            &file,
            directory,
            name.to_string_lossy().as_ref(),
            (observed.identity.device, observed.identity.inode),
        )? != (HeartbeatFinalBinding::Exact, 1)
    {
        return Err(CommandFailure::diagnostic("heartbeat identity changed"));
    }
    Ok(Some(file))
}

#[cfg(target_os = "linux")]
fn with_retained_bridge_predecessor_authority<T>(
    identity: ClaimMutationIdentity<'_>,
    observe_pid: impl FnOnce(&str, u32, &str, &str, &str) -> StartupPidLiveness,
    boundary: impl FnOnce(),
    operation: impl FnOnce() -> Result<T, CommandFailure>,
) -> Result<Option<T>, CommandFailure> {
    use nix::fcntl::{open, AtFlags, OFlag};
    use nix::sys::stat::{fstatat, Mode};

    let root_path = heartbeat_root()?;
    let root = match open(
        &root_path,
        OFlag::O_PATH | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
        Mode::empty(),
    ) {
        Err(nix::errno::Errno::ENOENT) => return Ok(None),
        Err(error) => {
            return Err(CommandFailure::diagnostic(format!(
                "retained bridge heartbeat root open: {error}"
            )))
        }
        Ok(root) => fs::File::from(root),
    };
    private_heartbeat_directory_identity(&root, "retained bridge root")?;
    let repo_name = super::autonomous::drain::repository_progress_key(identity.repo);
    match fstatat(&root, repo_name.as_str(), AtFlags::AT_SYMLINK_NOFOLLOW) {
        Err(nix::errno::Errno::ENOENT) => return Ok(None),
        Err(error) => {
            return Err(CommandFailure::diagnostic(format!(
                "retained bridge repository inspect: {error}"
            )))
        }
        Ok(_) => {}
    }
    let repo = open_heartbeat_directory_beneath(&root, Path::new(&repo_name))?;
    let expected = StartupHeartbeatExpectation {
        repo: identity.repo,
        issue: identity.issue,
        worker_id: identity.worker_id,
        branch: identity.branch,
        pull_request: "",
        claim_id: identity.claim_id,
        step: "claimed",
    };
    match heartbeat_receipt_retry_decision(&repo, expected) {
        HeartbeatReceiptDecision::Absent => return Ok(None),
        HeartbeatReceiptDecision::Completed => {}
        HeartbeatReceiptDecision::Pending | HeartbeatReceiptDecision::Blocking => {
            return Err(CommandFailure::diagnostic(
                "retained bridge predecessor receipt is incomplete or unsafe",
            ))
        }
    }
    let (pending_name, receipt_name) = heartbeat_receipt_names(expected);
    let archive_name = receipt_name.replace(".receipt", ".json");
    let handoff = open_receipt_anchors_with_hook(&repo, |_| {})?;
    let handoff_identity = private_heartbeat_directory_identity(&handoff, "bridge handoff")?;
    let receipt_snapshot = read_regular_file_at_no_follow(&handoff, receipt_name.as_ref())
        .map_err(|error| CommandFailure::diagnostic(format!("bridge receipt read: {error}")))?;
    if inspect_heartbeat_receipt(&handoff, receipt_name.as_ref()) != HeartbeatReceiptEntry::Exact {
        return Err(CommandFailure::diagnostic(
            "retained bridge predecessor receipt is unsafe",
        ));
    }
    let heartbeat = match classify_startup_heartbeat_at(
        &handoff,
        archive_name.as_ref(),
        expected,
        unix_now()?,
        observe_pid,
    ) {
        StartupHeartbeatClassification::ExpiredDead(snapshot) => snapshot,
        StartupHeartbeatClassification::Absent => {
            return Err(CommandFailure::diagnostic(
                "retained bridge predecessor heartbeat is absent",
            ))
        }
        StartupHeartbeatClassification::Blocking => {
            return Err(CommandFailure::diagnostic(
                "retained bridge predecessor heartbeat is unsafe",
            ))
        }
    };
    let heartbeat = *heartbeat;
    let validate = || -> Result<(), CommandFailure> {
        if heartbeat_receipt_retry_decision(&repo, expected) != HeartbeatReceiptDecision::Completed
        {
            return Err(CommandFailure::diagnostic(
                "retained bridge predecessor receipt changed",
            ));
        }
        let handoff = open_receipt_anchors_with_hook(&repo, |_| {})?;
        let observed_receipt = read_regular_file_at_no_follow(&handoff, receipt_name.as_ref())
            .map_err(|error| {
                CommandFailure::diagnostic(format!("bridge receipt re-read: {error}"))
            })?;
        if private_heartbeat_directory_identity(&handoff, "bridge handoff")? != handoff_identity
            || observed_receipt != receipt_snapshot
            || inspect_heartbeat_receipt(&handoff, pending_name.as_ref())
                != HeartbeatReceiptEntry::Missing
            || inspect_heartbeat_receipt(&handoff, receipt_name.as_ref())
                != HeartbeatReceiptEntry::Exact
        {
            return Err(CommandFailure::diagnostic(
                "retained bridge predecessor receipt is unsafe",
            ));
        }
        let retained =
            held_heartbeat_at(&handoff, archive_name.as_ref(), &heartbeat)?.ok_or_else(|| {
                CommandFailure::diagnostic("retained bridge predecessor heartbeat disappeared")
            })?;
        let synthetic_live_name = format!(".{receipt_name}.bridge-authority");
        validate_retained_heartbeat(
            &root_path,
            &repo,
            synthetic_live_name.as_ref(),
            retained,
            &heartbeat,
            archive_name.clone(),
            handoff,
            None,
            &mut |_| Ok(()),
        )?;
        Ok(())
    };
    validate()?;
    boundary();
    validate()?;
    operation().map(Some)
}

#[cfg(target_os = "linux")]
#[allow(dead_code)]
fn handoff_retained_heartbeat(
    state_root: &Path,
    repo: &fs::File,
    live_name: &std::ffi::OsStr,
    snapshot: &StartupHeartbeatSnapshot,
    mut boundary: impl FnMut(&str, &fs::File, &fs::File, &str),
    mut after_sync: impl FnMut(HeartbeatHandoffSyncBoundary) -> Result<(), CommandFailure>,
) -> Result<std::path::PathBuf, CommandFailure> {
    use nix::fcntl::RenameFlags;
    use nix::unistd::fsync;

    let issue = snapshot
        .evidence
        .issue
        .parse::<u64>()
        .map_err(|_| CommandFailure::diagnostic("heartbeat issue is not an integer"))?;
    let expected = StartupHeartbeatExpectation {
        repo: &snapshot.evidence.repo,
        issue,
        worker_id: &snapshot.evidence.worker_id,
        branch: &snapshot.evidence.branch,
        pull_request: &snapshot.evidence.pr,
        claim_id: &snapshot.evidence.claim_id,
        step: &snapshot.evidence.step,
    };
    let (pending, completed) = heartbeat_receipt_names(expected);
    let archive_name = completed.replace(".receipt", ".json");
    let quarantine = ensure_receipt_directory(repo, "quarantine")?;
    ensure_receipt_directory(&quarantine, "startup-heartbeat-handoffs")?;
    let decision = heartbeat_receipt_retry_decision(repo, expected);
    let handoff = match decision {
        HeartbeatReceiptDecision::Absent => None,
        HeartbeatReceiptDecision::Pending | HeartbeatReceiptDecision::Completed => {
            Some(open_receipt_anchors_with_hook(repo, |_| {})?)
        }
        HeartbeatReceiptDecision::Blocking => {
            return Err(CommandFailure::diagnostic("heartbeat receipt is unsafe"))
        }
    };
    let source = held_heartbeat_at(repo, live_name, snapshot)?;
    let archived = handoff
        .as_ref()
        .map(|directory| held_heartbeat_at(directory, archive_name.as_ref(), snapshot))
        .transpose()?
        .flatten();
    if decision == HeartbeatReceiptDecision::Completed && (source.is_some() || archived.is_none()) {
        return Err(CommandFailure::diagnostic("completed handoff inconsistent"));
    }
    let transaction = match decision {
        HeartbeatReceiptDecision::Absent if source.is_some() && archived.is_none() => {
            begin_heartbeat_receipt(repo, expected)?
        }
        HeartbeatReceiptDecision::Pending
            if (source.is_some() && archived.is_none())
                || (source.is_none() && archived.is_some()) =>
        {
            HeartbeatReceiptTransaction {
                handoff: handoff.expect("pending receipt has handoff"),
                pending,
                completed,
            }
        }
        HeartbeatReceiptDecision::Completed => {
            let retained = archived.expect("validated completed archive");
            let handoff = handoff.expect("completed receipt has handoff");
            return validate_retained_heartbeat(
                state_root,
                repo,
                live_name,
                retained,
                snapshot,
                archive_name,
                handoff,
                None,
                &mut after_sync,
            );
        }
        _ => {
            return Err(CommandFailure::diagnostic(
                "heartbeat handoff state is ambiguous",
            ))
        }
    };
    let handoff = &transaction.handoff;
    let retained = if let Some(source) = source {
        boundary("before-check", repo, handoff, &archive_name);
        if heartbeat_final_binding(
            &source,
            repo,
            live_name.to_string_lossy().as_ref(),
            (snapshot.file.identity.device, snapshot.file.identity.inode),
        )? != (HeartbeatFinalBinding::Exact, 1)
        {
            return Err(CommandFailure::diagnostic("heartbeat source changed"));
        }
        boundary("before-rename", repo, handoff, &archive_name);
        nix::fcntl::renameat2(
            repo,
            live_name,
            handoff,
            archive_name.as_str(),
            RenameFlags::RENAME_NOREPLACE,
        )
        .map_err(|error| {
            CommandFailure::diagnostic(format!("could not retain heartbeat: {error}"))
        })?;
        if heartbeat_final_binding(
            &source,
            handoff,
            &archive_name,
            (snapshot.file.identity.device, snapshot.file.identity.inode),
        )? != (HeartbeatFinalBinding::Exact, 1)
        {
            restore_moved_heartbeat(repo, live_name, handoff, &archive_name, &mut after_sync)?;
            return Err(CommandFailure::diagnostic(
                "renamed heartbeat was not the held inode",
            ));
        }
        boundary("after-rename", repo, handoff, &archive_name);
        source
    } else {
        archived.expect("pending receipt has source or archive")
    };
    fsync(repo).map_err(|error| CommandFailure::diagnostic(format!("source fsync: {error}")))?;
    after_sync(HeartbeatHandoffSyncBoundary::Source)?;
    fsync(handoff)
        .map_err(|error| CommandFailure::diagnostic(format!("archive fsync: {error}")))?;
    after_sync(HeartbeatHandoffSyncBoundary::Handoff)?;
    boundary("final", repo, handoff, &archive_name);
    let handoff_guard = handoff
        .try_clone()
        .map_err(|error| CommandFailure::diagnostic(format!("retained archive clone: {error}")))?;
    validate_retained_heartbeat(
        state_root,
        repo,
        live_name,
        retained,
        snapshot,
        archive_name,
        handoff_guard,
        Some(transaction),
        &mut after_sync,
    )
}

#[cfg(target_os = "linux")]
#[allow(clippy::too_many_arguments)]
fn validate_retained_heartbeat(
    state_root: &Path,
    repo: &fs::File,
    live_name: &std::ffi::OsStr,
    retained: fs::File,
    snapshot: &StartupHeartbeatSnapshot,
    archive_name: String,
    handoff: fs::File,
    transaction: Option<HeartbeatReceiptTransaction>,
    after_sync: &mut impl FnMut(HeartbeatHandoffSyncBoundary) -> Result<(), CommandFailure>,
) -> Result<std::path::PathBuf, CommandFailure> {
    use nix::fcntl::AtFlags;
    use nix::sys::stat::{fstat, fstatat};
    if transaction.is_some() {
        after_sync(HeartbeatHandoffSyncBoundary::Cleanup)?;
    }
    let mut reader = retained.try_clone().map_err(|error| {
        CommandFailure::diagnostic(format!("retained heartbeat clone: {error}"))
    })?;
    std::io::Seek::rewind(&mut reader)
        .map_err(|error| CommandFailure::diagnostic(format!("heartbeat rewind: {error}")))?;
    let observed = read_regular_file(reader)
        .map_err(|error| CommandFailure::diagnostic(format!("retained heartbeat read: {error}")))?;
    let stat = fstat(&retained)
        .map_err(|error| CommandFailure::diagnostic(format!("retained fstat: {error}")))?;
    if !matches!(
        fstatat(repo, live_name, AtFlags::AT_SYMLINK_NOFOLLOW),
        Err(nix::errno::Errno::ENOENT)
    ) || stat.st_uid != nix::unistd::geteuid().as_raw()
        || stat.st_mode & 0o7777 != 0o600
        || stat.st_nlink != 1
        || observed != snapshot.file
        || heartbeat_final_binding(
            &retained,
            &handoff,
            &archive_name,
            (snapshot.file.identity.device, snapshot.file.identity.inode),
        )? != (HeartbeatFinalBinding::Exact, 1)
    {
        return Err(CommandFailure::diagnostic(
            "retained heartbeat final validation failed",
        ));
    }
    if let Some(transaction) = transaction {
        retire_heartbeat_receipt_with_sync(transaction, |directory| {
            nix::unistd::fsync(directory)
                .map_err(|error| CommandFailure::diagnostic(format!("receipt fsync: {error}")))
        })?;
    }
    Ok(state_root
        .join("quarantine/startup-heartbeat-handoffs")
        .join(archive_name))
}

#[cfg(unix)]
#[allow(dead_code)]
fn handoff_stale_heartbeat_path(
    state_root: &Path,
    source: &Path,
    snapshot: &StartupHeartbeatSnapshot,
) -> Result<std::path::PathBuf, CommandFailure> {
    handoff_stale_heartbeat_path_with_hooks(
        state_root,
        source,
        snapshot,
        || {},
        |_, _, _| {},
        |_| Ok(()),
    )
}

#[cfg(unix)]
#[allow(dead_code)]
fn handoff_stale_heartbeat_path_with_hooks(
    state_root: &Path,
    source: &Path,
    snapshot: &StartupHeartbeatSnapshot,
    mut before_move: impl FnMut(),
    mut after_move: impl FnMut(&std::os::fd::OwnedFd, &std::os::fd::OwnedFd, &str),
    mut after_sync: impl FnMut(HeartbeatHandoffSyncBoundary) -> Result<(), CommandFailure>,
) -> Result<std::path::PathBuf, CommandFailure> {
    use nix::fcntl::{open, AtFlags, OFlag};
    use nix::sys::stat::{fstat, fstatat, Mode};
    use nix::unistd::{fsync, unlinkat, UnlinkatFlags};

    let live_name = source
        .file_name()
        .ok_or_else(|| CommandFailure::diagnostic("startup heartbeat source has no filename"))?;
    if source.parent() != Some(state_root) {
        return Err(CommandFailure::diagnostic(
            "startup heartbeat source is outside the private state root",
        ));
    }
    let copy = persist_heartbeat_copy(state_root, source, snapshot)?;
    let flags = OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC;
    let root = open(state_root, flags, Mode::empty()).map_err(|_| {
        CommandFailure::diagnostic("heartbeat private state root is not a safe directory")
    })?;
    let stat = fstat(&root).map_err(|error| {
        CommandFailure::diagnostic(format!("could not inspect private state root: {error}"))
    })?;
    if stat.st_uid != nix::unistd::geteuid().as_raw() || stat.st_mode & 0o7777 != 0o700 {
        return Err(CommandFailure::diagnostic(
            "heartbeat private state root must be owned by the effective user with mode 0700",
        ));
    }
    fn ignore_component(_: &str) {}
    let quarantine = open_private_quarantine_component(&root, "quarantine", &mut ignore_component)?;
    let handoff = open_private_quarantine_component(
        &quarantine,
        "startup-heartbeat-handoffs",
        &mut ignore_component,
    )?;
    let observed = read_regular_file_at_no_follow(&root, live_name).map_err(|error| {
        CommandFailure::diagnostic(format!("could not revalidate startup heartbeat: {error}"))
    })?;
    revalidate_heartbeat_snapshot(&observed, &snapshot.file)?;
    let moved_name = format!(
        "{}-{}-{}.json",
        snapshot.evidence.issue,
        heartbeat_session_key(&snapshot.evidence.nonce),
        unique_operation_id("handoff")?
    );
    before_move();
    rename_heartbeat_no_clobber(&root, live_name, &handoff, &moved_name)?;
    fsync(&root).map_err(|error| {
        CommandFailure::diagnostic(format!(
            "could not sync heartbeat source directory: {error}"
        ))
    })?;
    after_sync(HeartbeatHandoffSyncBoundary::Source)?;
    fsync(&handoff).map_err(|error| {
        CommandFailure::diagnostic(format!(
            "could not sync heartbeat handoff directory: {error}"
        ))
    })?;
    after_sync(HeartbeatHandoffSyncBoundary::Handoff)?;
    after_move(&root, &handoff, &moved_name);
    let moved = read_regular_file_at_no_follow(&handoff, std::ffi::OsStr::new(&moved_name))
        .map_err(|error| {
            CommandFailure::diagnostic(format!("could not validate moved heartbeat: {error}"))
        });
    let validation = moved.and_then(|moved| {
        revalidate_heartbeat_snapshot(&moved, &snapshot.file)?;
        if parse_startup_heartbeat(&moved.document).as_ref() != Some(&snapshot.evidence) {
            return Err(CommandFailure::diagnostic(
                "startup heartbeat JSON drift after handoff",
            ));
        }
        Ok(())
    });
    if let Err(error) = validation {
        restore_moved_heartbeat(&root, live_name, &handoff, &moved_name, &mut after_sync)?;
        return Err(error);
    }
    match fstatat(&root, live_name, AtFlags::AT_SYMLINK_NOFOLLOW) {
        Err(nix::errno::Errno::ENOENT) => {}
        _ => {
            restore_moved_heartbeat(&root, live_name, &handoff, &moved_name, &mut after_sync)?;
            return Err(CommandFailure::diagnostic(
                "startup heartbeat live name reappeared during handoff",
            ));
        }
    }
    unlinkat(&handoff, moved_name.as_str(), UnlinkatFlags::NoRemoveDir).map_err(|error| {
        CommandFailure::diagnostic(format!("could not remove moved startup heartbeat: {error}"))
    })?;
    after_sync(HeartbeatHandoffSyncBoundary::Cleanup)?;
    fsync(&handoff).map_err(|error| {
        CommandFailure::diagnostic(format!("could not sync heartbeat cleanup: {error}"))
    })?;
    Ok(copy)
}

#[cfg(target_os = "linux")]
#[allow(dead_code)]
fn observe_local_startup_pid(
    _worker_id: &str,
    pid: u32,
    host: &str,
    boot_id: &str,
    process_start: &str,
) -> StartupPidLiveness {
    let current_host = fs::read_to_string("/proc/sys/kernel/hostname")
        .ok()
        .map(|host| host.trim().to_string());
    let current_boot = super::autonomous::current_boot_identity().ok();
    if current_host.as_deref() != Some(host) || current_boot.as_deref() != Some(boot_id) {
        return StartupPidLiveness::Unknown;
    }
    match super::autonomous::process_birth_identity(pid) {
        Ok(None) => return StartupPidLiveness::Dead,
        Ok(Some((observed_boot, observed_start)))
            if observed_boot == boot_id && observed_start == process_start => {}
        Ok(Some(_)) | Err(_) => return StartupPidLiveness::Unknown,
    }
    match nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid as i32), None) {
        Ok(()) | Err(nix::errno::Errno::EPERM) => StartupPidLiveness::Live,
        Err(nix::errno::Errno::ESRCH) => StartupPidLiveness::Dead,
        Err(_) => StartupPidLiveness::Unknown,
    }
}

#[cfg(not(target_os = "linux"))]
#[allow(dead_code)]
fn observe_local_startup_pid(
    _worker_id: &str,
    pid: u32,
    host: &str,
    boot_id: &str,
    process_start: &str,
) -> StartupPidLiveness {
    let current_host = heartbeat_portable::hostname().ok();
    let current_boot = super::autonomous::current_boot_identity().ok();
    if current_host.as_deref() != Some(host) || current_boot.as_deref() != Some(boot_id) {
        return StartupPidLiveness::Unknown;
    }
    match super::autonomous::process_birth_identity(pid) {
        Ok(None) => StartupPidLiveness::Dead,
        Ok(Some((observed_boot, observed_start)))
            if observed_boot == boot_id && observed_start == process_start =>
        {
            StartupPidLiveness::Live
        }
        Ok(Some(_)) | Err(_) => StartupPidLiveness::Unknown,
    }
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

fn branch_blocks_stale_recovery(branch: &str, base_branch: Option<&str>) -> bool {
    if branch.trim().is_empty() {
        return false;
    }
    let local = format!("refs/heads/{branch}");
    match Command::new("git")
        .args(["show-ref", "--verify", "--quiet", &local])
        .status()
    {
        Ok(status) if status.success() => {
            let Some(base_branch) = base_branch else {
                return true;
            };
            if !local_branch_is_integrated_and_inactive(&local, base_branch) {
                return true;
            }
        }
        Ok(status) if status.code() == Some(1) => {}
        Ok(_) | Err(_) => return true,
    }
    for reference in [format!("refs/remotes/origin/{branch}")] {
        match Command::new("git")
            .args(["show-ref", "--verify", "--quiet", &reference])
            .status()
        {
            Ok(status) if status.success() => return true,
            Ok(status) if status.code() == Some(1) => {}
            Ok(_) | Err(_) => return true,
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

fn local_branch_is_integrated_and_inactive(reference: &str, base_branch: &str) -> bool {
    let worktrees = match Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .output()
    {
        Ok(output) if output.status.success() => output.stdout,
        Ok(_) | Err(_) => return false,
    };
    let checked_out = format!("branch {reference}");
    if String::from_utf8_lossy(&worktrees)
        .lines()
        .any(|line| line == checked_out)
    {
        return false;
    }
    let base = format!("refs/remotes/origin/{base_branch}");
    if !Command::new("git")
        .args(["show-ref", "--verify", "--quiet", &base])
        .status()
        .is_ok_and(|status| status.success())
    {
        return false;
    }
    Command::new("git")
        .args(["merge-base", "--is-ancestor", reference, &base])
        .status()
        .is_ok_and(|status| status.success())
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
        "autospec claim\n\nUSAGE:\n    autospec claim state read --issue <N> [--repo OWNER/REPO]\n    autospec claim state upsert --issue <N> --worker-id <ID> --claim-id <ID> --branch <NAME> --state <STATE> [OPTIONS]\n    autospec claim state refresh --issue <N> --worker-id <ID> --claim-id <ID> --branch <NAME> --step <STEP> [OPTIONS]\n    autospec claim acquire --issue <N> [--repo OWNER/REPO] [--worker-id ID] [--branch NAME] [--session-id ID]\n    autospec claim release --issue <N> --claim-id <ID> [--repo OWNER/REPO] [--state released|failed|merged]\n\nCOMMANDS:\n    state          Read and update GitHub-backed claim state\n    acquire        Validate issue eligibility before acquiring an issue lease\n    release        Release, fail, or terminally merge an issue lease"
    );
}

fn print_state_help() {
    println!(
        "autospec claim state\n\nUSAGE:\n    autospec claim state read --issue <N> [--repo OWNER/REPO]\n    autospec claim state upsert --issue <N> --worker-id <ID> --claim-id <ID> --branch <NAME> --state <STATE> [OPTIONS]\n    autospec claim state refresh --issue <N> --worker-id <ID> --claim-id <ID> --branch <NAME> --step <STEP> [OPTIONS]\n    autospec claim state clear --issue <N> --worker-id <ID> --claim-id <ID> --branch <NAME> [--repo OWNER/REPO]\n    autospec claim state reconcile-linked-pr --issue <N> --worker-id <ID> --claim-id <ID> --branch <NAME> [--repo OWNER/REPO]\n    autospec claim state recover-stale-startup --issue <N> [--repo OWNER/REPO] [--timeout-seconds 300]\n\nCOMMANDS:\n    read                   Read the authoritative parent-linked run-state generation\n    upsert                 Append one exact-owner successor generation\n    refresh                Renew one exact claim generation and confirm ownership\n    clear                  CAS the exact claim generation to released\n    reconcile-linked-pr    Record a linked PR for one exact claim generation\n    recover-stale-startup  CAS an evidenceless stale startup claim to available"
    );
}

mod heartbeat_liveness;
#[cfg(any(not(target_os = "linux"), test))]
mod heartbeat_portable;
mod heartbeat_predecessor;
pub(crate) mod lease;
mod open_beneath;
use heartbeat_liveness::startup_heartbeat_exists;
use lease::{
    claim_retry_attempts, claim_retry_sleep_ms, read_gh_with_retry, server_lease_is_fresh,
    server_lease_is_stale,
};
#[cfg(test)]
mod tests;
