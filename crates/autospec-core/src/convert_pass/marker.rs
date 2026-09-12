//! The worktree marker (issue #3654, acceptance criterion 1).
//!
//! A marker file inside the worktree names the holder of an in-flight
//! conversion pass and its PID. The original worktree lock excluded only
//! other converters: a human operator could reset, clean, or prune the tree
//! mid-pass and the corrupted result was memoized as fact. The marker makes
//! the holder visible to the operator and gives the shared maintenance
//! helpers in [`super::maintenance`] something to refuse against.
//!
//! Acquisition is atomic (`O_EXCL`), so two converters racing on one
//! worktree cannot both hold it. Release is RAII and removes only a marker
//! that still names the releasing process, so a stale marker left by a
//! killed holder is never deleted by someone else. A marker that exists but
//! cannot be read is held, full stop: the guard fails closed.

use std::fs;
use std::fs::OpenOptions;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// Directory inside the worktree that holds the marker.
pub const MARKER_DIR: &str = ".autospec";

/// Marker file name inside [`MARKER_DIR`].
pub const MARKER_FILE: &str = "convert-pass";

/// Maximum length of a holder name, in bytes.
pub const MAX_HOLDER_LEN: usize = 120;

/// What a marker names: who holds the worktree and since when.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarkerRecord {
    /// Operator- or agent-chosen name, e.g. `operator@host`.
    pub holder: String,
    /// PID of the holding process.
    pub pid: u32,
    /// Seconds since the Unix epoch when the marker was acquired.
    pub since_epoch_secs: u64,
}

/// Errors from reading or acquiring a marker.
#[derive(Debug)]
pub enum MarkerError {
    /// A marker already names a holder for this worktree.
    Held(MarkerRecord),
    /// A marker is present but unreadable; the holder is unknown, so the
    /// worktree must be treated as held (fail closed).
    UnreadableHolder { path: PathBuf, reason: String },
    /// The caller named an unusable holder.
    InvalidHolder(String),
    /// A filesystem failure while reading, writing, or removing the marker.
    Io { path: PathBuf, reason: String },
}

impl std::fmt::Display for MarkerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Held(record) => {
                write!(
                    f,
                    "worktree is held by {} (pid {})",
                    record.holder, record.pid
                )
            }
            Self::UnreadableHolder { path, reason } => write!(
                f,
                "marker at {} is unreadable and is treated as held: {reason}",
                path.display()
            ),
            Self::InvalidHolder(holder) => write!(f, "unusable holder name: {holder:?}"),
            Self::Io { path, reason } => {
                write!(f, "marker I/O at {} failed: {reason}", path.display())
            }
        }
    }
}

impl std::error::Error for MarkerError {}

/// The RAII handle for a held worktree marker.
#[derive(Debug)]
pub struct Marker {
    root: PathBuf,
    holder: String,
    pid: u32,
}

impl Marker {
    /// Path of the marker for this worktree: `<root>/.autospec/convert-pass`.
    pub fn path(root: &Path) -> PathBuf {
        root.join(MARKER_DIR).join(MARKER_FILE)
    }

    /// Atomically claim `root` for `holder`.
    ///
    /// Fails with [`MarkerError::Held`] when a readable marker is already
    /// present and with [`MarkerError::UnreadableHolder`] when a marker is
    /// present but cannot be parsed.
    pub fn acquire(root: &Path, holder: &str) -> Result<Marker, MarkerError> {
        validate_holder(holder)?;
        let dir = root.join(MARKER_DIR);
        fs::create_dir_all(&dir).map_err(|error| MarkerError::Io {
            path: dir,
            reason: error.to_string(),
        })?;
        let path = Self::path(root);
        let record = MarkerRecord {
            holder: holder.to_string(),
            pid: std::process::id(),
            since_epoch_secs: now_epoch_secs(),
        };
        let body = serde_json::to_vec(&record).map_err(|error| MarkerError::Io {
            path: path.clone(),
            reason: error.to_string(),
        })?;
        let mut file = loop {
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => break file,
                Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                    return match Self::read(root)? {
                        Some(record) => Err(MarkerError::Held(record)),
                        // The marker vanished between the two syscalls; retry.
                        None => continue,
                    };
                }
                Err(error) => {
                    return Err(MarkerError::Io {
                        path,
                        reason: error.to_string(),
                    });
                }
            }
        };
        if let Err(error) = file.write_all(&body) {
            // A half-written marker would fail closed against the very
            // process that owns it; undo the create instead.
            let _ = fs::remove_file(&path);
            return Err(MarkerError::Io {
                path,
                reason: error.to_string(),
            });
        }
        Ok(Marker {
            root: root.to_path_buf(),
            holder: holder.to_string(),
            pid: record.pid,
        })
    }

    /// Read the current marker, if any.
    ///
    /// `Ok(None)` means the worktree is not held; `Ok(Some(record))` names
    /// the holder. `Err` means a marker is present but unusable; callers
    /// gating a destructive operation must treat that as held.
    pub fn read(root: &Path) -> Result<Option<MarkerRecord>, MarkerError> {
        let path = Self::path(root);
        match fs::read(&path) {
            Ok(body) => serde_json::from_slice(&body).map(Some).map_err(|error| {
                MarkerError::UnreadableHolder {
                    path,
                    reason: error.to_string(),
                }
            }),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
            Err(error) => Err(MarkerError::Io {
                path,
                reason: error.to_string(),
            }),
        }
    }

    /// True when the worktree must be treated as held: a marker is present,
    /// readable or not.
    pub fn held(root: &Path) -> bool {
        !matches!(Self::read(root), Ok(None))
    }

    /// Remove the marker, but only while it still names this process.
    fn release(&self) {
        let path = Self::path(&self.root);
        let body = match fs::read(&path) {
            Ok(body) => body,
            Err(_) => return, // already gone; nothing to release
        };
        let record = match serde_json::from_slice::<MarkerRecord>(&body) {
            Ok(record) => record,
            Err(_) => return, // not ours to interpret, so not ours to remove
        };
        if record.holder == self.holder && record.pid == self.pid {
            let _ = fs::remove_file(path);
        }
    }
}

impl Drop for Marker {
    fn drop(&mut self) {
        self.release();
    }
}

fn validate_holder(holder: &str) -> Result<(), MarkerError> {
    let invalid = holder.trim().is_empty()
        || holder.len() > MAX_HOLDER_LEN
        || holder.contains('\0')
        || holder.chars().any(char::is_whitespace);
    if invalid {
        return Err(MarkerError::InvalidHolder(holder.to_string()));
    }
    Ok(())
}

fn now_epoch_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "autospec-convert-marker-{name}-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::SeqCst)
        ));
        fs::create_dir_all(&dir).expect("create scratch dir");
        dir
    }

    #[test]
    fn acquire_names_holder_and_pid_and_release_removes_the_file() {
        let root = scratch("acquire-release");
        let marker = Marker::acquire(&root, "unit@host").expect("acquire");
        let path = Marker::path(&root);
        assert!(path.is_file(), "the marker file must exist while held");
        let record = Marker::read(&root).expect("read").expect("some record");
        assert_eq!(record.holder, "unit@host");
        assert_eq!(record.pid, std::process::id());
        drop(marker);
        assert!(
            !Marker::held(&root),
            "release on drop must remove the marker"
        );
    }

    #[test]
    fn a_second_holder_is_refused_and_named() {
        let root = scratch("refusal");
        let first = Marker::acquire(&root, "first").expect("first acquires");
        let second = Marker::acquire(&root, "second");
        match second {
            Err(MarkerError::Held(record)) => {
                assert_eq!(record.holder, "first");
            }
            other => panic!("second acquire must be refused, got {other:?}"),
        }
        drop(first);
        let reopened = Marker::acquire(&root, "second").expect("acquires after release");
        drop(reopened);
    }

    #[test]
    fn a_corrupt_marker_is_treated_as_held() {
        let root = scratch("corrupt");
        fs::create_dir_all(Marker::path(&root).parent().expect("marker dir")).expect("dir");
        fs::write(Marker::path(&root), b"this is not json at all").expect("write");
        assert!(Marker::held(&root), "an unreadable marker fails closed");
        assert!(
            Marker::acquire(&root, "late").is_err(),
            "refuses to acquire over it"
        );
    }

    #[test]
    fn unusable_holder_names_are_rejected() {
        let root = scratch("holder");
        for holder in [
            "",
            "   ",
            "a\0b",
            "has space",
            &"x".repeat(MAX_HOLDER_LEN + 1),
        ] {
            assert!(
                matches!(
                    Marker::acquire(&root, holder),
                    Err(MarkerError::InvalidHolder(_))
                ),
                "{holder:?} must be rejected"
            );
        }
        assert!(
            !Marker::held(&root),
            "a rejected holder must not leave a marker"
        );
    }
}
