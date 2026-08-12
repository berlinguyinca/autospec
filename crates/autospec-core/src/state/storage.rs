use std::fs::{self, File};
use std::path::{Path, PathBuf};

use super::{SpecStateStore, STATE_FILE, TEMP_STATE_FILE};

pub(super) struct StatePaths {
    pub(super) root: PathBuf,
    pub(super) autospec_directory: PathBuf,
    pub(super) directory: PathBuf,
    pub(super) primary: PathBuf,
    pub(super) temporary: PathBuf,
}

impl StatePaths {
    pub(super) fn new(root: &Path) -> Self {
        let root = root.to_path_buf();
        let autospec_directory = root.join(".autospec");
        let directory = autospec_directory.join("state");
        Self {
            root,
            autospec_directory,
            primary: directory.join(STATE_FILE),
            temporary: directory.join(TEMP_STATE_FILE),
            directory,
        }
    }
}

pub(super) enum FileState {
    Missing,
    Valid(SpecStateStore),
    Invalid(String),
}

pub(super) fn load_state_file(path: &Path) -> FileState {
    let document = match fs::read_to_string(path) {
        Ok(document) => document,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return FileState::Missing,
        Err(error) => return FileState::Invalid(format!("failed to read: {error}")),
    };

    match super::parse::parse_store(&document) {
        Ok(store) => FileState::Valid(store),
        Err(error) => FileState::Invalid(error),
    }
}

pub(super) fn promote_temporary(paths: &StatePaths) -> Result<(), String> {
    let promotion = match fs::rename(&paths.temporary, &paths.primary) {
        Ok(()) => Ok(()),
        Err(first_error) => {
            if !paths.primary.exists() {
                Err(format!(
                    "failed to promote temporary spec state file {} to {}: {first_error}",
                    paths.temporary.display(),
                    paths.primary.display()
                ))
            } else {
                fs::remove_file(&paths.primary).map_err(|error| {
                    format!(
                        "failed to replace spec state file {} after temporary write: {error}",
                        paths.primary.display()
                    )
                })?;
                fs::rename(&paths.temporary, &paths.primary).map_err(|error| {
                    format!(
                        "failed to promote temporary spec state file {} to {}: {error}",
                        paths.temporary.display(),
                        paths.primary.display()
                    )
                })
            }
        }
    };
    promotion?;
    sync_directory(&paths.directory)
}

#[cfg(unix)]
pub(super) fn sync_created_directories(
    paths: &StatePaths,
    autospec_was_missing: bool,
    state_was_missing: bool,
) -> Result<(), String> {
    sync_directory(&paths.directory)?;
    if state_was_missing || autospec_was_missing {
        sync_directory(&paths.autospec_directory)?;
    }
    if autospec_was_missing {
        sync_directory(&paths.root)?;
    }
    Ok(())
}

#[cfg(not(unix))]
pub(super) fn sync_created_directories(
    _paths: &StatePaths,
    _autospec_was_missing: bool,
    _state_was_missing: bool,
) -> Result<(), String> {
    Ok(())
}

#[cfg(unix)]
pub(super) fn sync_directory(directory: &Path) -> Result<(), String> {
    File::open(directory)
        .and_then(|handle| handle.sync_all())
        .map_err(|error| {
            format!(
                "failed to synchronize spec state directory {}: {error}",
                directory.display()
            )
        })
}

#[cfg(not(unix))]
pub(super) fn sync_directory(_directory: &Path) -> Result<(), String> {
    Ok(())
}
