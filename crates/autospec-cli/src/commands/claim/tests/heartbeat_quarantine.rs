// claim tests: heartbeat / quarantine — 7 cases.
//
// Split out of tests.rs; see the note in that file.

use super::support::{
    anchored_startup_heartbeat_fixture, assert_mode, drift_heartbeat_at,
    expected_startup_heartbeat, expired_heartbeat_snapshot, heartbeat_copy_path,
    heartbeat_handoff_count, startup_heartbeat_fixture, write_new_heartbeat_at,
};
use crate::commands::claim;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[cfg(unix)]
#[test]
fn heartbeat_quarantine_copy_is_private_exact_and_create_new() {
    let (directory, source) = startup_heartbeat_fixture("copy-success");
    let snapshot = expired_heartbeat_snapshot(&source);
    let mut sync_order = Vec::new();
    let target = claim::persist_heartbeat_copy_with_hooks(
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
            claim::HeartbeatCopySyncBoundary::File,
            claim::HeartbeatCopySyncBoundary::Directory,
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
    let duplicate = claim::persist_heartbeat_copy(&directory, &source, &snapshot)
        .expect_err("copy target must be create-new");
    assert!(duplicate.to_string().contains("already exists"));
    let flags = nix::fcntl::OFlag::O_RDONLY | nix::fcntl::OFlag::O_DIRECTORY;
    let directory_fd =
        nix::fcntl::open(&directory, flags, nix::sys::stat::Mode::empty()).expect("open root");
    let (pipe, _writer) = nix::unistd::pipe().expect("file-sync pipe");
    let error = claim::sync_heartbeat_copy(&std::fs::File::from(pipe), &directory_fd, &mut |_| {
        panic!("boundary emitted after failed file sync")
    })
    .expect_err("pipe file cannot sync");
    assert!(error.to_string().contains("sync heartbeat quarantine copy"));

    let file = std::fs::File::options().write(true).open(&source).unwrap();
    let (pipe, _writer) = nix::unistd::pipe().expect("directory-sync pipe");
    let mut boundaries = Vec::new();
    let error = claim::sync_heartbeat_copy(&file, &pipe, &mut |boundary| {
        boundaries.push(boundary);
        Ok(())
    })
    .expect_err("pipe directory cannot sync");
    assert!(error
        .to_string()
        .contains("sync heartbeat quarantine directory"));
    assert_eq!(boundaries, [claim::HeartbeatCopySyncBoundary::File]);
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
        claim::revalidate_heartbeat_snapshot(&observed, &snapshot.file)
            .expect_err("identity drift")
            .to_string()
            .contains("identity drift")
    );
    observed = snapshot.file.clone();
    observed.document[0] = b'[';
    let mut document_drift = (*snapshot).clone();
    document_drift.file = observed.clone();
    assert!(
        claim::persist_heartbeat_copy(&directory, &source, &document_drift)
            .expect_err("document drift")
            .to_string()
            .contains("document drift")
    );

    std::fs::rename(&source, directory.join("old.json")).expect("move classified inode");
    std::fs::write(&source, &snapshot.file.document).expect("publish replacement inode");
    assert!(
        claim::persist_heartbeat_copy(&directory, &source, &snapshot)
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
    assert!(claim::persist_heartbeat_copy(
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
    assert!(
        claim::persist_heartbeat_copy(&root_link, &preexisting_source, &preexisting_snapshot)
            .is_err()
    );
    std::fs::remove_file(root_link).expect("remove state root symlink");
    std::os::unix::fs::symlink(&outside, preexisting_root.join("quarantine"))
        .expect("preexisting quarantine symlink");
    assert!(claim::persist_heartbeat_copy(
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
    let result = claim::persist_heartbeat_copy_with_hooks(
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
    let result = claim::handoff_stale_heartbeat_path_with_hooks(
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
        let result = claim::handoff_stale_heartbeat_path_with_hooks(
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
        let result = claim::handoff_stale_heartbeat_path_with_hooks(
            &directory,
            &source,
            &snapshot,
            || {},
            |root, _, _| {
                if mode == 2 {
                    nix::unistd::mkfifo(&source, nix::sys::stat::Mode::from_bits_truncate(0o600))
                        .expect("publish live FIFO");
                } else if mode != 1 {
                    write_new_heartbeat_at(root, &snapshot.file.document);
                }
            },
            |boundary| {
                if mode == 1 && boundary == claim::HeartbeatHandoffSyncBoundary::Cleanup {
                    std::fs::write(&source, &snapshot.file.document)
                        .expect("publish post-unlink replacement");
                    return Err(claim::CommandFailure::diagnostic(
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
    use claim::HeartbeatReceiptDecision::{Completed, Pending};

    let boundaries = [
        claim::HeartbeatHandoffSyncBoundary::Source,
        claim::HeartbeatHandoffSyncBoundary::Handoff,
        claim::HeartbeatHandoffSyncBoundary::Cleanup,
        claim::HeartbeatHandoffSyncBoundary::RestoreRoot,
        claim::HeartbeatHandoffSyncBoundary::RestoreUnlink,
        claim::HeartbeatHandoffSyncBoundary::RestoreHandoff,
    ];
    for failed in boundaries {
        let label = format!("atomic-{failed:?}");
        let (parent_path, repo_path, source, repo, snapshot) =
            anchored_startup_heartbeat_fixture(label.as_str());
        let expectation = expected_startup_heartbeat("host:user:rust:4242:nonce-a");
        let result = claim::handoff_stale_heartbeat(
            &repo_path,
            &repo,
            source.file_name().unwrap(),
            &snapshot,
            |_, handoff, moved| {
                if matches!(
                    failed,
                    claim::HeartbeatHandoffSyncBoundary::RestoreRoot
                        | claim::HeartbeatHandoffSyncBoundary::RestoreUnlink
                        | claim::HeartbeatHandoffSyncBoundary::RestoreHandoff
                ) {
                    drift_heartbeat_at(handoff, moved);
                }
            },
            |boundary| {
                if failed == boundary {
                    Err(claim::CommandFailure::diagnostic("injected sync failure"))
                } else {
                    Ok(())
                }
            },
            |directory| {
                nix::unistd::fsync(directory).map_err(|error| {
                    claim::CommandFailure::diagnostic(format!("cleanup fsync: {error}"))
                })
            },
        );
        assert!(result.is_err(), "{failed:?} must abort");
        assert_eq!(
            claim::heartbeat_receipt_retry_decision(&repo, expectation),
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
        let result = claim::handoff_stale_heartbeat(
            &repo_path,
            &repo,
            source.file_name().unwrap(),
            &snapshot,
            |root, handoff, moved| match mode {
                0 => {
                    nix::unistd::unlinkat(handoff, moved, nix::unistd::UnlinkatFlags::NoRemoveDir)
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
                    claim::CommandFailure::diagnostic(format!("cleanup fsync: {error}"))
                })
            },
        );
        assert_eq!(
            claim::heartbeat_receipt_retry_decision(&repo, expectation),
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
    assert!(claim::handoff_stale_heartbeat(
        &repo_path,
        &repo,
        source.file_name().unwrap(),
        &snapshot,
        |_, _, _| {},
        |_| Ok(()),
        |_| {
            nix::unistd::fsync(&pipe)
                .map_err(|error| claim::CommandFailure::diagnostic(format!("fsync: {error}")))
        },
    )
    .is_err());
    assert_eq!(
        claim::heartbeat_receipt_retry_decision(&repo, expectation),
        Pending
    );
    let handoff = repo_path.join("quarantine/startup-heartbeat-handoffs");
    assert!(source.exists() || std::fs::read_dir(handoff).unwrap().count() > 1);
    std::fs::remove_dir_all(parent_path).unwrap();
}
