// executor_bridge tests: sidecar / launch — 7 cases.
//
// Split out of tests.rs; see the note in that file.

use super::super::{
    BridgePhase, HarnessInvocation, MutationSnapshot, PersistedInvocation, SupervisionOutcome,
};
use super::support_base::{test_environment, DetachedSupervisorCleanup, GitFixture};
use super::support_invocation::{
    detach_harness_for_adoption, shell_invocation, supervision_config, supervision_state,
};
use crate::commands::autonomous::executor_bridge as bridge;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_sidecar_only_writer_is_cleaned_before_fresh_launch() {
    let _environment = test_environment();
    let fixture = GitFixture::new("sidecar-only-writer");
    let mut state = supervision_state(&fixture);
    let state_path = fixture.root.join("state/invocation.json");
    let event_log = fixture.root.join("log/executor.jsonl");
    let _ = detach_harness_for_adoption(
        &fixture,
        &state_path,
        &mut state,
        "while :; do printf 'old-writer\\n'; sleep 0.01; done",
    );
    let old_harness = state.process.clone().expect("old harness identity");
    state.process = None;
    bridge::write_invocation_atomic(&state_path, &state).expect("persist sidecar-only state");
    let fresh_marker = fixture.root.join("fresh-launch");
    let invocation = shell_invocation(
        &fixture.repo,
        &format!("printf fresh > '{}'", fresh_marker.display()),
    );
    let fresh = bridge::validate_invocation(
        &HarnessInvocation {
            program: invocation.program.canonicalize().expect("canonical shell"),
            supervised_executable: invocation
                .program
                .canonicalize()
                .expect("canonical shell"),
            args: invocation.args,
            current_dir: invocation
                .current_dir
                .canonicalize()
                .expect("canonical repo"),
            requires_mutation_snapshots: false,
        },
        &state.identity.worktree,
    )
    .expect("validate fresh harness");
    let snapshot =
        MutationSnapshot::capture(&fixture.repo, &state.identity.branch).expect("snapshot");

    let outcome = bridge::supervise_validated_harness(
        &state_path,
        &event_log,
        &mut state,
        &fresh,
        &snapshot,
        supervision_config(2_000),
    )
    .expect("fresh launch after sidecar cleanup");

    assert_eq!(outcome, SupervisionOutcome::Exited { exit_code: 0 });
    assert_eq!(
        fs::read_to_string(fresh_marker).expect("fresh marker"),
        "fresh"
    );
    assert!(
        bridge::observe_process_birth(old_harness.pid)
            .expect("old writer liveness")
            .is_none(),
        "sidecar-only writer survived cleanup"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_sidecar_cleanup_survives_pgid_transition() {
    let _environment = test_environment();
    // Break caught: sidecar-only cleanup requiring the mutable process group stored at launch
    // and therefore leaving the exact live instance behind after it moves groups.
    let fixture = GitFixture::new("sidecar-pgid-transition");
    let mut state = supervision_state(&fixture);
    let state_path = fixture.root.join("state/invocation.json");
    let event_log = fixture.root.join("log/executor.jsonl");
    let ready = fixture.root.join("ready");
    let release = fixture.root.join("release");
    let script = format!(
        "from pathlib import Path\nimport os,time\nPath(r'{}').write_text('ready')\n\
         gate=Path(r'{}')\nwhile not gate.exists(): time.sleep(0.001)\n\
         os.setpgid(0,0)\ntime.sleep(30)\n",
        ready.display(),
        release.display()
    );
    let args = vec!["-c".to_string(), script];
    let mut old = Command::new("/usr/bin/python3")
        .args(&args)
        .spawn()
        .expect("spawn sidecar PGID fixture");
    let deadline = Instant::now() + Duration::from_secs(2);
    while !ready.is_file() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(1));
    }
    let identity = bridge::observe_process_identity(old.id(), &bridge::argv_digest(&args))
        .expect("observe sidecar PGID fixture")
        .expect("live sidecar PGID fixture");
    state.supervisor = Some(identity.clone());
    state.process = None;
    bridge::write_invocation_atomic(&state_path, &state).expect("persist sidecar PGID state");
    fs::write(&release, b"release").expect("release PGID transition");
    let deadline = Instant::now() + Duration::from_secs(2);
    while bridge::observe_process_birth(old.id())
        .expect("observe transitioned sidecar")
        .is_some_and(|birth| birth.process_group == identity.process_group)
        && Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(1));
    }
    let fresh_marker = fixture.root.join("fresh");
    let invocation = shell_invocation(
        &fixture.repo,
        &format!("printf fresh > '{}'", fresh_marker.display()),
    );
    let fresh = bridge::validate_invocation(
        &HarnessInvocation {
            program: invocation.program.canonicalize().expect("canonical shell"),
            supervised_executable: invocation
                .program
                .canonicalize()
                .expect("canonical shell"),
            args: invocation.args,
            current_dir: invocation
                .current_dir
                .canonicalize()
                .expect("canonical repo"),
            requires_mutation_snapshots: false,
        },
        &state.identity.worktree,
    )
    .expect("validate fresh harness");
    let snapshot =
        MutationSnapshot::capture(&fixture.repo, &state.identity.branch).expect("snapshot");

    let outcome = bridge::supervise_validated_harness(
        &state_path,
        &event_log,
        &mut state,
        &fresh,
        &snapshot,
        supervision_config(2_000),
    )
    .expect("fresh launch after PGID sidecar cleanup");

    assert_eq!(outcome, SupervisionOutcome::Exited { exit_code: 0 });
    assert!(fresh_marker.is_file());
    assert!(bridge::observe_process_birth(old.id())
        .expect("observe cleaned PGID sidecar")
        .is_none());
    let _ = old.wait();
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_sidecar_cleanup_survives_unlinked_executable() {
    let _environment = test_environment();
    // Break caught: cleanup-only sidecar recovery requiring an executable path that no longer
    // exists even though PID, boot ID, and start identity still name the exact live instance.
    let fixture = GitFixture::new("sidecar-unlinked-executable");
    let mut state = supervision_state(&fixture);
    let state_path = fixture.root.join("state/invocation.json");
    let event_log = fixture.root.join("log/executor.jsonl");
    let executable = fixture.root.join("temporary-sleep");
    fs::copy("/usr/bin/sleep", &executable).expect("copy temporary executable");
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700))
        .expect("temporary executable mode");
    let args = vec!["30".to_string()];
    let mut old = Command::new(&executable)
        .args(&args)
        .spawn()
        .expect("spawn temporary executable");
    let identity = bridge::observe_process_identity(old.id(), &bridge::argv_digest(&args))
        .expect("observe temporary executable")
        .expect("live temporary executable");
    state.supervisor = Some(identity);
    state.process = None;
    bridge::write_invocation_atomic(&state_path, &state).expect("persist unlinked sidecar state");
    fs::remove_file(&executable).expect("unlink live sidecar executable");
    let fresh_marker = fixture.root.join("fresh");
    let invocation = shell_invocation(
        &fixture.repo,
        &format!("printf fresh > '{}'", fresh_marker.display()),
    );
    let fresh = bridge::validate_invocation(
        &HarnessInvocation {
            program: invocation.program.canonicalize().expect("canonical shell"),
            supervised_executable: invocation
                .program
                .canonicalize()
                .expect("canonical shell"),
            args: invocation.args,
            current_dir: invocation
                .current_dir
                .canonicalize()
                .expect("canonical repo"),
            requires_mutation_snapshots: false,
        },
        &state.identity.worktree,
    )
    .expect("validate fresh harness");
    let snapshot =
        MutationSnapshot::capture(&fixture.repo, &state.identity.branch).expect("snapshot");

    let outcome = bridge::supervise_validated_harness(
        &state_path,
        &event_log,
        &mut state,
        &fresh,
        &snapshot,
        supervision_config(2_000),
    )
    .expect("fresh launch after unlinked sidecar cleanup");

    assert_eq!(outcome, SupervisionOutcome::Exited { exit_code: 0 });
    assert!(fresh_marker.is_file());
    assert!(bridge::observe_process_birth(old.id())
        .expect("observe cleaned unlinked sidecar")
        .is_none());
    let _ = old.wait();
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_adopts_fast_exit_after_harness_identity_disappears() {
    let _environment = test_environment();
    let fixture = GitFixture::new("adopt-fast-exit-race");
    let mut state = supervision_state(&fixture);
    let state_path = fixture.root.join("state/invocation.json");
    let event_log = fixture.root.join("log/executor.jsonl");
    let validated = detach_harness_for_adoption(&fixture, &state_path, &mut state, "exit 0");
    let harness = state.process.clone().expect("persisted harness");
    let supervisor = state.supervisor.clone().expect("persisted supervisor");
    state.supervisor = None;
    bridge::write_invocation_atomic(&state_path, &state)
        .expect("persist process-only restart state");
    let sinks =
        bridge::output_sink_paths(&state_path, &state.identity.invocation_id).expect("sinks");
    for _ in 0..100 {
        if bridge::read_live_executor_exit_status(&sinks.exit_status).expect("exit sidecar")
            == Some(0)
            && bridge::observe_process_identity(harness.pid, &harness.argv_digest)
                .expect("observe exited harness")
                .is_none()
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        bridge::read_executor_exit_status(&sinks.exit_status).expect("durable exit"),
        Some(0)
    );
    assert!(
        bridge::observe_process_identity(harness.pid, &harness.argv_digest)
            .expect("final harness observation")
            .is_none(),
        "fixture did not reach the post-exit adoption race"
    );
    let snapshot =
        MutationSnapshot::capture(&fixture.repo, &state.identity.branch).expect("snapshot");

    let result = bridge::supervise_validated_harness(
        &state_path,
        &event_log,
        &mut state,
        &validated,
        &snapshot,
        supervision_config(500),
    );
    if bridge::observe_process_identity(supervisor.pid, &supervisor.argv_digest)
        .expect("observe legacy supervisor after recovery")
        .is_some()
    {
        let mut owned =
            bridge::OwnedProcessSet::adopt(&supervisor).expect("capture leaked supervisor");
        owned.terminate().expect("clean RED fixture supervisor");
    }
    let outcome = result.expect("process-only restart recovers supervisor from durable journal");

    assert_eq!(outcome, SupervisionOutcome::Exited { exit_code: 0 });
    assert!(
        bridge::observe_process_birth(supervisor.pid)
            .expect("final supervisor observation")
            .is_none(),
        "process-only fast exit left the stable supervisor alive"
    );
    assert!(state.supervisor.is_none());
    assert!(state.process.is_none());
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_pending_restart_reconciles_invocation_sidecar_before_launch() {
    let _environment = test_environment();
    let fixture = GitFixture::new("pending-sidecar-restart");
    let mut state = supervision_state(&fixture);
    let state_path = fixture.root.join("state/invocation.json");
    let event_log = fixture.root.join("log/executor.jsonl");
    let launches = fixture.root.join("launches");
    let validated = detach_harness_for_adoption(
        &fixture,
        &state_path,
        &mut state,
        &format!(
            "printf 'launch\\n' >> '{}'; while :; do sleep 1; done",
            launches.display()
        ),
    );
    let supervisor = state.supervisor.clone().expect("supervisor identity");
    let _cleanup = DetachedSupervisorCleanup(supervisor.clone());
    state.phase = BridgePhase::Pending;
    state.supervisor = None;
    state.process = None;
    bridge::write_invocation_atomic(&state_path, &state)
        .expect("persist crash-window pending state");
    let snapshot =
        MutationSnapshot::capture(&fixture.repo, &state.identity.branch).expect("snapshot");

    let outcome = bridge::supervise_validated_harness(
        &state_path,
        &event_log,
        &mut state,
        &validated,
        &snapshot,
        supervision_config(100),
    )
    .expect("restart reconciles accepted supervisor sidecar");

    assert_eq!(outcome, SupervisionOutcome::Stalled);
    assert_eq!(
        fs::read_to_string(&launches).expect("single harness launch"),
        "launch\n",
        "pending restart launched a duplicate process tree"
    );
    assert!(
        bridge::observe_process_identity(supervisor.pid, &supervisor.argv_digest)
            .expect("observe reconciled supervisor")
            .is_none(),
        "reconciled supervisor survived exact cleanup"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_true_pre_sidecar_legacy_state_stays_quarantined() {
    let _environment = test_environment();
    let fixture = GitFixture::new("true-pre-sidecar-quarantine");
    let mut state = supervision_state(&fixture);
    let state_path = fixture.root.join("state/invocation.json");
    let event_log = fixture.root.join("log/executor.jsonl");
    let launches = fixture.root.join("launches");
    let validated = detach_harness_for_adoption(
        &fixture,
        &state_path,
        &mut state,
        &format!("printf 'launch\\n' >> '{}'; exit 0", launches.display()),
    );
    let supervisor = state.supervisor.clone().expect("supervisor identity");
    let harness = state.process.clone().expect("harness identity");
    let sinks =
        bridge::output_sink_paths(&state_path, &state.identity.invocation_id).expect("sinks");
    fs::remove_file(&sinks.supervisor_identity).expect("remove post-G sidecar");
    state.phase = BridgePhase::Implementing;
    state.supervisor = None;
    bridge::write_invocation_atomic(&state_path, &state)
        .expect("persist true pre-sidecar process-only state");
    for _ in 0..100 {
        if bridge::observe_process_identity(harness.pid, &harness.argv_digest)
            .expect("observe exited legacy harness")
            .is_none()
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        bridge::observe_process_identity(harness.pid, &harness.argv_digest)
            .expect("final legacy harness observation")
            .is_none(),
        "fixture harness did not exit"
    );
    let snapshot =
        MutationSnapshot::capture(&fixture.repo, &state.identity.branch).expect("snapshot");

    for attempt in 0..2 {
        let error = bridge::supervise_validated_harness(
            &state_path,
            &event_log,
            &mut state,
            &validated,
            &snapshot,
            supervision_config(100),
        )
        .expect_err("unrecoverable pre-sidecar ownership remains quarantined");
        assert!(error.contains("quarantin"), "attempt {attempt}: {error}");
        let durable = PersistedInvocation::from_json(
            &fs::read_to_string(&state_path).expect("durable legacy quarantine"),
        )
        .expect("strict legacy quarantine");
        assert_eq!(durable.phase, BridgePhase::Interrupted);
        assert_eq!(durable.process.as_ref(), Some(&harness));
        state = durable;
    }
    assert_eq!(
        fs::read_to_string(&launches).expect("original launch evidence"),
        "launch\n",
        "legacy quarantine permitted a duplicate launch"
    );

    if bridge::observe_process_identity(supervisor.pid, &supervisor.argv_digest)
        .expect("observe fixture supervisor")
        .is_some()
    {
        let mut owned =
            bridge::OwnedProcessSet::adopt(&supervisor).expect("capture fixture supervisor");
        owned.terminate().expect("clean fixture supervisor");
    }
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_partial_adoption_error_cleans_captured_supervisor_tree() {
    let _environment = test_environment();
    let fixture = GitFixture::new("adopt-partial-cleanup");
    let mut state = supervision_state(&fixture);
    let state_path = fixture.root.join("state/invocation.json");
    let event_log = fixture.root.join("log/executor.jsonl");
    let descendant_pid = fixture.root.join("descendant.pid");
    let script = format!(
        "sleep 30 & printf '%s\\n' \"$!\" > '{}'; while :; do sleep 1; done",
        descendant_pid.display()
    );
    let validated = detach_harness_for_adoption(&fixture, &state_path, &mut state, &script);
    for _ in 0..100 {
        if descendant_pid.exists() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let supervisor = state.supervisor.as_ref().expect("supervisor").pid;
    let harness = state.process.as_ref().expect("harness").pid;
    state
        .process
        .as_mut()
        .expect("persisted harness")
        .argv_digest = "f".repeat(64);
    bridge::write_invocation_atomic(&state_path, &state).expect("persist mismatched identity");
    let snapshot =
        MutationSnapshot::capture(&fixture.repo, &state.identity.branch).expect("snapshot");

    let error = bridge::supervise_validated_harness(
        &state_path,
        &event_log,
        &mut state,
        &validated,
        &snapshot,
        supervision_config(500),
    )
    .expect_err("partial adoption must fail full identity validation");

    assert!(error.contains("full identity"), "{error}");
    let descendant = fs::read_to_string(descendant_pid)
        .expect("descendant identity")
        .trim()
        .to_string();
    for pid in [supervisor.to_string(), harness.to_string(), descendant] {
        assert!(
            !Path::new(&format!("/proc/{pid}")).exists(),
            "partial adoption leaked owned PID {pid}"
        );
    }
    let durable = PersistedInvocation::from_json(
        &fs::read_to_string(&state_path).expect("durable partial-adoption failure"),
    )
    .expect("strict partial-adoption failure");
    assert_eq!(durable.phase, BridgePhase::Interrupted);
    assert!(durable.supervisor.is_none());
    assert!(durable.process.is_none());
    let events = fs::read_to_string(event_log).expect("partial-adoption event");
    assert_eq!(
        events
            .matches("\"event\":\"child_supervision_error\"")
            .count(),
        1
    );
    assert!(events.contains("\"adopted\":true"), "{events}");
}
