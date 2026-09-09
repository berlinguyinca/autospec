#[cfg(target_os = "linux")]
use std::fs;
use std::process::{Command, Stdio};
use std::sync::{Mutex, MutexGuard};

/// Serializes the tests that drive the real bridge end to end.
static REAL_BRIDGE_E2E: Mutex<()> = Mutex::new(());

/// The real-bridge serialization guard, tolerant of a poisoned mutex.
///
/// `lock().expect(..)` turns one failing test into a cascade: the panic poisons the mutex
/// and every later test that needs the guard dies on the lock instead of running. That is
/// what a CI run of this file looked like -- one real assertion failure at
/// `foreground_repeated_restart_observes_one_live_harness_until_merge`, then six
/// "real bridge E2E lock" panics that said nothing about anything. The data this guard
/// protects is `()`, so there is no invariant a previous panic can have broken; taking the
/// guard anyway is strictly more informative. Matches the pattern `test_environment()` in
/// the executor-bridge tests already uses.
/// The in-process mutex only orders tests within this binary; the
/// cross-binary half is the shared filesystem family lock, which every
/// `test_environment()`-guarded launch test in the unit test binary also
/// holds, so the supervision family is mutually exclusive across both test
/// binaries (#3857). The family lock is released by the field's Drop on
/// unwind, like the mutex guard.
pub(super) struct RealBridgeE2eLock {
    _mutex: MutexGuard<'static, ()>,
    _family: Option<supervision_family_lock::SupervisionFamilyLock>,
}

pub(super) fn real_bridge_e2e_lock() -> RealBridgeE2eLock {
    let mutex = REAL_BRIDGE_E2E
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    RealBridgeE2eLock {
        _mutex: mutex,
        _family: supervision_family_lock::acquire(),
    }
}

pub(super) fn process_is_running(pid: u32) -> bool {
    Command::new("ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .output()
        .is_ok_and(|output| {
            output.status.success()
                && !String::from_utf8_lossy(&output.stdout)
                    .trim_start()
                    .starts_with('Z')
        })
}

pub(super) fn wait_for_process_exit(pid: u32) {
    for _ in 0..100 {
        if !process_is_running(pid) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

pub(super) fn process_identity(pid: u32) -> Option<(u32, u64)> {
    #[cfg(target_os = "linux")]
    {
        let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        let (_, fields) = stat.rsplit_once(") ")?;
        let fields = fields.split_whitespace().collect::<Vec<_>>();
        return Some((fields.get(2)?.parse().ok()?, fields.get(19)?.parse().ok()?));
    }

    #[cfg(target_os = "macos")]
    {
        let raw_pid = i32::try_from(pid).ok()?;
        let mut info = std::mem::MaybeUninit::<nix::libc::proc_bsdinfo>::zeroed();
        // SAFETY: proc_pidinfo receives a correctly sized writable proc_bsdinfo buffer.
        let observed = unsafe {
            nix::libc::proc_pidinfo(
                raw_pid,
                nix::libc::PROC_PIDTBSDINFO,
                0,
                info.as_mut_ptr().cast(),
                std::mem::size_of::<nix::libc::proc_bsdinfo>() as i32,
            )
        };
        if observed as usize != std::mem::size_of::<nix::libc::proc_bsdinfo>() {
            return None;
        }
        // SAFETY: proc_pidinfo filled the complete structure above.
        let info = unsafe { info.assume_init() };
        let start = info
            .pbi_start_tvsec
            .checked_mul(1_000_000)?
            .checked_add(info.pbi_start_tvusec)?;
        let group = nix::unistd::getpgid(Some(nix::unistd::Pid::from_raw(raw_pid)))
            .ok()?
            .as_raw();
        return Some((u32::try_from(group).ok()?, start));
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    None
}

pub(super) fn current_host_identity() -> String {
    let output = Command::new("hostname")
        .output()
        .expect("read current host identity");
    assert!(output.status.success(), "hostname command failed");
    String::from_utf8(output.stdout)
        .expect("host identity is UTF-8")
        .trim()
        .to_string()
}

pub(super) fn current_boot_identity() -> String {
    #[cfg(target_os = "linux")]
    {
        return fs::read_to_string("/proc/sys/kernel/random/boot_id")
            .expect("read current boot identity")
            .trim()
            .to_string();
    }

    #[cfg(target_os = "macos")]
    {
        let name = b"kern.boottime\0";
        let mut boot = std::mem::MaybeUninit::<nix::libc::timeval>::zeroed();
        let mut length = std::mem::size_of::<nix::libc::timeval>();
        // SAFETY: sysctlbyname receives a static name and a correctly sized timeval buffer.
        let result = unsafe {
            nix::libc::sysctlbyname(
                name.as_ptr().cast(),
                boot.as_mut_ptr().cast(),
                &mut length,
                std::ptr::null_mut(),
                0,
            )
        };
        assert_eq!(result, 0, "read current boot identity");
        assert_eq!(length, std::mem::size_of::<nix::libc::timeval>());
        // SAFETY: sysctlbyname filled the complete structure above.
        let boot = unsafe { boot.assume_init() };
        return ((boot.tv_sec as u64) * 1_000_000 + boot.tv_usec as u64).to_string();
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    panic!("unsupported test host")
}

pub(super) fn terminate_process_group(pid: u32) {
    let _ = Command::new("kill")
        .args(["-KILL", "--", &format!("-{pid}")])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

pub(super) fn terminate_process(pid: u32) {
    let _ = Command::new("kill")
        .args(["-KILL", "--", &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[test]
fn real_bridge_e2e_lock_survives_a_poisoned_guard() {
    // Poison it deliberately, the way a failing test does, and then take it again. With
    // `lock().expect(..)` this second acquisition is the panic that hid the real failure.
    let poisoned = std::panic::catch_unwind(|| {
        let _guard = real_bridge_e2e_lock();
        panic!("poisoning the real-bridge guard on purpose");
    });
    assert!(poisoned.is_err(), "the panic must have happened");
    let _guard = real_bridge_e2e_lock();
}

/// Integration-side half of the cross-binary supervision family lock.
///
/// The supervision / process-lifecycle tests in the unit test binary (via
/// `test_environment()`) and the real-bridge E2E tests in this binary (via
/// `real_bridge_e2e_lock()`) both launch supervised harnesses, and that
/// machinery is observable process-globally. This module keeps the two
/// implementations in step — same file name, same claim format, same
/// staleness rule as
/// src/commands/autonomous/executor_bridge/tests/supervision_family_lock.rs
/// — so the family is mutually exclusive across both test binaries (#3857).
mod supervision_family_lock {
    use super::process_is_running;
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
            if !process_is_running(self.pid) {
                return true;
            }
            holder_exe_mismatch(self.pid, &self.exe)
        }
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
    /// `REAL_BRIDGE_E2E` guard that owns it.
    pub(super) struct SupervisionFamilyLock {
        lock_path: PathBuf,
        claim: Claim,
    }

    impl Drop for SupervisionFamilyLock {
        fn drop(&mut self) {
            // Release only while we are still the published holder. A live holder
            // is never replaced (only a stale one is), so a mismatch means this
            // process already lost the lock and must not delete the winner's file.
            if matches!(read_claim(&self.lock_path), Some(current) if current == self.claim) {
                let _ = fs::remove_file(&self.lock_path);
            }
        }
    }

    /// Block until the family lock is ours. Fails open (returns `None`) when the
    /// lock cannot be placed at all — for example `current_exe` is unavailable —
    /// matching the poison-recovery philosophy of `real_bridge_e2e_lock()`: a
    /// missing exclusion degrades to the pre-fix behaviour, it never hangs the
    /// suite.
    pub(super) fn acquire() -> Option<SupervisionFamilyLock> {
        let lock_path = lock_path()?;
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
                    return Some(SupervisionFamilyLock { lock_path, claim });
                }
                _ => {
                    let _ = fs::remove_file(&claim_path);
                    // Lost the rename race to another reclaimer; re-evaluate.
                }
            }
        }
    }
}
