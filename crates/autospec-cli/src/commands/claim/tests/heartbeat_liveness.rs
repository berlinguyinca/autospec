// claim tests: a heartbeat only blocks recovery while its owner is actually alive.
//
// Regression cover for the wedge in berlinguyinca/autospec#3012 §2: the capacity guard was a
// bare `is_file()`, so a heartbeat left behind by a worker that died without cleaning up made
// its claim look active until the 3h TTL. On autotrade#1426 that blocked every successor
// conductor -- each exiting in ~1s with `claim_lost` while the supervisor restarted it 1017x.
//
// These exercise `startup_heartbeat_owner_is_gone` directly rather than going through
// `startup_heartbeat_exists`, which resolves its root from AUTOSPEC_HEARTBEAT_DIR. Setting that
// is a process-wide mutation, and sibling suites here serialise on a *different* mutex, so an
// env-based test raced them and made an unrelated bridge_terminal case fail intermittently.
// The predicate is where the liveness decision lives; the caller only supplies the path.

use crate::commands::claim;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

fn sandbox(label: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!(
        "autospec-heartbeat-liveness-{label}-{}-{}",
        std::process::id(),
        claim::UNIQUE_ID_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&directory).expect("sandbox");
    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))
        .expect("private sandbox");
    directory
}

/// A well-formed heartbeat for issue 42 owned by `pid` on `boot_id`.
fn heartbeat(directory: &Path, pid: u32, boot_id: &str) -> PathBuf {
    heartbeat_with_start(directory, pid, boot_id, "1")
}

fn heartbeat_with_start(directory: &Path, pid: u32, boot_id: &str, process_start: &str) -> PathBuf {
    let nonce = claim::startup_heartbeat_nonce("owner/repo", 42, "claim-1");
    let document = format!(
        r#"{{"repo":"owner/repo","issue":"42","worker_id":"worker-1","branch":"feat/worker","pr":"","claim_id":"claim-1","step":"claimed","ts":1,"ttl_seconds":10800,"pid":{pid},"nonce":"{nonce}","host":"bender","boot_id":"{boot_id}","process_start":"{process_start}"}}"#
    );
    let path = directory.join("42.json");
    std::fs::write(&path, document).expect("write heartbeat");
    path
}

fn this_boot() -> String {
    super::super::super::autonomous::current_boot_identity().expect("boot identity")
}

/// The native start identity for a live pid — what a genuine heartbeat would record.
fn start_identity_of(pid: u32) -> String {
    super::super::super::autonomous::process_birth_identity(pid)
        .expect("observe process")
        .expect("process is live")
        .1
}

#[cfg(unix)]
#[test]
fn a_heartbeat_owned_by_a_live_process_is_not_gone() {
    let directory = sandbox("live");
    // Our own pid is unambiguously alive.
    let pid = std::process::id();
    let path = heartbeat_with_start(&directory, pid, &this_boot(), &start_identity_of(pid));
    assert!(
        !claim::heartbeat_liveness::startup_heartbeat_owner_is_gone(&path),
        "a live owner must keep its claim -- releasing it would let two workers run one issue"
    );
}

#[cfg(unix)]
#[test]
fn a_heartbeat_owned_by_a_dead_process_is_gone() {
    let directory = sandbox("dead");
    // Above the kernel's pid maximum, so probing yields ESRCH rather than naming a real process.
    let path = heartbeat(&directory, i32::MAX as u32, &this_boot());
    assert!(
        claim::heartbeat_liveness::startup_heartbeat_owner_is_gone(&path),
        "a heartbeat whose owner is provably gone must stop blocking recovery (#3012 §2)"
    );
}

#[cfg(unix)]
#[test]
fn a_dead_pid_from_another_boot_fails_closed() {
    let directory = sandbox("foreign-boot");
    // PIDs are recycled across reboots, so a record from another boot proves nothing about the
    // pid it names. Keep the old behaviour rather than guess.
    let path = heartbeat(
        &directory,
        i32::MAX as u32,
        "00000000-0000-0000-0000-000000000000",
    );
    assert!(
        !claim::heartbeat_liveness::startup_heartbeat_owner_is_gone(&path),
        "a record from another boot is not evidence of death; it must fail closed"
    );
}

#[cfg(unix)]
#[test]
fn an_unparseable_heartbeat_fails_closed() {
    let directory = sandbox("garbage");
    let path = directory.join("42.json");
    std::fs::write(&path, b"{not json").expect("write");
    assert!(
        !claim::heartbeat_liveness::startup_heartbeat_owner_is_gone(&path),
        "an unreadable heartbeat is not proof of death; it must keep blocking"
    );
}

#[cfg(unix)]
#[test]
fn a_missing_heartbeat_fails_closed() {
    let directory = sandbox("absent");
    assert!(
        !claim::heartbeat_liveness::startup_heartbeat_owner_is_gone(&directory.join("42.json")),
        "absence is handled by the caller's is_file() check, not by claiming death"
    );
}

#[cfg(unix)]
#[test]
fn a_mismatched_pid_identity_fails_closed() {
    let directory = sandbox("recycled");
    // The pid is live, but its immutable start identity does not match the heartbeat. That could
    // be PID reuse or corrupt evidence, so the cleanup path must not guess which occurred.
    let pid = std::process::id();
    let path = heartbeat_with_start(&directory, pid, &this_boot(), "1");
    assert_ne!(
        start_identity_of(pid),
        "1",
        "fixture assumes a real start identity"
    );
    assert!(
        !claim::heartbeat_liveness::startup_heartbeat_owner_is_gone(&path),
        "identity mismatch is ambiguous and must not transfer cleanup authority"
    );
}
