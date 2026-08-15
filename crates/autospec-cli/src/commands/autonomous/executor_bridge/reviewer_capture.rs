use std::fs::{File, OpenOptions};
use std::path::PathBuf;

#[cfg(unix)]
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

#[cfg(unix)]
use nix::fcntl::OFlag;

use super::{
    reject_symlink_path, validate_private_state_file, ReviewerCapturePolicy,
    MAX_DIRECT_OUTPUT_BYTES,
};

#[cfg(unix)]
pub(super) struct ActiveReviewerCapture {
    pub(super) artifacts: Vec<(PathBuf, File, u64, u64)>,
}

#[cfg(unix)]
impl ActiveReviewerCapture {
    #[cfg(test)]
    pub(super) fn empty() -> Self {
        Self {
            artifacts: Vec::new(),
        }
    }

    pub(super) fn open(policy: &ReviewerCapturePolicy) -> Result<Self, String> {
        let mut artifacts = Vec::new();
        for path in &policy.artifacts {
            reject_symlink_path(path)?;
            validate_private_state_file(path)?;
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .custom_flags(OFlag::O_NOFOLLOW.bits())
                .open(path)
                .map_err(|error| format!("open reviewer capture {}: {error}", path.display()))?;
            let metadata = file
                .metadata()
                .map_err(|error| format!("inspect reviewer capture: {error}"))?;
            let current = fs::metadata(path)
                .map_err(|error| format!("reinspect reviewer capture: {error}"))?;
            if metadata.dev() != current.dev() || metadata.ino() != current.ino() {
                return Err("reviewer capture identity changed while opening".to_string());
            }
            artifacts.push((path.clone(), file, metadata.dev(), metadata.ino()));
        }
        Ok(Self { artifacts })
    }

    pub(super) fn at_limit(&self) -> Result<bool, String> {
        self.artifacts.iter().try_fold(false, |overflow, item| {
            item.1
                .metadata()
                .map(|metadata| overflow || metadata.len() >= MAX_DIRECT_OUTPUT_BYTES)
                .map_err(|error| format!("inspect reviewer capture length: {error}"))
        })
    }

    pub(super) fn finalize(&self) -> Result<bool, String> {
        let mut overflow = false;
        for (path, file, device, inode) in &self.artifacts {
            validate_private_state_file(path)?;
            let current = fs::metadata(path)
                .map_err(|error| format!("reinspect reviewer capture: {error}"))?;
            if current.dev() != *device || current.ino() != *inode {
                return Err("reviewer capture identity changed during execution".to_string());
            }
            let length = file
                .metadata()
                .map_err(|error| format!("inspect reviewer capture length: {error}"))?
                .len();
            overflow |= length >= MAX_DIRECT_OUTPUT_BYTES;
            if length > MAX_DIRECT_OUTPUT_BYTES {
                file.set_len(MAX_DIRECT_OUTPUT_BYTES)
                    .and_then(|()| file.sync_all())
                    .map_err(|error| format!("bound reviewer capture: {error}"))?;
            }
        }
        Ok(overflow)
    }
}

#[cfg(windows)]
pub(super) struct ActiveReviewerCapture {
    pub(super) artifacts: Vec<(PathBuf, File)>,
}

#[cfg(windows)]
impl ActiveReviewerCapture {
    #[cfg(test)]
    pub(super) fn empty() -> Self {
        Self {
            artifacts: Vec::new(),
        }
    }

    pub(super) fn open(policy: &ReviewerCapturePolicy) -> Result<Self, String> {
        let mut artifacts = Vec::new();
        for path in &policy.artifacts {
            reject_symlink_path(path)?;
            validate_private_state_file(path)?;
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .open(path)
                .map_err(|error| format!("open reviewer capture {}: {error}", path.display()))?;
            artifacts.push((path.clone(), file));
        }
        Ok(Self { artifacts })
    }

    pub(super) fn at_limit(&self) -> Result<bool, String> {
        self.artifacts.iter().try_fold(false, |overflow, item| {
            item.1
                .metadata()
                .map(|metadata| overflow || metadata.len() >= MAX_DIRECT_OUTPUT_BYTES)
                .map_err(|error| format!("inspect reviewer capture length: {error}"))
        })
    }

    pub(super) fn finalize(&self) -> Result<bool, String> {
        let mut overflow = false;
        for (path, file) in &self.artifacts {
            reject_symlink_path(path)?;
            validate_private_state_file(path)?;
            let length = file
                .metadata()
                .map_err(|error| format!("inspect reviewer capture length: {error}"))?
                .len();
            overflow |= length >= MAX_DIRECT_OUTPUT_BYTES;
            if length > MAX_DIRECT_OUTPUT_BYTES {
                file.set_len(MAX_DIRECT_OUTPUT_BYTES)
                    .and_then(|()| file.sync_all())
                    .map_err(|error| format!("bound reviewer capture: {error}"))?;
            }
        }
        Ok(overflow)
    }
}
