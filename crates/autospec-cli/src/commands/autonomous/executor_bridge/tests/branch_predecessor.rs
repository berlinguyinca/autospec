// executor_bridge tests: branch / predecessor — 6 cases.
//
// Split out of tests.rs; see the note in that file.

use super::super::{provision_issue_worktree, resolve_base, BridgePhase, MutationSnapshot};
use super::support_base::{git, git_stdout, test_environment, GitFixture, TEST_SEQUENCE};
use super::support_invocation::{
    prunable_zero_effect_branch_fixture, shell_invocation, supervision_config, supervision_state,
};
use crate::commands::autonomous::executor_bridge as bridge;
use std::collections::BTreeMap;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::Ordering;

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_codex_sandbox_entrypoint_live_recovery_helper() {
    let _environment = test_environment();
    let Some(state_path) = std::env::var_os("AUTOSPEC_TEST_RECOVERY_STATE") else {
        return;
    };
    let state_path = PathBuf::from(state_path);
    let event_log =
        PathBuf::from(std::env::var_os("AUTOSPEC_TEST_RECOVERY_EVENTS").expect("event log"));
    let ready = PathBuf::from(std::env::var_os("AUTOSPEC_TEST_RECOVERY_READY").expect("ready"));
    let release =
        PathBuf::from(std::env::var_os("AUTOSPEC_TEST_RECOVERY_RELEASE").expect("release"));
    let mut state = bridge::PersistedInvocation::from_json(
        &fs::read_to_string(&state_path).expect("read recovery fixture state"),
    )
    .expect("parse recovery fixture state");
    let snapshot =
        MutationSnapshot::capture(&state.identity.repository_path, &state.identity.branch)
            .expect("capture recovery fixture snapshot");
    let worktree = state.identity.worktree.clone();
    let command = format!(
        "printf ready > '{}'; while [ ! -f '{}' ]; do /usr/bin/sleep 0.05; done",
        ready.display(),
        release.display()
    );
    let _ = bridge::supervise_harness(
        &state_path,
        &event_log,
        &mut state,
        &shell_invocation(&worktree, &command),
        &snapshot,
        supervision_config(30_000),
    );
}

fn recover_test_predecessor<Authorized>(
    fixture: &GitFixture,
    state_dir: &Path,
    worktree: &bridge::IssueWorktree,
    authorize: Authorized,
) -> Result<bool, String>
where
    Authorized: FnMut(
        &bridge::PersistedInvocation,
        &mut dyn FnMut() -> Result<(), String>,
    ) -> Result<bool, String>,
{
    bridge::recover_released_interrupted_predecessor_transfer(
        state_dir,
        "owner/repo",
        &fixture.repo.canonicalize().expect("canonical repo"),
        42,
        &worktree.branch,
        authorize,
    )
}

#[test]
fn autonomous_executor_bridge_recovers_released_interrupted_predecessor_transfer() {
    let fixture = GitFixture::new("worktree-takeover-transfer");
    let base = resolve_base(&fixture.repo, &BTreeMap::new()).expect("resolve base");
    let scope = format!(
        "takeover_transfer_{}_{}",
        std::process::id(),
        TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    );
    let worktree = bridge::provision_issue_worktree_for_claim(
        &fixture.repo,
        &scope,
        42,
        &base,
        Some(("old-claim", "old-invocation")),
    )
    .expect("provision old generation");
    let initial_transfer: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(bridge::ownership_transfer_path(
            worktree.path.parent().expect("scope root"),
            42,
        ))
        .expect("initial ownership record"),
    )
    .expect("parse initial ownership");
    assert_eq!(initial_transfer["state"], "adopted");
    assert_eq!(initial_transfer["to_claim_id"], "old-claim");
    let wip = worktree.path.join("uncommitted-successor-input.txt");
    fs::write(&wip, "preserve me\n").expect("write old-generation WIP");
    let early = bridge::provision_issue_worktree_for_claim(
        &fixture.repo,
        &scope,
        42,
        &base,
        Some(("new-claim", "new-invocation")),
    )
    .expect_err("successor waits for predecessor transfer");
    assert!(early.contains("active in another generation"), "{early}");
    assert_eq!(
        fs::read_to_string(&wip).expect("early successor leaves WIP untouched"),
        "preserve me\n"
    );
    let mut state = supervision_state(&fixture);
    state.phase = BridgePhase::Interrupted;
    state.identity.worktree = worktree.path.clone();
    state.identity.branch = worktree.branch.clone();
    state.identity.claim_id = "old-claim".to_string();
    state.identity.invocation_id = "old-invocation".to_string();
    state.identity.runtime_environment_dir = None;
    state.identity.runtime_session_id = None;
    let state_dir = fixture.root.join("state");
    let generation = &bridge::sha256_hex(b"old-claim")[..16];
    let state_path = state_dir.join(format!("issue-42-{generation}.json"));
    bridge::write_invocation_atomic(&state_path, &state).expect("persist old invocation");

    let malformed_path = state_dir.join("issue-42-0000000000000000.json");
    fs::write(&malformed_path, [123]).expect("write malformed predecessor");
    fs::set_permissions(&malformed_path, fs::Permissions::from_mode(0o600))
        .expect("harden malformed predecessor");
    let malformed_error = recover_test_predecessor(&fixture, &state_dir, &worktree, |_, _| {
        panic!("malformed predecessor must fail before the release check")
    })
    .expect_err("malformed predecessor fails closed");
    assert!(
        malformed_error.contains("JSON") || malformed_error.contains("parse"),
        "{malformed_error}"
    );
    fs::remove_file(malformed_path).expect("remove malformed predecessor");

    let mut symlinked = state.clone();
    symlinked.identity.claim_id = "symlink-claim".to_string();
    symlinked.identity.invocation_id = "symlink-invocation".to_string();
    let symlink_target = fixture.root.join("symlink-predecessor.json");
    bridge::write_invocation_atomic(&symlink_target, &symlinked)
        .expect("persist symlink predecessor target");
    let symlink_generation = &bridge::sha256_hex(b"symlink-claim")[..16];
    let symlink_path = state_dir.join(format!("issue-42-{symlink_generation}.json"));
    symlink(&symlink_target, &symlink_path).expect("install predecessor state symlink");
    let symlink_error = recover_test_predecessor(&fixture, &state_dir, &worktree, |_, _| {
        panic!("symlink predecessor must fail before the release check")
    })
    .expect_err("symlink predecessor fails closed");
    assert!(symlink_error.contains("symlink"), "{symlink_error}");
    fs::remove_file(symlink_path).expect("remove predecessor state symlink");
    fs::remove_file(symlink_target).expect("remove predecessor symlink target");

    let transfer_path =
        bridge::ownership_transfer_path(worktree.path.parent().expect("scope root"), 42);
    bridge::write_private_atomic(
        &transfer_path,
        &[123],
        "malformed executor ownership transfer",
    )
    .expect("write malformed transfer");
    let malformed_transfer = recover_test_predecessor(&fixture, &state_dir, &worktree, |_, _| {
        panic!("malformed transfer must fail before the release check")
    })
    .expect_err("malformed transfer fails closed");
    assert!(malformed_transfer.contains("parse"), "{malformed_transfer}");
    bridge::write_private_atomic(
        &transfer_path,
        serde_json::to_string(&initial_transfer)
            .expect("serialize initial transfer")
            .as_bytes(),
        "restored executor ownership transfer",
    )
    .expect("restore transfer");

    let transfer_target = fixture.root.join("ownership-transfer-target.json");
    fs::copy(&transfer_path, &transfer_target).expect("move transfer target");
    fs::remove_file(&transfer_path).expect("remove transfer source");
    symlink(&transfer_target, &transfer_path).expect("install transfer symlink");
    let symlink_transfer = recover_test_predecessor(&fixture, &state_dir, &worktree, |_, _| {
        panic!("symlinked transfer must fail before the release check")
    })
    .expect_err("symlinked transfer fails closed");
    assert!(symlink_transfer.contains("symlink"), "{symlink_transfer}");
    fs::remove_file(&transfer_path).expect("remove transfer symlink");
    fs::copy(&transfer_target, &transfer_path).expect("restore transfer target");
    fs::remove_file(transfer_target).expect("remove transfer source");

    let mut mismatched_transfer = initial_transfer.clone();
    mismatched_transfer["to_claim_id"] = serde_json::json!("missing-claim");
    mismatched_transfer["to_invocation_id"] = serde_json::json!("missing-invocation");
    bridge::write_private_atomic(
        &transfer_path,
        serde_json::to_string(&mismatched_transfer)
            .expect("serialize mismatched transfer")
            .as_bytes(),
        "mismatched executor ownership transfer",
    )
    .expect("write mismatched transfer");
    let mismatch = recover_test_predecessor(&fixture, &state_dir, &worktree, |_, _| {
        panic!("mismatched transfer must fail before the release check")
    })
    .expect_err("mismatched transfer fails closed");
    assert!(mismatch.contains("does not name"), "{mismatch}");
    bridge::write_private_atomic(
        &transfer_path,
        serde_json::to_string(&initial_transfer)
            .expect("serialize initial transfer")
            .as_bytes(),
        "restored executor ownership transfer",
    )
    .expect("restore transfer");

    let mut historical = state.clone();
    historical.identity.claim_id = "older-claim".to_string();
    historical.identity.invocation_id = "older-invocation".to_string();
    historical.process =
        bridge::observe_process_identity(std::process::id(), "").expect("observe test process");
    let historical_generation = &bridge::sha256_hex(b"older-claim")[..16];
    let historical_path = state_dir.join(format!("issue-42-{historical_generation}.json"));
    bridge::write_invocation_atomic(&historical_path, &historical)
        .expect("persist historical predecessor");
    let live_historical = recover_test_predecessor(&fixture, &state_dir, &worktree, |_, _| {
        panic!("live historical predecessor must fail before the release check")
    })
    .expect_err("live historical predecessor blocks ownership transfer");
    assert!(
        live_historical.contains("process is still live"),
        "{live_historical}"
    );
    historical.process = None;
    bridge::write_invocation_atomic(&historical_path, &historical)
        .expect("retire historical predecessor process");

    let unreleased = recover_test_predecessor(&fixture, &state_dir, &worktree, |_, _| Ok(false))
        .expect_err("unreleased predecessor stays owned");
    assert!(unreleased.contains("claim is not released"), "{unreleased}");
    let unchanged: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(bridge::ownership_transfer_path(
            worktree.path.parent().expect("scope root"),
            42,
        ))
        .expect("unreleased transfer"),
    )
    .expect("parse unreleased transfer");
    assert_eq!(unchanged["state"], "adopted");
    assert_eq!(unchanged["to_claim_id"], "old-claim");

    state.process =
        bridge::observe_process_identity(std::process::id(), "").expect("observe test process");
    bridge::write_invocation_atomic(&state_path, &state).expect("persist live invocation");
    let live = recover_test_predecessor(&fixture, &state_dir, &worktree, |_, _| {
        panic!("live predecessor must be rejected before the release check")
    })
    .expect_err("live predecessor stays owned");
    assert!(live.contains("process is still live"), "{live}");

    let mut exited = Command::new("sh")
        .args(["-c", "sleep 0.1"])
        .spawn()
        .expect("spawn predecessor process");
    let dead = bridge::observe_process_identity(exited.id(), "")
        .expect("observe predecessor process")
        .expect("predecessor process identity");
    exited.wait().expect("wait for predecessor process");
    state.supervisor = Some(dead.clone());
    state.process = Some(dead);
    bridge::write_invocation_atomic(&state_path, &state)
        .expect("persist quiescent process anchors");
    let withheld = recover_test_predecessor(&fixture, &state_dir, &worktree, |_, _| Ok(true))
        .expect_err("authority cannot escape without owning the transfer");
    assert!(
        withheld.contains("did not transfer ownership"),
        "{withheld}"
    );
    let unchanged = fs::read_to_string(&wip).expect("withheld authority preserves WIP");
    assert_eq!(unchanged, "preserve me\n");
    let withheld_state = bridge::PersistedInvocation::from_json(
        &fs::read_to_string(&state_path).expect("read withheld predecessor"),
    )
    .expect("parse withheld predecessor");
    assert!(withheld_state.supervisor.is_some());
    assert!(withheld_state.process.is_some());
    let release_checks = std::cell::Cell::new(0usize);
    assert!(
        recover_test_predecessor(&fixture, &state_dir, &worktree, |candidate, transfer| {
            release_checks.set(release_checks.get() + 1);
            if candidate.identity.claim_id != "old-claim" {
                return Ok(false);
            }
            transfer()?;
            Ok(true)
        },)
        .expect("recover released interrupted predecessor")
    );
    assert_eq!(
        release_checks.get(),
        1,
        "only the transfer-bound predecessor reaches the authority gate"
    );
    let transferred_state = bridge::PersistedInvocation::from_json(
        &fs::read_to_string(&state_path).expect("read transferred predecessor"),
    )
    .expect("parse transferred predecessor");
    assert!(transferred_state.supervisor.is_none());
    assert!(transferred_state.process.is_none());
    assert_eq!(
        bridge::PersistedInvocation::from_json(
            &fs::read_to_string(&historical_path).expect("read historical predecessor")
        )
        .expect("parse historical predecessor")
        .phase,
        BridgePhase::Interrupted,
        "historical predecessor remains immutable"
    );
    assert_eq!(
        fs::read_to_string(&wip).expect("old WIP remains"),
        "preserve me\n"
    );

    let adopted = bridge::provision_issue_worktree_for_claim(
        &fixture.repo,
        &scope,
        42,
        &base,
        Some(("new-claim", "new-invocation")),
    )
    .expect("adopt worktree for successor generation");
    assert_eq!(adopted, worktree);
    assert_eq!(
        fs::read_to_string(&wip).expect("successor receives exact WIP"),
        "preserve me\n"
    );
    let transfer: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(bridge::ownership_transfer_path(
            worktree.path.parent().expect("scope root"),
            42,
        ))
        .expect("ownership transfer"),
    )
    .expect("parse ownership transfer");
    assert_eq!(transfer["state"], "adopted");
    assert_eq!(transfer["from_claim_id"], "old-claim");
    assert_eq!(transfer["to_claim_id"], "new-claim");

    fs::remove_file(historical_path).expect("remove historical predecessor");
    fs::remove_file(wip).expect("remove test WIP");
    git(
        &fixture.repo,
        &[
            "worktree",
            "remove",
            worktree.path.to_str().expect("worktree path"),
        ],
    );
    let _ = fs::remove_dir_all(worktree.path.parent().expect("scope root"));
}

#[test]
fn autonomous_executor_bridge_retry_fast_forwards_proven_empty_worktree_to_advanced_base() {
    let _environment = test_environment();
    let fixture = GitFixture::new("retry-empty-base-adoption");
    let original = resolve_base(&fixture.repo, &BTreeMap::new()).expect("resolve original base");
    let scope = format!(
        "retry_empty_base_{}_{}",
        std::process::id(),
        TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    );
    let worktree = provision_issue_worktree(&fixture.repo, &scope, 43, &original)
        .expect("provision original worktree");
    assert_eq!(
        git_stdout(&worktree.path, &["rev-parse", "HEAD"]),
        original.base_oid
    );

    fs::write(fixture.repo.join("base-advance.txt"), "advanced\n").expect("write base advance");
    git(&fixture.repo, &["add", "base-advance.txt"]);
    git(&fixture.repo, &["commit", "-m", "feat: advance base"]);
    git(&fixture.repo, &["push", "origin", "main"]);
    let advanced = resolve_base(&fixture.repo, &BTreeMap::new()).expect("resolve advanced base");

    bridge::EMPTY_RETRY_BASE_FAILPOINT.store(1, Ordering::SeqCst);
    let interrupted = provision_issue_worktree(&fixture.repo, &scope, 43, &advanced)
        .expect_err("interrupt after proven-empty fast-forward");
    assert!(interrupted.contains("injected crash"), "{interrupted}");
    assert_eq!(
        git_stdout(&worktree.path, &["rev-parse", "HEAD"]),
        advanced.base_oid
    );
    let adopted = provision_issue_worktree(&fixture.repo, &scope, 43, &advanced)
        .expect("resume proven-empty worktree adoption");
    assert_eq!(
        git_stdout(&adopted.path, &["rev-parse", "HEAD"]),
        advanced.base_oid,
        "a proven-empty retry must start exactly at the current base"
    );
    assert_eq!(
        git_stdout(&adopted.path, &["show", "-s", "--format=%P", "HEAD"]),
        original.base_oid,
        "the current base commit must not be wrapped in a synthetic merge"
    );

    git(
        &fixture.repo,
        &[
            "worktree",
            "remove",
            adopted.path.to_str().expect("worktree path"),
        ],
    );
    let _ = fs::remove_dir_all(adopted.path.parent().expect("scope root"));
}

#[test]
fn autonomous_executor_bridge_reclaims_prunable_zero_effect_branch_for_fresh_claim() {
    let (fixture, scope, worktree, advanced) =
        prunable_zero_effect_branch_fixture("prunable-zero-effect-branch", false);
    let unrelated = fixture.root.join("unrelated-prunable");
    git(
        &fixture.repo,
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "unrelated-prunable",
            unrelated.to_str().expect("unrelated worktree path"),
            &advanced.base_oid,
        ],
    );
    fs::remove_dir_all(&unrelated).expect("make unrelated registration prunable");

    let reclaimed = bridge::provision_issue_worktree_for_claim(
        &fixture.repo,
        &scope,
        42,
        &advanced,
        Some(("claim-fresh", "invocation-fresh")),
    )
    .expect("reclaim exact zero-effect branch");

    assert_eq!(reclaimed.path, worktree.path);
    assert_eq!(
        git_stdout(&reclaimed.path, &["rev-parse", "--verify", "HEAD^{commit}"]),
        advanced.base_oid
    );
    assert_eq!(
        git_stdout(&reclaimed.path, &["status", "--porcelain=v1"]),
        ""
    );
    let registry = git_stdout(&fixture.repo, &["worktree", "list", "--porcelain"]);
    assert!(
        registry.contains(&format!("worktree {}", unrelated.display()))
            && registry.lines().any(|line| line.starts_with("prunable ")),
        "reclaim must leave unrelated prunable registrations untouched: {registry}"
    );

    git(
        &fixture.repo,
        &[
            "worktree",
            "remove",
            reclaimed.path.to_str().expect("worktree path"),
        ],
    );
    let _ = fs::remove_dir_all(reclaimed.path.parent().expect("scope root"));
}

#[test]
fn autonomous_executor_bridge_reclaims_orphaned_zero_effect_branch() {
    let (fixture, scope, worktree, advanced) =
        prunable_zero_effect_branch_fixture("orphaned-zero-effect-branch", false);
    git(&fixture.repo, &["worktree", "prune", "--expire", "now"]);
    let registry = git_stdout(&fixture.repo, &["worktree", "list", "--porcelain"]);
    assert!(!registry.contains(&format!("worktree {}", worktree.path.display())));
    assert!(!registry.contains(&format!("branch refs/heads/{}", worktree.branch)));

    let reclaimed = bridge::provision_issue_worktree_for_claim(
        &fixture.repo,
        &scope,
        42,
        &advanced,
        Some(("claim-fresh", "invocation-fresh")),
    )
    .expect("reclaim proven orphaned zero-effect branch");

    assert_eq!(reclaimed.path, worktree.path);
    assert_eq!(
        git_stdout(&reclaimed.path, &["rev-parse", "--verify", "HEAD^{commit}"]),
        advanced.base_oid
    );
    assert_eq!(
        git_stdout(&reclaimed.path, &["status", "--porcelain=v1"]),
        ""
    );
    git(
        &fixture.repo,
        &[
            "worktree",
            "remove",
            reclaimed.path.to_str().expect("worktree path"),
        ],
    );
    let _ = fs::remove_dir_all(reclaimed.path.parent().expect("scope root"));
}

#[test]
fn autonomous_executor_bridge_rejects_competing_branch_registration_without_mutation() {
    let (fixture, scope, worktree, advanced) =
        prunable_zero_effect_branch_fixture("competing-branch-registration", false);
    let competing = fixture.root.join("competing-worktree");
    git(
        &fixture.repo,
        &[
            "worktree",
            "add",
            "--force",
            "--quiet",
            competing.to_str().expect("competing worktree path"),
            &worktree.branch,
        ],
    );
    let registry_before = git_stdout(&fixture.repo, &["worktree", "list", "--porcelain"]);
    let intent = worktree
        .path
        .parent()
        .expect("scope root")
        .join("issue-42.prunable-reclaim-intent.json");

    let error = bridge::provision_issue_worktree_for_claim(
        &fixture.repo,
        &scope,
        42,
        &advanced,
        Some(("claim-fresh", "invocation-fresh")),
    )
    .expect_err("competing branch registration must fail closed");

    assert!(error.contains("one exact prunable registration"), "{error}");
    assert_eq!(
        git_stdout(&fixture.repo, &["worktree", "list", "--porcelain"]),
        registry_before,
        "a rejected reclaim must not mutate the worktree registry"
    );
    assert!(
        !intent.exists(),
        "a rejected reclaim must not create durable intent"
    );
    git(
        &fixture.repo,
        &[
            "worktree",
            "remove",
            competing.to_str().expect("competing worktree path"),
        ],
    );
    git(&fixture.repo, &["worktree", "prune", "--expire", "now"]);
    let _ = fs::remove_dir_all(worktree.path.parent().expect("scope root"));
}
