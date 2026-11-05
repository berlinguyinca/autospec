//! Ship an archive, not a tree (issue #4577).
//!
//! The agent base copy walked ~23,003 small files — mostly loose git objects —
//! off shared storage whose per-file round-trip is ~22 ms (46 files/s), while
//! its sequential throughput is 859 MB/s. At 46 files/s a single reader spends
//! ≈500 s on one base before any contention, and five agents were observed
//! doing it at once, all stalled. The storage is not degraded: it has a
//! different cost model, and a startup path that fans out into tens of
//! thousands of per-file operations on it is misdesigned regardless of total
//! bytes (the tree is only ~100 MB).
//!
//! The fix (invariant 2): collapse the tree an agent must fetch into ONE file
//! and copy that. A base refresh publishes an immutable generation (see
//! [`crate::immutable_base`]); this module turns a published generation into a
//! single [`ARCHIVE_FILE`] beside the generation's files and records it in
//! [`ARCHIVE_RECORD`]. An agent then fetches that one file with a single
//! sequential read ([`fetch_archive`]) and verifies it against the sha256 the
//! writer recorded ([`verify_archive`]) instead of walking the tree.
//!
//! `.git` is never shipped to the archive (invariant 3): loose objects
//! dominate the file count, and an agent that only needs a working tree plus
//! commit ability gets commit ability from its own shallow clone / fresh
//! `git init`, not from a 23,000-object copy. [`build_archive`] fails closed if
//! a `.git` member ever slips into the archive.
//!
//! The measured cost model that motivated this is recorded here (invariant 4)
//! so the next designer sees the number instead of re-concluding "the
//! filesystem is slow":
//!
//! | workload | result |
//! |---|---|
//! | Quobyte sequential (2 GB, `iflag=direct`) | 859 MB/s |
//! | Quobyte many small files (2,000 loose objects) | 46 files/s |
//! | node-local /scratch, identical pattern | 926 files/s |
//!
//! Sequential and per-file are the same storage read in different shapes;
//! hot startup paths must be designed against the per-file cost, not the
//! byte count.

use std::fmt;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};

/// The single-file artifact a refresh publishes beside the generation.
pub const ARCHIVE_FILE: &str = "base-archive.tar";
/// The record that names the artifact and carries its integrity metadata.
pub const ARCHIVE_RECORD: &str = "BASE_ARCHIVE";
/// The `tar` binary. Argument vectors only: every flag is a fixed constant in
/// this module, so no caller-controlled text reaches a shell.
const TAR_BIN: &str = "tar";

/// Top-level entries excluded from the archive. `.git` first: loose objects
/// dominate the file count and are never shipped (invariant 3). The build and
/// vendor dirs are dead weight an agent never reads. The archive and its
/// record are excluded so `tar` never reads the very file it is writing.
const EXCLUDED_ENTRIES: &[&str] = &[
    ".git",
    "target",
    "dist",
    "build",
    "node_modules",
    ".venv",
    "venv",
    "__pycache__",
    ARCHIVE_FILE,
    ARCHIVE_RECORD,
];

/// The single-file artifact and the metadata that lets a reader trust it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveInfo {
    /// The archive file name, relative to the generation root.
    pub file: String,
    /// Number of regular files packed in the archive (the working tree).
    pub files: u64,
    /// The archive's size in bytes.
    pub bytes: u64,
    /// sha256 of the archive bytes. Per-build integrity, not a build
    /// fingerprint: `tar` embeds mtimes, so the same tree hashes differently on
    /// each build. A reader verifies a fetch against the record the writer
    /// wrote for that build, never against a remembered value.
    pub sha256: String,
}

/// Why an archive operation failed. Fail closed: an unreadable or corrupt
/// archive is an error, never "fall back to the tree."
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArchiveError {
    /// The generation path is not a directory.
    NoGeneration { path: PathBuf },
    /// The generation has no [`ARCHIVE_RECORD`] record.
    NoArchive { path: PathBuf },
    /// The [`ARCHIVE_RECORD`] record is present but malformed.
    BadRecord { path: PathBuf, reason: String },
    /// The archive holds no regular files — a base that packs to nothing is a
    /// defect, not an empty base.
    EmptyArchive { path: PathBuf },
    /// A `.git` member is present in the archive (invariant 3 violated).
    ContainsGit { member: String },
    /// `tar` failed; the status and stderr are recorded for the caller.
    Tar { status: String, stderr: String },
    /// A filesystem operation failed.
    Io {
        operation: String,
        path: PathBuf,
        source: String,
    },
    /// A fetched archive does not hash to the recorded sha256.
    Corrupt { expected: String, actual: String },
}

impl fmt::Display for ArchiveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ArchiveError::NoGeneration { path } => {
                write!(f, "not a generation directory: {}", path.display())
            }
            ArchiveError::NoArchive { path } => {
                write!(f, "no {ARCHIVE_RECORD} record in {}", path.display())
            }
            ArchiveError::BadRecord { path, reason } => {
                write!(
                    f,
                    "bad {ARCHIVE_RECORD} record in {}: {}",
                    path.display(),
                    reason
                )
            }
            ArchiveError::EmptyArchive { path } => {
                write!(f, "archive of {} packs no regular files", path.display())
            }
            ArchiveError::ContainsGit { member } => {
                write!(
                    f,
                    "archive ships a .git member ({member}); .git is never shipped"
                )
            }
            ArchiveError::Tar { status, stderr } => {
                write!(f, "tar failed ({status}): {}", stderr.trim())
            }
            ArchiveError::Io {
                operation,
                path,
                source,
            } => write!(f, "{operation} on {} failed: {}", path.display(), source),
            ArchiveError::Corrupt { expected, actual } => {
                write!(
                    f,
                    "archive corrupt: sha256 {actual} does not match the recorded {expected}"
                )
            }
        }
    }
}

impl std::error::Error for ArchiveError {}

fn io_err(operation: &str, path: &Path, source: &std::io::Error) -> ArchiveError {
    ArchiveError::Io {
        operation: operation.to_owned(),
        path: path.to_path_buf(),
        source: source.to_string(),
    }
}

/// Stream `path` into a sha256 and return the lowercase hex digest.
fn sha256_file(path: &Path) -> Result<String, ArchiveError> {
    let mut file = fs::File::open(path).map_err(|e| io_err("open", path, &e))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = file.read(&mut buf).map_err(|e| io_err("read", path, &e))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let mut out = String::with_capacity(64);
    for b in hasher.finalize() {
        out.push_str(&format!("{b:02x}"));
    }
    Ok(out)
}

/// Run `tar <args>` and return stdout, failing closed on a non-zero status.
/// `context` names the path in any error so the caller can locate the failure.
fn run_tar(args: &[String], context: &Path) -> Result<String, ArchiveError> {
    let output = Command::new(TAR_BIN)
        .args(args)
        .output()
        .map_err(|e| io_err("spawn tar", context, &e))?;
    if !output.status.success() {
        return Err(ArchiveError::Tar {
            status: output.status.to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn exclude_args() -> Vec<String> {
    EXCLUDED_ENTRIES
        .iter()
        .map(|entry| format!("--exclude=./{entry}"))
        .collect()
}

/// Recursively count the regular files under `dir`.
fn count_regular_files(dir: &Path) -> Result<u64, ArchiveError> {
    fn walk(dir: &Path, count: &mut u64) -> Result<(), ArchiveError> {
        let entries = fs::read_dir(dir).map_err(|e| io_err("read_dir", dir, &e))?;
        for entry in entries {
            let entry = entry.map_err(|e| io_err("read_dir entry", dir, &e))?;
            let path = entry.path();
            if path.is_dir() {
                walk(&path, count)?;
            } else if path.is_file() {
                *count += 1;
            }
        }
        Ok(())
    }
    let mut count = 0u64;
    walk(dir, &mut count)?;
    Ok(count)
}

/// Turn a published generation into a single-file archive and record it.
///
/// `generation` is a directory — the thing
/// [`crate::immutable_base::publish_generation`] publishes. This writes
/// [`ARCHIVE_FILE`] beside the generation's files, excluding `.git` and the
/// build/vendor dirs, then writes [`ARCHIVE_RECORD`] carrying the regular-file
/// count, byte size, and sha256. After it returns, an agent can fetch the whole
/// working tree with one sequential read of one file instead of walking the
/// tree.
pub fn build_archive(generation: &Path) -> Result<ArchiveInfo, ArchiveError> {
    if !generation.is_dir() {
        return Err(ArchiveError::NoGeneration {
            path: generation.to_path_buf(),
        });
    }
    let archive = generation.join(ARCHIVE_FILE);
    let archive_str = archive.to_string_lossy().into_owned();

    // `tar -cf <archive> --exclude=./<each> -C <generation> .`
    let mut args: Vec<String> = vec!["-cf".to_owned(), archive_str.clone()];
    args.extend(exclude_args());
    args.push("-C".to_owned());
    args.push(generation.to_string_lossy().into_owned());
    args.push(".".to_owned());
    run_tar(&args, generation)?;

    // Inspect the produced archive: it must hold regular files and no `.git`.
    let listing = run_tar(&["-tf".to_owned(), archive_str.clone()], &archive)?;
    let mut files = 0u64;
    for line in listing.lines() {
        let name = line.trim();
        if name.is_empty() || name.ends_with('/') {
            continue; // skip blanks and directory entries
        }
        let rel = name.strip_prefix("./").unwrap_or(name);
        if rel == ".git" || rel.starts_with(".git/") {
            return Err(ArchiveError::ContainsGit {
                member: name.to_owned(),
            });
        }
        files += 1;
    }
    if files == 0 {
        let _ = fs::remove_file(&archive);
        return Err(ArchiveError::EmptyArchive {
            path: generation.to_path_buf(),
        });
    }

    let bytes = fs::metadata(&archive)
        .map_err(|e| io_err("metadata", &archive, &e))?
        .len();
    let sha256 = sha256_file(&archive)?;

    let record = generation.join(ARCHIVE_RECORD);
    let text = format!("file: {ARCHIVE_FILE}\nfiles: {files}\nbytes: {bytes}\nsha256: {sha256}\n");
    fs::write(&record, text).map_err(|e| io_err("write", &record, &e))?;

    Ok(ArchiveInfo {
        file: ARCHIVE_FILE.to_owned(),
        files,
        bytes,
        sha256,
    })
}

/// Read back the [`ARCHIVE_RECORD`] a [`build_archive`] wrote.
pub fn read_archive(generation: &Path) -> Result<ArchiveInfo, ArchiveError> {
    let record = generation.join(ARCHIVE_RECORD);
    if !record.is_file() {
        return Err(ArchiveError::NoArchive {
            path: generation.to_path_buf(),
        });
    }
    let text = fs::read_to_string(&record).map_err(|e| io_err("read", &record, &e))?;
    let mut file: Option<String> = None;
    let mut files: Option<u64> = None;
    let mut bytes: Option<u64> = None;
    let mut sha256: Option<String> = None;
    for line in text.lines() {
        let (key, value) = match line.split_once(':') {
            Some((k, v)) => (k.trim(), v.trim()),
            None => continue,
        };
        match key {
            "file" => file = Some(value.to_owned()),
            "files" => {
                files = Some(value.parse().map_err(|_| ArchiveError::BadRecord {
                    path: record.clone(),
                    reason: format!("files is not an integer: {value:?}"),
                })?)
            }
            "bytes" => {
                bytes = Some(value.parse().map_err(|_| ArchiveError::BadRecord {
                    path: record.clone(),
                    reason: format!("bytes is not an integer: {value:?}"),
                })?)
            }
            "sha256" => sha256 = Some(value.to_owned()),
            _ => {}
        }
    }
    let (file, files, bytes, sha256) = match (file, files, bytes, sha256) {
        (Some(file), Some(files), Some(bytes), Some(sha256)) => (file, files, bytes, sha256),
        _ => {
            return Err(ArchiveError::BadRecord {
                path: record,
                reason: "record is missing one of file/files/bytes/sha256".to_owned(),
            })
        }
    };
    Ok(ArchiveInfo {
        file,
        files,
        bytes,
        sha256,
    })
}

/// The agent side: fetch the single archive file into `dest_dir` with one copy.
///
/// This is the whole fix. Where the old path did ~23,000 per-file reads off
/// shared storage, this does one `fs::copy` of one file — a single sequential
/// read. It does not walk the generation tree. Returns the fetched archive's
/// path.
pub fn fetch_archive(generation: &Path, dest_dir: &Path) -> Result<PathBuf, ArchiveError> {
    let info = read_archive(generation)?;
    let src = generation.join(&info.file);
    if !src.is_file() {
        return Err(ArchiveError::NoArchive {
            path: generation.to_path_buf(),
        });
    }
    fs::create_dir_all(dest_dir).map_err(|e| io_err("create_dir_all", dest_dir, &e))?;
    let dest = dest_dir.join(&info.file);
    fs::copy(&src, &dest).map_err(|e| io_err("copy", &src, &e))?;
    Ok(dest)
}

/// Verify a fetched archive against the recorded sha256. Fail closed: a
/// mismatch is [`ArchiveError::Corrupt`], never silently accepted.
pub fn verify_archive(info: &ArchiveInfo, fetched: &Path) -> Result<(), ArchiveError> {
    let actual = sha256_file(fetched)?;
    if actual != info.sha256 {
        return Err(ArchiveError::Corrupt {
            expected: info.sha256.clone(),
            actual,
        });
    }
    Ok(())
}

/// Extract the single archive into `dest_dir` and return how many regular files
/// it produced. An agent that needs the tree (not just commit ability) does
/// this once, locally, on the one file it fetched.
pub fn extract_archive(archive: &Path, dest_dir: &Path) -> Result<u64, ArchiveError> {
    if !archive.is_file() {
        return Err(ArchiveError::NoArchive {
            path: archive.to_path_buf(),
        });
    }
    fs::create_dir_all(dest_dir).map_err(|e| io_err("create_dir_all", dest_dir, &e))?;
    let args = vec![
        "-xf".to_owned(),
        archive.to_string_lossy().into_owned(),
        "-C".to_owned(),
        dest_dir.to_string_lossy().into_owned(),
    ];
    run_tar(&args, dest_dir)?;
    count_regular_files(dest_dir)
}
