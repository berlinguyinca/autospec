//! Exclusive worktree lock, verdict attribution, and job-supersession
//! primitives (issue #3608).
//!
//! Two conversion runs that share one git worktree fabricate each other's
//! test failures: a patch that actually passes can report "81 failed"
//! because the other run rewrote the same checkout underneath it. The
//! lock makes that impossible and the verdict makes a contaminated result
//! detectable after the fact.
//!
//! The rules every tool that mutates a git checkout must follow:
//!
//! 1. Take an **exclusive** `flock` on a lock file **beside** the worktree
//!    (never inside it, so git status stays clean), and **refuse to run**
//!    with a reason instead of queueing when another process holds it —
//!    [`WorktreeLock::acquire`].
//! 2. Hold the lock for the **entire checkout-apply-test cycle**, not per
//!    command: keep the returned guard alive for the whole cycle; the lock
//!    is released only when the guard is dropped (or the process exits).
//! 3. Use **separate checkouts** for concurrent work — one per worker
//!    (see [`super::patch_pipeline::plan_workers`]); N checkouts get N
//!    independent locks.
//! 4. Record which **checkout** a verdict came from — [`Verdict`].
//! 5. When a background job is **superseded**, the replacement **stops the
//!    old job first** and announces it — [`Supersession`].

use std::fs::{self, File, OpenOptions};
use std::io::{self, Seek};
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// The identity written into the lock file by the process that holds it, so
/// a refused second invocation can say *why* it stopped (which pid, when it
/// started).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockHolder {
    /// Process id of the holder.
    pub pid: u32,
    /// Unix epoch seconds at which the lock was taken.
    pub started_at: u64,
}

/// Path of the lock file for a worktree: a dot-file **beside** the
/// worktree directory, named after it. Returns `None` when the worktree
/// path has no name or no parent (for example the filesystem root).
///
/// The file lives outside the checkout itself so it never appears in
/// `git status` and survives a `git checkout -B` resetting the tree.
pub fn lock_path(worktree: &Path) -> Option<PathBuf> {
    let name = worktree.file_name()?.to_str()?;
    if name.is_empty() {
        return None;
    }
    worktree
        .parent()
        .map(|parent| parent.join(format!(".{name}.autospec-lock")))
}

/// Why a worktree could not be locked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorktreeLockError {
    /// The worktree path cannot have a lock file beside it.
    InvalidPath { worktree: String, reason: String },
    /// The worktree directory does not exist (or is not a directory).
    MissingWorktree { worktree: String },
    /// Another process holds the exclusive lock. This is a **refusal**,
    /// not a queue: the caller must stop and say why, or use a separate
    /// checkout.
    AlreadyHeld {
        worktree: String,
        holder: Option<LockHolder>,
    },
    /// A filesystem failure while opening, locking, or inspecting the lock
    /// file.
    Io { worktree: String, source: String },
    /// The platform does not support `flock`.
    #[cfg(not(unix))]
    Unsupported,
}

impl std::fmt::Display for WorktreeLockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidPath { worktree, reason } => {
                write!(f, "worktree path is not lockable: {worktree}: {reason}")
            }
            Self::MissingWorktree { worktree } => {
                write!(
                    f,
                    "worktree does not exist or is not a directory: {worktree}"
                )
            }
            Self::AlreadyHeld { worktree, holder } => match holder {
                Some(holder) => write!(
                    f,
                    "refusing to run: {worktree} is locked by pid {} (started at epoch {}s); \
                     concurrent work needs separate checkouts, one per worker",
                    holder.pid, holder.started_at
                ),
                None => write!(
                    f,
                    "refusing to run: {worktree} is locked by another process; \
                     concurrent work needs separate checkouts, one per worker"
                ),
            },
            Self::Io { worktree, source } => {
                write!(f, "worktree lock I/O error: {worktree}: {source}")
            }
            #[cfg(not(unix))]
            Self::Unsupported => {
                write!(f, "worktree locking is not supported on this platform")
            }
        }
    }
}

impl std::error::Error for WorktreeLockError {}

/// An exclusive advisory lock on a git checkout, held on a lock file
/// beside the worktree.
///
/// The lock is acquired **non-blocking**: a second acquisition attempt
/// fails immediately with [`WorktreeLockError::AlreadyHeld`] (which names
/// the holder) rather than queueing. Callers must keep the guard alive
/// for the full checkout-apply-test cycle — the `flock` is released when
/// the guard is dropped or the process exits, so the guard itself is the
/// "hold for the whole cycle" mechanism.
#[derive(Debug)]
pub struct WorktreeLock {
    /// The lock file; the descriptor must stay open while the lock is held.
    /// Never read — its only job is to keep the fd (and so the flock)
    /// alive until the guard is dropped.
    #[allow(dead_code)]
    file: File,
    worktree: PathBuf,
    lock_file: PathBuf,
    holder: LockHolder,
}

impl WorktreeLock {
    /// Take the exclusive lock on `worktree`, refusing (with the holder's
    /// identity) if another process already holds it.
    ///
    /// # Rules enforced here
    ///
    /// - The worktree must be an existing directory (rule 3: real,
    ///   separate checkouts — one per worker).
    /// - The lock is exclusive and non-blocking (rule 1: refuse, don't
    ///   queue; the error says why the second invocation stopped).
    /// - The returned guard holds the lock until dropped (rule 2: the
    ///   whole checkout-apply-test cycle, not per command).
    pub fn acquire(worktree: &Path) -> Result<Self, WorktreeLockError> {
        let worktree_str = worktree.to_string_lossy().into_owned();
        let lock_file = lock_path(worktree).ok_or_else(|| WorktreeLockError::InvalidPath {
            worktree: worktree_str.clone(),
            reason: "the worktree must be a named directory with a parent".to_owned(),
        })?;
        if !worktree.is_dir() {
            return Err(WorktreeLockError::MissingWorktree {
                worktree: worktree_str.clone(),
            });
        }

        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true);
        #[cfg(unix)]
        options.custom_flags((nix::fcntl::OFlag::O_NOFOLLOW | nix::fcntl::OFlag::O_CLOEXEC).bits());
        let file = options
            .open(&lock_file)
            .map_err(|error| WorktreeLockError::Io {
                worktree: worktree_str.clone(),
                source: error.to_string(),
            })?;
        #[cfg(unix)]
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|error| WorktreeLockError::Io {
                worktree: worktree_str.clone(),
                source: error.to_string(),
            })?;

        acquire_flock(&file, &worktree_str, &lock_file)?;

        let holder = LockHolder {
            pid: process::id(),
            started_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_secs())
                .unwrap_or(0),
        };
        // Best-effort: a refusal still works without readable holder info
        // (it just reports "another process").
        let _ = write_holder(&file, &holder);

        Ok(Self {
            file,
            worktree: worktree.to_path_buf(),
            lock_file,
            holder,
        })
    }

    /// The worktree this guard protects.
    pub fn worktree(&self) -> &Path {
        &self.worktree
    }

    /// The lock file beside the worktree.
    pub fn lock_file(&self) -> &Path {
        &self.lock_file
    }

    /// The identity recorded for this holder.
    pub fn holder(&self) -> &LockHolder {
        &self.holder
    }
}

#[cfg(unix)]
fn acquire_flock(file: &File, worktree: &str, lock_file: &Path) -> Result<(), WorktreeLockError> {
    use std::os::fd::AsRawFd;

    // SAFETY: `flock` takes a live descriptor (owned by `file`) and the
    // valid operation constants LOCK_EX | LOCK_NB (2 | 4).
    extern "C" {
        fn flock(fd: i32, operation: i32) -> i32;
    }
    const LOCK_EX: i32 = 2;
    const LOCK_NB: i32 = 4;

    if unsafe { flock(file.as_raw_fd(), LOCK_EX | LOCK_NB) } != 0 {
        let error = io::Error::last_os_error();
        if error.kind() == io::ErrorKind::WouldBlock {
            return Err(WorktreeLockError::AlreadyHeld {
                worktree: worktree.to_owned(),
                holder: read_holder(lock_file),
            });
        }
        return Err(WorktreeLockError::Io {
            worktree: worktree.to_owned(),
            source: error.to_string(),
        });
    }
    Ok(())
}

#[cfg(not(unix))]
fn acquire_flock(
    _file: &File,
    _worktree: &str,
    _lock_file: &Path,
) -> Result<(), WorktreeLockError> {
    Err(WorktreeLockError::Unsupported)
}

fn write_holder(file: &File, holder: &LockHolder) -> io::Result<()> {
    let mut file = file.try_clone()?;
    file.set_len(0)?;
    file.seek(io::SeekFrom::Start(0))?;
    serde_json::to_writer(&mut file, holder)?;
    Ok(())
}

fn read_holder(lock_file: &Path) -> Option<LockHolder> {
    let contents = fs::read_to_string(lock_file).ok()?;
    serde_json::from_str(&contents).ok()
}

/// The outcome of a checkout-apply-test cycle, **attributed to the
/// checkout it was run in** (rule 4).
///
/// A verdict that cannot say which checkout produced it is exactly the
/// contamination problem of #3608: "81 failed" with no way to tell whether
/// the failures came from the patch or from the other run rewriting the
/// same worktree. The checkout field is therefore mandatory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    checkout: String,
    passed: u32,
    failed: u32,
}

impl Verdict {
    /// Record a verdict from `checkout`. The checkout path must be
    /// non-empty — a verdict without one is rejected.
    pub fn new(checkout: impl AsRef<str>, passed: u32, failed: u32) -> Result<Self, String> {
        let checkout = checkout.as_ref().trim();
        if checkout.is_empty() {
            return Err(
                "a verdict must record the checkout it was run in (issue #3608)".to_owned(),
            );
        }
        Ok(Self {
            checkout: checkout.to_owned(),
            passed,
            failed,
        })
    }

    /// The checkout this verdict was produced from.
    pub fn checkout(&self) -> &str {
        &self.checkout
    }

    /// Tests that passed.
    pub fn passed(&self) -> u32 {
        self.passed
    }

    /// Tests that failed.
    pub fn failed(&self) -> u32 {
        self.failed
    }

    /// True when the cycle ran clean (no failures).
    pub fn is_clean(&self) -> bool {
        self.failed == 0
    }
}

impl std::fmt::Display for Verdict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let status = if self.is_clean() { "PASSED" } else { "FAILED" };
        write!(
            f,
            "{status}. {} passed; {} failed — checkout: {}",
            self.passed, self.failed, self.checkout
        )
    }
}

/// A single ordered step of a job-supersession.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SupersessionStep {
    /// Stop a background job.
    Stop { job: String },
    /// Start a background job.
    Start { job: String },
}

/// The replacement of a superseded background job (rule 5).
///
/// The only legal order is: **stop the old job first, then start the
/// replacement**, and the replacement announces that it superseded the
/// old one. See [`Supersession::steps`] and [`Supersession::announcement`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Supersession {
    superseded_job: String,
    replacement_job: String,
}

impl Supersession {
    /// Plan the supersession of `superseded_job` by `replacement_job`.
    /// Both names must be non-empty and distinct.
    pub fn new(
        superseded_job: impl AsRef<str>,
        replacement_job: impl AsRef<str>,
    ) -> Result<Self, String> {
        let superseded_job = superseded_job.as_ref().trim();
        let replacement_job = replacement_job.as_ref().trim();
        if superseded_job.is_empty() || replacement_job.is_empty() {
            return Err("a supersession needs both the old and the replacement job".to_owned());
        }
        if superseded_job == replacement_job {
            return Err(
                "a job cannot supersede itself; stop it and start a replacement instead".to_owned(),
            );
        }
        Ok(Self {
            superseded_job: superseded_job.to_owned(),
            replacement_job: replacement_job.to_owned(),
        })
    }

    /// The job being superseded (stopped first).
    pub fn superseded_job(&self) -> &str {
        &self.superseded_job
    }

    /// The job replacing it.
    pub fn replacement_job(&self) -> &str {
        &self.replacement_job
    }

    /// The only legal order of operations: stop the old job **first**,
    /// then start the replacement.
    pub fn steps(&self) -> [SupersessionStep; 2] {
        [
            SupersessionStep::Stop {
                job: self.superseded_job.clone(),
            },
            SupersessionStep::Start {
                job: self.replacement_job.clone(),
            },
        ]
    }

    /// The announcement the replacement must make: which job it
    /// superseded, and that the old job was stopped first.
    pub fn announcement(&self) -> String {
        format!(
            "superseding background job {} with {}: the old job is stopped before the \
             replacement starts",
            self.superseded_job, self.replacement_job
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(tag: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("autospec-worktree-lock-{tag}-{}", process::id()));
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn lock_file_lives_beside_the_worktree_and_outside_the_checkout() {
        let path = lock_path(Path::new("/scratch/convert/worker-0")).unwrap();
        assert_eq!(path, Path::new("/scratch/convert/.worker-0.autospec-lock"));

        assert_eq!(
            lock_path(Path::new("/scratch/convert/worker-1")).unwrap(),
            Path::new("/scratch/convert/.worker-1.autospec-lock")
        );
    }

    #[test]
    fn lock_path_rejects_a_worktree_without_a_name_or_parent() {
        assert_eq!(lock_path(Path::new("/")), None);
        assert_eq!(lock_path(Path::new("")), None);
    }

    #[test]
    fn acquire_refuses_when_the_worktree_does_not_exist() {
        let root = temp_root("missing");
        let worktree = root.join("worker-0");
        let error = WorktreeLock::acquire(&worktree).unwrap_err();
        assert_eq!(
            error,
            WorktreeLockError::MissingWorktree {
                worktree: worktree.to_string_lossy().into_owned()
            }
        );
        assert!(error.to_string().contains("does not exist"));
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn second_acquisition_is_refused_and_says_why() {
        let root = temp_root("refused");
        let worktree = root.join("worker-0");
        fs::create_dir_all(&worktree).unwrap();

        let lock = WorktreeLock::acquire(&worktree).unwrap();
        assert_eq!(lock.worktree(), worktree.as_path());
        assert_eq!(
            lock.lock_file(),
            Path::new(&root).join(".worker-0.autospec-lock")
        );
        assert_eq!(lock.holder().pid, process::id());

        let second = WorktreeLock::acquire(&worktree).unwrap_err();
        let WorktreeLockError::AlreadyHeld {
            worktree: worktree_path,
            holder,
        } = &second
        else {
            panic!("expected AlreadyHeld, got {second:?}");
        };
        assert_eq!(worktree_path.as_str(), worktree.to_string_lossy().as_ref());
        let holder = holder.expect("holder info must be readable by the refuser");
        assert_eq!(holder.pid, process::id());
        // The refusal must say why the second invocation stopped.
        let message = second.to_string();
        assert!(message.contains("refusing to run"), "{message}");
        assert!(
            message.contains(&format!("pid {}", process::id())),
            "{message}"
        );
        assert!(message.contains("separate checkouts"), "{message}");

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_held_lock_refuses_a_second_open_even_in_the_same_process() {
        let root = temp_root("same-process");
        let worktree = root.join("worker-0");
        fs::create_dir_all(&worktree).unwrap();

        let lock = WorktreeLock::acquire(&worktree).unwrap();
        let contender = worktree.clone();
        let contender_path = contender.to_string_lossy().into_owned();
        let handle = std::thread::spawn(move || WorktreeLock::acquire(&contender));
        let second = handle.join().unwrap();
        match second {
            Err(WorktreeLockError::AlreadyHeld {
                worktree,
                holder: Some(h),
            }) => {
                assert_eq!(worktree, contender_path);
                assert_eq!(h.pid, process::id());
            }
            other => panic!("expected AlreadyHeld, got {other:?}"),
        }

        // Dropping the guard releases the lock: the same path can be
        // acquired again in a fresh checkout-apply-test cycle.
        drop(lock);
        let reacquired = WorktreeLock::acquire(&worktree).unwrap();
        assert_eq!(reacquired.holder().pid, process::id());

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn separate_checkouts_lock_independently() {
        let root = temp_root("separate");
        let worker_a = root.join("worker-0");
        let worker_b = root.join("worker-1");
        fs::create_dir_all(&worker_a).unwrap();
        fs::create_dir_all(&worker_b).unwrap();

        let lock_a = WorktreeLock::acquire(&worker_a).unwrap();
        // A different checkout is unaffected by lock A — this is what
        // "one checkout per worker" buys: true concurrency.
        let lock_b = WorktreeLock::acquire(&worker_b).unwrap();
        assert_ne!(lock_a.lock_file(), lock_b.lock_file());

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn verdict_records_the_checkout_it_was_run_in() {
        let verdict = Verdict::new("/scratch/convert/worker-0", 791, 81).unwrap();
        assert_eq!(verdict.checkout(), "/scratch/convert/worker-0");
        assert_eq!(verdict.passed(), 791);
        assert_eq!(verdict.failed(), 81);
        assert!(!verdict.is_clean());
        let rendered = verdict.to_string();
        assert!(
            rendered.starts_with("FAILED. 791 passed; 81 failed"),
            "{rendered}"
        );
        assert!(rendered.contains("/scratch/convert/worker-0"), "{rendered}");
    }

    #[test]
    fn a_clean_verdict_reports_passed() {
        let verdict = Verdict::new("/scratch/convert/worker-1", 872, 0).unwrap();
        assert!(verdict.is_clean());
        let rendered = verdict.to_string();
        assert!(
            rendered.starts_with("PASSED. 872 passed; 0 failed"),
            "{rendered}"
        );
    }

    #[test]
    fn a_verdict_without_a_checkout_is_rejected() {
        assert!(Verdict::new("", 100, 0).is_err());
        assert!(Verdict::new("   ", 100, 0).is_err());
        assert!(Verdict::new("", 100, 0)
            .unwrap_err()
            .contains("must record the checkout"));
    }

    #[test]
    fn supersession_stops_the_old_job_first_and_announces_it() {
        let plan = Supersession::new("job-2605", "job-2605-r2").unwrap();
        let steps = plan.steps();
        assert_eq!(
            steps,
            [
                SupersessionStep::Stop {
                    job: "job-2605".to_owned()
                },
                SupersessionStep::Start {
                    job: "job-2605-r2".to_owned()
                },
            ]
        );
        let announcement = plan.announcement();
        assert!(announcement.contains("job-2605"), "{announcement}");
        assert!(announcement.contains("job-2605-r2"), "{announcement}");
        assert!(announcement.contains("stopped before"), "{announcement}");
    }

    #[test]
    fn supersession_rejects_empty_or_self_jobs() {
        assert!(Supersession::new("", "r2").is_err());
        assert!(Supersession::new("job", "").is_err());
        assert!(Supersession::new("job", "job").is_err());
        assert!(Supersession::new("job", "job")
            .unwrap_err()
            .contains("cannot supersede itself"));
    }
}
