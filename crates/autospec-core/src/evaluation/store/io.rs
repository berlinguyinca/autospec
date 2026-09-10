//! Crash-safe write helpers for the evaluation store.
//!
//! Torn writes are the threat being designed out (spec:
//! `docs/specs/2026-09-05-evaluator-coevolution-design.md` §Architecture):
//!
//! - [`atomic_write`] replaces a document via a temporary file in the same
//!   directory: `create_new` temp, `write_all`, `sync_all`, `rename`, then a
//!   parent-directory `sync_all` so the rename itself is durable.
//! - [`write_immutable_json`] opens with `create_new`, so a second write to
//!   the same version is an `immutable` error naming the path.
//! - [`append_synced_line`] records the file length before appending and
//!   rolls the file back with `set_len` when the write is incomplete —
//!   including when `fail_after` injects a partial write for crash tests.

use std::fs::{self, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::de::DeserializeOwned;
use serde::Serialize;

use super::{EvaluationError, EvaluationErrorKind, Result};

/// Monotonic per-process serial so concurrent `atomic_write` calls never
/// pick the same temporary name.
static TMP_SERIAL: AtomicU64 = AtomicU64::new(0);

fn parent_of(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

fn ensure_parent(path: &Path) -> Result<()> {
    let parent = parent_of(path);
    fs::create_dir_all(parent).map_err(|error| {
        EvaluationError::io(format!(
            "failed to create parent directory {}: {error}",
            parent.display()
        ))
    })
}

fn sync_directory(directory: &Path) -> Result<()> {
    fs::File::open(directory)
        .and_then(|handle| handle.sync_all())
        .map_err(|error| {
            EvaluationError::io(format!(
                "failed to synchronize directory {}: {error}",
                directory.display()
            ))
        })
}

/// Replace `path` with `bytes` atomically.
///
/// Writes `path.tmp-<pid>-<serial>` in the same directory, `sync_all`s it,
/// renames it over `path`, then `sync_all`s the parent directory. On any
/// failure before the rename the temporary file is removed and `path` is
/// left exactly as it was.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    ensure_parent(path)?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            EvaluationError::new(
                EvaluationErrorKind::Invariant,
                format!("path has no usable file name: {}", path.display()),
            )
        })?;
    let serial = TMP_SERIAL.fetch_add(1, Ordering::Relaxed);
    let temporary =
        parent_of(path).join(format!("{file_name}.tmp-{}-{serial}", std::process::id()));

    let written = (|| -> Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        Ok(())
    })();

    if let Err(error) = written {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }

    fs::rename(&temporary, path).map_err(|error| {
        let _ = fs::remove_file(&temporary);
        EvaluationError::io(format!(
            "failed to rename {} to {}: {error}",
            temporary.display(),
            path.display()
        ))
    })?;
    sync_directory(parent_of(path))
}

/// Append `line` to `path` and sync the file.
///
/// The pre-append length is recorded first. If `fail_after` is `Some(n)`,
/// only the first `n` bytes (at most the whole line) are written before the
/// write is treated as torn; the file is then rolled back to its previous
/// length with `set_len` + `sync_all` and an `io` error is returned. Real
/// short writes follow the same rollback path, so a crash mid-append never
/// leaves a partial line behind.
pub fn append_synced_line(path: &Path, line: &[u8], fail_after: Option<usize>) -> Result<()> {
    ensure_parent(path)?;
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|error| {
            EvaluationError::io(format!(
                "failed to open {} for append: {error}",
                path.display()
            ))
        })?;
    let length = file.metadata().map(|metadata| metadata.len())?;
    file.seek(SeekFrom::End(0))?;

    // Both the injected fault and a genuine short/failed write land in the
    // same `write_result`, so both take the same rollback path (the
    // `managed_project.rs` `append_synced_line` body).
    let write_result = match fail_after.map(|limit| limit.min(line.len())) {
        Some(limit) => file
            .write_all(&line[..limit])
            .and_then(|()| Err(std::io::Error::other("injected partial append"))),
        None => file.write_all(line),
    };

    if let Err(error) = write_result {
        if let Err(rollback_error) = rollback(file, path, length) {
            return Err(EvaluationError::io(format!(
                "append to {} failed ({error}) and rollback failed ({rollback_error})",
                path.display()
            )));
        }
        return Err(EvaluationError::io(format!(
            "append to {} rolled back to {length} bytes: {error}",
            path.display()
        )));
    }
    file.sync_all().map_err(|error| {
        EvaluationError::io(format!("failed to synchronize {}: {error}", path.display()))
    })
}

fn rollback(file: fs::File, path: &Path, length: u64) -> Result<()> {
    file.set_len(length)
        .and_then(|()| file.sync_all())
        .map_err(|error| {
            EvaluationError::io(format!(
                "failed to roll back torn append in {}: {error}",
                path.display()
            ))
        })
}

/// Serialize `value` as JSON and create `path` for the first and only time.
///
/// The file is opened with `create_new`; a second write to the same path is
/// an `immutable` error whose message names the path. The content is synced
/// before the handle is dropped.
pub fn write_immutable_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let mut bytes = serde_json::to_vec_pretty(value).map_err(|error| {
        EvaluationError::new(
            EvaluationErrorKind::Invariant,
            format!(
                "failed to serialize document for {}: {error}",
                path.display()
            ),
        )
    })?;
    bytes.push(b'\n');

    ensure_parent(path)?;
    let file = match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err(EvaluationError::new(
                EvaluationErrorKind::Immutable,
                format!("immutable document already exists: {}", path.display()),
            ));
        }
        Err(error) => {
            return Err(EvaluationError::io(format!(
                "failed to create {}: {error}",
                path.display()
            )))
        }
    };
    let mut file = file;
    file.write_all(&bytes)?;
    file.sync_all().map_err(|error| {
        EvaluationError::io(format!("failed to synchronize {}: {error}", path.display()))
    })
}

/// Read and parse the JSON document at `path`.
pub fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let text = fs::read_to_string(path).map_err(|error| {
        EvaluationError::io(format!("failed to read {}: {error}", path.display()))
    })?;
    serde_json::from_str(&text).map_err(|error| {
        EvaluationError::new(
            EvaluationErrorKind::Parse,
            format!("invalid JSON in {}: {error}", path.display()),
        )
    })
}

/// Read and parse the JSON document at `path`, or `Ok(None)` if the file
/// does not exist yet.
pub fn read_json_if_exists<T: DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    match fs::read(path) {
        Ok(bytes) => String::from_utf8(bytes)
            .map_err(|error| {
                EvaluationError::new(
                    EvaluationErrorKind::Parse,
                    format!("invalid UTF-8 in {}: {error}", path.display()),
                )
            })
            .and_then(|text| {
                serde_json::from_str(&text).map_err(|error| {
                    EvaluationError::new(
                        EvaluationErrorKind::Parse,
                        format!("invalid JSON in {}: {error}", path.display()),
                    )
                })
            })
            .map(Some),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(EvaluationError::io(format!(
            "failed to read {}: {error}",
            path.display()
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "autospec-eval-io-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn atomic_write_creates_replaces_and_leaves_no_temporaries() {
        let dir = temp_dir("atomic");
        let target = dir.join("evaluators/architecture/v1.json");
        atomic_write(&target, b"v1").unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), "v1");

        atomic_write(&target, b"v2").unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), "v2");

        let leftovers = fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains(".tmp-"))
            .count();
        assert_eq!(leftovers, 0);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_immutable_json_twice_is_immutable_and_names_the_path() {
        let dir = temp_dir("immutable");
        let path = dir.join("evaluators/architecture/v1.json");
        write_immutable_json(&path, &serde_json::json!({"slot": "architecture"})).unwrap();
        let read_back: serde_json::Value = read_json(&path).unwrap();
        assert_eq!(read_back["slot"], "architecture");

        let error =
            write_immutable_json(&path, &serde_json::json!({"slot": "architecture"})).unwrap_err();
        assert_eq!(error.kind, EvaluationErrorKind::Immutable);
        assert!(
            error.message.contains("evaluators/architecture/v1.json"),
            "message must name the path: {error}"
        );
        assert!(error.to_string().starts_with("immutable: "));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_json_if_exists_distinguishes_missing_from_invalid() {
        let dir = temp_dir("read-if-exists");
        let missing = dir.join("policy.json");
        assert_eq!(
            read_json_if_exists::<serde_json::Value>(&missing).unwrap(),
            None
        );

        fs::write(&missing, b"{").unwrap();
        let error = read_json_if_exists::<serde_json::Value>(&missing).unwrap_err();
        assert_eq!(error.kind, EvaluationErrorKind::Parse);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn append_synced_line_appends_and_syncs() {
        let dir = temp_dir("append");
        let path = dir.join("events.jsonl");
        append_synced_line(&path, b"first\n", None).unwrap();
        append_synced_line(&path, b"second\n", None).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "first\nsecond\n");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn injected_partial_write_rolls_back_to_previous_length() {
        let dir = temp_dir("rollback");
        let path = dir.join("events.jsonl");
        append_synced_line(&path, b"keep\n", None).unwrap();
        let previous_length = fs::metadata(&path).unwrap().len();

        let error =
            append_synced_line(&path, b"torn line that never lands\n", Some(5)).unwrap_err();
        assert_eq!(error.kind, EvaluationErrorKind::Io);

        let metadata = fs::metadata(&path).unwrap();
        assert_eq!(
            metadata.len(),
            previous_length,
            "file must be at its previous length"
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), "keep\n");
        let _ = fs::remove_dir_all(&dir);
    }
}
