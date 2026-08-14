// executor_bridge tests: runtime / fixture — 7 cases.
//
// Split out of tests.rs; see the note in that file.

use super::super::{
    provision_issue_worktree, recover_invocation, resolve_base, runtime_session_adapter,
    validate_trusted_ownership, write_invocation_atomic, BridgeIdentity, BridgePhase, HarnessKind,
    PersistedInvocation,
};
use super::support_base::{
    git, observe_spawned_identity, test_environment, GitFixture, TEST_SEQUENCE,
};
use super::support_invocation::{persisted_invocation, session_record_ids};
use crate::commands::autonomous::executor_bridge as bridge;
use autospec_core::runtime_env::{EnvironmentLifecycle, EnvironmentOwner};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::Ordering;

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_ownership_is_bound_to_the_executor_uid() {
    use std::os::unix::fs::MetadataExt;

    let fixture = GitFixture::new("trusted-owner");
    let actual_uid = fs::metadata(&fixture.repo).expect("repo metadata").uid();
    let error = validate_trusted_ownership(&[&fixture.repo], actual_uid.saturating_add(1))
        .expect_err("candidate-to-candidate ownership must not establish trust");

    assert!(error.contains("trusted executor owner"), "{error}");
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_rejects_foreign_symlink_and_detached_reuse() {
    use std::os::unix::fs::symlink;

    let fixture = GitFixture::new("unsafe-reuse");
    let base = resolve_base(&fixture.repo, &BTreeMap::new()).expect("resolve base");
    let foreign_scope = format!("foreign_{}", TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed));
    let scope_path = bridge::executor_worktree_root()
        .join(bridge::safe_scope(&foreign_scope).expect("safe scope"));
    fs::create_dir_all(&scope_path).expect("create scope");
    fs::create_dir_all(fixture.root.join("foreign")).expect("create foreign directory");
    symlink(fixture.root.join("foreign"), scope_path.join("issue-13"))
        .expect("create foreign symlink");
    assert!(
        provision_issue_worktree(&fixture.repo, &foreign_scope, 13, &base).is_err(),
        "symlinked foreign state must fail closed"
    );
    fs::remove_file(scope_path.join("issue-13")).expect("remove foreign symlink");
    fs::create_dir(scope_path.join("issue-13")).expect("create unregistered directory");
    assert!(
        provision_issue_worktree(&fixture.repo, &foreign_scope, 13, &base).is_err(),
        "unregistered foreign state must fail closed"
    );
    fs::remove_dir(scope_path.join("issue-13")).expect("remove foreign directory");
    fs::remove_dir_all(&scope_path).expect("remove foreign scope");

    let detached_scope = format!("detached_{}", TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed));
    let worktree = provision_issue_worktree(&fixture.repo, &detached_scope, 14, &base)
        .expect("provision worktree");
    git(&worktree.path, &["checkout", "--detach"]);
    assert!(
        provision_issue_worktree(&fixture.repo, &detached_scope, 14, &base).is_err(),
        "detached worktree must fail closed"
    );
    git(
        &fixture.repo,
        &[
            "worktree",
            "remove",
            "--force",
            worktree.path.to_str().unwrap(),
        ],
    );
    let _ = fs::remove_dir_all(worktree.path.parent().expect("detached scope"));
}

#[test]
fn autonomous_executor_bridge_isolates_equal_issue_numbers_and_runtime_sessions() {
    let _environment = test_environment();
    let first = GitFixture::new("same-issue-first");
    let second = GitFixture::new("same-issue-second");
    let base_one = resolve_base(&first.repo, &BTreeMap::new()).expect("first base");
    let base_two = resolve_base(&second.repo, &BTreeMap::new()).expect("second base");
    let one =
        provision_issue_worktree(&first.repo, "owner_first", 7, &base_one).expect("first worktree");
    let two = provision_issue_worktree(&second.repo, "owner_second", 7, &base_two)
        .expect("second worktree");
    assert_ne!(one.path, two.path);
    assert!(runtime_session_adapter(&one.path)
        .expect("no-manifest adapter")
        .is_none());
    assert!(runtime_session_adapter(&two.path)
        .expect("no-manifest adapter")
        .is_none());
    write_runtime_fixture(&one.path, ".autospec/runtime.yml", "one");
    write_runtime_fixture(&two.path, ".agent-runtime.yml", "two");
    let state_root = first.root.join("runtime-state");
    let previous_state_root = std::env::var_os("AGENT_ENV_STATE_ROOT");
    std::env::set_var("AGENT_ENV_STATE_ROOT", &state_root);
    let adapter_one = runtime_session_adapter(&one.path)
        .expect("valid runtime adapter")
        .expect("manifest-backed runtime");
    let adapter_two = runtime_session_adapter(&two.path)
        .expect("valid second runtime adapter")
        .expect("second manifest-backed runtime");
    assert_eq!(adapter_one.mode, "auto");
    assert_eq!(adapter_one.repo, one.path);
    assert_ne!(adapter_one.session_id, adapter_two.session_id);
    let live_records = session_record_ids(&state_root);
    assert!(live_records.contains(&adapter_one.session_id));
    assert!(live_records.contains(&adapter_two.session_id));
    assert_eq!(live_records.len(), 2);

    adapter_one
        .run(&["/usr/bin/true".to_string()])
        .expect("run first typed session");
    adapter_two
        .run(&["/usr/bin/true".to_string()])
        .expect("run second typed session");
    assert!(session_record_ids(&state_root).is_empty());
    match previous_state_root {
        Some(value) => std::env::set_var("AGENT_ENV_STATE_ROOT", value),
        None => std::env::remove_var("AGENT_ENV_STATE_ROOT"),
    }

    remove_runtime_fixture(&one.path, ".autospec/runtime.yml", "one");
    fs::remove_dir(one.path.join(".autospec")).expect("remove runtime config");
    remove_runtime_fixture(&two.path, ".agent-runtime.yml", "two");
    git(
        &first.repo,
        &["worktree", "remove", one.path.to_str().unwrap()],
    );
    git(
        &second.repo,
        &["worktree", "remove", two.path.to_str().unwrap()],
    );
    let _ = fs::remove_dir_all(one.path.parent().expect("first scope root"));
    let _ = fs::remove_dir_all(two.path.parent().expect("second scope root"));
}

#[test]
fn autonomous_executor_bridge_runtime_binding_mismatch_is_zero_mutation() {
    let _environment = test_environment();
    let fixture = GitFixture::new("runtime-drift");
    let base = resolve_base(&fixture.repo, &BTreeMap::new()).expect("resolve base");
    let scope = format!(
        "runtime_drift_{}_{}",
        std::process::id(),
        TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    );
    let worktree =
        provision_issue_worktree(&fixture.repo, &scope, 18, &base).expect("provision worktree");
    write_runtime_fixture(&worktree.path, ".autospec/runtime.yml", "before");
    let state_root = fixture.root.join("runtime-drift-state");
    let previous_state_root = std::env::var_os("AGENT_ENV_STATE_ROOT");
    std::env::set_var("AGENT_ENV_STATE_ROOT", &state_root);
    let adapter = runtime_session_adapter(&worktree.path)
        .expect("prepare runtime")
        .expect("manifest runtime");
    let environment_dir = adapter.environment_dir().to_path_buf();
    let session_id = adapter.session_id.clone();
    let record_path = environment_dir
        .join("sessions")
        .join(format!("{session_id}.json"));
    let before = fs::read(&record_path).expect("prior session record");
    let wrong_environment = environment_dir.with_file_name("foreign-environment");
    let error =
        bridge::reattach_runtime_session_adapter(&worktree.path, &wrong_environment, &session_id)
            .expect_err("environment drift must fail before prior-session mutation");

    assert!(error.contains("RUNTIME_OWNER_MISMATCH"), "{error}");
    assert_eq!(fs::read(&record_path).unwrap(), before);
    assert!(environment_dir
        .join("sessions")
        .join(format!("{session_id}.lock"))
        .is_file());
    adapter.abort().expect("cleanup original runtime");

    match previous_state_root {
        Some(value) => std::env::set_var("AGENT_ENV_STATE_ROOT", value),
        None => std::env::remove_var("AGENT_ENV_STATE_ROOT"),
    }
    let _ = fs::remove_dir_all(&state_root);
    remove_runtime_fixture(&worktree.path, ".autospec/runtime.yml", "before");
    fs::remove_dir(worktree.path.join(".autospec")).expect("remove runtime config");
    git(
        &fixture.repo,
        &["worktree", "remove", worktree.path.to_str().unwrap()],
    );
    let _ = fs::remove_dir_all(worktree.path.parent().expect("scope root"));
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_observes_live_persisted_child_before_runtime_recovery() {
    let args = vec!["-c".to_string(), "sleep 30".to_string()];
    let mut child = Command::new("/bin/sh")
        .args(&args)
        .spawn()
        .expect("spawn persisted child");
    let identity = observe_spawned_identity(child.id(), &args);
    let mut state = persisted_invocation();
    state.supervisor = Some(identity);

    let live = bridge::persisted_executor_is_live(&state).expect("observe persisted child");

    assert!(live);
    child.kill().expect("stop persisted child");
    child.wait().expect("reap persisted child");
}

#[test]
fn autonomous_executor_bridge_cleanup_failure_publication_holds_environment_lease() {
    let _environment = test_environment();
    let fixture = GitFixture::new("runtime-abort-failure");
    let base = resolve_base(&fixture.repo, &BTreeMap::new()).expect("resolve base");
    let scope = format!(
        "runtime_abort_{}_{}",
        std::process::id(),
        TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    );
    let worktree =
        provision_issue_worktree(&fixture.repo, &scope, 31, &base).expect("provision worktree");
    fs::create_dir_all(worktree.path.join(".autospec")).expect("create runtime config");
    fs::write(
        worktree.path.join("runtime-up-abort.sh"),
        "python3 -m http.server \"$AGENT_FRONTEND_PORT\" > runtime-abort.log 2>&1 &\nprintf '%s\\n' \"$!\" > runtime-abort.pid\n",
    )
    .expect("write runtime up");
    fs::write(
        worktree.path.join("runtime-down-abort.sh"),
        "pid=$(cat runtime-abort.pid)\nkill \"$pid\"\nrm -f runtime-abort.pid\nexit 42\n",
    )
    .expect("write failing runtime down");
    fs::write(
        worktree.path.join(".autospec/runtime.yml"),
        "version: 1\ndefault_mode: local\nmodes:\n  local:\n    command: sh runtime-up-abort.sh\n    down: sh runtime-down-abort.sh\n",
    )
    .expect("write runtime manifest");
    let state_root = fixture.root.join("runtime-abort-state");
    let previous_state_root = std::env::var_os("AGENT_ENV_STATE_ROOT");
    std::env::set_var("AGENT_ENV_STATE_ROOT", &state_root);

    let adapter = runtime_session_adapter(&worktree.path)
        .expect("prepare runtime adapter")
        .expect("manifest-backed runtime");
    let environment = fs::read_dir(&state_root)
        .expect("runtime state root")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.join("owner.json").is_file())
        .expect("authoritative runtime environment");
    let (publication_entered, allow_publication) =
        crate::commands::runtime::env::install_cleanup_failure_test_hook();
    let abort = std::thread::spawn(move || adapter.abort());
    publication_entered.wait();
    let transition_interleaved =
        crate::commands::runtime::env::try_transition_environment_lifecycle_for_test(
            &environment,
            EnvironmentLifecycle::Active,
        )
        .expect("try concurrent lifecycle transition");
    allow_publication.wait();
    let failure = abort
        .join()
        .expect("abort thread")
        .expect_err("explicit abort must report failing cleanup");

    assert_eq!(failure.exit_code, 42);
    assert!(
        !transition_interleaved,
        "a newer lifecycle transition acquired the lease before cleanup evidence was published"
    );
    let owner: EnvironmentOwner =
        autospec_core::runtime_env::read_json(&environment.join("owner.json"))
            .expect("recoverable authoritative owner");
    assert_eq!(owner.lifecycle, EnvironmentLifecycle::CleanupFailed);
    assert!(environment.join("plan.json").is_file());
    assert!(environment.join("inventory.json").is_file());
    assert!(session_record_ids(&state_root).is_empty());
    crate::commands::runtime::env::transition_environment_lifecycle_for_test(
        &environment,
        EnvironmentLifecycle::Active,
    )
    .expect("newer lifecycle transition after cleanup publication");
    let owner: EnvironmentOwner =
        autospec_core::runtime_env::read_json(&environment.join("owner.json"))
            .expect("newer authoritative owner");
    assert_eq!(owner.lifecycle, EnvironmentLifecycle::Active);

    match previous_state_root {
        Some(value) => std::env::set_var("AGENT_ENV_STATE_ROOT", value),
        None => std::env::remove_var("AGENT_ENV_STATE_ROOT"),
    }
    let _ = fs::remove_file(worktree.path.join("runtime-abort.log"));
    for path in [
        ".autospec/runtime.yml",
        "runtime-up-abort.sh",
        "runtime-down-abort.sh",
    ] {
        fs::remove_file(worktree.path.join(path)).expect("remove runtime fixture");
    }
    fs::remove_dir(worktree.path.join(".autospec")).expect("remove runtime config");
    git(
        &fixture.repo,
        &["worktree", "remove", worktree.path.to_str().unwrap()],
    );
    let _ = fs::remove_dir_all(worktree.path.parent().expect("scope root"));
}

fn remove_runtime_fixture(repo: &Path, manifest: &str, label: &str) {
    for path in [
        repo.join(manifest),
        repo.join(format!("runtime-up-{label}.sh")),
        repo.join(format!("runtime-down-{label}.sh")),
        repo.join(format!("runtime-{label}.log")),
    ] {
        if let Err(error) = fs::remove_file(&path) {
            assert_eq!(
                error.kind(),
                std::io::ErrorKind::NotFound,
                "remove {}: {error}",
                path.display()
            );
        }
    }
}

fn write_runtime_fixture(repo: &Path, manifest: &str, label: &str) {
    let manifest = repo.join(manifest);
    fs::create_dir_all(manifest.parent().expect("manifest parent"))
        .expect("create manifest parent");
    fs::write(
        repo.join(format!("runtime-up-{label}.sh")),
        format!(
            "python3 -m http.server \"$AGENT_FRONTEND_PORT\" > runtime-{label}.log 2>&1 &\nprintf '%s\\n' \"$!\" > runtime-{label}.pid\n"
        ),
    )
    .expect("write runtime up");
    fs::write(
        repo.join(format!("runtime-down-{label}.sh")),
        format!("pid=$(cat runtime-{label}.pid)\nkill \"$pid\"\nrm -f runtime-{label}.pid\n"),
    )
    .expect("write runtime down");
    fs::write(
        manifest,
        format!(
            "version: 1\ndefault_mode: local\nmodes:\n  local:\n    command: sh runtime-up-{label}.sh\n    down: sh runtime-down-{label}.sh\n"
        ),
    )
    .expect("write runtime manifest");
}

#[test]
fn autonomous_executor_bridge_persists_nonterminal_recovery_atomically() {
    let fixture = GitFixture::new("recovery");
    let base = resolve_base(&fixture.repo, &BTreeMap::new()).expect("resolve base");
    let scope = format!(
        "owner_recovery_{}",
        TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    );
    let worktree =
        provision_issue_worktree(&fixture.repo, &scope, 9, &base).expect("provision worktree");
    let state_path = fixture.root.join("state/invocation.json");
    let invocation = PersistedInvocation {
        schema: bridge::INVOCATION_SCHEMA,
        identity: BridgeIdentity {
            repository: "owner/recovery".into(),
            repository_path: fixture.repo.clone(),
            issue: 9,
            worker_id: "worker".into(),
            branch: worktree.branch.clone(),
            claim_id: "claim".into(),
            invocation_id: "invocation".into(),
            base_ref: base.base_ref.clone(),
            base_oid: base.base_oid.clone(),
            worktree: worktree.path.clone(),
            runtime_environment_dir: None,
            runtime_session_id: None,
        },
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
    let recovered = recover_invocation(&state_path, &invocation.identity)
        .expect("recover invocation")
        .expect("state exists");
    assert_eq!(recovered, invocation);
    assert!(fs::read_dir(state_path.parent().unwrap())
        .expect("state directory")
        .all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp")));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        assert_eq!(
            fs::metadata(state_path.parent().unwrap())
                .expect("state directory metadata")
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(&state_path)
                .expect("state file metadata")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }

    let mut foreign = invocation.identity.clone();
    foreign.claim_id = "takeover".into();
    assert!(recover_invocation(&state_path, &foreign).is_err());
    git(
        &fixture.repo,
        &["worktree", "remove", worktree.path.to_str().unwrap()],
    );
    let _ = fs::remove_dir_all(worktree.path.parent().expect("scope root"));
}
