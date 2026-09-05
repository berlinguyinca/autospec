use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::process::{Command, Output};

use autospec_core::claim::{
    replace_safety_review_section, ClaimSafetyInput, SafetyReviewDecision, SafetyReviewVerdict,
};
use autospec_core::coordination::{
    dependency_numbers, parse_dependency_issue_json, parse_remote_issue_page_json,
    parse_remote_pull_request_page_json, plan_ready_queue_with_trusted_actors, PullRequestEvidence,
    QueueIssueView, QueuePolicy, ReadyQueueInput, ReadyQueuePlan, RemoteIssue, RemoteIssuePage,
};

use super::autonomous::gh_read::run_gh_read_with_retry;
use super::claim::{
    active_issue_counts_toward_worker_capacity, lease::requeue_abandoned_active_issue,
    reconcile_authoritative_active_issue, recover_active_issue,
};
use super::lint::{
    confirm_issue_safety_for_queue, load_issue_safety_policy, review_issue_safety_for_queue,
};
use super::CommandFailure;

mod accountability;
use accountability::{is_accountability_issue, reviewable_issue_with_recheck, RecheckScope};
pub fn run(args: &[String]) -> Result<(), CommandFailure> {
    match args {
        [] => Err(CommandFailure::diagnostic(
            "autospec queue requires a subcommand",
        )),
        [flag] if matches!(flag.as_str(), "--help" | "-h") => {
            print_help();
            Ok(())
        }
        [command, rest @ ..] if command == "ready" => ready(rest),
        [command, rest @ ..] if command == "review-safety" => review_safety(rest),
        [command, ..] => Err(CommandFailure::diagnostic(format!(
            "unknown autospec queue command: {command}"
        ))),
    }
}

#[derive(Debug, Default)]
struct ReviewSafetyOptions {
    repo: Option<String>,
    limit: Option<usize>,
    issue: Option<u64>,
    /// Re-derive a verdict for `security:quarantined` issues, and lift the
    /// quarantine when the current rules no longer block them. Off by default:
    /// a quarantine must not evaporate on a routine sweep. See
    /// `reviewable_issue_with_recheck` for why the escape hatch has to exist.
    recheck: bool,
}

#[derive(Debug, Default)]
pub(crate) struct ReviewSafetyTotals {
    pass: usize,
    ambiguous: usize,
    block: usize,
    stale: usize,
    conflicted: usize,
    skipped: usize,
}

enum ReviewSafetyOutcome {
    Pass,
    Ambiguous,
    Block,
    Stale,
    Conflicted,
    Skipped,
}

fn review_safety(args: &[String]) -> Result<(), CommandFailure> {
    let options = parse_review_safety_options(args)?;
    let repo = options.repo.map_or_else(infer_repo, Ok)?;
    let limit = options
        .limit
        .ok_or_else(|| CommandFailure::diagnostic("--limit is required"))?;
    let totals = review_safety_for_repo_with_recheck(&repo, limit, options.issue, options.recheck)?;
    println!("{}", review_safety_json(&totals));
    Ok(())
}

pub(crate) fn review_safety_for_repo(
    repo: &str,
    limit: usize,
    issue: Option<u64>,
) -> Result<ReviewSafetyTotals, CommandFailure> {
    review_safety_for_repo_with_recheck(repo, limit, issue, false)
}

pub(crate) fn review_safety_for_repo_with_recheck(
    repo: &str,
    limit: usize,
    issue: Option<u64>,
    recheck: bool,
) -> Result<ReviewSafetyTotals, CommandFailure> {
    let scope = RecheckScope::new(recheck, issue.is_some());
    let issues = review_safety_issues(repo, issue)?;
    let (mut totals, candidates) = review_safety_candidates(issues, limit, scope)?;
    for candidate in candidates {
        let outcome = review_safety_candidate(repo, &candidate, scope).unwrap_or_else(|error| {
            eprintln!(
                "queue safety conflict for issue {}: {error}",
                candidate.number
            );
            ReviewSafetyOutcome::Conflicted
        });
        match outcome {
            ReviewSafetyOutcome::Pass => totals.pass += 1,
            ReviewSafetyOutcome::Ambiguous => totals.ambiguous += 1,
            ReviewSafetyOutcome::Block => totals.block += 1,
            ReviewSafetyOutcome::Stale => totals.stale += 1,
            ReviewSafetyOutcome::Conflicted => totals.conflicted += 1,
            ReviewSafetyOutcome::Skipped => totals.skipped += 1,
        }
    }
    Ok(totals)
}

fn review_safety_issues(
    repo: &str,
    issue: Option<u64>,
) -> Result<Vec<RemoteIssue>, CommandFailure> {
    match issue {
        Some(number) => Ok(vec![read_issue(repo, number)?]),
        None => list_issues(repo, "auto-implement"),
    }
}

fn review_safety_candidates(
    candidates: Vec<RemoteIssue>,
    limit: usize,
    scope: RecheckScope,
) -> Result<(ReviewSafetyTotals, Vec<RemoteIssue>), CommandFailure> {
    let mut totals = ReviewSafetyTotals::default();
    let mut unreviewed = Vec::new();
    for candidate in candidates {
        // Normally an issue the deterministic screen already allows needs no
        // review. Under recheck a quarantined issue is the exception: the screen
        // allowing it now is precisely the evidence that its quarantine is
        // stale, and skipping it would leave the label no path back off.
        if reviewable_issue_with_recheck(&candidate, scope)
            && (accountability::stale_safety_label(&candidate, scope)
                || !confirm_issue_safety_for_queue(&issue_safety_input(&candidate))?)
        {
            unreviewed.push(candidate);
        } else {
            totals.skipped += 1;
        }
    }
    totals.skipped += unreviewed.len().saturating_sub(limit);
    unreviewed.truncate(limit);
    Ok((totals, unreviewed))
}

fn review_safety_candidate(
    repo: &str,
    candidate: &RemoteIssue,
    scope: RecheckScope,
) -> Result<ReviewSafetyOutcome, CommandFailure> {
    let current = read_issue(repo, candidate.number)?;
    if !reviewable_issue_with_recheck(&current, scope) {
        return Ok(ReviewSafetyOutcome::Stale);
    }
    let stale_label = accountability::stale_safety_label(&current, scope);
    if confirm_issue_safety_for_queue(&issue_safety_input(&current))? && !stale_label {
        return Ok(ReviewSafetyOutcome::Skipped);
    }
    if issue_has_label(&current, "safety:reviewed") {
        return Ok(ReviewSafetyOutcome::Conflicted);
    }
    let verdict = review_issue_safety_for_queue(&issue_safety_input(&current))?;
    match verdict.decision {
        SafetyReviewDecision::Pass if apply_passing_safety_review(repo, &current)? => {
            // A re-derived pass is what lifts the quarantine. It is removed only
            // here, after the typed reviewer has said pass on the CURRENT body
            // under the CURRENT rules — never as a standalone label edit, so the
            // audit trail always carries a verdict that justifies it.
            for label in scope.liftable_labels(&current) {
                remove_issue_label(repo, current.number, label)?;
            }
            Ok(ReviewSafetyOutcome::Pass)
        }
        SafetyReviewDecision::Pass => Ok(ReviewSafetyOutcome::Conflicted),
        SafetyReviewDecision::Ambiguous => {
            apply_non_passing_safety_review(repo, &current, &verdict, "autospec:needs-human")?;
            Ok(ReviewSafetyOutcome::Ambiguous)
        }
        SafetyReviewDecision::Block => {
            apply_non_passing_safety_review(repo, &current, &verdict, "security:quarantined")?;
            Ok(ReviewSafetyOutcome::Block)
        }
    }
}

fn parse_review_safety_options(args: &[String]) -> Result<ReviewSafetyOptions, CommandFailure> {
    let mut options = ReviewSafetyOptions::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--repo" => set_review_repo(&mut options, args, &mut index)?,
            "--limit" => set_review_limit(&mut options, args, &mut index)?,
            "--issue" => set_review_issue(&mut options, args, &mut index)?,
            "--recheck" => options.recheck = true,
            "--help" | "-h" => return review_safety_help_error(),
            option => return unknown_review_safety_option(option),
        }
        index += 1;
    }
    Ok(options)
}

fn set_review_issue(
    options: &mut ReviewSafetyOptions,
    args: &[String],
    index: &mut usize,
) -> Result<(), CommandFailure> {
    let value = next_value(args, index, "--issue")?;
    let issue = value
        .parse::<u64>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| CommandFailure::diagnostic("--issue must be a positive integer"))?;
    if options.issue.is_some() {
        return Err(CommandFailure::diagnostic(
            "--issue accepts exactly one value",
        ));
    }
    options.issue = Some(issue);
    Ok(())
}

fn review_safety_help_error() -> Result<ReviewSafetyOptions, CommandFailure> {
    Err(CommandFailure::diagnostic(
        "--help cannot be combined with queue review-safety options",
    ))
}

fn unknown_review_safety_option(option: &str) -> Result<ReviewSafetyOptions, CommandFailure> {
    Err(CommandFailure::diagnostic(format!(
        "unknown autospec queue review-safety option: {option}"
    )))
}

fn set_review_repo(
    options: &mut ReviewSafetyOptions,
    args: &[String],
    index: &mut usize,
) -> Result<(), CommandFailure> {
    let value = next_value(args, index, "--repo")?;
    if options.repo.is_some() {
        return Err(CommandFailure::diagnostic(
            "--repo accepts exactly one value",
        ));
    }
    options.repo = Some(value);
    Ok(())
}

fn set_review_limit(
    options: &mut ReviewSafetyOptions,
    args: &[String],
    index: &mut usize,
) -> Result<(), CommandFailure> {
    let value = next_value(args, index, "--limit")?;
    let limit = value
        .parse::<usize>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| CommandFailure::diagnostic("--limit must be a positive integer"))?;
    if options.limit.is_some() {
        return Err(CommandFailure::diagnostic(
            "--limit accepts exactly one value",
        ));
    }
    options.limit = Some(limit);
    Ok(())
}

fn issue_has_label(issue: &RemoteIssue, label: &str) -> bool {
    issue.labels.iter().any(|current| current == label)
}

fn issue_safety_input(issue: &RemoteIssue) -> ClaimSafetyInput {
    ClaimSafetyInput::new(
        issue.labels.clone(),
        issue.title.clone(),
        issue.body.clone(),
        issue.author.clone(),
    )
}

fn read_issue(repo: &str, number: u64) -> Result<RemoteIssue, CommandFailure> {
    const ISSUE_FIELDS: &str = "{number, title:(.title // \"\"), body:(.body // \"\"), labels:[.labels[].name], author:{login:(.user.login // \"\")}, state:(.state // \"OPEN\")}";
    let endpoint = format!("repos/{repo}/issues/{number}");
    let output = run_gh_read_with_retry(
        &["api", "--method", "GET", &endpoint, "--jq", ISSUE_FIELDS],
        &format!("gh issue reread {number}"),
    )?;
    parse_dependency_issue_json(&String::from_utf8_lossy(&output.stdout), number).map_err(|error| {
        CommandFailure::diagnostic(format!(
            "could not parse GitHub issue reread {number}: {error}"
        ))
    })
}

pub(crate) fn issue_title_body(
    repo: &str,
    number: u64,
) -> Result<(String, String), CommandFailure> {
    let issue = read_issue(repo, number)?;
    Ok((issue.title, issue.body))
}

fn apply_passing_safety_review(repo: &str, issue: &RemoteIssue) -> Result<bool, CommandFailure> {
    let body = replace_safety_review_section(&issue.body, SafetyReviewDecision::Pass).map_err(
        |error| {
            CommandFailure::diagnostic(format!(
                "could not replace safety review for issue {}: {error:?}",
                issue.number
            ))
        },
    )?;
    update_issue_body(repo, issue.number, &body)?;
    add_issue_label(repo, issue.number, "safety:reviewed")?;
    let reread = read_issue(repo, issue.number)?;
    reviewable_pass(&reread)
}

fn reviewable_pass(issue: &RemoteIssue) -> Result<bool, CommandFailure> {
    Ok(!issue.closed
        && issue_has_label(issue, "auto-implement")
        && !is_accountability_issue(issue)
        && confirm_issue_safety_for_queue(&issue_safety_input(issue))?)
}

fn update_issue_body(repo: &str, number: u64, body: &str) -> Result<(), CommandFailure> {
    let endpoint = format!("repos/{repo}/issues/{number}");
    let body_field = format!("body={body}");
    let output = run_gh(&["api", "--method", "PATCH", &endpoint, "-f", &body_field])?;
    if output.status.success() {
        Ok(())
    } else {
        Err(CommandFailure::diagnostic(format!(
            "gh issue body update {number} failed: {}",
            command_error(&output)
        )))
    }
}

fn apply_non_passing_safety_review(
    repo: &str,
    issue: &RemoteIssue,
    verdict: &SafetyReviewVerdict,
    label: &str,
) -> Result<(), CommandFailure> {
    let decision = safety_decision_name(&verdict.decision);
    if !has_safety_decision_comment(repo, issue.number, decision)? {
        post_issue_comment(
            repo,
            issue.number,
            &safety_decision_comment(issue.number, verdict),
        )?;
    }
    add_issue_label(repo, issue.number, label)
}

fn has_safety_decision_comment(
    repo: &str,
    number: u64,
    decision: &str,
) -> Result<bool, CommandFailure> {
    const PAGE_SIZE: usize = 100;
    const BEGIN_MARKER: &str = "<!-- autospec-safety-decision:begin -->";
    let mut page = 1usize;
    loop {
        let endpoint =
            format!("repos/{repo}/issues/{number}/comments?per_page={PAGE_SIZE}&page={page}");
        let fields = format!(
            "{{raw_count:length,items:[.[] | select((.body // \"\") | contains({})) | {{number:0,title:\"\",body:(.body // \"\"),labels:[],author:{{login:\"\"}},state:\"OPEN\"}}]}}",
            json_string(BEGIN_MARKER),
        );
        let output = run_gh_read_with_retry(
            &["api", "--method", "GET", &endpoint, "--jq", &fields],
            &format!("gh safety decision comment page {page} for issue {number}"),
        )?;
        let comment_page = parse_safety_decision_comment_page(&output.stdout, page, number)?;
        if comment_page
            .issues
            .iter()
            .any(|comment| comment.body.contains(decision))
        {
            return Ok(true);
        }
        if comment_page.raw_count < PAGE_SIZE {
            return Ok(false);
        }
        page = page.checked_add(1).ok_or_else(|| {
            CommandFailure::diagnostic(format!(
                "GitHub safety decision comment page number overflowed for issue {number}"
            ))
        })?;
    }
}

fn parse_safety_decision_comment_page(
    stdout: &[u8],
    page: usize,
    number: u64,
) -> Result<autospec_core::coordination::RemoteIssuePage, CommandFailure> {
    parse_remote_issue_page_json(&String::from_utf8_lossy(stdout)).map_err(|error| {
        CommandFailure::diagnostic(format!(
            "could not parse GitHub safety decision comment page {page} for issue {number}: {error}"
        ))
    })
}

fn post_issue_comment(repo: &str, number: u64, body: &str) -> Result<(), CommandFailure> {
    let endpoint = format!("repos/{repo}/issues/{number}/comments");
    let body_field = format!("body={body}");
    let output = run_gh(&["api", "--method", "POST", &endpoint, "-f", &body_field])?;
    if output.status.success() {
        Ok(())
    } else {
        Err(CommandFailure::diagnostic(format!(
            "gh safety decision comment write for issue {number} failed: {}",
            command_error(&output)
        )))
    }
}

fn add_issue_label(repo: &str, number: u64, label: &str) -> Result<(), CommandFailure> {
    let endpoint = format!("repos/{repo}/issues/{number}/labels");
    let label_field = format!("labels[]={label}");
    let output = run_gh(&["api", "--method", "POST", &endpoint, "-f", &label_field])?;
    if output.status.success() {
        Ok(())
    } else {
        Err(CommandFailure::diagnostic(format!(
            "gh safety label {label} write for issue {number} failed: {}",
            command_error(&output)
        )))
    }
}

/// Removes one label, tolerating its absence.
///
/// Only reached on a re-derived pass under `--recheck`, so the removal always
/// has a verdict behind it. A 404 means the label is already gone, which is the
/// state we wanted — treating that as failure would make a retried recheck fail
/// on a queue it had already repaired.
fn remove_issue_label(repo: &str, number: u64, label: &str) -> Result<(), CommandFailure> {
    let endpoint = format!("repos/{repo}/issues/{number}/labels/{label}");
    let output = run_gh(&["api", "--method", "DELETE", &endpoint])?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains("404") || stderr.contains("Label does not exist") {
        return Ok(());
    }
    Err(CommandFailure::diagnostic(format!(
        "gh safety label {label} removal for issue {number} failed: {}",
        command_error(&output)
    )))
}

fn safety_decision_name(decision: &SafetyReviewDecision) -> &'static str {
    match decision {
        SafetyReviewDecision::Pass => "SAFETY_PASS",
        SafetyReviewDecision::Ambiguous => "SAFETY_AMBIGUOUS",
        SafetyReviewDecision::Block => "SAFETY_BLOCK",
    }
}

fn safety_decision_comment(number: u64, verdict: &SafetyReviewVerdict) -> String {
    let findings = verdict
        .findings
        .iter()
        .map(|finding| finding.rule_id)
        .collect::<Vec<_>>();
    let findings = if findings.is_empty() {
        "none".to_string()
    } else {
        findings.join(", ")
    };
    format!(
        "<!-- autospec-safety-decision:begin -->\n- **issue:** `{number}`\n- **decision:** `{}`\n- **findings:** {findings}\n<!-- autospec-safety-decision:end -->",
        safety_decision_name(&verdict.decision),
    )
}

fn review_safety_json(totals: &ReviewSafetyTotals) -> String {
    format!(
        "{{\"pass\":{},\"ambiguous\":{},\"block\":{},\"stale\":{},\"conflicted\":{},\"skipped\":{}}}",
        totals.pass,
        totals.ambiguous,
        totals.block,
        totals.stale,
        totals.conflicted,
        totals.skipped,
    )
}

#[derive(Debug, Default)]
struct ReadyOptions {
    repo: Option<String>,
    batch_size: Option<usize>,
}

fn ready(args: &[String]) -> Result<(), CommandFailure> {
    let options = parse_ready_options(args)?;
    let repo = options.repo.map_or_else(infer_repo, Ok)?;
    let batch_size = options.batch_size.unwrap_or_else(default_batch_size);
    let plan = ready_plan_for(&repo, batch_size)?;
    let constrained = !only_issues().is_empty();
    println!("{}", plan_json(&plan, constrained));
    Ok(())
}

pub(crate) fn ready_plan_for(
    repo: &str,
    batch_size: usize,
) -> Result<ReadyQueuePlan, CommandFailure> {
    let mut active = list_issues(repo, "in-progress-by-bot")?;
    for issue in &active {
        let _ = reconcile_authoritative_active_issue(repo, issue.number);
        let _ = recover_active_issue(repo, issue.number, 300);
        let _ = requeue_abandoned_active_issue(repo, issue.number);
    }
    let candidates = list_issues(repo, "auto-implement")?
        .into_iter()
        .filter(|issue| !is_accountability_issue(issue))
        .collect::<Vec<_>>();
    active = list_issues(repo, "in-progress-by-bot")?
        .into_iter()
        .filter(|issue| {
            active_issue_counts_toward_worker_capacity(repo, issue.number, 300).unwrap_or(true)
        })
        .collect();
    let dependencies = load_dependencies(repo, &candidates);
    let pull_requests = list_pull_requests(repo);
    let only_issues = only_issues();
    let mut policy = QueuePolicy::new(batch_size, max_repo_workers());
    policy.only_issues = only_issues.clone();
    policy.non_blocking_dependency_labels = non_blocking_dependency_labels();
    let safety_policy = load_issue_safety_policy(None)?;
    if safety_policy.has_unsupported_pattern {
        return Err(CommandFailure::diagnostic(
            "queue safety policy contains unsupported custom regex",
        ));
    }
    let trusted_actors = safety_policy
        .trusted_actors
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    Ok(plan_ready_queue_with_trusted_actors(
        &ReadyQueueInput {
            candidates,
            active,
            dependencies,
            pull_requests,
            policy,
        },
        &trusted_actors,
    ))
}

type ReadyOptionSetter = fn(&mut ReadyOptions, &[String], &mut usize) -> Result<(), CommandFailure>;

const READY_OPTION_SETTERS: &[(&str, ReadyOptionSetter)] = &[
    ("--repo", set_ready_repo),
    ("--batch-size", set_ready_batch_size),
];

fn parse_ready_options(args: &[String]) -> Result<ReadyOptions, CommandFailure> {
    let mut options = ReadyOptions::default();
    let mut index = 0;
    while index < args.len() {
        let option = args[index].as_str();
        match READY_OPTION_SETTERS
            .iter()
            .find(|(name, _)| *name == option)
        {
            Some((_, setter)) => setter(&mut options, args, &mut index)?,
            None if matches!(option, "--help" | "-h") => {
                return Err(CommandFailure::diagnostic(
                    "--help cannot be combined with queue ready options",
                ));
            }
            None => {
                return Err(CommandFailure::diagnostic(format!(
                    "unknown autospec queue ready option: {option}"
                )));
            }
        }
        index += 1;
    }
    Ok(options)
}

fn set_ready_repo(
    options: &mut ReadyOptions,
    args: &[String],
    index: &mut usize,
) -> Result<(), CommandFailure> {
    let value = next_value(args, index, "--repo")?;
    if options.repo.replace(value).is_some() {
        return Err(CommandFailure::diagnostic(
            "--repo accepts exactly one value",
        ));
    }
    Ok(())
}

fn set_ready_batch_size(
    options: &mut ReadyOptions,
    args: &[String],
    index: &mut usize,
) -> Result<(), CommandFailure> {
    let value = next_value(args, index, "--batch-size")?;
    let batch_size = value
        .parse::<usize>()
        .map_err(|_| CommandFailure::diagnostic("--batch-size must be an integer"))?;
    if options.batch_size.replace(batch_size.max(1)).is_some() {
        return Err(CommandFailure::diagnostic(
            "--batch-size accepts exactly one value",
        ));
    }
    Ok(())
}

fn next_value(args: &[String], index: &mut usize, option: &str) -> Result<String, CommandFailure> {
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

fn infer_repo() -> Result<String, CommandFailure> {
    let Ok(output) = run_gh_read_with_retry(
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

fn list_issues(repo: &str, label: &str) -> Result<Vec<RemoteIssue>, CommandFailure> {
    const PAGE_SIZE: usize = 100;
    const PAGE_RETRIES: usize = 3;
    const ISSUE_FIELDS: &str = "{raw_count:length, items:[.[] | select(.pull_request == null) | {number, title:(.title // \"\"), body:(.body // \"\"), labels:[.labels[].name], author:{login:(.user.login // \"\")}}]}";

    let mut issues = Vec::new();
    let mut page = 1usize;
    loop {
        let endpoint = format!(
            "repos/{repo}/issues?state=open&labels={label}&per_page={PAGE_SIZE}&page={page}"
        );
        let issue_page = fetch_issue_page_with_retries(PAGE_RETRIES, || {
            let output = run_gh_read_with_retry(
                &["api", "--method", "GET", &endpoint, "--jq", ISSUE_FIELDS],
                "gh issue page read",
            )
            .map_err(|error| error.message)?;
            let body = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if body.is_empty() {
                return Err("empty GitHub issue page response".to_string());
            }
            parse_remote_issue_page_json(&body).map_err(|error| error.to_string())
        })
        .map_err(|error| {
            CommandFailure::diagnostic(format!(
                "could not fetch GitHub {label} issue page {page} after {PAGE_RETRIES} attempts: {error}"
            ))
        })?;
        let is_last_page = issue_page.raw_count < PAGE_SIZE;
        issues.extend(issue_page.issues);
        if is_last_page {
            return Ok(issues);
        }
        page = page.checked_add(1).ok_or_else(|| {
            CommandFailure::diagnostic(format!("GitHub {label} issue page number overflowed"))
        })?;
    }
}

fn fetch_issue_page_with_retries<F>(
    attempts: usize,
    mut fetch: F,
) -> Result<RemoteIssuePage, String>
where
    F: FnMut() -> Result<RemoteIssuePage, String>,
{
    if attempts == 0 {
        return Err("issue page retry budget must be positive".to_string());
    }
    let mut last_error = String::new();
    for _ in 0..attempts {
        match fetch() {
            Ok(page) => return Ok(page),
            Err(error) => last_error = error,
        }
    }
    Err(last_error)
}

fn load_dependencies(repo: &str, candidates: &[RemoteIssue]) -> BTreeMap<u64, RemoteIssue> {
    let candidate_numbers = candidates
        .iter()
        .map(|issue| issue.number)
        .collect::<BTreeSet<_>>();
    let dependency_numbers = candidates
        .iter()
        .flat_map(|issue| dependency_numbers(&issue.body))
        .filter(|number| !candidate_numbers.contains(number))
        .collect::<BTreeSet<_>>();
    dependency_numbers
        .into_iter()
        .map(|number| {
            let issue = load_dependency(repo, number).unwrap_or_else(|| {
                RemoteIssue::open(number, format!("issue-{number}"), "", Vec::new(), "")
            });
            (number, issue)
        })
        .collect()
}

fn load_dependency(repo: &str, number: u64) -> Option<RemoteIssue> {
    let issue = number.to_string();
    let output = run_gh_read_with_retry(
        &[
            "issue",
            "view",
            issue.as_str(),
            "--repo",
            repo,
            "--json",
            "state,body,labels",
            "--jq",
            "{state:(.state // \"OPEN\"), body:(.body // \"\"), labels:[.labels[].name]}",
        ],
        "read a dependency issue",
    )
    .ok()?;
    parse_dependency_issue_json(&String::from_utf8_lossy(&output.stdout), number).ok()
}

fn list_pull_requests(repo: &str) -> PullRequestEvidence {
    const PULL_REQUEST_QUERY: &str = "query($owner:String!,$name:String!,$endCursor:String){repository(owner:$owner,name:$name){pullRequests(first:100,after:$endCursor,states:OPEN){nodes{number state body statusCheckRollup{contexts(first:100){totalCount nodes{__typename ... on CheckRun{name status conclusion} ... on StatusContext{context state}}}}} pageInfo{hasNextPage endCursor}}}}";
    const PULL_REQUEST_FIELDS: &str = "{items:[.data.repository.pullRequests.nodes[] | {number, state:(.state // \"OPEN\"), body:(.body // \"\"), statusCheckRollup:([(.statusCheckRollup.contexts.nodes // [])[] | if .__typename == \"CheckRun\" then {name:(.name // \"\"), status:(.status // \"\"), conclusion} else {name:(.context // \"\"), status:(if (.state == \"PENDING\" or .state == \"EXPECTED\") then \"IN_PROGRESS\" else \"COMPLETED\" end), conclusion:(if (.state == \"PENDING\" or .state == \"EXPECTED\") then null else .state end)} end] + if ((.statusCheckRollup.contexts.totalCount // 0) > ((.statusCheckRollup.contexts.nodes // []) | length)) then [{name:\"incomplete check evidence\",status:\"IN_PROGRESS\",conclusion:null}] else [] end)}], page_info:{has_next_page:.data.repository.pullRequests.pageInfo.hasNextPage, end_cursor:.data.repository.pullRequests.pageInfo.endCursor}}";

    let Some((owner, name)) = repo.split_once('/') else {
        return PullRequestEvidence::Unavailable(format!(
            "GitHub repository must use OWNER/REPO form: {repo}"
        ));
    };
    if owner.is_empty() || name.is_empty() || name.contains('/') {
        return PullRequestEvidence::Unavailable(format!(
            "GitHub repository must use OWNER/REPO form: {repo}"
        ));
    }

    let mut pull_requests = Vec::new();
    let mut cursor = None;
    loop {
        let mut arguments = vec![
            "api".to_string(),
            "graphql".to_string(),
            "-f".to_string(),
            format!("query={PULL_REQUEST_QUERY}"),
            "-f".to_string(),
            format!("owner={owner}"),
            "-f".to_string(),
            format!("name={name}"),
            "--jq".to_string(),
            PULL_REQUEST_FIELDS.to_string(),
        ];
        if let Some(cursor) = &cursor {
            arguments.push("-f".to_string());
            arguments.push(format!("endCursor={cursor}"));
        }
        let argument_refs = arguments.iter().map(String::as_str).collect::<Vec<_>>();
        let output = match run_gh_read_with_retry(&argument_refs, "read pull request page") {
            Ok(output) => output,
            Err(error) => return PullRequestEvidence::Unavailable(error.message),
        };
        let pull_request_page =
            match parse_remote_pull_request_page_json(&String::from_utf8_lossy(&output.stdout)) {
                Ok(page) => page,
                Err(error) => {
                    return PullRequestEvidence::Unavailable(format!(
                        "could not parse GitHub pull request page: {error}"
                    ));
                }
            };
        pull_requests.extend(pull_request_page.pull_requests);
        if !pull_request_page.has_next_page {
            return PullRequestEvidence::Available(pull_requests);
        }
        cursor = pull_request_page.end_cursor;
    }
}

fn run_gh(arguments: &[&str]) -> Result<Output, CommandFailure> {
    Command::new("gh")
        .args(arguments)
        .output()
        .map_err(|error| CommandFailure::diagnostic(format!("could not run gh: {error}")))
}

fn command_error(output: &Output) -> String {
    let error = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if error.is_empty() {
        format!("gh exited with {}", output.status)
    } else {
        error
    }
}

fn only_issues() -> BTreeSet<u64> {
    std::env::var("AUTOSPEC_RUN_ONLY_ISSUES")
        .unwrap_or_default()
        .split_whitespace()
        .filter_map(|value| value.parse::<u64>().ok())
        .collect()
}

fn non_blocking_dependency_labels() -> BTreeSet<String> {
    std::env::var("AUTOSPEC_NON_BLOCKING_DEP_LABELS")
        .unwrap_or_else(|_| "epic umbrella".to_string())
        .split(|character: char| character.is_ascii_whitespace() || character == ',')
        .filter(|label| !label.is_empty())
        .map(|label| label.to_ascii_lowercase())
        .collect()
}

fn default_batch_size() -> usize {
    std::env::var("AUTOSPEC_BATCH_SIZE")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(1)
        .max(1)
}

fn max_repo_workers() -> usize {
    let configured = config_scalar("autonomous.concurrency.max_concurrent_repo_workers")
        .or_else(|| std::env::var("AUTOSPEC_MAX_CONCURRENT_REPO_WORKERS").ok());
    match configured.as_deref().map(str::trim) {
        Some("auto") | None => discovered_workers(),
        Some(value) => value.parse::<usize>().unwrap_or(0),
    }
}

fn config_scalar(path: &str) -> Option<String> {
    let config = std::env::var("AUTOSPEC_CONFIG_FILE")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| ".autospec/autospec.yml".to_string());
    let text = fs::read_to_string(config).ok()?;
    let mut indentation = Vec::new();
    for raw in text.lines() {
        if raw.trim().is_empty() || raw.trim_start().starts_with('#') || !raw.contains(':') {
            continue;
        }
        let indent = raw.len() - raw.trim_start().len();
        let (key, value) = raw.trim().split_once(':')?;
        while indentation
            .last()
            .is_some_and(|(level, _)| *level >= indent)
        {
            indentation.pop();
        }
        if value.trim().is_empty() {
            indentation.push((indent, key.trim().to_string()));
            continue;
        }
        let mut parts = indentation
            .iter()
            .map(|(_, key)| key.as_str())
            .collect::<Vec<_>>();
        parts.push(key.trim());
        if parts.join(".") == path {
            return Some(value.trim().trim_matches(['\'', '\"']).to_string());
        }
    }
    None
}

fn discovered_workers() -> usize {
    for (command, arguments) in [
        ("getconf", &["_NPROCESSORS_ONLN"][..]),
        ("sysctl", &["-n", "hw.ncpu"][..]),
        ("nproc", &[][..]),
    ] {
        let Ok(output) = Command::new(command).args(arguments).output() else {
            continue;
        };
        if !output.status.success() {
            continue;
        }
        if let Ok(count) = String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse::<usize>()
        {
            return (count / 4).clamp(1, 4);
        }
    }
    1
}

fn plan_json(plan: &ReadyQueuePlan, constrained: bool) -> String {
    let diagnostics = local_diagnostics_json(plan);
    let diagnostics_field = if diagnostics == "[]" {
        String::new()
    } else {
        format!(",\"diagnostics\":{diagnostics}")
    };
    format!(
        "{{\"ready\":{},\"blocked\":{},\"claimed\":{},\"conflicts\":{},\"gate_counts\":{}{},\"scan_scope\":{},\"worker_cap\":{{\"max_repo_workers\":{},\"active_count\":{},\"remaining\":{},\"reached\":{}}},\"batch\":{}}}",
        views_json(&plan.ready),
        views_json(&plan.blocked),
        issues_json(&plan.claimed),
        views_json(&plan.conflicts),
        gate_counts_json(plan),
        diagnostics_field,
        json_string(if constrained { "slice" } else { "repository" }),
        plan.worker_cap.max_repo_workers,
        plan.worker_cap.active_count,
        plan.worker_cap.remaining,
        json_bool(plan.worker_cap.reached),
        views_json(&plan.batch),
    )
}

fn local_diagnostics_json(plan: &ReadyQueuePlan) -> String {
    let mut seen = BTreeSet::new();
    let diagnostics = plan
        .blocked
        .iter()
        .filter(|view| {
            view.reason.as_deref() == Some("safety_gate_failed")
                && view
                    .safety_gate
                    .as_ref()
                    .is_some_and(|gate| gate.reason == "missing_safety_reviewed")
                && discovery_filed_issue(&view.issue)
        })
        .filter(|view| seen.insert(view.issue.number))
        .map(discovery_missing_safety_diagnostic_json)
        .collect::<Vec<_>>();
    format!("[{}]", diagnostics.join(","))
}

fn discovery_filed_issue(issue: &RemoteIssue) -> bool {
    issue.labels.iter().any(|label| label == "explore")
        || issue.body.contains("explore research-cycle finalize gate")
        || issue
            .body
            .contains("Auto-filed by /autospec-explore round ")
        || issue.body.contains("<!-- explore-ledger source=")
}

fn discovery_missing_safety_diagnostic_json(view: &QueueIssueView) -> String {
    format!(
        "{{\"kind\":\"discovery_missing_safety_review\",\"issue\":{},\"reason\":\"missing_safety_reviewed\",\"message\":{}}}",
        view.issue.number,
        json_string(
            "discovery-filed auto-implement issue is blocked locally until canonical safety review metadata is stamped"
        )
    )
}

fn gate_counts_json(plan: &ReadyQueuePlan) -> String {
    let counts = &plan.gate_counts;
    format!(
        "{{\"open\":{},\"candidate\":{},\"reviewed\":{},\"blocked\":{},\"dependency_blocked\":{},\"linked_pr_blocked\":{},\"path_conflicted\":{},\"ready\":{},\"claimed\":{},\"selected\":{}}}",
        counts.open,
        counts.candidate,
        counts.reviewed,
        counts.blocked,
        counts.dependency_blocked,
        counts.linked_pr_blocked,
        counts.path_conflicted,
        counts.ready,
        counts.claimed,
        counts.selected,
    )
}

fn views_json(views: &[QueueIssueView]) -> String {
    format!(
        "[{}]",
        views.iter().map(view_json).collect::<Vec<_>>().join(",")
    )
}

fn issues_json(issues: &[RemoteIssue]) -> String {
    format!(
        "[{}]",
        issues.iter().map(issue_json).collect::<Vec<_>>().join(",")
    )
}

type ViewFieldDispatcher = fn(&QueueIssueView, &mut Vec<String>);

const VIEW_FIELD_DISPATCHERS: &[ViewFieldDispatcher] = &[
    append_reason_field,
    append_blocked_label_field,
    append_safety_gate_field,
    append_linked_pr_field,
    append_unmet_dependency_fields,
    append_cycle_dependency_field,
    append_conflicts_with_field,
    append_path_field,
    append_parallel_safety_fields,
];

fn view_json(view: &QueueIssueView) -> String {
    let mut fields = issue_fields(&view.issue);
    for append in view_field_dispatchers() {
        append(view, &mut fields);
    }
    format!("{{{}}}", fields.join(","))
}

fn view_field_dispatchers() -> &'static [ViewFieldDispatcher] {
    VIEW_FIELD_DISPATCHERS
}

fn append_reason_field(view: &QueueIssueView, fields: &mut Vec<String>) {
    if let Some(reason) = &view.reason {
        fields.push(json_field("reason", json_string(reason)));
    }
}

fn append_blocked_label_field(view: &QueueIssueView, fields: &mut Vec<String>) {
    if let Some(blocked_label) = &view.blocked_label {
        fields.push(json_field("blocked_label", json_string(blocked_label)));
    }
}

fn append_safety_gate_field(view: &QueueIssueView, fields: &mut Vec<String>) {
    if let Some(safety_gate) = &view.safety_gate {
        fields.push(json_field(
            "safety_gate",
            format!(
                "{{\"ok\":{},\"reason\":{}}}",
                json_bool(safety_gate.ok),
                json_string(&safety_gate.reason)
            ),
        ));
    }
}

fn append_linked_pr_field(view: &QueueIssueView, fields: &mut Vec<String>) {
    if let Some(linked_pr) = view.linked_pr {
        fields.push(json_field("linked_pr", linked_pr.to_string()));
    }
}

fn append_unmet_dependency_fields(view: &QueueIssueView, fields: &mut Vec<String>) {
    if !view.unmet_dependencies.is_empty() {
        fields.push(json_field(
            "unmet_dependencies",
            numbers_json(&view.unmet_dependencies),
        ));
        fields.push(json_field(
            "non_blocking_refs",
            references_json(&view.non_blocking_refs),
        ));
    }
}

fn append_cycle_dependency_field(view: &QueueIssueView, fields: &mut Vec<String>) {
    if !view.cycle_dependencies.is_empty() {
        fields.push(json_field(
            "cycle_dependencies",
            numbers_json(&view.cycle_dependencies),
        ));
    }
}

fn append_conflicts_with_field(view: &QueueIssueView, fields: &mut Vec<String>) {
    if let Some(conflicts_with) = view.conflicts_with {
        fields.push(json_field("conflicts_with", conflicts_with.to_string()));
    }
}

fn append_path_field(view: &QueueIssueView, fields: &mut Vec<String>) {
    if let Some(path) = &view.path {
        fields.push(json_field("path", json_string(path)));
    }
}

fn append_parallel_safety_fields(view: &QueueIssueView, fields: &mut Vec<String>) {
    if view.parallel_safe.is_some() {
        fields.push(json_field("paths", strings_json(&view.paths)));
        fields.push(json_field(
            "non_blocking_refs",
            references_json(&view.non_blocking_refs),
        ));
        fields.push(json_field(
            "serialization_reasons",
            strings_json(&view.serialization_reasons),
        ));
        fields.push(json_field(
            "parallel_safe",
            json_bool(view.parallel_safe == Some(true)).to_string(),
        ));
    }
}

fn issue_json(issue: &RemoteIssue) -> String {
    format!("{{{}}}", issue_fields(issue).join(","))
}

fn issue_fields(issue: &RemoteIssue) -> Vec<String> {
    vec![
        json_field("number", issue.number.to_string()),
        json_field("title", json_string(&issue.title)),
        json_field("body", json_string(&issue.body)),
        json_field(
            "labels",
            format!(
                "[{}]",
                issue
                    .labels
                    .iter()
                    .map(|label| format!("{{\"name\":{}}}", json_string(label)))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
        ),
        json_field(
            "author",
            if issue.author.is_empty() {
                "null".to_string()
            } else {
                format!("{{\"login\":{}}}", json_string(&issue.author))
            },
        ),
    ]
}

fn references_json(references: &[autospec_core::coordination::NonBlockingReference]) -> String {
    format!(
        "[{}]",
        references
            .iter()
            .map(|reference| {
                format!(
                    "{{\"issue\":{},\"reason\":{},\"cycle\":{}}}",
                    reference.issue,
                    json_string(&reference.reason),
                    json_bool(reference.cycle)
                )
            })
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn numbers_json(numbers: &[u64]) -> String {
    format!(
        "[{}]",
        numbers
            .iter()
            .map(u64::to_string)
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn strings_json(values: &[String]) -> String {
    format!(
        "[{}]",
        values
            .iter()
            .map(|value| json_string(value))
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn json_field(key: &str, value: String) -> String {
    format!("\"{key}\":{value}")
}

fn json_bool(value: bool) -> &'static str {
    if value {
        "true"
    } else {
        "false"
    }
}

fn json_string(value: &str) -> String {
    let mut escaped = String::new();
    for character in value.chars() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            '\"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            character if character.is_control() => {
                escaped.push_str(&format!("\\u{:04x}", character as u32));
            }
            character => escaped.push(character),
        }
    }
    format!("\"{escaped}\"")
}

fn print_help() {
    println!(
        "autospec queue\n\nUSAGE:\n    autospec queue ready [--repo OWNER/REPO] [--batch-size N]\n    autospec queue review-safety [--repo OWNER/REPO] [--limit N] [--issue N]\n\nCOMMANDS:\n    ready            Compute the safe, dependency-aware GitHub issue batch\n    review-safety    Write bounded Rust safety-review outcomes to GitHub issues"
    );
}

#[cfg(test)]
mod tests {
    include!("queue/accountability_tests.rs");
    use super::accountability::reviewable_issue;
    use super::{fetch_issue_page_with_retries, reviewable_issue_with_recheck, RecheckScope};
    use autospec_core::coordination::parse_remote_issue_page_json;

    #[test]
    fn issue_page_retry_recovers_from_transient_empty_and_invalid_pages() {
        let mut calls = 0;
        let page = fetch_issue_page_with_retries(3, || {
            calls += 1;
            match calls {
                1 => Err("empty response".to_string()),
                2 => Err("invalid json".to_string()),
                _ => parse_remote_issue_page_json(r#"{"raw_count":0,"items":[]}"#),
            }
        })
        .expect("third attempt succeeds");
        assert_eq!(calls, 3);
        assert_eq!(page.raw_count, 0);
    }

    #[test]
    fn issue_page_retry_fails_after_budget() {
        let mut calls = 0;
        let error = fetch_issue_page_with_retries(3, || {
            calls += 1;
            Err::<autospec_core::coordination::RemoteIssuePage, _>("still invalid".to_string())
        })
        .expect_err("retry budget is bounded");
        assert_eq!(calls, 3);
        assert_eq!(error, "still invalid");
    }
}
