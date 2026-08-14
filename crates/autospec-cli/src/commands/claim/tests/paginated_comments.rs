// claim tests: paginated / comments — 11 cases.
//
// Split out of tests.rs; see the note in that file.

use super::super::{claim_settle_millis, publish_session_binding, SessionBindingIdentity};
#[cfg(target_os = "linux")]
use super::support::{
    anchored_startup_heartbeat_fixture, expected_startup_heartbeat, expired_heartbeat_snapshot,
    mutate_retained, startup_heartbeat_document, startup_heartbeat_fixture, STARTUP_HEARTBEAT_ENV,
};
use super::support::{claim_record, lifecycle_evidence};
use crate::commands::claim;
use autospec_core::autonomous_lifecycle::{
    ClaimBranch, ClaimContext, ClaimEvidence, IssueNumber, LeaseFreshness, RepositoryScope,
    WorkerId,
};
use autospec_core::claim::RemoteComment;
#[cfg(unix)]
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::{Arc, Barrier};

#[cfg(target_os = "linux")]
#[test]
fn startup_heartbeat_retained_handoff() {
    use std::os::unix::fs::MetadataExt;
    for race in
        "before-check before-rename after-rename collision malformed fifo hardlink".split(' ')
    {
        let (parent, repo_path, source, repo, snapshot) = anchored_startup_heartbeat_fixture(race);
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
        let result = claim::handoff_retained_heartbeat(
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
                        std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o600))
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
        claim::HeartbeatHandoffSyncBoundary::Source,
        claim::HeartbeatHandoffSyncBoundary::Handoff,
    ] {
        let (parent, repo_path, source, repo, snapshot) =
            anchored_startup_heartbeat_fixture("retained-fsync");
        let inode = std::fs::metadata(&source).unwrap().ino();
        assert!(claim::handoff_retained_heartbeat(
            &repo_path,
            &repo,
            source.file_name().unwrap(),
            &snapshot,
            |_, _, _, _| {},
            |boundary| (boundary != failed)
                .then_some(())
                .ok_or_else(|| claim::CommandFailure::diagnostic("injected fsync failure")),
        )
        .is_err());
        for _ in 0..2 {
            let retained = claim::handoff_retained_heartbeat(
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
        let (_, completed) = claim::heartbeat_receipt_names(expected);
        let archive = repo_path
            .join("quarantine/startup-heartbeat-handoffs")
            .join(completed.replace(".receipt", ".json"));
        let result = claim::handoff_retained_heartbeat(
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
                if boundary == claim::HeartbeatHandoffSyncBoundary::Cleanup
                    && mutation.starts_with("cleanup-")
                {
                    mutate_retained(&archive, &source, mutation);
                }
                Ok(())
            },
        );
        assert!(result.is_err(), "{mutation}");
        let receipts = claim::open_receipt_anchors_with_hook(&repo, |_| {}).unwrap();
        assert!(
            claim::inspect_heartbeat_receipt(&receipts, completed.as_ref())
                == claim::HeartbeatReceiptEntry::Missing,
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
    let repo = claim::open_heartbeat_directory_beneath(&root_fd, Path::new(&repo_name)).unwrap();
    let archive = claim::handoff_retained_heartbeat(
        &repo_path,
        &repo,
        source.file_name().unwrap(),
        &snapshot,
        |_, _, _, _| {},
        |_| Ok(()),
    )
    .unwrap();
    // SAFETY: single-threaded test setup; no other thread reads the environment here.
    unsafe { std::env::set_var("AUTOSPEC_HEARTBEAT_DIR", &root) };
    let identity = claim::ClaimMutationIdentity {
        repo: "owner/repo",
        issue: 42,
        worker_id: "host:user:rust:4242:nonce-a",
        branch: "feat/worker",
        claim_id: "claim-a",
    };
    let calls = Cell::new(0usize);
    let accepted = claim::with_retained_bridge_predecessor_authority(
        identity,
        |_, _, _, _, _| claim::StartupPidLiveness::Dead,
        || {},
        || {
            calls.set(calls.get() + 1);
            Ok("transferred")
        },
    )
    .unwrap();
    assert_eq!(accepted, Some("transferred"));
    assert_eq!(calls.get(), 1);

    let missing = claim::with_retained_bridge_predecessor_authority(
        claim::ClaimMutationIdentity {
            claim_id: "another-claim",
            ..identity
        },
        |_, _, _, _, _| claim::StartupPidLiveness::Dead,
        || {},
        || {
            calls.set(calls.get() + 1);
            Ok(())
        },
    )
    .unwrap();
    assert!(missing.is_none());
    let live = claim::with_retained_bridge_predecessor_authority(
        identity,
        |_, _, _, _, _| claim::StartupPidLiveness::Live,
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
    let raced = claim::with_retained_bridge_predecessor_authority(
        identity,
        |_, _, _, _, _| claim::StartupPidLiveness::Dead,
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
        // SAFETY: single-threaded test setup; no other thread reads the environment here.
        Some(value) => unsafe { std::env::set_var("AUTOSPEC_HEARTBEAT_DIR", value) },
        // SAFETY: single-threaded test setup; no other thread reads the environment here.
        None => unsafe { std::env::remove_var("AUTOSPEC_HEARTBEAT_DIR") },
    }
    std::fs::remove_dir_all(sandbox).unwrap();
}

#[test]
fn paginated_comments_parser_flattens_two_raw_pages() {
    // Break caught: treating each slurped page array as a comment object.
    let comments = claim::parse_paginated_comments_json(
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
    let comments = claim::parse_paginated_comments_json(
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
    assert!(claim::parse_paginated_comments_json(
        r#"[[{"id":100,"body":"state","updated_at":"now"}],{"id":101,"body":"other","updated_at":"now"}]"#,
    )
    .is_err());

    for malformed in [
        r#"[{"id":"100","body":"state","updated_at":"now"}]"#,
        r#"[{"id":100,"body":false,"updated_at":"now"}]"#,
        r#"[{"id":100,"body":"state","updated_at":[]}]"#,
    ] {
        assert!(
            claim::parse_paginated_comments_json(malformed).is_err(),
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
        claim::UNIQUE_ID_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
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
        claim::UNIQUE_ID_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
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
