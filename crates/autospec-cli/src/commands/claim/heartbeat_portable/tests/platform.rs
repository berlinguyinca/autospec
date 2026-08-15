use super::*;

#[cfg(unix)]
#[test]
fn unix_directory_is_private_at_its_creation_boundary() {
    use nix::sys::stat::{umask, Mode};
    use std::os::unix::fs::PermissionsExt;

    const CHILD: &str = "AUTOSPEC_TEST_PORTABLE_PRIVATE_DIRECTORY_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().expect("test binary"))
            .args([
                "--exact",
                "commands::claim::heartbeat_portable::tests::unix_directory_is_private_at_its_creation_boundary",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .output()
            .expect("isolated umask test");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }

    let fixture = Fixture::new("atomic-private-directory");
    let directory = fixture.root.join("created-private");
    let previous = umask(Mode::empty());
    let mut observed_mode = None;
    let result = ensure_private_directory_with_hook(&directory, &mut |created| {
        observed_mode = Some(
            std::fs::symlink_metadata(created)
                .expect("created directory metadata")
                .permissions()
                .mode()
                & 0o7777,
        );
    });
    umask(previous);
    result.expect("private directory creation");

    assert_eq!(observed_mode, Some(0o700));
}

#[cfg(windows)]
#[test]
fn concurrent_windows_publication_has_exactly_one_winner() {
    let fixture = std::sync::Arc::new(Fixture::new("windows-exclusive-publication"));
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
    let mut publishers = Vec::new();
    for claim_id in ["claim-a", "claim-b"] {
        let fixture = std::sync::Arc::clone(&fixture);
        let barrier = std::sync::Arc::clone(&barrier);
        publishers.push(std::thread::spawn(move || {
            let document = fixture.document(claim_id, None);
            barrier.wait();
            publish(&fixture.root, "owner/repo", 42, None, &document)
        }));
    }
    barrier.wait();
    let results = publishers
        .into_iter()
        .map(|publisher| publisher.join().expect("publisher thread"))
        .collect::<Vec<_>>();

    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    let winner = std::fs::read_to_string(fixture.issue_path()).expect("winning heartbeat");
    assert!(winner.contains("claim-a") || winner.contains("claim-b"));
}

#[cfg(windows)]
#[test]
fn windows_publication_rejects_multi_link_target() {
    let fixture = Fixture::new("windows-multi-link-target");
    let document = fixture.document("claim-a", None);
    publish(&fixture.root, "owner/repo", 42, None, &document).expect("initial heartbeat");
    std::fs::hard_link(fixture.issue_path(), fixture.repo_path().join("alias.json"))
        .expect("create second link");

    let error = publish(&fixture.root, "owner/repo", 42, None, &document)
        .expect_err("multi-link target must be rejected");

    assert_eq!(error.message, "heartbeat publication target conflicts");
}

#[cfg(windows)]
fn replace_directory_with_junction(path: &Path, replacement: &Path, backup: &Path) {
    std::fs::rename(path, backup).expect("move validated directory aside");
    let output = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(path)
        .arg(replacement)
        .output()
        .expect("create replacement junction");
    assert!(
        output.status.success(),
        "junction creation failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(windows)]
#[test]
fn windows_retirement_stays_bound_when_repository_component_is_replaced() {
    let fixture = Fixture::new("windows-retirement-repo-reparse-race");
    let document = fixture.document("claim-a", None);
    publish(&fixture.root, "owner/repo", 42, None, &document).expect("heartbeat");
    let repo = fixture.repo_path();
    let original_repo = fixture.root.join("original-repo");
    let outside = fixture.root.join("outside-repo");
    std::fs::create_dir(&outside).expect("outside repo");
    std::fs::write(outside.join("42.json"), &document).expect("outside heartbeat");

    retire_released_at_with_boundary_hooks(
        &fixture.root,
        ClaimMutationIdentity {
            repo: "owner/repo",
            issue: 42,
            worker_id: "worker-a",
            branch: "feat/worker",
            claim_id: "claim-a",
        },
        &mut |_| {
            replace_directory_with_junction(&repo, &outside, &original_repo);
            Ok(())
        },
        &mut |_| Ok(()),
        &mut |_| Ok(()),
    )
    .expect("handle-bound repository retirement");

    assert!(
        outside.join("42.json").exists(),
        "replacement target heartbeat was deleted"
    );
    assert!(
        !original_repo.join("42.json").exists(),
        "validated repository heartbeat was not retired"
    );
    std::fs::remove_dir(&repo).expect("remove repository junction");
}

#[cfg(windows)]
#[test]
fn windows_retirement_stays_bound_when_sessions_component_is_replaced() {
    let fixture = Fixture::new("windows-retirement-session-reparse-race");
    let document = fixture.document("claim-a", Some("session-a"));
    publish(
        &fixture.root,
        "owner/repo",
        42,
        Some("session-a"),
        &document,
    )
    .expect("heartbeat");
    let sessions = fixture.repo_path().join("sessions");
    let original_sessions = fixture.repo_path().join("original-sessions");
    let outside = fixture.root.join("outside-sessions");
    std::fs::create_dir(&outside).expect("outside sessions");
    let session_name = format!("{}.json", heartbeat_session_key("session-a"));
    std::fs::write(outside.join(&session_name), &document).expect("outside heartbeat");

    retire_released_at_with_boundary_hooks(
        &fixture.root,
        ClaimMutationIdentity {
            repo: "owner/repo",
            issue: 42,
            worker_id: "worker-a",
            branch: "feat/worker",
            claim_id: "claim-a",
        },
        &mut |_| Ok(()),
        &mut |_| Ok(()),
        &mut |_| {
            replace_directory_with_junction(&sessions, &outside, &original_sessions);
            Ok(())
        },
    )
    .expect("handle-bound sessions retirement");

    assert!(
        outside.join(&session_name).exists(),
        "replacement target session heartbeat was deleted"
    );
    assert!(
        !original_sessions.join(&session_name).exists(),
        "validated session heartbeat was not retired"
    );
    assert!(!fixture.issue_path().exists(), "issue heartbeat remained");
    std::fs::remove_dir(&sessions).expect("remove sessions junction");
}
