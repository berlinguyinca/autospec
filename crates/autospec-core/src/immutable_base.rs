//! Shared-base refresh that never disturbs a running reader (#3732).
//!
//! The race: a "base refresh" (git fetch + reset --hard) mutates the shared
//! base directory in place while concurrently dispatched jobs are copying
//! it, so a job's copy fails mid-read ("tar: file changed as we read it")
//! and the job dies. A fix that mutates shared state must be exclusive with
//! its readers; here the writer and the readers are different processes, so
//! this module makes the mutation impossible instead of taking a lock:
//!
//! * The writer publishes each refresh as a NEW immutable generation
//!   directory and swaps the `current` symlink onto it
//!   ([`publish_generation`]). It never writes into a directory that
//!   already exists, so no write lands in any directory a running job can
//!   be reading.
//! * A job pins its base ONCE at start: resolve the symlink exactly once
//!   and copy from the resolved real path ([`resolve_current`]). The
//!   resolved base carries the explicit sha the generation was built at,
//!   so a job that prefers to fetch its own base has the exact revision to
//!   check out instead of a shared directory it must copy.
//! * Old generations are retired by [`plan_gc`] / [`gc_generations`]: a
//!   generation is removed only when it is no longer `current`, no longer
//!   among the newest `keep`, and older than `min_age_secs`. The gc
//!   re-reads the symlink before each removal, so a generation that became
//!   current mid-gc survives.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Directory that holds the published generations.
pub const GENERATIONS_DIR: &str = "generations";
/// Symlink that names the generation jobs should read.
pub const CURRENT_LINK: &str = "current";
/// Record inside each generation carrying the base sha it was built at.
pub const SHA_RECORD: &str = "BASE_SHA";
/// Prefix of every generation directory name (`gen-<zero-padded id>`).
pub const GENERATION_PREFIX: &str = "gen-";

/// One published base generation: a directory that is never written after
/// publication.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaseGeneration {
    /// Zero-padded sequence number (`0000000001`); the directory is
    /// `gen-<id>`.
    pub id: String,
    /// The explicit base sha this generation was built at.
    pub sha: String,
    /// The real (non-symlink) generation path.
    pub path: PathBuf,
    /// Publication time in unix milliseconds (0 when unknown).
    pub created_millis: i64,
}

/// Receipt for a successful [`publish_generation`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BasePublishReceipt {
    /// The new generation that now carries the refreshed base.
    pub generation: BaseGeneration,
    /// The `current` symlink, now pointing at the new generation.
    pub current: PathBuf,
    /// The generation the symlink pointed at before the swap, if any.
    pub previous: Option<PathBuf>,
}

/// A base pinned by a job: resolve once, then read only from `generation`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinnedBase {
    /// The base root the job resolved.
    pub root: PathBuf,
    /// The real generation the job must copy from — never the symlink.
    pub generation: BaseGeneration,
}

/// Retention plan produced by [`plan_gc`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GcPlan {
    /// Generation ids to remove.
    pub remove: Vec<String>,
    /// Generation ids to keep, with no reason attached (every retained
    /// generation is either current, among the newest `keep`, or fresh).
    pub retain: Vec<String>,
}

/// Result of [`gc_generations`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GcReport {
    /// Generation directories that were removed.
    pub removed: Vec<PathBuf>,
    /// Generation directories that were not removed, with the reason.
    pub skipped: Vec<(PathBuf, String)>,
}

/// Why a base operation failed. Fail closed: an unresolvable base is an
/// error, never "use whatever is there".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BaseError {
    /// The `current` symlink is missing or unresolvable.
    NoCurrent { root: PathBuf, reason: String },
    /// A generation directory has no usable `BASE_SHA` record.
    MissingShaRecord { path: PathBuf },
    /// The caller passed a sha that is not a plausible git revision.
    BadSha { sha: String },
    /// The caller's prepare step failed while filling the new generation.
    Prepare { path: PathBuf, source: String },
    /// A filesystem operation failed.
    Io {
        operation: String,
        path: PathBuf,
        source: String,
    },
}

impl fmt::Display for BaseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BaseError::NoCurrent { root, reason } => {
                write!(f, "no current base at {}: {}", root.display(), reason)
            }
            BaseError::MissingShaRecord { path } => {
                write!(f, "missing {} record in {}", SHA_RECORD, path.display())
            }
            BaseError::BadSha { sha } => write!(f, "refusing implausible base sha {sha:?}"),
            BaseError::Prepare { path, source } => {
                write!(f, "prepare failed for {}: {}", path.display(), source)
            }
            BaseError::Io {
                operation,
                path,
                source,
            } => write!(f, "{operation} on {} failed: {}", path.display(), source),
        }
    }
}

impl std::error::Error for BaseError {}

fn io_err(operation: &str, path: &Path, source: &std::io::Error) -> BaseError {
    BaseError::Io {
        operation: operation.to_owned(),
        path: path.to_path_buf(),
        source: source.to_string(),
    }
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn mtime_millis(path: &Path) -> i64 {
    fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn validate_sha(sha: &str) -> Result<(), BaseError> {
    let ok = !sha.is_empty() && sha.len() <= 64 && sha.bytes().all(|b| b.is_ascii_hexdigit());
    if ok {
        Ok(())
    } else {
        Err(BaseError::BadSha {
            sha: sha.to_owned(),
        })
    }
}

fn generation_path(generations: &Path, id: i64) -> PathBuf {
    generations.join(format!("{GENERATION_PREFIX}{id:010}"))
}

/// Scan `generations/` for existing ids and return the next one.
fn next_generation_id(generations: &Path) -> Result<i64, BaseError> {
    let mut max = 0i64;
    let entries = fs::read_dir(generations).map_err(|e| io_err("read_dir", generations, &e))?;
    for entry in entries {
        let entry = entry.map_err(|e| io_err("read_dir", generations, &e))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(num) = name
            .strip_prefix(GENERATION_PREFIX)
            .and_then(|s| s.parse::<i64>().ok())
        else {
            continue;
        };
        if num > max {
            max = num;
        }
    }
    Ok(max + 1)
}

/// Allocate a fresh generation directory. `create_dir` is atomic (O_EXCL),
/// so concurrent publishers cannot land on the same id: on a collision the
/// scan is repeated.
fn allocate_generation_dir(generations: &Path) -> Result<(i64, PathBuf), BaseError> {
    for _ in 0..32 {
        let id = next_generation_id(generations)?;
        let dir = generation_path(generations, id);
        match fs::create_dir(&dir) {
            Ok(()) => return Ok((id, dir)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(io_err("create_dir", &dir, &e)),
        }
    }
    Err(io_err(
        "allocate generation (id collision loop)",
        generations,
        &std::io::Error::other("allocation failed after 32 attempts"),
    ))
}

/// Best-effort fsync of a directory so the rename is durable.
fn sync_dir(dir: &Path) -> Result<(), BaseError> {
    if let Ok(file) = fs::File::open(dir) {
        file.sync_all().map_err(|e| io_err("sync_all", dir, &e))?;
    }
    Ok(())
}

static PUBLISH_NONCE: AtomicU64 = AtomicU64::new(0);

/// Create `current` atomically: build a temp symlink beside it, then
/// `rename` over `current` (atomic on POSIX: readers see either the old or
/// the new target, never neither). A stale temp left by a crashed
/// publisher is ours to reclaim.
fn swap_symlink(current: &Path, target: &str) -> Result<(), BaseError> {
    let nonce = PUBLISH_NONCE.fetch_add(1, Ordering::Relaxed);
    let tmp = current.with_file_name(format!(".current.{}-{nonce}", process::id()));
    let _ = fs::remove_file(&tmp);
    std::os::unix::fs::symlink(target, &tmp).map_err(|e| io_err("symlink", &tmp, &e))?;
    if let Err(e) = fs::rename(&tmp, current) {
        let _ = fs::remove_file(&tmp);
        return Err(io_err("rename", current, &e));
    }
    Ok(())
}

/// Publish a refreshed base as a new immutable generation and atomically
/// swap the `current` symlink onto it.
///
/// `sha` is the explicit base revision the generation is built at (it is
/// recorded in `BASE_SHA` so a reader can name it, or fetch its own copy).
/// `prepare` fills the fresh generation directory — `git fetch` + checkout
/// of `sha`, or a copy — and after it returns the directory is never
/// written again. Existing generations are untouched by this call, which is
/// what keeps a job that already resolved one reading a stable tree.
pub fn publish_generation(
    root: &Path,
    sha: &str,
    prepare: impl FnOnce(&Path) -> Result<(), String>,
) -> Result<BasePublishReceipt, BaseError> {
    validate_sha(sha)?;
    let generations = root.join(GENERATIONS_DIR);
    fs::create_dir_all(&generations).map_err(|e| io_err("create_dir_all", &generations, &e))?;
    let (id, generation) = allocate_generation_dir(&generations)?;
    if let Err(source) = prepare(&generation) {
        let _ = fs::remove_dir_all(&generation);
        return Err(BaseError::Prepare {
            path: generation,
            source,
        });
    }
    let record = generation.join(SHA_RECORD);
    fs::write(&record, format!("{sha}\n")).map_err(|e| io_err("write", &record, &e))?;
    sync_dir(&generation)?;
    sync_dir(&generations)?;
    let current = root.join(CURRENT_LINK);
    let previous = fs::read_link(&current)
        .ok()
        .map(|t| current.parent().map(|p| p.join(&t)).unwrap_or(t));
    // The symlink target is the generation's path relative to the root, so
    // the link keeps working if the base root is moved or copied.
    let target = format!("{GENERATIONS_DIR}/{GENERATION_PREFIX}{id:010}");
    swap_symlink(&current, &target)?;
    sync_dir(root)?;
    let created_millis = mtime_millis(&generation);
    Ok(BasePublishReceipt {
        generation: BaseGeneration {
            id: format!("{id:010}"),
            sha: sha.to_owned(),
            path: generation,
            created_millis,
        },
        current,
        previous,
    })
}

/// Resolve the `current` symlink exactly once and return the generation a
/// job should copy from, together with the explicit sha it was built at.
///
/// A job calls this ONCE at start and then reads only from the returned
/// real path; re-resolving mid-copy is how a job would chase a writer. The
/// writer never mutates a published generation, so the pinned path stays
/// byte-stable for the life of the job.
pub fn resolve_current(root: &Path) -> Result<PinnedBase, BaseError> {
    let current = root.join(CURRENT_LINK);
    let target = fs::read_link(&current).map_err(|e| BaseError::NoCurrent {
        root: root.to_path_buf(),
        reason: format!("cannot read link: {e}"),
    })?;
    let generation = if target.is_absolute() {
        target
    } else {
        root.join(target)
    };
    if !generation.is_dir() {
        return Err(BaseError::NoCurrent {
            root: root.to_path_buf(),
            reason: format!(
                "{} points at {} which is not a directory",
                CURRENT_LINK,
                generation.display()
            ),
        });
    }
    let name = generation
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    let Some(id_str) = name.strip_prefix(GENERATION_PREFIX) else {
        return Err(BaseError::NoCurrent {
            root: root.to_path_buf(),
            reason: format!(
                "{} does not name a {}* directory",
                CURRENT_LINK, GENERATION_PREFIX
            ),
        });
    };
    if id_str.parse::<i64>().is_err() {
        return Err(BaseError::NoCurrent {
            root: root.to_path_buf(),
            reason: format!("generation id {id_str:?} is not numeric"),
        });
    }
    let record = generation.join(SHA_RECORD);
    let sha = fs::read_to_string(&record).map_err(|_| BaseError::MissingShaRecord {
        path: record.clone(),
    })?;
    let sha = sha.trim().to_owned();
    if sha.is_empty() {
        return Err(BaseError::MissingShaRecord { path: record });
    }
    let created_millis = mtime_millis(&generation);
    Ok(PinnedBase {
        root: root.to_path_buf(),
        generation: BaseGeneration {
            id: id_str.to_owned(),
            sha,
            path: generation,
            created_millis,
        },
    })
}

/// Pure retention plan: which generation ids to remove.
///
/// A generation is retained when it is the `current` one, when it is among
/// the newest `keep` (by id), or when it is younger than `min_age_secs`;
/// everything else is removed. `now_millis` and `created_millis` are unix
/// milliseconds so the plan is a pure function testable without a clock.
pub fn plan_gc(
    generations: &[BaseGeneration],
    current_id: Option<&str>,
    keep: usize,
    min_age_secs: u64,
    now_millis: i64,
) -> GcPlan {
    let mut sorted: Vec<&BaseGeneration> = generations.iter().collect();
    sorted.sort_by(|a, b| b.id.cmp(&a.id));
    let min_age_millis = i64::try_from(min_age_secs)
        .map(|s| s * 1000)
        .unwrap_or(i64::MAX);
    let mut plan = GcPlan {
        remove: Vec::new(),
        retain: Vec::new(),
    };
    for (rank, gen) in sorted.iter().enumerate() {
        let age = now_millis.saturating_sub(gen.created_millis);
        let fresh = age < min_age_millis;
        let is_current = Some(gen.id.as_str()) == current_id;
        if is_current || rank < keep || fresh {
            plan.retain.push(gen.id.clone());
        } else {
            plan.remove.push(gen.id.clone());
        }
    }
    plan
}

/// Read the id the `current` symlink points at, if it does.
fn read_current_id(root: &Path) -> Option<String> {
    let target = fs::read_link(root.join(CURRENT_LINK)).ok()?;
    target
        .file_name()
        .and_then(|n| n.to_str())?
        .strip_prefix(GENERATION_PREFIX)
        .map(|s| s.to_owned())
}

/// Scan `root`, plan with [`plan_gc`], and remove the planned generations.
///
/// The `current` symlink is re-read before each removal: a generation that
/// became current between the scan and the removal survives (the plan
/// already protects the current one; the re-read closes the window).
/// Generations without a usable `BASE_SHA` record are skipped, never
/// removed — a generation that cannot be identified is not garbage.
pub fn gc_generations(root: &Path, keep: usize, min_age_secs: u64) -> Result<GcReport, BaseError> {
    let generations = root.join(GENERATIONS_DIR);
    let mut report = GcReport {
        removed: Vec::new(),
        skipped: Vec::new(),
    };
    if !generations.is_dir() {
        return Ok(report);
    }
    let mut found = Vec::new();
    let entries = fs::read_dir(&generations).map_err(|e| io_err("read_dir", &generations, &e))?;
    for entry in entries {
        let entry = entry.map_err(|e| io_err("read_dir", &generations, &e))?;
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(id_str) = name.strip_prefix(GENERATION_PREFIX) else {
            continue;
        };
        let Ok(id_num) = id_str.parse::<i64>() else {
            continue;
        };
        let record = path.join(SHA_RECORD);
        let Ok(raw) = fs::read_to_string(&record) else {
            report
                .skipped
                .push((path, "missing BASE_SHA record".to_owned()));
            continue;
        };
        let sha = raw.trim().to_owned();
        if sha.is_empty() {
            report
                .skipped
                .push((path, "empty BASE_SHA record".to_owned()));
            continue;
        }
        let created_millis = mtime_millis(&path);
        found.push(BaseGeneration {
            id: format!("{id_num:010}"),
            sha,
            path,
            created_millis,
        });
    }
    let current_id = read_current_id(root);
    let plan = plan_gc(
        &found,
        current_id.as_deref(),
        keep,
        min_age_secs,
        now_millis(),
    );
    for id in &plan.remove {
        let Some(gen) = found.iter().find(|g| &g.id == id) else {
            continue;
        };
        let path = gen.path.clone();
        // TOCTOU guard: re-read the symlink before removing.
        if read_current_id(root).is_some_and(|live| live == *id) {
            report
                .skipped
                .push((path, "became current during gc".to_owned()));
            continue;
        }
        match fs::remove_dir_all(&path) {
            Ok(()) => report.removed.push(path),
            Err(e) => report.skipped.push((path, format!("remove failed: {e}"))),
        }
    }
    Ok(report)
}
