// executor_bridge tests: cross-binary exclusion for the supervision test family.
//
// The supervision / process-lifecycle tests launch real harnesses: a forked
// supervisor (setpgid + PR_SET_CHILD_SUBREAPER), a harness in its own process
// group, identity checks read from /proc. That machinery is observable
// process-globally, so two test binaries of this package running side by side
// (this unit test binary and the `autonomous_conductor_commands` integration
// binary, which drives live conductors) can perturb each other even when each
// binary serializes its own tests (#3857). A `static Mutex` cannot cross a
// process boundary, so the whole family shares one filesystem lock: every
// test that takes `test_environment()` holds this lock, and the integration
// side takes the same file from `real_bridge_e2e_lock()`
// (the `supervision_family_lock` module in tests/support/autonomous_conductor_process.rs
// keeps the two implementations in step — same file name, same claim format,
// same staleness rule).
//
// The protocol is claim-file + atomic rename rather than O_EXCL create: when a
// stale lock is reclaimed, the last renamer wins, the loser re-reads and loops,
// and no process can believe it holds a lock it does not. flock was rejected
// because the forked supervisor inherits the lock descriptor, and a
// quarantined supervisor kept alive by a cleanup test would hold the family
// lock long after its test finished.

use std::fs;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const LOCK_FILE_NAME: &str = "autospec-supervision-family.lock";
/// A holder is stale when its process is gone, its executable no longer
/// matches the recorded test binary, or when it has held the lock this long.
/// The last clause is crash safety for a killed run: no single test holds the
/// lock anywhere near this long (the slowest family test is a few dozen
/// seconds), so a live holder is never reclaimed by age alone, and a lock
/// stranded by `kill -9` costs at most this much waiting on the next run.
const STALE_AFTER_SECS: u64 = 600;
const POLL_INTERVAL: Duration = Duration::from_millis(25);
/// Publishes the current holder's claim header to processes the holder
/// spawns. Some family tests execute a helper as a *second copy of this same
/// test binary* (for example
/// `restart_direct::autonomous_executor_bridge_parent_crash_helper`, launched
/// by its parent test with `--exact`). That child also takes
/// `test_environment()`, but the protocol is not re-entrant across exec: the
/// child would poll for a lock its living ancestor holds, and the parent would
/// time out waiting for the child's launch records. A child that sees a live,
/// matching claim in this variable inherits the ancestor's hold instead of
/// contesting it. The variable is validated against the lock file and holder
/// liveness, so a stale or foreign value is ignored.
const HOLDER_ENV: &str = "AUTOSPEC_SUPERVISION_FAMILY_LOCK_CLAIM";

#[derive(Clone, PartialEq, Eq)]
struct Claim {
    pid: u32,
    started: u64,
    nonce: u64,
    /// The holder's own executable, recorded so a reused PID running an
    /// unrelated process cannot make a stranded lock look live.
    exe: String,
}

impl Claim {
    fn render(&self) -> String {
        // Two lines: the exe path may contain spaces, the header may not.
        format!(
            "{} {} {}\n{}\n",
            self.pid, self.started, self.nonce, self.exe
        )
    }

    fn parse(text: &str) -> Option<Claim> {
        let mut lines = text.lines();
        let mut parts = lines.next()?.split_whitespace();
        let pid = parts.next()?.parse().ok()?;
        let started = parts.next()?.parse().ok()?;
        let nonce = parts.next()?.parse().ok()?;
        Some(Claim {
            pid,
            started,
            nonce,
            exe: lines.next().unwrap_or("").to_owned(),
        })
    }

    /// Stale when the record is aged past the crash-safety bound, the recorded
    /// process is gone, or that PID now runs a different executable. The
    /// record's own PID is never an opponent (re-entrant acquisition in this
    /// process).
    fn is_stale(&self) -> bool {
        if self.pid == std::process::id() {
            return false;
        }
        let now = now_secs();
        if now.saturating_sub(self.started) > STALE_AFTER_SECS {
            return true;
        }
        if !holder_is_alive(self.pid) {
            return true;
        }
        holder_exe_mismatch(self.pid, &self.exe)
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn nonce() -> u64 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    std::thread::current().id().hash(&mut hasher);
    nanos as u64 ^ hasher.finish()
}

#[cfg(unix)]
fn holder_is_alive(pid: u32) -> bool {
    use nix::sys::signal::kill;
    use nix::unistd::Pid;
    // SAFETY: none — nix wraps kill(2); sending no signal probes liveness only.
    matches!(kill(Pid::from_raw(pid as i32), None), Ok(()))
}

#[cfg(not(unix))]
fn holder_is_alive(_pid: u32) -> bool {
    // Non-Unix hosts cannot probe; the age bound alone reclaims stale records.
    true
}

/// True when the recorded PID is alive but no longer runs the recorded test
/// binary — i.e. the PID was reused. Unverifiable (no /proc, permission,
/// empty record) leans live: the age bound still reclaims.
fn holder_exe_mismatch(pid: u32, recorded_exe: &str) -> bool {
    if recorded_exe.is_empty() {
        return false;
    }
    match std::fs::read_link(format!("/proc/{pid}/exe")) {
        Ok(current) => current.to_string_lossy() != recorded_exe,
        Err(_) => false,
    }
}

/// Both test binaries of this package are linked into `<target>/.../deps`, so
/// the parent of the running executable is one directory shared by every
/// binary of one build tree — the natural home for the family lock.
fn lock_path() -> Option<PathBuf> {
    let executable = std::env::current_exe().ok()?;
    let directory = executable.parent()?;
    Some(directory.join(LOCK_FILE_NAME))
}

fn read_claim(path: &std::path::Path) -> Option<Claim> {
    let text = fs::read_to_string(path).ok()?;
    Claim::parse(&text)
}

/// Holds the family lock until dropped, releasing it on unwind like the
/// `TestEnvironment` guard that owns it.
pub(super) struct SupervisionFamilyLock {
    lock_path: PathBuf,
    claim: Claim,
    /// Set when this handle inherits a live ancestor's hold via `HOLDER_ENV`
    /// instead of publishing a claim of its own. Such a handle never unlinks
    /// the lock file: the ancestor owns the publication.
    nested: bool,
}

impl Drop for SupervisionFamilyLock {
    fn drop(&mut self) {
        if self.nested {
            return;
        }
        // Release only while we are still the published holder. A live holder
        // is never replaced (only a stale one is), so a mismatch means this
        // process already lost the lock and must not delete the winner's file.
        if matches!(read_claim(&self.lock_path), Some(current) if current == self.claim) {
            let _ = fs::remove_file(&self.lock_path);
        }
    }
}

/// The claim header (`pid started nonce`) published in `HOLDER_ENV` by the
/// current top-level holder.
fn claim_header(claim: &Claim) -> String {
    format!("{} {} {}", claim.pid, claim.started, claim.nonce)
}

/// When a live ancestor process holds the family lock and published its claim
/// in `HOLDER_ENV`, return that claim so this process inherits the hold.
/// Returns `None` (falling through to normal acquisition) when the variable is
/// absent, names this process, no longer matches the lock file, or belongs to
/// a dead or replaced holder.
fn nested_ancestor_claim(lock_path: &std::path::Path) -> Option<Claim> {
    let header = std::env::var(HOLDER_ENV).ok()?;
    let current = read_claim(lock_path)?;
    (claim_header(&current) == header.trim())
        .then_some(current)
        .filter(|claim| claim.pid != std::process::id() && !claim.is_stale())
}

/// Block until the family lock is ours. Fails open (returns `None`) when the
/// lock cannot be placed at all — for example `current_exe` is unavailable —
/// matching the poison-recovery philosophy of `test_environment()`: a missing
/// exclusion degrades to the pre-fix behaviour, it never hangs the suite.
pub(super) fn acquire() -> Option<SupervisionFamilyLock> {
    let lock_path = lock_path()?;
    if let Some(holder) = nested_ancestor_claim(&lock_path) {
        return Some(SupervisionFamilyLock {
            lock_path,
            claim: holder,
            nested: true,
        });
    }
    let exe = std::env::current_exe()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_default();
    let claim = Claim {
        pid: std::process::id(),
        started: now_secs(),
        nonce: nonce(),
        exe,
    };
    loop {
        match read_claim(&lock_path) {
            Some(holder) if !holder.is_stale() => {
                std::thread::sleep(POLL_INTERVAL);
                continue;
            }
            _ => {}
        }
        // Publish by renaming a private claim file over the lock path: the
        // rename is atomic, the loser of a stale-reclaim race sees the winner
        // on the read-back below and loops instead of double-holding.
        let claim_path = lock_path.with_file_name(format!(
            "{}.claim-{}-{}",
            LOCK_FILE_NAME, claim.pid, claim.nonce
        ));
        if fs::write(&claim_path, claim.render()).is_err() {
            std::thread::sleep(POLL_INTERVAL);
            continue;
        }
        if fs::rename(&claim_path, &lock_path).is_err() {
            let _ = fs::remove_file(&claim_path);
            std::thread::sleep(POLL_INTERVAL);
            continue;
        }
        match read_claim(&lock_path) {
            Some(current) if current == claim => {
                let _ = fs::remove_file(&claim_path);
                // Publish the claim for helper processes this test spawns as
                // second copies of the binary. Never removed: a stale value is
                // rejected by `nested_ancestor_claim`, and each top-level
                // acquisition overwrites it.
                std::env::set_var(HOLDER_ENV, claim_header(&claim));
                return Some(SupervisionFamilyLock {
                    lock_path,
                    claim,
                    nested: false,
                });
            }
            _ => {
                let _ = fs::remove_file(&claim_path);
                // Lost the rename race to another reclaimer; re-evaluate.
            }
        }
    }
}
