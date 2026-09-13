//! Guarded publishing for files a live reader may be streaming.
//!
//! Corrected rule (issue #3689): atomic replacement — write a temporary file,
//! then rename it over the target — is atomic with respect to *opening* a file,
//! not with respect to *reading* one. A reader that already holds the file open
//! keeps reading the old inode after the rename, and on a network filesystem
//! (Quobyte, NFS) that orphaned inode is torn down, so reads fail mid-stream
//! ("Stale file handle"; for a shell streaming a long script, "error reading
//! input file"). Therefore a file must never be replaced while a live process
//! holds it open: check first, then either refuse the edit
//! ([`publish_overwrite`] refuses) or defer it to a versioned path
//! ([`publish_versioned`]) that no running job references yet.
//!
//! Holder detection reads the Linux `/proc/<pid>/fd` table. On platforms
//! without `/proc` the check is indeterminate and [`publish_overwrite`] fails
//! closed for existing targets: when the check cannot run, the versioned path
//! is the safe alternative.

use std::fmt;
use std::fs;
use std::io::Write;
// Gated to match the only code that uses it: both `.dev()`/`.ino()` call
// sites live inside `#[cfg(target_os = "linux")]` blocks, and `std::os::unix`
// does not exist on Windows, so an unconditional import failed to resolve
// (E0433) in a job that has been red long enough for nobody to read it.
#[cfg(target_os = "linux")]
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// A process currently holding the target file open. Detection is by
/// device + inode, so hard links to the file are covered as well.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenFileHolder {
    /// PID of the holding process.
    pub pid: u32,
    /// File descriptor the process holds open on the file.
    pub fd: u32,
}

/// Outcome of a "who is reading this right now" check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenersStatus {
    /// The file does not exist, or no live process holds it open.
    None,
    /// Live processes holding the file open, one entry per (pid, fd) pair.
    Holders(Vec<OpenFileHolder>),
    /// The check could not be performed (e.g. no `/proc` on this platform).
    /// Callers that refuse to guess must treat this as "possibly held".
    Indeterminate(String),
}

/// Receipt for a successful publish.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishReceipt {
    /// The path that now carries the new bytes.
    pub published_to: PathBuf,
    /// The file that carried the previous bytes: for an overwrite this is the
    /// same path; for a versioned publish it is the canonical path, still live
    /// for the running reader. `None` when the file is new.
    pub previous: Option<PathBuf>,
}

/// Why a guarded publish was refused or could not complete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SafePublishError {
    /// The target exists and a live process holds it open. Renaming over it
    /// would orphan the inode the reader is streaming.
    Refused {
        path: PathBuf,
        holders: Vec<OpenFileHolder>,
    },
    /// The holder check could not be performed; the publish is refused
    /// fail-closed rather than guessing.
    Indeterminate { path: PathBuf, reason: String },
    /// A filesystem operation failed.
    Io {
        operation: String,
        path: PathBuf,
        source: String,
    },
}

impl fmt::Display for SafePublishError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused { path, holders } => write!(
                f,
                "refusing to replace {}: held open by {} live process(es); publish a versioned path or wait for the readers",
                path.display(),
                holders.len()
            ),
            Self::Indeterminate { path, reason } => write!(
                f,
                "cannot verify {}: {reason}; refusing fail-closed, publish a versioned path",
                path.display()
            ),
            Self::Io {
                operation,
                path,
                source,
            } => write!(f, "{operation} failed at {}: {source}", path.display()),
        }
    }
}

impl std::error::Error for SafePublishError {}

/// List the live processes holding `path` open right now.
///
/// On Linux this stats the target, then scans `/proc/<pid>/fd` for
/// descriptors pointing at the same (device, inode). The scan only *reads*
/// `/proc`; it never opens, signals, or otherwise disturbs any process.
pub fn open_file_holders(path: &Path) -> OpenersStatus {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return OpenersStatus::None,
        Err(error) => return OpenersStatus::Indeterminate(format!("stat failed: {error}")),
    };
    #[cfg(target_os = "linux")]
    {
        scan_proc_for_holders(metadata.dev(), metadata.ino())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = metadata;
        OpenersStatus::Indeterminate(
            "open-file detection requires a Linux /proc table; refusing to guess".to_owned(),
        )
    }
}

/// Next free versioned sibling of `path`: version 1 is the unversioned name
/// itself, so `foo.sh` yields `foo-2.sh`, skipping versions that already
/// exist; a versioned input continues the sequence (`foo-2.sh` yields
/// `foo-3.sh`).
pub fn next_versioned_path(path: &Path) -> PathBuf {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    let ext = path.extension().and_then(|e| e.to_str());
    // A versioned input continues its own sequence (foo-2 -> foo-3): the
    // numeric suffix is replaced, not appended to.
    let (base, mut version) = match stem_version(stem) {
        Some(current) => {
            let dash = stem.rfind('-').unwrap_or(0);
            (stem[..dash].to_owned(), current + 1)
        }
        None => (stem.to_owned(), 2),
    };
    loop {
        let candidate = match ext {
            Some(ext) => format!("{base}-{version}.{ext}"),
            None => format!("{base}-{version}"),
        };
        let candidate_path = path.with_file_name(candidate);
        if !candidate_path.exists() {
            return candidate_path;
        }
        if version == u64::MAX {
            panic!("exhausted versioned names for {}", path.display());
        }
        version += 1;
    }
}

/// Refuse-or-replace publish: atomically rename new bytes over `path`, but
/// only if no live process holds the file open — checked before writing the
/// temp file and again immediately before the rename. Errors (rather than
/// renaming) when a holder is found or the check is indeterminate.
pub fn publish_overwrite(path: &Path, bytes: &[u8]) -> Result<PublishReceipt, SafePublishError> {
    let previous = path.metadata().ok().map(|_| path.to_path_buf());
    reject_if_held(path, &open_file_holders(path))?;
    let temp = temp_path_for(path);
    write_temp_bytes(&temp, bytes)?;
    // Re-check immediately before the rename: a reader that opened the file
    // after the first check is still caught here. The residual window is
    // sub-millisecond; only the versioned path can never disturb a reader.
    reject_if_held(path, &open_file_holders(path))?;
    rename_temp(&temp, path)?;
    Ok(PublishReceipt {
        published_to: path.to_path_buf(),
        previous,
    })
}

/// Versioned publish: write the new bytes to the next free versioned sibling
/// of `path` and leave the running file byte-identical, so a reader already
/// streaming it is never disturbed. A missing `path` is published under its
/// own (unversioned) name — that is version 1, and no reader can be running.
pub fn publish_versioned(path: &Path, bytes: &[u8]) -> Result<PublishReceipt, SafePublishError> {
    if path.metadata().is_err() {
        let temp = temp_path_for(path);
        write_temp_bytes(&temp, bytes)?;
        rename_temp(&temp, path)?;
        return Ok(PublishReceipt {
            published_to: path.to_path_buf(),
            previous: None,
        });
    }
    let target = next_versioned_path(path);
    let temp = temp_path_for(&target);
    write_temp_bytes(&temp, bytes)?;
    rename_temp(&temp, &target)?;
    Ok(PublishReceipt {
        published_to: target,
        previous: Some(path.to_path_buf()),
    })
}

#[cfg(target_os = "linux")]
fn scan_proc_for_holders(target_dev: u64, target_ino: u64) -> OpenersStatus {
    let proc = match fs::read_dir("/proc") {
        Ok(proc) => proc,
        Err(error) => return OpenersStatus::Indeterminate(format!("/proc unreadable: {error}")),
    };
    let mut holders = Vec::new();
    for entry in proc.flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        holders.extend(holders_in_pid(pid, target_dev, target_ino));
    }
    if holders.is_empty() {
        OpenersStatus::None
    } else {
        OpenersStatus::Holders(holders)
    }
}

#[cfg(target_os = "linux")]
fn holders_in_pid(pid: u32, target_dev: u64, target_ino: u64) -> Vec<OpenFileHolder> {
    let mut holders = Vec::new();
    // A process can vanish or its fd table stop being readable mid-scan;
    // such pids simply do not appear. The publish paths re-check immediately
    // before the rename, so a vanishing holder is not a safety gap.
    let Ok(fds) = fs::read_dir(format!("/proc/{pid}/fd")) else {
        return holders;
    };
    for fd_entry in fds.flatten() {
        let Ok(fd) = fd_entry.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        // Following the fd link reaches the actual object even when it is an
        // orphaned inode; a closed fd or a failed stat resolves to no match.
        let Ok(metadata) = fs::metadata(fd_entry.path()) else {
            continue;
        };
        if (metadata.dev(), metadata.ino()) == (target_dev, target_ino) {
            holders.push(OpenFileHolder { pid, fd });
        }
    }
    holders
}

fn reject_if_held(path: &Path, status: &OpenersStatus) -> Result<(), SafePublishError> {
    match status {
        OpenersStatus::None => Ok(()),
        OpenersStatus::Holders(holders) => Err(SafePublishError::Refused {
            path: path.to_path_buf(),
            holders: holders.clone(),
        }),
        OpenersStatus::Indeterminate(reason) => Err(SafePublishError::Indeterminate {
            path: path.to_path_buf(),
            reason: reason.clone(),
        }),
    }
}

fn stem_version(stem: &str) -> Option<u64> {
    let dash = stem.rfind('-')?;
    stem[dash + 1..].parse::<u64>().ok()
}

static TEMP_NONCE: AtomicU64 = AtomicU64::new(0);

/// Temp file for `path` in the same directory (rename must stay on one
/// filesystem), named after the target plus a per-process nonce so
/// concurrent publishers cannot clobber each other's temp files.
fn temp_path_for(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let nonce = TEMP_NONCE.fetch_add(1, Ordering::Relaxed);
    parent.join(format!(
        ".{name}.safe-publish-{}-{nonce}",
        std::process::id()
    ))
}

fn write_temp_bytes(path: &Path, bytes: &[u8]) -> Result<(), SafePublishError> {
    let mut file = fs::File::create(path).map_err(|error| io_err("create", path, error))?;
    file.write_all(bytes)
        .map_err(|error| io_err("write", path, error))?;
    file.sync_all()
        .map_err(|error| io_err("sync_all", path, error))?;
    Ok(())
}

fn rename_temp(temp: &Path, target: &Path) -> Result<(), SafePublishError> {
    fs::rename(temp, target).map_err(|error| {
        let _ = fs::remove_file(temp);
        io_err("rename", target, error)
    })
}

fn io_err(operation: &str, path: &Path, error: std::io::Error) -> SafePublishError {
    SafePublishError::Io {
        operation: operation.to_owned(),
        path: path.to_path_buf(),
        source: error.to_string(),
    }
}
