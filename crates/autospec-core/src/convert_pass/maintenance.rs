//! The maintenance helpers (issue #3654, acceptance criterion 2).
//!
//! When a conversion pass stalls, the human response is usually one of the
//! old trio: reset the tree, clean it, prune the worktree registration.
//! The original lock excluded only other converters, so that trio ran
//! happily on top of an in-flight pass. Every helper here goes through the
//! same guard: read the worktree marker and refuse — naming the holder and
//! its PID — while a pass is in flight. A marker that is present but
//! unreadable also refuses: failing open is how the "HELD: build error"
//! verdict got made in the first place.

use std::path::Path;
use std::process::Command;

use crate::convert_pass::marker::Marker;

/// The destructive maintenance operations a human runs against a worktree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaintenanceAction {
    /// Hard reset of the tree to `HEAD`: discard uncommitted edits.
    Reset,
    /// `git clean -fd`: remove untracked files and directories.
    Clean,
    /// `git worktree prune`: drop stale worktree registrations.
    Prune,
}

impl MaintenanceAction {
    /// The short name used in diagnostics.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Reset => "reset",
            Self::Clean => "clean",
            Self::Prune => "prune",
        }
    }

    /// Git arguments for the action, kept as separate words so no shell
    /// ever sees a command line.
    fn git_args(&self) -> &'static [&'static str] {
        match self {
            Self::Reset => &["reset", "--hard", "HEAD"],
            Self::Clean => &["clean", "-fd"],
            Self::Prune => &["worktree", "prune"],
        }
    }
}

/// Errors from a maintenance attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MaintenanceError {
    /// The worktree is held by an in-flight pass.
    Held {
        action: MaintenanceAction,
        holder: String,
        pid: u32,
    },
    /// A marker is present but unreadable; the guard fails closed.
    MarkerUnreadable {
        action: MaintenanceAction,
        reason: String,
    },
    /// Git ran and reported failure.
    Git {
        action: MaintenanceAction,
        message: String,
    },
    /// Git could not be spawned at all.
    Spawn(String),
}

impl std::fmt::Display for MaintenanceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Held {
                action,
                holder,
                pid,
            } => write!(
                f,
                "refusing {action:?} maintenance: worktree is held by {holder} (pid {pid})"
            ),
            Self::MarkerUnreadable { action, reason } => write!(
                f,
                "refusing {action:?} maintenance: worktree marker is unreadable \
                 and is treated as held: {reason}"
            ),
            Self::Git { action, message } => {
                write!(f, "{action:?} maintenance failed: {message}")
            }
            Self::Spawn(message) => write!(f, "could not spawn git for maintenance: {message}"),
        }
    }
}

impl std::error::Error for MaintenanceError {}

/// Run one maintenance action on `root` if, and only if, no conversion
/// pass holds the worktree. Returns git's trimmed stdout on success.
pub fn maintain(root: &Path, action: MaintenanceAction) -> Result<String, MaintenanceError> {
    check_marker(root, action)?;
    run_git(root, action)
}

/// Hard reset of `root` to `HEAD`, guarded by the marker.
pub fn reset(root: &Path) -> Result<String, MaintenanceError> {
    maintain(root, MaintenanceAction::Reset)
}

/// `git clean -fd` in `root`, guarded by the marker.
pub fn clean(root: &Path) -> Result<String, MaintenanceError> {
    maintain(root, MaintenanceAction::Clean)
}

/// `git worktree prune`, run from `root`, guarded by the marker.
pub fn prune(root: &Path) -> Result<String, MaintenanceError> {
    maintain(root, MaintenanceAction::Prune)
}

/// The shared refusal (acceptance criterion 2): proceed only when the
/// worktree carries no marker at all.
fn check_marker(root: &Path, action: MaintenanceAction) -> Result<(), MaintenanceError> {
    match Marker::read(root) {
        Ok(None) => Ok(()),
        Ok(Some(record)) => Err(MaintenanceError::Held {
            action,
            holder: record.holder,
            pid: record.pid,
        }),
        Err(marker_error) => Err(MaintenanceError::MarkerUnreadable {
            action,
            reason: marker_error.to_string(),
        }),
    }
}

fn run_git(root: &Path, action: MaintenanceAction) -> Result<String, MaintenanceError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(action.git_args())
        .output()
        .map_err(|error| MaintenanceError::Spawn(error.to_string()))?;
    if !output.status.success() {
        return Err(MaintenanceError::Git {
            action,
            message: failure_message(&output),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn failure_message(output: &std::process::Output) -> String {
    let mut text = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if text.is_empty() {
        text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    }
    format!("git exited {}: {text}", output.status)
}
