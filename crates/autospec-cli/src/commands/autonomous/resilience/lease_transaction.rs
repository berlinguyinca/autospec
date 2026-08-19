//! The resilience lease transaction.
//!
//! Its own module because `resilience.rs` is past the size ratchet, and because the release
//! rule deserves somewhere to be stated: closing the descriptor is not a release once a fork
//! has duplicated it.

use std::fs;
use std::path::Path;

use super::StoreError;

pub(super) struct LeaseTransaction {
    file: fs::File,
}

impl Drop for LeaseTransaction {
    fn drop(&mut self) {
        // Release by unlocking, not by closing.
        //
        // `with_current_lifecycle_lease` holds this transaction across an arbitrary
        // operation, and fork() duplicates the descriptor into a child that refers to the
        // same open file description. An flock belongs to the description, so if a child is
        // forked while this is open -- and the conductor's supervisor never execs -- closing
        // our copy leaves the lease held for as long as that child lives, and the next
        // conductor is told the lease is Held. Unlocking reaches the description and frees
        // it. Same defect, same fix as the evidence-attempt lease in #3225.
        let _ = self.file.unlock();
    }
}

impl LeaseTransaction {
    #[cfg(unix)]
    pub(super) fn try_open(path: &Path) -> Result<Self, StoreError> {
        let parent = path.parent().expect("lease lock path has a parent");
        fs::create_dir_all(parent).map_err(|error| {
            StoreError::Diagnostic(format!(
                "cannot create resilience lease directory {}: {error}",
                parent.display()
            ))
        })?;
        let file = fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(path)
            .map_err(|error| {
                StoreError::Diagnostic(format!(
                    "cannot open resilience lease lock {}: {error}",
                    path.display()
                ))
            })?;

        match file.try_lock() {
            Ok(()) => Ok(Self { file }),
            Err(fs::TryLockError::WouldBlock) => Err(StoreError::Held),
            Err(fs::TryLockError::Error(error)) => Err(StoreError::Diagnostic(format!(
                "cannot lock resilience lease {}: {error}",
                path.display()
            ))),
        }
    }

    #[cfg(not(unix))]
    pub(super) fn try_open(_path: &Path) -> Result<Self, StoreError> {
        Err(StoreError::Diagnostic(
            "resilience lease transactions require Unix flock support".to_string(),
        ))
    }
}
