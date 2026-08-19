use std::sync::{Mutex, MutexGuard};
use std::fs;
use std::process::{Command, Stdio};

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
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let (_, fields) = stat.rsplit_once(") ")?;
    let fields = fields.split_whitespace().collect::<Vec<_>>();
    Some((fields.get(2)?.parse().ok()?, fields.get(19)?.parse().ok()?))
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
