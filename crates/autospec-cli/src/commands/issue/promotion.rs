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
) -> Result<Vec<String>, CommandFailure> {
    let mut removed = Vec::new();
    for label in remove_labels {
        if has_label(initial, label) {
            if let Err(error) = remove_issue_label(repo, number, label) {
                return match rollback_cleanup_labels(repo, number, initial, remove_labels, &removed)
                {
                    Ok(()) => Err(error),
                    Err(rollback) => Err(combine_failures(rollback, error)),
                };
            }
            removed.push(label.clone());
        }
    }
    Ok(removed)
}

pub(super) fn rollback_cleanup_labels(
    repo: &str,
    number: u64,
    initial: &RemoteIssue,
    owned_labels: &[String],
    removed_labels: &[String],
) -> Result<(), CommandFailure> {
    let mut mutation_failures = Vec::new();
    for label in removed_labels {
        if let Err(error) = add_issue_label(repo, number, label) {
            mutation_failures.push(error.message);
        }
    }

    let current = match read_issue(repo, number) {
        Ok(issue) => issue,
        Err(error) => {
            return Err(rollback_failure(
                mutation_failures,
                format!("could not verify cleanup rollback state: {}", error.message),
            ))
        }
    };
    let residual_state = owned_labels
        .iter()
        .filter(|label| has_label(&current, label) != has_label(initial, label))
        .map(|label| format!("{label} was not restored to its initial state"))
        .collect::<Vec<_>>();
    if residual_state.is_empty() {
        return Ok(());
    }
    Err(rollback_failure(
        mutation_failures,
        residual_state.join("; "),
    ))
}

pub(super) fn rollback_owned_labels(
    repo: &str,
    number: u64,
    initial: &RemoteIssue,
    removed_labels: &[String],
) -> Result<(), CommandFailure> {
    let mut mutation_failures = Vec::new();
    if let Err(error) = remove_issue_label(repo, number, "auto-implement") {
        mutation_failures.push(error.message);
    }
    if !has_label(initial, "safety:reviewed") {
        if let Err(error) = remove_issue_label(repo, number, "safety:reviewed") {
            mutation_failures.push(error.message);
        }
    }
    for label in removed_labels {
        if has_label(initial, label) {
            if let Err(error) = add_issue_label(repo, number, label) {
                mutation_failures.push(error.message);
            }
        }
    }

    let current = match read_issue(repo, number) {
        Ok(issue) => issue,
        Err(error) => {
            return Err(rollback_failure(
                mutation_failures,
                format!("could not verify rollback state: {}", error.message),
            ))
        }
    };
    let mut residual_state = Vec::new();
    if has_label(&current, "auto-implement") {
        residual_state.push("auto-implement remains present".to_string());
    }
    if has_label(&current, "safety:reviewed") != has_label(initial, "safety:reviewed") {
        residual_state.push("safety:reviewed was not restored to its initial state".to_string());
    }
    for label in removed_labels {
        if has_label(initial, label) && !has_label(&current, label) {
            residual_state.push(format!("{label} remains absent"));
        }
    }
    if residual_state.is_empty() {
        return Ok(());
    }
    Err(rollback_failure(
        mutation_failures,
        residual_state.join("; "),
    ))
}

fn rollback_failure(mut mutation_failures: Vec<String>, verification: String) -> CommandFailure {
    mutation_failures.push(verification);
    CommandFailure::diagnostic(format!(
        "ISSUE_PROMOTION_ROLLBACK_FAILED: {}",
        mutation_failures.join("; ")
    ))
}

fn combine_failures(rollback: CommandFailure, original: CommandFailure) -> CommandFailure {
    CommandFailure::status(
        format!(
            "{}; original failure: {}",
            rollback.message, original.message
        ),
        rollback.exit_code,
    )
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
