// executor_bridge tests: harness / supervisor — 10 cases.
//
// Split out of tests.rs; see the note in that file.

use super::super::{BridgePhase, MutationSnapshot, SupervisionOutcome};
use super::support_base::{test_environment, DetachedForkedCleanup, GitFixture};
use super::support_invocation::{
    detach_harness_for_adoption, supervision_config, supervision_state, NonDescendantDirectFixture,
};
use crate::commands::autonomous::executor_bridge as bridge;
#[cfg(target_os = "linux")]
use nix::sys::signal::Signal;
use std::fs;
#[cfg(target_os = "linux")]
use std::os::unix::fs::PermissionsExt;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::path::Path;
#[cfg(target_os = "linux")]
use std::process::{Command, Stdio};
#[cfg(target_os = "linux")]
use std::time::{Duration, Instant};

#[cfg(target_os = "macos")]
#[test]
fn darwin_owned_group_spawns_in_an_exact_dedicated_process_group() {
    use super::super::{darwin_supervisor::DarwinOwnedGroup, OutputSinkPaths, ValidatedInvocation};
    use std::ffi::OsString;

    let root = std::env::current_dir()
        .expect("current Darwin fixture directory")
        .join("target/executor-bridge-tests")
        .join(format!(
            "autospec-darwin-group-{}-{}",
            std::process::id(),
            bridge::unix_now().expect("clock")
        ));
    fs::create_dir_all(&root).expect("create Darwin supervisor fixture");
    let sinks = OutputSinkPaths {
        stdout: root.join("stdout"),
        stderr: root.join("stderr"),
        stdout_writer_cursor: root.join("stdout.writer"),
        stderr_writer_cursor: root.join("stderr.writer"),
        stdout_reader_cursor: root.join("stdout.reader"),
        stderr_reader_cursor: root.join("stderr.reader"),
        exit_status: root.join("exit"),
        supervisor_identity: root.join("supervisor.json"),
    };
    let invocation = ValidatedInvocation {
        program: Path::new("/bin/sh").to_path_buf(),
        argv_zero: None::<OsString>,
        args: vec![
            "-c".into(),
            "trap '' TERM; while :; do sleep 1; done".into(),
        ],
        current_dir: root.clone(),
        environment_overrides: Vec::new(),
    };

    let mut group = DarwinOwnedGroup::spawn(&invocation, &sinks).expect("spawn owned Darwin group");
    assert_eq!(group.identity().pid, group.identity().process_group);
    assert_eq!(
        bridge::observe_process_birth(group.identity().pid)
            .expect("observe leader")
            .expect("leader remains live")
            .process_group,
        group.identity().process_group
    );
    group.release().expect("release persisted Darwin group");
    group.terminate().expect("terminate exact Darwin group");
    let _ = fs::remove_dir_all(root);
}

#[cfg(target_os = "macos")]
#[test]
fn darwin_cleanup_uncertainty_persists_interrupted_exact_identity_and_event() {
    use super::super::tests::support_invocation::persisted_invocation;

    let root = std::env::current_dir()
        .expect("current Darwin fixture directory")
        .join("target/executor-bridge-tests")
        .join(format!(
            "autospec-darwin-recovery-required-{}",
            std::process::id()
        ));
    let _ = fs::remove_dir_all(&root);
    bridge::ensure_private_directory(&root).expect("private recovery fixture");
    let state_path = root.join("invocation.json");
    let event_log = root.join("events.jsonl");
    let mut state = persisted_invocation();
    let expected = state.process.clone().expect("exact process identity");

    bridge::record_darwin_recovery_required(
        &state_path,
        &event_log,
        &mut state,
        "child_stall_cleanup_uncertain",
        "permission denied",
    )
    .expect("persist recovery-required state");

    let durable = bridge::PersistedInvocation::from_json(
        &fs::read_to_string(&state_path).expect("durable invocation"),
    )
    .expect("parse durable invocation");
    assert_eq!(durable.phase, bridge::BridgePhase::Interrupted);
    assert_eq!(
        durable.process.as_ref().expect("retained identity").pid,
        expected.pid
    );
    let event = fs::read_to_string(&event_log).expect("recovery event");
    assert!(event.contains("\"recovery_required\":true"), "{event}");
    assert!(
        event.contains("\"exact_process_identity_retained\":true"),
        "{event}"
    );
    let _ = fs::remove_dir_all(root);
}

#[cfg(target_os = "macos")]
#[test]
fn darwin_actual_stall_cleanup_uncertainty_retains_recovery_evidence() {
    use super::super::darwin_supervisor::{
        force_next_termination_uncertainty_for_test, group_is_empty,
    };
    use nix::errno::Errno;
    use nix::sys::signal::{killpg, Signal};
    use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
    use nix::unistd::Pid;
    use std::time::{Duration, Instant};

    let (_fixture, mut state, snapshot, _closeout) =
        implementation_proof_fixture("darwin-actual-stall-cleanup-uncertain");
    state.phase = BridgePhase::Implementing;
    state.process = None;
    let state_path = state.identity.worktree.join(".autospec/invocation.json");
    let event_log = state.identity.worktree.join(".autospec/events.jsonl");
    bridge::ensure_private_directory(state_path.parent().expect("state parent"))
        .expect("private state parent");
    bridge::write_invocation_atomic(&state_path, &state).expect("initial invocation state");
    let harness = shell_invocation(
        &state.identity.worktree,
        "trap '' TERM; while :; do :; done",
    );
    let _uncertainty = force_next_termination_uncertainty_for_test();

    let error = bridge::supervise_harness(
        &state_path,
        &event_log,
        &mut state,
        &harness,
        &snapshot,
        supervision_config(25),
    )
    .expect_err("actual stall cleanup must retain uncertain ownership");

    assert!(
        error.contains("injected Darwin termination uncertainty"),
        "{error}"
    );
    let durable = bridge::PersistedInvocation::from_json(
        &fs::read_to_string(&state_path).expect("durable invocation"),
    )
    .expect("parse durable invocation");
    assert_eq!(durable.phase, BridgePhase::Interrupted);
    let retained = durable.process.as_ref().expect("retained exact identity");
    let event = fs::read_to_string(&event_log).expect("recovery-required event");
    assert!(
        event.contains("\"child_stall_cleanup_uncertain\""),
        "{event}"
    );
    assert!(event.contains("\"recovery_required\":true"), "{event}");
    assert!(
        event.contains("\"exact_process_identity_retained\":true"),
        "{event}"
    );

    let pid = Pid::from_raw(retained.pid as i32);
    let process_group = Pid::from_raw(retained.process_group as i32);
    if !group_is_empty(retained.process_group).expect("inspect quarantined exact group") {
        killpg(process_group, Signal::SIGKILL).expect("clean quarantined test group");
    }
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match waitpid(pid, Some(WaitPidFlag::WNOHANG)) {
            Ok(WaitStatus::Exited(..) | WaitStatus::Signaled(..))
            | Err(Errno::ECHILD)
            | Err(Errno::ESRCH) => break,
            Ok(WaitStatus::StillAlive) | Err(Errno::EINTR) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            status => panic!("quarantined Darwin test group was not reaped: {status:?}"),
        }
    }
    assert!(group_is_empty(retained.process_group).expect("prove test group empty"));
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_nested_quarantine_preflight_spans_the_whole_subtree() {
    // Break caught: actionful reconciliation in a parent/sibling direct root occurring before
    // a malformed marker-only child root is recursively validated.
    let fixture = NonDescendantDirectFixture::new("nested-quarantine-whole-subtree");
    fixture.replace_marker(&fixture.exact_marker());
    let root = fixture.paths.record.parent().expect("nested root");
    let later_root = root.join("full");
    bridge::ensure_private_directory(&later_root).expect("later nested root");
    let later_paths = bridge::direct_attempt_paths(&later_root, 0);
    let later_marker = bridge::direct_ownership_disproven_marker(&later_paths, &"b".repeat(64))
        .expect("later nested marker");
    bridge::write_private_create_once(
        &later_marker,
        b"{\"schema\":1}",
        "malformed child ownership quarantine",
    )
    .expect("malformed child marker");
    let first_launch = fs::read(&fixture.paths.launch).expect("first launch snapshot");
    let later_marker_body = fs::read(&later_marker).expect("child marker snapshot");

    let error = bridge::reconcile_nested_direct_ownership(root)
        .expect_err("whole nested subtree must validate before action");
    assert!(error.contains("quarantine marker schema"), "{error}");
    fixture.assert_anchor_liveness(true, true);
    assert_eq!(
        fs::read(&fixture.paths.launch).expect("first launch after subtree preflight"),
        first_launch
    );
    assert_eq!(
        fs::read(&later_marker).expect("child marker after subtree preflight"),
        later_marker_body
    );
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_nested_marker_only_bad_filename_fails_closed() {
    // Break caught: an ownership-like marker-only filename with a trailing suffix bypassing
    // direct-artifact detection and therefore strict grammar validation.
    let fixture = GitFixture::new("nested-marker-only-bad-filename");
    let root = fixture.root.join("nested");
    bridge::ensure_private_directory(&root).expect("nested root");
    let marker = root.join(format!(
        "command-000.ownership-disproven-{}.json.extra",
        "c".repeat(64)
    ));
    bridge::write_private_create_once(&marker, b"{}", "bad filename ownership quarantine")
        .expect("bad filename marker");
    let before = fs::read(&marker).expect("bad filename marker snapshot");

    let error = bridge::reconcile_nested_direct_ownership(&root)
        .expect_err("ownership-like bad filename must fail closed");
    assert!(error.contains("non-canonical"), "{error}");
    assert_eq!(
        fs::read(&marker).expect("bad filename marker after preflight"),
        before
    );
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_cleanup_capture_tracks_same_instance_after_pgid_change() {
    // Break caught: cleanup treating a mutable process-group transition as PID reuse and
    // either abandoning the live process or adopting it without an immutable birth check.
    let fixture = GitFixture::new("cleanup-pgid-transition");
    let ready = fixture.root.join("ready");
    let release = fixture.root.join("release");
    let script = format!(
        "from pathlib import Path\nimport os,time\nPath(r'{}').write_text('ready')\n\
         gate=Path(r'{}')\nwhile not gate.exists():\n time.sleep(0.001)\n\
         os.setpgid(0,0)\ntime.sleep(30)\n",
        ready.display(),
        release.display()
    );
    let args = vec!["-c".to_string(), script.clone()];
    let mut child = Command::new("/usr/bin/python3")
        .args(&args)
        .spawn()
        .expect("spawn PGID transition fixture");
    let mut cleanup = DetachedForkedCleanup::new(child.id()).expect("arm PGID transition cleanup");
    let deadline = Instant::now() + Duration::from_secs(2);
    while !ready.is_file() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(1));
    }
    let expected = bridge::observe_process_identity(child.id(), &bridge::argv_digest(&args))
        .expect("observe pre-transition fixture")
        .expect("live pre-transition fixture");
    cleanup.confirm_identity(expected.clone());
    fs::write(&release, "release").expect("release PGID transition");
    let deadline = Instant::now() + Duration::from_secs(2);
    let observed = loop {
        let observed = bridge::observe_process_identity(child.id(), &expected.argv_digest)
            .expect("observe post-transition fixture")
            .expect("live post-transition fixture");
        if observed.process_group != expected.process_group {
            break observed;
        }
        assert!(
            Instant::now() < deadline,
            "fixture never changed its process group"
        );
        std::thread::sleep(Duration::from_millis(1));
    };

    assert!(expected.owns_instance(&observed.birth()));
    assert!(!expected.owns_birth(&observed.birth()));
    let captured = bridge::OwnedProcess::capture_cleanup_instance(&expected)
        .expect("capture same immutable instance after PGID transition");
    assert_eq!(captured.birth.process_group, observed.process_group);
    assert!(captured.is_live().expect("post-transition pidfd liveness"));
    captured
        .signal(Signal::SIGKILL)
        .expect("terminate PGID transition fixture");
    child.wait().expect("reap PGID transition fixture");
    cleanup.processes = None;
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_absent_anchors_do_not_prove_orphan_cleanup() {
    // Break caught: two dead persisted anchors being treated as proof that an unrelated
    // closed-stdio orphan from the old tree cannot still be alive.
    let mut anchor = Command::new("/usr/bin/sleep")
        .arg("30")
        .spawn()
        .expect("spawn anchor fixture");
    let args = vec!["30".to_string()];
    let identity = bridge::observe_process_identity(anchor.id(), &bridge::argv_digest(&args))
        .expect("observe anchor fixture")
        .expect("live anchor fixture");
    anchor.kill().expect("kill anchor fixture");
    anchor.wait().expect("reap anchor fixture");
    let mut orphan = Command::new("/usr/bin/sleep")
        .arg("30")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn closed-stdio orphan fixture");

    let error =
        bridge::OwnedProcessSet::terminate_supervised_for_cleanup(&identity, Some(&identity))
            .expect_err("absent anchors cannot prove whole-tree cleanup");

    assert!(error.reason.contains("unproven"), "{error:?}");
    assert!(
        bridge::observe_process_birth(orphan.id())
            .expect("observe guarded orphan")
            .is_some(),
        "unowned orphan fixture unexpectedly disappeared"
    );
    orphan.kill().expect("guard cleans known orphan");
    orphan.wait().expect("guard reaps known orphan");
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_dead_supervisor_cleans_live_harness_before_recovery() {
    let _environment = test_environment();
    let fixture = GitFixture::new("dead-supervisor-live-harness");
    let mut state = supervision_state(&fixture);
    let state_path = fixture.root.join("state/invocation.json");
    let event_log = fixture.root.join("log/executor.jsonl");
    let launches = fixture.root.join("unexpected-launch");
    let validated = detach_harness_for_adoption(
        &fixture,
        &state_path,
        &mut state,
        "printf 'running\\n'; exec >/dev/null 2>&1; trap '' HUP; while :; do sleep 1; done",
    );
    let supervisor = state.supervisor.clone().expect("supervisor identity");
    let harness = state.process.clone().expect("harness identity");
    let owned =
        bridge::OwnedProcess::capture(&supervisor.birth()).expect("capture exact supervisor");
    owned.signal(Signal::SIGKILL).expect("kill only supervisor");
    for _ in 0..100 {
        if bridge::observe_process_birth(supervisor.pid)
            .expect("supervisor observation")
            .is_none()
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        bridge::observe_process_birth(harness.pid)
            .expect("harness observation")
            .is_some(),
        "fixture harness must survive supervisor loss"
    );
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
    .expect_err("dead supervisor requires exact harness cleanup");

    assert!(error.contains("recovery"), "{error}");
    assert!(
        bridge::observe_process_birth(harness.pid)
            .expect("post-cleanup harness")
            .is_none(),
        "live harness survived dead-supervisor recovery"
    );
    assert!(!launches.exists(), "duplicate harness was launched");
}

#[cfg(target_os = "linux")]
fn assert_persisted_dead_supervisor_cleans_exec_replaced_harness(
    fixture: &GitFixture,
    script: &str,
    marker: &Path,
) {
    let mut state = supervision_state(fixture);
    let state_path = fixture.root.join("state/invocation.json");
    let event_log = fixture.root.join("log/executor.jsonl");
    let validated = detach_harness_for_adoption(fixture, &state_path, &mut state, script);
    let supervisor = state.supervisor.clone().expect("supervisor identity");
    let harness = state.process.clone().expect("harness identity");
    let deadline = Instant::now() + Duration::from_secs(2);
    while !marker.is_file() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(marker.is_file(), "exec-replaced harness never became ready");
    let observed = bridge::observe_process_birth(harness.pid)
        .expect("observe exec-replaced harness")
        .expect("live exec-replaced harness");
    assert!(harness.owns_instance(&observed));

    bridge::OwnedProcess::capture(&supervisor.birth())
        .expect("capture exact supervisor")
        .signal(Signal::SIGKILL)
        .expect("kill only supervisor");
    let deadline = Instant::now() + Duration::from_secs(2);
    while bridge::observe_process_birth(supervisor.pid)
        .expect("observe dead supervisor")
        .is_some()
        && Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(10));
    }
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
    .expect_err("dead supervisor without durable EXIT requires recovery");

    assert!(
        error.contains("without a durable completion record"),
        "{error}"
    );
    assert!(
        bridge::observe_process_birth(harness.pid)
            .expect("post-cleanup harness")
            .is_none(),
        "exec-replaced harness survived persisted-state recovery"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_persisted_recovery_cleans_shebang_harness() {
    let _environment = test_environment();
    let fixture = GitFixture::new("persisted-shebang-cleanup");
    let marker = fixture.root.join("shebang.ready");
    let program = fixture.root.join("shebang-harness");
    fs::write(
        &program,
        format!(
            "#!/usr/bin/python3\nfrom pathlib import Path\nimport time\nPath(r'{}').write_text('ready')\ntime.sleep(30)\n",
            marker.display()
        ),
    )
    .expect("write shebang harness");
    fs::set_permissions(&program, fs::Permissions::from_mode(0o700))
        .expect("executable shebang harness");

    assert_persisted_dead_supervisor_cleans_exec_replaced_harness(
        &fixture,
        &format!("exec '{}'", program.display()),
        &marker,
    );
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_persisted_recovery_cleans_immediate_exec_harness() {
    let _environment = test_environment();
    let fixture = GitFixture::new("persisted-immediate-exec-cleanup");
    let marker = fixture.root.join("exec.ready");
    let script = format!(
        "exec /usr/bin/python3 -c 'from pathlib import Path; import time; Path(r\"{}\").write_text(\"ready\"); time.sleep(30)'",
        marker.display()
    );

    assert_persisted_dead_supervisor_cleans_exec_replaced_harness(&fixture, &script, &marker);
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_dead_supervisor_recovers_synced_exit_and_output() {
    let _environment = test_environment();
    let fixture = GitFixture::new("dead-supervisor-complete");
    let mut state = supervision_state(&fixture);
    let state_path = fixture.root.join("state/invocation.json");
    let event_log = fixture.root.join("log/executor.jsonl");
    let validated = detach_harness_for_adoption(
        &fixture,
        &state_path,
        &mut state,
        "printf 'dead-supervisor-durable-tail\\n' >&2; exit 7",
    );
    let supervisor = state.supervisor.clone().expect("supervisor identity");
    let sinks =
        bridge::output_sink_paths(&state_path, &state.identity.invocation_id).expect("sinks");
    let deadline = Instant::now() + Duration::from_secs(2);
    while bridge::read_live_executor_exit_status(&sinks.exit_status).expect("poll durable exit")
        != Some(7)
        && Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        bridge::read_executor_exit_status(&sinks.exit_status).expect("synced durable exit"),
        Some(7)
    );
    bridge::OwnedProcess::capture(&supervisor.birth())
        .expect("capture completed supervisor")
        .signal(Signal::SIGKILL)
        .expect("kill completed supervisor");
    let deadline = Instant::now() + Duration::from_secs(2);
    while bridge::observe_process_birth(supervisor.pid)
        .expect("observe completed supervisor")
        .is_some()
        && Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(10));
    }
    let snapshot =
        MutationSnapshot::capture(&fixture.repo, &state.identity.branch).expect("snapshot");

    let outcome = bridge::supervise_validated_harness(
        &state_path,
        &event_log,
        &mut state,
        &validated,
        &snapshot,
        supervision_config(500),
    )
    .expect("recover exact durable failure outcome");
    let events = fs::read_to_string(event_log).expect("recovered events");

    assert_eq!(outcome, SupervisionOutcome::Exited { exit_code: 7 });
    assert_eq!(state.phase, BridgePhase::Interrupted);
    assert!(events.contains("dead-supervisor-durable-tail"), "{events}");
    assert!(
        events.contains("\"recovered_after_supervisor_exit\":true"),
        "{events}"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_restart_finalizes_done_after_anchor_clear_crashes() {
    let environment = test_environment();
    // Break caught: clearing durable anchors before drain/snapshot, then treating the dead
    // sidecar as unproven on restart even though strict whole-tree DONE is durable.
    for boundary in [
        bridge::LaunchFailpoint::RecoveryAfterAnchorClear,
        bridge::LaunchFailpoint::RecoveryBeforeSnapshot,
    ] {
        let fixture = GitFixture::new(&format!("done-anchor-clear-{boundary:?}"));
        let mut state = supervision_state(&fixture);
        let state_path = fixture.root.join("state/invocation.json");
        let event_log = fixture.root.join("log/executor.jsonl");
        let validated = detach_harness_for_adoption(
            &fixture,
            &state_path,
            &mut state,
            "printf 'restart-finalized-tail\\n' >&2; exit 7",
        );
        let supervisor = state.supervisor.clone().expect("supervisor identity");
        let sinks = bridge::output_sink_paths(&state_path, &state.identity.invocation_id)
            .expect("output sinks");
        let deadline = Instant::now() + Duration::from_secs(2);
        while bridge::read_live_executor_exit_status(&sinks.exit_status).expect("poll durable exit")
            != Some(7)
            && Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(10));
        }
        bridge::OwnedProcess::capture(&supervisor.birth())
            .expect("capture completed supervisor")
            .signal(Signal::SIGKILL)
            .expect("kill completed supervisor");
        let deadline = Instant::now() + Duration::from_secs(2);
        while bridge::observe_process_birth(supervisor.pid)
            .expect("observe completed supervisor")
            .is_some()
            && Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(10));
        }
        let snapshot =
            MutationSnapshot::capture(&fixture.repo, &state.identity.branch).expect("snapshot");

        environment.launch(boundary);
        let interrupted = bridge::supervise_validated_harness(
            &state_path,
            &event_log,
            &mut state,
            &validated,
            &snapshot,
            supervision_config(500),
        );
        environment.launch(bridge::LaunchFailpoint::None);
        interrupted.expect_err("recovery failpoint interrupts finalization");
        assert!(state.supervisor.is_none());
        assert!(state.process.is_none());

        let outcome = bridge::supervise_validated_harness(
            &state_path,
            &event_log,
            &mut state,
            &validated,
            &snapshot,
            supervision_config(500),
        )
        .expect("restart finalizes strict DONE without anchors");
        let events = fs::read_to_string(&event_log).expect("recovered events");

        assert_eq!(outcome, SupervisionOutcome::Exited { exit_code: 7 });
        assert_eq!(state.phase, BridgePhase::Interrupted);
        assert!(events.contains("restart-finalized-tail"), "{events}");
        assert!(events.contains("\"event\":\"child_exited\""), "{events}");
    }
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_dead_supervisor_rejects_partial_exit_record() {
    let _environment = test_environment();
    let fixture = GitFixture::new("dead-supervisor-partial-exit");
    let mut state = supervision_state(&fixture);
    let state_path = fixture.root.join("state/invocation.json");
    let event_log = fixture.root.join("log/executor.jsonl");
    let validated = detach_harness_for_adoption(&fixture, &state_path, &mut state, "sleep 30");
    let supervisor = state.supervisor.clone().expect("supervisor identity");
    let harness = state.process.clone().expect("harness identity");
    bridge::OwnedProcess::capture(&supervisor.birth())
        .expect("capture partial-record supervisor")
        .signal(Signal::SIGKILL)
        .expect("kill partial-record supervisor");
    let sinks =
        bridge::output_sink_paths(&state_path, &state.identity.invocation_id).expect("sinks");
    let mut partial = [0_u8; 16];
    partial[4..6].copy_from_slice(b"EX");
    fs::write(&sinks.exit_status, partial).expect("write partial exit record");
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
    .expect_err("dead partial exit record must fail closed");

    assert!(
        error.contains("invalid durable completion record"),
        "{error}"
    );
    assert!(error.contains("malformed"), "{error}");
    assert_eq!(state.phase, BridgePhase::Interrupted);
    assert!(
        bridge::observe_process_birth(harness.pid)
            .expect("partial-record harness cleanup")
            .is_none(),
        "partial record recovery leaked the exact harness"
    );
    let events = fs::read_to_string(event_log).expect("partial record events");
    assert!(!events.contains("\"event\":\"child_exited\""), "{events}");
}
