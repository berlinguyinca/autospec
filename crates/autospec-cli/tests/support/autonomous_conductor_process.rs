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
pub(super) fn real_bridge_e2e_lock() -> MutexGuard<'static, ()> {
    REAL_BRIDGE_E2E
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
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
