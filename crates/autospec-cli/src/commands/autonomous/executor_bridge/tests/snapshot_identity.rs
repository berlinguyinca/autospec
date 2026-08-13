// executor_bridge tests: snapshot / identity — 11 cases.
//
// Split out of tests.rs; see the note in that file.

use crate::commands::autonomous::executor_bridge as bridge;
use super::super::{
    supervise_harness, BridgePhase, MutationSnapshot, PersistedInvocation, SupervisionOutcome,
};
use super::support_base::{DetachedForkedCleanup, GitFixture, git, git_stdout, observe_spawned_identity, test_environment};
use super::support_invocation::{shell_invocation, supervision_config, supervision_state};
#[cfg(target_os = "linux")]
use nix::sys::signal::Signal;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::Path;
use std::process::Command;
use std::time::Duration;

#[test]
fn autonomous_executor_bridge_stall_cleans_exact_process_group_nonterminally() {
    let fixture = GitFixture::new("supervise-stall");
    let mut state = supervision_state(&fixture);
    let snapshot =
        MutationSnapshot::capture(&fixture.repo, &state.identity.branch).expect("snapshot");
    let state_path = fixture.root.join("state/invocation.json");
    let event_log = fixture.root.join("log/executor.jsonl");
    let descendant_pid = fixture.root.join("descendant.pid");
    let script = format!(
        "sleep 30 & child=$!; printf '%s\\n' \"$child\" > '{}'; wait \"$child\"",
        descendant_pid.display()
    );

    let outcome = supervise_harness(
        &state_path,
        &event_log,
        &mut state,
        &shell_invocation(&fixture.repo, &script),
        &snapshot,
        supervision_config(100),
    )
    .expect("terminate stalled child");

    assert_eq!(outcome, SupervisionOutcome::Stalled);
    assert_eq!(state.phase, BridgePhase::Interrupted);
    assert!(state.process.is_none());
    assert!(fs::read_to_string(event_log)
        .expect("stall event")
        .contains("\"event\":\"child_stalled\""));
    let descendant = fs::read_to_string(descendant_pid)
        .expect("descendant identity")
        .trim()
        .to_string();
    assert!(
        !Path::new(&format!("/proc/{descendant}")).exists(),
        "stalled descendant survived exact process-group cleanup"
    );
}

#[test]
fn autonomous_executor_bridge_does_not_duplicate_or_signal_mismatched_identity() {
    let fixture = GitFixture::new("supervise-identity");
    let mut running = Command::new("/bin/sh");
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        running.process_group(0);
    }
    let mut running = running
        .args(["-c", "while :; do sleep 1; done"])
        .spawn()
        .expect("spawn identity fixture");
    let args = vec!["-c".to_string(), "while :; do sleep 1; done".to_string()];
    let mut running_cleanup =
        DetachedForkedCleanup::new(running.id()).expect("arm running fixture cleanup");
    let expected = observe_spawned_identity(running.id(), &args);
    running_cleanup.confirm_identity(expected.clone());
    let mut state = supervision_state(&fixture);
    state.process = Some(expected.clone());
    state.phase = BridgePhase::Implementing;
    let snapshot =
        MutationSnapshot::capture(&fixture.repo, &state.identity.branch).expect("snapshot");
    let launches = fixture.root.join("unexpected-launch");
    let invocation = shell_invocation(
        &fixture.repo,
        &format!("printf launched > '{}'", launches.display()),
    );

    let error = supervise_harness(
        &fixture.root.join("state/invocation.json"),
        &fixture.root.join("log/executor.jsonl"),
        &mut state,
        &invocation,
        &snapshot,
        supervision_config(500),
    )
    .expect_err("unproven process-only parent must be quarantined");
    assert!(error.contains("legacy executor ownership"), "{error}");
    assert!(!launches.exists(), "a duplicate harness was launched");
    assert!(running
        .try_wait()
        .expect("quarantined fixture live")
        .is_none());
    bridge::terminate_exact_process_group(&expected, &mut running)
        .expect("clean quarantined identity fixture");

    let mut replacement = Command::new("/bin/sh");
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        replacement.process_group(0);
    }
    let mut replacement = replacement
        .args(["-c", "while :; do sleep 1; done"])
        .spawn()
        .expect("spawn mismatched identity fixture");
    let mut replacement_cleanup =
        DetachedForkedCleanup::new(replacement.id()).expect("arm replacement fixture cleanup");
    let replacement_identity = observe_spawned_identity(replacement.id(), &args);
    replacement_cleanup.confirm_identity(replacement_identity.clone());
    state.process = Some(replacement_identity.clone());
    state.phase = BridgePhase::Implementing;
    state
        .process
        .as_mut()
        .expect("persisted process")
        .start_identity
        .push('9');
    let error = supervise_harness(
        &fixture.root.join("state/invocation.json"),
        &fixture.root.join("log/executor.jsonl"),
        &mut state,
        &invocation,
        &snapshot,
        supervision_config(500),
    )
    .expect_err("PID reuse must fail closed");
    assert!(
        error.contains("quarantined") || error.contains("full identity"),
        "unexpected error: {error}"
    );
    assert!(replacement.try_wait().expect("observe fixture").is_none());
    bridge::terminate_exact_process_group(&replacement_identity, &mut replacement)
        .expect("clean identity fixture");
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_pidfd_signal_ignores_substituted_numeric_identity() {
    // Break caught: cleanup re-reading a mutable numeric PID at signal time rather than using
    // the immutable kernel handle opened for the originally captured process.
    let mut captured_child = Command::new("/bin/sh")
        .args(["-c", "sleep 30"])
        .spawn()
        .expect("spawn captured child");
    let mut replacement = Command::new("/bin/sh")
        .args(["-c", "sleep 30"])
        .spawn()
        .expect("spawn replacement child");
    let mut captured =
        bridge::OwnedProcess::capture_forked_child(captured_child.id()).expect("capture pidfd");
    let replacement_birth = bridge::observe_process_birth(replacement.id())
        .expect("observe replacement")
        .expect("live replacement");

    captured.birth = replacement_birth;
    captured
        .signal(Signal::SIGTERM)
        .expect("signal immutable pidfd");
    for _ in 0..20 {
        if captured_child
            .try_wait()
            .expect("reap captured child")
            .is_some()
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    assert!(
        captured_child.try_wait().expect("captured exit").is_some(),
        "original pidfd target survived"
    );
    assert!(
        replacement
            .try_wait()
            .expect("replacement liveness")
            .is_none(),
        "substituted numeric identity was signaled"
    );
    let replacement_process =
        bridge::OwnedProcess::capture_forked_child(replacement.id()).expect("replacement pidfd");
    replacement_process
        .signal(Signal::SIGKILL)
        .expect("clean replacement");
    let _ = replacement.wait().expect("reap replacement");
}

#[test]
fn autonomous_executor_bridge_fails_closed_on_protected_ref_mutation() {
    let fixture = GitFixture::new("supervise-mutation");
    let mut state = supervision_state(&fixture);
    let snapshot =
        MutationSnapshot::capture(&fixture.repo, &state.identity.branch).expect("snapshot");
    let head = git_stdout(&fixture.repo, &["rev-parse", "HEAD"]);
    let script = format!("sleep 0.1; git update-ref -d refs/heads/main {head}");

    let error = supervise_harness(
        &fixture.root.join("state/invocation.json"),
        &fixture.root.join("log/executor.jsonl"),
        &mut state,
        &shell_invocation(&fixture.repo, &script),
        &snapshot,
        supervision_config(2_000),
    )
    .expect_err("protected mutation must fail closed");

    assert!(
        error.contains("mutated the primary checkout"),
        "unexpected error: {error}"
    );
}

#[test]
fn autonomous_executor_bridge_snapshot_seals_dirty_tracked_contents() {
    let fixture = GitFixture::new("snapshot-dirty-tracked");
    let tracked = fixture.repo.join("README.md");
    fs::write(&tracked, "operator edit before launch\n").expect("dirty tracked file");
    let snapshot =
        MutationSnapshot::capture(&fixture.repo, "feat/autonomous-issue-42").expect("snapshot");

    fs::write(&tracked, "executor replacement\n").expect("replace dirty tracked file");

    assert!(
        snapshot
            .verify(&fixture.repo, "feat/autonomous-issue-42")
            .is_err(),
        "same porcelain status must not hide dirty tracked content replacement"
    );
}

#[test]
fn autonomous_executor_bridge_snapshot_seals_untracked_contents() {
    let fixture = GitFixture::new("snapshot-untracked");
    let untracked = fixture.repo.join("operator-notes.txt");
    fs::write(&untracked, "operator notes before launch\n").expect("untracked file");
    let snapshot =
        MutationSnapshot::capture(&fixture.repo, "feat/autonomous-issue-42").expect("snapshot");

    fs::write(&untracked, "executor replacement\n").expect("replace untracked file");

    assert!(
        snapshot
            .verify(&fixture.repo, "feat/autonomous-issue-42")
            .is_err(),
        "same porcelain status must not hide untracked content replacement"
    );
}

#[test]
fn autonomous_executor_bridge_snapshot_fails_closed_on_dirty_submodule() {
    // Break caught: a dirty gitlink retaining identical porcelain while its nested HEAD or
    // working tree changes after the primary-checkout snapshot.
    let fixture = GitFixture::new("snapshot-dirty-submodule");
    let submodule = fixture.root.join("submodule-source");
    git(
        &fixture.root,
        &["init", submodule.to_str().expect("submodule path")],
    );
    git(
        &submodule,
        &["config", "user.email", "autospec@example.invalid"],
    );
    git(&submodule, &["config", "user.name", "Autospec Test"]);
    fs::write(submodule.join("nested.txt"), "captured\n").expect("submodule contents");
    git(&submodule, &["add", "nested.txt"]);
    git(&submodule, &["commit", "-m", "nested fixture"]);
    git(
        &fixture.repo,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            submodule.to_str().expect("submodule source"),
            "vendor/nested",
        ],
    );
    git(&fixture.repo, &["commit", "-am", "add nested fixture"]);
    fs::write(
        fixture.repo.join("vendor/nested/nested.txt"),
        "dirty nested contents\n",
    )
    .expect("dirty submodule");

    let error = MutationSnapshot::capture(&fixture.repo, "feat/autonomous-issue-42")
        .expect_err("dirty submodule directories must fail closed");

    assert!(
        error.contains("dirty directory"),
        "unexpected error: {error}"
    );
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_snapshot_seals_modes_symlink_targets_and_types() {
    let fixture = GitFixture::new("snapshot-node-identity");
    let tracked = fixture.repo.join("README.md");
    fs::write(&tracked, "operator edit before launch\n").expect("dirty tracked file");
    fs::set_permissions(&tracked, fs::Permissions::from_mode(0o744))
        .expect("set dirty tracked mode");
    let link = fixture.repo.join("operator-link");
    symlink("operator-target-a", &link).expect("create untracked symlink");
    let snapshot =
        MutationSnapshot::capture(&fixture.repo, "feat/autonomous-issue-42").expect("snapshot");

    fs::set_permissions(&tracked, fs::Permissions::from_mode(0o644))
        .expect("change dirty tracked mode");
    assert!(
        snapshot
            .verify(&fixture.repo, "feat/autonomous-issue-42")
            .is_err(),
        "mode-only changes must invalidate the snapshot"
    );

    fs::set_permissions(&tracked, fs::Permissions::from_mode(0o744))
        .expect("restore captured dirty tracked mode");
    let symlink_snapshot =
        MutationSnapshot::capture(&fixture.repo, "feat/autonomous-issue-42").expect("snapshot");
    fs::remove_file(&link).expect("remove untracked symlink");
    symlink("operator-target-b", &link).expect("replace untracked symlink target");
    assert!(
        symlink_snapshot
            .verify(&fixture.repo, "feat/autonomous-issue-42")
            .is_err(),
        "symlink-target-only changes must invalidate the snapshot"
    );

    let type_snapshot =
        MutationSnapshot::capture(&fixture.repo, "feat/autonomous-issue-42").expect("snapshot");
    fs::remove_file(&link).expect("remove symlink before type change");
    fs::create_dir(&link).expect("replace symlink with directory");
    assert!(
        type_snapshot
            .verify(&fixture.repo, "feat/autonomous-issue-42")
            .is_err(),
        "node type changes must invalidate the snapshot"
    );
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_snapshot_seals_deletion_and_same_status_type_change() {
    let fixture = GitFixture::new("snapshot-deletion-type");
    let node = fixture.repo.join("operator-node");
    fs::write(&node, "operator bytes\n").expect("create untracked file");
    let snapshot =
        MutationSnapshot::capture(&fixture.repo, "feat/autonomous-issue-42").expect("snapshot");

    fs::remove_file(&node).expect("remove untracked file");
    symlink("operator-target", &node).expect("replace file with symlink");
    assert!(
        snapshot
            .verify(&fixture.repo, "feat/autonomous-issue-42")
            .is_err(),
        "same untracked porcelain entry must not hide a node type change"
    );

    fs::remove_file(&node).expect("delete dirty path");
    assert!(
        snapshot
            .verify(&fixture.repo, "feat/autonomous-issue-42")
            .is_err(),
        "deleting a pre-existing dirty path must invalidate the snapshot"
    );
}

#[test]
fn autonomous_executor_bridge_mutation_failure_persists_only_interrupted() {
    let fixture = GitFixture::new("snapshot-persist-interrupted");
    let tracked = fixture.repo.join("README.md");
    fs::write(&tracked, "operator edit before launch\n").expect("dirty tracked file");
    let mut state = supervision_state(&fixture);
    let snapshot =
        MutationSnapshot::capture(&fixture.repo, &state.identity.branch).expect("snapshot");
    let state_path = fixture.root.join("state/invocation.json");
    let event_log = fixture.root.join("log/executor.jsonl");

    let error = supervise_harness(
        &state_path,
        &event_log,
        &mut state,
        &shell_invocation(
            &fixture.repo,
            "printf 'executor replacement\\n' > README.md",
        ),
        &snapshot,
        supervision_config(2_000),
    )
    .expect_err("dirty primary checkout mutation must fail closed");

    assert!(error.contains("mutated the primary checkout"), "{error}");
    let persisted = PersistedInvocation::from_json(
        &fs::read_to_string(&state_path).expect("persisted invocation"),
    )
    .expect("strict invocation");
    assert_eq!(persisted.phase, BridgePhase::Interrupted);
    assert!(persisted.process.is_none());
    assert!(
        !fs::read_to_string(&state_path)
            .expect("state text")
            .contains("\"phase\":\"implementation_complete\""),
        "unverified completion must never be the durable recovery boundary"
    );
}

#[test]
fn autonomous_executor_bridge_preverification_crash_never_publishes_complete() {
    let environment = test_environment();
    let fixture = GitFixture::new("snapshot-preverify-crash");
    let mut state = supervision_state(&fixture);
    let snapshot =
        MutationSnapshot::capture(&fixture.repo, &state.identity.branch).expect("snapshot");
    let state_path = fixture.root.join("state/invocation.json");
    let event_log = fixture.root.join("log/executor.jsonl");
    let invocation = shell_invocation(&fixture.repo, "exit 0");

    environment.launch(bridge::LaunchFailpoint::BeforeSnapshotVerification);
    let error = supervise_harness(
        &state_path,
        &event_log,
        &mut state,
        &invocation,
        &snapshot,
        supervision_config(2_000),
    )
    .expect_err("injected crash before snapshot verification");
    environment.launch(bridge::LaunchFailpoint::None);

    assert!(error.contains("pre-verify"), "{error}");
    let recovered = PersistedInvocation::from_json(
        &fs::read_to_string(&state_path).expect("persisted invocation"),
    )
    .expect("strict invocation");
    assert_eq!(recovered.phase, BridgePhase::Interrupted);
    assert!(recovered.process.is_none());
    assert!(recovered.supervisor.is_none());
    assert!(fs::read_to_string(event_log)
        .expect("structured preverification failure")
        .contains("\"event\":\"child_supervision_error\""));
}
