// executor_bridge tests: closeout / harness — 6 cases.
//
// Split out of tests.rs; see the note in that file.

use super::super::{resolve_base, BridgePhase};
use super::support_base::{
    git, git_stdout, zero_effect_classifier_fixture, GitFixture, TEST_SEQUENCE,
};
use super::support_invocation::supervision_state;
use crate::commands::autonomous::executor_bridge as bridge;
use std::collections::BTreeMap;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::sync::atomic::Ordering;

#[test]
fn autonomous_executor_bridge_codex_sandbox_repairs_prunable_post_child_worktree() {
    let fixture = GitFixture::new("missing-post-child-worktree");
    let base = resolve_base(&fixture.repo, &BTreeMap::new()).expect("resolve base");
    let scope = format!(
        "missing_post_child_{}_{}",
        std::process::id(),
        TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    );
    let worktree = bridge::provision_issue_worktree_for_claim(
        &fixture.repo,
        &scope,
        42,
        &base,
        Some(("claim-repair", "invocation-repair")),
    )
    .expect("provision issue worktree");
    let mut state = supervision_state(&fixture);
    state.phase = BridgePhase::ImplementationComplete;
    state.identity.repository_path = fs::canonicalize(&fixture.repo).expect("canonical repo");
    state.identity.worktree = worktree.path.clone();
    state.identity.branch = worktree.branch.clone();
    state.identity.base_ref = base.base_ref.clone();
    state.identity.base_oid = base.base_oid.clone();
    fs::remove_dir_all(&worktree.path).expect("simulate disappeared worktree");

    bridge::WORKTREE_REPAIR_FAILPOINT.store(1, Ordering::SeqCst);
    let interrupted = bridge::repair_missing_post_child_worktree(&state)
        .expect_err("interrupt repair after durable prune");
    assert!(interrupted.contains("injected executor worktree repair crash"));
    let repair_intent = worktree
        .path
        .parent()
        .expect("scope root")
        .join("issue-42.repair-intent.json");
    assert!(repair_intent.is_file());
    assert!(
        !git_stdout(&fixture.repo, &["worktree", "list", "--porcelain"])
            .contains(&format!("worktree {}", worktree.path.display()))
    );

    assert!(
        bridge::repair_missing_post_child_worktree(&state).expect("repair exact prunable worktree")
    );
    assert!(!repair_intent.exists());
    assert!(worktree.path.is_dir());
    assert_eq!(
        git_stdout(&worktree.path, &["status", "--porcelain=v1"]),
        ""
    );
    assert_eq!(
        git_stdout(&worktree.path, &["symbolic-ref", "--short", "HEAD"]),
        worktree.branch
    );

    let worktree_path = worktree.path.to_str().expect("worktree path");
    git(&fixture.repo, &["worktree", "remove", worktree_path]);
    let _ = fs::remove_dir_all(worktree.path.parent().expect("scope root"));
}

#[test]
fn autonomous_executor_bridge_classifies_only_exact_zero_effect_recovery() {
    let (_exact_fixture, exact, exact_state_path, _) =
        zero_effect_classifier_fixture("zero-effect-exact", false, true);
    assert!(
        bridge::exact_prunable_zero_effect_completion(&exact_state_path, &exact)
            .expect("classify exact zero-effect completion")
    );

    let (_changed_fixture, changed, changed_state_path, _) =
        zero_effect_classifier_fixture("zero-effect-changed", true, true);
    assert!(
        !bridge::exact_prunable_zero_effect_completion(&changed_state_path, &changed)
            .expect("changed HEAD remains fail-closed")
    );

    let (_dirty_fixture, dirty, dirty_state_path, _) =
        zero_effect_classifier_fixture("zero-effect-dirty", false, false);
    fs::write(dirty.identity.worktree.join("dirty.txt"), "dirty\n").expect("dirty fixture");
    assert!(
        !bridge::exact_prunable_zero_effect_completion(&dirty_state_path, &dirty)
            .expect("present dirty worktree remains fail-closed")
    );

    let (_malformed_fixture, malformed, malformed_state_path, malformed_exit) =
        zero_effect_classifier_fixture("zero-effect-malformed", false, true);
    fs::write(&malformed_exit, [1_u8; 16]).expect("malformed exit record");
    assert!(
        bridge::exact_prunable_zero_effect_completion(&malformed_state_path, &malformed)
            .expect_err("malformed exit remains an invariant")
            .contains("malformed")
    );

    let (_pr_fixture, mut pr_state, pr_state_path, _) =
        zero_effect_classifier_fixture("zero-effect-prelaunch-pr", false, true);
    let snapshot_path = bridge::remote_snapshot_path(&pr_state_path);
    let mut snapshot: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&snapshot_path).expect("read prelaunch snapshot"))
            .expect("parse prelaunch snapshot");
    snapshot["pull_requests"] = serde_json::json!([{
        "number": 17,
        "body": "pre-existing",
        "headRefName": pr_state.identity.branch,
        "headRefOid": pr_state.identity.base_oid,
        "isDraft": true,
        "baseRefName": "main",
    }]);
    let snapshot = format!("{snapshot}\n");
    fs::write(&snapshot_path, &snapshot).expect("write prelaunch PR snapshot");
    pr_state.remote_snapshot_digest = Some(bridge::sha256_hex(snapshot.as_bytes()));
    bridge::write_invocation_atomic(&pr_state_path, &pr_state)
        .expect("persist prelaunch PR digest");
    assert!(
        !bridge::exact_prunable_zero_effect_completion(&pr_state_path, &pr_state)
            .expect("prelaunch feature PR remains fail-closed")
    );

    let (ambiguous_fixture, ambiguous, ambiguous_state_path, _) =
        zero_effect_classifier_fixture("zero-effect-ambiguous", false, true);
    let other = ambiguous_fixture.root.join("other-prunable-worktree");
    git(
        &ambiguous_fixture.repo,
        &[
            "worktree",
            "add",
            "--detach",
            other.to_str().expect("other worktree path"),
            "origin/main",
        ],
    );
    fs::remove_dir_all(other).expect("make second worktree prunable");
    assert!(
        !bridge::exact_prunable_zero_effect_completion(&ambiguous_state_path, &ambiguous)
            .expect("ambiguous prunable registrations remain fail-closed")
    );
}

#[test]
fn autonomous_executor_bridge_prepares_private_closeout_sink() {
    let (_fixture, state, _state_path, _) =
        zero_effect_classifier_fixture("private-closeout-sink", false, false);
    let artifact_dir = state.identity.worktree.join(".autospec");
    let closeout = artifact_dir.join("executor-closeout.md");
    fs::create_dir_all(&artifact_dir).expect("create public artifact directory");
    fs::write(&closeout, "preserved closeout\n").expect("write public closeout");
    fs::set_permissions(&artifact_dir, fs::Permissions::from_mode(0o775))
        .expect("make artifact directory public");
    fs::set_permissions(&closeout, fs::Permissions::from_mode(0o664))
        .expect("make closeout public");

    bridge::prepare_private_closeout_sink(&state.identity.worktree, &closeout)
        .expect("prepare private closeout sink");

    assert_eq!(
        fs::metadata(&artifact_dir)
            .expect("artifact directory metadata")
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(&closeout)
            .expect("closeout metadata")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(
        fs::read_to_string(&closeout).expect("read preserved closeout"),
        "preserved closeout\n"
    );

    fs::remove_file(&closeout).expect("remove recovered closeout");
    fs::set_permissions(&artifact_dir, fs::Permissions::from_mode(0o775))
        .expect("restore public artifact directory");
    bridge::prepare_private_closeout_sink(&state.identity.worktree, &closeout)
        .expect("create private closeout sink");
    assert_eq!(
        fs::metadata(&artifact_dir)
            .expect("created artifact directory metadata")
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(&closeout)
            .expect("created closeout metadata")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(
        fs::metadata(&closeout)
            .expect("created closeout length")
            .len(),
        0
    );
}

#[test]
fn autonomous_executor_bridge_rehardens_replaced_closeout_after_harness_exit_and_rejects_links() {
    let (_fixture, state, _state_path, _) =
        zero_effect_classifier_fixture("reharden-replaced-closeout", false, false);
    let closeout = state
        .identity
        .worktree
        .join(".autospec/executor-closeout.md");
    bridge::prepare_private_closeout_sink(&state.identity.worktree, &closeout)
        .expect("prepare original closeout sink");
    let replacement = closeout.with_extension("replacement");
    fs::write(&replacement, "replacement closeout\n").expect("write replacement closeout");
    fs::set_permissions(&replacement, fs::Permissions::from_mode(0o664))
        .expect("make replacement public");
    fs::rename(&replacement, &closeout).expect("replace prepared closeout sink");

    bridge::reharden_completed_closeout(&state.identity.worktree, &closeout)
        .expect("reharden completed closeout");

    assert_eq!(
        fs::metadata(&closeout)
            .expect("rehardened closeout metadata")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(
        fs::read_to_string(closeout).expect("read replacement closeout"),
        "replacement closeout\n"
    );

    let closeout = state
        .identity
        .worktree
        .join(".autospec/executor-closeout.md");
    let alias = closeout.with_extension("alias");
    fs::remove_file(&closeout).expect("remove hardened closeout");
    fs::write(&alias, "aliased closeout\n").expect("write aliased closeout");
    fs::set_permissions(&alias, fs::Permissions::from_mode(0o664)).expect("make alias public");
    fs::hard_link(&alias, &closeout).expect("hard-link closeout replacement");

    let error = bridge::reharden_completed_closeout(&state.identity.worktree, &closeout)
        .expect_err("reject hard-linked closeout");
    assert!(error.contains("hard link"), "{error}");
    assert_eq!(
        fs::metadata(alias)
            .expect("alias metadata")
            .permissions()
            .mode()
            & 0o777,
        0o664,
        "rejection must not chmod the aliased inode"
    );
}

#[test]
fn autonomous_executor_bridge_hardens_closeout_before_recovery_classification() {
    let (_fixture, mut state, state_path, _) =
        zero_effect_classifier_fixture("private-closeout-recovery", false, false);
    state.identity.invocation_id = format!("{}-{}", state.identity.issue, state.identity.claim_id);
    let snapshot_path = bridge::remote_snapshot_path(&state_path);
    let mut snapshot: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&snapshot_path).expect("read remote snapshot"))
            .expect("parse remote snapshot");
    snapshot["identity"]["invocation_id"] = serde_json::json!(state.identity.invocation_id);
    let snapshot = format!("{snapshot}\n");
    fs::write(&snapshot_path, &snapshot).expect("write exact remote snapshot");
    fs::set_permissions(&snapshot_path, fs::Permissions::from_mode(0o600))
        .expect("secure exact remote snapshot");
    state.remote_snapshot_digest = Some(bridge::sha256_hex(snapshot.as_bytes()));
    bridge::write_invocation_atomic(&state_path, &state).expect("persist exact invocation");
    let sinks = bridge::output_sink_paths(&state_path, &state.identity.invocation_id)
        .expect("exact output sinks");
    fs::create_dir_all(sinks.exit_status.parent().expect("exit parent"))
        .expect("create exact exit parent");
    let mut exit = [0_u8; 16];
    exit[..4].copy_from_slice(&0_i32.to_ne_bytes());
    exit[4..8].copy_from_slice(b"EXIT");
    exit[8..12].copy_from_slice(&0_i32.to_ne_bytes());
    exit[12..].copy_from_slice(b"DONE");
    fs::write(&sinks.exit_status, exit).expect("write exact exit status");
    fs::set_permissions(&sinks.exit_status, fs::Permissions::from_mode(0o600))
        .expect("secure exact exit status");
    fs::write(
        state.identity.worktree.join("implementation.txt"),
        "implemented\n",
    )
    .expect("write implementation diff");
    let closeout = state
        .identity
        .worktree
        .join(".autospec/executor-closeout.md");
    fs::create_dir_all(closeout.parent().expect("closeout parent"))
        .expect("create closeout parent");
    fs::write(
        &closeout,
        "## Closeout report\n\n\
Result: Added the requested implementation.\n\
Claims: [verified] runtime the focused test exits with status 0.\n\
Proof type: runtime\n\
Before/after: Before 0 implementation files; after 1 implementation file.\n\
Artifacts: `implementation.txt`; rerun with `test -f implementation.txt`.\n\
Scoped git status: Added `implementation.txt`; closeout excluded from the commit.\n\
One likely hidden failure: The focused fixture does not exercise a remote push.\n",
    )
    .expect("write closeout");
    fs::set_permissions(&closeout, fs::Permissions::from_mode(0o664))
        .expect("make closeout public");

    let state_dir = state_path.parent().expect("state directory");
    let generation = &bridge::sha256_hex(state.identity.claim_id.as_bytes())[..16];
    let exact_state_path =
        state_dir.join(format!("issue-{}-{generation}.json", state.identity.issue));
    fs::rename(&state_path, &exact_state_path).expect("move exact invocation");
    fs::rename(
        bridge::remote_snapshot_path(&state_path),
        bridge::remote_snapshot_path(&exact_state_path),
    )
    .expect("move exact remote snapshot");
    let lease = crate::commands::claim::ClaimLease {
        issue: state.identity.issue,
        repo: state.identity.repository.clone(),
        worker_id: state.identity.worker_id.clone(),
        branch: state.identity.branch.clone(),
        claim_id: state.identity.claim_id.clone(),
        session_id: None,
    };

    assert!(
        bridge::recoverable_implementation_completion(state_dir, &lease)
            .expect("classify hardened completion")
    );
    assert_eq!(
        fs::metadata(&closeout)
            .expect("hardened closeout metadata")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[test]
fn autonomous_executor_bridge_interrupted_harness_receipt_requires_durable_exit() {
    let (_fixture, mut state, original_state_path, _) =
        zero_effect_classifier_fixture("interrupted-harness-receipt", false, false);
    state.phase = BridgePhase::Interrupted;
    state.supervisor = None;
    state.process = None;
    state.identity.invocation_id = format!("{}-{}", state.identity.issue, state.identity.claim_id);
    let state_dir = original_state_path.parent().expect("state directory");
    let generation = &bridge::sha256_hex(state.identity.claim_id.as_bytes())[..16];
    let state_path = state_dir.join(format!("issue-{}-{generation}.json", state.identity.issue));
    bridge::write_invocation_atomic(&state_path, &state).expect("persist interrupted state");
    let sinks =
        bridge::output_sink_paths_for_state(&state_path, &state).expect("interrupted output sinks");
    for path in [&sinks.exit_status, &sinks.supervisor_identity] {
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => panic!("clear recovery evidence {}: {error}", path.display()),
        }
    }

    let lease = crate::commands::claim::ClaimLease {
        issue: state.identity.issue,
        repo: state.identity.repository.clone(),
        worker_id: state.identity.worker_id.clone(),
        branch: state.identity.branch.clone(),
        claim_id: state.identity.claim_id.clone(),
        session_id: None,
    };
    assert!(
        !bridge::recoverable_interrupted_harness_receipt(state_dir, &lease)
            .expect("classify missing exit evidence")
    );

    let mut foreign = lease.clone();
    foreign.repo = "other/repo".to_string();
    let error = bridge::recoverable_interrupted_harness_receipt(state_dir, &foreign)
        .expect_err("foreign lease must fail closed");
    assert!(error.contains("does not match"), "error={error}");

    fs::create_dir_all(sinks.exit_status.parent().expect("exit parent"))
        .expect("create exit parent");
    let mut exit = [0_u8; 16];
    exit[..4].copy_from_slice(&0_i32.to_ne_bytes());
    exit[4..8].copy_from_slice(b"EXIT");
    exit[8..12].copy_from_slice(&0_i32.to_ne_bytes());
    exit[12..].copy_from_slice(b"DONE");
    fs::write(&sinks.exit_status, exit).expect("write completed exit receipt");
    fs::set_permissions(&sinks.exit_status, fs::Permissions::from_mode(0o600))
        .expect("secure exit receipt");

    assert!(
        bridge::recoverable_interrupted_harness_receipt(state_dir, &lease)
            .expect("classify durable exit evidence")
    );
}
