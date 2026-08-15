use super::*;

#[cfg(unix)]
#[test]
fn retirement_rejects_an_intermediate_repository_symlink() {
    use std::os::unix::fs::{symlink, PermissionsExt};

    let fixture = Fixture::new("retirement-repo-symlink");
    let repo_name = crate::commands::autonomous::drain::repository_progress_key("owner/repo");
    let outside = fixture.root.join("outside-repo");
    std::fs::create_dir(&outside).expect("outside repo");
    std::fs::set_permissions(&outside, std::fs::Permissions::from_mode(0o700))
        .expect("outside repo permissions");
    let document = fixture.document("claim-a", None);
    std::fs::write(outside.join("42.json"), &document).expect("outside heartbeat");
    std::fs::set_permissions(
        outside.join("42.json"),
        std::fs::Permissions::from_mode(0o600),
    )
    .expect("outside heartbeat permissions");
    symlink(&outside, fixture.root.join(repo_name)).expect("repository symlink");

    let result = retire_released_at(
        &fixture.root,
        ClaimMutationIdentity {
            repo: "owner/repo",
            issue: 42,
            worker_id: "worker-a",
            branch: "feat/worker",
            claim_id: "claim-a",
        },
    );

    assert!(result.is_err(), "intermediate symlink was accepted");
    assert_eq!(
        std::fs::read(outside.join("42.json")).expect("outside heartbeat retained"),
        document
    );
}

#[cfg(unix)]
#[test]
fn retirement_deletes_the_detached_generation_not_its_replacement() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = Fixture::new("retirement-replacement-race");
    let original = fixture.document("claim-a", None);
    publish(&fixture.root, "owner/repo", 42, None, &original).expect("original heartbeat");
    let replacement = fixture.document("claim-b", None);
    let replacement_path = fixture.issue_path().with_extension("replacement");
    std::fs::write(&replacement_path, &replacement).expect("replacement heartbeat");
    std::fs::set_permissions(&replacement_path, std::fs::Permissions::from_mode(0o600))
        .expect("replacement heartbeat permissions");

    retire_released_at_with_hook(
        &fixture.root,
        ClaimMutationIdentity {
            repo: "owner/repo",
            issue: 42,
            worker_id: "worker-a",
            branch: "feat/worker",
            claim_id: "claim-a",
        },
        &mut |vacated_issue| {
            std::fs::rename(&replacement_path, vacated_issue)
                .map_err(|error| CommandFailure::diagnostic(error.to_string()))
        },
    )
    .expect("exact detached retirement");

    assert_eq!(
        std::fs::read(fixture.issue_path()).expect("replacement retained"),
        replacement
    );
}

#[cfg(unix)]
#[test]
fn retirement_rejects_an_intermediate_session_symlink_without_losing_the_issue() {
    use std::os::unix::fs::{symlink, PermissionsExt};

    let fixture = Fixture::new("retirement-session-symlink");
    let document = fixture.document("claim-a", Some("session-a"));
    publish(
        &fixture.root,
        "owner/repo",
        42,
        Some("session-a"),
        &document,
    )
    .expect("heartbeat");
    let repo = fixture.issue_path().parent().expect("repo").to_path_buf();
    std::fs::remove_dir_all(repo.join("sessions")).expect("remove real sessions");
    let outside = fixture.root.join("outside-sessions");
    std::fs::create_dir(&outside).expect("outside sessions");
    std::fs::set_permissions(&outside, std::fs::Permissions::from_mode(0o700))
        .expect("outside sessions permissions");
    let session_name = format!("{}.json", heartbeat_session_key("session-a"));
    std::fs::write(outside.join(&session_name), &document).expect("outside session heartbeat");
    std::fs::set_permissions(
        outside.join(&session_name),
        std::fs::Permissions::from_mode(0o600),
    )
    .expect("outside session permissions");
    symlink(&outside, repo.join("sessions")).expect("sessions symlink");

    let result = retire_released_at(
        &fixture.root,
        ClaimMutationIdentity {
            repo: "owner/repo",
            issue: 42,
            worker_id: "worker-a",
            branch: "feat/worker",
            claim_id: "claim-a",
        },
    );

    assert!(result.is_err(), "intermediate session symlink was accepted");
    assert_eq!(
        std::fs::read(fixture.issue_path()).expect("issue heartbeat restored"),
        document
    );
    assert!(outside.join(session_name).exists());
}

#[cfg(unix)]
#[test]
fn repository_lock_serializes_retirement_and_publication() {
    let fixture = Fixture::new("retirement-publication-lock");
    let original = fixture.document("claim-a", None);
    publish(&fixture.root, "owner/repo", 42, None, &original).expect("original heartbeat");
    let replacement = fixture.document("claim-b", None);
    let root = fixture.root.clone();
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (completed_tx, completed_rx) = std::sync::mpsc::channel();
    let mut publisher = None;

    retire_released_at_with_hook(
        &fixture.root,
        ClaimMutationIdentity {
            repo: "owner/repo",
            issue: 42,
            worker_id: "worker-a",
            branch: "feat/worker",
            claim_id: "claim-a",
        },
        &mut |_| {
            let root = root.clone();
            let replacement = replacement.clone();
            let started_tx = started_tx.clone();
            let completed_tx = completed_tx.clone();
            publisher = Some(std::thread::spawn(move || {
                started_tx.send(()).expect("publisher started");
                let result = publish(&root, "owner/repo", 42, None, &replacement);
                completed_tx.send(result).expect("publisher completed");
            }));
            started_rx
                .recv_timeout(std::time::Duration::from_secs(1))
                .expect("publisher reached publication");
            assert!(
                completed_rx
                    .recv_timeout(std::time::Duration::from_millis(100))
                    .is_err(),
                "publisher crossed retirement's repository lock"
            );
            Ok(())
        },
    )
    .expect("serialized retirement");

    completed_rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .expect("publisher completed after retirement")
        .expect("replacement publication");
    publisher
        .take()
        .expect("publisher handle")
        .join()
        .expect("publisher thread");
    assert_eq!(
        std::fs::read(fixture.issue_path()).expect("replacement heartbeat"),
        replacement
    );
}
