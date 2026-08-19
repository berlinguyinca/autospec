//! The per-lane evidence-attempt lease.
//!
//! Its own module for the same reason `post_fork` is: this file's parent may not grow, and
//! the release rule here is subtle enough to deserve somewhere to state it. A lease is an
//! flock on `<lane>/attempt.lock`, and the release has to survive a descriptor that a fork
//! handed to a supervisor which never execs.

use std::fs::{File, OpenOptions};
use std::path::Path;

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

use super::reject_symlink_path;

#[derive(Debug)]
pub(super) struct EvidenceAttemptLease {
    file: File,
}

impl Drop for EvidenceAttemptLease {
    fn drop(&mut self) {
        // Unlock explicitly rather than relying on the close.
        //
        // fork() hands the child a duplicate descriptor referring to the SAME open file
        // description, and an flock belongs to the description, not to the descriptor. This
        // process forks a supervisor that never execs (see launch_detached_supervisor), so a
        // lease open at that moment is inherited and CLOEXEC cannot help. Closing our copy
        // then releases nothing: the lock lives on in the supervisor for its whole life, and
        // the next attempt on that lane is told another attempt owns it.
        //
        // Unlocking is the release that reaches the description, so it frees the lane even
        // though the inherited descriptor is still open. Verified against the kernel: with a
        // forked holder alive, close-only leaves the lock held and unlock-then-close does not.
        let _ = self.file.unlock();
    }
}

pub(super) fn acquire_evidence_attempt_lease(lane_root: &Path) -> Result<EvidenceAttemptLease, String> {
    let path = lane_root.join("attempt.lock");
    reject_symlink_path(&path)?;
    let mut options = OpenOptions::new();
    options.create(true).read(true).write(true);
    #[cfg(unix)]
    options.mode(0o600);
    let file = options
        .open(&path)
        .map_err(|error| format!("open evidence attempt lock: {error}"))?;
    file.try_lock().map_err(|error| match error {
        std::fs::TryLockError::WouldBlock => {
            "another evidence attempt owns this exact lane".to_string()
        }
        std::fs::TryLockError::Error(error) => format!("lock evidence attempt: {error}"),
    })?;
    Ok(EvidenceAttemptLease { file })
}
