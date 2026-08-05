use super::{
    advance_claim_ref_in, claim_settle_millis, create_claim_ref_commit,
    lifecycle_claim_evidence_from_record, private_claim_git_dir_in, publish_session_binding,
    read_claim_ref_in, validated_claim_remote, ClaimRefAdvance, SessionBindingIdentity,
};
use autospec_core::autonomous_lifecycle::{
    ClaimBranch, ClaimContext, ClaimEvidence, IssueNumber, LeaseFreshness, RepositoryScope,
    WorkerId,
};
use autospec_core::claim::{ExecutorResultEvidence, RemoteComment, RunStateRecord};
#[cfg(unix)]
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Barrier, Mutex};

static BRIDGE_TRANSITION_ENV: Mutex<()> = Mutex::new(());
static STARTUP_HEARTBEAT_ENV: Mutex<()> = Mutex::new(());

fn startup_heartbeat_fixture(label: &str) -> (PathBuf, PathBuf) {
    let directory = std::env::temp_dir().join(format!(
        "autospec-startup-heartbeat-{label}-{}-{}",
        std::process::id(),
        super::UNIQUE_ID_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&directory).expect("startup heartbeat fixture");
    #[cfg(unix)]
    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))
        .expect("private startup heartbeat fixture");
    let path = directory.join("42.json");
    (directory, path)
}

#[cfg(unix)]
fn assert_fifo_reader_nonblocking(
    fifo: &Path,
    reader: impl FnOnce() -> std::io::Result<super::RegularFileSnapshot> + Send + 'static,
) {
    let (send, receive) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        let _ = send.send(reader());
    });
    match receive.recv_timeout(std::time::Duration::from_secs(2)) {
        Ok(result) => assert!(result.is_err(), "FIFO was accepted as a regular file"),
        Err(error) => {
            if let Ok(writer) = nix::fcntl::open(
                fifo,
                nix::fcntl::OFlag::O_WRONLY | nix::fcntl::OFlag::O_NONBLOCK,
                nix::sys::stat::Mode::empty(),
            ) {
                drop(writer);
            }
            if receive
                .recv_timeout(std::time::Duration::from_secs(2))
                .is_ok()
            {
                let _ = reader.join();
            }
            panic!("heartbeat FIFO reader blocked: {error}");
        }
    }
    reader.join().unwrap();
}

#[cfg(target_os = "linux")]
fn anchored_startup_heartbeat_fixture(
    label: &str,
) -> (
    PathBuf,
    PathBuf,
    PathBuf,
    std::fs::File,
    Box<super::StartupHeartbeatSnapshot>,
) {
    use nix::fcntl::{open, OFlag};
    use nix::sys::stat::Mode;

    let (parent_path, _) = startup_heartbeat_fixture(label);
    let repo_path = parent_path.join("repo");
    std::fs::create_dir(&repo_path).unwrap();
    std::fs::set_permissions(&repo_path, std::fs::Permissions::from_mode(0o700)).unwrap();
    let source = repo_path.join("42.json");
    std::fs::write(
        &source,
        startup_heartbeat_document("host:user:rust:4242:nonce-a", 0),
    )
    .unwrap();
    std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o600)).unwrap();
    let snapshot = expired_heartbeat_snapshot(&source);
    let parent = std::fs::File::from(
        open(
            &parent_path,
            OFlag::O_PATH | OFlag::O_DIRECTORY,
            Mode::empty(),
        )
        .unwrap(),
    );
    let repo = super::open_heartbeat_directory_beneath(&parent, Path::new("repo")).unwrap();
    (parent_path, repo_path, source, repo, snapshot)
}

fn startup_heartbeat_document(worker: &str, pid: u32) -> String {
    let nonce = super::startup_heartbeat_nonce("owner/repo", 42, "claim-a");
    format!(
        r#"{{"repo":"owner/repo","issue":"42","worker_id":"{worker}","branch":"feat/worker","pr":"","claim_id":"claim-a","step":"claimed","ts":100,"ttl_seconds":10,"pid":{pid},"nonce":"{nonce}","host":"host-a","boot_id":"boot-a","process_start":"1"}}"#
    )
}

fn expected_startup_heartbeat<'a>(
    worker_id: &'a str,
) -> super::StartupHeartbeatExpectation<'a> {
    super::StartupHeartbeatExpectation {
        repo: "owner/repo",
        issue: 42,
        worker_id,
        branch: "feat/worker",
        pull_request: "",
        claim_id: "claim-a",
        step: "claimed",
    }
}

fn inject_heartbeat_boundary(
    observed: &str,
    target: &str,
    message: &str,
) -> Result<(), super::CommandFailure> {
    if observed == target {
        Err(super::CommandFailure::diagnostic(message))
    } else {
        Ok(())
    }
}

#[cfg(target_os = "linux")]
#[test]
fn startup_heartbeat_atomic_publication() {
    use super::HeartbeatPublicationDurability::{Durable, Unconfirmed};
    use super::HeartbeatPublicationFailure::{PostCommit, PreCommit};
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
        super::publish_private_heartbeat_file(
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
                    Err(super::CommandFailure::diagnostic(
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
        let result = super::publish_private_heartbeat_file(
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
    unsafe { std::env::set_var("AUTOSPEC_HEARTBEAT_DIR", &root) };
    let issue = repo.join("42.json");
    let session = sessions.join("73657373696f6e2d61.json");
    let write = || {
        super::write_startup_heartbeat(
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
        super::publish_startup_heartbeat_transaction_with_hook(
            &root,
            "owner/repo",
            issue,
            Some(session),
            document,
            &mut |role, boundary| {
                ((role, boundary) != failed)
                    .then_some(())
                    .ok_or_else(|| super::CommandFailure::diagnostic("injected failure"))
            },
        )
    };
    assert!(attempt(43, "session-b", prepared, ("session", "before-link")).is_err());
    assert!(
        !repo.join("43.json").exists() && !sessions.join("73657373696f6e2d62.json").exists()
    );

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
        let result = super::publish_startup_heartbeat_transaction_with_hook(
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
    super::prepare_heartbeat_root_parent_with_hook(&root, |boundary| {
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
    super::prepare_heartbeat_root_parent_with_hook(&root, |boundary| {
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
    assert!(super::prepare_heartbeat_root_parent_with_hook(&root, |_| Ok(())).is_err());
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
    let failed = super::prepare_heartbeat_root_parent_with_hook(&root, |boundary| {
        inject_heartbeat_boundary(boundary, "chmod", "injected chmod failure")
    });
    umask(previous);

    assert!(failed.is_err());
    assert!(!parent.exists());
    assert_no_staging();
    let failed = super::prepare_heartbeat_root_parent_with_hook(&root, |boundary| {
        inject_heartbeat_boundary(boundary, "parent", "injected fsync failure")
    });
    assert!(failed.is_err());
    assert!(!parent.exists());
    assert_no_staging();
    super::prepare_heartbeat_root_parent_with_hook(&root, |_| Ok(())).unwrap();
    assert_eq!(
        std::fs::metadata(&parent).unwrap().permissions().mode() & 0o7777,
        0o700
    );

    std::fs::remove_dir(&parent).unwrap();
    let replacement_identity = std::cell::Cell::new((0, 0));
    let raced = super::prepare_heartbeat_root_parent_with_hook(&root, |boundary| {
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
                return Err(super::CommandFailure::diagnostic(
                    "injected ancestor fsync failure",
                ));
            }
        }
        Ok(())
    };
    assert!(
        super::prepare_heartbeat_root_parent_with_hook(&root, &mut ancestor_failure).is_err()
    );
    assert!(
        parent.is_dir(),
        "published parent remains pending durability"
    );
    super::prepare_heartbeat_root_parent_with_hook(&root, &mut ancestor_failure).unwrap();
    assert_eq!(ancestor_attempts.get(), 2);

    std::fs::remove_dir(&parent).unwrap();
    let cleanup_failure = super::prepare_heartbeat_root_parent_with_hook(&root, |boundary| {
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
            return Err(super::CommandFailure::diagnostic("injected staged failure"));
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
    let identity = super::ClaimMutationIdentity {
        repo: "owner/repo",
        issue: 42,
        worker_id: "worker-a",
        branch: "feat/worker",
        claim_id: "claim-a",
    };
    let result = super::retire_released_startup_heartbeat(identity);
    match previous {
        Some(value) => std::env::set_var("AUTOSPEC_HEARTBEAT_DIR", value),
        None => std::env::remove_var("AUTOSPEC_HEARTBEAT_DIR"),
    }
    std::fs::remove_dir_all(root).unwrap();
    result.expect_err("missing issue evidence must keep terminal preparation retryable");
}
#[cfg(target_os = "linux")]
#[test]
fn prior_generation_authorization_rejects_heartbeat_replacement() {
    let _guard = STARTUP_HEARTBEAT_ENV.lock().expect("heartbeat env");
    let (sandbox, _) = startup_heartbeat_fixture("prior-generation-race");
    let heartbeat_root = sandbox.join("heartbeats");
    let repo_key = super::super::autonomous::drain::repository_progress_key("owner/repo");
    let repo = heartbeat_root.join(repo_key);
    std::fs::create_dir_all(&repo).unwrap();
    for directory in [&heartbeat_root, &repo] {
        std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let host = std::fs::read_to_string("/proc/sys/kernel/hostname")
        .unwrap()
        .trim()
        .to_string();
    let boot = super::super::autonomous::current_boot_identity().unwrap();
    let document = |worker: &str, claim: &str| {
        let nonce = super::startup_heartbeat_nonce("owner/repo", 42, claim);
        format!(
            r#"{{"repo":"owner/repo","issue":"42","worker_id":"{worker}","branch":"feat/worker","pr":"","claim_id":"{claim}","step":"claimed","ts":1,"ttl_seconds":10,"pid":2147483647,"nonce":"{nonce}","host":"{host}","boot_id":"{boot}","process_start":"1"}}"#
        )
    };
    let source = repo.join("42.json");
    std::fs::write(&source, document("prior-worker", "prior-claim")).unwrap();
    std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o600)).unwrap();
    let file = super::read_regular_file(std::fs::File::open(&source).unwrap()).unwrap();
    let authorized = super::StartupHeartbeatSnapshot {
        evidence: super::parse_startup_heartbeat(&file.document).unwrap(),
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
    let result = super::quarantine_authoritative_stale_heartbeat(
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
    let file = super::read_regular_file(std::fs::File::open(&source).unwrap()).unwrap();
    let authorized = super::StartupHeartbeatSnapshot {
        evidence: super::parse_startup_heartbeat(&file.document).unwrap(),
        file,
    };
    assert!(super::quarantine_authoritative_stale_heartbeat(
        "owner/repo",
        42,
        &record,
        Some(&authorized),
        &mut || Ok(()),
    )
    .unwrap());
    assert!(!source.exists());
    let retained = super::expired_prior_generation_heartbeat("owner/repo", 42, &record)
        .unwrap()
        .expect("retained authorization");
    assert!(super::quarantine_authoritative_stale_heartbeat(
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
    let opened = super::open_heartbeat_directory_beneath(&parent, Path::new("heartbeat"))
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
    assert!(super::open_heartbeat_directory_beneath(
        &parent,
        Path::new("parent-link/heartbeat")
    )
    .is_err());
    assert!(super::open_heartbeat_directory_beneath(&parent, Path::new("")).is_err());
    assert!(super::open_heartbeat_directory_beneath(&parent, &trusted_child).is_err());
    assert!(super::open_heartbeat_directory_beneath(&parent, Path::new("../escape")).is_err());
    assert!(super::heartbeat_openat2_resolve_flags().contains(ResolveFlag::RESOLVE_NO_XDEV));

    let (drifting, _) = startup_heartbeat_fixture("openat2-drifting");
    make_child(&drifting);
    let drifting_parent = open_parent(&drifting);
    let drift = super::open_heartbeat_directory_beneath_with_hook(
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
    let opened = super::open_heartbeat_directory_beneath_with_hook(
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
    let swapped_child = super::open_heartbeat_directory_beneath_with_hook(
        &anchored_parent,
        Path::new("heartbeat"),
        || {
            std::fs::rename(&replaceable_child, &displaced_child)
                .expect("displace trusted child");
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
    let opened = super::open_heartbeat_directory_portable_unix_with_hook(
        &parent,
        Path::new("heartbeat"),
        || {},
    )
    .expect("open one-component descendant");
    let expected = std::fs::metadata(&trusted_child).expect("descendant metadata");
    let observed = fstat(&opened).expect("opened metadata");
    assert_eq!(
        (observed.st_dev, observed.st_ino),
        (expected.dev(), expected.ino())
    );

    let (replacement, _) = startup_heartbeat_fixture("portable-unix-replacement");
    let replacement_child = make_private_child(&replacement);
    let displaced = trusted.join("displaced");
    let swapped = super::open_heartbeat_directory_portable_unix_with_hook(
        &parent,
        Path::new("heartbeat"),
        || {
            std::fs::rename(&trusted_child, &displaced).expect("displace trusted child");
            std::fs::rename(&replacement_child, &trusted_child)
                .expect("install replacement child");
        },
    );
    assert!(swapped.is_err(), "changed name binding was accepted");
    std::fs::remove_dir(&trusted_child).expect("remove replacement child");
    symlink(&displaced, &trusted_child).expect("install descendant symlink");
    assert!(
        super::open_heartbeat_directory_portable_unix_with_hook(
            &parent,
            Path::new("heartbeat"),
            || {},
        )
        .is_err(),
        "descendant symlink was accepted"
    );
    assert!(
        super::open_heartbeat_directory_portable_unix_with_hook(
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
    let classified = super::classify_startup_heartbeat(
        &path,
        expected_startup_heartbeat("worker-a"),
        200,
        |_, _, _, _, _| super::StartupPidLiveness::Dead,
    );
    assert_eq!(classified, super::StartupHeartbeatClassification::Absent);
    std::fs::remove_dir_all(directory).expect("remove heartbeat fixture");
}

#[cfg(target_os = "linux")]
#[test]
fn startup_heartbeat_process_identity() {
    use super::{StartupHeartbeatClassification::ExpiredDead, StartupPidLiveness};

    let _environment = STARTUP_HEARTBEAT_ENV.lock().unwrap();
    let (directory, _) = startup_heartbeat_fixture("process-identity");
    let root = directory.join(".autospec/process-heartbeats");
    let old_root = std::env::var_os("AUTOSPEC_HEARTBEAT_DIR");
    let old_ttl = std::env::var_os("AUTOSPEC_CLAIM_LEASE_SECONDS");
    std::env::set_var("AUTOSPEC_HEARTBEAT_DIR", &root);
    std::env::set_var("AUTOSPEC_CLAIM_LEASE_SECONDS", "0");
    let path = root
        .join(super::super::autonomous::drain::repository_progress_key(
            "owner/repo",
        ))
        .join("42.json");
    let publish = |claim_id, session_id| {
        super::write_startup_heartbeat(
            "owner/repo",
            42,
            "worker-a",
            "feat/worker",
            claim_id,
            Some(session_id),
        )
        .unwrap();
        super::parse_startup_heartbeat(&std::fs::read(&path).unwrap())
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
        super::write_startup_heartbeat(
            "owner/repo",
            42,
            worker,
            "feat/worker",
            "claim-a",
            None,
        )
        .unwrap();
        let document = std::fs::read(&path).unwrap();
        let evidence = super::parse_startup_heartbeat(&document).unwrap();
        assert_eq!(evidence.worker_id, worker);
        assert_eq!(evidence.pid, std::process::id());
        assert_eq!(evidence.ttl_seconds, 1);
        assert!(!evidence.nonce.is_empty());
        assert_eq!(
            super::observe_local_startup_pid(
                worker,
                evidence.pid,
                &evidence.host,
                &evidence.boot_id,
                &evidence.process_start,
            ),
            StartupPidLiveness::Live
        );
        assert!(matches!(
            super::classify_startup_heartbeat(
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
            super::classify_startup_heartbeat(
                &path,
                expected_startup_heartbeat(worker),
                evidence.ts + evidence.ttl_seconds + 1,
                |_, _, _, _, _| StartupPidLiveness::Dead,
            ),
            super::StartupHeartbeatClassification::Blocking
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

#[cfg(target_os = "linux")]
#[test]
fn startup_heartbeat_remote_process_identity() {
    let _environment = STARTUP_HEARTBEAT_ENV.lock().unwrap();
    let (directory, _) = startup_heartbeat_fixture("remote-process-identity");
    let root = directory.join(".autospec/process-heartbeats");
    let old_root = std::env::var_os("AUTOSPEC_HEARTBEAT_DIR");
    std::env::set_var("AUTOSPEC_HEARTBEAT_DIR", &root);
    super::write_startup_heartbeat(
        "owner/repo",
        42,
        "opaque-worker",
        "feat/worker",
        "claim-a",
        None,
    )
    .unwrap();
    let path = root
        .join(super::super::autonomous::drain::repository_progress_key(
            "owner/repo",
        ))
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
            super::classify_startup_heartbeat(
                &path,
                expected_startup_heartbeat("opaque-worker"),
                value["ts"].as_u64().unwrap() + value["ttl_seconds"].as_u64().unwrap() + 1,
                super::observe_local_startup_pid,
            ),
            super::StartupHeartbeatClassification::Blocking,
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
    use super::HeartbeatReceiptDecision::{Absent, Blocking, Completed, Pending};
    use nix::fcntl::{open, OFlag};
    use nix::sys::stat::Mode;
    use std::os::unix::fs::symlink;
    use std::os::unix::net::UnixListener;
    use std::os::unix::prelude::AsRawFd;
    use std::time::{Duration, Instant};
    let expected = expected_startup_heartbeat("host:user:rust:4242:nonce-a");
    let open_parent = |path: &Path| {
        std::fs::File::from(
            open(path, OFlag::O_PATH | OFlag::O_DIRECTORY, Mode::empty()).unwrap(),
        )
    };
    let (parent_path, _) = startup_heartbeat_fixture("receipt-red");
    let repo_path = parent_path.join("repo");
    std::fs::create_dir(&repo_path).expect("repo directory");
    std::fs::set_permissions(&repo_path, std::fs::Permissions::from_mode(0o700))
        .expect("private repo");
    let parent = open_parent(&parent_path);
    let repo =
        super::open_heartbeat_directory_beneath(&parent, Path::new("repo")).expect("repo fd");
    let decision = || super::heartbeat_receipt_retry_decision(&repo, expected);
    let transaction = super::begin_heartbeat_receipt(&repo, expected).expect("begin receipt");
    assert_eq!(decision(), Pending);
    let mut unrelated = expected;
    unrelated.issue += 1;
    assert_eq!(
        super::heartbeat_receipt_retry_decision(&repo, unrelated),
        Absent
    );
    super::retire_heartbeat_receipt_with_sync(transaction, |_| {
        Err(super::CommandFailure::diagnostic("injected sync failure"))
    })
    .expect_err("sync failure");
    assert_eq!(decision(), Completed);
    assert!(super::begin_heartbeat_receipt(&repo, expected).is_err());
    let (pending, completed) = super::heartbeat_receipt_names(expected);
    let handoff = repo_path.join("quarantine/startup-heartbeat-handoffs");
    let pending_path = handoff.join(&pending);
    let completed_path = handoff.join(&completed);
    std::fs::remove_file(&completed_path).unwrap();
    super::begin_heartbeat_receipt_with_hook(&repo, expected, || {
        std::fs::write(&completed_path, b"").unwrap();
        std::fs::set_permissions(&completed_path, std::fs::Permissions::from_mode(0o600))
            .unwrap();
    })
    .err()
    .expect("completed wins");
    assert_eq!(decision(), Blocking);
    assert!(pending_path.exists() && completed_path.exists());
    std::fs::remove_file(&pending_path).unwrap();
    std::fs::remove_file(&completed_path).unwrap();
    let mut transaction = super::begin_heartbeat_receipt(&repo, expected).unwrap();
    transaction.pending.push_str("-missing");
    super::retire_heartbeat_receipt_with_sync(transaction, |_| Ok(()))
        .expect_err("rename failure");
    assert_eq!(decision(), Pending);
    let (pending, completed) = super::heartbeat_receipt_names(expected);
    let handoff_fd = std::fs::File::open(&handoff).unwrap();
    let socket_root = PathBuf::from(format!("/proc/self/fd/{}", handoff_fd.as_raw_fd()));
    let drift =
        super::heartbeat_receipt_retry_decision_with_hook(&repo, expected, |boundary| {
            if boundary == "handoff" {
                std::fs::set_permissions(&handoff, std::fs::Permissions::from_mode(0o755))
                    .unwrap();
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
            "fifo" => {
                nix::unistd::mkfifo(&pending_path, Mode::from_bits_truncate(0o600)).unwrap()
            }
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
        super::inspect_heartbeat_receipt(&dev, std::ffi::OsStr::new("null")),
        super::HeartbeatReceiptEntry::Unsafe
    );
    std::fs::remove_dir_all(parent_path).expect("remove failure fixture");
    let (parent_path, _) = startup_heartbeat_fixture("receipt-renames");
    let repo_path = parent_path.join("repo");
    std::fs::create_dir(&repo_path).unwrap();
    std::fs::set_permissions(&repo_path, std::fs::Permissions::from_mode(0o700)).unwrap();
    let parent = open_parent(&parent_path);
    let repo = super::open_heartbeat_directory_beneath(&parent, Path::new("repo")).unwrap();
    drop(super::begin_heartbeat_receipt(&repo, expected).unwrap());
    let replace = |path: &Path, moved: &Path| {
        std::fs::rename(path, moved).unwrap();
        std::fs::create_dir(path).unwrap();
    };
    let decision =
        super::heartbeat_receipt_retry_decision_with_hook(&repo, expected, |boundary| {
            match boundary {
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
            }
        });
    assert_eq!(decision, Pending);
    std::fs::remove_dir_all(parent_path).expect("remove rename fixture");
}

#[test]
fn unsupported_platform_stale_recovery_is_fail_closed() {
    let source = include_str!("../claim.rs");
    let fallback = source
        .split(
            "#[cfg(not(target_os = \"linux\"))]\nfn quarantine_authoritative_stale_heartbeat",
        )
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
    use super::StartupPidLiveness::{Dead, Live};
    let (directory, path) = startup_heartbeat_fixture("blocking");
    let worker = "host:user:rust:4242:nonce-a";
    let exact = startup_heartbeat_document(worker, 4242);
    let changed = |from, to| exact.replace(from, to);
    let mut cases = vec![
        ("fresh", exact.clone(), 105, Dead, false),
        ("live", exact.clone(), 200, Live, true),
    ];
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
            super::classify_startup_heartbeat(
                &path,
                expected_startup_heartbeat(worker),
                now,
                |_, _, _, _, _| {
                    observed = true;
                    liveness
                },
            ),
            super::StartupHeartbeatClassification::Blocking,
            "{label}"
        );
        assert_eq!(observed, should_observe, "{label} liveness call");
    }
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
    let classified = super::classify_startup_heartbeat(
        &path,
        expected_startup_heartbeat(worker),
        200,
        |worker, pid, _, _, _| {
            assert_eq!((worker, pid), ("host:user:rust:4242:nonce-a", 4242));
            super::StartupPidLiveness::Dead
        },
    );
    let super::StartupHeartbeatClassification::ExpiredDead(snapshot) = classified else {
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

#[cfg(unix)]
#[test]
fn classify_startup_heartbeat_rejects_symlink_and_observes_current_pid_as_live() {
    use super::StartupHeartbeatClassification::Blocking;

    let (directory, path) = startup_heartbeat_fixture("symlink-live");
    let target = directory.join("target.json");
    let host = std::env::var("HOSTNAME").unwrap_or_else(|_| "unknown-host".into());
    let user = std::env::var("USER").unwrap_or_else(|_| "unknown-user".into());
    let pid = std::process::id();
    let worker = format!("{host}:{user}:rust:{pid}:nonce-a");
    let identity = super::startup_process_identity(pid).unwrap();
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
        let result = super::classify_startup_heartbeat(
            &path,
            expected_startup_heartbeat(worker),
            200,
            |worker, pid, host, boot_id, process_start| {
                observed = true;
                super::observe_local_startup_pid(worker, pid, host, boot_id, process_start)
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
        super::observe_local_startup_pid(
            &remote_worker,
            pid,
            &identity.0,
            &identity.1,
            &identity.2,
        ),
        super::StartupPidLiveness::Live
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
        super::read_regular_file_no_follow(&read_path)
    });
    assert_eq!(
        super::classify_startup_heartbeat(
            &fifo,
            expected_startup_heartbeat("host:user:rust:4242:nonce-a"),
            200,
            |_, _, _, _, _| super::StartupPidLiveness::Dead,
        ),
        super::StartupHeartbeatClassification::Blocking
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
        super::read_regular_file_at_no_follow(&root, &name)
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

#[cfg(unix)]
fn expired_heartbeat_snapshot(path: &Path) -> Box<super::StartupHeartbeatSnapshot> {
    let worker = "host:user:rust:4242:nonce-a";
    std::fs::write(path, startup_heartbeat_document(worker, 4242))
        .expect("write expired heartbeat");
    let classified = super::classify_startup_heartbeat(
        path,
        expected_startup_heartbeat(worker),
        200,
        |_, _, _, _, _| super::StartupPidLiveness::Dead,
    );
    let super::StartupHeartbeatClassification::ExpiredDead(snapshot) = classified else {
        panic!("fixture heartbeat was not expired and dead");
    };
    snapshot
}

#[cfg(unix)]
fn heartbeat_copy_path(root: &Path) -> PathBuf {
    let nonce = super::startup_heartbeat_nonce("owner/repo", 42, "claim-a");
    root.join(format!(
        "quarantine/startup-heartbeats/42-{}.json",
        super::heartbeat_session_key(&nonce)
    ))
}

#[cfg(unix)]
fn heartbeat_handoff_count(root: &Path) -> usize {
    std::fs::read_dir(root.join("quarantine/startup-heartbeat-handoffs"))
        .expect("handoff directory")
        .count()
}

#[cfg(unix)]
fn write_new_heartbeat_at(directory: &impl std::os::fd::AsFd, document: &[u8]) {
    let fd = nix::fcntl::openat(
        directory,
        "42.json",
        nix::fcntl::OFlag::O_WRONLY | nix::fcntl::OFlag::O_CREAT | nix::fcntl::OFlag::O_EXCL,
        nix::sys::stat::Mode::from_bits_truncate(0o600),
    )
    .expect("publish live replacement");
    std::fs::File::from(fd)
        .write_all(document)
        .expect("write replacement");
}

#[cfg(unix)]
fn drift_heartbeat_at(directory: &impl std::os::fd::AsFd, name: &str) {
    let fd = nix::fcntl::openat(
        directory,
        name,
        nix::fcntl::OFlag::O_WRONLY | nix::fcntl::OFlag::O_TRUNC,
        nix::sys::stat::Mode::empty(),
    )
    .expect("open moved heartbeat");
    std::fs::File::from(fd).write_all(b"drift").unwrap();
}

#[cfg(target_os = "linux")]
fn mutate_retained(path: &Path, source: &Path, mutation: &str) {
    match mutation.trim_start_matches("cleanup-") {
        "content" => std::fs::write(path, b"drift"),
        "mode" => std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o640)),
        "binding" => std::fs::rename(path, path.with_extension("moved")),
        "source" => std::fs::write(source, b"foreign"),
        _ => unreachable!(),
    }
    .unwrap();
}
#[cfg(unix)]
fn assert_mode(path: &Path, expected: u32) {
    let permissions = std::fs::metadata(path)
        .expect("private path metadata")
        .permissions();
    assert_eq!(permissions.mode() & 0o777, expected);
}

#[cfg(unix)]
#[test]
fn heartbeat_quarantine_copy_is_private_exact_and_create_new() {
    let (directory, source) = startup_heartbeat_fixture("copy-success");
    let snapshot = expired_heartbeat_snapshot(&source);
    let mut sync_order = Vec::new();
    let target = super::persist_heartbeat_copy_with_hooks(
        &directory,
        &source,
        &snapshot,
        |_| {},
        |boundary| {
            sync_order.push(boundary);
            Ok(())
        },
    )
    .expect("persist quarantine copy");

    assert_eq!(
        sync_order,
        [
            super::HeartbeatCopySyncBoundary::File,
            super::HeartbeatCopySyncBoundary::Directory,
        ]
    );
    assert_eq!(
        std::fs::read(&target).expect("read copy"),
        snapshot.file.document
    );
    assert_eq!(
        std::fs::read(&source).expect("read source"),
        snapshot.file.document
    );
    assert_mode(&directory.join("quarantine"), 0o700);
    assert_mode(&directory.join("quarantine/startup-heartbeats"), 0o700);
    assert_mode(&target, 0o600);
    let duplicate = super::persist_heartbeat_copy(&directory, &source, &snapshot)
        .expect_err("copy target must be create-new");
    assert!(duplicate.to_string().contains("already exists"));
    let flags = nix::fcntl::OFlag::O_RDONLY | nix::fcntl::OFlag::O_DIRECTORY;
    let directory_fd =
        nix::fcntl::open(&directory, flags, nix::sys::stat::Mode::empty()).expect("open root");
    let (pipe, _writer) = nix::unistd::pipe().expect("file-sync pipe");
    let error =
        super::sync_heartbeat_copy(&std::fs::File::from(pipe), &directory_fd, &mut |_| {
            panic!("boundary emitted after failed file sync")
        })
        .expect_err("pipe file cannot sync");
    assert!(error.to_string().contains("sync heartbeat quarantine copy"));

    let file = std::fs::File::options().write(true).open(&source).unwrap();
    let (pipe, _writer) = nix::unistd::pipe().expect("directory-sync pipe");
    let mut boundaries = Vec::new();
    let error = super::sync_heartbeat_copy(&file, &pipe, &mut |boundary| {
        boundaries.push(boundary);
        Ok(())
    })
    .expect_err("pipe directory cannot sync");
    assert!(error
        .to_string()
        .contains("sync heartbeat quarantine directory"));
    assert_eq!(boundaries, [super::HeartbeatCopySyncBoundary::File]);
    std::fs::remove_dir_all(directory).expect("remove heartbeat fixture");
}

#[cfg(unix)]
#[test]
fn heartbeat_quarantine_copy_rejects_snapshot_drift_deterministically() {
    let (directory, source) = startup_heartbeat_fixture("copy-drift");
    let snapshot = expired_heartbeat_snapshot(&source);
    let mut observed = snapshot.file.clone();
    observed.identity.inode += 1;
    assert!(
        super::revalidate_heartbeat_snapshot(&observed, &snapshot.file)
            .expect_err("identity drift")
            .to_string()
            .contains("identity drift")
    );
    observed = snapshot.file.clone();
    observed.document[0] = b'[';
    let mut document_drift = (*snapshot).clone();
    document_drift.file = observed.clone();
    assert!(
        super::persist_heartbeat_copy(&directory, &source, &document_drift)
            .expect_err("document drift")
            .to_string()
            .contains("document drift")
    );

    std::fs::rename(&source, directory.join("old.json")).expect("move classified inode");
    std::fs::write(&source, &snapshot.file.document).expect("publish replacement inode");
    assert!(
        super::persist_heartbeat_copy(&directory, &source, &snapshot)
            .expect_err("replacement source")
            .to_string()
            .contains("identity drift")
    );
    assert_eq!(
        std::fs::read(&source).expect("replacement survives"),
        snapshot.file.document
    );
    std::fs::remove_dir_all(directory).expect("remove heartbeat fixture");
}

#[cfg(unix)]
#[test]
fn heartbeat_quarantine_copy_rejects_preexisting_and_swapped_symlink_ancestors() {
    let (preexisting_root, preexisting_source) =
        startup_heartbeat_fixture("copy-preexisting-symlink");
    let preexisting_snapshot = expired_heartbeat_snapshot(&preexisting_source);
    let outside = startup_heartbeat_fixture("copy-outside").0;
    std::fs::set_permissions(&preexisting_root, std::fs::Permissions::from_mode(0o755))
        .expect("make state root non-private");
    assert!(super::persist_heartbeat_copy(
        &preexisting_root,
        &preexisting_source,
        &preexisting_snapshot,
    )
    .expect_err("non-private root")
    .to_string()
    .contains("private state root"));
    std::fs::set_permissions(&preexisting_root, std::fs::Permissions::from_mode(0o700))
        .expect("restore private state root");
    let root_link = preexisting_root.with_extension("root-link");
    std::os::unix::fs::symlink(&preexisting_root, &root_link).expect("state root symlink");
    assert!(super::persist_heartbeat_copy(
        &root_link,
        &preexisting_source,
        &preexisting_snapshot
    )
    .is_err());
    std::fs::remove_file(root_link).expect("remove state root symlink");
    std::os::unix::fs::symlink(&outside, preexisting_root.join("quarantine"))
        .expect("preexisting quarantine symlink");
    assert!(super::persist_heartbeat_copy(
        &preexisting_root,
        &preexisting_source,
        &preexisting_snapshot,
    )
    .is_err());
    assert_eq!(
        std::fs::read_dir(&outside)
            .expect("outside directory")
            .count(),
        0
    );

    let (swapped_root, swapped_source) = startup_heartbeat_fixture("copy-swapped-symlink");
    let swapped_snapshot = expired_heartbeat_snapshot(&swapped_source);
    let mut swapped = false;
    let result = super::persist_heartbeat_copy_with_hooks(
        &swapped_root,
        &swapped_source,
        &swapped_snapshot,
        |component| {
            if component == "quarantine" {
                std::fs::rename(
                    swapped_root.join("quarantine"),
                    swapped_root.join("displaced"),
                )
                .expect("displace opened quarantine");
                std::os::unix::fs::symlink(&outside, swapped_root.join("quarantine"))
                    .expect("swap quarantine symlink");
                swapped = true;
            }
        },
        |_| Ok(()),
    );
    assert!(swapped);
    assert!(result.is_err());
    assert_eq!(
        std::fs::read_dir(&outside)
            .expect("outside directory")
            .count(),
        0
    );
    assert_eq!(
        std::fs::read(&swapped_source).expect("source survives"),
        swapped_snapshot.file.document
    );

    std::fs::remove_dir_all(preexisting_root).expect("remove preexisting fixture");
    std::fs::remove_dir_all(swapped_root).expect("remove swapped fixture");
    std::fs::remove_dir_all(outside).expect("remove outside fixture");
}

#[cfg(unix)]
#[test]
fn stale_heartbeat_quarantine_aborts_for_replacement_before_handoff() {
    let (directory, source) = startup_heartbeat_fixture("handoff-replacement");
    let snapshot = expired_heartbeat_snapshot(&source);
    let replacement = b"new live heartbeat".to_vec();
    let displaced = directory.join("classified.json");
    let result = super::handoff_stale_heartbeat_path_with_hooks(
        &directory,
        &source,
        &snapshot,
        || {
            std::fs::rename(&source, &displaced).expect("displace classified heartbeat");
            std::fs::write(&source, &replacement).expect("publish replacement heartbeat");
        },
        |_, _, _| {},
        |_| Ok(()),
    );

    assert!(result
        .expect_err("replacement must abort")
        .to_string()
        .contains("drift"));
    assert_eq!(
        std::fs::read(&source).expect("replacement survives"),
        replacement
    );
    let copy = heartbeat_copy_path(&directory);
    assert_eq!(
        std::fs::read(copy).expect("durable copy"),
        snapshot.file.document
    );
    std::fs::remove_dir_all(directory).expect("remove heartbeat fixture");
}

#[cfg(unix)]
#[test]
fn stale_heartbeat_handoff_restores_drift_or_preserves_it_beside_live_replacement() {
    for occupied in [false, true] {
        let (directory, source) = startup_heartbeat_fixture(if occupied {
            "handoff-occupied"
        } else {
            "handoff-restore"
        });
        let snapshot = expired_heartbeat_snapshot(&source);
        let result = super::handoff_stale_heartbeat_path_with_hooks(
            &directory,
            &source,
            &snapshot,
            || {},
            |root, handoff, moved| {
                drift_heartbeat_at(handoff, moved);
                if occupied {
                    write_new_heartbeat_at(root, b"replacement");
                }
            },
            |_| Ok(()),
        );
        assert!(result
            .expect_err("drift must abort")
            .to_string()
            .contains("drift"));
        assert_eq!(
            std::fs::read(&source).expect("live name survives"),
            if occupied {
                b"replacement".as_slice()
            } else {
                b"drift".as_slice()
            }
        );
        if !occupied {
            let live = std::fs::metadata(&source).expect("restored heartbeat metadata");
            assert_eq!(
                std::os::unix::fs::MetadataExt::ino(&live),
                snapshot.file.identity.inode
            );
        }
        assert_eq!(heartbeat_handoff_count(&directory), usize::from(occupied));
        std::fs::remove_dir_all(directory).expect("remove heartbeat fixture");
    }
}

#[cfg(unix)]
#[test]
fn stale_heartbeat_handoff_preserves_replacement_before_and_after_cleanup() {
    for mode in 0..3 {
        let (directory, source) = startup_heartbeat_fixture("handoff-sync");
        let snapshot = expired_heartbeat_snapshot(&source);
        let result = super::handoff_stale_heartbeat_path_with_hooks(
            &directory,
            &source,
            &snapshot,
            || {},
            |root, _, _| {
                if mode == 2 {
                    nix::unistd::mkfifo(
                        &source,
                        nix::sys::stat::Mode::from_bits_truncate(0o600),
                    )
                    .expect("publish live FIFO");
                } else if mode != 1 {
                    write_new_heartbeat_at(root, &snapshot.file.document);
                }
            },
            |boundary| {
                if mode == 1 && boundary == super::HeartbeatHandoffSyncBoundary::Cleanup {
                    std::fs::write(&source, &snapshot.file.document)
                        .expect("publish post-unlink replacement");
                    return Err(super::CommandFailure::diagnostic(
                        "injected cleanup failure",
                    ));
                }
                Ok(())
            },
        );

        assert!(result.is_err());
        if mode == 2 {
            let kind = std::fs::symlink_metadata(&source)
                .expect("FIFO metadata")
                .file_type();
            assert!(std::os::unix::fs::FileTypeExt::is_fifo(&kind));
        } else {
            assert_eq!(std::fs::read(&source).unwrap(), snapshot.file.document);
        }
        assert!(heartbeat_copy_path(&directory).is_file());
        assert_eq!(heartbeat_handoff_count(&directory), usize::from(mode != 1));
        std::fs::remove_dir_all(directory).expect("remove heartbeat fixture");
    }
}

#[cfg(target_os = "linux")]
#[test]
fn stale_heartbeat_handoff_failure_atomic() {
    use super::HeartbeatReceiptDecision::{Completed, Pending};

    let boundaries = [
        super::HeartbeatHandoffSyncBoundary::Source,
        super::HeartbeatHandoffSyncBoundary::Handoff,
        super::HeartbeatHandoffSyncBoundary::Cleanup,
        super::HeartbeatHandoffSyncBoundary::RestoreRoot,
        super::HeartbeatHandoffSyncBoundary::RestoreUnlink,
        super::HeartbeatHandoffSyncBoundary::RestoreHandoff,
    ];
    for failed in boundaries {
        let label = format!("atomic-{failed:?}");
        let (parent_path, repo_path, source, repo, snapshot) =
            anchored_startup_heartbeat_fixture(label.as_str());
        let expectation = expected_startup_heartbeat("host:user:rust:4242:nonce-a");
        let result = super::handoff_stale_heartbeat(
            &repo_path,
            &repo,
            source.file_name().unwrap(),
            &snapshot,
            |_, handoff, moved| {
                if matches!(
                    failed,
                    super::HeartbeatHandoffSyncBoundary::RestoreRoot
                        | super::HeartbeatHandoffSyncBoundary::RestoreUnlink
                        | super::HeartbeatHandoffSyncBoundary::RestoreHandoff
                ) {
                    drift_heartbeat_at(handoff, moved);
                }
            },
            |boundary| {
                if failed == boundary {
                    Err(super::CommandFailure::diagnostic("injected sync failure"))
                } else {
                    Ok(())
                }
            },
            |directory| {
                nix::unistd::fsync(directory).map_err(|error| {
                    super::CommandFailure::diagnostic(format!("cleanup fsync: {error}"))
                })
            },
        );
        assert!(result.is_err(), "{failed:?} must abort");
        assert_eq!(
            super::heartbeat_receipt_retry_decision(&repo, expectation),
            Pending
        );
        let handoff = repo_path.join("quarantine/startup-heartbeat-handoffs");
        assert!(
            source.exists() || std::fs::read_dir(handoff).unwrap().count() > 1,
            "{failed:?} removed live and moved identities"
        );
        std::fs::remove_dir_all(parent_path).unwrap();
    }

    for mode in 0..3 {
        let (parent_path, repo_path, source, repo, snapshot) =
            anchored_startup_heartbeat_fixture("atomic-drift");
        let expectation = expected_startup_heartbeat("host:user:rust:4242:nonce-a");
        let result = super::handoff_stale_heartbeat(
            &repo_path,
            &repo,
            source.file_name().unwrap(),
            &snapshot,
            |root, handoff, moved| match mode {
                0 => {
                    nix::unistd::unlinkat(
                        handoff,
                        moved,
                        nix::unistd::UnlinkatFlags::NoRemoveDir,
                    )
                    .unwrap();
                }
                1 => {
                    drift_heartbeat_at(handoff, moved);
                    write_new_heartbeat_at(root, b"replacement");
                }
                _ => {}
            },
            |_| Ok(()),
            |directory| {
                nix::unistd::fsync(directory).map_err(|error| {
                    super::CommandFailure::diagnostic(format!("cleanup fsync: {error}"))
                })
            },
        );
        assert_eq!(
            super::heartbeat_receipt_retry_decision(&repo, expectation),
            if mode == 2 { Completed } else { Pending }
        );
        assert_eq!(result.is_ok(), mode == 2);
        if mode == 1 {
            assert_eq!(std::fs::read(&source).unwrap(), b"replacement");
        }
        std::fs::remove_dir_all(parent_path).unwrap();
    }

    let (parent_path, repo_path, source, repo, snapshot) =
        anchored_startup_heartbeat_fixture("atomic-real-fsync");
    let expectation = expected_startup_heartbeat("host:user:rust:4242:nonce-a");
    let (pipe, _writer) = nix::unistd::pipe().unwrap();
    assert!(super::handoff_stale_heartbeat(
        &repo_path,
        &repo,
        source.file_name().unwrap(),
        &snapshot,
        |_, _, _| {},
        |_| Ok(()),
        |_| {
            nix::unistd::fsync(&pipe)
                .map_err(|error| super::CommandFailure::diagnostic(format!("fsync: {error}")))
        },
    )
    .is_err());
    assert_eq!(
        super::heartbeat_receipt_retry_decision(&repo, expectation),
        Pending
    );
    let handoff = repo_path.join("quarantine/startup-heartbeat-handoffs");
    assert!(source.exists() || std::fs::read_dir(handoff).unwrap().count() > 1);
    std::fs::remove_dir_all(parent_path).unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn startup_heartbeat_retained_handoff() {
    use std::os::unix::fs::MetadataExt;
    for race in
        "before-check before-rename after-rename collision malformed fifo hardlink".split(' ')
    {
        let (parent, repo_path, source, repo, snapshot) =
            anchored_startup_heartbeat_fixture(race);
        let replacement = repo_path.join("replacement");
        std::fs::write(&replacement, b"foreign").unwrap();
        std::fs::set_permissions(&replacement, std::fs::Permissions::from_mode(0o600)).unwrap();
        match race {
            "malformed" => std::fs::write(&source, [123]).unwrap(),
            "fifo" => {
                std::fs::remove_file(&source).unwrap();
                nix::unistd::mkfifo(&source, nix::sys::stat::Mode::from_bits_truncate(0o600))
                    .unwrap();
            }
            "hardlink" => std::fs::hard_link(&source, source.with_extension("link")).unwrap(),
            _ => {}
        }
        let result = super::handoff_retained_heartbeat(
            &repo_path,
            &repo,
            source.file_name().unwrap(),
            &snapshot,
            |boundary, _, handoff, name| {
                if race == "collision" && boundary == "before-check" {
                    let file = nix::fcntl::openat(
                        handoff,
                        name,
                        nix::fcntl::OFlag::O_WRONLY
                            | nix::fcntl::OFlag::O_CREAT
                            | nix::fcntl::OFlag::O_EXCL,
                        nix::sys::stat::Mode::from_bits_truncate(0o600),
                    )
                    .unwrap();
                    std::fs::File::from(file).write_all(b"foreign").unwrap();
                } else if boundary == race {
                    if race == "after-rename" {
                        std::fs::write(&source, b"foreign").unwrap();
                        std::fs::set_permissions(
                            &source,
                            std::fs::Permissions::from_mode(0o600),
                        )
                        .unwrap();
                    } else {
                        std::fs::rename(&replacement, &source).unwrap();
                    }
                }
            },
            |_| Ok(()),
        );
        assert!(result.is_err(), "{race}");
        if matches!(race, "before-check" | "before-rename" | "after-rename") {
            assert_eq!(std::fs::read(&source).unwrap(), b"foreign", "{race}");
        }
        std::fs::remove_dir_all(parent).unwrap();
    }

    for failed in [
        super::HeartbeatHandoffSyncBoundary::Source,
        super::HeartbeatHandoffSyncBoundary::Handoff,
    ] {
        let (parent, repo_path, source, repo, snapshot) =
            anchored_startup_heartbeat_fixture("retained-fsync");
        let inode = std::fs::metadata(&source).unwrap().ino();
        assert!(super::handoff_retained_heartbeat(
            &repo_path,
            &repo,
            source.file_name().unwrap(),
            &snapshot,
            |_, _, _, _| {},
            |boundary| (boundary != failed)
                .then_some(())
                .ok_or_else(|| super::CommandFailure::diagnostic("injected fsync failure")),
        )
        .is_err());
        for _ in 0..2 {
            let retained = super::handoff_retained_heartbeat(
                &repo_path,
                &repo,
                source.file_name().unwrap(),
                &snapshot,
                |_, _, _, _| {},
                |_| Ok(()),
            )
            .unwrap();
            assert_eq!(std::fs::metadata(retained).unwrap().ino(), inode);
        }
        std::fs::remove_dir_all(parent).unwrap();
    }

    for mutation in [
        "content",
        "mode",
        "binding",
        "cleanup-content",
        "cleanup-mode",
        "cleanup-binding",
        "cleanup-source",
    ] {
        let (parent, repo_path, source, repo, snapshot) =
            anchored_startup_heartbeat_fixture(mutation);
        let expected = expected_startup_heartbeat("host:user:rust:4242:nonce-a");
        let (_, completed) = super::heartbeat_receipt_names(expected);
        let archive = repo_path
            .join("quarantine/startup-heartbeat-handoffs")
            .join(completed.replace(".receipt", ".json"));
        let result = super::handoff_retained_heartbeat(
            &repo_path,
            &repo,
            source.file_name().unwrap(),
            &snapshot,
            |boundary, _, _, _| {
                if boundary == "final" && !mutation.starts_with("cleanup-") {
                    mutate_retained(&archive, &source, mutation);
                }
            },
            |boundary| {
                if boundary == super::HeartbeatHandoffSyncBoundary::Cleanup
                    && mutation.starts_with("cleanup-")
                {
                    mutate_retained(&archive, &source, mutation);
                }
                Ok(())
            },
        );
        assert!(result.is_err(), "{mutation}");
        let receipts = super::open_receipt_anchors_with_hook(&repo, |_| {}).unwrap();
        assert!(
            super::inspect_heartbeat_receipt(&receipts, completed.as_ref())
                == super::HeartbeatReceiptEntry::Missing,
            "{mutation}"
        );
        std::fs::remove_dir_all(parent).unwrap();
    }
}

#[cfg(target_os = "linux")]
#[test]
fn retained_bridge_predecessor_authority_is_exact_and_boundary_bound() {
    use nix::fcntl::{open, OFlag};
    use nix::sys::stat::Mode;
    use std::cell::Cell;

    let _guard = STARTUP_HEARTBEAT_ENV.lock().unwrap();
    let previous = std::env::var_os("AUTOSPEC_HEARTBEAT_DIR");
    let (sandbox, _) = startup_heartbeat_fixture("retained-bridge-authority");
    let root = sandbox.join("heartbeats");
    std::fs::create_dir(&root).unwrap();
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
    let repo_name = crate::commands::autonomous::drain::repository_progress_key("owner/repo");
    let repo_path = root.join(&repo_name);
    std::fs::create_dir(&repo_path).unwrap();
    std::fs::set_permissions(&repo_path, std::fs::Permissions::from_mode(0o700)).unwrap();
    let source = repo_path.join("42.json");
    std::fs::write(
        &source,
        startup_heartbeat_document("host:user:rust:4242:nonce-a", 4242),
    )
    .unwrap();
    std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o600)).unwrap();
    let snapshot = expired_heartbeat_snapshot(&source);
    let root_fd = std::fs::File::from(
        open(
            &root,
            OFlag::O_PATH | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW,
            Mode::empty(),
        )
        .unwrap(),
    );
    let repo =
        super::open_heartbeat_directory_beneath(&root_fd, Path::new(&repo_name)).unwrap();
    let archive = super::handoff_retained_heartbeat(
        &repo_path,
        &repo,
        source.file_name().unwrap(),
        &snapshot,
        |_, _, _, _| {},
        |_| Ok(()),
    )
    .unwrap();
    unsafe { std::env::set_var("AUTOSPEC_HEARTBEAT_DIR", &root) };
    let identity = super::ClaimMutationIdentity {
        repo: "owner/repo",
        issue: 42,
        worker_id: "host:user:rust:4242:nonce-a",
        branch: "feat/worker",
        claim_id: "claim-a",
    };
    let calls = Cell::new(0usize);
    let accepted = super::with_retained_bridge_predecessor_authority(
        identity,
        |_, _, _, _, _| super::StartupPidLiveness::Dead,
        || {},
        || {
            calls.set(calls.get() + 1);
            Ok("transferred")
        },
    )
    .unwrap();
    assert_eq!(accepted, Some("transferred"));
    assert_eq!(calls.get(), 1);

    let missing = super::with_retained_bridge_predecessor_authority(
        super::ClaimMutationIdentity {
            claim_id: "another-claim",
            ..identity
        },
        |_, _, _, _, _| super::StartupPidLiveness::Dead,
        || {},
        || {
            calls.set(calls.get() + 1);
            Ok(())
        },
    )
    .unwrap();
    assert!(missing.is_none());
    let live = super::with_retained_bridge_predecessor_authority(
        identity,
        |_, _, _, _, _| super::StartupPidLiveness::Live,
        || {},
        || {
            calls.set(calls.get() + 1);
            Ok(())
        },
    );
    assert!(live.is_err());
    assert_eq!(calls.get(), 1);

    let receipt = archive.with_extension("receipt");
    let replacement = repo_path.join("replacement-receipt");
    std::fs::write(&replacement, "").unwrap();
    std::fs::set_permissions(&replacement, std::fs::Permissions::from_mode(0o600)).unwrap();
    let raced = super::with_retained_bridge_predecessor_authority(
        identity,
        |_, _, _, _, _| super::StartupPidLiveness::Dead,
        || {
            std::fs::remove_file(&receipt).unwrap();
            std::fs::rename(&replacement, &receipt).unwrap();
        },
        || {
            calls.set(calls.get() + 1);
            Ok(())
        },
    );
    assert!(raced.is_err());
    assert_eq!(calls.get(), 1, "raced proof must not invoke transfer");

    match previous {
        Some(value) => unsafe { std::env::set_var("AUTOSPEC_HEARTBEAT_DIR", value) },
        None => unsafe { std::env::remove_var("AUTOSPEC_HEARTBEAT_DIR") },
    }
    std::fs::remove_dir_all(sandbox).unwrap();
}

#[test]
fn paginated_comments_parser_flattens_two_raw_pages() {
    // Break caught: treating each slurped page array as a comment object.
    let comments = super::parse_paginated_comments_json(
        r#"[
          [{"id":100,"body":"first","updated_at":"2026-07-27T00:00:00Z","user":{"login":"autospec"}}],
          [{"id":101,"body":null,"updated_at":null,"reactions":{"total_count":2}}]
        ]"#,
    )
    .expect("nested GitHub comment pages");

    assert_eq!(
        comments,
        vec![
            RemoteComment::new(100, "first", "2026-07-27T00:00:00Z"),
            RemoteComment::new(101, "", ""),
        ]
    );
}

#[test]
fn paginated_comments_parser_preserves_flat_single_page_fixtures() {
    // Break caught: requiring an outer page array would break existing projected fixtures.
    let comments = super::parse_paginated_comments_json(
        r#"[{"id":100,"body":"state","updated_at":"2026-07-27T00:00:00Z"}]"#,
    )
    .expect("flat projected fixture");

    assert_eq!(
        comments,
        vec![RemoteComment::new(100, "state", "2026-07-27T00:00:00Z")]
    );
}

#[test]
fn paginated_comments_parser_rejects_mixed_pages_and_malformed_fields() {
    // Break caught: partially flattening a mixed payload could silently drop comments.
    assert!(super::parse_paginated_comments_json(
        r#"[[{"id":100,"body":"state","updated_at":"now"}],{"id":101,"body":"other","updated_at":"now"}]"#,
    )
    .is_err());

    for malformed in [
        r#"[{"id":"100","body":"state","updated_at":"now"}]"#,
        r#"[{"id":100,"body":false,"updated_at":"now"}]"#,
        r#"[{"id":100,"body":"state","updated_at":[]}]"#,
    ] {
        assert!(
            super::parse_paginated_comments_json(malformed).is_err(),
            "accepted malformed typed comment: {malformed}"
        );
    }
}

#[test]
fn claim_settle_millis_preserves_decimal_second_configuration() {
    assert_eq!(claim_settle_millis(None, Some("0.2")), Some(200));
    assert_eq!(claim_settle_millis(None, Some("1.2349")), Some(1_234));
    assert_eq!(claim_settle_millis(None, Some("0")), Some(0));
}

#[test]
fn claim_settle_millis_prefers_explicit_milliseconds_and_rejects_invalid_seconds() {
    assert_eq!(claim_settle_millis(Some("17"), Some("0.2")), Some(17));
    assert_eq!(claim_settle_millis(None, Some("-0.2")), None);
    assert_eq!(claim_settle_millis(None, Some("not-a-duration")), None);
}

#[test]
fn session_binding_is_create_once_and_idempotent_for_the_same_identity() {
    let directory = std::env::temp_dir().join(format!(
        "autospec-session-binding-{}-{}",
        std::process::id(),
        super::UNIQUE_ID_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&directory).expect("session binding fixture");
    let path = directory.join("session.json");
    let original = SessionBindingIdentity::new("session", 42, "worker", "feat/a", "claim-a");
    let successor = SessionBindingIdentity::new("session", 42, "worker", "feat/a", "claim-b");

    let original_document = br#"{"issue":"42","branch":"feat/a","worker_id":"worker","claim_id":"claim-a","session_id":"session"}"#;
    let refresh_document = br#"{"issue":"42","branch":"feat/a","worker_id":"worker","claim_id":"claim-a","session_id":"session","step":"refresh"}"#;
    let successor_document = br#"{"issue":"42","branch":"feat/a","worker_id":"worker","claim_id":"claim-b","session_id":"session"}"#;
    publish_session_binding(&path, original_document, &original).expect("initial binding");
    publish_session_binding(&path, refresh_document, &original).expect("idempotent refresh");
    assert!(publish_session_binding(&path, successor_document, &successor).is_err());
    assert_eq!(
        std::fs::read(&path).expect("preserved binding"),
        original_document
    );
    std::fs::remove_dir_all(directory).expect("remove session binding fixture");
}

#[test]
fn concurrent_session_binding_publishers_cannot_overwrite_the_winner() {
    let directory = std::env::temp_dir().join(format!(
        "autospec-session-binding-race-{}-{}",
        std::process::id(),
        super::UNIQUE_ID_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&directory).expect("session binding fixture");
    let path = Arc::new(directory.join("session.json"));
    let barrier = Arc::new(Barrier::new(3));
    let handles = [
        (
            "claim-a",
            br#"{"issue":"42","branch":"feat/a","worker_id":"worker","claim_id":"claim-a","session_id":"session"}"#.as_slice(),
        ),
        (
            "claim-b",
            br#"{"issue":"42","branch":"feat/a","worker_id":"worker","claim_id":"claim-b","session_id":"session"}"#.as_slice(),
        ),
    ]
        .into_iter()
        .map(|(claim_id, document)| {
            let path = Arc::clone(&path);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                let identity =
                    SessionBindingIdentity::new("session", 42, "worker", "feat/a", claim_id);
                barrier.wait();
                publish_session_binding(&path, document, &identity)
            })
        })
        .collect::<Vec<_>>();
    barrier.wait();
    let results = handles
        .into_iter()
        .map(|handle| handle.join().expect("publisher thread"))
        .collect::<Vec<_>>();

    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    let stored = std::fs::read_to_string(&*path).expect("winning session binding");
    assert!(stored.contains("claim-a") || stored.contains("claim-b"));
    std::fs::remove_dir_all(directory).expect("remove session binding fixture");
}

struct ClaimRefFixture {
    root: PathBuf,
    remote: PathBuf,
    clients: [PathBuf; 2],
}

impl ClaimRefFixture {
    fn new(label: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "autospec-claim-ref-{label}-{}-{}",
            std::process::id(),
            super::UNIQUE_ID_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let remote = root.join("remote.git");
        std::fs::create_dir_all(&root).expect("claim ref fixture root");
        git(&root, &["init", "--bare", remote.to_str().unwrap()]);
        let clients = [root.join("client-a"), root.join("client-b")];
        for client in &clients {
            git(&root, &["init", client.to_str().unwrap()]);
        }
        Self {
            root,
            remote,
            clients,
        }
    }
}

impl Drop for ClaimRefFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn git(directory: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(directory)
        .output()
        .expect("run git fixture command");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_stdout(directory: &Path, args: &[&str]) -> Vec<u8> {
    let output = Command::new("git")
        .args(args)
        .current_dir(directory)
        .output()
        .expect("run git fixture command");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn source_function<'a>(source: &'a str, name: &str) -> &'a str {
    let marker = format!("fn {name}(");
    let start = source.find(&marker).expect("source function");
    let tail = &source[start..];
    let end = tail[marker.len()..]
        .find("\nfn ")
        .map(|end| marker.len() + end)
        .unwrap_or(tail.len());
    &tail[..end]
}

fn claim_record(worker: &str, claim_id: &str, state: &str) -> RunStateRecord {
    RunStateRecord::new(
        "owner/repo",
        42,
        worker,
        state,
        format!("feat/{worker}"),
        "",
        state,
        Vec::new(),
        "2026-07-25T00:00:00Z",
        "2026-07-25T00:00:00Z",
        1,
    )
    .with_claim_id(claim_id)
}

fn lifecycle_evidence(record: &RunStateRecord) -> Result<ClaimEvidence, super::CommandFailure> {
    lifecycle_claim_evidence_from_record(
        RepositoryScope::try_from("owner/repo").expect("repository scope"),
        IssueNumber::new(42).expect("issue"),
        WorkerId::try_from("worker-requested").expect("requested worker"),
        ClaimBranch::try_from("feat/requested").expect("requested branch"),
        record,
    )
}

#[test]
fn lifecycle_claim_state_matrix_reuses_only_valid_available_generations() {
    let requested = ClaimEvidence::Observed(ClaimContext::active(
        RepositoryScope::try_from("owner/repo").expect("repository scope"),
        IssueNumber::new(42).expect("issue"),
        WorkerId::try_from("worker-requested").expect("requested worker"),
        ClaimBranch::try_from("feat/requested").expect("requested branch"),
        LeaseFreshness::Fresh,
    ));
    for state in ["released", "retryable"] {
        assert_eq!(
            lifecycle_evidence(&claim_record("worker-old", "claim-old", state))
                .expect("reusable claim evidence"),
            requested,
            "{state} must be available to the requested owner"
        );
    }

    for state in ["claimed", "failed", "needs-human"] {
        let mut record = claim_record("worker-old", "claim-old", state);
        record.updated_at = "2999-01-01T00:00:00Z".to_string();
        assert_eq!(
            lifecycle_evidence(&record).expect("active claim evidence"),
            ClaimEvidence::Observed(ClaimContext::active(
                RepositoryScope::try_from("owner/repo").expect("repository scope"),
                IssueNumber::new(42).expect("issue"),
                WorkerId::try_from("worker-old").expect("recorded worker"),
                ClaimBranch::try_from("feat/worker-old").expect("recorded branch"),
                LeaseFreshness::Fresh,
            )),
            "{state} must retain the recorded owner"
        );
    }

    assert_eq!(
        lifecycle_evidence(&claim_record("worker-old", "claim-old", "merged"))
            .expect("merged claim evidence"),
        ClaimEvidence::Observed(ClaimContext::terminal(
            RepositoryScope::try_from("owner/repo").expect("repository scope"),
            IssueNumber::new(42).expect("issue"),
            WorkerId::try_from("worker-old").expect("recorded worker"),
            ClaimBranch::try_from("feat/worker-old").expect("recorded branch"),
        ))
    );
}

#[test]
fn lifecycle_claim_rejects_malformed_reusable_generations() {
    let mut empty_branch = claim_record("worker-old", "claim-old", "released");
    empty_branch.branch.clear();
    assert!(lifecycle_evidence(&empty_branch).is_err());

    let mut missing_generation = claim_record("worker-old", "claim-old", "retryable");
    missing_generation.claim_id = None;
    assert!(lifecycle_evidence(&missing_generation).is_err());
}

#[test]
fn bridge_terminal_projection_resumes_only_the_exact_terminal_generation() {
    let identity = super::ClaimMutationIdentity {
        repo: "owner/repo",
        issue: 42,
        worker_id: "worker-a",
        branch: "feat/worker-a",
        claim_id: "claim-a",
    };
    let mut terminal = claim_record("worker-a", "claim-a", "merged");
    terminal.step = "merged".into();
    terminal.pr = "17".into();
    assert_eq!(
        super::bridge_terminal_mode(
            &terminal,
            identity,
            "merged",
            "merged",
            "17",
            "terminal_prepared:merged",
        ),
        super::BridgeTerminalMode::Complete
    );

    terminal.claim_id = Some("claim-takeover".into());
    assert_eq!(
        super::bridge_terminal_mode(
            &terminal,
            identity,
            "merged",
            "merged",
            "17",
            "terminal_prepared:merged",
        ),
        super::BridgeTerminalMode::Lost
    );
}

#[test]
fn bridge_result_recovery_collapses_only_identical_publication_retries() {
    let evidence = ExecutorResultEvidence::new(
        "owner/repo",
        42,
        "worker-a",
        "feat/worker-a",
        "succeeded",
        Some(17),
        "executor_succeeded",
        "stable-receipt",
        Some("claim-a".into()),
        Some("a".repeat(40)),
        Some("b".repeat(64)),
    );
    let body = evidence.to_marked_comment();
    let comments = vec![
        RemoteComment::new(1, body.clone(), "2026-07-26T00:00:00Z"),
        RemoteComment::new(2, body, "2026-07-26T00:00:01Z"),
    ];
    assert_eq!(
        super::exact_successful_executor_result(
            &comments,
            super::ClaimMutationIdentity {
                repo: "owner/repo",
                issue: 42,
                worker_id: "worker-a",
                branch: "feat/worker-a",
                claim_id: "claim-a",
            },
            super::ExecutorResultAuthorityBinding {
                pull_request: 17,
                commit: &"a".repeat(40),
                premerge_receipt: &"b".repeat(64),
                receipt_id: "stable-receipt",
            },
        ),
        Some(evidence)
    );
}

#[test]
fn bridge_result_authority_filters_exact_generation_before_uniqueness() {
    let old = ExecutorResultEvidence::new(
        "owner/repo",
        42,
        "worker-a",
        "feat/worker-a",
        "succeeded",
        Some(17),
        "executor_succeeded",
        "bridge-old",
        Some("claim-old".into()),
        Some("a".repeat(40)),
        Some("b".repeat(64)),
    );
    let successor = ExecutorResultEvidence::new(
        "owner/repo",
        42,
        "worker-a",
        "feat/worker-a",
        "succeeded",
        Some(17),
        "executor_succeeded",
        "bridge-successor",
        Some("claim-successor".into()),
        Some("a".repeat(40)),
        Some("b".repeat(64)),
    );
    let comments = vec![
        RemoteComment::new(1, old.to_marked_comment(), "2026-07-26T00:00:00Z"),
        RemoteComment::new(2, successor.to_marked_comment(), "2026-07-26T00:00:01Z"),
        RemoteComment::new(3, successor.to_marked_comment(), "2026-07-26T00:00:02Z"),
    ];

    assert_eq!(
        super::exact_successful_executor_result(
            &comments,
            super::ClaimMutationIdentity {
                repo: "owner/repo",
                issue: 42,
                worker_id: "worker-a",
                branch: "feat/worker-a",
                claim_id: "claim-successor",
            },
            super::ExecutorResultAuthorityBinding {
                pull_request: 17,
                commit: &"a".repeat(40),
                premerge_receipt: &"b".repeat(64),
                receipt_id: "bridge-successor",
            },
        ),
        Some(successor)
    );
}

#[cfg(unix)]
#[test]
fn executor_result_takeover_after_renewal_is_inert_for_successor_authority() {
    let _guard = BRIDGE_TRANSITION_ENV.lock().expect("transition env");
    let fixture = ClaimRefFixture::new("executor-result-takeover");
    let bin = fixture.root.join("bin");
    let gh = bin.join("gh");
    let comments = fixture.root.join("comments.json");
    let observed_renewal = fixture.root.join("observed-renewal.txt");
    std::fs::create_dir(&bin).expect("bin");
    std::fs::write(&comments, "[]").expect("comments");
    std::fs::write(
        &gh,
        "#!/bin/sh\n\
         set -eu\n\
         if [ \"$1\" = api ]; then cat \"$GH_COMMENTS\"; exit 0; fi\n\
         if [ \"$1 $2\" = 'pr list' ]; then cat <<'EOF'\n\
         [{\"number\":17,\"body\":\"Closes #42\\n\\n## Closeout report\\nResult\",\"headRefName\":\"feat/worker-a\",\"headRefOid\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",\"isDraft\":false,\"baseRefName\":\"main\"}]\n\
         EOF\n\
         exit 0; fi\n\
         if [ \"$1 $2\" = 'issue comment' ]; then\n\
           body=''; while [ \"$#\" -gt 0 ]; do if [ \"$1\" = --body ]; then shift; body=$1; fi; shift || true; done\n\
           jq --arg body \"$body\" '. + [{id:(length + 1),body:$body,updated_at:\"2026-07-26T00:00:00Z\"}]' \"$GH_COMMENTS\" > \"$GH_COMMENTS.tmp\"\n\
           mv \"$GH_COMMENTS.tmp\" \"$GH_COMMENTS\"\n\
           current=$(git --git-dir=\"$GH_REMOTE\" rev-parse refs/autospec/claims/issue-42)\n\
           git --git-dir=\"$GH_REMOTE\" show -s --format=%B \"$current\" > \"$GH_OBSERVED_RENEWAL\"\n\
           git --git-dir=\"$GH_REMOTE\" update-ref refs/autospec/claims/issue-42 \"$GH_TAKEOVER_OID\"\n\
           exit 0\n\
         fi\n\
         exit 64\n",
    )
    .expect("gh");
    std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).expect("gh mode");

    let mut original = claim_record("worker-a", "claim-old", "claimed");
    original.ttl_seconds = u64::MAX;
    let initial = match advance_claim_ref_in(
        Path::new("git"),
        &fixture.clients[0],
        fixture.remote.to_str().unwrap(),
        "owner/repo",
        42,
        None,
        &original,
    )
    .expect("seed claim")
    {
        ClaimRefAdvance::Won(head) => head,
        ClaimRefAdvance::Lost => panic!("initial claim must win"),
    };
    git(
        &fixture.clients[1],
        &[
            "fetch",
            fixture.remote.to_str().unwrap(),
            "refs/autospec/claims/issue-42",
        ],
    );
    let mut successor = claim_record("worker-a", "claim-successor", "claimed");
    successor.ttl_seconds = u64::MAX;
    let successor_oid = create_claim_ref_commit(
        Path::new("git"),
        &fixture.clients[1],
        Some(&initial),
        "successor-generation",
        &successor,
    )
    .expect("create successor claim");
    git(
        &fixture.clients[1],
        &[
            "push",
            fixture.remote.to_str().unwrap(),
            &format!("{successor_oid}:refs/autospec/test/successor"),
        ],
    );

    let old_path = std::env::var_os("PATH");
    let old_remote = std::env::var_os("AUTOSPEC_CLAIM_GIT_REMOTE");
    let old_state = std::env::var_os("AUTOSPEC_CLAIM_GIT_STATE_DIR");
    let old_comments = std::env::var_os("GH_COMMENTS");
    let old_gh_remote = std::env::var_os("GH_REMOTE");
    let old_takeover = std::env::var_os("GH_TAKEOVER_OID");
    let old_observed = std::env::var_os("GH_OBSERVED_RENEWAL");
    let old_retries = std::env::var_os("AUTOSPEC_GH_API_RETRIES");
    std::env::set_var(
        "PATH",
        format!(
            "{}:{}",
            bin.display(),
            old_path.as_deref().unwrap_or_default().to_string_lossy()
        ),
    );
    std::env::set_var("AUTOSPEC_CLAIM_GIT_REMOTE", &fixture.remote);
    std::env::set_var(
        "AUTOSPEC_CLAIM_GIT_STATE_DIR",
        fixture.root.join("claim-state"),
    );
    std::env::set_var("GH_COMMENTS", &comments);
    std::env::set_var("GH_REMOTE", &fixture.remote);
    std::env::set_var("GH_TAKEOVER_OID", &successor_oid);
    std::env::set_var("GH_OBSERVED_RENEWAL", &observed_renewal);
    std::env::set_var("AUTOSPEC_GH_API_RETRIES", "1");

    let old_identity = super::ClaimMutationIdentity {
        repo: "owner/repo",
        issue: 42,
        worker_id: "worker-a",
        branch: "feat/worker-a",
        claim_id: "claim-old",
    };
    assert_eq!(
        super::record_executor_result_with_receipt(
            old_identity,
            &autospec_core::coordination::ConductorOutcome::Succeeded,
            Some(17),
            Some(super::ExecutorSuccessBinding {
                claim_id: "claim-old",
                commit: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                premerge_receipt:
                    "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            }),
            Some("bridge-old"),
        )
        .expect("publish stale result"),
        super::ExecutorResultRecord::OwnershipLost
    );
    let renewal = std::fs::read_to_string(&observed_renewal).expect("renewal observation");
    assert!(
        renewal.contains("\"step\":\"executor_succeeded\""),
        "{renewal}"
    );
    assert!(!renewal.contains("\"updated_at\":\"2026-07-25T00:00:00Z\""));

    let successor_evidence = ExecutorResultEvidence::new(
        "owner/repo",
        42,
        "worker-a",
        "feat/worker-a",
        "succeeded",
        Some(17),
        "executor_succeeded",
        "bridge-successor",
        Some("claim-successor".into()),
        Some("a".repeat(40)),
        Some("b".repeat(64)),
    );
    let mut published: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&comments).expect("comments"))
            .expect("comments JSON");
    published
        .as_array_mut()
        .expect("comment array")
        .push(serde_json::json!({
            "id": 2,
            "body": successor_evidence.to_marked_comment(),
            "updated_at": "2026-07-26T00:00:01Z"
        }));
    std::fs::write(
        &comments,
        serde_json::to_vec(&published).expect("serialize comments"),
    )
    .expect("publish successor evidence");

    assert_eq!(
        super::authoritative_executor_result(
            super::ClaimMutationIdentity {
                repo: "owner/repo",
                issue: 42,
                worker_id: "worker-a",
                branch: "feat/worker-a",
                claim_id: "claim-successor",
            },
            super::ExecutorResultAuthorityBinding {
                pull_request: 17,
                commit: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                premerge_receipt:
                    "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                receipt_id: "bridge-successor",
            },
        )
        .expect("read successor authority"),
        Some(successor_evidence)
    );

    for (key, value) in [
        ("PATH", old_path),
        ("AUTOSPEC_CLAIM_GIT_REMOTE", old_remote),
        ("AUTOSPEC_CLAIM_GIT_STATE_DIR", old_state),
        ("GH_COMMENTS", old_comments),
        ("GH_REMOTE", old_gh_remote),
        ("GH_TAKEOVER_OID", old_takeover),
        ("GH_OBSERVED_RENEWAL", old_observed),
        ("AUTOSPEC_GH_API_RETRIES", old_retries),
    ] {
        match value {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }
    }
}

#[cfg(unix)]
fn assert_bridge_transition_projection(
    label: &str,
    disposition: super::BridgeClaimDisposition,
    expected_state: &str,
    expected_prepared_step: &str,
    expected_edit: &str,
    expected_comments: usize,
    interrupt_after_preparation: bool,
) {
    let fixture = ClaimRefFixture::new(label);
    let bin = fixture.root.join("bin");
    let gh = bin.join("gh");
    let calls = fixture.root.join("gh-calls");
    let comments = fixture.root.join("comments.json");
    let label_claims = fixture.root.join("label-claims");
    let first_label_failed = fixture.root.join("first-label-failed");
    std::fs::create_dir(&bin).expect("bin");
    std::fs::write(&comments, "[]").expect("comments");
    std::fs::write(
        &gh,
        "#!/bin/sh\n\
         set -eu\n\
         printf '%s\\n' \"$*\" >> \"$GH_CALLS\"\n\
         if [ \"$1\" = api ]; then cat \"$GH_COMMENTS\"; exit 0; fi\n\
         if [ \"$1 $2\" = 'issue comment' ]; then\n\
           body=''; while [ \"$#\" -gt 0 ]; do if [ \"$1\" = --body ]; then shift; body=$1; fi; shift || true; done\n\
           jq --arg body \"$body\" '. + [{id:(length + 1),body:$body,updated_at:\"2026-07-26T00:00:00Z\"}]' \"$GH_COMMENTS\" > \"$GH_COMMENTS.tmp\"\n\
           mv \"$GH_COMMENTS.tmp\" \"$GH_COMMENTS\"; exit 0\n\
         fi\n\
         if [ \"$1 $2\" = 'issue edit' ]; then\n\
           current=$(git -C \"$GH_REMOTE\" rev-parse refs/autospec/claims/issue-42)\n\
           printf '%s\\n' \"$current\" >> \"$GH_LABEL_CLAIMS\"\n\
           if [ \"${GH_FAIL_FIRST_LABEL:-0}\" = 1 ] && [ ! -e \"$GH_FIRST_LABEL_FAILED\" ]; then\n\
             : > \"$GH_FIRST_LABEL_FAILED\"\n\
             exit 23\n\
           fi\n\
           exit 0\n\
         fi\n\
         exit 64\n",
    )
    .expect("gh");
    std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).expect("gh mode");
    let old_path = std::env::var_os("PATH");
    let old_remote = std::env::var_os("AUTOSPEC_CLAIM_GIT_REMOTE");
    let old_state = std::env::var_os("AUTOSPEC_CLAIM_GIT_STATE_DIR");
    let old_calls = std::env::var_os("GH_CALLS");
    let old_comments = std::env::var_os("GH_COMMENTS");
    let old_gh_remote = std::env::var_os("GH_REMOTE");
    let old_label_claims = std::env::var_os("GH_LABEL_CLAIMS");
    let old_fail_first_label = std::env::var_os("GH_FAIL_FIRST_LABEL");
    let old_first_label_failed = std::env::var_os("GH_FIRST_LABEL_FAILED");
    let old_retries = std::env::var_os("AUTOSPEC_GH_API_RETRIES");
    let old_heartbeat = std::env::var_os("AUTOSPEC_HEARTBEAT_DIR");
    std::env::set_var(
        "PATH",
        format!(
            "{}:{}",
            bin.display(),
            old_path.as_deref().unwrap_or_default().to_string_lossy()
        ),
    );
    std::env::set_var("AUTOSPEC_CLAIM_GIT_REMOTE", &fixture.remote);
    std::env::set_var(
        "AUTOSPEC_CLAIM_GIT_STATE_DIR",
        fixture.root.join("claim-state"),
    );
    std::env::set_var("GH_CALLS", &calls);
    std::env::set_var("GH_COMMENTS", &comments);
    std::env::set_var("GH_REMOTE", &fixture.remote);
    std::env::set_var("GH_LABEL_CLAIMS", &label_claims);
    std::env::set_var(
        "GH_FAIL_FIRST_LABEL",
        if interrupt_after_preparation {
            "1"
        } else {
            "0"
        },
    );
    std::env::set_var("GH_FIRST_LABEL_FAILED", &first_label_failed);
    std::env::set_var("AUTOSPEC_GH_API_RETRIES", "1");
    std::env::set_var("AUTOSPEC_HEARTBEAT_DIR", fixture.root.join("heartbeats"));

    let claimed = claim_record("worker-a", "claim-a", "claimed");
    assert!(matches!(
        advance_claim_ref_in(
            Path::new("git"),
            &fixture.clients[0],
            fixture.remote.to_str().unwrap(),
            "owner/repo",
            42,
            None,
            &claimed,
        )
        .expect("seed authoritative claim"),
        ClaimRefAdvance::Won(_)
    ));
    let identity = super::ClaimMutationIdentity {
        repo: "owner/repo",
        issue: 42,
        worker_id: "worker-a",
        branch: "feat/worker-a",
        claim_id: "claim-a",
    };
    if disposition == super::BridgeClaimDisposition::Retryable {
        super::write_startup_heartbeat(
            identity.repo,
            identity.issue,
            identity.worker_id,
            identity.branch,
            identity.claim_id,
            None,
        )
        .expect("retryable heartbeat");
    }
    let pr = (disposition == super::BridgeClaimDisposition::Merged).then_some(17);
    if interrupt_after_preparation {
        super::transition_bridge_claim(identity, pr, disposition)
            .expect_err("first label projection must interrupt after preparation");
        let prepared = super::read_claim_ref("owner/repo", 42)
            .expect("read prepared claim")
            .expect("prepared claim head");
        assert_eq!(prepared.record.state, "claimed");
        assert_eq!(prepared.record.step, expected_prepared_step);
    }
    assert_eq!(
        super::transition_bridge_claim(identity, pr, disposition)
            .expect("transition or prepared restart"),
        super::BridgeClaimTransition::Transitioned
    );
    assert_eq!(
        super::transition_bridge_claim(identity, pr, disposition).expect("resume projection"),
        super::BridgeClaimTransition::Transitioned
    );
    let head = super::read_claim_ref("owner/repo", 42)
        .expect("read claim")
        .expect("claim head");
    assert_eq!(head.record.state, expected_state);
    assert_ne!(head.record.state, "claimed");
    let call_log = std::fs::read_to_string(calls).expect("calls");
    assert!(call_log.contains(expected_edit), "{call_log}");
    assert_eq!(
        call_log.matches(expected_edit).count(),
        if interrupt_after_preparation { 2 } else { 1 },
        "terminal restart must not reapply labels: {call_log}"
    );
    let label_claim_oid =
        std::fs::read_to_string(&label_claims).expect("claim observed during label projection");
    let label_claim_oid = label_claim_oid.lines().next().expect("label claim oid");
    let label_claim = String::from_utf8(git_stdout(
        &fixture.root,
        &[
            "-C",
            fixture.remote.to_str().expect("remote path"),
            "cat-file",
            "commit",
            label_claim_oid,
        ],
    ))
    .expect("claim commit utf8");
    let (_, label_claim) = label_claim
        .split_once("\n\n")
        .expect("claim commit message");
    let prepared =
        super::parse_claim_ref_message("a".repeat(40), label_claim, "owner/repo", 42)
            .expect("prepared terminal claim");
    assert_eq!(prepared.record.state, "claimed");
    assert_eq!(prepared.record.step, expected_prepared_step);
    let comment_value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(comments).expect("comments JSON"))
            .expect("comments");
    assert_eq!(
        comment_value.as_array().expect("comment array").len(),
        expected_comments,
        "restart duplicated a projection"
    );

    for (key, value) in [
        ("PATH", old_path),
        ("AUTOSPEC_CLAIM_GIT_REMOTE", old_remote),
        ("AUTOSPEC_CLAIM_GIT_STATE_DIR", old_state),
        ("GH_CALLS", old_calls),
        ("GH_COMMENTS", old_comments),
        ("GH_REMOTE", old_gh_remote),
        ("GH_LABEL_CLAIMS", old_label_claims),
        ("GH_FAIL_FIRST_LABEL", old_fail_first_label),
        ("GH_FIRST_LABEL_FAILED", old_first_label_failed),
        ("AUTOSPEC_GH_API_RETRIES", old_retries),
        ("AUTOSPEC_HEARTBEAT_DIR", old_heartbeat),
    ] {
        match value {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }
    }
}

#[cfg(unix)]
#[test]
fn bridge_terminal_transitions_prepare_before_labels_and_restarts_do_not_relabel() {
    let _guard = BRIDGE_TRANSITION_ENV.lock().expect("transition env");
    let retryable_edit = [
        "issue edit 42 ",
        "repo owner/repo ",
        "remove-label in-progress-by-bot ",
        "remove-label autospec:needs-human ",
        "add-label auto-implement",
    ]
    .join("--");
    assert_bridge_transition_projection(
        "bridge-retryable",
        super::BridgeClaimDisposition::Retryable,
        "released",
        "terminal_prepared:retryable_released",
        &retryable_edit,
        1,
        false,
    );
    assert_bridge_transition_projection(
        "bridge-needs-human",
        super::BridgeClaimDisposition::NeedsHuman,
        "failed",
        "terminal_prepared:needs_human",
        "issue edit 42 --repo owner/repo --remove-label in-progress-by-bot --remove-label auto-implement --add-label autospec:needs-human",
        1,
        false,
    );
    assert_bridge_transition_projection(
        "bridge-merged",
        super::BridgeClaimDisposition::Merged,
        "merged",
        "terminal_prepared:merged",
        "issue edit 42 --repo owner/repo --remove-label in-progress-by-bot",
        2,
        false,
    );
    assert_bridge_transition_projection(
        "bridge-retryable-prepared-restart",
        super::BridgeClaimDisposition::Retryable,
        "released",
        "terminal_prepared:retryable_released",
        &retryable_edit,
        1,
        true,
    );
}

#[test]
fn claim_ref_initial_creation_has_exactly_one_receive_pack_winner() {
    // Break caught: two absent-ref acquisitions both succeeding on eventually-consistent comments.
    let fixture = ClaimRefFixture::new("initial-race");
    let barrier = Arc::new(Barrier::new(3));
    let handles = fixture
        .clients
        .clone()
        .into_iter()
        .enumerate()
        .map(|(index, client)| {
            let barrier = Arc::clone(&barrier);
            let remote = fixture.remote.clone();
            std::thread::spawn(move || {
                let worker = format!("worker-{index}");
                let record = claim_record(&worker, &format!("claim-{index}"), "claimed");
                barrier.wait();
                advance_claim_ref_in(
                    Path::new("git"),
                    &client,
                    remote.to_str().unwrap(),
                    "owner/repo",
                    42,
                    None,
                    &record,
                )
            })
        })
        .collect::<Vec<_>>();
    barrier.wait();
    let results = handles
        .into_iter()
        .map(|handle| {
            handle
                .join()
                .expect("claim publisher")
                .expect("claim result")
        })
        .collect::<Vec<_>>();

    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, ClaimRefAdvance::Won(_)))
            .count(),
        1
    );
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, ClaimRefAdvance::Lost))
            .count(),
        1
    );
}

#[test]
fn claim_ref_rejects_a_stale_terminal_transition_after_takeover() {
    // Break caught: stale release publishing an absorbing terminal comment before ownership CAS.
    let fixture = ClaimRefFixture::new("stale-terminal");
    let original = claim_record("worker-a", "claim-a", "claimed");
    let initial = advance_claim_ref_in(
        Path::new("git"),
        &fixture.clients[0],
        fixture.remote.to_str().unwrap(),
        "owner/repo",
        42,
        None,
        &original,
    )
    .expect("initial claim");
    let ClaimRefAdvance::Won(parent) = initial else {
        panic!("initial claim must win");
    };
    let takeover = claim_record("worker-b", "claim-b", "claimed");
    assert!(matches!(
        advance_claim_ref_in(
            Path::new("git"),
            &fixture.clients[1],
            fixture.remote.to_str().unwrap(),
            "owner/repo",
            42,
            Some(&parent),
            &takeover,
        )
        .expect("takeover"),
        ClaimRefAdvance::Won(_)
    ));
    let terminal = claim_record("worker-a", "claim-a", "merged");
    assert_eq!(
        advance_claim_ref_in(
            Path::new("git"),
            &fixture.clients[0],
            fixture.remote.to_str().unwrap(),
            "owner/repo",
            42,
            Some(&parent),
            &terminal,
        )
        .expect("stale terminal"),
        ClaimRefAdvance::Lost
    );

    let head = read_claim_ref_in(
        Path::new("git"),
        &fixture.clients[0],
        fixture.remote.to_str().unwrap(),
        "owner/repo",
        42,
    )
    .expect("read winner")
    .expect("claim ref");
    assert_eq!(head.record.worker_id, "worker-b");
    assert_eq!(head.record.state, "claimed");
}

#[cfg(unix)]
#[test]
fn claim_ref_keeps_an_unchanged_failed_push_transient() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = ClaimRefFixture::new("unchanged-push-failure");
    let original = claim_record("worker-a", "claim-a", "claimed");
    let ClaimRefAdvance::Won(parent) = advance_claim_ref_in(
        Path::new("git"),
        &fixture.clients[0],
        fixture.remote.to_str().unwrap(),
        "owner/repo",
        42,
        None,
        &original,
    )
    .expect("seed claim") else {
        panic!("seed claim must win");
    };
    let wrapper = fixture.root.join("git-fail-push");
    std::fs::write(
        &wrapper,
        "#!/bin/sh\nfor arg in \"$@\"; do if [ \"$arg\" = push ]; then echo outage >&2; exit 75; fi; done\nexec git \"$@\"\n",
    )
    .expect("git wrapper");
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755))
        .expect("git wrapper mode");
    let refreshed = claim_record("worker-a", "claim-a", "claimed");

    let error = advance_claim_ref_in(
        &wrapper,
        &fixture.clients[0],
        fixture.remote.to_str().unwrap(),
        "owner/repo",
        42,
        Some(&parent),
        &refreshed,
    )
    .expect_err("unchanged failed push must remain retryable");

    assert_eq!(error.kind, crate::commands::CommandFailureKind::Transient);
    assert!(
        error.message.contains("without changing"),
        "{}",
        error.message
    );
}

fn seed_claim(fixture: &ClaimRefFixture) -> super::ClaimRefHead {
    let initial = claim_record("worker-a", "claim-a", "claimed");
    match advance_claim_ref_in(
        Path::new("git"),
        &fixture.clients[0],
        fixture.remote.to_str().unwrap(),
        "owner/repo",
        42,
        None,
        &initial,
    )
    .expect("seed claim")
    {
        ClaimRefAdvance::Won(head) => *head,
        ClaimRefAdvance::Lost => panic!("seed claim lost"),
    }
}

fn race_claim_ref_transitions(
    fixture: &ClaimRefFixture,
    parent: &super::ClaimRefHead,
    records: [RunStateRecord; 2],
) -> Vec<ClaimRefAdvance> {
    let barrier = Arc::new(Barrier::new(3));
    let handles = fixture
        .clients
        .clone()
        .into_iter()
        .zip(records)
        .map(|(client, record)| {
            let barrier = Arc::clone(&barrier);
            let remote = fixture.remote.clone();
            let parent = parent.clone();
            std::thread::spawn(move || {
                barrier.wait();
                advance_claim_ref_in(
                    Path::new("git"),
                    &client,
                    remote.to_str().unwrap(),
                    "owner/repo",
                    42,
                    Some(&parent),
                    &record,
                )
            })
        })
        .collect::<Vec<_>>();
    barrier.wait();
    handles
        .into_iter()
        .map(|handle| {
            handle
                .join()
                .expect("claim publisher")
                .expect("claim result")
        })
        .collect()
}

#[test]
fn claim_ref_renewal_race_has_one_exact_parent_winner() {
    // Break caught: two renewals both extending the same generation.
    let fixture = ClaimRefFixture::new("renewal-race");
    let parent = seed_claim(&fixture);
    let mut renewal_a = parent.record.clone();
    renewal_a.updated_at = "2026-07-25T00:00:01Z".to_string();
    let mut renewal_b = parent.record.clone();
    renewal_b.updated_at = "2026-07-25T00:00:02Z".to_string();
    let results = race_claim_ref_transitions(&fixture, &parent, [renewal_a, renewal_b]);
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, ClaimRefAdvance::Won(_)))
            .count(),
        1
    );
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, ClaimRefAdvance::Lost))
            .count(),
        1
    );
}

#[test]
fn claim_ref_takeover_and_renewal_each_win_when_received_first() {
    // Break caught: eventual comment ordering allowing both a takeover and renewal.
    for takeover_first in [false, true] {
        let fixture = ClaimRefFixture::new(if takeover_first {
            "takeover-first"
        } else {
            "renewal-first"
        });
        let parent = seed_claim(&fixture);
        let mut renewal = parent.record.clone();
        renewal.updated_at = "2026-07-25T00:00:01Z".to_string();
        let takeover = claim_record("worker-b", "claim-b", "claimed");
        let ordered = if takeover_first {
            [takeover, renewal]
        } else {
            [renewal, takeover]
        };
        let first = advance_claim_ref_in(
            Path::new("git"),
            &fixture.clients[0],
            fixture.remote.to_str().unwrap(),
            "owner/repo",
            42,
            Some(&parent),
            &ordered[0],
        )
        .expect("first transition");
        assert!(matches!(first, ClaimRefAdvance::Won(_)));
        let second = advance_claim_ref_in(
            Path::new("git"),
            &fixture.clients[1],
            fixture.remote.to_str().unwrap(),
            "owner/repo",
            42,
            Some(&parent),
            &ordered[1],
        )
        .expect("second transition");
        assert_eq!(second, ClaimRefAdvance::Lost);
        let head = read_claim_ref_in(
            Path::new("git"),
            &fixture.clients[0],
            fixture.remote.to_str().unwrap(),
            "owner/repo",
            42,
        )
        .expect("read winner")
        .expect("claim ref");
        assert_eq!(head.record.worker_id, ordered[0].worker_id);
        assert_eq!(head.record.claim_id, ordered[0].claim_id);
    }
}

#[test]
fn claim_ref_ambiguous_push_rereads_without_a_second_mutation() {
    // Break caught: retrying a POST/push after receive-pack committed but the response vanished.
    use std::os::unix::fs::PermissionsExt;

    let fixture = ClaimRefFixture::new("ambiguous-push");
    let wrapper = fixture.root.join("ambiguous-git");
    let push_log = fixture.root.join("push.log");
    let script = format!(
        "#!/bin/sh\nif [ \"$1\" = push ]; then\n  /usr/bin/git \"$@\"\n  status=$?\n  printf 'push\\n' >> '{}'\n  [ \"$status\" -eq 0 ] && exit 73\n  exit \"$status\"\nfi\nexec /usr/bin/git \"$@\"\n",
        push_log.display()
    );
    std::fs::write(&wrapper, script).expect("write git wrapper");
    let mut permissions = std::fs::metadata(&wrapper)
        .expect("git wrapper metadata")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&wrapper, permissions).expect("make git wrapper executable");

    let record = claim_record("worker-a", "claim-a", "claimed");
    assert!(matches!(
        advance_claim_ref_in(
            &wrapper,
            &fixture.clients[0],
            fixture.remote.to_str().unwrap(),
            "owner/repo",
            42,
            None,
            &record,
        )
        .expect("ambiguous transition"),
        ClaimRefAdvance::Won(_)
    ));
    assert_eq!(
        std::fs::read_to_string(push_log)
            .expect("push invocation log")
            .lines()
            .count(),
        1
    );
}

#[test]
fn claim_ref_terminal_transition_rejects_foreign_identity() {
    // Break caught: foreign worker, branch, or claim ID publishing a terminal state.
    let fixture = ClaimRefFixture::new("foreign-release");
    let parent = seed_claim(&fixture);
    for (worker, branch, claim_id) in [
        ("worker-b", "feat/worker-a", "claim-a"),
        ("worker-a", "feat/worker-b", "claim-a"),
        ("worker-a", "feat/worker-a", "claim-b"),
    ] {
        let mut terminal = parent.record.clone();
        terminal.state = "merged".to_string();
        terminal.step = "merged".to_string();
        terminal.worker_id = worker.to_string();
        terminal.branch = branch.to_string();
        terminal.claim_id = Some(claim_id.to_string());
        assert!(advance_claim_ref_in(
            Path::new("git"),
            &fixture.clients[0],
            fixture.remote.to_str().unwrap(),
            "owner/repo",
            42,
            Some(&parent),
            &terminal,
        )
        .is_err());
    }
    let head = read_claim_ref_in(
        Path::new("git"),
        &fixture.clients[0],
        fixture.remote.to_str().unwrap(),
        "owner/repo",
        42,
    )
    .expect("read claim")
    .expect("claim ref");
    assert_eq!(head, parent);
}

#[test]
fn claim_ref_https_push_uses_gh_credential_helper_without_a_token_argument() {
    // Break caught: relying on global `gh auth setup-git` or exposing a token in argv.
    let arguments = super::claim_remote_arguments(
        "https://github.enterprise.example/owner/repo.git",
        &["push", "origin", "abc123:refs/autospec/claims/issue-42"],
    );
    assert_eq!(
        arguments,
        [
            "-c",
            "credential.helper=!gh auth git-credential",
            "-c",
            "credential.useHttpPath=true",
            "push",
            "origin",
            "abc123:refs/autospec/claims/issue-42",
        ]
    );
    assert!(!arguments.iter().any(|argument| {
        argument.contains("token")
            || argument.contains("Authorization")
            || argument.starts_with("https://")
    }));
    for remote_command in [
        vec!["ls-remote", "--refs", "origin"],
        vec!["fetch", "--no-tags", "origin"],
    ] {
        let arguments = super::claim_remote_arguments(
            "https://github.enterprise.example/owner/repo.git",
            &remote_command,
        );
        assert_eq!(
            &arguments[..4],
            [
                "-c",
                "credential.helper=!gh auth git-credential",
                "-c",
                "credential.useHttpPath=true",
            ]
        );
        assert!(!arguments.iter().any(|argument| argument.contains("token")));
    }
    assert_eq!(
        super::claim_remote_arguments("/tmp/remote.git", &["fetch", "/tmp/remote.git"]),
        ["fetch", "/tmp/remote.git"]
    );
    assert_eq!(
        validated_claim_remote("owner/repo", "https://github.enterprise.example/owner/repo")
            .expect("enterprise clone URL"),
        "https://github.enterprise.example/owner/repo.git"
    );
    assert!(validated_claim_remote(
        "owner/repo",
        "https://github.enterprise.example/other/repo"
    )
    .is_err());
}

#[test]
fn validated_claim_remote_rejects_unsafe_https_shapes() {
    for url in [
        "https://github.example/prefix/owner/repo",
        "https://github.example/owner/repo?token=secret",
        "https://github.example/owner/repo#fragment",
        "https://attacker@example.com/owner/repo",
        "https://github.example/not-owner/repo",
        "https://github.example/owner/repo/extra",
    ] {
        assert!(
            validated_claim_remote("owner/repo", url).is_err(),
            "accepted unsafe claim remote {url}"
        );
    }
}

#[test]
fn claim_ref_private_git_dirs_are_distinct_and_leave_target_git_metadata_unchanged() {
    // Break caught: claim fetches mutating FETCH_HEAD/objects in a shared target worktree.
    use std::os::unix::fs::PermissionsExt;

    let fixture = ClaimRefFixture::new("private-git");
    let state_root = fixture.root.join("private-state");
    let first = private_claim_git_dir_in(&state_root, "owner/repo")
        .expect("first private claim Git dir");
    let second = private_claim_git_dir_in(&state_root, "owner/repo")
        .expect("second private claim Git dir");
    assert_ne!(first.path, second.path);
    assert_eq!(
        std::fs::metadata(&first.path)
            .expect("private claim dir")
            .permissions()
            .mode()
            & 0o777,
        0o700
    );

    let target = &fixture.clients[0];
    let status_before = git_stdout(target, &["status", "--porcelain=v1"]);
    let refs_before = git_stdout(
        target,
        &["for-each-ref", "--format=%(refname) %(objectname)"],
    );
    let fetch_head = target.join(".git/FETCH_HEAD");
    let fetch_head_before = std::fs::read(&fetch_head).ok();
    let record = claim_record("worker-a", "claim-a", "claimed");
    assert!(matches!(
        advance_claim_ref_in(
            Path::new("git"),
            &first.path,
            fixture.remote.to_str().unwrap(),
            "owner/repo",
            42,
            None,
            &record,
        )
        .expect("private claim transition"),
        ClaimRefAdvance::Won(_)
    ));
    assert_eq!(
        git_stdout(target, &["status", "--porcelain=v1"]),
        status_before
    );
    assert_eq!(
        git_stdout(
            target,
            &["for-each-ref", "--format=%(refname) %(objectname)"]
        ),
        refs_before
    );
    assert_eq!(std::fs::read(&fetch_head).ok(), fetch_head_before);
}

#[test]
fn claim_mutation_paths_use_the_ref_ledger_or_fail_closed() {
    // Break caught: legacy state subcommands mutating comments/labels around the CAS ledger.
    let source = include_str!("../claim.rs");
    for function in [
        "release",
        "acquire_record",
        "refresh_claim_generation",
        "upsert_record",
        "clear",
        "recover_authoritative_stale_startup",
        "record_executor_result",
    ] {
        assert!(
            source_function(source, function).contains("advance_claim_ref"),
            "{function} must advance the claim ref before audit projection"
        );
    }
    assert!(source_function(source, "clear").contains("has_exact_claim_generation"));
    let recovery = source_function(source, "recover_authoritative_stale_startup");
    assert!(recovery.contains("\"available\""));
    assert!(recovery.contains("\"stale_startup_recovered\""));
    for function in [
        "record_executor_result_with_step",
        "reconcile_linked_pr_record",
    ] {
        let body = source_function(source, function);
        assert!(
            body.contains("read_claim_ref") && body.contains("upsert_record"),
            "{function} must route through the exact-ref mutation helper"
        );
        assert!(
            !body.contains("select_run_state"),
            "{function} must not derive a transition from stale audit projection"
        );
    }
    let lease = source_function(source, "has_active_executor_claim");
    assert!(lease.contains("record.updated_at"));
    assert!(lease.contains("record.ttl_seconds"));
}

#[test]
fn prepared_recovery_counts_toward_capacity_until_labels_converge() {
    let record = RunStateRecord::new(
        "owner/repo",
        42,
        "worker-a",
        "available",
        "",
        "",
        "stale_startup_recovered",
        Vec::new(),
        "2999-01-01T00:00:00Z",
        "2999-01-01T00:00:00Z",
        300,
    )
    .with_claim_id("claim-a");

    assert!(
        super::active_record_counts_toward_worker_capacity("owner/repo", 42, &record, 300)
            .expect("classify prepared recovery")
    );
}
