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
    let output = Command::new("gh")
        .args(["repo", "view", repo, "--json", "url", "--jq", ".url"])
        .output()
        .map_err(|error| {
            CommandFailure::transient(format!("resolve authenticated claim repository: {error}"))
        })?;
    if !output.status.success() {
        return Err(CommandFailure::transient(format!(
            "resolve authenticated claim repository {repo}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
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
        if let Some(owner) = read_claim_ref(repo, issue)?.and_then(|head| {
            (head.record.state == "claimed"
                && (head.record.worker_id != worker_id || head.record.branch != branch))
                .then_some(head.record.worker_id)
        }) {
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
            "claimed" | "merged" | "released" | "failed" | "retryable" | "needs-human"
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
        if let Some(owner) = prior.as_ref().and_then(|head| {
            (!matches!(
                head.record.state.as_str(),
                "available" | "released" | "retryable"
            ) && (head.record.step.starts_with("heartbeat-publishing:")
                || !server_lease_is_stale(&head.record.updated_at, head.record.ttl_seconds)))
            .then_some(head.record.worker_id.as_str())
        }) {
            return unavailable_claim_with_observed_owner(options.issue, &repo, &worker_id, owner);
        }
        retire_released_predecessor_heartbeat(&repo, options.issue, prior.as_ref())?;
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

fn retire_released_predecessor_heartbeat(
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
    retire_released_startup_heartbeat_with_hook(identity, true, &mut |_, _| Ok(()))
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

#[cfg(not(target_os = "linux"))]
fn released_predecessor_heartbeat_evidence_exists(
    _identity: ClaimMutationIdentity<'_>,
) -> Result<bool, CommandFailure> {
    Err(CommandFailure::diagnostic(
        "released predecessor heartbeat recovery is unavailable on this platform",
    ))
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
        || branch_blocks_stale_recovery(&selected.record.branch, base_branch)
        || !server_lease_is_stale(&selected.record.updated_at, timeout_seconds)
    {
        return Ok(RecoveryOutcome {
            recovered: false,
            reason: "claim_evidence_or_fresh_state".to_string(),
        });
    }
    if !quarantine_authoritative_stale_heartbeat(repo, issue, &selected.record)? {
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
fn quarantine_authoritative_stale_heartbeat(
    repo_name: &str,
    issue: u64,
    record: &RunStateRecord,
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
                            if heartbeat_lifecycle_step(&record.step) =>
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
        .args(["api", endpoint.as_str(), "--paginate", "--slurp"])
        .output()
        .map_err(|error| {
            CommandFailure::transient(format!("could not run gh api issue comments: {error}"))
        })?;
    if !output.status.success() {
        return Err(CommandFailure::transient(format!(
            "gh api issue comments failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
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
        .map_err(|error| CommandFailure::transient(format!("could not run gh issue view: {error}")))?;
    if !output.status.success() {
        return Err(CommandFailure::transient(format!(
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
            "number,body,headRefName,headRefOid,isDraft,baseRefName",
        ])
        .output()
        .map_err(|error| CommandFailure::transient(format!("could not run gh pr list: {error}")))?;
    if !output.status.success() {
        return Err(CommandFailure::transient(format!(
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

fn list_required_checks(
    repo: &str,
    pull_request: u64,
) -> Result<Vec<autospec_core::claim::RequiredCheck>, CommandFailure> {
    let output = Command::new("gh")
        .args([
            "pr",
            "checks",
            &pull_request.to_string(),
            "--repo",
            repo,
            "--required",
            "--json",
            "name,state",
        ])
        .output()
        .map_err(|error| {
            CommandFailure::transient(format!("could not run gh pr checks: {error}"))
        })?;
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
    let root = heartbeat_root()?;
    let timestamp = unix_now()?;
    let ttl_seconds = claim_ttl_seconds().max(1);
    let pid = std::process::id();
    let nonce = startup_heartbeat_nonce(repo, issue, claim_id);
    let (host, boot_id, process_start) = startup_process_identity(pid)?;
    let session_field = session_id.map_or_else(String::new, |session_id| {
        format!(",\"session_id\":\"{}\"", json_escape(session_id))
    });
    let body = format!(
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
        &root,
        repo,
        issue,
        session_id,
        body.as_bytes(),
        &mut |_, _| Ok(()),
    );
    #[cfg(not(target_os = "linux"))]
    Err(CommandFailure::diagnostic(
        "heartbeat publisher unavailable",
    ))
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
fn startup_process_identity(_pid: u32) -> Result<(String, String, String), CommandFailure> {
    Err(CommandFailure::diagnostic(
        "heartbeat process identity requires Linux /proc",
    ))
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
    _identity: ClaimMutationIdentity<'_>,
) -> Result<(), CommandFailure> {
    Ok(())
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

#[allow(dead_code)]
fn open_heartbeat_directory_beneath(
    trusted_parent: &fs::File,
    descendant: &Path,
) -> Result<fs::File, CommandFailure> {
    open_heartbeat_directory_beneath_with_hook(trusted_parent, descendant, || {})
}

#[cfg(unix)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct HeartbeatDirectoryIdentity {
    device: u64,
    inode: u64,
    owner: u32,
    mode: u32,
}

#[cfg(unix)]
fn private_heartbeat_directory_identity(
    directory: &impl std::os::fd::AsFd,
    role: &str,
) -> Result<HeartbeatDirectoryIdentity, CommandFailure> {
    use nix::sys::stat::{fstat, SFlag};

    let stat = fstat(directory).map_err(|error| {
        CommandFailure::diagnostic(format!("could not inspect heartbeat {role}: {error}"))
    })?;
    if !SFlag::from_bits_truncate(stat.st_mode).contains(SFlag::S_IFDIR)
        || stat.st_uid != nix::unistd::geteuid().as_raw()
        || stat.st_mode & 0o7777 != 0o700
    {
        return Err(CommandFailure::diagnostic(format!(
            "heartbeat {role} must be owned by the effective user with mode 0700"
        )));
    }
    Ok(HeartbeatDirectoryIdentity {
        device: stat.st_dev as u64,
        inode: stat.st_ino as u64,
        owner: stat.st_uid as u32,
        mode: (stat.st_mode & 0o7777) as u32,
    })
}

#[cfg(unix)]
#[allow(dead_code)]
fn private_heartbeat_name_identity(
    parent: &impl std::os::fd::AsFd,
    name: &Path,
    role: &str,
) -> Result<HeartbeatDirectoryIdentity, CommandFailure> {
    use nix::fcntl::AtFlags;
    use nix::sys::stat::{fstatat, SFlag};

    let stat = fstatat(parent, name, AtFlags::AT_SYMLINK_NOFOLLOW).map_err(|error| {
        CommandFailure::diagnostic(format!("could not inspect heartbeat {role}: {error}"))
    })?;
    if !SFlag::from_bits_truncate(stat.st_mode).contains(SFlag::S_IFDIR)
        || stat.st_uid != nix::unistd::geteuid().as_raw()
        || stat.st_mode & 0o7777 != 0o700
    {
        return Err(CommandFailure::diagnostic(format!(
            "heartbeat {role} must be owned by the effective user with mode 0700"
        )));
    }
    Ok(HeartbeatDirectoryIdentity {
        device: stat.st_dev as u64,
        inode: stat.st_ino as u64,
        owner: stat.st_uid as u32,
        mode: (stat.st_mode & 0o7777) as u32,
    })
}

#[cfg(target_os = "linux")]
fn heartbeat_openat2_resolve_flags() -> nix::fcntl::ResolveFlag {
    use nix::fcntl::ResolveFlag;

    ResolveFlag::RESOLVE_BENEATH
        | ResolveFlag::RESOLVE_NO_SYMLINKS
        | ResolveFlag::RESOLVE_NO_MAGICLINKS
        | ResolveFlag::RESOLVE_NO_XDEV
}

#[cfg(target_os = "linux")]
#[allow(dead_code)]
fn open_heartbeat_directory_beneath_with_hook(
    trusted_parent: &fs::File,
    descendant: &Path,
    before_open: impl FnOnce(),
) -> Result<fs::File, CommandFailure> {
    use nix::fcntl::{openat2, OFlag, OpenHow};
    use std::path::Component;

    if descendant.as_os_str().is_empty()
        || descendant.is_absolute()
        || descendant
            .components()
            .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(CommandFailure::diagnostic(
            "heartbeat descendant must be relative, non-empty, and contain no '..'",
        ));
    }
    let parent_identity = private_heartbeat_directory_identity(trusted_parent, "parent")?;
    let expected_binding =
        private_heartbeat_name_identity(trusted_parent, descendant, "descendant binding")?;
    before_open();
    if private_heartbeat_directory_identity(trusted_parent, "parent")? != parent_identity {
        return Err(CommandFailure::diagnostic(
            "heartbeat parent descriptor identity drift before descendant open",
        ));
    }
    let how = OpenHow::new()
        .flags(OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC)
        .resolve(heartbeat_openat2_resolve_flags());
    let directory = openat2(trusted_parent, descendant, how).map_err(|error| {
        CommandFailure::diagnostic(format!(
            "heartbeat directory secure resolution failed: {error}"
        ))
    })?;
    if private_heartbeat_directory_identity(trusted_parent, "parent")? != parent_identity {
        return Err(CommandFailure::diagnostic(
            "heartbeat parent descriptor identity drift after descendant open",
        ));
    }
    let opened = private_heartbeat_directory_identity(&directory, "descendant")?;
    if opened != expected_binding
        || private_heartbeat_name_identity(trusted_parent, descendant, "descendant binding")?
            != opened
    {
        return Err(CommandFailure::diagnostic(
            "heartbeat descendant name binding changed during open",
        ));
    }
    Ok(fs::File::from(directory))
}

#[cfg(unix)]
#[allow(dead_code)]
fn open_heartbeat_directory_portable_unix_with_hook(
    trusted_parent: &fs::File,
    descendant: &Path,
    before_open: impl FnOnce(),
) -> Result<fs::File, CommandFailure> {
    use nix::fcntl::{openat, OFlag};
    use nix::sys::stat::Mode;
    use std::path::Component;

    if !matches!(
        descendant.components().collect::<Vec<_>>().as_slice(),
        [Component::Normal(_)]
    ) {
        return Err(CommandFailure::diagnostic(
            "portable heartbeat descendant must be exactly one normal component",
        ));
    }
    let parent_identity = private_heartbeat_directory_identity(trusted_parent, "parent")?;
    let expected_binding =
        private_heartbeat_name_identity(trusted_parent, descendant, "descendant binding")?;
    before_open();
    if private_heartbeat_directory_identity(trusted_parent, "parent")? != parent_identity {
        return Err(CommandFailure::diagnostic(
            "heartbeat parent descriptor identity drift before descendant open",
        ));
    }
    let directory = openat(
        trusted_parent,
        descendant,
        OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
        Mode::empty(),
    )
    .map_err(|error| {
        CommandFailure::diagnostic(format!(
            "portable heartbeat directory resolution failed: {error}"
        ))
    })?;
    let opened = private_heartbeat_directory_identity(&directory, "descendant")?;
    if opened != expected_binding
        || private_heartbeat_name_identity(trusted_parent, descendant, "descendant binding")?
            != opened
    {
        return Err(CommandFailure::diagnostic(
            "heartbeat descendant name binding changed during open",
        ));
    }
    if private_heartbeat_directory_identity(trusted_parent, "parent")? != parent_identity {
        return Err(CommandFailure::diagnostic(
            "heartbeat parent descriptor identity drift after descendant open",
        ));
    }
    Ok(fs::File::from(directory))
}

#[cfg(all(unix, not(target_os = "linux")))]
#[allow(dead_code)]
fn open_heartbeat_directory_beneath_with_hook(
    trusted_parent: &fs::File,
    descendant: &Path,
    before_open: impl FnOnce(),
) -> Result<fs::File, CommandFailure> {
    open_heartbeat_directory_portable_unix_with_hook(trusted_parent, descendant, before_open)
}

#[cfg(not(unix))]
#[allow(dead_code)]
fn open_heartbeat_directory_beneath_with_hook(
    _trusted_parent: &fs::File,
    _descendant: &Path,
    _before_open: impl FnOnce(),
) -> Result<fs::File, CommandFailure> {
    Err(CommandFailure::diagnostic(
        "secure heartbeat directory resolution requires Unix descriptor operations",
    ))
}

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

#[cfg(unix)]
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
    now: u64,
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
    let expired = now.saturating_sub(evidence.ts) > evidence.ttl_seconds && now >= evidence.ts;
    if !exact || !expired || !cfg!(unix) {
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

#[cfg(unix)]
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

#[cfg(not(unix))]
#[allow(dead_code)]
fn observe_local_startup_pid(
    _worker_id: &str,
    _pid: u32,
    _host: &str,
    _boot_id: &str,
    _process_start: &str,
) -> StartupPidLiveness {
    StartupPidLiveness::Unknown
}

fn startup_heartbeat_exists(repo: &str, issue: u64) -> bool {
    heartbeat_root().is_ok_and(|root| {
        root.join(super::autonomous::drain::repository_progress_key(repo))
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
        "autospec claim\n\nUSAGE:\n    autospec claim state read --issue <N> [--repo OWNER/REPO]\n    autospec claim state upsert --issue <N> --worker-id <ID> --claim-id <ID> --branch <NAME> --state <STATE> [OPTIONS]\n    autospec claim state refresh --issue <N> --worker-id <ID> --claim-id <ID> --branch <NAME> --step <STEP> [OPTIONS]\n    autospec claim acquire --issue <N> [--repo OWNER/REPO] [--worker-id ID] [--branch NAME] [--session-id ID]\n    autospec claim release --issue <N> --claim-id <ID> [--repo OWNER/REPO] [--state released|failed|merged]\n\nCOMMANDS:\n    state          Read and update GitHub-backed claim state\n    acquire        Validate issue eligibility before acquiring an issue lease\n    release        Release, fail, or terminally merge an issue lease"
    );
}

fn print_state_help() {
    println!(
        "autospec claim state\n\nUSAGE:\n    autospec claim state read --issue <N> [--repo OWNER/REPO]\n    autospec claim state upsert --issue <N> --worker-id <ID> --claim-id <ID> --branch <NAME> --state <STATE> [OPTIONS]\n    autospec claim state refresh --issue <N> --worker-id <ID> --claim-id <ID> --branch <NAME> --step <STEP> [OPTIONS]\n    autospec claim state clear --issue <N> --worker-id <ID> --claim-id <ID> --branch <NAME> [--repo OWNER/REPO]\n    autospec claim state reconcile-linked-pr --issue <N> --worker-id <ID> --claim-id <ID> --branch <NAME> [--repo OWNER/REPO]\n    autospec claim state recover-stale-startup --issue <N> [--repo OWNER/REPO] [--timeout-seconds 300]\n\nCOMMANDS:\n    read                   Read the authoritative parent-linked run-state generation\n    upsert                 Append one exact-owner successor generation\n    refresh                Renew one exact claim generation and confirm ownership\n    clear                  CAS the exact claim generation to released\n    reconcile-linked-pr    Record a linked PR for one exact claim generation\n    recover-stale-startup  CAS an evidenceless stale startup claim to available"
    );
}

#[cfg(test)]
mod tests {
    use super::{
        advance_claim_ref_in, claim_settle_millis, create_claim_ref_commit,
        lifecycle_claim_evidence_from_record, private_claim_git_dir_in, publish_session_binding,
        read_claim_ref_in, validated_claim_remote, ClaimRefAdvance, SessionBindingIdentity,
    };
    use autospec_core::autonomous_lifecycle::{
        ClaimBranch, ClaimContext, ClaimEvidence, IssueNumber, LeaseFreshness, RepositoryScope,
        WorkerId,
    };
    use autospec_core::claim::{ExecutorResultEvidence, RemoteComment, RunStateRecord};
    #[cfg(unix)]
    use std::io::Write;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::{Arc, Barrier, Mutex};

    static BRIDGE_TRANSITION_ENV: Mutex<()> = Mutex::new(());
    static STARTUP_HEARTBEAT_ENV: Mutex<()> = Mutex::new(());

    fn startup_heartbeat_fixture(label: &str) -> (PathBuf, PathBuf) {
        let directory = std::env::temp_dir().join(format!(
            "autospec-startup-heartbeat-{label}-{}-{}",
            std::process::id(),
            super::UNIQUE_ID_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&directory).expect("startup heartbeat fixture");
        #[cfg(unix)]
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))
            .expect("private startup heartbeat fixture");
        let path = directory.join("42.json");
        (directory, path)
    }

    #[cfg(unix)]
    fn assert_fifo_reader_nonblocking(
        fifo: &Path,
        reader: impl FnOnce() -> std::io::Result<super::RegularFileSnapshot> + Send + 'static,
    ) {
        let (send, receive) = std::sync::mpsc::channel();
        let reader = std::thread::spawn(move || {
            let _ = send.send(reader());
        });
        match receive.recv_timeout(std::time::Duration::from_secs(2)) {
            Ok(result) => assert!(result.is_err(), "FIFO was accepted as a regular file"),
            Err(error) => {
                if let Ok(writer) = nix::fcntl::open(
                    fifo,
                    nix::fcntl::OFlag::O_WRONLY | nix::fcntl::OFlag::O_NONBLOCK,
                    nix::sys::stat::Mode::empty(),
                ) {
                    drop(writer);
                }
                if receive
                    .recv_timeout(std::time::Duration::from_secs(2))
                    .is_ok()
                {
                    let _ = reader.join();
                }
                panic!("heartbeat FIFO reader blocked: {error}");
            }
        }
        reader.join().unwrap();
    }

    #[cfg(target_os = "linux")]
    fn anchored_startup_heartbeat_fixture(
        label: &str,
    ) -> (
        PathBuf,
        PathBuf,
        PathBuf,
        std::fs::File,
        Box<super::StartupHeartbeatSnapshot>,
    ) {
        use nix::fcntl::{open, OFlag};
        use nix::sys::stat::Mode;

        let (parent_path, _) = startup_heartbeat_fixture(label);
        let repo_path = parent_path.join("repo");
        std::fs::create_dir(&repo_path).unwrap();
        std::fs::set_permissions(&repo_path, std::fs::Permissions::from_mode(0o700)).unwrap();
        let source = repo_path.join("42.json");
        std::fs::write(
            &source,
            startup_heartbeat_document("host:user:rust:4242:nonce-a", 0),
        )
        .unwrap();
        std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o600)).unwrap();
        let snapshot = expired_heartbeat_snapshot(&source);
        let parent = std::fs::File::from(
            open(
                &parent_path,
                OFlag::O_PATH | OFlag::O_DIRECTORY,
                Mode::empty(),
            )
            .unwrap(),
        );
        let repo = super::open_heartbeat_directory_beneath(&parent, Path::new("repo")).unwrap();
        (parent_path, repo_path, source, repo, snapshot)
    }

    fn startup_heartbeat_document(worker: &str, pid: u32) -> String {
        let nonce = super::startup_heartbeat_nonce("owner/repo", 42, "claim-a");
        format!(
            r#"{{"repo":"owner/repo","issue":"42","worker_id":"{worker}","branch":"feat/worker","pr":"","claim_id":"claim-a","step":"claimed","ts":100,"ttl_seconds":10,"pid":{pid},"nonce":"{nonce}","host":"host-a","boot_id":"boot-a","process_start":"1"}}"#
        )
    }

    fn expected_startup_heartbeat<'a>(
        worker_id: &'a str,
    ) -> super::StartupHeartbeatExpectation<'a> {
        super::StartupHeartbeatExpectation {
            repo: "owner/repo",
            issue: 42,
            worker_id,
            branch: "feat/worker",
            pull_request: "",
            claim_id: "claim-a",
            step: "claimed",
        }
    }

    fn inject_heartbeat_boundary(
        observed: &str,
        target: &str,
        message: &str,
    ) -> Result<(), super::CommandFailure> {
        if observed == target {
            Err(super::CommandFailure::diagnostic(message))
        } else {
            Ok(())
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn startup_heartbeat_atomic_publication() {
        use super::HeartbeatPublicationDurability::{Durable, Unconfirmed};
        use super::HeartbeatPublicationFailure::{PostCommit, PreCommit};
        use nix::fcntl::{open, OFlag};
        use nix::sys::stat::{umask, Mode};
        use std::io::Read;
        use std::os::unix::fs::{MetadataExt, PermissionsExt};

        const CHILD: &str = "AUTOSPEC_TEST_ATOMIC_HEARTBEAT_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let output = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "commands::claim::tests::startup_heartbeat_atomic_publication",
                    "--nocapture",
                ])
                .env(CHILD, "1")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }

        let (fixture, issue) = startup_heartbeat_fixture("atomic-publication");
        let directory = open(
            &fixture,
            OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW,
            Mode::empty(),
        )
        .unwrap();
        let previous = umask(Mode::from_bits_truncate(0o777));
        let attempt = |name: &str, failed: Option<&str>| {
            super::publish_private_heartbeat_file(
                &directory,
                name,
                b"heartbeat\n",
                "test",
                &mut |_, boundary| {
                    if failed == Some("hardlink") && boundary == "revalidate" {
                        let peer = fixture.join(format!("{name}.peer"));
                        std::fs::hard_link(fixture.join(name), peer).unwrap();
                        Ok(())
                    } else if failed == Some(boundary) {
                        Err(super::CommandFailure::diagnostic(
                            "injected persistence failure",
                        ))
                    } else {
                        Ok(())
                    }
                },
            )
        };
        for name in ["", ".", "..", "/tmp/x", "a/b", "a/", "./a"] {
            assert!(matches!(attempt(name, None), Err(PreCommit(_))));
            assert_eq!(std::fs::read_dir(&fixture).unwrap().count(), 0);
        }
        for failed in ["chmod", "write", "file-fsync", "before-link"] {
            assert!(matches!(
                attempt("42.json", Some(failed)),
                Err(PreCommit(_))
            ));
            assert!(!issue.exists());
        }
        let outside = fixture.join("outside");
        std::fs::write(&outside, b"caller-owned").unwrap();
        std::fs::set_permissions(&outside, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::fs::hard_link(&outside, &issue).unwrap();
        assert!(matches!(attempt("42.json", None), Err(PreCommit(_))));
        assert_eq!(std::fs::read(&outside).unwrap(), b"caller-owned");
        assert_eq!(std::fs::metadata(&outside).unwrap().nlink(), 2);
        std::fs::remove_file(&issue).unwrap();
        nix::unistd::mkfifo(&issue, Mode::from_bits_truncate(0o600)).unwrap();
        std::fs::set_permissions(&issue, std::fs::Permissions::from_mode(0o600)).unwrap();
        let mut fifo = std::fs::File::from(
            open(&issue, OFlag::O_RDONLY | OFlag::O_NONBLOCK, Mode::empty()).unwrap(),
        );
        let before = fifo.metadata().unwrap();
        let fifo_directory = std::fs::File::from(directory.try_clone().unwrap());
        let (send, receive) = std::sync::mpsc::channel();
        let publisher = std::thread::spawn(move || {
            let result = super::publish_private_heartbeat_file(
                &fifo_directory,
                "42.json",
                b"heartbeat\n",
                "test",
                &mut |_, _| Ok(()),
            );
            send.send(result).unwrap();
        });
        assert!(matches!(
            receive.recv_timeout(std::time::Duration::from_secs(2)),
            Ok(Err(PreCommit(_)))
        ));
        publisher.join().unwrap();
        let after = std::fs::symlink_metadata(&issue).unwrap();
        let identity = |meta: &std::fs::Metadata| (meta.dev(), meta.ino(), meta.mode());
        assert_eq!(identity(&before), identity(&after));
        assert_eq!(fifo.read(&mut [0]).unwrap(), 0);
        std::fs::remove_file(&issue).unwrap();
        let session = fixture.join("session.json");
        let issue_publication = attempt("42.json", None).unwrap();
        let session_publication = attempt("session.json", None).unwrap();
        umask(previous);
        for (path, publication) in [(&issue, issue_publication), (&session, session_publication)] {
            assert_eq!(publication.durability, Durable);
            let metadata = std::fs::metadata(path).unwrap();
            assert_eq!(metadata.permissions().mode() & 0o7777, 0o600);
            assert_eq!(
                (metadata.dev(), metadata.ino()),
                (publication.device, publication.inode)
            );
            let held = publication.file.metadata().unwrap();
            assert_eq!((held.dev(), held.ino()), (metadata.dev(), metadata.ino()));
            assert_eq!(held.nlink(), 1);
        }

        for (name, failed, expected) in [
            ("pending.json", "directory-fsync", Unconfirmed),
            ("revalidate-error.json", "revalidate", Unconfirmed),
            ("extra-link.json", "hardlink", Unconfirmed),
        ] {
            let error = attempt(name, Some(failed)).unwrap_err();
            let PostCommit {
                publication,
                error: _,
            } = error
            else {
                panic!("post-link failure was reported as pre-commit");
            };
            assert_eq!(publication.durability, expected);
            let metadata = std::fs::metadata(fixture.join(name)).unwrap();
            assert_eq!(
                (metadata.dev(), metadata.ino()),
                (publication.device, publication.inode)
            );
        }

        let root = fixture.join("transaction-root");
        let repo = root.join(crate::commands::autonomous::drain::repository_progress_key(
            "owner/repo",
        ));
        let sessions = repo.join("sessions");
        std::fs::create_dir_all(&sessions).unwrap();
        unsafe { std::env::set_var("AUTOSPEC_HEARTBEAT_DIR", &root) };
        let issue = repo.join("42.json");
        let session = sessions.join("73657373696f6e2d61.json");
        let write = || {
            super::write_startup_heartbeat(
                "owner/repo",
                42,
                "worker-a",
                "feat/worker",
                "claim-a",
                Some("session-a"),
            )
        };
        let caller = fixture.join("caller-owned-heartbeat");
        std::fs::write(&caller, b"caller-owned").unwrap();
        std::fs::hard_link(&caller, &issue).unwrap();
        assert!(write().is_err());
        assert!(std::fs::read(&caller).unwrap() == b"caller-owned" && !session.exists());

        let prepared = b"{\"issue\":\"43\",\"branch\":\"feat/worker\",\"step\":\"claimed\",\"ts\":1,\"pr\":\"\",\"repo\":\"owner/repo\",\"worker_id\":\"worker-a\",\"claim_id\":\"claim-b\",\"session_id\":\"session-b\"}\n";
        let attempt = |issue, session, document: &[u8], failed| {
            super::publish_startup_heartbeat_transaction_with_hook(
                &root,
                "owner/repo",
                issue,
                Some(session),
                document,
                &mut |role, boundary| {
                    ((role, boundary) != failed)
                        .then_some(())
                        .ok_or_else(|| super::CommandFailure::diagnostic("injected failure"))
                },
            )
        };
        assert!(attempt(43, "session-b", prepared, ("session", "before-link")).is_err());
        assert!(
            !repo.join("43.json").exists() && !sessions.join("73657373696f6e2d62.json").exists()
        );

        std::fs::remove_file(&issue).unwrap();
        let transaction_umask = umask(Mode::from_bits_truncate(0o777));
        write().unwrap();
        umask(transaction_umask);
        assert_eq!(
            [&issue, &session]
                .map(|path| { std::fs::metadata(path).unwrap().permissions().mode() & 0o7777 }),
            [0o600; 2]
        );
        let expected = std::fs::read(&issue).unwrap();
        let mut stable_drift: serde_json::Value = serde_json::from_slice(&expected).unwrap();
        stable_drift["nonce"] = "foreign-generation".into();
        let stable_drift = serde_json::to_vec(&stable_drift).unwrap();
        let replacement = repo.join("replacement");
        let chmod =
            |mode| std::fs::set_permissions(&issue, std::fs::Permissions::from_mode(mode)).unwrap();
        for mutation in ["rename", "overwrite", "chmod"] {
            std::fs::write(&issue, &expected).unwrap();
            chmod(0o600);
            std::fs::remove_file(&session).unwrap();
            std::fs::write(&replacement, b"foreign").unwrap();
            let result = super::publish_startup_heartbeat_transaction_with_hook(
                &root,
                "owner/repo",
                42,
                Some("session-a"),
                &expected,
                &mut |role, boundary| {
                    if (role, boundary) == ("session", "directory-fsync") {
                        match mutation {
                            "rename" => std::fs::rename(&replacement, &issue).unwrap(),
                            "overwrite" => std::fs::write(&issue, &stable_drift).unwrap(),
                            _ => chmod(0o640),
                        }
                    }
                    Ok(())
                },
            );
            assert!(result.is_err(), "{mutation}");
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn heartbeat_root_parent_bootstrap_and_migration_are_durable() {
        use std::os::unix::fs::symlink;

        let (fixture, _) = startup_heartbeat_fixture("root-parent-bootstrap");
        let parent = fixture.join(".autospec");
        let root = parent.join("process-heartbeats");
        let mut syncs = Vec::new();
        super::prepare_heartbeat_root_parent_with_hook(&root, |boundary| {
            syncs.push(boundary);
            Ok(())
        })
        .unwrap();
        assert_eq!(
            std::fs::metadata(&parent).unwrap().permissions().mode() & 0o7777,
            0o700
        );
        assert_eq!(
            syncs,
            [
                "chmod",
                "parent",
                "before-publish",
                "ancestor-fsync",
                "ancestor"
            ]
        );

        std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o775)).unwrap();
        syncs.clear();
        super::prepare_heartbeat_root_parent_with_hook(&root, |boundary| {
            syncs.push(boundary);
            Ok(())
        })
        .unwrap();
        assert_eq!(
            std::fs::metadata(&parent).unwrap().permissions().mode() & 0o7777,
            0o700
        );
        assert_eq!(syncs, ["chmod", "parent", "ancestor-fsync", "ancestor"]);

        std::fs::remove_dir(&parent).unwrap();
        let outside = fixture.join("outside");
        std::fs::create_dir(&outside).unwrap();
        let outside_mode = std::fs::metadata(&outside).unwrap().permissions().mode();
        symlink(&outside, &parent).unwrap();
        assert!(super::prepare_heartbeat_root_parent_with_hook(&root, |_| Ok(())).is_err());
        assert_eq!(
            std::fs::metadata(&outside).unwrap().permissions().mode(),
            outside_mode
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn startup_heartbeat_restrictive_umask() {
        use nix::sys::stat::{umask, Mode};
        use std::os::unix::fs::MetadataExt;

        const CHILD: &str = "AUTOSPEC_TEST_RESTRICTIVE_UMASK_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let output = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "commands::claim::tests::startup_heartbeat_restrictive_umask",
                    "--nocapture",
                ])
                .env(CHILD, "1")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "child stderr: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        let (fixture, _) = startup_heartbeat_fixture("restrictive-umask");
        let parent = fixture.join(".autospec");
        let root = parent.join("process-heartbeats");
        let assert_no_staging = || {
            assert!(std::fs::read_dir(&fixture).unwrap().all(|entry| {
                !entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".autospec-heartbeat-stage-")
            }));
        };
        let previous = umask(Mode::from_bits_truncate(0o777));
        let failed = super::prepare_heartbeat_root_parent_with_hook(&root, |boundary| {
            inject_heartbeat_boundary(boundary, "chmod", "injected chmod failure")
        });
        umask(previous);

        assert!(failed.is_err());
        assert!(!parent.exists());
        assert_no_staging();
        let failed = super::prepare_heartbeat_root_parent_with_hook(&root, |boundary| {
            inject_heartbeat_boundary(boundary, "parent", "injected fsync failure")
        });
        assert!(failed.is_err());
        assert!(!parent.exists());
        assert_no_staging();
        super::prepare_heartbeat_root_parent_with_hook(&root, |_| Ok(())).unwrap();
        assert_eq!(
            std::fs::metadata(&parent).unwrap().permissions().mode() & 0o7777,
            0o700
        );

        std::fs::remove_dir(&parent).unwrap();
        let replacement_identity = std::cell::Cell::new((0, 0));
        let raced = super::prepare_heartbeat_root_parent_with_hook(&root, |boundary| {
            if boundary == "before-publish" {
                std::fs::create_dir(&parent).unwrap();
                std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o750)).unwrap();
                let metadata = std::fs::metadata(&parent).unwrap();
                replacement_identity.set((metadata.ino(), metadata.mode()));
            }
            Ok(())
        });
        assert!(raced.is_err());
        let replacement = std::fs::metadata(&parent).unwrap();
        assert_eq!(
            (replacement.ino(), replacement.mode()),
            replacement_identity.get()
        );
        assert_no_staging();

        std::fs::remove_dir(&parent).unwrap();
        let fail_ancestor_once = std::cell::Cell::new(true);
        let ancestor_attempts = std::cell::Cell::new(0);
        let mut ancestor_failure = |boundary| {
            if boundary == "ancestor-fsync" {
                ancestor_attempts.set(ancestor_attempts.get() + 1);
                if fail_ancestor_once.replace(false) {
                    return Err(super::CommandFailure::diagnostic(
                        "injected ancestor fsync failure",
                    ));
                }
            }
            Ok(())
        };
        assert!(
            super::prepare_heartbeat_root_parent_with_hook(&root, &mut ancestor_failure).is_err()
        );
        assert!(
            parent.is_dir(),
            "published parent remains pending durability"
        );
        super::prepare_heartbeat_root_parent_with_hook(&root, &mut ancestor_failure).unwrap();
        assert_eq!(ancestor_attempts.get(), 2);

        std::fs::remove_dir(&parent).unwrap();
        let cleanup_failure = super::prepare_heartbeat_root_parent_with_hook(&root, |boundary| {
            if boundary == "parent" {
                let staging = std::fs::read_dir(&fixture)
                    .unwrap()
                    .map(|entry| entry.unwrap().path())
                    .find(|path| {
                        path.file_name()
                            .unwrap()
                            .to_string_lossy()
                            .starts_with(".autospec-heartbeat-stage-")
                    })
                    .unwrap();
                std::fs::write(staging.join("block-cleanup"), "occupied").unwrap();
                return Err(super::CommandFailure::diagnostic("injected staged failure"));
            }
            Ok(())
        })
        .unwrap_err();
        assert!(cleanup_failure
            .message
            .contains("could not remove staged heartbeat root parent"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn retryable_release_requires_exact_heartbeat_evidence() {
        let _guard = STARTUP_HEARTBEAT_ENV.lock().expect("heartbeat env");
        let (root, _) = startup_heartbeat_fixture("released-missing");
        let heartbeat_root = root.join("heartbeats");
        std::fs::create_dir(&heartbeat_root).unwrap();
        std::fs::set_permissions(&heartbeat_root, std::fs::Permissions::from_mode(0o700)).unwrap();
        let previous = std::env::var_os("AUTOSPEC_HEARTBEAT_DIR");
        std::env::set_var("AUTOSPEC_HEARTBEAT_DIR", heartbeat_root);
        let identity = super::ClaimMutationIdentity {
            repo: "owner/repo",
            issue: 42,
            worker_id: "worker-a",
            branch: "feat/worker",
            claim_id: "claim-a",
        };
        let result = super::retire_released_startup_heartbeat(identity);
        match previous {
            Some(value) => std::env::set_var("AUTOSPEC_HEARTBEAT_DIR", value),
            None => std::env::remove_var("AUTOSPEC_HEARTBEAT_DIR"),
        }
        std::fs::remove_dir_all(root).unwrap();
        result.expect_err("missing issue evidence must keep terminal preparation retryable");
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn heartbeat_directory_openat2() {
        use nix::fcntl::{open, OFlag, ResolveFlag};
        use nix::sys::stat::fstat;
        use std::os::unix::fs::{symlink, MetadataExt};

        let open_parent = |path: &Path| {
            std::fs::File::from(
                open(
                    path,
                    OFlag::O_PATH | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
                    nix::sys::stat::Mode::empty(),
                )
                .expect("open trusted parent"),
            )
        };
        let make_child = |parent: &Path| {
            let child = parent.join("heartbeat");
            std::fs::create_dir(&child).expect("heartbeat child");
            std::fs::set_permissions(&child, std::fs::Permissions::from_mode(0o700))
                .expect("private heartbeat child");
            child
        };

        let (trusted, _) = startup_heartbeat_fixture("openat2-trusted");
        let trusted_child = make_child(&trusted);
        let parent = open_parent(&trusted);
        let opened = super::open_heartbeat_directory_beneath(&parent, Path::new("heartbeat"))
            .expect("open descendant");
        let expected = std::fs::metadata(&trusted_child).expect("descendant metadata");
        let observed = fstat(&opened).expect("opened metadata");
        assert_eq!(
            (observed.st_dev, observed.st_ino),
            (expected.dev(), expected.ino())
        );

        let (outside, _) = startup_heartbeat_fixture("openat2-outside");
        make_child(&outside);
        symlink(&outside, trusted.join("parent-link")).expect("descendant parent symlink");
        assert!(super::open_heartbeat_directory_beneath(
            &parent,
            Path::new("parent-link/heartbeat")
        )
        .is_err());
        assert!(super::open_heartbeat_directory_beneath(&parent, Path::new("")).is_err());
        assert!(super::open_heartbeat_directory_beneath(&parent, &trusted_child).is_err());
        assert!(super::open_heartbeat_directory_beneath(&parent, Path::new("../escape")).is_err());
        assert!(super::heartbeat_openat2_resolve_flags().contains(ResolveFlag::RESOLVE_NO_XDEV));

        let (drifting, _) = startup_heartbeat_fixture("openat2-drifting");
        make_child(&drifting);
        let drifting_parent = open_parent(&drifting);
        let drift = super::open_heartbeat_directory_beneath_with_hook(
            &drifting_parent,
            Path::new("heartbeat"),
            || {
                std::fs::set_permissions(&drifting, std::fs::Permissions::from_mode(0o755))
                    .expect("drift parent mode");
            },
        );
        assert!(drift.is_err());
        std::fs::set_permissions(&drifting, std::fs::Permissions::from_mode(0o700))
            .expect("restore parent mode");

        let (replaceable, _) = startup_heartbeat_fixture("openat2-replaceable");
        let replaceable_child = make_child(&replaceable);
        let original_inode = std::fs::metadata(&replaceable_child)
            .expect("original child metadata")
            .ino();
        let (replacement, _) = startup_heartbeat_fixture("openat2-replacement");
        let replacement_child = make_child(&replacement);
        let replacement_inode = std::fs::metadata(&replacement_child)
            .expect("replacement child metadata")
            .ino();
        let anchored_parent = open_parent(&replaceable);
        let anchored = replaceable.with_extension("anchored");
        let opened = super::open_heartbeat_directory_beneath_with_hook(
            &anchored_parent,
            Path::new("heartbeat"),
            || {
                std::fs::rename(&replaceable, &anchored).expect("move trusted directory");
                std::fs::rename(&replacement, &replaceable).expect("install replacement directory");
            },
        )
        .expect("open anchored original child");
        let opened_inode = fstat(&opened).expect("opened child metadata").st_ino;
        assert_eq!(opened_inode, original_inode);
        assert_ne!(opened_inode, replacement_inode);
        std::fs::rename(&replaceable, &replacement).expect("remove replacement directory");
        std::fs::rename(&anchored, &replaceable).expect("restore trusted directory");

        let displaced_child = replaceable.join("displaced");
        let swapped_child = super::open_heartbeat_directory_beneath_with_hook(
            &anchored_parent,
            Path::new("heartbeat"),
            || {
                std::fs::rename(&replaceable_child, &displaced_child)
                    .expect("displace trusted child");
                std::fs::rename(&replacement_child, &replaceable_child)
                    .expect("install replacement child");
            },
        );
        assert!(swapped_child.is_err(), "changed child binding was accepted");
        std::fs::rename(&replaceable_child, &replacement_child).expect("remove replacement child");
        std::fs::rename(&displaced_child, &replaceable_child).expect("restore trusted child");

        std::fs::remove_dir_all(trusted).expect("remove trusted fixture");
        std::fs::remove_dir_all(outside).expect("remove outside fixture");
        std::fs::remove_dir_all(drifting).expect("remove drift fixture");
        std::fs::remove_dir_all(replaceable).expect("remove replacement fixture");
        std::fs::remove_dir_all(replacement).expect("remove rename-in fixture");
    }

    #[cfg(unix)]
    #[test]
    fn startup_heartbeat_portable_unix() {
        use nix::fcntl::{open, OFlag};
        use nix::sys::stat::{fstat, Mode};
        use std::os::unix::fs::{symlink, MetadataExt};

        let make_private_child = |parent: &Path| {
            let child = parent.join("heartbeat");
            std::fs::create_dir(&child).expect("heartbeat child");
            std::fs::set_permissions(&child, std::fs::Permissions::from_mode(0o700))
                .expect("private heartbeat child");
            child
        };
        let open_parent = |path: &Path| {
            std::fs::File::from(
                open(
                    path,
                    OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
                    Mode::empty(),
                )
                .expect("open trusted parent"),
            )
        };

        let (trusted, _) = startup_heartbeat_fixture("portable-unix");
        let trusted_child = make_private_child(&trusted);
        let parent = open_parent(&trusted);
        let opened = super::open_heartbeat_directory_portable_unix_with_hook(
            &parent,
            Path::new("heartbeat"),
            || {},
        )
        .expect("open one-component descendant");
        let expected = std::fs::metadata(&trusted_child).expect("descendant metadata");
        let observed = fstat(&opened).expect("opened metadata");
        assert_eq!(
            (observed.st_dev, observed.st_ino),
            (expected.dev(), expected.ino())
        );

        let (replacement, _) = startup_heartbeat_fixture("portable-unix-replacement");
        let replacement_child = make_private_child(&replacement);
        let displaced = trusted.join("displaced");
        let swapped = super::open_heartbeat_directory_portable_unix_with_hook(
            &parent,
            Path::new("heartbeat"),
            || {
                std::fs::rename(&trusted_child, &displaced).expect("displace trusted child");
                std::fs::rename(&replacement_child, &trusted_child)
                    .expect("install replacement child");
            },
        );
        assert!(swapped.is_err(), "changed name binding was accepted");
        std::fs::remove_dir(&trusted_child).expect("remove replacement child");
        symlink(&displaced, &trusted_child).expect("install descendant symlink");
        assert!(
            super::open_heartbeat_directory_portable_unix_with_hook(
                &parent,
                Path::new("heartbeat"),
                || {},
            )
            .is_err(),
            "descendant symlink was accepted"
        );
        assert!(
            super::open_heartbeat_directory_portable_unix_with_hook(
                &parent,
                Path::new("nested/heartbeat"),
                || {},
            )
            .is_err(),
            "multi-component descendant was accepted"
        );

        std::fs::remove_dir_all(trusted).expect("remove trusted fixture");
        std::fs::remove_dir_all(replacement).expect("remove replacement fixture");
    }

    #[test]
    fn classify_startup_heartbeat_marks_missing_evidence_absent() {
        let (directory, path) = startup_heartbeat_fixture("absent");
        let classified = super::classify_startup_heartbeat(
            &path,
            expected_startup_heartbeat("worker-a"),
            200,
            |_, _, _, _, _| super::StartupPidLiveness::Dead,
        );
        assert_eq!(classified, super::StartupHeartbeatClassification::Absent);
        std::fs::remove_dir_all(directory).expect("remove heartbeat fixture");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn startup_heartbeat_process_identity() {
        use super::{StartupHeartbeatClassification::ExpiredDead, StartupPidLiveness};

        let _environment = STARTUP_HEARTBEAT_ENV.lock().unwrap();
        let (directory, _) = startup_heartbeat_fixture("process-identity");
        let root = directory.join(".autospec/process-heartbeats");
        let old_root = std::env::var_os("AUTOSPEC_HEARTBEAT_DIR");
        let old_ttl = std::env::var_os("AUTOSPEC_CLAIM_LEASE_SECONDS");
        std::env::set_var("AUTOSPEC_HEARTBEAT_DIR", &root);
        std::env::set_var("AUTOSPEC_CLAIM_LEASE_SECONDS", "0");
        let path = root
            .join(super::super::autonomous::drain::repository_progress_key(
                "owner/repo",
            ))
            .join("42.json");
        let publish = |claim_id, session_id| {
            super::write_startup_heartbeat(
                "owner/repo",
                42,
                "worker-a",
                "feat/worker",
                claim_id,
                Some(session_id),
            )
            .unwrap();
            super::parse_startup_heartbeat(&std::fs::read(&path).unwrap())
                .unwrap()
                .nonce
        };
        let first_nonce = publish("claim-a", "session-a");
        assert_eq!(publish("claim-a", "session-a"), first_nonce);
        std::fs::remove_file(&path).unwrap();
        assert_ne!(publish("claim-b", "session-b"), first_nonce);
        let mut nonces = Vec::new();
        let mut last = None;
        for worker in ["worker-a", "opaque worker:with/slash"] {
            std::fs::remove_file(&path).unwrap();
            super::write_startup_heartbeat(
                "owner/repo",
                42,
                worker,
                "feat/worker",
                "claim-a",
                None,
            )
            .unwrap();
            let document = std::fs::read(&path).unwrap();
            let evidence = super::parse_startup_heartbeat(&document).unwrap();
            assert_eq!(evidence.worker_id, worker);
            assert_eq!(evidence.pid, std::process::id());
            assert_eq!(evidence.ttl_seconds, 1);
            assert!(!evidence.nonce.is_empty());
            assert_eq!(
                super::observe_local_startup_pid(
                    worker,
                    evidence.pid,
                    &evidence.host,
                    &evidence.boot_id,
                    &evidence.process_start,
                ),
                StartupPidLiveness::Live
            );
            assert!(matches!(
                super::classify_startup_heartbeat(
                    &path,
                    expected_startup_heartbeat(worker),
                    evidence.ts + evidence.ttl_seconds + 1,
                    |observed, pid, host, boot_id, process_start| {
                        assert_eq!((observed, pid), (worker, evidence.pid));
                        assert_eq!(
                            (host, boot_id, process_start),
                            (
                                evidence.host.as_str(),
                                evidence.boot_id.as_str(),
                                evidence.process_start.as_str()
                            )
                        );
                        StartupPidLiveness::Dead
                    },
                ),
                ExpiredDead(_)
            ));
            nonces.push(evidence.nonce.clone());
            last = Some((worker, document, evidence));
        }
        assert_eq!(nonces[0], nonces[1]);

        let (worker, document, evidence) = last.unwrap();
        let document = String::from_utf8(document).unwrap();
        let pid = format!("\"pid\":{}", evidence.pid);
        let nonce = format!("\"nonce\":\"{}\"", evidence.nonce);
        for malformed in [
            document.replace(&pid, "\"pid\":0"),
            document.replace(&pid, "\"pid\":\"bad\""),
            document.replace(&nonce, "\"nonce\":\"\""),
            document.replace(&nonce, &format!("\"nonce\":\"{}\"", "f".repeat(64))),
            document.replace(worker, "different-worker"),
        ] {
            std::fs::write(&path, malformed).unwrap();
            assert_eq!(
                super::classify_startup_heartbeat(
                    &path,
                    expected_startup_heartbeat(worker),
                    evidence.ts + evidence.ttl_seconds + 1,
                    |_, _, _, _, _| StartupPidLiveness::Dead,
                ),
                super::StartupHeartbeatClassification::Blocking
            );
        }
        for (key, value) in [
            ("AUTOSPEC_HEARTBEAT_DIR", old_root),
            ("AUTOSPEC_CLAIM_LEASE_SECONDS", old_ttl),
        ] {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn startup_heartbeat_remote_process_identity() {
        let _environment = STARTUP_HEARTBEAT_ENV.lock().unwrap();
        let (directory, _) = startup_heartbeat_fixture("remote-process-identity");
        let root = directory.join(".autospec/process-heartbeats");
        let old_root = std::env::var_os("AUTOSPEC_HEARTBEAT_DIR");
        std::env::set_var("AUTOSPEC_HEARTBEAT_DIR", &root);
        super::write_startup_heartbeat(
            "owner/repo",
            42,
            "opaque-worker",
            "feat/worker",
            "claim-a",
            None,
        )
        .unwrap();
        let path = root
            .join(super::super::autonomous::drain::repository_progress_key(
                "owner/repo",
            ))
            .join("42.json");
        let document = std::fs::read(&path).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&document).unwrap();
        let identity = |name| {
            value
                .get(name)
                .and_then(serde_json::Value::as_str)
                .filter(|field| !field.is_empty())
                .unwrap()
                .to_string()
        };
        let host = identity("host");
        let boot_id = identity("boot_id");
        let process_start = identity("process_start");
        assert_ne!(host, boot_id);
        assert_ne!(boot_id, process_start);
        let absent_pid = i32::MAX as u32;
        assert!(!Path::new(&format!("/proc/{absent_pid}")).exists());
        for (field, replacement, remote) in [
            ("host", "remote-host", true),
            ("boot_id", "remote-boot", false),
            ("process_start", "0", true),
            ("process_start", "garbage", true),
            ("process_start", "01", true),
        ] {
            let mut mutated = value.clone();
            mutated[field] = serde_json::json!(replacement);
            if remote {
                mutated["pid"] = serde_json::json!(absent_pid);
            }
            std::fs::write(&path, serde_json::to_vec(&mutated).unwrap()).unwrap();
            assert_eq!(
                super::classify_startup_heartbeat(
                    &path,
                    expected_startup_heartbeat("opaque-worker"),
                    value["ts"].as_u64().unwrap() + value["ttl_seconds"].as_u64().unwrap() + 1,
                    super::observe_local_startup_pid,
                ),
                super::StartupHeartbeatClassification::Blocking,
                "{field}"
            );
        }
        match old_root {
            Some(value) => std::env::set_var("AUTOSPEC_HEARTBEAT_DIR", value),
            None => std::env::remove_var("AUTOSPEC_HEARTBEAT_DIR"),
        }
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn stale_heartbeat_receipt_transaction() {
        use super::HeartbeatReceiptDecision::{Absent, Blocking, Completed, Pending};
        use nix::fcntl::{open, OFlag};
        use nix::sys::stat::Mode;
        use std::os::unix::fs::symlink;
        use std::os::unix::net::UnixListener;
        use std::os::unix::prelude::AsRawFd;
        use std::time::{Duration, Instant};
        let expected = expected_startup_heartbeat("host:user:rust:4242:nonce-a");
        let open_parent = |path: &Path| {
            std::fs::File::from(
                open(path, OFlag::O_PATH | OFlag::O_DIRECTORY, Mode::empty()).unwrap(),
            )
        };
        let (parent_path, _) = startup_heartbeat_fixture("receipt-red");
        let repo_path = parent_path.join("repo");
        std::fs::create_dir(&repo_path).expect("repo directory");
        std::fs::set_permissions(&repo_path, std::fs::Permissions::from_mode(0o700))
            .expect("private repo");
        let parent = open_parent(&parent_path);
        let repo =
            super::open_heartbeat_directory_beneath(&parent, Path::new("repo")).expect("repo fd");
        let decision = || super::heartbeat_receipt_retry_decision(&repo, expected);
        let transaction = super::begin_heartbeat_receipt(&repo, expected).expect("begin receipt");
        assert_eq!(decision(), Pending);
        let mut unrelated = expected;
        unrelated.issue += 1;
        assert_eq!(
            super::heartbeat_receipt_retry_decision(&repo, unrelated),
            Absent
        );
        super::retire_heartbeat_receipt_with_sync(transaction, |_| {
            Err(super::CommandFailure::diagnostic("injected sync failure"))
        })
        .expect_err("sync failure");
        assert_eq!(decision(), Completed);
        assert!(super::begin_heartbeat_receipt(&repo, expected).is_err());
        let (pending, completed) = super::heartbeat_receipt_names(expected);
        let handoff = repo_path.join("quarantine/startup-heartbeat-handoffs");
        let pending_path = handoff.join(&pending);
        let completed_path = handoff.join(&completed);
        std::fs::remove_file(&completed_path).unwrap();
        super::begin_heartbeat_receipt_with_hook(&repo, expected, || {
            std::fs::write(&completed_path, b"").unwrap();
            std::fs::set_permissions(&completed_path, std::fs::Permissions::from_mode(0o600))
                .unwrap();
        })
        .err()
        .expect("completed wins");
        assert_eq!(decision(), Blocking);
        assert!(pending_path.exists() && completed_path.exists());
        std::fs::remove_file(&pending_path).unwrap();
        std::fs::remove_file(&completed_path).unwrap();
        let mut transaction = super::begin_heartbeat_receipt(&repo, expected).unwrap();
        transaction.pending.push_str("-missing");
        super::retire_heartbeat_receipt_with_sync(transaction, |_| Ok(()))
            .expect_err("rename failure");
        assert_eq!(decision(), Pending);
        let (pending, completed) = super::heartbeat_receipt_names(expected);
        let handoff_fd = std::fs::File::open(&handoff).unwrap();
        let socket_root = PathBuf::from(format!("/proc/self/fd/{}", handoff_fd.as_raw_fd()));
        let drift =
            super::heartbeat_receipt_retry_decision_with_hook(&repo, expected, |boundary| {
                if boundary == "handoff" {
                    std::fs::set_permissions(&handoff, std::fs::Permissions::from_mode(0o755))
                        .unwrap();
                }
            });
        assert_eq!(drift, Blocking);
        std::fs::set_permissions(&handoff, std::fs::Permissions::from_mode(0o700)).unwrap();
        let pending_path = handoff.join(&pending);
        let completed_path = handoff.join(&completed);
        std::fs::write(&completed_path, b"").unwrap();
        std::fs::set_permissions(&completed_path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(decision(), Blocking);
        std::fs::remove_file(&completed_path).unwrap();
        for unsafe_kind in ["fifo", "symlink", "socket", "size", "mode"] {
            std::fs::remove_file(&pending_path).unwrap();
            match unsafe_kind {
                "fifo" => {
                    nix::unistd::mkfifo(&pending_path, Mode::from_bits_truncate(0o600)).unwrap()
                }
                "symlink" => symlink("/dev/null", &pending_path).unwrap(),
                "socket" => {
                    drop(UnixListener::bind(socket_root.join(&pending)).unwrap());
                    std::fs::set_permissions(&pending_path, std::fs::Permissions::from_mode(0o600))
                        .unwrap();
                }
                "size" => std::fs::write(&pending_path, b"x").unwrap(),
                _ => std::fs::write(&pending_path, b"").unwrap(),
            }
            if unsafe_kind == "mode" {
                std::fs::set_permissions(&pending_path, std::fs::Permissions::from_mode(0o644))
                    .unwrap();
            }
            let started = Instant::now();
            assert_eq!(decision(), Blocking);
            assert!(started.elapsed() < Duration::from_secs(2));
        }
        std::fs::remove_file(&pending_path).unwrap();
        drop(UnixListener::bind(socket_root.join(&completed)).unwrap());
        std::fs::set_permissions(&completed_path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let started = Instant::now();
        assert_eq!(decision(), Blocking);
        assert!(started.elapsed() < Duration::from_secs(2));
        let dev = std::fs::File::open("/dev").unwrap();
        assert_eq!(
            super::inspect_heartbeat_receipt(&dev, std::ffi::OsStr::new("null")),
            super::HeartbeatReceiptEntry::Unsafe
        );
        std::fs::remove_dir_all(parent_path).expect("remove failure fixture");
        let (parent_path, _) = startup_heartbeat_fixture("receipt-renames");
        let repo_path = parent_path.join("repo");
        std::fs::create_dir(&repo_path).unwrap();
        std::fs::set_permissions(&repo_path, std::fs::Permissions::from_mode(0o700)).unwrap();
        let parent = open_parent(&parent_path);
        let repo = super::open_heartbeat_directory_beneath(&parent, Path::new("repo")).unwrap();
        drop(super::begin_heartbeat_receipt(&repo, expected).unwrap());
        let replace = |path: &Path, moved: &Path| {
            std::fs::rename(path, moved).unwrap();
            std::fs::create_dir(path).unwrap();
        };
        let decision =
            super::heartbeat_receipt_retry_decision_with_hook(&repo, expected, |boundary| {
                match boundary {
                    "repo" => replace(&repo_path, &parent_path.join("repo-old")),
                    "quarantine" => {
                        let old = parent_path.join("repo-old");
                        replace(&old.join("quarantine"), &old.join("quarantine-old"));
                    }
                    "handoff" => {
                        let old = parent_path.join("repo-old/quarantine-old");
                        replace(
                            &old.join("startup-heartbeat-handoffs"),
                            &old.join("handoff-old"),
                        );
                    }
                    _ => unreachable!(),
                }
            });
        assert_eq!(decision, Pending);
        std::fs::remove_dir_all(parent_path).expect("remove rename fixture");
    }

    #[test]
    fn unsupported_platform_stale_recovery_is_fail_closed() {
        let source = include_str!("claim.rs");
        let fallback = source
            .split(
                "#[cfg(not(target_os = \"linux\"))]\nfn quarantine_authoritative_stale_heartbeat",
            )
            .nth(1)
            .expect("unsupported-platform recovery fallback");
        let fallback = fallback
            .split("fn release_stale_startup_labels")
            .next()
            .expect("fallback boundary");
        assert!(fallback.contains("Ok(false)"));
        assert!(!fallback.contains("startup_heartbeat_exists"));
    }

    #[test]
    fn classify_startup_heartbeat_blocks_fresh_live_malformed_mismatched_and_remote_evidence() {
        use super::StartupPidLiveness::{Dead, Live};
        let (directory, path) = startup_heartbeat_fixture("blocking");
        let worker = "host:user:rust:4242:nonce-a";
        let exact = startup_heartbeat_document(worker, 4242);
        let changed = |from, to| exact.replace(from, to);
        let mut cases = vec![
            ("fresh", exact.clone(), 105, Dead, false),
            ("live", exact.clone(), 200, Live, true),
        ];
        let mutations = r#"malformed|not-json
owner/repo|other/repo
"42"|"43"
host:user|other:user
feat/worker|feat/other
"pr":""|"pr":"17"
claim-a|claim-b
claimed|review
"ttl_seconds":10|"ttl_seconds":0
"pid":4242|"pid":0
:nonce-a"|:nonce-b"
"process_start":"1"|"process_start":"""#;
        cases.extend(mutations.lines().map(|row| {
            let (from, to) = row.split_once('|').expect("mutation row");
            let document = if from == "malformed" {
                to.into()
            } else {
                changed(from, to)
            };
            ("malformed or mismatch", document, 200, Dead, false)
        }));
        for (label, document, now, liveness, should_observe) in cases {
            std::fs::write(&path, document).expect("write heartbeat fixture");
            let mut observed = false;
            assert_eq!(
                super::classify_startup_heartbeat(
                    &path,
                    expected_startup_heartbeat(worker),
                    now,
                    |_, _, _, _, _| {
                        observed = true;
                        liveness
                    },
                ),
                super::StartupHeartbeatClassification::Blocking,
                "{label}"
            );
            assert_eq!(observed, should_observe, "{label} liveness call");
        }
        std::fs::remove_dir_all(directory).expect("remove heartbeat fixture");
    }

    #[cfg(unix)]
    #[test]
    fn classify_startup_heartbeat_returns_snapshot_only_for_expired_dead_local_pid() {
        use std::os::unix::fs::MetadataExt;
        let (directory, path) = startup_heartbeat_fixture("expired-dead");
        let worker = "host:user:rust:4242:nonce-a";
        let document = startup_heartbeat_document(worker, 4242);
        std::fs::write(&path, &document).expect("write heartbeat fixture");
        let classified = super::classify_startup_heartbeat(
            &path,
            expected_startup_heartbeat(worker),
            200,
            |worker, pid, _, _, _| {
                assert_eq!((worker, pid), ("host:user:rust:4242:nonce-a", 4242));
                super::StartupPidLiveness::Dead
            },
        );
        let super::StartupHeartbeatClassification::ExpiredDead(snapshot) = classified else {
            panic!("exact expired dead evidence was not reclaimable");
        };
        let metadata = std::fs::metadata(&path).expect("heartbeat metadata");
        assert_eq!(snapshot.file.document, document.as_bytes());
        assert_eq!(snapshot.file.identity.device, metadata.dev());
        assert_eq!(snapshot.file.identity.inode, metadata.ino());
        assert_eq!(snapshot.file.identity.length, metadata.len());
        assert_eq!(snapshot.file.identity.modified_seconds, metadata.mtime());
        assert_eq!(snapshot.file.identity.modified_nanos, metadata.mtime_nsec());
        std::fs::remove_dir_all(directory).expect("remove heartbeat fixture");
    }

    #[cfg(unix)]
    #[test]
    fn classify_startup_heartbeat_rejects_symlink_and_observes_current_pid_as_live() {
        use super::StartupHeartbeatClassification::Blocking;

        let (directory, path) = startup_heartbeat_fixture("symlink-live");
        let target = directory.join("target.json");
        let host = std::env::var("HOSTNAME").unwrap_or_else(|_| "unknown-host".into());
        let user = std::env::var("USER").unwrap_or_else(|_| "unknown-user".into());
        let pid = std::process::id();
        let worker = format!("{host}:{user}:rust:{pid}:nonce-a");
        let identity = super::startup_process_identity(pid).unwrap();
        let make_document = |worker| {
            startup_heartbeat_document(worker, pid)
                .replace("host-a", &identity.0)
                .replace("boot-a", &identity.1)
                .replace("start-a", &identity.2)
        };
        let document = make_document(&worker);
        std::fs::write(&target, &document).expect("write target heartbeat");
        std::os::unix::fs::symlink(&target, &path).expect("symlink heartbeat");
        let classify = |worker: &str| {
            let mut observed = false;
            let result = super::classify_startup_heartbeat(
                &path,
                expected_startup_heartbeat(worker),
                200,
                |worker, pid, host, boot_id, process_start| {
                    observed = true;
                    super::observe_local_startup_pid(worker, pid, host, boot_id, process_start)
                },
            );
            (result, observed)
        };
        assert_eq!(classify(&worker), (Blocking, false));
        std::fs::remove_file(&path).expect("remove symlink");
        std::fs::rename(&target, &path).expect("publish regular heartbeat");
        assert_eq!(classify(&worker), (Blocking, true));
        let remote_worker = format!("remote-{host}:{user}:rust:{pid}:nonce-a");
        std::fs::write(&path, make_document(&remote_worker)).expect("write remote heartbeat");
        assert_eq!(
            super::observe_local_startup_pid(
                &remote_worker,
                pid,
                &identity.0,
                &identity.1,
                &identity.2,
            ),
            super::StartupPidLiveness::Live
        );
        assert_eq!(classify(&remote_worker), (Blocking, true));
        std::fs::remove_dir_all(directory).expect("remove heartbeat fixture");
    }

    #[cfg(unix)]
    #[test]
    fn startup_heartbeat_fifo_path_reader_is_nonblocking() {
        let (directory, fifo) = startup_heartbeat_fixture("fifo-path");
        nix::unistd::mkfifo(&fifo, nix::sys::stat::Mode::from_bits_truncate(0o600)).unwrap();
        let read_path = fifo.clone();
        assert_fifo_reader_nonblocking(&fifo, move || {
            super::read_regular_file_no_follow(&read_path)
        });
        assert_eq!(
            super::classify_startup_heartbeat(
                &fifo,
                expected_startup_heartbeat("host:user:rust:4242:nonce-a"),
                200,
                |_, _, _, _, _| super::StartupPidLiveness::Dead,
            ),
            super::StartupHeartbeatClassification::Blocking
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn startup_heartbeat_fifo_at_reader_is_nonblocking() {
        let (directory, fifo) = startup_heartbeat_fixture("fifo-at");
        nix::unistd::mkfifo(&fifo, nix::sys::stat::Mode::from_bits_truncate(0o600)).unwrap();
        let root = std::fs::File::open(&directory).unwrap();
        let name = fifo.file_name().unwrap().to_owned();
        assert_fifo_reader_nonblocking(&fifo, move || {
            super::read_regular_file_at_no_follow(&root, &name)
        });
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn startup_heartbeat_fifo_timeout_recovery_is_bounded() {
        let (directory, fifo) = startup_heartbeat_fixture("fifo-timeout-recovery");
        nix::unistd::mkfifo(&fifo, nix::sys::stat::Mode::from_bits_truncate(0o600)).unwrap();
        let read_path = fifo.clone();
        let panic = std::panic::catch_unwind(|| {
            assert_fifo_reader_nonblocking(&fifo, move || {
                let mut file = std::fs::File::open(read_path)?;
                let mut payload = Vec::new();
                std::io::Read::read_to_end(&mut file, &mut payload)?;
                Err(std::io::Error::other("payload reader completed"))
            });
        })
        .expect_err("timeout recovery must retain the original failure");
        let message = panic
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| panic.downcast_ref::<&str>().copied())
            .expect("timeout recovery panic message");
        assert!(
            message.contains("heartbeat FIFO reader blocked: timed out waiting on channel"),
            "unexpected timeout recovery diagnostic: {message}"
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    fn expired_heartbeat_snapshot(path: &Path) -> Box<super::StartupHeartbeatSnapshot> {
        let worker = "host:user:rust:4242:nonce-a";
        std::fs::write(path, startup_heartbeat_document(worker, 4242))
            .expect("write expired heartbeat");
        let classified = super::classify_startup_heartbeat(
            path,
            expected_startup_heartbeat(worker),
            200,
            |_, _, _, _, _| super::StartupPidLiveness::Dead,
        );
        let super::StartupHeartbeatClassification::ExpiredDead(snapshot) = classified else {
            panic!("fixture heartbeat was not expired and dead");
        };
        snapshot
    }

    #[cfg(unix)]
    fn heartbeat_copy_path(root: &Path) -> PathBuf {
        let nonce = super::startup_heartbeat_nonce("owner/repo", 42, "claim-a");
        root.join(format!(
            "quarantine/startup-heartbeats/42-{}.json",
            super::heartbeat_session_key(&nonce)
        ))
    }

    #[cfg(unix)]
    fn heartbeat_handoff_count(root: &Path) -> usize {
        std::fs::read_dir(root.join("quarantine/startup-heartbeat-handoffs"))
            .expect("handoff directory")
            .count()
    }

    #[cfg(unix)]
    fn write_new_heartbeat_at(directory: &impl std::os::fd::AsFd, document: &[u8]) {
        let fd = nix::fcntl::openat(
            directory,
            "42.json",
            nix::fcntl::OFlag::O_WRONLY | nix::fcntl::OFlag::O_CREAT | nix::fcntl::OFlag::O_EXCL,
            nix::sys::stat::Mode::from_bits_truncate(0o600),
        )
        .expect("publish live replacement");
        std::fs::File::from(fd)
            .write_all(document)
            .expect("write replacement");
    }

    #[cfg(unix)]
    fn drift_heartbeat_at(directory: &impl std::os::fd::AsFd, name: &str) {
        let fd = nix::fcntl::openat(
            directory,
            name,
            nix::fcntl::OFlag::O_WRONLY | nix::fcntl::OFlag::O_TRUNC,
            nix::sys::stat::Mode::empty(),
        )
        .expect("open moved heartbeat");
        std::fs::File::from(fd).write_all(b"drift").unwrap();
    }

    #[cfg(target_os = "linux")]
    fn mutate_retained(path: &Path, source: &Path, mutation: &str) {
        match mutation.trim_start_matches("cleanup-") {
            "content" => std::fs::write(path, b"drift"),
            "mode" => std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o640)),
            "binding" => std::fs::rename(path, path.with_extension("moved")),
            "source" => std::fs::write(source, b"foreign"),
            _ => unreachable!(),
        }
        .unwrap();
    }
    #[cfg(unix)]
    fn assert_mode(path: &Path, expected: u32) {
        let permissions = std::fs::metadata(path)
            .expect("private path metadata")
            .permissions();
        assert_eq!(permissions.mode() & 0o777, expected);
    }

    #[cfg(unix)]
    #[test]
    fn heartbeat_quarantine_copy_is_private_exact_and_create_new() {
        let (directory, source) = startup_heartbeat_fixture("copy-success");
        let snapshot = expired_heartbeat_snapshot(&source);
        let mut sync_order = Vec::new();
        let target = super::persist_heartbeat_copy_with_hooks(
            &directory,
            &source,
            &snapshot,
            |_| {},
            |boundary| {
                sync_order.push(boundary);
                Ok(())
            },
        )
        .expect("persist quarantine copy");

        assert_eq!(
            sync_order,
            [
                super::HeartbeatCopySyncBoundary::File,
                super::HeartbeatCopySyncBoundary::Directory,
            ]
        );
        assert_eq!(
            std::fs::read(&target).expect("read copy"),
            snapshot.file.document
        );
        assert_eq!(
            std::fs::read(&source).expect("read source"),
            snapshot.file.document
        );
        assert_mode(&directory.join("quarantine"), 0o700);
        assert_mode(&directory.join("quarantine/startup-heartbeats"), 0o700);
        assert_mode(&target, 0o600);
        let duplicate = super::persist_heartbeat_copy(&directory, &source, &snapshot)
            .expect_err("copy target must be create-new");
        assert!(duplicate.to_string().contains("already exists"));
        let flags = nix::fcntl::OFlag::O_RDONLY | nix::fcntl::OFlag::O_DIRECTORY;
        let directory_fd =
            nix::fcntl::open(&directory, flags, nix::sys::stat::Mode::empty()).expect("open root");
        let (pipe, _writer) = nix::unistd::pipe().expect("file-sync pipe");
        let error =
            super::sync_heartbeat_copy(&std::fs::File::from(pipe), &directory_fd, &mut |_| {
                panic!("boundary emitted after failed file sync")
            })
            .expect_err("pipe file cannot sync");
        assert!(error.to_string().contains("sync heartbeat quarantine copy"));

        let file = std::fs::File::options().write(true).open(&source).unwrap();
        let (pipe, _writer) = nix::unistd::pipe().expect("directory-sync pipe");
        let mut boundaries = Vec::new();
        let error = super::sync_heartbeat_copy(&file, &pipe, &mut |boundary| {
            boundaries.push(boundary);
            Ok(())
        })
        .expect_err("pipe directory cannot sync");
        assert!(error
            .to_string()
            .contains("sync heartbeat quarantine directory"));
        assert_eq!(boundaries, [super::HeartbeatCopySyncBoundary::File]);
        std::fs::remove_dir_all(directory).expect("remove heartbeat fixture");
    }

    #[cfg(unix)]
    #[test]
    fn heartbeat_quarantine_copy_rejects_snapshot_drift_deterministically() {
        let (directory, source) = startup_heartbeat_fixture("copy-drift");
        let snapshot = expired_heartbeat_snapshot(&source);
        let mut observed = snapshot.file.clone();
        observed.identity.inode += 1;
        assert!(
            super::revalidate_heartbeat_snapshot(&observed, &snapshot.file)
                .expect_err("identity drift")
                .to_string()
                .contains("identity drift")
        );
        observed = snapshot.file.clone();
        observed.document[0] = b'[';
        let mut document_drift = (*snapshot).clone();
        document_drift.file = observed.clone();
        assert!(
            super::persist_heartbeat_copy(&directory, &source, &document_drift)
                .expect_err("document drift")
                .to_string()
                .contains("document drift")
        );

        std::fs::rename(&source, directory.join("old.json")).expect("move classified inode");
        std::fs::write(&source, &snapshot.file.document).expect("publish replacement inode");
        assert!(
            super::persist_heartbeat_copy(&directory, &source, &snapshot)
                .expect_err("replacement source")
                .to_string()
                .contains("identity drift")
        );
        assert_eq!(
            std::fs::read(&source).expect("replacement survives"),
            snapshot.file.document
        );
        std::fs::remove_dir_all(directory).expect("remove heartbeat fixture");
    }

    #[cfg(unix)]
    #[test]
    fn heartbeat_quarantine_copy_rejects_preexisting_and_swapped_symlink_ancestors() {
        let (preexisting_root, preexisting_source) =
            startup_heartbeat_fixture("copy-preexisting-symlink");
        let preexisting_snapshot = expired_heartbeat_snapshot(&preexisting_source);
        let outside = startup_heartbeat_fixture("copy-outside").0;
        std::fs::set_permissions(&preexisting_root, std::fs::Permissions::from_mode(0o755))
            .expect("make state root non-private");
        assert!(super::persist_heartbeat_copy(
            &preexisting_root,
            &preexisting_source,
            &preexisting_snapshot,
        )
        .expect_err("non-private root")
        .to_string()
        .contains("private state root"));
        std::fs::set_permissions(&preexisting_root, std::fs::Permissions::from_mode(0o700))
            .expect("restore private state root");
        let root_link = preexisting_root.with_extension("root-link");
        std::os::unix::fs::symlink(&preexisting_root, &root_link).expect("state root symlink");
        assert!(super::persist_heartbeat_copy(
            &root_link,
            &preexisting_source,
            &preexisting_snapshot
        )
        .is_err());
        std::fs::remove_file(root_link).expect("remove state root symlink");
        std::os::unix::fs::symlink(&outside, preexisting_root.join("quarantine"))
            .expect("preexisting quarantine symlink");
        assert!(super::persist_heartbeat_copy(
            &preexisting_root,
            &preexisting_source,
            &preexisting_snapshot,
        )
        .is_err());
        assert_eq!(
            std::fs::read_dir(&outside)
                .expect("outside directory")
                .count(),
            0
        );

        let (swapped_root, swapped_source) = startup_heartbeat_fixture("copy-swapped-symlink");
        let swapped_snapshot = expired_heartbeat_snapshot(&swapped_source);
        let mut swapped = false;
        let result = super::persist_heartbeat_copy_with_hooks(
            &swapped_root,
            &swapped_source,
            &swapped_snapshot,
            |component| {
                if component == "quarantine" {
                    std::fs::rename(
                        swapped_root.join("quarantine"),
                        swapped_root.join("displaced"),
                    )
                    .expect("displace opened quarantine");
                    std::os::unix::fs::symlink(&outside, swapped_root.join("quarantine"))
                        .expect("swap quarantine symlink");
                    swapped = true;
                }
            },
            |_| Ok(()),
        );
        assert!(swapped);
        assert!(result.is_err());
        assert_eq!(
            std::fs::read_dir(&outside)
                .expect("outside directory")
                .count(),
            0
        );
        assert_eq!(
            std::fs::read(&swapped_source).expect("source survives"),
            swapped_snapshot.file.document
        );

        std::fs::remove_dir_all(preexisting_root).expect("remove preexisting fixture");
        std::fs::remove_dir_all(swapped_root).expect("remove swapped fixture");
        std::fs::remove_dir_all(outside).expect("remove outside fixture");
    }

    #[cfg(unix)]
    #[test]
    fn stale_heartbeat_quarantine_aborts_for_replacement_before_handoff() {
        let (directory, source) = startup_heartbeat_fixture("handoff-replacement");
        let snapshot = expired_heartbeat_snapshot(&source);
        let replacement = b"new live heartbeat".to_vec();
        let displaced = directory.join("classified.json");
        let result = super::handoff_stale_heartbeat_path_with_hooks(
            &directory,
            &source,
            &snapshot,
            || {
                std::fs::rename(&source, &displaced).expect("displace classified heartbeat");
                std::fs::write(&source, &replacement).expect("publish replacement heartbeat");
            },
            |_, _, _| {},
            |_| Ok(()),
        );

        assert!(result
            .expect_err("replacement must abort")
            .to_string()
            .contains("drift"));
        assert_eq!(
            std::fs::read(&source).expect("replacement survives"),
            replacement
        );
        let copy = heartbeat_copy_path(&directory);
        assert_eq!(
            std::fs::read(copy).expect("durable copy"),
            snapshot.file.document
        );
        std::fs::remove_dir_all(directory).expect("remove heartbeat fixture");
    }

    #[cfg(unix)]
    #[test]
    fn stale_heartbeat_handoff_restores_drift_or_preserves_it_beside_live_replacement() {
        for occupied in [false, true] {
            let (directory, source) = startup_heartbeat_fixture(if occupied {
                "handoff-occupied"
            } else {
                "handoff-restore"
            });
            let snapshot = expired_heartbeat_snapshot(&source);
            let result = super::handoff_stale_heartbeat_path_with_hooks(
                &directory,
                &source,
                &snapshot,
                || {},
                |root, handoff, moved| {
                    drift_heartbeat_at(handoff, moved);
                    if occupied {
                        write_new_heartbeat_at(root, b"replacement");
                    }
                },
                |_| Ok(()),
            );
            assert!(result
                .expect_err("drift must abort")
                .to_string()
                .contains("drift"));
            assert_eq!(
                std::fs::read(&source).expect("live name survives"),
                if occupied {
                    b"replacement".as_slice()
                } else {
                    b"drift".as_slice()
                }
            );
            if !occupied {
                let live = std::fs::metadata(&source).expect("restored heartbeat metadata");
                assert_eq!(
                    std::os::unix::fs::MetadataExt::ino(&live),
                    snapshot.file.identity.inode
                );
            }
            assert_eq!(heartbeat_handoff_count(&directory), usize::from(occupied));
            std::fs::remove_dir_all(directory).expect("remove heartbeat fixture");
        }
    }

    #[cfg(unix)]
    #[test]
    fn stale_heartbeat_handoff_preserves_replacement_before_and_after_cleanup() {
        for mode in 0..3 {
            let (directory, source) = startup_heartbeat_fixture("handoff-sync");
            let snapshot = expired_heartbeat_snapshot(&source);
            let result = super::handoff_stale_heartbeat_path_with_hooks(
                &directory,
                &source,
                &snapshot,
                || {},
                |root, _, _| {
                    if mode == 2 {
                        nix::unistd::mkfifo(
                            &source,
                            nix::sys::stat::Mode::from_bits_truncate(0o600),
                        )
                        .expect("publish live FIFO");
                    } else if mode != 1 {
                        write_new_heartbeat_at(root, &snapshot.file.document);
                    }
                },
                |boundary| {
                    if mode == 1 && boundary == super::HeartbeatHandoffSyncBoundary::Cleanup {
                        std::fs::write(&source, &snapshot.file.document)
                            .expect("publish post-unlink replacement");
                        return Err(super::CommandFailure::diagnostic(
                            "injected cleanup failure",
                        ));
                    }
                    Ok(())
                },
            );

            assert!(result.is_err());
            if mode == 2 {
                let kind = std::fs::symlink_metadata(&source)
                    .expect("FIFO metadata")
                    .file_type();
                assert!(std::os::unix::fs::FileTypeExt::is_fifo(&kind));
            } else {
                assert_eq!(std::fs::read(&source).unwrap(), snapshot.file.document);
            }
            assert!(heartbeat_copy_path(&directory).is_file());
            assert_eq!(heartbeat_handoff_count(&directory), usize::from(mode != 1));
            std::fs::remove_dir_all(directory).expect("remove heartbeat fixture");
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn stale_heartbeat_handoff_failure_atomic() {
        use super::HeartbeatReceiptDecision::{Completed, Pending};

        let boundaries = [
            super::HeartbeatHandoffSyncBoundary::Source,
            super::HeartbeatHandoffSyncBoundary::Handoff,
            super::HeartbeatHandoffSyncBoundary::Cleanup,
            super::HeartbeatHandoffSyncBoundary::RestoreRoot,
            super::HeartbeatHandoffSyncBoundary::RestoreUnlink,
            super::HeartbeatHandoffSyncBoundary::RestoreHandoff,
        ];
        for failed in boundaries {
            let label = format!("atomic-{failed:?}");
            let (parent_path, repo_path, source, repo, snapshot) =
                anchored_startup_heartbeat_fixture(label.as_str());
            let expectation = expected_startup_heartbeat("host:user:rust:4242:nonce-a");
            let result = super::handoff_stale_heartbeat(
                &repo_path,
                &repo,
                source.file_name().unwrap(),
                &snapshot,
                |_, handoff, moved| {
                    if matches!(
                        failed,
                        super::HeartbeatHandoffSyncBoundary::RestoreRoot
                            | super::HeartbeatHandoffSyncBoundary::RestoreUnlink
                            | super::HeartbeatHandoffSyncBoundary::RestoreHandoff
                    ) {
                        drift_heartbeat_at(handoff, moved);
                    }
                },
                |boundary| {
                    if failed == boundary {
                        Err(super::CommandFailure::diagnostic("injected sync failure"))
                    } else {
                        Ok(())
                    }
                },
                |directory| {
                    nix::unistd::fsync(directory).map_err(|error| {
                        super::CommandFailure::diagnostic(format!("cleanup fsync: {error}"))
                    })
                },
            );
            assert!(result.is_err(), "{failed:?} must abort");
            assert_eq!(
                super::heartbeat_receipt_retry_decision(&repo, expectation),
                Pending
            );
            let handoff = repo_path.join("quarantine/startup-heartbeat-handoffs");
            assert!(
                source.exists() || std::fs::read_dir(handoff).unwrap().count() > 1,
                "{failed:?} removed live and moved identities"
            );
            std::fs::remove_dir_all(parent_path).unwrap();
        }

        for mode in 0..3 {
            let (parent_path, repo_path, source, repo, snapshot) =
                anchored_startup_heartbeat_fixture("atomic-drift");
            let expectation = expected_startup_heartbeat("host:user:rust:4242:nonce-a");
            let result = super::handoff_stale_heartbeat(
                &repo_path,
                &repo,
                source.file_name().unwrap(),
                &snapshot,
                |root, handoff, moved| match mode {
                    0 => {
                        nix::unistd::unlinkat(
                            handoff,
                            moved,
                            nix::unistd::UnlinkatFlags::NoRemoveDir,
                        )
                        .unwrap();
                    }
                    1 => {
                        drift_heartbeat_at(handoff, moved);
                        write_new_heartbeat_at(root, b"replacement");
                    }
                    _ => {}
                },
                |_| Ok(()),
                |directory| {
                    nix::unistd::fsync(directory).map_err(|error| {
                        super::CommandFailure::diagnostic(format!("cleanup fsync: {error}"))
                    })
                },
            );
            assert_eq!(
                super::heartbeat_receipt_retry_decision(&repo, expectation),
                if mode == 2 { Completed } else { Pending }
            );
            assert_eq!(result.is_ok(), mode == 2);
            if mode == 1 {
                assert_eq!(std::fs::read(&source).unwrap(), b"replacement");
            }
            std::fs::remove_dir_all(parent_path).unwrap();
        }

        let (parent_path, repo_path, source, repo, snapshot) =
            anchored_startup_heartbeat_fixture("atomic-real-fsync");
        let expectation = expected_startup_heartbeat("host:user:rust:4242:nonce-a");
        let (pipe, _writer) = nix::unistd::pipe().unwrap();
        assert!(super::handoff_stale_heartbeat(
            &repo_path,
            &repo,
            source.file_name().unwrap(),
            &snapshot,
            |_, _, _| {},
            |_| Ok(()),
            |_| {
                nix::unistd::fsync(&pipe)
                    .map_err(|error| super::CommandFailure::diagnostic(format!("fsync: {error}")))
            },
        )
        .is_err());
        assert_eq!(
            super::heartbeat_receipt_retry_decision(&repo, expectation),
            Pending
        );
        let handoff = repo_path.join("quarantine/startup-heartbeat-handoffs");
        assert!(source.exists() || std::fs::read_dir(handoff).unwrap().count() > 1);
        std::fs::remove_dir_all(parent_path).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn startup_heartbeat_retained_handoff() {
        use std::os::unix::fs::MetadataExt;
        for race in
            "before-check before-rename after-rename collision malformed fifo hardlink".split(' ')
        {
            let (parent, repo_path, source, repo, snapshot) =
                anchored_startup_heartbeat_fixture(race);
            let replacement = repo_path.join("replacement");
            std::fs::write(&replacement, b"foreign").unwrap();
            std::fs::set_permissions(&replacement, std::fs::Permissions::from_mode(0o600)).unwrap();
            match race {
                "malformed" => std::fs::write(&source, [123]).unwrap(),
                "fifo" => {
                    std::fs::remove_file(&source).unwrap();
                    nix::unistd::mkfifo(&source, nix::sys::stat::Mode::from_bits_truncate(0o600))
                        .unwrap();
                }
                "hardlink" => std::fs::hard_link(&source, source.with_extension("link")).unwrap(),
                _ => {}
            }
            let result = super::handoff_retained_heartbeat(
                &repo_path,
                &repo,
                source.file_name().unwrap(),
                &snapshot,
                |boundary, _, handoff, name| {
                    if race == "collision" && boundary == "before-check" {
                        let file = nix::fcntl::openat(
                            handoff,
                            name,
                            nix::fcntl::OFlag::O_WRONLY
                                | nix::fcntl::OFlag::O_CREAT
                                | nix::fcntl::OFlag::O_EXCL,
                            nix::sys::stat::Mode::from_bits_truncate(0o600),
                        )
                        .unwrap();
                        std::fs::File::from(file).write_all(b"foreign").unwrap();
                    } else if boundary == race {
                        if race == "after-rename" {
                            std::fs::write(&source, b"foreign").unwrap();
                            std::fs::set_permissions(
                                &source,
                                std::fs::Permissions::from_mode(0o600),
                            )
                            .unwrap();
                        } else {
                            std::fs::rename(&replacement, &source).unwrap();
                        }
                    }
                },
                |_| Ok(()),
            );
            assert!(result.is_err(), "{race}");
            if matches!(race, "before-check" | "before-rename" | "after-rename") {
                assert_eq!(std::fs::read(&source).unwrap(), b"foreign", "{race}");
            }
            std::fs::remove_dir_all(parent).unwrap();
        }

        for failed in [
            super::HeartbeatHandoffSyncBoundary::Source,
            super::HeartbeatHandoffSyncBoundary::Handoff,
        ] {
            let (parent, repo_path, source, repo, snapshot) =
                anchored_startup_heartbeat_fixture("retained-fsync");
            let inode = std::fs::metadata(&source).unwrap().ino();
            assert!(super::handoff_retained_heartbeat(
                &repo_path,
                &repo,
                source.file_name().unwrap(),
                &snapshot,
                |_, _, _, _| {},
                |boundary| (boundary != failed)
                    .then_some(())
                    .ok_or_else(|| super::CommandFailure::diagnostic("injected fsync failure")),
            )
            .is_err());
            for _ in 0..2 {
                let retained = super::handoff_retained_heartbeat(
                    &repo_path,
                    &repo,
                    source.file_name().unwrap(),
                    &snapshot,
                    |_, _, _, _| {},
                    |_| Ok(()),
                )
                .unwrap();
                assert_eq!(std::fs::metadata(retained).unwrap().ino(), inode);
            }
            std::fs::remove_dir_all(parent).unwrap();
        }

        for mutation in [
            "content",
            "mode",
            "binding",
            "cleanup-content",
            "cleanup-mode",
            "cleanup-binding",
            "cleanup-source",
        ] {
            let (parent, repo_path, source, repo, snapshot) =
                anchored_startup_heartbeat_fixture(mutation);
            let expected = expected_startup_heartbeat("host:user:rust:4242:nonce-a");
            let (_, completed) = super::heartbeat_receipt_names(expected);
            let archive = repo_path
                .join("quarantine/startup-heartbeat-handoffs")
                .join(completed.replace(".receipt", ".json"));
            let result = super::handoff_retained_heartbeat(
                &repo_path,
                &repo,
                source.file_name().unwrap(),
                &snapshot,
                |boundary, _, _, _| {
                    if boundary == "final" && !mutation.starts_with("cleanup-") {
                        mutate_retained(&archive, &source, mutation);
                    }
                },
                |boundary| {
                    if boundary == super::HeartbeatHandoffSyncBoundary::Cleanup
                        && mutation.starts_with("cleanup-")
                    {
                        mutate_retained(&archive, &source, mutation);
                    }
                    Ok(())
                },
            );
            assert!(result.is_err(), "{mutation}");
            let receipts = super::open_receipt_anchors_with_hook(&repo, |_| {}).unwrap();
            assert!(
                super::inspect_heartbeat_receipt(&receipts, completed.as_ref())
                    == super::HeartbeatReceiptEntry::Missing,
                "{mutation}"
            );
            std::fs::remove_dir_all(parent).unwrap();
        }
    }

    #[test]
    fn paginated_comments_parser_flattens_two_raw_pages() {
        // Break caught: treating each slurped page array as a comment object.
        let comments = super::parse_paginated_comments_json(
            r#"[
              [{"id":100,"body":"first","updated_at":"2026-07-27T00:00:00Z","user":{"login":"autospec"}}],
              [{"id":101,"body":null,"updated_at":null,"reactions":{"total_count":2}}]
            ]"#,
        )
        .expect("nested GitHub comment pages");

        assert_eq!(
            comments,
            vec![
                RemoteComment::new(100, "first", "2026-07-27T00:00:00Z"),
                RemoteComment::new(101, "", ""),
            ]
        );
    }

    #[test]
    fn paginated_comments_parser_preserves_flat_single_page_fixtures() {
        // Break caught: requiring an outer page array would break existing projected fixtures.
        let comments = super::parse_paginated_comments_json(
            r#"[{"id":100,"body":"state","updated_at":"2026-07-27T00:00:00Z"}]"#,
        )
        .expect("flat projected fixture");

        assert_eq!(
            comments,
            vec![RemoteComment::new(100, "state", "2026-07-27T00:00:00Z")]
        );
    }

    #[test]
    fn paginated_comments_parser_rejects_mixed_pages_and_malformed_fields() {
        // Break caught: partially flattening a mixed payload could silently drop comments.
        assert!(super::parse_paginated_comments_json(
            r#"[[{"id":100,"body":"state","updated_at":"now"}],{"id":101,"body":"other","updated_at":"now"}]"#,
        )
        .is_err());

        for malformed in [
            r#"[{"id":"100","body":"state","updated_at":"now"}]"#,
            r#"[{"id":100,"body":false,"updated_at":"now"}]"#,
            r#"[{"id":100,"body":"state","updated_at":[]}]"#,
        ] {
            assert!(
                super::parse_paginated_comments_json(malformed).is_err(),
                "accepted malformed typed comment: {malformed}"
            );
        }
    }

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

    #[test]
    fn session_binding_is_create_once_and_idempotent_for_the_same_identity() {
        let directory = std::env::temp_dir().join(format!(
            "autospec-session-binding-{}-{}",
            std::process::id(),
            super::UNIQUE_ID_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&directory).expect("session binding fixture");
        let path = directory.join("session.json");
        let original = SessionBindingIdentity::new("session", 42, "worker", "feat/a", "claim-a");
        let successor = SessionBindingIdentity::new("session", 42, "worker", "feat/a", "claim-b");

        let original_document = br#"{"issue":"42","branch":"feat/a","worker_id":"worker","claim_id":"claim-a","session_id":"session"}"#;
        let refresh_document = br#"{"issue":"42","branch":"feat/a","worker_id":"worker","claim_id":"claim-a","session_id":"session","step":"refresh"}"#;
        let successor_document = br#"{"issue":"42","branch":"feat/a","worker_id":"worker","claim_id":"claim-b","session_id":"session"}"#;
        publish_session_binding(&path, original_document, &original).expect("initial binding");
        publish_session_binding(&path, refresh_document, &original).expect("idempotent refresh");
        assert!(publish_session_binding(&path, successor_document, &successor).is_err());
        assert_eq!(
            std::fs::read(&path).expect("preserved binding"),
            original_document
        );
        std::fs::remove_dir_all(directory).expect("remove session binding fixture");
    }

    #[test]
    fn concurrent_session_binding_publishers_cannot_overwrite_the_winner() {
        let directory = std::env::temp_dir().join(format!(
            "autospec-session-binding-race-{}-{}",
            std::process::id(),
            super::UNIQUE_ID_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&directory).expect("session binding fixture");
        let path = Arc::new(directory.join("session.json"));
        let barrier = Arc::new(Barrier::new(3));
        let handles = [
            (
                "claim-a",
                br#"{"issue":"42","branch":"feat/a","worker_id":"worker","claim_id":"claim-a","session_id":"session"}"#.as_slice(),
            ),
            (
                "claim-b",
                br#"{"issue":"42","branch":"feat/a","worker_id":"worker","claim_id":"claim-b","session_id":"session"}"#.as_slice(),
            ),
        ]
            .into_iter()
            .map(|(claim_id, document)| {
                let path = Arc::clone(&path);
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    let identity =
                        SessionBindingIdentity::new("session", 42, "worker", "feat/a", claim_id);
                    barrier.wait();
                    publish_session_binding(&path, document, &identity)
                })
            })
            .collect::<Vec<_>>();
        barrier.wait();
        let results = handles
            .into_iter()
            .map(|handle| handle.join().expect("publisher thread"))
            .collect::<Vec<_>>();

        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        let stored = std::fs::read_to_string(&*path).expect("winning session binding");
        assert!(stored.contains("claim-a") || stored.contains("claim-b"));
        std::fs::remove_dir_all(directory).expect("remove session binding fixture");
    }

    struct ClaimRefFixture {
        root: PathBuf,
        remote: PathBuf,
        clients: [PathBuf; 2],
    }

    impl ClaimRefFixture {
        fn new(label: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "autospec-claim-ref-{label}-{}-{}",
                std::process::id(),
                super::UNIQUE_ID_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            let remote = root.join("remote.git");
            std::fs::create_dir_all(&root).expect("claim ref fixture root");
            git(&root, &["init", "--bare", remote.to_str().unwrap()]);
            let clients = [root.join("client-a"), root.join("client-b")];
            for client in &clients {
                git(&root, &["init", client.to_str().unwrap()]);
            }
            Self {
                root,
                remote,
                clients,
            }
        }
    }

    impl Drop for ClaimRefFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn git(directory: &Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(directory)
            .output()
            .expect("run git fixture command");
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn git_stdout(directory: &Path, args: &[&str]) -> Vec<u8> {
        let output = Command::new("git")
            .args(args)
            .current_dir(directory)
            .output()
            .expect("run git fixture command");
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        output.stdout
    }

    fn source_function<'a>(source: &'a str, name: &str) -> &'a str {
        let marker = format!("fn {name}(");
        let start = source.find(&marker).expect("source function");
        let tail = &source[start..];
        let end = tail[marker.len()..]
            .find("\nfn ")
            .map(|end| marker.len() + end)
            .unwrap_or(tail.len());
        &tail[..end]
    }

    fn claim_record(worker: &str, claim_id: &str, state: &str) -> RunStateRecord {
        RunStateRecord::new(
            "owner/repo",
            42,
            worker,
            state,
            format!("feat/{worker}"),
            "",
            state,
            Vec::new(),
            "2026-07-25T00:00:00Z",
            "2026-07-25T00:00:00Z",
            1,
        )
        .with_claim_id(claim_id)
    }

    fn lifecycle_evidence(record: &RunStateRecord) -> Result<ClaimEvidence, super::CommandFailure> {
        lifecycle_claim_evidence_from_record(
            RepositoryScope::try_from("owner/repo").expect("repository scope"),
            IssueNumber::new(42).expect("issue"),
            WorkerId::try_from("worker-requested").expect("requested worker"),
            ClaimBranch::try_from("feat/requested").expect("requested branch"),
            record,
        )
    }

    #[test]
    fn lifecycle_claim_state_matrix_reuses_only_valid_available_generations() {
        let requested = ClaimEvidence::Observed(ClaimContext::active(
            RepositoryScope::try_from("owner/repo").expect("repository scope"),
            IssueNumber::new(42).expect("issue"),
            WorkerId::try_from("worker-requested").expect("requested worker"),
            ClaimBranch::try_from("feat/requested").expect("requested branch"),
            LeaseFreshness::Fresh,
        ));
        for state in ["released", "retryable"] {
            assert_eq!(
                lifecycle_evidence(&claim_record("worker-old", "claim-old", state))
                    .expect("reusable claim evidence"),
                requested,
                "{state} must be available to the requested owner"
            );
        }

        for state in ["claimed", "failed", "needs-human"] {
            let mut record = claim_record("worker-old", "claim-old", state);
            record.updated_at = "2999-01-01T00:00:00Z".to_string();
            assert_eq!(
                lifecycle_evidence(&record).expect("active claim evidence"),
                ClaimEvidence::Observed(ClaimContext::active(
                    RepositoryScope::try_from("owner/repo").expect("repository scope"),
                    IssueNumber::new(42).expect("issue"),
                    WorkerId::try_from("worker-old").expect("recorded worker"),
                    ClaimBranch::try_from("feat/worker-old").expect("recorded branch"),
                    LeaseFreshness::Fresh,
                )),
                "{state} must retain the recorded owner"
            );
        }

        assert_eq!(
            lifecycle_evidence(&claim_record("worker-old", "claim-old", "merged"))
                .expect("merged claim evidence"),
            ClaimEvidence::Observed(ClaimContext::terminal(
                RepositoryScope::try_from("owner/repo").expect("repository scope"),
                IssueNumber::new(42).expect("issue"),
                WorkerId::try_from("worker-old").expect("recorded worker"),
                ClaimBranch::try_from("feat/worker-old").expect("recorded branch"),
            ))
        );
    }

    #[test]
    fn lifecycle_claim_rejects_malformed_reusable_generations() {
        let mut empty_branch = claim_record("worker-old", "claim-old", "released");
        empty_branch.branch.clear();
        assert!(lifecycle_evidence(&empty_branch).is_err());

        let mut missing_generation = claim_record("worker-old", "claim-old", "retryable");
        missing_generation.claim_id = None;
        assert!(lifecycle_evidence(&missing_generation).is_err());
    }

    #[test]
    fn bridge_terminal_projection_resumes_only_the_exact_terminal_generation() {
        let identity = super::ClaimMutationIdentity {
            repo: "owner/repo",
            issue: 42,
            worker_id: "worker-a",
            branch: "feat/worker-a",
            claim_id: "claim-a",
        };
        let mut terminal = claim_record("worker-a", "claim-a", "merged");
        terminal.step = "merged".into();
        terminal.pr = "17".into();
        assert_eq!(
            super::bridge_terminal_mode(
                &terminal,
                identity,
                "merged",
                "merged",
                "17",
                "terminal_prepared:merged",
            ),
            super::BridgeTerminalMode::Complete
        );

        terminal.claim_id = Some("claim-takeover".into());
        assert_eq!(
            super::bridge_terminal_mode(
                &terminal,
                identity,
                "merged",
                "merged",
                "17",
                "terminal_prepared:merged",
            ),
            super::BridgeTerminalMode::Lost
        );
    }

    #[test]
    fn bridge_result_recovery_collapses_only_identical_publication_retries() {
        let evidence = ExecutorResultEvidence::new(
            "owner/repo",
            42,
            "worker-a",
            "feat/worker-a",
            "succeeded",
            Some(17),
            "executor_succeeded",
            "stable-receipt",
            Some("claim-a".into()),
            Some("a".repeat(40)),
            Some("b".repeat(64)),
        );
        let body = evidence.to_marked_comment();
        let comments = vec![
            RemoteComment::new(1, body.clone(), "2026-07-26T00:00:00Z"),
            RemoteComment::new(2, body, "2026-07-26T00:00:01Z"),
        ];
        assert_eq!(
            super::exact_successful_executor_result(
                &comments,
                super::ClaimMutationIdentity {
                    repo: "owner/repo",
                    issue: 42,
                    worker_id: "worker-a",
                    branch: "feat/worker-a",
                    claim_id: "claim-a",
                },
                super::ExecutorResultAuthorityBinding {
                    pull_request: 17,
                    commit: &"a".repeat(40),
                    premerge_receipt: &"b".repeat(64),
                    receipt_id: "stable-receipt",
                },
            ),
            Some(evidence)
        );
    }

    #[test]
    fn bridge_result_authority_filters_exact_generation_before_uniqueness() {
        let old = ExecutorResultEvidence::new(
            "owner/repo",
            42,
            "worker-a",
            "feat/worker-a",
            "succeeded",
            Some(17),
            "executor_succeeded",
            "bridge-old",
            Some("claim-old".into()),
            Some("a".repeat(40)),
            Some("b".repeat(64)),
        );
        let successor = ExecutorResultEvidence::new(
            "owner/repo",
            42,
            "worker-a",
            "feat/worker-a",
            "succeeded",
            Some(17),
            "executor_succeeded",
            "bridge-successor",
            Some("claim-successor".into()),
            Some("a".repeat(40)),
            Some("b".repeat(64)),
        );
        let comments = vec![
            RemoteComment::new(1, old.to_marked_comment(), "2026-07-26T00:00:00Z"),
            RemoteComment::new(2, successor.to_marked_comment(), "2026-07-26T00:00:01Z"),
            RemoteComment::new(3, successor.to_marked_comment(), "2026-07-26T00:00:02Z"),
        ];

        assert_eq!(
            super::exact_successful_executor_result(
                &comments,
                super::ClaimMutationIdentity {
                    repo: "owner/repo",
                    issue: 42,
                    worker_id: "worker-a",
                    branch: "feat/worker-a",
                    claim_id: "claim-successor",
                },
                super::ExecutorResultAuthorityBinding {
                    pull_request: 17,
                    commit: &"a".repeat(40),
                    premerge_receipt: &"b".repeat(64),
                    receipt_id: "bridge-successor",
                },
            ),
            Some(successor)
        );
    }

    #[cfg(unix)]
    #[test]
    fn executor_result_takeover_after_renewal_is_inert_for_successor_authority() {
        let _guard = BRIDGE_TRANSITION_ENV.lock().expect("transition env");
        let fixture = ClaimRefFixture::new("executor-result-takeover");
        let bin = fixture.root.join("bin");
        let gh = bin.join("gh");
        let comments = fixture.root.join("comments.json");
        let observed_renewal = fixture.root.join("observed-renewal.txt");
        std::fs::create_dir(&bin).expect("bin");
        std::fs::write(&comments, "[]").expect("comments");
        std::fs::write(
            &gh,
            "#!/bin/sh\n\
             set -eu\n\
             if [ \"$1\" = api ]; then cat \"$GH_COMMENTS\"; exit 0; fi\n\
             if [ \"$1 $2\" = 'pr list' ]; then cat <<'EOF'\n\
             [{\"number\":17,\"body\":\"Closes #42\\n\\n## Closeout report\\nResult\",\"headRefName\":\"feat/worker-a\",\"headRefOid\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",\"isDraft\":false,\"baseRefName\":\"main\"}]\n\
             EOF\n\
             exit 0; fi\n\
             if [ \"$1 $2\" = 'issue comment' ]; then\n\
               body=''; while [ \"$#\" -gt 0 ]; do if [ \"$1\" = --body ]; then shift; body=$1; fi; shift || true; done\n\
               jq --arg body \"$body\" '. + [{id:(length + 1),body:$body,updated_at:\"2026-07-26T00:00:00Z\"}]' \"$GH_COMMENTS\" > \"$GH_COMMENTS.tmp\"\n\
               mv \"$GH_COMMENTS.tmp\" \"$GH_COMMENTS\"\n\
               current=$(git --git-dir=\"$GH_REMOTE\" rev-parse refs/autospec/claims/issue-42)\n\
               git --git-dir=\"$GH_REMOTE\" show -s --format=%B \"$current\" > \"$GH_OBSERVED_RENEWAL\"\n\
               git --git-dir=\"$GH_REMOTE\" update-ref refs/autospec/claims/issue-42 \"$GH_TAKEOVER_OID\"\n\
               exit 0\n\
             fi\n\
             exit 64\n",
        )
        .expect("gh");
        std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).expect("gh mode");

        let mut original = claim_record("worker-a", "claim-old", "claimed");
        original.ttl_seconds = u64::MAX;
        let initial = match advance_claim_ref_in(
            Path::new("git"),
            &fixture.clients[0],
            fixture.remote.to_str().unwrap(),
            "owner/repo",
            42,
            None,
            &original,
        )
        .expect("seed claim")
        {
            ClaimRefAdvance::Won(head) => head,
            ClaimRefAdvance::Lost => panic!("initial claim must win"),
        };
        git(
            &fixture.clients[1],
            &[
                "fetch",
                fixture.remote.to_str().unwrap(),
                "refs/autospec/claims/issue-42",
            ],
        );
        let mut successor = claim_record("worker-a", "claim-successor", "claimed");
        successor.ttl_seconds = u64::MAX;
        let successor_oid = create_claim_ref_commit(
            Path::new("git"),
            &fixture.clients[1],
            Some(&initial),
            "successor-generation",
            &successor,
        )
        .expect("create successor claim");
        git(
            &fixture.clients[1],
            &[
                "push",
                fixture.remote.to_str().unwrap(),
                &format!("{successor_oid}:refs/autospec/test/successor"),
            ],
        );

        let old_path = std::env::var_os("PATH");
        let old_remote = std::env::var_os("AUTOSPEC_CLAIM_GIT_REMOTE");
        let old_state = std::env::var_os("AUTOSPEC_CLAIM_GIT_STATE_DIR");
        let old_comments = std::env::var_os("GH_COMMENTS");
        let old_gh_remote = std::env::var_os("GH_REMOTE");
        let old_takeover = std::env::var_os("GH_TAKEOVER_OID");
        let old_observed = std::env::var_os("GH_OBSERVED_RENEWAL");
        let old_retries = std::env::var_os("AUTOSPEC_GH_API_RETRIES");
        std::env::set_var(
            "PATH",
            format!(
                "{}:{}",
                bin.display(),
                old_path.as_deref().unwrap_or_default().to_string_lossy()
            ),
        );
        std::env::set_var("AUTOSPEC_CLAIM_GIT_REMOTE", &fixture.remote);
        std::env::set_var(
            "AUTOSPEC_CLAIM_GIT_STATE_DIR",
            fixture.root.join("claim-state"),
        );
        std::env::set_var("GH_COMMENTS", &comments);
        std::env::set_var("GH_REMOTE", &fixture.remote);
        std::env::set_var("GH_TAKEOVER_OID", &successor_oid);
        std::env::set_var("GH_OBSERVED_RENEWAL", &observed_renewal);
        std::env::set_var("AUTOSPEC_GH_API_RETRIES", "1");

        let old_identity = super::ClaimMutationIdentity {
            repo: "owner/repo",
            issue: 42,
            worker_id: "worker-a",
            branch: "feat/worker-a",
            claim_id: "claim-old",
        };
        assert_eq!(
            super::record_executor_result_with_receipt(
                old_identity,
                &autospec_core::coordination::ConductorOutcome::Succeeded,
                Some(17),
                Some(super::ExecutorSuccessBinding {
                    claim_id: "claim-old",
                    commit: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    premerge_receipt:
                        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                }),
                Some("bridge-old"),
            )
            .expect("publish stale result"),
            super::ExecutorResultRecord::OwnershipLost
        );
        let renewal = std::fs::read_to_string(&observed_renewal).expect("renewal observation");
        assert!(
            renewal.contains("\"step\":\"executor_succeeded\""),
            "{renewal}"
        );
        assert!(!renewal.contains("\"updated_at\":\"2026-07-25T00:00:00Z\""));

        let successor_evidence = ExecutorResultEvidence::new(
            "owner/repo",
            42,
            "worker-a",
            "feat/worker-a",
            "succeeded",
            Some(17),
            "executor_succeeded",
            "bridge-successor",
            Some("claim-successor".into()),
            Some("a".repeat(40)),
            Some("b".repeat(64)),
        );
        let mut published: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&comments).expect("comments"))
                .expect("comments JSON");
        published
            .as_array_mut()
            .expect("comment array")
            .push(serde_json::json!({
                "id": 2,
                "body": successor_evidence.to_marked_comment(),
                "updated_at": "2026-07-26T00:00:01Z"
            }));
        std::fs::write(
            &comments,
            serde_json::to_vec(&published).expect("serialize comments"),
        )
        .expect("publish successor evidence");

        assert_eq!(
            super::authoritative_executor_result(
                super::ClaimMutationIdentity {
                    repo: "owner/repo",
                    issue: 42,
                    worker_id: "worker-a",
                    branch: "feat/worker-a",
                    claim_id: "claim-successor",
                },
                super::ExecutorResultAuthorityBinding {
                    pull_request: 17,
                    commit: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    premerge_receipt:
                        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                    receipt_id: "bridge-successor",
                },
            )
            .expect("read successor authority"),
            Some(successor_evidence)
        );

        for (key, value) in [
            ("PATH", old_path),
            ("AUTOSPEC_CLAIM_GIT_REMOTE", old_remote),
            ("AUTOSPEC_CLAIM_GIT_STATE_DIR", old_state),
            ("GH_COMMENTS", old_comments),
            ("GH_REMOTE", old_gh_remote),
            ("GH_TAKEOVER_OID", old_takeover),
            ("GH_OBSERVED_RENEWAL", old_observed),
            ("AUTOSPEC_GH_API_RETRIES", old_retries),
        ] {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }

    #[cfg(unix)]
    fn assert_bridge_transition_projection(
        label: &str,
        disposition: super::BridgeClaimDisposition,
        expected_state: &str,
        expected_prepared_step: &str,
        expected_edit: &str,
        expected_comments: usize,
        interrupt_after_preparation: bool,
    ) {
        let fixture = ClaimRefFixture::new(label);
        let bin = fixture.root.join("bin");
        let gh = bin.join("gh");
        let calls = fixture.root.join("gh-calls");
        let comments = fixture.root.join("comments.json");
        let label_claims = fixture.root.join("label-claims");
        let first_label_failed = fixture.root.join("first-label-failed");
        std::fs::create_dir(&bin).expect("bin");
        std::fs::write(&comments, "[]").expect("comments");
        std::fs::write(
            &gh,
            "#!/bin/sh\n\
             set -eu\n\
             printf '%s\\n' \"$*\" >> \"$GH_CALLS\"\n\
             if [ \"$1\" = api ]; then cat \"$GH_COMMENTS\"; exit 0; fi\n\
             if [ \"$1 $2\" = 'issue comment' ]; then\n\
               body=''; while [ \"$#\" -gt 0 ]; do if [ \"$1\" = --body ]; then shift; body=$1; fi; shift || true; done\n\
               jq --arg body \"$body\" '. + [{id:(length + 1),body:$body,updated_at:\"2026-07-26T00:00:00Z\"}]' \"$GH_COMMENTS\" > \"$GH_COMMENTS.tmp\"\n\
               mv \"$GH_COMMENTS.tmp\" \"$GH_COMMENTS\"; exit 0\n\
             fi\n\
             if [ \"$1 $2\" = 'issue edit' ]; then\n\
               current=$(git -C \"$GH_REMOTE\" rev-parse refs/autospec/claims/issue-42)\n\
               printf '%s\\n' \"$current\" >> \"$GH_LABEL_CLAIMS\"\n\
               if [ \"${GH_FAIL_FIRST_LABEL:-0}\" = 1 ] && [ ! -e \"$GH_FIRST_LABEL_FAILED\" ]; then\n\
                 : > \"$GH_FIRST_LABEL_FAILED\"\n\
                 exit 23\n\
               fi\n\
               exit 0\n\
             fi\n\
             exit 64\n",
        )
        .expect("gh");
        std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).expect("gh mode");
        let old_path = std::env::var_os("PATH");
        let old_remote = std::env::var_os("AUTOSPEC_CLAIM_GIT_REMOTE");
        let old_state = std::env::var_os("AUTOSPEC_CLAIM_GIT_STATE_DIR");
        let old_calls = std::env::var_os("GH_CALLS");
        let old_comments = std::env::var_os("GH_COMMENTS");
        let old_gh_remote = std::env::var_os("GH_REMOTE");
        let old_label_claims = std::env::var_os("GH_LABEL_CLAIMS");
        let old_fail_first_label = std::env::var_os("GH_FAIL_FIRST_LABEL");
        let old_first_label_failed = std::env::var_os("GH_FIRST_LABEL_FAILED");
        let old_retries = std::env::var_os("AUTOSPEC_GH_API_RETRIES");
        let old_heartbeat = std::env::var_os("AUTOSPEC_HEARTBEAT_DIR");
        std::env::set_var(
            "PATH",
            format!(
                "{}:{}",
                bin.display(),
                old_path.as_deref().unwrap_or_default().to_string_lossy()
            ),
        );
        std::env::set_var("AUTOSPEC_CLAIM_GIT_REMOTE", &fixture.remote);
        std::env::set_var(
            "AUTOSPEC_CLAIM_GIT_STATE_DIR",
            fixture.root.join("claim-state"),
        );
        std::env::set_var("GH_CALLS", &calls);
        std::env::set_var("GH_COMMENTS", &comments);
        std::env::set_var("GH_REMOTE", &fixture.remote);
        std::env::set_var("GH_LABEL_CLAIMS", &label_claims);
        std::env::set_var(
            "GH_FAIL_FIRST_LABEL",
            if interrupt_after_preparation {
                "1"
            } else {
                "0"
            },
        );
        std::env::set_var("GH_FIRST_LABEL_FAILED", &first_label_failed);
        std::env::set_var("AUTOSPEC_GH_API_RETRIES", "1");
        std::env::set_var("AUTOSPEC_HEARTBEAT_DIR", fixture.root.join("heartbeats"));

        let claimed = claim_record("worker-a", "claim-a", "claimed");
        assert!(matches!(
            advance_claim_ref_in(
                Path::new("git"),
                &fixture.clients[0],
                fixture.remote.to_str().unwrap(),
                "owner/repo",
                42,
                None,
                &claimed,
            )
            .expect("seed authoritative claim"),
            ClaimRefAdvance::Won(_)
        ));
        let identity = super::ClaimMutationIdentity {
            repo: "owner/repo",
            issue: 42,
            worker_id: "worker-a",
            branch: "feat/worker-a",
            claim_id: "claim-a",
        };
        if disposition == super::BridgeClaimDisposition::Retryable {
            super::write_startup_heartbeat(
                identity.repo,
                identity.issue,
                identity.worker_id,
                identity.branch,
                identity.claim_id,
                None,
            )
            .expect("retryable heartbeat");
        }
        let pr = (disposition == super::BridgeClaimDisposition::Merged).then_some(17);
        if interrupt_after_preparation {
            super::transition_bridge_claim(identity, pr, disposition)
                .expect_err("first label projection must interrupt after preparation");
            let prepared = super::read_claim_ref("owner/repo", 42)
                .expect("read prepared claim")
                .expect("prepared claim head");
            assert_eq!(prepared.record.state, "claimed");
            assert_eq!(prepared.record.step, expected_prepared_step);
        }
        assert_eq!(
            super::transition_bridge_claim(identity, pr, disposition)
                .expect("transition or prepared restart"),
            super::BridgeClaimTransition::Transitioned
        );
        assert_eq!(
            super::transition_bridge_claim(identity, pr, disposition).expect("resume projection"),
            super::BridgeClaimTransition::Transitioned
        );
        let head = super::read_claim_ref("owner/repo", 42)
            .expect("read claim")
            .expect("claim head");
        assert_eq!(head.record.state, expected_state);
        assert_ne!(head.record.state, "claimed");
        let call_log = std::fs::read_to_string(calls).expect("calls");
        assert!(call_log.contains(expected_edit), "{call_log}");
        assert_eq!(
            call_log.matches(expected_edit).count(),
            if interrupt_after_preparation { 2 } else { 1 },
            "terminal restart must not reapply labels: {call_log}"
        );
        let label_claim_oid =
            std::fs::read_to_string(&label_claims).expect("claim observed during label projection");
        let label_claim_oid = label_claim_oid.lines().next().expect("label claim oid");
        let label_claim = String::from_utf8(git_stdout(
            &fixture.root,
            &[
                "-C",
                fixture.remote.to_str().expect("remote path"),
                "cat-file",
                "commit",
                label_claim_oid,
            ],
        ))
        .expect("claim commit utf8");
        let (_, label_claim) = label_claim
            .split_once("\n\n")
            .expect("claim commit message");
        let prepared =
            super::parse_claim_ref_message("a".repeat(40), label_claim, "owner/repo", 42)
                .expect("prepared terminal claim");
        assert_eq!(prepared.record.state, "claimed");
        assert_eq!(prepared.record.step, expected_prepared_step);
        let comment_value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(comments).expect("comments JSON"))
                .expect("comments");
        assert_eq!(
            comment_value.as_array().expect("comment array").len(),
            expected_comments,
            "restart duplicated a projection"
        );

        for (key, value) in [
            ("PATH", old_path),
            ("AUTOSPEC_CLAIM_GIT_REMOTE", old_remote),
            ("AUTOSPEC_CLAIM_GIT_STATE_DIR", old_state),
            ("GH_CALLS", old_calls),
            ("GH_COMMENTS", old_comments),
            ("GH_REMOTE", old_gh_remote),
            ("GH_LABEL_CLAIMS", old_label_claims),
            ("GH_FAIL_FIRST_LABEL", old_fail_first_label),
            ("GH_FIRST_LABEL_FAILED", old_first_label_failed),
            ("AUTOSPEC_GH_API_RETRIES", old_retries),
            ("AUTOSPEC_HEARTBEAT_DIR", old_heartbeat),
        ] {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn bridge_terminal_transitions_prepare_before_labels_and_restarts_do_not_relabel() {
        let _guard = BRIDGE_TRANSITION_ENV.lock().expect("transition env");
        let retryable_edit = [
            "issue edit 42 ",
            "repo owner/repo ",
            "remove-label in-progress-by-bot ",
            "remove-label autospec:needs-human ",
            "add-label auto-implement",
        ]
        .join("--");
        assert_bridge_transition_projection(
            "bridge-retryable",
            super::BridgeClaimDisposition::Retryable,
            "released",
            "terminal_prepared:retryable_released",
            &retryable_edit,
            1,
            false,
        );
        assert_bridge_transition_projection(
            "bridge-needs-human",
            super::BridgeClaimDisposition::NeedsHuman,
            "failed",
            "terminal_prepared:needs_human",
            "issue edit 42 --repo owner/repo --remove-label in-progress-by-bot --remove-label auto-implement --add-label autospec:needs-human",
            1,
            false,
        );
        assert_bridge_transition_projection(
            "bridge-merged",
            super::BridgeClaimDisposition::Merged,
            "merged",
            "terminal_prepared:merged",
            "issue edit 42 --repo owner/repo --remove-label in-progress-by-bot",
            2,
            false,
        );
        assert_bridge_transition_projection(
            "bridge-retryable-prepared-restart",
            super::BridgeClaimDisposition::Retryable,
            "released",
            "terminal_prepared:retryable_released",
            &retryable_edit,
            1,
            true,
        );
    }

    #[test]
    fn claim_ref_initial_creation_has_exactly_one_receive_pack_winner() {
        // Break caught: two absent-ref acquisitions both succeeding on eventually-consistent comments.
        let fixture = ClaimRefFixture::new("initial-race");
        let barrier = Arc::new(Barrier::new(3));
        let handles = fixture
            .clients
            .clone()
            .into_iter()
            .enumerate()
            .map(|(index, client)| {
                let barrier = Arc::clone(&barrier);
                let remote = fixture.remote.clone();
                std::thread::spawn(move || {
                    let worker = format!("worker-{index}");
                    let record = claim_record(&worker, &format!("claim-{index}"), "claimed");
                    barrier.wait();
                    advance_claim_ref_in(
                        Path::new("git"),
                        &client,
                        remote.to_str().unwrap(),
                        "owner/repo",
                        42,
                        None,
                        &record,
                    )
                })
            })
            .collect::<Vec<_>>();
        barrier.wait();
        let results = handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .expect("claim publisher")
                    .expect("claim result")
            })
            .collect::<Vec<_>>();

        assert_eq!(
            results
                .iter()
                .filter(|result| matches!(result, ClaimRefAdvance::Won(_)))
                .count(),
            1
        );
        assert_eq!(
            results
                .iter()
                .filter(|result| matches!(result, ClaimRefAdvance::Lost))
                .count(),
            1
        );
    }

    #[test]
    fn claim_ref_rejects_a_stale_terminal_transition_after_takeover() {
        // Break caught: stale release publishing an absorbing terminal comment before ownership CAS.
        let fixture = ClaimRefFixture::new("stale-terminal");
        let original = claim_record("worker-a", "claim-a", "claimed");
        let initial = advance_claim_ref_in(
            Path::new("git"),
            &fixture.clients[0],
            fixture.remote.to_str().unwrap(),
            "owner/repo",
            42,
            None,
            &original,
        )
        .expect("initial claim");
        let ClaimRefAdvance::Won(parent) = initial else {
            panic!("initial claim must win");
        };
        let takeover = claim_record("worker-b", "claim-b", "claimed");
        assert!(matches!(
            advance_claim_ref_in(
                Path::new("git"),
                &fixture.clients[1],
                fixture.remote.to_str().unwrap(),
                "owner/repo",
                42,
                Some(&parent),
                &takeover,
            )
            .expect("takeover"),
            ClaimRefAdvance::Won(_)
        ));
        let terminal = claim_record("worker-a", "claim-a", "merged");
        assert_eq!(
            advance_claim_ref_in(
                Path::new("git"),
                &fixture.clients[0],
                fixture.remote.to_str().unwrap(),
                "owner/repo",
                42,
                Some(&parent),
                &terminal,
            )
            .expect("stale terminal"),
            ClaimRefAdvance::Lost
        );

        let head = read_claim_ref_in(
            Path::new("git"),
            &fixture.clients[0],
            fixture.remote.to_str().unwrap(),
            "owner/repo",
            42,
        )
        .expect("read winner")
        .expect("claim ref");
        assert_eq!(head.record.worker_id, "worker-b");
        assert_eq!(head.record.state, "claimed");
    }

    #[cfg(unix)]
    #[test]
    fn claim_ref_keeps_an_unchanged_failed_push_transient() {
        use std::os::unix::fs::PermissionsExt;

        let fixture = ClaimRefFixture::new("unchanged-push-failure");
        let original = claim_record("worker-a", "claim-a", "claimed");
        let ClaimRefAdvance::Won(parent) = advance_claim_ref_in(
            Path::new("git"),
            &fixture.clients[0],
            fixture.remote.to_str().unwrap(),
            "owner/repo",
            42,
            None,
            &original,
        )
        .expect("seed claim") else {
            panic!("seed claim must win");
        };
        let wrapper = fixture.root.join("git-fail-push");
        std::fs::write(
            &wrapper,
            "#!/bin/sh\nfor arg in \"$@\"; do if [ \"$arg\" = push ]; then echo outage >&2; exit 75; fi; done\nexec git \"$@\"\n",
        )
        .expect("git wrapper");
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755))
            .expect("git wrapper mode");
        let refreshed = claim_record("worker-a", "claim-a", "claimed");

        let error = advance_claim_ref_in(
            &wrapper,
            &fixture.clients[0],
            fixture.remote.to_str().unwrap(),
            "owner/repo",
            42,
            Some(&parent),
            &refreshed,
        )
        .expect_err("unchanged failed push must remain retryable");

        assert_eq!(error.kind, crate::commands::CommandFailureKind::Transient);
        assert!(
            error.message.contains("without changing"),
            "{}",
            error.message
        );
    }

    fn seed_claim(fixture: &ClaimRefFixture) -> super::ClaimRefHead {
        let initial = claim_record("worker-a", "claim-a", "claimed");
        match advance_claim_ref_in(
            Path::new("git"),
            &fixture.clients[0],
            fixture.remote.to_str().unwrap(),
            "owner/repo",
            42,
            None,
            &initial,
        )
        .expect("seed claim")
        {
            ClaimRefAdvance::Won(head) => *head,
            ClaimRefAdvance::Lost => panic!("seed claim lost"),
        }
    }

    fn race_claim_ref_transitions(
        fixture: &ClaimRefFixture,
        parent: &super::ClaimRefHead,
        records: [RunStateRecord; 2],
    ) -> Vec<ClaimRefAdvance> {
        let barrier = Arc::new(Barrier::new(3));
        let handles = fixture
            .clients
            .clone()
            .into_iter()
            .zip(records)
            .map(|(client, record)| {
                let barrier = Arc::clone(&barrier);
                let remote = fixture.remote.clone();
                let parent = parent.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    advance_claim_ref_in(
                        Path::new("git"),
                        &client,
                        remote.to_str().unwrap(),
                        "owner/repo",
                        42,
                        Some(&parent),
                        &record,
                    )
                })
            })
            .collect::<Vec<_>>();
        barrier.wait();
        handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .expect("claim publisher")
                    .expect("claim result")
            })
            .collect()
    }

    #[test]
    fn claim_ref_renewal_race_has_one_exact_parent_winner() {
        // Break caught: two renewals both extending the same generation.
        let fixture = ClaimRefFixture::new("renewal-race");
        let parent = seed_claim(&fixture);
        let mut renewal_a = parent.record.clone();
        renewal_a.updated_at = "2026-07-25T00:00:01Z".to_string();
        let mut renewal_b = parent.record.clone();
        renewal_b.updated_at = "2026-07-25T00:00:02Z".to_string();
        let results = race_claim_ref_transitions(&fixture, &parent, [renewal_a, renewal_b]);
        assert_eq!(
            results
                .iter()
                .filter(|result| matches!(result, ClaimRefAdvance::Won(_)))
                .count(),
            1
        );
        assert_eq!(
            results
                .iter()
                .filter(|result| matches!(result, ClaimRefAdvance::Lost))
                .count(),
            1
        );
    }

    #[test]
    fn claim_ref_takeover_and_renewal_each_win_when_received_first() {
        // Break caught: eventual comment ordering allowing both a takeover and renewal.
        for takeover_first in [false, true] {
            let fixture = ClaimRefFixture::new(if takeover_first {
                "takeover-first"
            } else {
                "renewal-first"
            });
            let parent = seed_claim(&fixture);
            let mut renewal = parent.record.clone();
            renewal.updated_at = "2026-07-25T00:00:01Z".to_string();
            let takeover = claim_record("worker-b", "claim-b", "claimed");
            let ordered = if takeover_first {
                [takeover, renewal]
            } else {
                [renewal, takeover]
            };
            let first = advance_claim_ref_in(
                Path::new("git"),
                &fixture.clients[0],
                fixture.remote.to_str().unwrap(),
                "owner/repo",
                42,
                Some(&parent),
                &ordered[0],
            )
            .expect("first transition");
            assert!(matches!(first, ClaimRefAdvance::Won(_)));
            let second = advance_claim_ref_in(
                Path::new("git"),
                &fixture.clients[1],
                fixture.remote.to_str().unwrap(),
                "owner/repo",
                42,
                Some(&parent),
                &ordered[1],
            )
            .expect("second transition");
            assert_eq!(second, ClaimRefAdvance::Lost);
            let head = read_claim_ref_in(
                Path::new("git"),
                &fixture.clients[0],
                fixture.remote.to_str().unwrap(),
                "owner/repo",
                42,
            )
            .expect("read winner")
            .expect("claim ref");
            assert_eq!(head.record.worker_id, ordered[0].worker_id);
            assert_eq!(head.record.claim_id, ordered[0].claim_id);
        }
    }

    #[test]
    fn claim_ref_ambiguous_push_rereads_without_a_second_mutation() {
        // Break caught: retrying a POST/push after receive-pack committed but the response vanished.
        use std::os::unix::fs::PermissionsExt;

        let fixture = ClaimRefFixture::new("ambiguous-push");
        let wrapper = fixture.root.join("ambiguous-git");
        let push_log = fixture.root.join("push.log");
        let script = format!(
            "#!/bin/sh\nif [ \"$1\" = push ]; then\n  /usr/bin/git \"$@\"\n  status=$?\n  printf 'push\\n' >> '{}'\n  [ \"$status\" -eq 0 ] && exit 73\n  exit \"$status\"\nfi\nexec /usr/bin/git \"$@\"\n",
            push_log.display()
        );
        std::fs::write(&wrapper, script).expect("write git wrapper");
        let mut permissions = std::fs::metadata(&wrapper)
            .expect("git wrapper metadata")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&wrapper, permissions).expect("make git wrapper executable");

        let record = claim_record("worker-a", "claim-a", "claimed");
        assert!(matches!(
            advance_claim_ref_in(
                &wrapper,
                &fixture.clients[0],
                fixture.remote.to_str().unwrap(),
                "owner/repo",
                42,
                None,
                &record,
            )
            .expect("ambiguous transition"),
            ClaimRefAdvance::Won(_)
        ));
        assert_eq!(
            std::fs::read_to_string(push_log)
                .expect("push invocation log")
                .lines()
                .count(),
            1
        );
    }

    #[test]
    fn claim_ref_terminal_transition_rejects_foreign_identity() {
        // Break caught: foreign worker, branch, or claim ID publishing a terminal state.
        let fixture = ClaimRefFixture::new("foreign-release");
        let parent = seed_claim(&fixture);
        for (worker, branch, claim_id) in [
            ("worker-b", "feat/worker-a", "claim-a"),
            ("worker-a", "feat/worker-b", "claim-a"),
            ("worker-a", "feat/worker-a", "claim-b"),
        ] {
            let mut terminal = parent.record.clone();
            terminal.state = "merged".to_string();
            terminal.step = "merged".to_string();
            terminal.worker_id = worker.to_string();
            terminal.branch = branch.to_string();
            terminal.claim_id = Some(claim_id.to_string());
            assert!(advance_claim_ref_in(
                Path::new("git"),
                &fixture.clients[0],
                fixture.remote.to_str().unwrap(),
                "owner/repo",
                42,
                Some(&parent),
                &terminal,
            )
            .is_err());
        }
        let head = read_claim_ref_in(
            Path::new("git"),
            &fixture.clients[0],
            fixture.remote.to_str().unwrap(),
            "owner/repo",
            42,
        )
        .expect("read claim")
        .expect("claim ref");
        assert_eq!(head, parent);
    }

    #[test]
    fn claim_ref_https_push_uses_gh_credential_helper_without_a_token_argument() {
        // Break caught: relying on global `gh auth setup-git` or exposing a token in argv.
        let arguments = super::claim_remote_arguments(
            "https://github.enterprise.example/owner/repo.git",
            &["push", "origin", "abc123:refs/autospec/claims/issue-42"],
        );
        assert_eq!(
            arguments,
            [
                "-c",
                "credential.helper=!gh auth git-credential",
                "-c",
                "credential.useHttpPath=true",
                "push",
                "origin",
                "abc123:refs/autospec/claims/issue-42",
            ]
        );
        assert!(!arguments.iter().any(|argument| {
            argument.contains("token")
                || argument.contains("Authorization")
                || argument.starts_with("https://")
        }));
        for remote_command in [
            vec!["ls-remote", "--refs", "origin"],
            vec!["fetch", "--no-tags", "origin"],
        ] {
            let arguments = super::claim_remote_arguments(
                "https://github.enterprise.example/owner/repo.git",
                &remote_command,
            );
            assert_eq!(
                &arguments[..4],
                [
                    "-c",
                    "credential.helper=!gh auth git-credential",
                    "-c",
                    "credential.useHttpPath=true",
                ]
            );
            assert!(!arguments.iter().any(|argument| argument.contains("token")));
        }
        assert_eq!(
            super::claim_remote_arguments("/tmp/remote.git", &["fetch", "/tmp/remote.git"]),
            ["fetch", "/tmp/remote.git"]
        );
        assert_eq!(
            validated_claim_remote("owner/repo", "https://github.enterprise.example/owner/repo")
                .expect("enterprise clone URL"),
            "https://github.enterprise.example/owner/repo.git"
        );
        assert!(validated_claim_remote(
            "owner/repo",
            "https://github.enterprise.example/other/repo"
        )
        .is_err());
    }

    #[test]
    fn validated_claim_remote_rejects_unsafe_https_shapes() {
        for url in [
            "https://github.example/prefix/owner/repo",
            "https://github.example/owner/repo?token=secret",
            "https://github.example/owner/repo#fragment",
            "https://attacker@example.com/owner/repo",
            "https://github.example/not-owner/repo",
            "https://github.example/owner/repo/extra",
        ] {
            assert!(
                validated_claim_remote("owner/repo", url).is_err(),
                "accepted unsafe claim remote {url}"
            );
        }
    }

    #[test]
    fn claim_ref_private_git_dirs_are_distinct_and_leave_target_git_metadata_unchanged() {
        // Break caught: claim fetches mutating FETCH_HEAD/objects in a shared target worktree.
        use std::os::unix::fs::PermissionsExt;

        let fixture = ClaimRefFixture::new("private-git");
        let state_root = fixture.root.join("private-state");
        let first = private_claim_git_dir_in(&state_root, "owner/repo")
            .expect("first private claim Git dir");
        let second = private_claim_git_dir_in(&state_root, "owner/repo")
            .expect("second private claim Git dir");
        assert_ne!(first.path, second.path);
        assert_eq!(
            std::fs::metadata(&first.path)
                .expect("private claim dir")
                .permissions()
                .mode()
                & 0o777,
            0o700
        );

        let target = &fixture.clients[0];
        let status_before = git_stdout(target, &["status", "--porcelain=v1"]);
        let refs_before = git_stdout(
            target,
            &["for-each-ref", "--format=%(refname) %(objectname)"],
        );
        let fetch_head = target.join(".git/FETCH_HEAD");
        let fetch_head_before = std::fs::read(&fetch_head).ok();
        let record = claim_record("worker-a", "claim-a", "claimed");
        assert!(matches!(
            advance_claim_ref_in(
                Path::new("git"),
                &first.path,
                fixture.remote.to_str().unwrap(),
                "owner/repo",
                42,
                None,
                &record,
            )
            .expect("private claim transition"),
            ClaimRefAdvance::Won(_)
        ));
        assert_eq!(
            git_stdout(target, &["status", "--porcelain=v1"]),
            status_before
        );
        assert_eq!(
            git_stdout(
                target,
                &["for-each-ref", "--format=%(refname) %(objectname)"]
            ),
            refs_before
        );
        assert_eq!(std::fs::read(&fetch_head).ok(), fetch_head_before);
    }

    #[test]
    fn claim_mutation_paths_use_the_ref_ledger_or_fail_closed() {
        // Break caught: legacy state subcommands mutating comments/labels around the CAS ledger.
        let source = include_str!("claim.rs");
        for function in [
            "release",
            "acquire_record",
            "refresh_claim_generation",
            "upsert_record",
            "clear",
            "recover_authoritative_stale_startup",
            "record_executor_result",
        ] {
            assert!(
                source_function(source, function).contains("advance_claim_ref"),
                "{function} must advance the claim ref before audit projection"
            );
        }
        assert!(source_function(source, "clear").contains("has_exact_claim_generation"));
        let recovery = source_function(source, "recover_authoritative_stale_startup");
        assert!(recovery.contains("\"available\""));
        assert!(recovery.contains("\"stale_startup_recovered\""));
        for function in [
            "record_executor_result_with_step",
            "reconcile_linked_pr_record",
        ] {
            let body = source_function(source, function);
            assert!(
                body.contains("read_claim_ref") && body.contains("upsert_record"),
                "{function} must route through the exact-ref mutation helper"
            );
            assert!(
                !body.contains("select_run_state"),
                "{function} must not derive a transition from stale audit projection"
            );
        }
        let lease = source_function(source, "has_active_executor_claim");
        assert!(lease.contains("record.updated_at"));
        assert!(lease.contains("record.ttl_seconds"));
    }

    #[test]
    fn prepared_recovery_counts_toward_capacity_until_labels_converge() {
        let record = RunStateRecord::new(
            "owner/repo",
            42,
            "worker-a",
            "available",
            "",
            "",
            "stale_startup_recovered",
            Vec::new(),
            "2999-01-01T00:00:00Z",
            "2999-01-01T00:00:00Z",
            300,
        )
        .with_claim_id("claim-a");

        assert!(
            super::active_record_counts_toward_worker_capacity("owner/repo", 42, &record, 300)
                .expect("classify prepared recovery")
        );
    }
}
