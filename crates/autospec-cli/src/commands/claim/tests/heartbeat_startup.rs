// claim tests: heartbeat / startup — 4 cases.
//
// Split out of tests.rs; see the note in that file.

#[cfg(target_os = "linux")]
use super::support::inject_heartbeat_boundary;
use super::support::{startup_heartbeat_fixture, STARTUP_HEARTBEAT_ENV};
use crate::commands::claim;
#[cfg(target_os = "linux")]
use std::os::unix::fs::PermissionsExt;
#[cfg(target_os = "linux")]
use std::process::Command;

#[cfg(not(target_os = "linux"))]
#[test]
fn startup_heartbeat_process_identity_is_stable_for_the_current_process() {
    let first =
        claim::startup_process_identity(std::process::id()).expect("portable process identity");
    let second = claim::startup_process_identity(std::process::id())
        .expect("stable portable process identity");

    assert_eq!(first, second);
    assert!(!first.0.is_empty() && !first.1.is_empty() && !first.2.is_empty());
    assert!(first.2.parse::<u64>().is_ok());
}

#[cfg(not(target_os = "linux"))]
#[test]
fn portable_publication_is_idempotent_but_rejects_another_generation() {
    let _guard = STARTUP_HEARTBEAT_ENV.lock().expect("heartbeat env");
    let (sandbox, _) = startup_heartbeat_fixture("portable-publication");
    let heartbeat_root = sandbox.join("heartbeats");
    let previous = std::env::var_os("AUTOSPEC_HEARTBEAT_DIR");
    unsafe { std::env::set_var("AUTOSPEC_HEARTBEAT_DIR", &heartbeat_root) };
    let publish = |claim_id| {
        claim::write_startup_heartbeat(
            "owner/repo",
            42,
            "worker-a",
            "feat/worker",
            claim_id,
            Some("session-a"),
        )
    };

    publish("claim-a").expect("initial publication");
    publish("claim-a").expect("idempotent replay");
    let error = publish("claim-b").expect_err("generation conflict");
    assert_eq!(error.message, "heartbeat publication target conflicts");

    match previous {
        Some(value) => unsafe { std::env::set_var("AUTOSPEC_HEARTBEAT_DIR", value) },
        None => unsafe { std::env::remove_var("AUTOSPEC_HEARTBEAT_DIR") },
    }
    std::fs::remove_dir_all(sandbox).expect("remove heartbeat fixture");
}

#[cfg(target_os = "linux")]
#[test]
fn startup_heartbeat_atomic_publication() {
    use claim::HeartbeatPublicationDurability::{Durable, Unconfirmed};
    use claim::HeartbeatPublicationFailure::{PostCommit, PreCommit};
    use nix::fcntl::{open, OFlag};
    use nix::sys::stat::{umask, Mode};
    use std::io::Read;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    const CHILD: &str = "AUTOSPEC_TEST_ATOMIC_HEARTBEAT_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "commands::claim::tests::startup_heartbeat_atomic_publication",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }

    let (fixture, issue) = startup_heartbeat_fixture("atomic-publication");
    let directory = open(
        &fixture,
        OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW,
        Mode::empty(),
    )
    .unwrap();
    let previous = umask(Mode::from_bits_truncate(0o777));
    let attempt = |name: &str, failed: Option<&str>| {
        claim::publish_private_heartbeat_file(
            &directory,
            name,
            b"heartbeat\n",
            "test",
            &mut |_, boundary| {
                if failed == Some("hardlink") && boundary == "revalidate" {
                    let peer = fixture.join(format!("{name}.peer"));
                    std::fs::hard_link(fixture.join(name), peer).unwrap();
                    Ok(())
                } else if failed == Some(boundary) {
                    Err(claim::CommandFailure::diagnostic(
                        "injected persistence failure",
                    ))
                } else {
                    Ok(())
                }
            },
        )
    };
    for name in ["", ".", "..", "/tmp/x", "a/b", "a/", "./a"] {
        assert!(matches!(attempt(name, None), Err(PreCommit(_))));
        assert_eq!(std::fs::read_dir(&fixture).unwrap().count(), 0);
    }
    for failed in ["chmod", "write", "file-fsync", "before-link"] {
        assert!(matches!(
            attempt("42.json", Some(failed)),
            Err(PreCommit(_))
        ));
        assert!(!issue.exists());
    }
    let outside = fixture.join("outside");
    std::fs::write(&outside, b"caller-owned").unwrap();
    std::fs::set_permissions(&outside, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::hard_link(&outside, &issue).unwrap();
    assert!(matches!(attempt("42.json", None), Err(PreCommit(_))));
    assert_eq!(std::fs::read(&outside).unwrap(), b"caller-owned");
    assert_eq!(std::fs::metadata(&outside).unwrap().nlink(), 2);
    std::fs::remove_file(&issue).unwrap();
    nix::unistd::mkfifo(&issue, Mode::from_bits_truncate(0o600)).unwrap();
    std::fs::set_permissions(&issue, std::fs::Permissions::from_mode(0o600)).unwrap();
    let mut fifo = std::fs::File::from(
        open(&issue, OFlag::O_RDONLY | OFlag::O_NONBLOCK, Mode::empty()).unwrap(),
    );
    let before = fifo.metadata().unwrap();
    let fifo_directory = std::fs::File::from(directory.try_clone().unwrap());
    let (send, receive) = std::sync::mpsc::channel();
    let publisher = std::thread::spawn(move || {
        let result = claim::publish_private_heartbeat_file(
            &fifo_directory,
            "42.json",
            b"heartbeat\n",
            "test",
            &mut |_, _| Ok(()),
        );
        send.send(result).unwrap();
    });
    assert!(matches!(
        receive.recv_timeout(std::time::Duration::from_secs(2)),
        Ok(Err(PreCommit(_)))
    ));
    publisher.join().unwrap();
    let after = std::fs::symlink_metadata(&issue).unwrap();
    let identity = |meta: &std::fs::Metadata| (meta.dev(), meta.ino(), meta.mode());
    assert_eq!(identity(&before), identity(&after));
    assert_eq!(fifo.read(&mut [0]).unwrap(), 0);
    std::fs::remove_file(&issue).unwrap();
    let session = fixture.join("session.json");
    let issue_publication = attempt("42.json", None).unwrap();
    let session_publication = attempt("session.json", None).unwrap();
    umask(previous);
    for (path, publication) in [(&issue, issue_publication), (&session, session_publication)] {
        assert_eq!(publication.durability, Durable);
        let metadata = std::fs::metadata(path).unwrap();
        assert_eq!(metadata.permissions().mode() & 0o7777, 0o600);
        assert_eq!(
            (metadata.dev(), metadata.ino()),
            (publication.device, publication.inode)
        );
        let held = publication.file.metadata().unwrap();
        assert_eq!((held.dev(), held.ino()), (metadata.dev(), metadata.ino()));
        assert_eq!(held.nlink(), 1);
    }

    for (name, failed, expected) in [
        ("pending.json", "directory-fsync", Unconfirmed),
        ("revalidate-error.json", "revalidate", Unconfirmed),
        ("extra-link.json", "hardlink", Unconfirmed),
    ] {
        let error = attempt(name, Some(failed)).unwrap_err();
        let PostCommit {
            publication,
            error: _,
        } = error
        else {
            panic!("post-link failure was reported as pre-commit");
        };
        assert_eq!(publication.durability, expected);
        let metadata = std::fs::metadata(fixture.join(name)).unwrap();
        assert_eq!(
            (metadata.dev(), metadata.ino()),
            (publication.device, publication.inode)
        );
    }

    let root = fixture.join("transaction-root");
    let repo = root.join(crate::commands::autonomous::drain::repository_progress_key(
        "owner/repo",
    ));
    let sessions = repo.join("sessions");
    std::fs::create_dir_all(&sessions).unwrap();
    // SAFETY: single-threaded test setup; no other thread reads the environment here.
    unsafe { std::env::set_var("AUTOSPEC_HEARTBEAT_DIR", &root) };
    let issue = repo.join("42.json");
    let session = sessions.join("73657373696f6e2d61.json");
    let write = || {
        claim::write_startup_heartbeat(
            "owner/repo",
            42,
            "worker-a",
            "feat/worker",
            "claim-a",
            Some("session-a"),
        )
    };
    let caller = fixture.join("caller-owned-heartbeat");
    std::fs::write(&caller, b"caller-owned").unwrap();
    std::fs::hard_link(&caller, &issue).unwrap();
    assert!(write().is_err());
    assert!(std::fs::read(&caller).unwrap() == b"caller-owned" && !session.exists());

    let prepared = b"{\"issue\":\"43\",\"branch\":\"feat/worker\",\"step\":\"claimed\",\"ts\":1,\"pr\":\"\",\"repo\":\"owner/repo\",\"worker_id\":\"worker-a\",\"claim_id\":\"claim-b\",\"session_id\":\"session-b\"}\n";
    let attempt = |issue, session, document: &[u8], failed| {
        claim::publish_startup_heartbeat_transaction_with_hook(
            &root,
            "owner/repo",
            issue,
            Some(session),
            document,
            &mut |role, boundary| {
                ((role, boundary) != failed)
                    .then_some(())
                    .ok_or_else(|| claim::CommandFailure::diagnostic("injected failure"))
            },
        )
    };
    assert!(attempt(43, "session-b", prepared, ("session", "before-link")).is_err());
    assert!(!repo.join("43.json").exists() && !sessions.join("73657373696f6e2d62.json").exists());

    std::fs::remove_file(&issue).unwrap();
    let transaction_umask = umask(Mode::from_bits_truncate(0o777));
    write().unwrap();
    umask(transaction_umask);
    assert_eq!(
        [&issue, &session]
            .map(|path| { std::fs::metadata(path).unwrap().permissions().mode() & 0o7777 }),
        [0o600; 2]
    );
    let expected = std::fs::read(&issue).unwrap();
    let mut stable_drift: serde_json::Value = serde_json::from_slice(&expected).unwrap();
    stable_drift["nonce"] = "foreign-generation".into();
    let stable_drift = serde_json::to_vec(&stable_drift).unwrap();
    let replacement = repo.join("replacement");
    let chmod =
        |mode| std::fs::set_permissions(&issue, std::fs::Permissions::from_mode(mode)).unwrap();
    for mutation in ["rename", "overwrite", "chmod"] {
        std::fs::write(&issue, &expected).unwrap();
        chmod(0o600);
        std::fs::remove_file(&session).unwrap();
        std::fs::write(&replacement, b"foreign").unwrap();
        let result = claim::publish_startup_heartbeat_transaction_with_hook(
            &root,
            "owner/repo",
            42,
            Some("session-a"),
            &expected,
            &mut |role, boundary| {
                if (role, boundary) == ("session", "directory-fsync") {
                    match mutation {
                        "rename" => std::fs::rename(&replacement, &issue).unwrap(),
                        "overwrite" => std::fs::write(&issue, &stable_drift).unwrap(),
                        _ => chmod(0o640),
                    }
                }
                Ok(())
            },
        );
        assert!(result.is_err(), "{mutation}");
    }
}

#[cfg(target_os = "linux")]
#[test]
fn heartbeat_root_parent_bootstrap_and_migration_are_durable() {
    use std::os::unix::fs::symlink;

    let (fixture, _) = startup_heartbeat_fixture("root-parent-bootstrap");
    let parent = fixture.join(".autospec");
    let root = parent.join("process-heartbeats");
    let mut syncs = Vec::new();
    claim::prepare_heartbeat_root_parent_with_hook(&root, |boundary| {
        syncs.push(boundary);
        Ok(())
    })
    .unwrap();
    assert_eq!(
        std::fs::metadata(&parent).unwrap().permissions().mode() & 0o7777,
        0o700
    );
    assert_eq!(
        syncs,
        [
            "chmod",
            "parent",
            "before-publish",
            "ancestor-fsync",
            "ancestor"
        ]
    );

    std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o775)).unwrap();
    syncs.clear();
    claim::prepare_heartbeat_root_parent_with_hook(&root, |boundary| {
        syncs.push(boundary);
        Ok(())
    })
    .unwrap();
    assert_eq!(
        std::fs::metadata(&parent).unwrap().permissions().mode() & 0o7777,
        0o700
    );
    assert_eq!(syncs, ["chmod", "parent", "ancestor-fsync", "ancestor"]);

    std::fs::remove_dir(&parent).unwrap();
    let outside = fixture.join("outside");
    std::fs::create_dir(&outside).unwrap();
    let outside_mode = std::fs::metadata(&outside).unwrap().permissions().mode();
    symlink(&outside, &parent).unwrap();
    assert!(claim::prepare_heartbeat_root_parent_with_hook(&root, |_| Ok(())).is_err());
    assert_eq!(
        std::fs::metadata(&outside).unwrap().permissions().mode(),
        outside_mode
    );
}

#[cfg(target_os = "linux")]
#[test]
fn startup_heartbeat_restrictive_umask() {
    use nix::sys::stat::{umask, Mode};
    use std::os::unix::fs::MetadataExt;

    const CHILD: &str = "AUTOSPEC_TEST_RESTRICTIVE_UMASK_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "commands::claim::tests::startup_heartbeat_restrictive_umask",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "child stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let (fixture, _) = startup_heartbeat_fixture("restrictive-umask");
    let parent = fixture.join(".autospec");
    let root = parent.join("process-heartbeats");
    let assert_no_staging = || {
        assert!(std::fs::read_dir(&fixture).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".autospec-heartbeat-stage-")
        }));
    };
    let previous = umask(Mode::from_bits_truncate(0o777));
    let failed = claim::prepare_heartbeat_root_parent_with_hook(&root, |boundary| {
        inject_heartbeat_boundary(boundary, "chmod", "injected chmod failure")
    });
    umask(previous);

    assert!(failed.is_err());
    assert!(!parent.exists());
    assert_no_staging();
    let failed = claim::prepare_heartbeat_root_parent_with_hook(&root, |boundary| {
        inject_heartbeat_boundary(boundary, "parent", "injected fsync failure")
    });
    assert!(failed.is_err());
    assert!(!parent.exists());
    assert_no_staging();
    claim::prepare_heartbeat_root_parent_with_hook(&root, |_| Ok(())).unwrap();
    assert_eq!(
        std::fs::metadata(&parent).unwrap().permissions().mode() & 0o7777,
        0o700
    );

    std::fs::remove_dir(&parent).unwrap();
    let replacement_identity = std::cell::Cell::new((0, 0));
    let raced = claim::prepare_heartbeat_root_parent_with_hook(&root, |boundary| {
        if boundary == "before-publish" {
            std::fs::create_dir(&parent).unwrap();
            std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o750)).unwrap();
            let metadata = std::fs::metadata(&parent).unwrap();
            replacement_identity.set((metadata.ino(), metadata.mode()));
        }
        Ok(())
    });
    assert!(raced.is_err());
    let replacement = std::fs::metadata(&parent).unwrap();
    assert_eq!(
        (replacement.ino(), replacement.mode()),
        replacement_identity.get()
    );
    assert_no_staging();

    std::fs::remove_dir(&parent).unwrap();
    let fail_ancestor_once = std::cell::Cell::new(true);
    let ancestor_attempts = std::cell::Cell::new(0);
    let mut ancestor_failure = |boundary| {
        if boundary == "ancestor-fsync" {
            ancestor_attempts.set(ancestor_attempts.get() + 1);
            if fail_ancestor_once.replace(false) {
                return Err(claim::CommandFailure::diagnostic(
                    "injected ancestor fsync failure",
                ));
            }
        }
        Ok(())
    };
    assert!(claim::prepare_heartbeat_root_parent_with_hook(&root, &mut ancestor_failure).is_err());
    assert!(
        parent.is_dir(),
        "published parent remains pending durability"
    );
    claim::prepare_heartbeat_root_parent_with_hook(&root, &mut ancestor_failure).unwrap();
    assert_eq!(ancestor_attempts.get(), 2);

    std::fs::remove_dir(&parent).unwrap();
    let cleanup_failure = claim::prepare_heartbeat_root_parent_with_hook(&root, |boundary| {
        if boundary == "parent" {
            let staging = std::fs::read_dir(&fixture)
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .find(|path| {
                    path.file_name()
                        .unwrap()
                        .to_string_lossy()
                        .starts_with(".autospec-heartbeat-stage-")
                })
                .unwrap();
            std::fs::write(staging.join("block-cleanup"), "occupied").unwrap();
            return Err(claim::CommandFailure::diagnostic("injected staged failure"));
        }
        Ok(())
    })
    .unwrap_err();
    assert!(cleanup_failure
        .message
        .contains("could not remove staged heartbeat root parent"));
}

#[cfg(target_os = "linux")]
#[test]
fn retryable_release_requires_exact_heartbeat_evidence() {
    let _guard = STARTUP_HEARTBEAT_ENV.lock().expect("heartbeat env");
    let (root, _) = startup_heartbeat_fixture("released-missing");
    let heartbeat_root = root.join("heartbeats");
    std::fs::create_dir(&heartbeat_root).unwrap();
    std::fs::set_permissions(&heartbeat_root, std::fs::Permissions::from_mode(0o700)).unwrap();
    let previous = std::env::var_os("AUTOSPEC_HEARTBEAT_DIR");
    std::env::set_var("AUTOSPEC_HEARTBEAT_DIR", heartbeat_root);
    let identity = claim::ClaimMutationIdentity {
        repo: "owner/repo",
        issue: 42,
        worker_id: "worker-a",
        branch: "feat/worker",
        claim_id: "claim-a",
    };
    let result = claim::retire_released_startup_heartbeat(identity);
    match previous {
        Some(value) => std::env::set_var("AUTOSPEC_HEARTBEAT_DIR", value),
        None => std::env::remove_var("AUTOSPEC_HEARTBEAT_DIR"),
    }
    std::fs::remove_dir_all(root).unwrap();
    result.expect_err("missing issue evidence must keep terminal preparation retryable");
}
