// executor_bridge tests: worktree / post — 5 cases.
//
// Split out of tests.rs; see the note in that file.

use super::super::{
    provision_issue_worktree, recover_invocation, resolve_base, write_invocation_atomic,
    BridgeIdentity, BridgePhase, HarnessKind, PersistedInvocation,
};
use super::support_base::{
    git, git_stdout, test_environment, test_root, write_executable, GitFixture, TEST_SEQUENCE,
};
use super::support_invocation::{
    commit_implementation, implementation_proof_fixture, persisted_invocation, supervision_state,
};
use super::support_launch::{
    automatic_review_command, direct_failure_archive_count, rewrite_direct_terminal_as_signal,
};
use crate::commands::autonomous::executor_bridge as bridge;
use std::collections::BTreeMap;
use std::fs;
use std::sync::atomic::Ordering;
use std::time::Duration;

#[test]
fn autonomous_executor_bridge_missing_worktree_post_ci_recovery() {
    let _environment = test_environment();
    let mut fixture = GitFixture::new("missing-post-ci-worktree");
    let base = resolve_base(&fixture.repo, &BTreeMap::new()).expect("resolve base");
    let repository = format!(
        "owner/missing-post-ci-{}",
        TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    );
    let worktree = provision_issue_worktree(&fixture.repo, &repository, 42, &base)
        .expect("provision worktree");
    fixture
        .executor_scope_roots
        .push(worktree.path.parent().unwrap().to_path_buf());
    fs::write(worktree.path.join("implementation.txt"), "implemented\n")
        .expect("write implementation");
    git(&worktree.path, &["add", "implementation.txt"]);
    git(&worktree.path, &["commit", "-m", "feat: implement fixture"]);
    git(&worktree.path, &["push", "-u", "origin", &worktree.branch]);
    let head = git_stdout(&worktree.path, &["rev-parse", "HEAD"]);
    let state_path = fixture.root.join("state/invocation.json");
    let mut state = supervision_state(&fixture);
    state.identity.repository = repository;
    state.identity.worktree = worktree.path.clone();
    state.identity.branch = worktree.branch.clone();
    state.identity.base_ref = base.base_ref.clone();
    state.identity.base_oid = base.base_oid.clone();
    state.phase = BridgePhase::CiPassed;
    state.pr = Some(55);
    state.head_oid = Some(head.clone());
    write_invocation_atomic(&state_path, &state).expect("persist post-CI state");
    let review_artifacts = state_path.with_extension("review-artifacts");
    let review_capture = fixture.root.join("review-capture");
    let reviewer = fixture.root.join("reviewer");
    let repaired = fixture.root.join("reviewer-repaired");
    write_executable(
        &reviewer,
        &format!("#!/bin/sh\n[ -f '{}' ] || exit 1\n", repaired.display()),
    );
    let review_plan = |identity: &str| bridge::DirectCommandPlan {
        commands: vec![automatic_review_command(
            &reviewer,
            &review_capture,
            identity,
        )],
    };
    bridge::execute_direct_plan(
        &worktree.path,
        &review_plan(&"a".repeat(64)),
        &review_artifacts,
        None,
        Duration::from_secs(5),
    )
    .expect_err("persist old automatic review failure");
    rewrite_direct_terminal_as_signal(&review_artifacts, 25);
    let old_review = fs::read(bridge::direct_attempt_paths(&review_artifacts, 0).record)
        .expect("read old review record");
    git(
        &fixture.repo,
        &["worktree", "remove", worktree.path.to_str().unwrap()],
    );

    let mut mismatched = state.clone();
    mismatched.head_oid = Some("a".repeat(40));
    write_invocation_atomic(&state_path, &mismatched).expect("persist mismatched head");
    assert!(recover_invocation(&state_path, &mismatched.identity)
        .expect_err("mismatched durable head must fail")
        .contains("head"));

    write_invocation_atomic(&state_path, &state).expect("restore exact state");
    git(&fixture.repo, &["branch", "-D", &worktree.branch]);
    assert!(recover_invocation(&state_path, &state.identity)
        .expect_err("missing branch must fail")
        .contains("branch"));
    git(&fixture.repo, &["branch", &worktree.branch, &head]);
    bridge::record_worktree_creation_identity(&fixture.repo, &worktree.branch, &base)
        .expect("restore branch identity");
    let base_key = format!("branch.{}.autospecBaseOid", worktree.branch);
    let branch_ref = format!("refs/heads/{}", worktree.branch);
    git(
        &fixture.repo,
        &[
            "push",
            "--force",
            "origin",
            &format!("{}:{branch_ref}", base.base_oid),
        ],
    );
    assert!(recover_invocation(&state_path, &state.identity)
        .expect_err("diverged remote head must fail")
        .contains("remote branch head mismatch"));
    git(
        &fixture.repo,
        &["push", "--force", "origin", &format!("{head}:{branch_ref}")],
    );
    let tree = git_stdout(
        &fixture.repo,
        &["rev-parse", &format!("{}^{{tree}}", base.base_oid)],
    );
    let unrelated = git_stdout(&fixture.repo, &["commit-tree", &tree, "-m", "unrelated"]);
    let mut unrelated_base = state.clone();
    unrelated_base.identity.base_oid = unrelated.clone();
    write_invocation_atomic(&state_path, &unrelated_base).expect("persist unrelated base");
    git(&fixture.repo, &["config", &base_key, &unrelated]);
    assert!(recover_invocation(&state_path, &unrelated_base.identity)
        .expect_err("unrelated base ancestry must fail")
        .contains("does not descend"));
    git(&fixture.repo, &["config", &base_key, &base.base_oid]);
    write_invocation_atomic(&state_path, &state).expect("restore exact base");

    let mut foreign = state.clone();
    foreign.identity.worktree = foreign.identity.worktree.with_file_name("issue-999");
    write_invocation_atomic(&state_path, &foreign).expect("persist foreign path");
    assert!(recover_invocation(&state_path, &foreign.identity)
        .expect_err("foreign recovery path must fail")
        .contains("deterministic private scope"));
    write_invocation_atomic(&state_path, &state).expect("restore exact path");

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let foreign = fixture.root.join("foreign");
        fs::create_dir(&foreign).expect("create foreign directory");
        symlink(&foreign, &worktree.path).expect("install worktree symlink");
        assert!(recover_invocation(&state_path, &state.identity)
            .expect_err("symlink replacement must fail")
            .contains("symlink"));
        fs::remove_file(&worktree.path).expect("remove worktree symlink");
    }

    bridge::POST_CI_RECREATE_FAILPOINT.store(1, Ordering::SeqCst);
    let interrupted = recover_invocation(&state_path, &state.identity)
        .expect_err("crash after durable worktree recreation");
    assert!(
        interrupted.contains("after worktree recreation"),
        "{interrupted}"
    );
    let complete = bridge::cleanup_record_path(&state_path, "worktree-recreate-complete");
    assert!(!complete.exists());
    assert_eq!(git_stdout(&worktree.path, &["rev-parse", "HEAD"]), head);
    assert_eq!(
        fs::read(bridge::direct_attempt_paths(&review_artifacts, 0).record)
            .expect("old review evidence survives"),
        old_review
    );
    assert_eq!(
        bridge::registered_worktree_paths(&fixture.repo)
            .expect("registered worktrees")
            .iter()
            .filter(|path| *path == &worktree.path)
            .count(),
        1
    );
    assert_eq!(
        recover_invocation(&state_path, &state.identity).unwrap(),
        Some(state.clone())
    );
    assert!(complete.is_file());

    fs::write(worktree.path.join("foreign.txt"), "dirty\n").expect("dirty replacement");
    assert!(recover_invocation(&state_path, &state.identity)
        .expect_err("dirty replacement must fail")
        .contains("not clean"));
    fs::remove_file(worktree.path.join("foreign.txt")).expect("remove dirty file");
    assert_eq!(
        recover_invocation(&state_path, &state.identity)
            .expect("restart recovery")
            .expect("restart invocation"),
        state
    );
    fs::remove_dir_all(&worktree.path).expect("simulate vanished registered worktree");
    assert_eq!(
        recover_invocation(&state_path, &state.identity)
            .expect("repair exact prunable registration")
            .expect("prunable recovery invocation"),
        state
    );
    assert_eq!(git_stdout(&worktree.path, &["rev-parse", "HEAD"]), head);
    fs::write(&repaired, "repaired\n").expect("repair automatic reviewer");
    let retried = bridge::execute_direct_plan(
        &worktree.path,
        &review_plan(&"b".repeat(64)),
        &review_artifacts,
        None,
        Duration::from_secs(5),
    )
    .expect("recreated worktree reaches one identity-aware review retry");
    assert_eq!(retried[0].terminal, bridge::AttemptTerminal::Exited(0));
    let paths = bridge::direct_attempt_paths(&review_artifacts, 0);
    assert_eq!(direct_failure_archive_count(&review_artifacts), 1);
    assert!(!bridge::changed_automatic_reviewer_failure(
        &worktree.path,
        &paths,
        &automatic_review_command(&reviewer, &review_capture, &"b".repeat(64)),
        None,
    )
    .expect("restart must not reserve another review"));
    assert_eq!(direct_failure_archive_count(&review_artifacts), 1);
    assert_eq!(
        fs::read_dir(bridge::direct_attempt_reservation_directory(&paths))
            .expect("review reservations")
            .count(),
        2
    );
}

#[test]
fn autonomous_executor_bridge_recovers_every_merge_completion_phase() {
    let (fixture, mut state, _snapshot, _) =
        implementation_proof_fixture("recovery-completion-phases");
    commit_implementation(&state);
    bridge::record_worktree_creation_identity(
        &state.identity.repository_path,
        &state.identity.branch,
        &bridge::ResolvedBase {
            base_ref: state.identity.base_ref.clone(),
            base_oid: state.identity.base_oid.clone(),
            explore_mode: false,
        },
    )
    .expect("recovery metadata");
    state.head_oid = Some(git_stdout(&state.identity.worktree, &["rev-parse", "HEAD"]));
    state.pr = Some(17);
    let state_path = fixture.root.join("state/completion.json");

    for phase in [BridgePhase::ResultAccepted, BridgePhase::MergeRequested] {
        state.phase = phase;
        state.terminal_result = Some(format!("accepted:{}", "a".repeat(64)));
        write_invocation_atomic(&state_path, &state).expect("persist accepted phase");
        assert_eq!(
            recover_invocation(&state_path, &state.identity)
                .expect("recover accepted phase")
                .expect("active recovery")
                .phase,
            phase
        );
    }
    for phase in [BridgePhase::Merged, BridgePhase::CleanupPending] {
        state.phase = phase;
        state.terminal_result = Some("b".repeat(40));
        write_invocation_atomic(&state_path, &state).expect("persist merge phase");
        assert_eq!(
            recover_invocation(&state_path, &state.identity)
                .expect("recover merge phase")
                .expect("active recovery")
                .phase,
            phase
        );
    }

    bridge::ensure_cleanup_record(
        &bridge::cleanup_record_path(&state_path, "worktree-intent"),
        &bridge::cleanup_binding(&state),
        "test worktree intent",
    )
    .expect("removal intent");
    git(
        &state.identity.repository_path,
        &[
            "worktree",
            "remove",
            state.identity.worktree.to_str().expect("worktree"),
        ],
    );
    assert_eq!(
        recover_invocation(&state_path, &state.identity)
            .expect("recover after removal")
            .expect("cleanup recovery")
            .phase,
        BridgePhase::CleanupPending
    );
}

#[test]
fn autonomous_executor_bridge_recovers_before_and_after_base_regeneration() {
    let (fixture, mut state, _snapshot, _) =
        implementation_proof_fixture("recovery-base-regeneration");
    commit_implementation(&state);
    bridge::record_worktree_creation_identity(
        &state.identity.repository_path,
        &state.identity.branch,
        &bridge::ResolvedBase {
            base_ref: state.identity.base_ref.clone(),
            base_oid: state.identity.base_oid.clone(),
            explore_mode: false,
        },
    )
    .expect("recovery metadata");
    state.phase = BridgePhase::ReviewPassed;
    state.head_oid = Some(git_stdout(&state.identity.worktree, &["rev-parse", "HEAD"]));
    let state_path = fixture.root.join("state/base-regeneration.json");
    write_invocation_atomic(&state_path, &state).expect("persist pre-drift state");

    fs::write(fixture.root.join("seed/recovery-drift.txt"), "drift\n").expect("base drift");
    git(&fixture.root.join("seed"), &["add", "recovery-drift.txt"]);
    git(
        &fixture.root.join("seed"),
        &["commit", "-m", "test: advance recovery base"],
    );
    git(&fixture.root.join("seed"), &["push", "origin", "main"]);
    let new_base = git_stdout(&fixture.root.join("seed"), &["rev-parse", "HEAD"]);
    let mut current_identity = state.identity.clone();
    current_identity.base_oid = new_base;
    assert!(recover_invocation(&state_path, &current_identity)
        .expect("recover before reconciliation")
        .is_some());

    assert!(
        bridge::reconcile_base_drift_with_refresh(&state_path, &mut state, || {
            Ok(bridge::BridgeClaimOwnership::Refreshed { ttl_seconds: 60 })
        })
        .expect("regenerate current base")
    );
    assert!(recover_invocation(&state_path, &state.identity)
        .expect("recover after reconciliation")
        .is_some());
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_atomic_write_preserves_destination_links() {
    use std::os::unix::fs::symlink;

    let root = test_root("invocation-write-links");
    let state = root.join("state/invocation.json");
    fs::create_dir_all(state.parent().expect("state parent")).expect("state directory");
    let invocation = persisted_invocation();

    for target in [
        root.join("missing-invocation.json"),
        root.join("foreign-invocation.json"),
    ] {
        if target.file_name().and_then(|name| name.to_str()) == Some("foreign-invocation.json") {
            fs::write(&target, "foreign\n").expect("write foreign invocation");
        }
        symlink(&target, &state).expect("invocation destination symlink");
        let error = write_invocation_atomic(&state, &invocation)
            .expect_err("destination symlink must fail closed");
        assert!(error.contains("symlink"), "{error}");
        assert!(fs::symlink_metadata(&state)
            .expect("destination symlink remains")
            .file_type()
            .is_symlink());
        fs::remove_file(&state).expect("remove destination symlink");
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn autonomous_executor_bridge_recovery_rejects_replaced_foreign_repository() {
    let fixture = GitFixture::new("replaced-recovery");
    let base = resolve_base(&fixture.repo, &BTreeMap::new()).expect("resolve base");
    let scope = format!(
        "replaced_recovery_{}",
        TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    );
    let worktree =
        provision_issue_worktree(&fixture.repo, &scope, 10, &base).expect("provision worktree");
    let state_path = fixture.root.join("state/replaced.json");
    let identity = BridgeIdentity {
        repository: "owner/replaced".into(),
        repository_path: fixture.repo.clone(),
        issue: 10,
        worker_id: "worker".into(),
        branch: worktree.branch.clone(),
        claim_id: "claim".into(),
        invocation_id: "invocation".into(),
        base_ref: base.base_ref.clone(),
        base_oid: base.base_oid.clone(),
        worktree: worktree.path.clone(),
        runtime_environment_dir: None,
        runtime_session_id: None,
    };
    let invocation = PersistedInvocation {
        schema: bridge::INVOCATION_SCHEMA,
        identity: identity.clone(),
        harness: HarnessKind::Codex,
        phase: BridgePhase::Implementing,
        supervisor: None,
        process: None,
        progress_at: 1,
        pr: None,
        head_oid: None,
        closeout_path: None,
        closeout_digest: None,
        remote_snapshot_digest: None,
        draft_process: None,
        terminal_result: None,
        umbrella: None,
        current_child: None,
        implementation_repair_attempt: 0,
    };
    write_invocation_atomic(&state_path, &invocation).expect("persist invocation");
    git(
        &fixture.repo,
        &[
            "worktree",
            "remove",
            worktree.path.to_str().expect("worktree path"),
        ],
    );
    git(
        &fixture.root,
        &["init", worktree.path.to_str().expect("path")],
    );
    git(
        &worktree.path,
        &["config", "user.email", "autospec@example.invalid"],
    );
    git(&worktree.path, &["config", "user.name", "Autospec Test"]);
    fs::write(worktree.path.join("README.md"), "foreign\n").expect("foreign file");
    git(&worktree.path, &["add", "."]);
    git(&worktree.path, &["commit", "-m", "foreign"]);
    git(&worktree.path, &["branch", "-M", &worktree.branch]);

    let error = recover_invocation(&state_path, &identity)
        .expect_err("foreign replacement must not be recovered");

    assert!(
        error.contains("registered") || error.contains("repository"),
        "{error}"
    );
    let _ = fs::remove_dir_all(worktree.path.parent().expect("scope root"));
}
