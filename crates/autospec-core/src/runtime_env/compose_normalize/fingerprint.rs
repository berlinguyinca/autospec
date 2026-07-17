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
        let canonical = std::fs::canonicalize(path).ok()?;
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
        identity: file_identity(&canonical)?,
        path: canonical,
        rendered: original.clone(),
        original,
    })
}

pub(super) fn read_current(files: &[PlannedFile]) -> Result<Vec<PlannedFile>, RuntimeEnvError> {
    files
        .iter()
        .map(|file| {
            let original = std::fs::read(&file.path).map_err(|error| {
                RuntimeEnvError::new(format!(
                    "could not re-read {}: {error}",
                    file.path.display()
                ))
            })?;
            Ok(PlannedFile {
                identity: file_identity(&file.path)?,
                path: file.path.clone(),
                rendered: original.clone(),
                original,
            })
        })
        .collect()
}

pub(super) fn recheck_all(files: &[&PlannedFile]) -> Result<(), RuntimeEnvError> {
    for file in files {
        let canonical = std::fs::canonicalize(&file.path).map_err(|error| {
            RuntimeEnvError::new(format!(
                "NORMALIZE_STALE_SOURCE: {}: {error}",
                file.path.display()
            ))
        })?;
        let unchanged = canonical == file.path
            && file_identity(&file.path)? == file.identity
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
    let path = file.path.to_str().ok_or_else(|| {
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
