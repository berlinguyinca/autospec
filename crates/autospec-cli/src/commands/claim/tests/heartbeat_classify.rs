// claim tests: heartbeat / classify — 10 cases.
//
// Split out of tests.rs; see the note in that file.

#[cfg(target_os = "linux")]
use super::support::lock_heartbeat_env;
use super::support::{
    assert_fifo_reader_nonblocking, expected_startup_heartbeat, startup_heartbeat_document,
    startup_heartbeat_fixture,
};
use crate::commands::claim;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(target_os = "linux")]
use std::path::{Path, PathBuf};

#[cfg(target_os = "linux")]
#[test]
fn startup_heartbeat_remote_process_identity() {
    let _environment = lock_heartbeat_env();
    let (directory, _) = startup_heartbeat_fixture("remote-process-identity");
    let root = directory.join(".autospec/process-heartbeats");
    let old_root = std::env::var_os("AUTOSPEC_HEARTBEAT_DIR");
    std::env::set_var("AUTOSPEC_HEARTBEAT_DIR", &root);
    claim::write_startup_heartbeat(
        "owner/repo",
        42,
        "opaque-worker",
        "feat/worker",
        "claim-a",
        None,
    )
    .unwrap();
    let path = root
        .join(super::super::super::autonomous::drain::repository_progress_key("owner/repo"))
        .join("42.json");
    let document = std::fs::read(&path).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&document).unwrap();
    let identity = |name| {
        value
            .get(name)
            .and_then(serde_json::Value::as_str)
            .filter(|field| !field.is_empty())
            .unwrap()
            .to_string()
    };
    let host = identity("host");
    let boot_id = identity("boot_id");
    let process_start = identity("process_start");
    assert_ne!(host, boot_id);
    assert_ne!(boot_id, process_start);
    let absent_pid = i32::MAX as u32;
    assert!(!Path::new(&format!("/proc/{absent_pid}")).exists());
    for (field, replacement, remote) in [
        ("host", "remote-host", true),
        ("boot_id", "remote-boot", false),
        ("process_start", "0", true),
        ("process_start", "garbage", true),
        ("process_start", "01", true),
    ] {
        let mut mutated = value.clone();
        mutated[field] = serde_json::json!(replacement);
        if remote {
            mutated["pid"] = serde_json::json!(absent_pid);
        }
        std::fs::write(&path, serde_json::to_vec(&mutated).unwrap()).unwrap();
        assert_eq!(
            claim::classify_startup_heartbeat(
                &path,
                expected_startup_heartbeat("opaque-worker"),
                value["ts"].as_u64().unwrap() + value["ttl_seconds"].as_u64().unwrap() + 1,
                claim::observe_local_startup_pid,
            ),
            claim::StartupHeartbeatClassification::Blocking,
            "{field}"
        );
    }
    match old_root {
        Some(value) => std::env::set_var("AUTOSPEC_HEARTBEAT_DIR", value),
        None => std::env::remove_var("AUTOSPEC_HEARTBEAT_DIR"),
    }
    std::fs::remove_dir_all(directory).unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn stale_heartbeat_receipt_transaction() {
    use claim::HeartbeatReceiptDecision::{Absent, Blocking, Completed, Pending};
    use nix::fcntl::{open, OFlag};
    use nix::sys::stat::Mode;
    use std::os::unix::fs::symlink;
    use std::os::unix::net::UnixListener;
    use std::os::unix::prelude::AsRawFd;
    use std::time::{Duration, Instant};
    let expected = expected_startup_heartbeat("host:user:rust:4242:nonce-a");
    let open_parent = |path: &Path| {
        std::fs::File::from(open(path, OFlag::O_PATH | OFlag::O_DIRECTORY, Mode::empty()).unwrap())
    };
    let (parent_path, _) = startup_heartbeat_fixture("receipt-red");
    let repo_path = parent_path.join("repo");
    std::fs::create_dir(&repo_path).expect("repo directory");
    std::fs::set_permissions(&repo_path, std::fs::Permissions::from_mode(0o700))
        .expect("private repo");
    let parent = open_parent(&parent_path);
    let repo =
        claim::open_heartbeat_directory_beneath(&parent, Path::new("repo")).expect("repo fd");
    let decision = || claim::heartbeat_receipt_retry_decision(&repo, expected);
    let transaction = claim::begin_heartbeat_receipt(&repo, expected).expect("begin receipt");
    assert_eq!(decision(), Pending);
    let mut unrelated = expected;
    unrelated.issue += 1;
    assert_eq!(
        claim::heartbeat_receipt_retry_decision(&repo, unrelated),
        Absent
    );
    claim::retire_heartbeat_receipt_with_sync(transaction, |_| {
        Err(claim::CommandFailure::diagnostic("injected sync failure"))
    })
    .expect_err("sync failure");
    assert_eq!(decision(), Completed);
    assert!(claim::begin_heartbeat_receipt(&repo, expected).is_err());
    let (pending, completed) = claim::heartbeat_receipt_names(expected);
    let handoff = repo_path.join("quarantine/startup-heartbeat-handoffs");
    let pending_path = handoff.join(&pending);
    let completed_path = handoff.join(&completed);
    std::fs::remove_file(&completed_path).unwrap();
    claim::begin_heartbeat_receipt_with_hook(&repo, expected, || {
        std::fs::write(&completed_path, b"").unwrap();
        std::fs::set_permissions(&completed_path, std::fs::Permissions::from_mode(0o600)).unwrap();
    })
    .err()
    .expect("completed wins");
    assert_eq!(decision(), Blocking);
    assert!(pending_path.exists() && completed_path.exists());
    std::fs::remove_file(&pending_path).unwrap();
    std::fs::remove_file(&completed_path).unwrap();
    let mut transaction = claim::begin_heartbeat_receipt(&repo, expected).unwrap();
    transaction.pending.push_str("-missing");
    claim::retire_heartbeat_receipt_with_sync(transaction, |_| Ok(())).expect_err("rename failure");
    assert_eq!(decision(), Pending);
    let (pending, completed) = claim::heartbeat_receipt_names(expected);
    let handoff_fd = std::fs::File::open(&handoff).unwrap();
    let socket_root = PathBuf::from(format!("/proc/self/fd/{}", handoff_fd.as_raw_fd()));
    let drift = claim::heartbeat_receipt_retry_decision_with_hook(&repo, expected, |boundary| {
        if boundary == "handoff" {
            std::fs::set_permissions(&handoff, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    });
    assert_eq!(drift, Blocking);
    std::fs::set_permissions(&handoff, std::fs::Permissions::from_mode(0o700)).unwrap();
    let pending_path = handoff.join(&pending);
    let completed_path = handoff.join(&completed);
    std::fs::write(&completed_path, b"").unwrap();
    std::fs::set_permissions(&completed_path, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(decision(), Blocking);
    std::fs::remove_file(&completed_path).unwrap();
    for unsafe_kind in ["fifo", "symlink", "socket", "size", "mode"] {
        std::fs::remove_file(&pending_path).unwrap();
        match unsafe_kind {
            "fifo" => nix::unistd::mkfifo(&pending_path, Mode::from_bits_truncate(0o600)).unwrap(),
            "symlink" => symlink("/dev/null", &pending_path).unwrap(),
            "socket" => {
                drop(UnixListener::bind(socket_root.join(&pending)).unwrap());
                std::fs::set_permissions(&pending_path, std::fs::Permissions::from_mode(0o600))
                    .unwrap();
            }
            "size" => std::fs::write(&pending_path, b"x").unwrap(),
            _ => std::fs::write(&pending_path, b"").unwrap(),
        }
        if unsafe_kind == "mode" {
            std::fs::set_permissions(&pending_path, std::fs::Permissions::from_mode(0o644))
                .unwrap();
        }
        let started = Instant::now();
        assert_eq!(decision(), Blocking);
        assert!(started.elapsed() < Duration::from_secs(2));
    }
    std::fs::remove_file(&pending_path).unwrap();
    drop(UnixListener::bind(socket_root.join(&completed)).unwrap());
    std::fs::set_permissions(&completed_path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let started = Instant::now();
    assert_eq!(decision(), Blocking);
    assert!(started.elapsed() < Duration::from_secs(2));
    let dev = std::fs::File::open("/dev").unwrap();
    assert_eq!(
        claim::inspect_heartbeat_receipt(&dev, std::ffi::OsStr::new("null")),
        claim::HeartbeatReceiptEntry::Unsafe
    );
    std::fs::remove_dir_all(parent_path).expect("remove failure fixture");
    let (parent_path, _) = startup_heartbeat_fixture("receipt-renames");
    let repo_path = parent_path.join("repo");
    std::fs::create_dir(&repo_path).unwrap();
    std::fs::set_permissions(&repo_path, std::fs::Permissions::from_mode(0o700)).unwrap();
    let parent = open_parent(&parent_path);
    let repo = claim::open_heartbeat_directory_beneath(&parent, Path::new("repo")).unwrap();
    drop(claim::begin_heartbeat_receipt(&repo, expected).unwrap());
    let replace = |path: &Path, moved: &Path| {
        std::fs::rename(path, moved).unwrap();
        std::fs::create_dir(path).unwrap();
    };
    let decision =
        claim::heartbeat_receipt_retry_decision_with_hook(
            &repo,
            expected,
            |boundary| match boundary {
                "repo" => replace(&repo_path, &parent_path.join("repo-old")),
                "quarantine" => {
                    let old = parent_path.join("repo-old");
                    replace(&old.join("quarantine"), &old.join("quarantine-old"));
                }
                "handoff" => {
                    let old = parent_path.join("repo-old/quarantine-old");
                    replace(
                        &old.join("startup-heartbeat-handoffs"),
                        &old.join("handoff-old"),
                    );
                }
                _ => unreachable!(),
            },
        );
    assert_eq!(decision, Pending);
    std::fs::remove_dir_all(parent_path).expect("remove rename fixture");
}

#[test]
fn unsupported_platform_stale_recovery_is_fail_closed() {
    let source = include_str!("../../claim.rs");
    let fallback = source
        .split("#[cfg(not(target_os = \"linux\"))]\nfn quarantine_authoritative_stale_heartbeat")
        .nth(1)
        .expect("unsupported-platform recovery fallback");
    let fallback = fallback
        .split("fn release_stale_startup_labels")
        .next()
        .expect("fallback boundary");
    assert!(fallback.contains("Ok(false)"));
    assert!(!fallback.contains("startup_heartbeat_exists"));
}

#[test]
fn classify_startup_heartbeat_blocks_fresh_live_malformed_mismatched_and_remote_evidence() {
    use claim::StartupPidLiveness::{Dead, Live};
    let (directory, path) = startup_heartbeat_fixture("blocking");
    let worker = "host:user:rust:4242:nonce-a";
    let exact = startup_heartbeat_document(worker, 4242);
    let changed = |from, to| exact.replace(from, to);
    // A fresh heartbeat whose owner is dead is no longer blocking; it is a takeover,
    // covered by `a_fresh_heartbeat_whose_owner_is_dead_is_taken_over`.
    let mut cases = vec![("live", exact.clone(), 200, Live, true)];
    let mutations = r#"malformed|not-json
owner/repo|other/repo
"42"|"43"
host:user|other:user
feat/worker|feat/other
"pr":""|"pr":"17"
claim-a|claim-b
claimed|review
"ttl_seconds":10|"ttl_seconds":0
"pid":4242|"pid":0
:nonce-a"|:nonce-b"
"process_start":"1"|"process_start":"""#;
    cases.extend(mutations.lines().map(|row| {
        let (from, to) = row.split_once('|').expect("mutation row");
        let document = if from == "malformed" {
            to.into()
        } else {
            changed(from, to)
        };
        ("malformed or mismatch", document, 200, Dead, false)
    }));
    for (label, document, now, liveness, should_observe) in cases {
        std::fs::write(&path, document).expect("write heartbeat fixture");
        let mut observed = false;
        assert_eq!(
            claim::classify_startup_heartbeat(
                &path,
                expected_startup_heartbeat(worker),
                now,
                |_, _, _, _, _| {
                    observed = true;
                    liveness
                },
            ),
            claim::StartupHeartbeatClassification::Blocking,
            "{label}"
        );
        assert_eq!(observed, should_observe, "{label} liveness call");
    }
    std::fs::remove_dir_all(directory).expect("remove heartbeat fixture");
}

/// A dead owner does not hold its claim, whatever the clock says.
///
/// The TTL used to gate this probe, so a worker that died seconds ago kept its lease
/// for the full three hours. Worse, every crashed successor rewrote the heartbeat and
/// pushed the expiry another TTL out, so a crash loop could hold an issue hostage
/// indefinitely. Identity, not elapsed time, is what makes the takeover safe.
#[cfg(unix)]
#[test]
fn a_fresh_heartbeat_whose_owner_is_dead_is_taken_over() {
    let (directory, path) = startup_heartbeat_fixture("fresh-dead");
    let worker = "host:user:rust:4242:nonce-a";
    std::fs::write(&path, startup_heartbeat_document(worker, 4242)).expect("write fixture");
    let mut observed = false;

    let classified = claim::classify_startup_heartbeat(
        &path,
        expected_startup_heartbeat(worker),
        105,
        |_, _, _, _, _| {
            observed = true;
            claim::StartupPidLiveness::Dead
        },
    );

    assert!(
        matches!(
            classified,
            claim::StartupHeartbeatClassification::ExpiredDead(_)
        ),
        "a dead owner inside its TTL must still yield the claim: {classified:?}"
    );
    assert!(
        observed,
        "liveness must be probed without waiting for expiry"
    );
    std::fs::remove_dir_all(directory).expect("remove heartbeat fixture");
}

#[cfg(unix)]
#[test]
fn classify_startup_heartbeat_returns_snapshot_only_for_expired_dead_local_pid() {
    use std::os::unix::fs::MetadataExt;
    let (directory, path) = startup_heartbeat_fixture("expired-dead");
    let worker = "host:user:rust:4242:nonce-a";
    let document = startup_heartbeat_document(worker, 4242);
    std::fs::write(&path, &document).expect("write heartbeat fixture");
    let classified = claim::classify_startup_heartbeat(
        &path,
        expected_startup_heartbeat(worker),
        200,
        |worker, pid, _, _, _| {
            assert_eq!((worker, pid), ("host:user:rust:4242:nonce-a", 4242));
            claim::StartupPidLiveness::Dead
        },
    );
    let claim::StartupHeartbeatClassification::ExpiredDead(snapshot) = classified else {
        panic!("exact expired dead evidence was not reclaimable");
    };
    let metadata = std::fs::metadata(&path).expect("heartbeat metadata");
    assert_eq!(snapshot.file.document, document.as_bytes());
    assert_eq!(snapshot.file.identity.device, metadata.dev());
    assert_eq!(snapshot.file.identity.inode, metadata.ino());
    assert_eq!(snapshot.file.identity.length, metadata.len());
    assert_eq!(snapshot.file.identity.modified_seconds, metadata.mtime());
    assert_eq!(snapshot.file.identity.modified_nanos, metadata.mtime_nsec());
    std::fs::remove_dir_all(directory).expect("remove heartbeat fixture");
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn startup_heartbeat_process_identity_is_native_and_complete() {
    let identity = claim::startup_process_identity(std::process::id())
        .expect("observe startup heartbeat process identity");
    assert!(!identity.0.is_empty(), "host identity must be present");
    assert!(!identity.1.is_empty(), "boot identity must be present");
    assert!(!identity.2.is_empty(), "start identity must be present");
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn classify_startup_heartbeat_rejects_symlink_and_observes_current_pid_as_live() {
    use claim::StartupHeartbeatClassification::Blocking;

    let (directory, path) = startup_heartbeat_fixture("symlink-live");
    let target = directory.join("target.json");
    let host = std::env::var("HOSTNAME").unwrap_or_else(|_| "unknown-host".into());
    let user = std::env::var("USER").unwrap_or_else(|_| "unknown-user".into());
    let pid = std::process::id();
    let worker = format!("{host}:{user}:rust:{pid}:nonce-a");
    let identity = claim::startup_process_identity(pid).unwrap();
    let make_document = |worker| {
        startup_heartbeat_document(worker, pid)
            .replace("host-a", &identity.0)
            .replace("boot-a", &identity.1)
            .replace("start-a", &identity.2)
    };
    let document = make_document(&worker);
    std::fs::write(&target, &document).expect("write target heartbeat");
    std::os::unix::fs::symlink(&target, &path).expect("symlink heartbeat");
    let classify = |worker: &str| {
        let mut observed = false;
        let result = claim::classify_startup_heartbeat(
            &path,
            expected_startup_heartbeat(worker),
            200,
            |worker, pid, host, boot_id, process_start| {
                observed = true;
                claim::observe_local_startup_pid(worker, pid, host, boot_id, process_start)
            },
        );
        (result, observed)
    };
    assert_eq!(classify(&worker), (Blocking, false));
    std::fs::remove_file(&path).expect("remove symlink");
    std::fs::rename(&target, &path).expect("publish regular heartbeat");
    assert_eq!(classify(&worker), (Blocking, true));
    let remote_worker = format!("remote-{host}:{user}:rust:{pid}:nonce-a");
    std::fs::write(&path, make_document(&remote_worker)).expect("write remote heartbeat");
    assert_eq!(
        claim::observe_local_startup_pid(
            &remote_worker,
            pid,
            &identity.0,
            &identity.1,
            &identity.2,
        ),
        claim::StartupPidLiveness::Live
    );
    assert_eq!(classify(&remote_worker), (Blocking, true));
    std::fs::remove_dir_all(directory).expect("remove heartbeat fixture");
}

#[cfg(unix)]
#[test]
fn startup_heartbeat_fifo_path_reader_is_nonblocking() {
    let (directory, fifo) = startup_heartbeat_fixture("fifo-path");
    nix::unistd::mkfifo(&fifo, nix::sys::stat::Mode::from_bits_truncate(0o600)).unwrap();
    let read_path = fifo.clone();
    assert_fifo_reader_nonblocking(&fifo, move || {
        claim::read_regular_file_no_follow(&read_path)
    });
    assert_eq!(
        claim::classify_startup_heartbeat(
            &fifo,
            expected_startup_heartbeat("host:user:rust:4242:nonce-a"),
            200,
            |_, _, _, _, _| claim::StartupPidLiveness::Dead,
        ),
        claim::StartupHeartbeatClassification::Blocking
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[cfg(unix)]
#[test]
fn startup_heartbeat_fifo_at_reader_is_nonblocking() {
    let (directory, fifo) = startup_heartbeat_fixture("fifo-at");
    nix::unistd::mkfifo(&fifo, nix::sys::stat::Mode::from_bits_truncate(0o600)).unwrap();
    let root = std::fs::File::open(&directory).unwrap();
    let name = fifo.file_name().unwrap().to_owned();
    assert_fifo_reader_nonblocking(&fifo, move || {
        claim::read_regular_file_at_no_follow(&root, &name)
    });
    std::fs::remove_dir_all(directory).unwrap();
}

#[cfg(unix)]
#[test]
fn startup_heartbeat_fifo_timeout_recovery_is_bounded() {
    let (directory, fifo) = startup_heartbeat_fixture("fifo-timeout-recovery");
    nix::unistd::mkfifo(&fifo, nix::sys::stat::Mode::from_bits_truncate(0o600)).unwrap();
    let read_path = fifo.clone();
    let panic = std::panic::catch_unwind(|| {
        assert_fifo_reader_nonblocking(&fifo, move || {
            let mut file = std::fs::File::open(read_path)?;
            let mut payload = Vec::new();
            std::io::Read::read_to_end(&mut file, &mut payload)?;
            Err(std::io::Error::other("payload reader completed"))
        });
    })
    .expect_err("timeout recovery must retain the original failure");
    let message = panic
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| panic.downcast_ref::<&str>().copied())
        .expect("timeout recovery panic message");
    assert!(
        message.contains("heartbeat FIFO reader blocked: timed out waiting on channel"),
        "unexpected timeout recovery diagnostic: {message}"
    );
    std::fs::remove_dir_all(directory).unwrap();
}

/// A dead heartbeat belonging to a *different* worker must not be reclaimed as this
/// record's own prior generation while the record's own lease is still inside its TTL.
///
/// This is `heartbeat_prior::released_claim_acquires_over_a_dead_prior_generation_heartbeat`
/// with one field changed -- the record's `updated_at` moves from long-expired to now --
/// so the only discriminator under examination is whether the authoritative record has
/// itself been abandoned. Two distinct workers with a genuinely expired second heartbeat
/// is ordinary lease contention, and the lease-timeout path arbitrates it; the
/// prior-generation quarantine must keep its hands off the evidence until then.
#[cfg(target_os = "linux")]
#[test]
fn live_released_claim_declines_a_foreign_dead_heartbeat() {
    let _guard = lock_heartbeat_env();
    let (sandbox, _) = startup_heartbeat_fixture("live-released-foreign-heartbeat");
    let heartbeat_root = sandbox.join("heartbeats");
    let repo_key = super::super::super::autonomous::drain::repository_progress_key("owner/repo");
    let repo = heartbeat_root.join(repo_key);
    std::fs::create_dir_all(&repo).unwrap();
    for directory in [&heartbeat_root, &repo] {
        std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let host = std::fs::read_to_string("/proc/sys/kernel/hostname")
        .unwrap()
        .trim()
        .to_string();
    let boot = super::super::super::autonomous::current_boot_identity().unwrap();
    let nonce = claim::startup_heartbeat_nonce("owner/repo", 42, "foreign-claim");
    // Same shape as the dead-prior-generation fixture: ts 1 with a 10s ttl is long
    // expired, and pid 2147483647 on this host and boot is dead, so the snapshot
    // classifies ExpiredDead rather than Blocking.
    let document = format!(
        r#"{{"repo":"owner/repo","issue":"42","worker_id":"foreign-worker","branch":"feat/worker","pr":"","claim_id":"foreign-claim","step":"claimed","ts":1,"ttl_seconds":10,"pid":2147483647,"nonce":"{nonce}","host":"{host}","boot_id":"{boot}","process_start":"1"}}"#
    );
    let foreign = repo.join("42.json");
    std::fs::write(&foreign, &document).unwrap();
    std::fs::set_permissions(&foreign, std::fs::Permissions::from_mode(0o600)).unwrap();

    let now = crate::commands::claim::utc_now_iso().expect("current timestamp");
    let released = autospec_core::claim::RunStateRecord::new(
        "owner/repo",
        42,
        "predecessor-worker",
        "released",
        "feat/worker",
        "",
        "retryable_released",
        Vec::new(),
        &now,
        &now,
        10_800,
    )
    .with_claim_id("predecessor-claim");

    let previous = std::env::var_os("AUTOSPEC_HEARTBEAT_DIR");
    std::env::set_var("AUTOSPEC_HEARTBEAT_DIR", &heartbeat_root);

    let authorized = claim::heartbeat_predecessor::expired_prior_generation_heartbeat(
        "owner/repo",
        42,
        &released,
    );

    match previous {
        Some(value) => std::env::set_var("AUTOSPEC_HEARTBEAT_DIR", value),
        None => std::env::remove_var("AUTOSPEC_HEARTBEAT_DIR"),
    }

    assert!(
        authorized
            .expect("classification must not fail")
            .is_none(),
        "a distinct worker's expired heartbeat was authorized as this record's own prior generation"
    );
    assert_eq!(
        std::fs::read_to_string(&foreign).unwrap(),
        document,
        "the foreign worker's heartbeat evidence was altered"
    );
    std::fs::remove_dir_all(sandbox).unwrap();
}
