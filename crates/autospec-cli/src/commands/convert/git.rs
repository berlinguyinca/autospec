//! The pass's git plumbing: the thin captured-run helpers every convert step
//! builds on.
//!
//! This module exists as a child of the convert module rather than as part of
//! `convert.rs`: the pass's command file is already past the size ratchet's
//! threshold, and an oversized file may be edited and shrunk but not grown.
//! Moving the plumbing out is what pays for the progress lines (#4572).

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use crate::commands::CommandFailure;

/// Tear a conversion worktree down: force-remove it, prune the registry, and
/// take the directory. Best-effort at every step — a worktree that survives
/// teardown is an operator-visible leftover, not a silent one, and the next
/// pass's fetch/prune passes clean it up.
pub(super) fn teardown_worktree(worktree: &Path) {
    let _ = run_git(&["worktree", "remove", "--force", worktree.to_str().unwrap_or_default()]);
    let _ = run_git(&["worktree", "prune"]);
    let _ = fs::remove_dir_all(worktree);
}

/// A captured git run in `dir`: `None` on a spawn error, else the `Output`.
pub(super) fn run_capture_in(dir: &Path, args: &[&str]) -> Option<Output> {
    Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .ok()
}

/// A git run in the pass's own repository; the stderr rides along in the
/// error, because "git failed" without the reason is an unactionable hold.
pub(super) fn run_git(args: &[&str]) -> Result<(), CommandFailure> {
    let output = Command::new("git").args(args).output().map_err(|error| {
        CommandFailure::transient(format!("could not run git {args:?}: {error}"))
    })?;
    if !output.status.success() {
        return Err(CommandFailure::status(
            format!("git {args:?} failed: {}", String::from_utf8_lossy(&output.stderr).trim()),
            output.status.code().unwrap_or(1),
        ));
    }
    Ok(())
}

/// A git run in a conversion worktree, same error contract as [`run_git`].
pub(super) fn run_git_in(dir: &Path, args: &[&str]) -> Result<(), CommandFailure> {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .map_err(|error| CommandFailure::transient(format!("could not run git in {dir:?}: {error}")))?;
    if !output.status.success() {
        return Err(CommandFailure::status(
            format!("git {args:?} failed: {}", String::from_utf8_lossy(&output.stderr).trim()),
            output.status.code().unwrap_or(1),
        ));
    }
    Ok(())
}

/// A git run whose stdout is the answer (a SHA, a ref, a count), trimmed.
pub(super) fn run_git_capture(args: &[&str]) -> Result<String, CommandFailure> {
    let output = Command::new("git").args(args).output().map_err(|error| {
        CommandFailure::transient(format!("could not run git {args:?}: {error}"))
    })?;
    if !output.status.success() {
        return Err(CommandFailure::status(
            format!("git {args:?} failed"),
            output.status.code().unwrap_or(1),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}
