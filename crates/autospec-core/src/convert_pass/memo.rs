//! The durable verdict memo (issue #3654, acceptance criteria 3 and 4).
//!
//! The memo maps (patch hash, base sha) to the verdict a conversion pass
//! reached, so a repeat pass over the same input can skip re-running. The
//! invariant that keeps it honest: a verdict is committed only after a
//! clean run completed — see [`super::run_pass`]. The "HELD: build error"
//! verdict existed because a pass memoized what it saw while a human was
//! resetting the tree underneath it. [`VerdictMemo::invalidate`] is the
//! cheap way to throw a suspect entry out, with a logged line so the
//! operator can see what was dropped.

use std::collections::BTreeMap;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// The verdict a conversion pass reaches about its target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Verdict {
    /// The converted target validated clean.
    Clean,
    /// The pass ran to completion and the target failed; `reason` is the
    /// one-line summary of why.
    Failed { reason: String },
}

impl Verdict {
    /// True when the pass cleared its target.
    pub fn is_clean(&self) -> bool {
        matches!(self, Self::Clean)
    }
}

/// The key of a durable verdict: the patch under conversion and the base
/// commit it applies to.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct MemoKey {
    /// Hash of the patch under conversion.
    pub patch_hash: String,
    /// SHA of the base commit the patch applies to.
    pub base_sha: String,
}

impl MemoKey {
    /// Build a key from the patch hash and base sha.
    pub fn new(patch_hash: impl Into<String>, base_sha: impl Into<String>) -> Self {
        Self {
            patch_hash: patch_hash.into(),
            base_sha: base_sha.into(),
        }
    }

    /// Stable single-string form used as the memo's map key.
    pub fn id(&self) -> String {
        format!("{}:{}", self.patch_hash, self.base_sha)
    }
}

/// One memo entry: the verdict and when it was recorded.
#[derive(Debug, Serialize, Deserialize)]
struct StoredVerdict {
    verdict: Verdict,
    recorded_at_epoch_secs: u64,
}

/// Errors from opening, committing, or invalidating memo state.
#[derive(Debug)]
pub enum MemoError {
    /// A filesystem failure while reading, writing, or moving the memo.
    Io { path: PathBuf, reason: String },
    /// The memo file exists but is not a memo; overwriting it would lose
    /// state, so the open fails instead.
    Corrupt { path: PathBuf, reason: String },
}

impl std::fmt::Display for MemoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io { path, reason } => {
                write!(f, "memo I/O at {} failed: {reason}", path.display())
            }
            Self::Corrupt { path, reason } => {
                write!(
                    f,
                    "memo at {} is not a memo and will not be overwritten: {reason}",
                    path.display()
                )
            }
        }
    }
}

impl std::error::Error for MemoError {}

/// A file-backed memo of conversion-pass verdicts.
#[derive(Debug)]
pub struct VerdictMemo {
    path: PathBuf,
    entries: BTreeMap<String, StoredVerdict>,
}

impl VerdictMemo {
    /// Open the memo at `path`, starting an empty one when the file is
    /// absent.
    pub fn open(path: &Path) -> Result<VerdictMemo, MemoError> {
        let path = path.to_path_buf();
        match fs::read(&path) {
            Ok(body) => {
                let entries: BTreeMap<String, StoredVerdict> = serde_json::from_slice(&body)
                    .map_err(|error| MemoError::Corrupt {
                        path: path.clone(),
                        reason: error.to_string(),
                    })?;
                Ok(Self { path, entries })
            }
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(Self {
                path,
                entries: BTreeMap::new(),
            }),
            Err(error) => Err(MemoError::Io {
                path,
                reason: error.to_string(),
            }),
        }
    }

    /// Where the memo is durably stored.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The cached verdict for `key`, if any.
    pub fn get(&self, key: &MemoKey) -> Option<&Verdict> {
        self.entries.get(&key.id()).map(|stored| &stored.verdict)
    }

    /// Commit a verdict durably (atomic tmp-file plus rename).
    pub fn record(&mut self, key: &MemoKey, verdict: &Verdict) -> Result<(), MemoError> {
        self.entries.insert(
            key.id(),
            StoredVerdict {
                verdict: verdict.clone(),
                recorded_at_epoch_secs: now_epoch_secs(),
            },
        );
        self.save()
    }

    /// Cheaply invalidate one cached verdict (acceptance criterion 4).
    ///
    /// Drops the key, persists the drop, and writes one line to `log` so
    /// the operator can see what was thrown out. Returns `true` when a
    /// verdict was cached and dropped, `false` when the key was absent.
    /// The library crate never prints on its own; the caller supplies the
    /// sink (a stderr writer, a journal, ...).
    pub fn invalidate(
        &mut self,
        key: &MemoKey,
        log: &mut dyn FnMut(&str),
    ) -> Result<bool, MemoError> {
        if self.entries.remove(&key.id()).is_none() {
            return Ok(false);
        }
        self.save()?;
        log(&format!(
            "convert-pass memo: invalidated cached verdict for {} in {}",
            key.id(),
            self.path.display()
        ));
        Ok(true)
    }

    /// Persist the whole memo atomically.
    fn save(&self) -> Result<(), MemoError> {
        let body = serde_json::to_vec_pretty(&self.entries).map_err(|error| MemoError::Io {
            path: self.path.clone(),
            reason: error.to_string(),
        })?;
        if let Some(parent) = self.path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent).map_err(|error| MemoError::Io {
                    path: self.path.clone(),
                    reason: error.to_string(),
                })?;
            }
        }
        let file_name = self
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("memo")
            .to_string();
        let tmp = self
            .path
            .with_file_name(format!("{file_name}.tmp-{}", std::process::id()));
        fs::write(&tmp, &body).map_err(|error| MemoError::Io {
            path: tmp.clone(),
            reason: error.to_string(),
        })?;
        fs::rename(&tmp, &self.path).map_err(|error| MemoError::Io {
            path: tmp,
            reason: error.to_string(),
        })?;
        Ok(())
    }
}

fn now_epoch_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}
