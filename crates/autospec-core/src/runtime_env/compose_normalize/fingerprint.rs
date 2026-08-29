use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use super::{NormalizationPlan, PlannedFile};
use crate::runtime_env::RuntimeEnvError;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct FileIdentity {
    device: u64,
    inode: u64,
}

impl NormalizationPlan {
    pub fn rendered_file(&self, path: &Path) -> Option<String> {
        self.rendered_bytes(path)
            .and_then(|bytes| String::from_utf8(bytes).ok())
    }

    pub fn rendered_bytes(&self, path: &Path) -> Option<Vec<u8>> {
        let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| {
            if path.is_absolute() {
                path.to_path_buf()
            } else {
                self.repo.join(path)
            }
        });
        self.files
            .iter()
            .find(|file| file.path == canonical)
            .map(|file| file.rendered.clone())
    }

    pub fn changed_files(&self) -> usize {
        self.files
            .iter()
            .filter(|file| file.original != file.rendered)
            .count()
    }

    pub fn to_json(&self) -> Result<String, RuntimeEnvError> {
        serde_json::to_string(self).map_err(|error| {
            RuntimeEnvError::new(format!("could not encode normalization plan: {error}"))
        })
    }
}

pub(super) fn read_inputs(
    repo: &Path,
    compose_files: &[PathBuf],
    manifest_path: &Path,
) -> Result<Vec<PlannedFile>, RuntimeEnvError> {
    let mut paths = compose_files.to_vec();
    paths.push(manifest_path.to_path_buf());
    paths.sort();
    paths.dedup();
    paths
        .into_iter()
        .map(|path| read_input(repo, &path))
        .collect()
}

fn read_input(repo: &Path, path: &Path) -> Result<PlannedFile, RuntimeEnvError> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return read_missing_input(repo, path)
        }
        Err(error) => {
            return Err(RuntimeEnvError::new(format!(
                "could not inspect {}: {error}",
                path.display()
            )))
        }
    };
    if !metadata.file_type().is_file() {
        return Err(RuntimeEnvError::new(format!(
            "normalization input is not a regular file: {}",
            path.display()
        )));
    }
    let canonical = std::fs::canonicalize(path).map_err(|error| {
        RuntimeEnvError::new(format!(
            "could not canonicalize {}: {error}",
            path.display()
        ))
    })?;
    if !canonical.starts_with(repo) {
        return Err(RuntimeEnvError::new(format!(
            "normalization input is outside repository: {}",
            path.display()
        )));
    }
    let original = std::fs::read(&canonical).map_err(|error| {
        RuntimeEnvError::new(format!("could not read {}: {error}", canonical.display()))
    })?;
    Ok(PlannedFile {
        identity: Some(file_identity(&canonical)?),
        parent_existed: true,
        repo: repo.to_path_buf(),
        path: canonical,
        rendered: original.clone(),
        original,
    })
}

fn read_missing_input(repo: &Path, path: &Path) -> Result<PlannedFile, RuntimeEnvError> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        repo.join(path)
    };
    validate_missing_destination(repo, &absolute)?;
    let parent_existed = absolute
        .parent()
        .is_some_and(|parent| std::fs::symlink_metadata(parent).is_ok());
    Ok(PlannedFile {
        identity: None,
        parent_existed,
        repo: repo.to_path_buf(),
        path: absolute,
        original: Vec::new(),
        rendered: Vec::new(),
    })
}

pub(super) fn validate_missing_destination(
    repo: &Path,
    path: &Path,
) -> Result<(), RuntimeEnvError> {
    if !path.starts_with(repo) || path == repo {
        return Err(RuntimeEnvError::new(format!(
            "normalization input is outside repository: {}",
            path.display()
        )));
    }
    let parent = path
        .parent()
        .ok_or_else(|| RuntimeEnvError::new("normalization path has no parent"))?;
    validate_destination_parent(repo, parent)
}

fn validate_destination_parent(repo: &Path, parent: &Path) -> Result<(), RuntimeEnvError> {
    let canonical_repo = std::fs::canonicalize(repo).map_err(|error| {
        RuntimeEnvError::new(format!(
            "could not canonicalize repository {}: {error}",
            repo.display()
        ))
    })?;
    match std::fs::symlink_metadata(parent) {
        Ok(metadata) if metadata.file_type().is_dir() => {
            let canonical = std::fs::canonicalize(parent).map_err(|error| {
                RuntimeEnvError::new(format!(
                    "could not canonicalize {}: {error}",
                    parent.display()
                ))
            })?;
            let fixed_platform_alias = cfg!(target_os = "macos")
                && [
                    (Path::new("/var"), Path::new("/private/var")),
                    (Path::new("/tmp"), Path::new("/private/tmp")),
                    (Path::new("/etc"), Path::new("/private/etc")),
                ]
                .iter()
                .any(|(alias, target)| {
                    parent
                        .strip_prefix(alias)
                        .ok()
                        .is_some_and(|suffix| target.join(suffix) == canonical)
                });
            if (canonical != parent && !fixed_platform_alias)
                || !canonical.starts_with(&canonical_repo)
            {
                return Err(RuntimeEnvError::new(format!(
                    "normalization destination parent is unsafe: {}",
                    parent.display()
                )));
            }
        }
        Ok(_) => {
            return Err(RuntimeEnvError::new(format!(
                "normalization destination parent is not a regular directory: {}",
                parent.display()
            )))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let ancestor = parent.parent().ok_or_else(|| {
                RuntimeEnvError::new("normalization destination parent has no ancestor")
            })?;
            if std::fs::canonicalize(ancestor).ok().as_deref() != Some(&canonical_repo) {
                return Err(RuntimeEnvError::new(format!(
                    "normalization destination parent is outside repository: {}",
                    parent.display()
                )));
            }
        }
        Err(error) => {
            return Err(RuntimeEnvError::new(format!(
                "could not inspect {}: {error}",
                parent.display()
            )))
        }
    }
    Ok(())
}

pub(super) fn read_current(files: &[PlannedFile]) -> Result<Vec<PlannedFile>, RuntimeEnvError> {
    files.iter().map(read_current_file).collect()
}

fn read_current_file(file: &PlannedFile) -> Result<PlannedFile, RuntimeEnvError> {
    if file.identity.is_none() {
        validate_missing_destination(&file.repo, &file.path)?;
        return match std::fs::symlink_metadata(&file.path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(file.clone()),
            Ok(_) => Err(RuntimeEnvError::new(format!(
                "NORMALIZE_STALE_SOURCE: {} appeared after planning",
                file.path.display()
            ))),
            Err(error) => Err(RuntimeEnvError::new(format!(
                "could not recheck {}: {error}",
                file.path.display()
            ))),
        };
    }
    let original = std::fs::read(&file.path).map_err(|error| {
        RuntimeEnvError::new(format!(
            "could not re-read {}: {error}",
            file.path.display()
        ))
    })?;
    Ok(PlannedFile {
        identity: Some(file_identity(&file.path)?),
        parent_existed: true,
        repo: file.repo.clone(),
        path: file.path.clone(),
        rendered: original.clone(),
        original,
    })
}

pub(super) fn recheck_all(files: &[&PlannedFile]) -> Result<(), RuntimeEnvError> {
    for file in files {
        if file.identity.is_none() {
            validate_missing_destination(&file.repo, &file.path)?;
            if std::fs::symlink_metadata(&file.path).is_ok() {
                return Err(RuntimeEnvError::new(format!(
                    "NORMALIZE_STALE_SOURCE: {} appeared before rename",
                    file.path.display()
                )));
            }
            continue;
        }
        let canonical = std::fs::canonicalize(&file.path).map_err(|error| {
            RuntimeEnvError::new(format!(
                "NORMALIZE_STALE_SOURCE: {}: {error}",
                file.path.display()
            ))
        })?;
        let unchanged = canonical == file.path
            && Some(file_identity(&file.path)?) == file.identity
            && std::fs::read(&file.path).is_ok_and(|bytes| bytes == file.original);
        if !unchanged {
            return Err(RuntimeEnvError::new(format!(
                "NORMALIZE_STALE_SOURCE: {} changed immediately before rename",
                file.path.display()
            )));
        }
    }
    Ok(())
}

#[cfg(unix)]
pub(super) fn file_identity(path: &Path) -> Result<FileIdentity, RuntimeEnvError> {
    use std::os::unix::fs::MetadataExt;

    let metadata = regular_metadata(path)?;
    Ok(FileIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

#[cfg(not(unix))]
pub(super) fn file_identity(path: &Path) -> Result<FileIdentity, RuntimeEnvError> {
    let metadata = regular_metadata(path)?;
    Ok(FileIdentity {
        device: metadata.len(),
        inode: 0,
    })
}

fn regular_metadata(path: &Path) -> Result<std::fs::Metadata, RuntimeEnvError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| {
        RuntimeEnvError::new(format!("could not inspect {}: {error}", path.display()))
    })?;
    if !metadata.file_type().is_file() {
        return Err(RuntimeEnvError::new(format!(
            "normalization input is not a regular file: {}",
            path.display()
        )));
    }
    Ok(metadata)
}

pub(super) fn digest(
    files: &[PlannedFile],
    resolved: &[u8],
    schema: u32,
    policy: u32,
) -> Result<String, RuntimeEnvError> {
    let inputs = files
        .iter()
        .map(digest_input)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(digest_parts(&inputs, resolved, schema, policy))
}

fn digest_input(file: &PlannedFile) -> Result<(&str, &[u8]), RuntimeEnvError> {
    let relative = file.path.strip_prefix(&file.repo).map_err(|_| {
        RuntimeEnvError::new(format!(
            "normalization path is outside repository: {}",
            file.path.display()
        ))
    })?;
    let path = relative.to_str().ok_or_else(|| {
        RuntimeEnvError::new(format!(
            "normalization path is not UTF-8: {}",
            file.path.display()
        ))
    })?;
    Ok((path, &file.original))
}

fn digest_parts(files: &[(&str, &[u8])], resolved: &[u8], schema: u32, policy: u32) -> String {
    let mut ordered = files.to_vec();
    ordered.sort_by_key(|(path, _)| *path);
    let mut digest = Sha256::new();
    digest.update(schema.to_be_bytes());
    digest.update(policy.to_be_bytes());
    hash_part(&mut digest, resolved);
    for (path, bytes) in ordered {
        hash_part(&mut digest, path.as_bytes());
        hash_part(&mut digest, bytes);
    }
    digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn hash_part(digest: &mut Sha256, value: &[u8]) {
    digest.update((value.len() as u64).to_be_bytes());
    digest.update(value);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_is_fixed_width_and_input_order_independent() {
        let first = digest_parts(&[("b", b"two"), ("a", b"one")], b"model", 1, 1);
        let second = digest_parts(&[("a", b"one"), ("b", b"two")], b"model", 1, 1);
        assert_eq!(first.len(), 64);
        assert_eq!(first, second);
    }

    #[test]
    fn digest_changes_for_every_versioned_input() {
        let base = digest_parts(&[("a", b"one")], b"model", 1, 1);
        for changed in [
            digest_parts(&[("b", b"one")], b"model", 1, 1),
            digest_parts(&[("a", b"two")], b"model", 1, 1),
            digest_parts(&[("a", b"one")], b"other", 1, 1),
            digest_parts(&[("a", b"one")], b"model", 2, 1),
            digest_parts(&[("a", b"one")], b"model", 1, 2),
        ] {
            assert_ne!(base, changed);
        }
    }
}
