// claim tests: heartbeat / prior — 5 cases.
//
// Split out of tests.rs; see the note in that file.

#[cfg(unix)]
use super::support::STARTUP_HEARTBEAT_ENV;
use super::support::{expected_startup_heartbeat, startup_heartbeat_fixture};
use crate::commands::claim;
#[cfg(unix)]
use autospec_core::claim::RunStateRecord;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

#[cfg(target_os = "linux")]
#[test]
fn prior_generation_authorization_rejects_heartbeat_replacement() {
    let _guard = STARTUP_HEARTBEAT_ENV.lock().expect("heartbeat env");
    let (sandbox, _) = startup_heartbeat_fixture("prior-generation-race");
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
    let document = |worker: &str, claim: &str| {
        let nonce = claim::startup_heartbeat_nonce("owner/repo", 42, claim);
        format!(
            r#"{{"repo":"owner/repo","issue":"42","worker_id":"{worker}","branch":"feat/worker","pr":"","claim_id":"{claim}","step":"claimed","ts":1,"ttl_seconds":10,"pid":2147483647,"nonce":"{nonce}","host":"{host}","boot_id":"{boot}","process_start":"1"}}"#
        )
    };
    let source = repo.join("42.json");
    std::fs::write(&source, document("prior-worker", "prior-claim")).unwrap();
    std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o600)).unwrap();
    let file = claim::read_regular_file(std::fs::File::open(&source).unwrap()).unwrap();
    let authorized = claim::StartupHeartbeatSnapshot {
        evidence: claim::parse_startup_heartbeat(&file.document).unwrap(),
        file,
    };
    let record = RunStateRecord::new(
        "owner/repo",
        42,
        "current-worker",
        "claimed",
        "feat/worker",
        "",
        "heartbeat-pending:none",
        Vec::new(),
        "2000-01-01T00:00:00Z",
        "2000-01-01T00:00:00Z",
        1,
    )
    .with_claim_id("current-claim");
    let previous = std::env::var_os("AUTOSPEC_HEARTBEAT_DIR");
    std::env::set_var("AUTOSPEC_HEARTBEAT_DIR", &heartbeat_root);
    let current = document("current-worker", "current-claim");
    let result = claim::quarantine_authoritative_stale_heartbeat(
        "owner/repo",
        42,
        &record,
        Some(&authorized),
        &mut || {
            std::fs::write(&source, &current).unwrap();
            Ok(())
        },
    );
    assert!(result
        .unwrap_err()
        .message
        .contains("changed before quarantine"));
    assert_eq!(std::fs::read_to_string(&source).unwrap(), current);
    std::fs::write(&source, document("prior-worker", "prior-claim")).unwrap();
    let file = claim::read_regular_file(std::fs::File::open(&source).unwrap()).unwrap();
    let authorized = claim::StartupHeartbeatSnapshot {
        evidence: claim::parse_startup_heartbeat(&file.document).unwrap(),
        file,
    };
    assert!(claim::quarantine_authoritative_stale_heartbeat(
        "owner/repo",
        42,
        &record,
        Some(&authorized),
        &mut || Ok(()),
    )
    .unwrap());
    assert!(!source.exists());
    let retained = claim::expired_prior_generation_heartbeat("owner/repo", 42, &record)
        .unwrap()
        .expect("retained authorization");
    assert!(claim::quarantine_authoritative_stale_heartbeat(
        "owner/repo",
        42,
        &record,
        Some(&retained),
        &mut || Ok(()),
    )
    .unwrap());
    match previous {
        Some(value) => std::env::set_var("AUTOSPEC_HEARTBEAT_DIR", value),
        None => std::env::remove_var("AUTOSPEC_HEARTBEAT_DIR"),
    }
    std::fs::remove_dir_all(sandbox).unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn heartbeat_directory_openat2() {
    use nix::fcntl::{open, OFlag, ResolveFlag};
    use nix::sys::stat::fstat;
    use std::os::unix::fs::{symlink, MetadataExt};

    let open_parent = |path: &Path| {
        std::fs::File::from(
            open(
                path,
                OFlag::O_PATH | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
                nix::sys::stat::Mode::empty(),
            )
            .expect("open trusted parent"),
        )
    };
    let make_child = |parent: &Path| {
        let child = parent.join("heartbeat");
        std::fs::create_dir(&child).expect("heartbeat child");
        std::fs::set_permissions(&child, std::fs::Permissions::from_mode(0o700))
            .expect("private heartbeat child");
        child
    };

    let (trusted, _) = startup_heartbeat_fixture("openat2-trusted");
    let trusted_child = make_child(&trusted);
    let parent = open_parent(&trusted);
    let opened = claim::open_heartbeat_directory_beneath(&parent, Path::new("heartbeat"))
        .expect("open descendant");
    let expected = std::fs::metadata(&trusted_child).expect("descendant metadata");
    let observed = fstat(&opened).expect("opened metadata");
    assert_eq!(
        (observed.st_dev, observed.st_ino),
        (expected.dev(), expected.ino())
    );

    let (outside, _) = startup_heartbeat_fixture("openat2-outside");
    make_child(&outside);
    symlink(&outside, trusted.join("parent-link")).expect("descendant parent symlink");
    assert!(
        claim::open_heartbeat_directory_beneath(&parent, Path::new("parent-link/heartbeat"))
            .is_err()
    );
    assert!(claim::open_heartbeat_directory_beneath(&parent, Path::new("")).is_err());
    assert!(claim::open_heartbeat_directory_beneath(&parent, &trusted_child).is_err());
    assert!(claim::open_heartbeat_directory_beneath(&parent, Path::new("../escape")).is_err());
    assert!(claim::heartbeat_openat2_resolve_flags().contains(ResolveFlag::RESOLVE_NO_XDEV));

    let (drifting, _) = startup_heartbeat_fixture("openat2-drifting");
    make_child(&drifting);
    let drifting_parent = open_parent(&drifting);
    let drift = claim::open_heartbeat_directory_beneath_with_hook(
        &drifting_parent,
        Path::new("heartbeat"),
        || {
            std::fs::set_permissions(&drifting, std::fs::Permissions::from_mode(0o755))
                .expect("drift parent mode");
        },
    );
    assert!(drift.is_err());
    std::fs::set_permissions(&drifting, std::fs::Permissions::from_mode(0o700))
        .expect("restore parent mode");

    let (replaceable, _) = startup_heartbeat_fixture("openat2-replaceable");
    let replaceable_child = make_child(&replaceable);
    let original_inode = std::fs::metadata(&replaceable_child)
        .expect("original child metadata")
        .ino();
    let (replacement, _) = startup_heartbeat_fixture("openat2-replacement");
    let replacement_child = make_child(&replacement);
    let replacement_inode = std::fs::metadata(&replacement_child)
        .expect("replacement child metadata")
        .ino();
    let anchored_parent = open_parent(&replaceable);
    let anchored = replaceable.with_extension("anchored");
    let opened = claim::open_heartbeat_directory_beneath_with_hook(
        &anchored_parent,
        Path::new("heartbeat"),
        || {
            std::fs::rename(&replaceable, &anchored).expect("move trusted directory");
            std::fs::rename(&replacement, &replaceable).expect("install replacement directory");
        },
    )
    .expect("open anchored original child");
    let opened_inode = fstat(&opened).expect("opened child metadata").st_ino;
    assert_eq!(opened_inode, original_inode);
    assert_ne!(opened_inode, replacement_inode);
    std::fs::rename(&replaceable, &replacement).expect("remove replacement directory");
    std::fs::rename(&anchored, &replaceable).expect("restore trusted directory");

    let displaced_child = replaceable.join("displaced");
    let swapped_child = claim::open_heartbeat_directory_beneath_with_hook(
        &anchored_parent,
        Path::new("heartbeat"),
        || {
            std::fs::rename(&replaceable_child, &displaced_child).expect("displace trusted child");
            std::fs::rename(&replacement_child, &replaceable_child)
                .expect("install replacement child");
        },
    );
    assert!(swapped_child.is_err(), "changed child binding was accepted");
    std::fs::rename(&replaceable_child, &replacement_child).expect("remove replacement child");
    std::fs::rename(&displaced_child, &replaceable_child).expect("restore trusted child");

    std::fs::remove_dir_all(trusted).expect("remove trusted fixture");
    std::fs::remove_dir_all(outside).expect("remove outside fixture");
    std::fs::remove_dir_all(drifting).expect("remove drift fixture");
    std::fs::remove_dir_all(replaceable).expect("remove replacement fixture");
    std::fs::remove_dir_all(replacement).expect("remove rename-in fixture");
}

#[cfg(unix)]
#[test]
fn startup_heartbeat_portable_unix() {
    use nix::fcntl::{open, OFlag};
    use nix::sys::stat::{fstat, Mode};
    use std::os::unix::fs::{symlink, MetadataExt};

    let make_private_child = |parent: &Path| {
        let child = parent.join("heartbeat");
        std::fs::create_dir(&child).expect("heartbeat child");
        std::fs::set_permissions(&child, std::fs::Permissions::from_mode(0o700))
            .expect("private heartbeat child");
        child
    };
    let open_parent = |path: &Path| {
        std::fs::File::from(
            open(
                path,
                OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
                Mode::empty(),
            )
            .expect("open trusted parent"),
        )
    };

    let (trusted, _) = startup_heartbeat_fixture("portable-unix");
    let trusted_child = make_private_child(&trusted);
    let parent = open_parent(&trusted);
    let opened = claim::open_heartbeat_directory_portable_unix_with_hook(
        &parent,
        Path::new("heartbeat"),
        || {},
    )
    .expect("open one-component descendant");
    let expected = std::fs::metadata(&trusted_child).expect("descendant metadata");
    let observed = fstat(&opened).expect("opened metadata");
    assert_eq!(
        (observed.st_dev as u64, observed.st_ino),
        (expected.dev(), expected.ino())
    );

    let (replacement, _) = startup_heartbeat_fixture("portable-unix-replacement");
    let replacement_child = make_private_child(&replacement);
    let displaced = trusted.join("displaced");
    let swapped = claim::open_heartbeat_directory_portable_unix_with_hook(
        &parent,
        Path::new("heartbeat"),
        || {
            std::fs::rename(&trusted_child, &displaced).expect("displace trusted child");
            std::fs::rename(&replacement_child, &trusted_child).expect("install replacement child");
        },
    );
    assert!(swapped.is_err(), "changed name binding was accepted");
    std::fs::remove_dir(&trusted_child).expect("remove replacement child");
    symlink(&displaced, &trusted_child).expect("install descendant symlink");
    assert!(
        claim::open_heartbeat_directory_portable_unix_with_hook(
            &parent,
            Path::new("heartbeat"),
            || {},
        )
        .is_err(),
        "descendant symlink was accepted"
    );
    assert!(
        claim::open_heartbeat_directory_portable_unix_with_hook(
            &parent,
            Path::new("nested/heartbeat"),
            || {},
        )
        .is_err(),
        "multi-component descendant was accepted"
    );

    std::fs::remove_dir_all(trusted).expect("remove trusted fixture");
    std::fs::remove_dir_all(replacement).expect("remove replacement fixture");
}

#[test]
fn classify_startup_heartbeat_marks_missing_evidence_absent() {
    let (directory, path) = startup_heartbeat_fixture("absent");
    let classified = claim::classify_startup_heartbeat(
        &path,
        expected_startup_heartbeat("worker-a"),
        200,
        |_, _, _, _, _| claim::StartupPidLiveness::Dead,
    );
    assert_eq!(classified, claim::StartupHeartbeatClassification::Absent);
    std::fs::remove_dir_all(directory).expect("remove heartbeat fixture");
}

#[cfg(unix)]
#[test]
fn startup_heartbeat_process_identity() {
    use claim::{StartupHeartbeatClassification::ExpiredDead, StartupPidLiveness};

    let _environment = STARTUP_HEARTBEAT_ENV.lock().unwrap();
    let (directory, _) = startup_heartbeat_fixture("process-identity");
    let root = directory.join(".autospec/process-heartbeats");
    let old_root = std::env::var_os("AUTOSPEC_HEARTBEAT_DIR");
    let old_ttl = std::env::var_os("AUTOSPEC_CLAIM_LEASE_SECONDS");
    std::env::set_var("AUTOSPEC_HEARTBEAT_DIR", &root);
    std::env::set_var("AUTOSPEC_CLAIM_LEASE_SECONDS", "0");
    let path = root
        .join(super::super::super::autonomous::drain::repository_progress_key("owner/repo"))
        .join("42.json");
    let publish = |claim_id, session_id| {
        claim::write_startup_heartbeat(
            "owner/repo",
            42,
            "worker-a",
            "feat/worker",
            claim_id,
            Some(session_id),
        )
        .unwrap();
        claim::parse_startup_heartbeat(&std::fs::read(&path).unwrap())
            .unwrap()
            .nonce
    };
    let first_nonce = publish("claim-a", "session-a");
    assert_eq!(publish("claim-a", "session-a"), first_nonce);
    std::fs::remove_file(&path).unwrap();
    assert_ne!(publish("claim-b", "session-b"), first_nonce);
    let mut nonces = Vec::new();
    let mut last = None;
    for worker in ["worker-a", "opaque worker:with/slash"] {
        std::fs::remove_file(&path).unwrap();
        claim::write_startup_heartbeat("owner/repo", 42, worker, "feat/worker", "claim-a", None)
            .unwrap();
        let document = std::fs::read(&path).unwrap();
        let evidence = claim::parse_startup_heartbeat(&document).unwrap();
        assert_eq!(evidence.worker_id, worker);
        assert_eq!(evidence.pid, std::process::id());
        assert_eq!(evidence.ttl_seconds, 1);
        assert!(!evidence.nonce.is_empty());
        assert_eq!(
            claim::observe_local_startup_pid(
                worker,
                evidence.pid,
                &evidence.host,
                &evidence.boot_id,
                &evidence.process_start,
            ),
            StartupPidLiveness::Live
        );
        assert!(matches!(
            claim::classify_startup_heartbeat(
                &path,
                expected_startup_heartbeat(worker),
                evidence.ts + evidence.ttl_seconds + 1,
                |observed, pid, host, boot_id, process_start| {
                    assert_eq!((observed, pid), (worker, evidence.pid));
                    assert_eq!(
                        (host, boot_id, process_start),
                        (
                            evidence.host.as_str(),
                            evidence.boot_id.as_str(),
                            evidence.process_start.as_str()
                        )
                    );
                    StartupPidLiveness::Dead
                },
            ),
            ExpiredDead(_)
        ));
        nonces.push(evidence.nonce.clone());
        last = Some((worker, document, evidence));
    }
    assert_eq!(nonces[0], nonces[1]);

    let (worker, document, evidence) = last.unwrap();
    let document = String::from_utf8(document).unwrap();
    let pid = format!("\"pid\":{}", evidence.pid);
    let nonce = format!("\"nonce\":\"{}\"", evidence.nonce);
    for malformed in [
        document.replace(&pid, "\"pid\":0"),
        document.replace(&pid, "\"pid\":\"bad\""),
        document.replace(&nonce, "\"nonce\":\"\""),
        document.replace(&nonce, &format!("\"nonce\":\"{}\"", "f".repeat(64))),
        document.replace(worker, "different-worker"),
    ] {
        std::fs::write(&path, malformed).unwrap();
        assert_eq!(
            claim::classify_startup_heartbeat(
                &path,
                expected_startup_heartbeat(worker),
                evidence.ts + evidence.ttl_seconds + 1,
                |_, _, _, _, _| StartupPidLiveness::Dead,
            ),
            claim::StartupHeartbeatClassification::Blocking
        );
    }
    for (key, value) in [
        ("AUTOSPEC_HEARTBEAT_DIR", old_root),
        ("AUTOSPEC_CLAIM_LEASE_SECONDS", old_ttl),
    ] {
        match value {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }
    }
    std::fs::remove_dir_all(directory).unwrap();
}
