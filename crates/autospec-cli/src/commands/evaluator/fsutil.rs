//! Filesystem helpers for the evaluation store: immutable writes, the atomic
//! pointer write, and JSON loading.

use std::fs;
use std::io::Write;
use std::path::Path;

use serde::de::DeserializeOwned;
use serde::Serialize;

use super::types::{EvaluationError, EvaluationErrorKind};

/// `v<N>.json` -> N; anything else is ignored by `list`.
pub(super) fn parse_version_file_name(file_name: &str) -> Option<u32> {
    let digits = file_name.strip_prefix("v")?.strip_suffix(".json")?;
    let version: u32 = digits.parse().ok()?;
    if version >= 1 {
        Some(version)
    } else {
        None
    }
}

pub(super) fn io_err(context: &'static str) -> impl FnOnce(std::io::Error) -> EvaluationError {
    move |err| EvaluationError::new(EvaluationErrorKind::Io, format!("{context}: {err}"))
}

pub(super) fn pretty_json(value: &impl Serialize) -> Result<Vec<u8>, EvaluationError> {
    serde_json::to_vec_pretty(value).map_err(|err| {
        EvaluationError::new(
            EvaluationErrorKind::Integrity,
            format!("serialize store document: {err}"),
        )
    })
}

pub fn load_json<T: DeserializeOwned>(path: &Path) -> Result<T, EvaluationError> {
    let raw = fs::read(path).map_err(|err| {
        EvaluationError::new(
            EvaluationErrorKind::Io,
            format!("read {}: {err}", path.display()),
        )
    })?;
    serde_json::from_slice(&raw).map_err(|err| {
        EvaluationError::new(
            EvaluationErrorKind::Parse,
            format!("parse {}: {err}", path.display()),
        )
    })
}

/// Write through a same-directory temp file, fsync, and rename over the
/// target. Used for the atomic pointer (`current.json`).
pub(super) fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), EvaluationError> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(io_err("create directory"))?;
    }
    let file_name = path
        .file_name()
        .ok_or_else(|| {
            EvaluationError::new(
                EvaluationErrorKind::Io,
                format!("no file name: {}", path.display()),
            )
        })?
        .to_string_lossy()
        .into_owned();
    let tmp = path.with_file_name(format!("{file_name}.tmp-{}", std::process::id()));
    let result = (|| -> std::io::Result<()> {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&tmp, path)?;
        if let Some(dir) = path.parent() {
            fs::File::open(dir)?.sync_all()?;
        }
        Ok(())
    })();
    if let Err(err) = result {
        let _ = fs::remove_file(&tmp);
        return Err(EvaluationError::new(
            EvaluationErrorKind::Io,
            format!("atomic write {}: {err}", path.display()),
        ));
    }
    Ok(())
}

/// Write an immutable document with `O_CREAT|O_EXCL`; an existing target is
/// an `immutable` error, never an overwrite.
pub(super) fn create_new_write(path: &Path, bytes: &[u8]) -> Result<(), EvaluationError> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(io_err("create directory"))?;
    }
    if path.exists() {
        return Err(EvaluationError::new(
            EvaluationErrorKind::Immutable,
            format!(
                "refusing to overwrite immutable document at {}",
                path.display()
            ),
        ));
    }
    let result = (|| -> std::io::Result<()> {
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        if let Some(dir) = path.parent() {
            fs::File::open(dir)?.sync_all()?;
        }
        Ok(())
    })();
    if let Err(err) = result {
        if path.exists() {
            return Err(EvaluationError::new(
                EvaluationErrorKind::Immutable,
                format!("document appeared concurrently at {}", path.display()),
            ));
        }
        return Err(EvaluationError::new(
            EvaluationErrorKind::Io,
            format!("write {}: {err}", path.display()),
        ));
    }
    Ok(())
}
