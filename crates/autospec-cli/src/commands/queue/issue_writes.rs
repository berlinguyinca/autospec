//! The GitHub writes a safety verdict performs.
//!
//! Split out of `queue.rs` so the review flow reads as decisions rather than
//! transport, and to keep that file inside the size ratchet.

use super::*;

pub(super) fn update_issue_body(repo: &str, number: u64, body: &str) -> Result<(), CommandFailure> {
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

pub(super) fn add_issue_label(repo: &str, number: u64, label: &str) -> Result<(), CommandFailure> {
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

/// Ensures the `autospec:needs-human` review-routing label exists in the repo.
///
/// Zero-output review routing applies this label to issues whose consecutive
/// zero-effect runs hit the review threshold. Label creation is repo-level and
/// idempotent: an existing label short-circuits before any write.
pub(super) fn ensure_needs_human_label(repo: &str) -> Result<(), CommandFailure> {
    let endpoint = format!("repos/{repo}/labels/autospec:needs-human");
    if run_gh_read_with_retry(&["api", "--method", "GET", &endpoint], "read review label").is_ok() {
        return Ok(());
    }
    let create = run_gh(&[
        "api",
        "--method",
        "POST",
        &format!("repos/{repo}/labels"),
        "-f",
        "name=autospec:needs-human",
        "-f",
        "color=d4c5f9",
        "-f",
        "description=Autospec autonomous review routing",
    ])?;
    if create.status.success() {
        return Ok(());
    }
    // Concurrent creation or a pre-existing label both surface as a failed POST
    // here; confirm by re-reading before declaring failure.
    if run_gh_read_with_retry(
        &["api", "--method", "GET", &endpoint],
        "confirm label creation",
    )
    .is_ok()
    {
        return Ok(());
    }
    Err(CommandFailure::diagnostic(format!(
        "gh review label write for {repo} failed: {}",
        command_error(&create)
    )))
}

/// Removes one label, tolerating its absence.
///
/// Only reached on a re-derived pass under `--recheck`, so the removal always
/// has a verdict behind it. A 404 means the label is already gone, which is the
/// state we wanted — treating that as failure would make a retried recheck fail
/// on a queue it had already repaired.
pub(super) fn remove_issue_label(
    repo: &str,
    number: u64,
    label: &str,
) -> Result<(), CommandFailure> {
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
