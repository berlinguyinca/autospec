use super::*;

#[test]
fn publication_rejects_process_identity_changes_but_allows_timestamp_refreshes() {
    let fixture = Fixture::new("immutable-process-identity");
    let original = fixture.document("claim-a", None);
    publish(&fixture.root, "owner/repo", 42, None, &original).expect("initial publication");

    let timestamp_refresh = String::from_utf8(original.clone())
        .expect("heartbeat is UTF-8")
        .replace(r#""ts":1"#, r#""ts":2"#)
        .into_bytes();
    publish(&fixture.root, "owner/repo", 42, None, &timestamp_refresh)
        .expect("timestamp-only replay");

    for changed in [
        String::from_utf8(original.clone())
            .expect("heartbeat is UTF-8")
            .replace(r#""pid":7"#, r#""pid":8"#)
            .into_bytes(),
        String::from_utf8(original.clone())
            .expect("heartbeat is UTF-8")
            .replace(r#""process_start":"9""#, r#""process_start":"10""#)
            .into_bytes(),
    ] {
        let error = publish(&fixture.root, "owner/repo", 42, None, &changed)
            .expect_err("process identity change must conflict");
        assert_eq!(error.message, "heartbeat publication target conflicts");
        assert_eq!(
            std::fs::read(fixture.issue_path()).expect("original heartbeat retained"),
            original
        );
    }
}

#[cfg(unix)]
#[test]
fn publication_rejects_a_final_symlink() {
    use std::os::unix::fs::{symlink, PermissionsExt};

    let fixture = Fixture::new("symlink");
    let repo = fixture.issue_path().parent().unwrap().to_path_buf();
    std::fs::create_dir(&repo).expect("repo directory");
    std::fs::set_permissions(&repo, std::fs::Permissions::from_mode(0o700))
        .expect("repo permissions");
    let outside = fixture.root.join("outside");
    std::fs::write(&outside, b"caller-owned").expect("outside file");
    symlink(&outside, fixture.issue_path()).expect("final symlink");

    let error = publish(
        &fixture.root,
        "owner/repo",
        42,
        None,
        &fixture.document("claim-a", None),
    )
    .expect_err("final symlink conflict");

    assert_eq!(error.message, "heartbeat publication target conflicts");
    assert_eq!(std::fs::read(outside).unwrap(), b"caller-owned");
}

#[test]
fn retirement_removes_only_the_exact_generation() {
    let fixture = Fixture::new("retirement");
    let document = fixture.document("claim-a", Some("session-a"));
    publish(
        &fixture.root,
        "owner/repo",
        42,
        Some("session-a"),
        &document,
    )
    .expect("heartbeat");

    retire_released_at(
        &fixture.root,
        ClaimMutationIdentity {
            repo: "owner/repo",
            issue: 42,
            worker_id: "worker-a",
            branch: "feat/worker",
            claim_id: "claim-b",
        },
    )
    .expect("mismatch is not retired");
    assert!(fixture.issue_path().exists());

    retire_released_at(
        &fixture.root,
        ClaimMutationIdentity {
            repo: "owner/repo",
            issue: 42,
            worker_id: "worker-a",
            branch: "feat/worker",
            claim_id: "claim-a",
        },
    )
    .expect("exact retirement");
    assert!(!fixture.issue_path().exists());
}

#[test]
fn retirement_of_an_exact_issue_tolerates_a_missing_session_copy() {
    let fixture = Fixture::new("retirement-missing-session");
    let document = fixture.document("claim-a", Some("session-a"));
    publish(
        &fixture.root,
        "owner/repo",
        42,
        Some("session-a"),
        &document,
    )
    .expect("heartbeat");
    std::fs::remove_dir_all(
        fixture
            .issue_path()
            .parent()
            .expect("repo")
            .join("sessions"),
    )
    .expect("remove session copy");

    retire_released_at(
        &fixture.root,
        ClaimMutationIdentity {
            repo: "owner/repo",
            issue: 42,
            worker_id: "worker-a",
            branch: "feat/worker",
            claim_id: "claim-a",
        },
    )
    .expect("exact issue retirement");

    assert!(!fixture.issue_path().exists());
}

#[test]
fn retirement_resumes_after_crash_between_issue_and_session_detachment() {
    let fixture = Fixture::new("retirement-resume-after-issue-detach");
    let document = fixture.document("claim-a", Some("session-a"));
    publish(
        &fixture.root,
        "owner/repo",
        42,
        Some("session-a"),
        &document,
    )
    .expect("heartbeat");
    let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = retire_released_at_with_hook(
            &fixture.root,
            ClaimMutationIdentity {
                repo: "owner/repo",
                issue: 42,
                worker_id: "worker-a",
                branch: "feat/worker",
                claim_id: "claim-a",
            },
            &mut |_| panic!("simulated retirement crash after issue detachment"),
        );
    }));
    assert!(interrupted.is_err());
    assert!(!fixture.issue_path().exists(), "issue was not detached");

    retire_released_at(
        &fixture.root,
        ClaimMutationIdentity {
            repo: "owner/repo",
            issue: 42,
            worker_id: "worker-a",
            branch: "feat/worker",
            claim_id: "claim-a",
        },
    )
    .expect("resume exact retirement");

    let session = fixture
        .repo_path()
        .join("sessions")
        .join(format!("{}.json", heartbeat_session_key("session-a")));
    assert!(!session.exists(), "resumed retirement left stale session");
    let successor = fixture.document("claim-b", Some("session-b"));
    publish(
        &fixture.root,
        "owner/repo",
        42,
        Some("session-b"),
        &successor,
    )
    .expect("publish successor after resumed retirement");
    assert!(fixture.issue_path().exists());
}

#[cfg(unix)]
#[test]
fn publication_remains_bound_to_open_repository_after_parent_swap() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = Fixture::new("publication-parent-swap");
    let repo_name = repository_progress_key("owner/repo");
    let repo = fixture.root.join(&repo_name);
    std::fs::create_dir(&repo).expect("repository directory");
    std::fs::set_permissions(&repo, std::fs::Permissions::from_mode(0o700))
        .expect("repository permissions");
    let retained = fixture.root.join("retained-repository");
    let replacement = fixture.root.join("replacement-repository");
    std::fs::create_dir(&replacement).expect("replacement repository");
    std::fs::set_permissions(&replacement, std::fs::Permissions::from_mode(0o700))
        .expect("replacement permissions");
    let document = fixture.document("claim-a", Some("session-a"));

    publish_with_hooks(
        &fixture.root,
        "owner/repo",
        42,
        Some("session-a"),
        &document,
        &mut |_| {
            std::fs::rename(&repo, &retained).expect("retain opened repository");
            std::fs::rename(&replacement, &repo).expect("swap repository path");
        },
        &mut |_| {},
    )
    .expect("handle-bound publication");

    assert!(retained.join("42.json").is_file());
    assert!(retained
        .join("sessions")
        .join(format!("{}.json", heartbeat_session_key("session-a")))
        .is_file());
    assert!(!repo.join("42.json").exists());
    assert!(!repo.join("sessions").exists());
}

#[cfg(unix)]
#[test]
fn publication_retry_cleans_crash_staging_aliases() {
    let fixture = Fixture::new("publication-crash-staging");
    let document = fixture.document("claim-a", None);
    let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = publish_with_hooks(
            &fixture.root,
            "owner/repo",
            42,
            None,
            &document,
            &mut |_| {},
            &mut |_| panic!("simulated publication crash before rename"),
        );
    }));
    assert!(interrupted.is_err());

    publish(&fixture.root, "owner/repo", 42, None, &document).expect("restart publication");
    assert!(
        fixture.staging_paths().is_empty(),
        "publication left staging aliases"
    );
}

#[test]
fn successor_publication_reconciles_an_abandoned_pre_rename_stage() {
    let fixture = Fixture::new("publication-successor-after-abandoned-stage");
    let abandoned = fixture.document("claim-a", None);
    let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = publish_with_hooks(
            &fixture.root,
            "owner/repo",
            42,
            None,
            &abandoned,
            &mut |_| {},
            &mut |_| panic!("simulated publication crash before rename"),
        );
    }));
    assert!(interrupted.is_err());

    let successor = fixture.document("claim-b", None);
    publish(&fixture.root, "owner/repo", 42, None, &successor)
        .expect("publish successor generation");

    assert_eq!(
        std::fs::read(fixture.issue_path()).expect("successor heartbeat"),
        successor
    );
    assert!(
        fixture.staging_paths().is_empty(),
        "successor left an abandoned staging file"
    );
}

#[cfg(unix)]
#[test]
fn stage_reconciliation_ignores_noncanonical_entries_and_symlinks() {
    use std::os::unix::fs::symlink;

    let fixture = Fixture::new("publication-safe-stage-reconciliation");
    let abandoned = fixture.document("claim-a", None);
    let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = publish_with_hooks(
            &fixture.root,
            "owner/repo",
            42,
            None,
            &abandoned,
            &mut |_| {},
            &mut |_| panic!("simulated publication crash before rename"),
        );
    }));
    assert!(interrupted.is_err());

    let caller_owned = fixture.repo_path().join("caller-owned");
    std::fs::write(&caller_owned, b"retain me").expect("caller-owned file");
    let stage_symlink = fixture
        .repo_path()
        .join(".autospec-heartbeat-42.json.00000000000000000000000000000000.stage");
    symlink(&caller_owned, &stage_symlink).expect("canonical-looking stage symlink");

    publish(
        &fixture.root,
        "owner/repo",
        42,
        None,
        &fixture.document("claim-b", None),
    )
    .expect("successor publication");

    assert_eq!(std::fs::read(caller_owned).unwrap(), b"retain me");
    assert!(
        stage_symlink.is_symlink(),
        "stage symlink was followed or removed"
    );
}
