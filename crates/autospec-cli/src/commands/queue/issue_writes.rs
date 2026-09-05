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
