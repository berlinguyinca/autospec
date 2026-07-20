use std::process::{Command, Output};

use autospec_core::coordination::{parse_dependency_issue_json, RemoteIssue};

use super::super::CommandFailure;

const ISSUE_FIELDS: &str = "{number, title:(.title // \"\"), body:(.body // \"\"), labels:[.labels[].name], author:{login:(.user.login // \"\")}, state:(.state // \"OPEN\")}";

pub(super) fn read_issue(repo: &str, number: u64) -> Result<RemoteIssue, CommandFailure> {
    let endpoint = format!("repos/{repo}/issues/{number}");
    let output = run_gh(&["api", "--method", "GET", &endpoint, "--jq", ISSUE_FIELDS])?;
    require_gh_success(&output, "gh issue read")?;
    parse_dependency_issue_json(&String::from_utf8_lossy(&output.stdout), number).map_err(|error| {
        CommandFailure::diagnostic(format!("could not parse GitHub issue {number}: {error}"))
    })
}

pub(super) fn update_issue_body(repo: &str, number: u64, body: &str) -> Result<(), CommandFailure> {
    let endpoint = format!("repos/{repo}/issues/{number}");
    let body_field = format!("body={body}");
    let output = run_gh(&["api", "--method", "PATCH", &endpoint, "-f", &body_field])?;
    require_gh_success(&output, "gh issue safety body write")
}

pub(super) fn add_issue_label(repo: &str, number: u64, label: &str) -> Result<(), CommandFailure> {
    let endpoint = format!("repos/{repo}/issues/{number}/labels");
    let label_field = format!("labels[]={label}");
    let output = run_gh(&["api", "--method", "POST", &endpoint, "-f", &label_field])?;
    require_gh_success(&output, &format!("gh issue label {label} write"))
}

pub(super) fn remove_issue_label(
    repo: &str,
    number: u64,
    label: &str,
) -> Result<(), CommandFailure> {
    let endpoint = format!("repos/{repo}/issues/{number}/labels/{label}");
    let output = run_gh(&["api", "--method", "DELETE", &endpoint])?;
    require_gh_success(&output, &format!("gh issue label {label} removal"))
}

pub(super) fn remove_owned_labels(
    repo: &str,
    number: u64,
    initial: &RemoteIssue,
    remove_labels: &[String],
) -> Result<bool, CommandFailure> {
    let mut changed = false;
    for label in remove_labels {
        if has_label(initial, label) {
            if let Err(error) = remove_issue_label(repo, number, label) {
                restore_removed_labels(repo, initial, remove_labels);
                return Err(error);
            }
            changed = true;
        }
    }
    Ok(changed)
}

pub(super) fn restore_removed_labels(repo: &str, initial: &RemoteIssue, removed_labels: &[String]) {
    for label in removed_labels {
        if has_label(initial, label) {
            let _ = add_issue_label(repo, initial.number, label);
        }
    }
}

pub(super) fn rollback_owned_labels(
    repo: &str,
    number: u64,
    initial: &RemoteIssue,
    removed_labels: &[String],
) {
    let _ = remove_issue_label(repo, number, "auto-implement");
    if !has_label(initial, "safety:reviewed") {
        let _ = remove_issue_label(repo, number, "safety:reviewed");
    }
    restore_removed_labels(repo, initial, removed_labels);
}

fn run_gh(args: &[&str]) -> Result<Output, CommandFailure> {
    Command::new("gh")
        .args(args)
        .output()
        .map_err(|error| CommandFailure::diagnostic(format!("could not execute gh: {error}")))
}

fn require_gh_success(output: &Output, operation: &str) -> Result<(), CommandFailure> {
    if output.status.success() {
        return Ok(());
    }
    Err(CommandFailure::diagnostic(format!(
        "{operation} failed: {}",
        String::from_utf8_lossy(&output.stderr).trim_end()
    )))
}

fn has_label(issue: &RemoteIssue, label: &str) -> bool {
    issue.labels.iter().any(|current| current == label)
}
