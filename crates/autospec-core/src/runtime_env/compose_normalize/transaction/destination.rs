use std::collections::HashSet;

use super::Staged;
use crate::runtime_env::compose_normalize::{fingerprint, PlannedFile};
use crate::runtime_env::RuntimeEnvError;

pub(super) fn ensure_parent(file: &PlannedFile) -> Result<(), RuntimeEnvError> {
    let parent = file
        .path
        .parent()
        .ok_or_else(|| RuntimeEnvError::new("normalization path has no parent"))?;
    match std::fs::create_dir(parent) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => {
            return Err(RuntimeEnvError::new(format!(
                "NORMALIZE_STAGE_FAILED: could not create {}: {error}",
                parent.display()
            )))
        }
    }
    fingerprint::validate_missing_destination(&file.repo, &file.path)
}

pub(super) fn remove_created(file: &PlannedFile) -> Result<(), RuntimeEnvError> {
    std::fs::remove_file(&file.path).map_err(|error| {
        RuntimeEnvError::new(format!("could not remove created destination: {error}"))
    })?;
    super::sync_parent(&file.path)?;
    if file.parent_existed {
        return Ok(());
    }
    let parent = file
        .path
        .parent()
        .ok_or_else(|| RuntimeEnvError::new("normalization path has no parent"))?;
    std::fs::remove_dir(parent)
        .map_err(|error| RuntimeEnvError::new(format!("could not remove created parent: {error}")))
}

pub(super) fn cleanup_created_parents(staged: &[Staged<'_>]) {
    let parents = staged
        .iter()
        .filter(|item| item.file.identity.is_none() && !item.file.parent_existed)
        .filter_map(|item| item.file.path.parent())
        .collect::<HashSet<_>>();
    for parent in parents {
        let _ = std::fs::remove_dir(parent);
    }
}
