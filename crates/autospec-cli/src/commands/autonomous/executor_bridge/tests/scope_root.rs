// executor_bridge tests: scope / root — 10 cases.
//
// Split out of tests.rs; see the note in that file.

use crate::commands::autonomous::executor_bridge as bridge;
use super::super::resolve_base;
use super::support_base::{GitFixture, TEST_SEQUENCE, git, git_stdout, test_environment, zero_effect_classifier_fixture};
use std::collections::BTreeMap;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

#[test]
fn autonomous_executor_bridge_recovers_exact_adopted_remote_implementation() {
    let (fixture, mut state, state_path, _) =
        zero_effect_classifier_fixture("adopted-remote-implementation", false, false);
    let implementation = state.identity.worktree.join("implementation.txt");
    fs::write(&implementation, "adopted implementation\n").expect("write implementation");
    git(&state.identity.worktree, &["add", "implementation.txt"]);
    git(
        &state.identity.worktree,
        &["commit", "-m", "test: preserve adopted implementation"],
    );
    let head = git_stdout(&state.identity.worktree, &["rev-parse", "HEAD"]);
    git(
        &state.identity.worktree,
        &["push", "-u", "origin", &state.identity.branch],
    );
    let closeout = state
        .identity
        .worktree
        .join(".autospec/executor-closeout.md");
    fs::create_dir_all(closeout.parent().expect("closeout parent"))
        .expect("create closeout parent");
    fs::write(
        &closeout,
        "## Closeout report\n\n\
Result: Preserved the adopted implementation.\n\
Claims: [verified] runtime the focused test exits with status 0.\n\
Proof type: runtime\n\
Before/after: Before 0 implementation files; after 1 implementation file.\n\
Artifacts: `implementation.txt`; rerun with `test -f implementation.txt`.\n\
Scoped git status: Added `implementation.txt`; closeout excluded from the commit.\n\
One likely hidden failure: The fixture does not exercise a pull request.\n",
    )
    .expect("write closeout");
    fs::set_permissions(&closeout, fs::Permissions::from_mode(0o600)).expect("private closeout");
    bridge::ensure_active_worktree_ownership(
        &state.identity.repository_path,
        state.identity.worktree.parent().expect("scope root"),
        state.identity.issue,
        &state.identity.worktree,
        &state.identity.branch,
        &state.identity.claim_id,
        &state.identity.invocation_id,
    )
    .expect("record adopted ownership transfer");
    let snapshot_path = bridge::remote_snapshot_path(&state_path);
    let mut snapshot: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&snapshot_path).expect("read remote snapshot"))
            .expect("parse remote snapshot");
    snapshot["identity"]["local_head"] = serde_json::json!(head);
    snapshot["refs"][format!("refs/heads/{}", state.identity.branch)] = serde_json::json!(head);
    let snapshot = format!("{snapshot}\n");
    fs::write(&snapshot_path, &snapshot).expect("write adopted remote snapshot");
    fs::set_permissions(&snapshot_path, fs::Permissions::from_mode(0o600))
        .expect("secure adopted remote snapshot");
    state.remote_snapshot_digest = Some(bridge::sha256_hex(snapshot.as_bytes()));
    bridge::write_invocation_atomic(&state_path, &state).expect("persist adopted invocation");

    assert!(
        bridge::recoverable_implementation_completion_for_state(&state_path, &state)
            .expect("classify exact adopted implementation")
    );

    let transfer_path = bridge::ownership_transfer_path(
        state.identity.worktree.parent().expect("scope root"),
        state.identity.issue,
    );
    let exact_transfer = fs::read_to_string(&transfer_path).expect("read exact transfer");

    let seed = fixture.root.join("seed");
    git(&seed, &["checkout", "main"]);
    fs::write(seed.join("base-advance.txt"), "advanced base\n").expect("advance base branch");
    git(&seed, &["add", "base-advance.txt"]);
    git(&seed, &["commit", "-m", "test: advance adopted base"]);
    git(&seed, &["push", "origin", "main"]);
    let advanced_base = git_stdout(&seed, &["rev-parse", "HEAD"]);
    git(&state.identity.worktree, &["fetch", "origin", "main"]);
    git(
        &state.identity.worktree,
        &["merge", "--no-ff", "--no-edit", &advanced_base],
    );
    let reconciled_head = git_stdout(&state.identity.worktree, &["rev-parse", "HEAD"]);
    git(
        &state.identity.worktree,
        &["push", "origin", &state.identity.branch],
    );
    state.identity.base_oid = advanced_base.clone();
    let mut snapshot: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(&snapshot_path).expect("read reconciled remote snapshot"),
    )
    .expect("parse reconciled remote snapshot");
    snapshot["identity"]["base_oid"] = serde_json::json!(advanced_base);
    snapshot["identity"]["local_head"] = serde_json::json!(reconciled_head);
    snapshot["refs"]["refs/heads/main"] = serde_json::json!(advanced_base);
    snapshot["refs"][format!("refs/heads/{}", state.identity.branch)] =
        serde_json::json!(reconciled_head);
    let snapshot = format!("{snapshot}\n");
    fs::write(&snapshot_path, &snapshot).expect("write reconciled remote snapshot");
    fs::set_permissions(&snapshot_path, fs::Permissions::from_mode(0o600))
        .expect("secure reconciled remote snapshot");
    state.remote_snapshot_digest = Some(bridge::sha256_hex(snapshot.as_bytes()));
    bridge::write_invocation_atomic(&state_path, &state)
        .expect("persist reconciled adopted invocation");

    assert!(
        bridge::recoverable_implementation_completion_for_state(&state_path, &state)
            .expect("classify exact adopted base-reconciliation merge"),
        "the transfer head may be the first parent of the exact base merge"
    );

    fs::write(
        seed.join("post-crash-base.txt"),
        "post-crash base advance\n",
    )
    .expect("advance base after executor crash");
    git(&seed, &["add", "post-crash-base.txt"]);
    git(
        &seed,
        &["commit", "-m", "test: advance base after executor crash"],
    );
    git(&seed, &["push", "origin", "main"]);
    let post_crash_base = git_stdout(&seed, &["rev-parse", "HEAD"]);
    assert!(
        bridge::recoverable_implementation_completion_for_state(&state_path, &state)
            .expect("classify adopted implementation after base advance"),
        "a descendant main advance must remain recoverable for the later base-drift gate"
    );

    git(&seed, &["checkout", "--orphan", "unrelated-main"]);
    fs::write(seed.join("unrelated-main.txt"), "unrelated main\n").expect("write unrelated main");
    git(&seed, &["add", "-A"]);
    git(&seed, &["commit", "-m", "test: create unrelated main"]);
    let unrelated_main = git_stdout(&seed, &["rev-parse", "HEAD"]);
    let remote = fixture.root.join("remote.git");
    git(&seed, &["push", "origin", "HEAD:refs/heads/unrelated-main"]);
    git(
        &fixture.root,
        &[
            "--git-dir",
            remote.to_str().expect("remote path"),
            "update-ref",
            "refs/heads/main",
            &unrelated_main,
        ],
    );
    git(
        &fixture.root,
        &[
            "--git-dir",
            remote.to_str().expect("remote path"),
            "update-ref",
            "-d",
            "refs/heads/unrelated-main",
        ],
    );
    assert!(
        !bridge::recoverable_implementation_completion_for_state(&state_path, &state)
            .expect("reject unrelated post-crash base"),
        "a non-descendant main replacement must stay fail-closed"
    );
    git(
        &fixture.root,
        &[
            "--git-dir",
            remote.to_str().expect("remote path"),
            "update-ref",
            "refs/heads/main",
            &post_crash_base,
        ],
    );

    let mut mismatched: serde_json::Value =
        serde_json::from_str(&exact_transfer).expect("parse exact transfer");
    mismatched["to_claim_id"] = serde_json::json!("claim-other");
    fs::write(&transfer_path, format!("{mismatched}\n")).expect("write mismatched transfer");
    fs::set_permissions(&transfer_path, fs::Permissions::from_mode(0o600))
        .expect("secure mismatched transfer");
    assert!(
        !bridge::recoverable_implementation_completion_for_state(&state_path, &state)
            .expect("reject mismatched transfer")
    );

    fs::write(&transfer_path, exact_transfer).expect("restore exact transfer");
    fs::set_permissions(&transfer_path, fs::Permissions::from_mode(0o600))
        .expect("secure restored transfer");
    let exact_transfer = fs::read_to_string(&transfer_path).expect("reread exact transfer");
    let mut mismatched_head: serde_json::Value =
        serde_json::from_str(&exact_transfer).expect("parse exact transfer");
    mismatched_head["head_oid"] = serde_json::json!("f".repeat(40));
    fs::write(&transfer_path, format!("{mismatched_head}\n"))
        .expect("write mismatched transfer head");
    fs::set_permissions(&transfer_path, fs::Permissions::from_mode(0o600))
        .expect("secure mismatched transfer head");
    assert!(
        !bridge::recoverable_implementation_completion_for_state(&state_path, &state)
            .expect("reject mismatched transfer head")
    );

    fs::write(&transfer_path, exact_transfer).expect("restore exact transfer head");
    fs::set_permissions(&transfer_path, fs::Permissions::from_mode(0o600))
        .expect("secure restored transfer head");
    git(&seed, &["fetch", "origin", &state.identity.branch]);
    git(&seed, &["checkout", "-B", "remote-advance", "FETCH_HEAD"]);
    fs::write(seed.join("remote-advance.txt"), "advanced\n").expect("advance remote branch");
    git(&seed, &["add", "remote-advance.txt"]);
    git(&seed, &["commit", "-m", "test: advance remote branch"]);
    git(
        &seed,
        &["push", "origin", &format!("HEAD:{}", state.identity.branch)],
    );
    assert!(
        !bridge::recoverable_implementation_completion_for_state(&state_path, &state)
            .expect("reject mismatched remote OID")
    );
}

#[cfg(unix)]
fn assert_private_scope(scope_root: &Path) {
    assert!(
        scope_root.is_dir(),
        "recovery must recreate the exact scope"
    );
    assert_eq!(
        fs::metadata(scope_root)
            .expect("recreated scope metadata")
            .permissions()
            .mode()
            & 0o777,
        0o700,
        "recreated scope must remain private"
    );
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_recreates_absent_exact_scope_after_durable_marker() {
    let environment = test_environment();
    let (_fixture, state, state_path, _) =
        zero_effect_classifier_fixture("zero-effect-missing-scope", false, true);
    let scope_root = state.identity.worktree.parent().expect("scope root");
    fs::remove_dir(scope_root).expect("remove empty executor scope");

    assert!(
        bridge::recoverable_zero_effect_completion_for_state(&state_path, &state)
            .expect("classify exact missing-scope completion")
    );
    assert!(
        !scope_root.exists(),
        "read-only classification must not recreate the scope"
    );
    assert!(
        !bridge::zero_effect_recovery_marker_path(&state_path).exists(),
        "read-only classification must not persist the recovery marker"
    );

    environment.zero_effect_recovery(
        bridge::ZeroEffectRecoveryFailpoint::AfterScopeCreate,
    );
    let interrupted = bridge::prepare_zero_effect_recovery(&state_path, &state)
        .expect_err("interrupt after exact scope recreation");
    environment.zero_effect_recovery(bridge::ZeroEffectRecoveryFailpoint::None);
    assert!(
        interrupted.contains("after scope recreation"),
        "{interrupted}"
    );
    assert!(
        bridge::zero_effect_recovery_marker_path(&state_path).is_file(),
        "scope recreation must occur only after the marker is durable"
    );
    assert!(
        !state.identity.worktree.exists(),
        "scope recreation crash must precede worktree repair"
    );
    assert_private_scope(scope_root);
    assert!(
        bridge::prepare_zero_effect_recovery(&state_path, &state)
            .expect("resume after exact scope recreation crash"),
        "recovery must repair the worktree idempotently after restart"
    );
    assert!(
        bridge::prepare_zero_effect_recovery(&state_path, &state)
            .expect("repeat completed missing-scope recovery"),
        "completed recovery must remain idempotent"
    );
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_retries_scope_parent_sync_before_worktree_repair() {
    let (_fixture, state, state_path, _) =
        zero_effect_classifier_fixture("zero-effect-scope-parent-sync", false, true);
    let scope_root = state.identity.worktree.parent().expect("scope root");
    fs::remove_dir(scope_root).expect("remove empty executor scope");

    bridge::ZERO_EFFECT_SCOPE_PARENT_SYNC_FAILPOINT.store(1, Ordering::SeqCst);
    let first = bridge::prepare_zero_effect_recovery(&state_path, &state)
        .expect_err("scope parent sync must fail after recreation");
    assert!(
        first.contains("sync recreated executor zero-effect scope"),
        "{first}"
    );
    assert_private_scope(scope_root);
    assert!(
        !state.identity.worktree.exists(),
        "repair must not start before the recreated scope is durable"
    );

    bridge::ZERO_EFFECT_SCOPE_PARENT_SYNC_FAILPOINT.store(1, Ordering::SeqCst);
    let retry = bridge::prepare_zero_effect_recovery(&state_path, &state)
        .expect_err("restart must retry parent sync for the existing scope");
    assert!(
        retry.contains("sync recreated executor zero-effect scope"),
        "{retry}"
    );
    assert!(
        !state.identity.worktree.exists(),
        "restart must not repair before the parent sync succeeds"
    );

    assert!(
        bridge::prepare_zero_effect_recovery(&state_path, &state)
            .expect("resume after durable scope parent sync"),
        "recovery must proceed after the parent sync succeeds"
    );
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_hardens_root_only_after_zero_effect_marker() {
    let (_fixture, state, state_path, _) =
        zero_effect_classifier_fixture("zero-effect-root-hardening", false, true);
    let scope_root = state.identity.worktree.parent().expect("scope root");
    fs::remove_dir(scope_root).expect("remove empty executor scope");

    bridge::EXECUTOR_ROOT_HARDEN_FAILPOINT.store(1, Ordering::SeqCst);
    assert!(
        bridge::recoverable_zero_effect_completion_for_state(&state_path, &state)
            .expect("read-only missing-scope classification")
    );
    assert_eq!(
        bridge::EXECUTOR_ROOT_HARDEN_FAILPOINT.load(Ordering::SeqCst),
        1,
        "classification must not harden or otherwise mutate the executor root"
    );
    assert!(
        !bridge::zero_effect_recovery_marker_path(&state_path).exists(),
        "classification must remain marker-free"
    );

    let error = bridge::prepare_zero_effect_recovery(&state_path, &state)
        .expect_err("recovery must harden the executor root before scope recreation");
    assert!(error.contains("harden executor worktree root"), "{error}");
    assert!(
        bridge::zero_effect_recovery_marker_path(&state_path).is_file(),
        "root hardening must happen only after durable recovery authorization"
    );
    assert!(
        !scope_root.exists() && !state.identity.worktree.exists(),
        "failed root hardening must precede scope creation and worktree repair"
    );
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_scope_classification_leaves_root_mode_unchanged() {
    let fixture = GitFixture::new("scope-root-mode");
    let executor_root = fixture.root.join("executor-root");
    let scope_root = executor_root.join("missing-scope");
    fs::create_dir(&executor_root).expect("create isolated executor root");
    fs::set_permissions(&executor_root, fs::Permissions::from_mode(0o775))
        .expect("make isolated executor root group-writable");

    assert!(
        !bridge::validate_zero_effect_scope_identity(&fixture.repo, &scope_root)
            .expect("read-only absent-scope validation")
    );
    assert_eq!(
        fs::metadata(&executor_root)
            .expect("executor root metadata")
            .permissions()
            .mode()
            & 0o777,
        0o775,
        "read-only classification must not chmod the shared root"
    );

    bridge::harden_executor_worktree_root(&fixture.repo, &executor_root)
        .expect("harden isolated executor root");
    assert_eq!(
        fs::metadata(&executor_root)
            .expect("hardened executor root metadata")
            .permissions()
            .mode()
            & 0o777,
        0o700,
        "authorized recovery must make the shared root private"
    );
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_rejects_symlinked_executor_root_before_hardening() {
    use std::os::unix::fs::symlink;

    let fixture = GitFixture::new("symlinked-executor-root");
    let target = fixture.root.join("foreign-root");
    let executor_root = fixture.root.join("executor-root-link");
    fs::create_dir(&target).expect("create foreign root target");
    symlink(&target, &executor_root).expect("create executor root symlink");
    let scope_root = executor_root.join("missing-scope");

    let classification = bridge::validate_zero_effect_scope_identity(&fixture.repo, &scope_root)
        .expect_err("symlinked root must fail closed during classification");
    assert!(classification.contains("symlink"), "{classification}");
    let hardening = bridge::harden_executor_worktree_root(&fixture.repo, &executor_root)
        .expect_err("symlinked root must fail closed before chmod");
    assert!(hardening.contains("symlink"), "{hardening}");
    assert!(
        !scope_root.exists(),
        "symlinked root rejection must not create the repository scope"
    );
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_provisioning_hardens_root_before_scope_creation() {
    let fixture = GitFixture::new("provision-root-hardening");
    let base = resolve_base(&fixture.repo, &BTreeMap::new()).expect("resolve base");
    let repository_scope = format!(
        "provision-root-hardening-{}",
        TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    );
    let scope_root = PathBuf::from("/tmp/autospec-executor")
        .join(bridge::safe_scope(&repository_scope).expect("safe scope"));

    bridge::EXECUTOR_ROOT_HARDEN_FAILPOINT.store(1, Ordering::SeqCst);
    match bridge::provision_issue_worktree(&fixture.repo, &repository_scope, 42, &base) {
        Err(error) => assert!(error.contains("harden executor worktree root"), "{error}"),
        Ok(worktree) => {
            git(
                &fixture.repo,
                &[
                    "worktree",
                    "remove",
                    worktree.path.to_str().expect("worktree path"),
                ],
            );
            let _ = fs::remove_dir_all(&scope_root);
            panic!("provisioning skipped executor-root hardening");
        }
    }
    assert!(
        !scope_root.exists(),
        "root hardening must precede repository scope creation"
    );
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_missing_scope_rejects_nondeterministic_path() {
    let (_nondeterministic_fixture, mut nondeterministic, state_path, _) =
        zero_effect_classifier_fixture("zero-effect-nondeterministic-scope", false, true);
    nondeterministic.identity.worktree = nondeterministic
        .identity
        .worktree
        .with_file_name("issue-999");
    let error =
        bridge::recoverable_zero_effect_completion_for_state(&state_path, &nondeterministic)
            .expect_err("non-deterministic worktree must remain fail-closed");
    assert!(error.contains("deterministic private scope"), "{error}");
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_missing_scope_rejects_symlink() {
    use std::os::unix::fs::symlink;

    let (symlink_fixture, symlink_state, symlink_state_path, _) =
        zero_effect_classifier_fixture("zero-effect-symlink-scope", false, true);
    let symlink_scope = symlink_state
        .identity
        .worktree
        .parent()
        .expect("symlink scope");
    fs::remove_dir(symlink_scope).expect("remove empty symlink scope");
    let foreign = symlink_fixture.root.join("foreign-scope");
    fs::create_dir(&foreign).expect("create foreign scope target");
    symlink(&foreign, symlink_scope).expect("install foreign scope symlink");
    let error =
        bridge::recoverable_zero_effect_completion_for_state(&symlink_state_path, &symlink_state)
            .expect_err("symlink scope must remain fail-closed");
    assert!(error.contains("symlink"), "{error}");
    assert!(
        !bridge::zero_effect_recovery_marker_path(&symlink_state_path).exists(),
        "unsafe scope must not gain a recovery marker"
    );
    fs::remove_file(symlink_scope).expect("remove scope symlink");
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_missing_scope_rejects_non_private_directory() {
    let (_public_fixture, public_state, public_state_path, _) =
        zero_effect_classifier_fixture("zero-effect-public-scope", false, true);
    let public_scope = public_state
        .identity
        .worktree
        .parent()
        .expect("public scope");
    fs::set_permissions(public_scope, fs::Permissions::from_mode(0o755))
        .expect("make scope non-private");
    let error =
        bridge::recoverable_zero_effect_completion_for_state(&public_state_path, &public_state)
            .expect_err("non-private scope must remain fail-closed");
    assert!(error.contains("private"), "{error}");
    assert!(
        !bridge::zero_effect_recovery_marker_path(&public_state_path).exists(),
        "non-private scope must not gain a recovery marker"
    );
}
