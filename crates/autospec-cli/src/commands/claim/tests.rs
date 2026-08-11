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

mod support;
mod heartbeat_startup;
mod heartbeat_prior;
mod heartbeat_classify;
use support::*;

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
