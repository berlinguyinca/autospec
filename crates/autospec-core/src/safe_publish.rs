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

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Child, Command};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_dir(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("autospec-safe-publish-{name}-{nonce}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn cleanup(dir: &Path) {
        let _ = fs::remove_dir_all(dir);
    }

    /// Spawn a process that holds `path` open for the test's duration:
    /// `tail -f` opens the file and keeps the descriptor open while
    /// following it, mirroring a shell streaming a long script.
    fn spawn_holder(path: &Path) -> Child {
        Command::new("tail").arg("-f").arg(path).spawn().unwrap()
    }

    fn reap(mut child: Child) {
        child.kill().unwrap();
        child.wait().unwrap();
    }

    #[test]
    fn next_versioned_path_appends_two_to_a_fresh_name() {
        assert_eq!(
            next_versioned_path(Path::new("/x/iw-issue.sh")),
            PathBuf::from("/x/iw-issue-2.sh")
        );
        assert_eq!(
            next_versioned_path(Path::new("foo")),
            PathBuf::from("foo-2")
        );
    }

    #[test]
    fn next_versioned_path_skips_existing_versions() {
        let dir = test_dir("next-version");
        fs::write(dir.join("foo-2.sh"), "x").unwrap();
        assert_eq!(
            next_versioned_path(&dir.join("foo.sh")),
            dir.join("foo-3.sh")
        );
        cleanup(&dir);
    }

    #[test]
    fn next_versioned_path_continues_from_a_versioned_name() {
        let dir = test_dir("next-versioned");
        assert_eq!(
            next_versioned_path(&dir.join("foo-2.sh")),
            dir.join("foo-3.sh")
        );
        cleanup(&dir);
    }

    #[test]
    fn open_file_holders_is_none_for_a_missing_file() {
        let dir = test_dir("missing");
        assert_eq!(
            open_file_holders(&dir.join("absent.sh")),
            OpenersStatus::None
        );
        cleanup(&dir);
    }

    #[test]
    fn open_file_holders_is_none_for_a_free_file() {
        let dir = test_dir("free");
        let file = dir.join("script.sh");
        fs::write(&file, "echo one\n").unwrap();
        assert_eq!(open_file_holders(&file), OpenersStatus::None);
        cleanup(&dir);
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn detects_a_live_reader_and_clears_after_exit() {
        let dir = test_dir("holders");
        let file = dir.join("script.sh");
        fs::write(&file, "echo one\n").unwrap();

        let child = spawn_holder(&file);
        let status = open_file_holders(&file);
        match status {
            OpenersStatus::Holders(holders) => {
                assert!(
                    holders.iter().any(|h| h.pid == child.id()),
                    "holder list {holders:?} must include the spawned reader"
                );
            }
            other => panic!("expected holders, got {other:?}"),
        }

        reap(child);
        assert_eq!(open_file_holders(&file), OpenersStatus::None);
        cleanup(&dir);
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn overwrite_refuses_while_a_reader_holds_the_file() {
        let dir = test_dir("refused");
        let file = dir.join("iw-issue.sh");
        fs::write(&file, "old content\n").unwrap();

        let child = spawn_holder(&file);
        match publish_overwrite(&file, b"new content\n") {
            Err(SafePublishError::Refused { path, holders }) => {
                assert_eq!(path, file);
                assert!(!holders.is_empty());
            }
            other => panic!("expected Refused, got {other:?}"),
        }
        // Original bytes are intact and no temp file was left behind.
        assert_eq!(fs::read_to_string(&file).unwrap(), "old content\n");
        let leftovers: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n != "iw-issue.sh")
            .collect();
        assert!(leftovers.is_empty(), "temp residue: {leftovers:?}");

        reap(child);
        cleanup(&dir);
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn overwrite_of_a_free_file_succeeds_and_records_previous() {
        let dir = test_dir("overwrite-free");
        let file = dir.join("foo.sh");
        fs::write(&file, "old\n").unwrap();

        let receipt = publish_overwrite(&file, b"new\n").unwrap();
        assert_eq!(receipt.published_to, file);
        assert_eq!(receipt.previous.as_deref(), Some(file.as_path()));
        assert_eq!(fs::read_to_string(&file).unwrap(), "new\n");
        cleanup(&dir);
    }

    #[test]
    fn overwrite_of_a_missing_file_is_a_plain_create() {
        let dir = test_dir("overwrite-missing");
        let file = dir.join("foo.sh");

        let receipt = publish_overwrite(&file, b"new\n").unwrap();
        assert_eq!(receipt.published_to, file);
        assert_eq!(receipt.previous, None);
        assert_eq!(fs::read_to_string(&file).unwrap(), "new\n");
        cleanup(&dir);
    }

    #[test]
    #[cfg(not(target_os = "linux"))]
    fn overwrite_fails_closed_when_detection_is_unavailable() {
        let dir = test_dir("overwrite-unsupported");
        let file = dir.join("foo.sh");
        fs::write(&file, "old\n").unwrap();

        match publish_overwrite(&file, b"new\n") {
            Err(SafePublishError::Indeterminate { path, reason }) => {
                assert_eq!(path, file);
                assert!(!reason.is_empty());
            }
            other => panic!("expected Indeterminate, got {other:?}"),
        }
        cleanup(&dir);
    }

    #[test]
    fn versioned_publish_of_a_new_file_uses_the_canonical_name() {
        let dir = test_dir("versioned-new");
        let file = dir.join("foo.sh");

        let receipt = publish_versioned(&file, b"v1\n").unwrap();
        assert_eq!(receipt.published_to, file);
        assert_eq!(receipt.previous, None);
        assert_eq!(fs::read_to_string(&file).unwrap(), "v1\n");
        cleanup(&dir);
    }

    #[test]
    fn versioned_publish_increments_across_calls() {
        let dir = test_dir("versioned-increment");
        let file = dir.join("foo.sh");
        fs::write(&file, "v1\n").unwrap();

        let first = publish_versioned(&file, b"v2\n").unwrap();
        let second = publish_versioned(&file, b"v3\n").unwrap();
        assert_eq!(first.published_to, dir.join("foo-2.sh"));
        assert_eq!(second.published_to, dir.join("foo-3.sh"));
        assert_eq!(fs::read_to_string(&dir.join("foo-2.sh")).unwrap(), "v2\n");
        assert_eq!(fs::read_to_string(&dir.join("foo-3.sh")).unwrap(), "v3\n");
        cleanup(&dir);
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn versioned_publish_leaves_the_running_file_untouched() {
        let dir = test_dir("versioned-held");
        let file = dir.join("iw-issue.sh");
        fs::write(&file, "v1\n").unwrap();

        let child = spawn_holder(&file);
        let receipt = publish_versioned(&file, b"v2\n").unwrap();
        assert_eq!(receipt.published_to, dir.join("iw-issue-2.sh"));
        assert_eq!(receipt.previous.as_deref(), Some(file.as_path()));
        assert_eq!(fs::read_to_string(&receipt.published_to).unwrap(), "v2\n");
        // The file the reader is streaming is byte-identical.
        assert_eq!(fs::read_to_string(&file).unwrap(), "v1\n");

        reap(child);
        cleanup(&dir);
    }

    #[test]
    fn publish_errors_carry_the_path_and_are_displayable() {
        let err = SafePublishError::Refused {
            path: PathBuf::from("/x/iw-issue.sh"),
            holders: vec![OpenFileHolder { pid: 4242, fd: 3 }],
        };
        let message = err.to_string();
        assert!(message.contains("/x/iw-issue.sh"), "{message}");
        assert!(message.contains("1"), "{message}");

        // A missing target directory is a plain Io error, not a panic.
        let missing = test_dir("io-error");
        let nested = missing.join("no").join("such").join("dir.sh");
        match publish_overwrite(&nested, b"x") {
            Err(SafePublishError::Io {
                operation,
                path,
                source: _,
            }) => {
                // The failure is reported at the temp file, which lives in the
                // target's (missing) directory.
                assert_eq!(path.parent(), nested.parent());
                assert!(!operation.is_empty());
            }
            other => panic!("expected Io error, got {other:?}"),
        }
        cleanup(&missing);
    }
}
