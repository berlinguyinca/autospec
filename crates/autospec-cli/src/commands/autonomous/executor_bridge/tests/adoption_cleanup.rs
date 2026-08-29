// executor_bridge tests: adoption / cleanup — 8 cases.
//
// Split out of tests.rs; see the note in that file.

use super::super::{BridgePhase, MutationSnapshot, PersistedInvocation, SupervisionOutcome};
use super::support_base::{
    observe_spawned_identity, test_environment, test_root, DetachedForkedCleanup,
    DetachedSupervisorCleanup, GitFixture,
};
use super::support_invocation::{
    detach_harness_for_adoption, supervision_config, supervision_state, NonDescendantDirectFixture,
};
use crate::commands::autonomous::executor_bridge as bridge;
#[cfg(target_os = "linux")]
use nix::sys::signal::Signal;
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_partial_adoption_cleanup_failure_retains_identities() {
    let environment = test_environment();
    let fixture = GitFixture::new("adopt-partial-quarantine");
    let mut state = supervision_state(&fixture);
    let state_path = fixture.root.join("state/invocation.json");
    let event_log = fixture.root.join("log/executor.jsonl");
    let validated = detach_harness_for_adoption(
        &fixture,
        &state_path,
        &mut state,
        "while :; do sleep 1; done",
    );
    let supervisor = state.supervisor.clone().expect("supervisor identity");
    let harness = state.process.clone().expect("harness identity");
    state
        .process
        .as_mut()
        .expect("persisted harness")
        .argv_digest = "f".repeat(64);
    bridge::write_invocation_atomic(&state_path, &state).expect("persist mismatched identity");
    let snapshot =
        MutationSnapshot::capture(&fixture.repo, &state.identity.branch).expect("snapshot");
    environment.cleanup(bridge::LaunchFailpoint::CleanupSignal);
    let error = bridge::supervise_validated_harness(
        &state_path,
        &event_log,
        &mut state,
        &validated,
        &snapshot,
        supervision_config(100),
    )
    .expect_err("partial adoption cleanup failure");
    let durable = PersistedInvocation::from_json(
        &fs::read_to_string(&state_path).expect("durable partial quarantine"),
    )
    .expect("strict partial quarantine");
    environment.cleanup(bridge::LaunchFailpoint::None);
    if bridge::observe_process_identity(supervisor.pid, &supervisor.argv_digest)
        .expect("observe quarantined supervisor")
        .is_some()
    {
        let mut owned =
            bridge::OwnedProcessSet::adopt_supervised(&supervisor, Some(&harness), false)
                .expect("recapture quarantined tree");
        owned.terminate().expect("clean quarantined tree");
    }

    assert!(error.contains("cleanup"), "{error}");
    assert_eq!(durable.phase, BridgePhase::Interrupted);
    assert!(durable.supervisor.is_some());
    assert!(durable.process.is_some());
    let events = fs::read_to_string(event_log).expect("partial quarantine event");
    assert!(events.contains("\"event\":\"child_supervision_error\""));
    assert!(events.contains("\"adopted\":true"), "{events}");
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_adoption_replays_ring_and_accounts_overwrite() {
    let _environment = test_environment();
    let fixture = GitFixture::new("adopt-ring-replay");
    let mut state = supervision_state(&fixture);
    let state_path = fixture.root.join("state/invocation.json");
    let event_log = fixture.root.join("log/executor.jsonl");
    let validated = detach_harness_for_adoption(
        &fixture,
        &state_path,
        &mut state,
        "sleep 0.1; head -c 2097152 /dev/zero | tr '\\0' x; printf '\\ncrash-window-marker\\n'; exit 0",
    );
    let _cleanup = DetachedSupervisorCleanup(
        state
            .supervisor
            .clone()
            .expect("detached supervisor identity"),
    );
    let sinks =
        bridge::output_sink_paths(&state_path, &state.identity.invocation_id).expect("sinks");
    // The supervisor fdatasyncs every 64 KiB ring write. On a loaded CI disk, the
    // 2 MiB overwrite fixture can legitimately need longer than ten seconds.
    for _ in 0..3_000 {
        if bridge::read_live_executor_exit_status(&sinks.exit_status)
            .expect("exit record")
            .is_some()
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        bridge::read_executor_exit_status(&sinks.exit_status).expect("durable exit"),
        Some(0)
    );
    let snapshot =
        MutationSnapshot::capture(&fixture.repo, &state.identity.branch).expect("snapshot");

    let outcome = bridge::supervise_validated_harness(
        &state_path,
        &event_log,
        &mut state,
        &validated,
        &snapshot,
        supervision_config(2_000),
    )
    .expect("adopt completed ring");

    assert_eq!(outcome, SupervisionOutcome::Exited { exit_code: 0 });
    assert_eq!(
        fs::metadata(&sinks.stdout).expect("stdout ring").len(),
        bridge::OUTPUT_SINK_LIMIT
    );
    let backup = event_log.with_extension("jsonl.1");
    let mut events = fs::read_to_string(&event_log).expect("current events");
    if backup.exists() {
        events.push_str(&fs::read_to_string(&backup).expect("rotated events"));
    }
    assert!(
        events.contains("\"event\":\"child_output_dropped\""),
        "{events}"
    );
    assert!(events.contains("\"dropped_bytes\":"), "{events}");
    assert!(events.contains("crash-window-marker"), "{events}");
    assert!(
        fs::metadata(&event_log)
            .expect("current event segment")
            .len()
            <= bridge::EVENT_LOG_SEGMENT_LIMIT
    );
    if backup.exists() {
        assert!(
            fs::metadata(backup).expect("backup event segment").len()
                <= bridge::EVENT_LOG_SEGMENT_LIMIT
        );
    }
    let writer = bridge::read_output_cursor(
        &OpenOptions::new()
            .read(true)
            .open(&sinks.stdout_writer_cursor)
            .expect("writer cursor"),
    )
    .expect("writer position");
    let reader = bridge::read_output_cursor(
        &OpenOptions::new()
            .read(true)
            .open(&sinks.stdout_reader_cursor)
            .expect("reader cursor"),
    )
    .expect("reader position");
    assert_eq!(
        reader.total, writer.total,
        "crash-window output was not acknowledged"
    );
    assert!(reader.dropped > 0, "overwritten bytes were not persisted");
}

#[test]
fn autonomous_executor_bridge_event_log_rotation_has_a_hard_disk_cap() {
    let fixture = GitFixture::new("bounded-event-log");
    let state = supervision_state(&fixture);
    let event_log = fixture.root.join("log/executor.jsonl");
    for sequence in 0..500 {
        bridge::append_executor_event(
            &event_log,
            &state,
            "child_output",
            Some(serde_json::json!({
                "stream": "stdout",
                "output": "x".repeat(4_096),
                "sequence": sequence
            })),
        )
        .expect("append bounded event");
    }

    let backup = event_log.with_extension("jsonl.1");
    assert!(
        fs::metadata(&event_log).expect("current segment").len() <= bridge::EVENT_LOG_SEGMENT_LIMIT
    );
    assert!(
        fs::metadata(&backup).expect("backup segment").len() <= bridge::EVENT_LOG_SEGMENT_LIMIT
    );
    let current = fs::read_to_string(event_log).expect("current events");
    assert!(current.contains("\"sequence\":499"), "{current}");
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_adopted_errors_are_structured_and_cleaned() {
    let environment = test_environment();
    for (name, failpoint) in [
        ("poll", bridge::LaunchFailpoint::AdoptedPoll),
        ("flush", bridge::LaunchFailpoint::AdoptedFlush),
        ("log", bridge::LaunchFailpoint::AdoptedLog),
    ] {
        let fixture = GitFixture::new(&format!("adopt-{name}-error"));
        let mut state = supervision_state(&fixture);
        let state_path = fixture.root.join("state/invocation.json");
        let event_log = fixture.root.join("log/executor.jsonl");
        let descendant_pid = fixture.root.join("descendant.pid");
        let validated = detach_harness_for_adoption(
            &fixture,
            &state_path,
            &mut state,
            &format!(
                "sleep 30 & child=$!; printf '%s\\n' \"$child\" > '{}'; printf 'progress\\n'; wait \"$child\"",
                descendant_pid.display()
            ),
        );
        for _ in 0..100 {
            if descendant_pid.exists() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let supervisor_pid = state.supervisor.as_ref().expect("supervisor").pid;
        let descendant = fs::read_to_string(&descendant_pid)
            .expect("descendant identity")
            .trim()
            .to_string();
        let snapshot =
            MutationSnapshot::capture(&fixture.repo, &state.identity.branch).expect("snapshot");

        environment.launch(failpoint);
        let error = bridge::supervise_validated_harness(
            &state_path,
            &event_log,
            &mut state,
            &validated,
            &snapshot,
            supervision_config(2_000),
        )
        .expect_err("injected adopted supervision error");
        environment.launch(bridge::LaunchFailpoint::None);

        assert!(error.contains("injected"), "{error}");
        assert!(state.supervisor.is_none());
        assert!(state.process.is_none());
        for pid in [supervisor_pid.to_string(), descendant] {
            for _ in 0..40 {
                if !Path::new(&format!("/proc/{pid}")).exists() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            assert!(
                !Path::new(&format!("/proc/{pid}")).exists(),
                "adopted process {pid} survived {name} failure"
            );
        }
        let events = fs::read_to_string(&event_log).expect("structured error event");
        assert!(
            events.contains("\"event\":\"child_supervision_error\""),
            "{events}"
        );
        assert!(
            events.contains(&format!("injected adopt-{name} failure")),
            "{events}"
        );
    }
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_cursor_failure_is_structured_and_cleaned() {
    let _environment = test_environment();
    let fixture = GitFixture::new("adopt-cursor-error");
    let mut state = supervision_state(&fixture);
    let state_path = fixture.root.join("state/invocation.json");
    let event_log = fixture.root.join("log/executor.jsonl");
    let validated = detach_harness_for_adoption(
        &fixture,
        &state_path,
        &mut state,
        "printf 'progress\\n'; sleep 30",
    );
    let supervisor_pid = state.supervisor.as_ref().expect("supervisor").pid;
    let sinks =
        bridge::output_sink_paths(&state_path, &state.identity.invocation_id).expect("sinks");
    OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(&sinks.stdout_reader_cursor)
        .expect("corrupt reader cursor")
        .write_all(&[0_u8; bridge::OUTPUT_CURSOR_FILE_BYTES as usize])
        .expect("write invalid cursor");
    let snapshot =
        MutationSnapshot::capture(&fixture.repo, &state.identity.branch).expect("snapshot");

    let error = bridge::supervise_validated_harness(
        &state_path,
        &event_log,
        &mut state,
        &validated,
        &snapshot,
        supervision_config(2_000),
    )
    .expect_err("invalid cursor must fail closed");

    assert!(error.contains("cursor"), "{error}");
    assert!(!Path::new(&format!("/proc/{supervisor_pid}")).exists());
    let events = fs::read_to_string(event_log).expect("structured cursor error");
    assert!(
        events.contains("\"event\":\"child_supervision_error\""),
        "{events}"
    );
    assert!(events.contains("cursor"), "{events}");
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_pidfd_adoption_requires_full_exec_identity() {
    let mut child = Command::new("/bin/sh")
        .args(["-c", "read _; exec sleep 30"])
        .stdin(Stdio::piped())
        .spawn()
        .expect("spawn pre-exec fixture");
    let args = vec!["-c".to_string(), "read _; exec sleep 30".to_string()];
    let mut cleanup = DetachedForkedCleanup::new(child.id()).expect("arm pre-exec fixture cleanup");
    let expected = observe_spawned_identity(child.id(), &args);
    cleanup.confirm_identity(expected.clone());
    child
        .stdin
        .take()
        .expect("fixture stdin")
        .write_all(b"go\n")
        .expect("release exec");
    for _ in 0..100 {
        if let Some(observed) = bridge::observe_process_identity(child.id(), &expected.argv_digest)
            .expect("observe exec transition")
        {
            if observed.executable != expected.executable {
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    let error = bridge::OwnedProcess::capture_identity(&expected)
        .expect_err("same-birth exec replacement must not be adopted");
    assert!(error.contains("full identity"), "{error}");
    assert!(child.try_wait().expect("replacement liveness").is_none());
    let owned =
        bridge::OwnedProcess::capture_forked_child(child.id()).expect("capture cleanup pidfd");
    owned.signal(Signal::SIGKILL).expect("clean replacement");
    let _ = child.wait();
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_prunes_exited_descendant_pidfds() {
    let _environment = test_environment();
    let leader = bridge::OwnedProcess::capture_forked_child(std::process::id())
        .expect("capture test process");
    let mut processes = bridge::OwnedProcessSet {
        leader,
        descendants: BTreeMap::new(),
        exited_descendants: BTreeMap::new(),
    };
    let mut exited_keys = Vec::new();
    for _ in 0..32 {
        let mut child = Command::new("/bin/sleep")
            .arg("30")
            .spawn()
            .expect("spawn exited descendant fixture");
        let _cleanup = DetachedForkedCleanup::new(child.id()).expect("arm exited fixture cleanup");
        let owned = bridge::OwnedProcess::capture_forked_child(child.id())
            .expect("capture exited descendant pidfd");
        let key = (owned.birth.pid, owned.birth.start_identity.clone());
        owned.signal(Signal::SIGKILL).expect("stop exited fixture");
        child.wait().expect("reap exited fixture");
        assert!(!owned.is_live().expect("observe exited fixture"));
        processes.descendants.insert(key.clone(), owned);
        exited_keys.push(key);
    }
    let mut zombie_child = Command::new("/bin/sleep")
        .arg("30")
        .spawn()
        .expect("spawn unreaped descendant fixture");
    let _zombie_cleanup =
        DetachedForkedCleanup::new(zombie_child.id()).expect("arm unreaped fixture cleanup");
    let zombie = bridge::OwnedProcess::capture_forked_child(zombie_child.id())
        .expect("capture unreaped descendant pidfd");
    let zombie_key = (zombie.birth.pid, zombie.birth.start_identity.clone());
    zombie
        .signal(Signal::SIGKILL)
        .expect("stop unreaped fixture");
    for _ in 0..100 {
        if !zombie.is_live().expect("observe unreaped fixture") {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!zombie.is_live().expect("observe unreaped fixture"));
    processes.descendants.insert(zombie_key.clone(), zombie);

    let nonchild_root = test_root("unreaped-non-child-descendant");
    let nonchild_pid_path = nonchild_root.join("pid");
    let mut intermediary = Command::new("/bin/sh")
        .arg("-c")
        .arg("sleep 30 & printf '%s\\n' \"$!\" > \"$1\"; exec sleep 30")
        .arg("sh")
        .arg(&nonchild_pid_path)
        .spawn()
        .expect("spawn intermediary descendant fixture");
    let _intermediary_cleanup =
        DetachedForkedCleanup::new(intermediary.id()).expect("arm intermediary fixture cleanup");
    let mut nonchild_pid = None;
    for _ in 0..100 {
        if let Ok(pid) = fs::read_to_string(&nonchild_pid_path) {
            nonchild_pid = Some(
                pid.trim()
                    .parse::<u32>()
                    .expect("parse non-child descendant PID"),
            );
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let nonchild_pid = nonchild_pid.expect("intermediary published descendant PID");
    let nonchild = bridge::OwnedProcess::capture_forked_child(nonchild_pid)
        .expect("capture non-child descendant pidfd");
    let nonchild_key = (nonchild.birth.pid, nonchild.birth.start_identity.clone());
    nonchild
        .signal(Signal::SIGKILL)
        .expect("stop non-child descendant fixture");
    for _ in 0..100 {
        if !nonchild.is_live().expect("observe non-child fixture") {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!nonchild.is_live().expect("observe non-child fixture"));
    processes.descendants.insert(nonchild_key.clone(), nonchild);
    let mut live_child = Command::new("/bin/sleep")
        .arg("30")
        .spawn()
        .expect("spawn live descendant fixture");
    let _live_cleanup =
        DetachedForkedCleanup::new(live_child.id()).expect("arm live fixture cleanup");
    let live = bridge::OwnedProcess::capture_forked_child(live_child.id())
        .expect("capture live descendant pidfd");
    let live_key = (live.birth.pid, live.birth.start_identity.clone());
    processes.descendants.insert(live_key.clone(), live);

    processes
        .capture_descendants_while_leader_live()
        .expect("first bounded capture");
    processes
        .capture_descendants_while_leader_live()
        .expect("second bounded capture");

    assert!(exited_keys
        .iter()
        .all(|key| !processes.descendants.contains_key(key)));
    assert!(!processes.descendants.contains_key(&zombie_key));
    assert!(!processes.descendants.contains_key(&nonchild_key));
    assert!(processes.exited_descendants.contains_key(&nonchild_key));
    assert!(processes.descendants.contains_key(&live_key));
    match zombie_child.wait() {
        Ok(_) => {}
        Err(error) if error.raw_os_error() == Some(nix::libc::ECHILD) => {}
        Err(error) => panic!("reap unreaped fixture: {error}"),
    }
    bridge::OwnedProcess::capture_forked_child(intermediary.id())
        .expect("capture intermediary cleanup pidfd")
        .signal(Signal::SIGKILL)
        .expect("stop intermediary fixture");
    intermediary.wait().expect("reap intermediary fixture");
    let nonchild_disappeared = (0..100).any(|_| {
        processes
            .reap_descendants()
            .expect("reap retained non-child fixture");
        let disappeared = bridge::observe_process_birth(nonchild_pid)
            .expect("observe reparented non-child fixture")
            .is_none();
        if !disappeared {
            std::thread::sleep(Duration::from_millis(10));
        }
        disappeared
    });
    processes
        .capture_descendants_while_leader_live()
        .expect("reconcile reparented non-child tombstone");
    assert_eq!(
        processes.exited_descendants.contains_key(&nonchild_key),
        !nonchild_disappeared
    );
    processes
        .descendants
        .get(&live_key)
        .expect("retained live pidfd")
        .signal(Signal::SIGKILL)
        .expect("stop live fixture");
    live_child.wait().expect("reap live fixture");
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_cleanup_never_retires_a_live_non_descendant_harness() {
    // Break caught: cleanup of a valid supervisor treating an unrelated persisted harness as
    // cleaned and retiring the only durable identity that can quarantine it.
    let fixture = NonDescendantDirectFixture::new("non-descendant-quarantine");
    let error = bridge::reconcile_direct_launch(&fixture.paths, Some(&fixture.intent))
        .expect_err("live non-descendant harness must remain quarantined");
    assert!(error.contains("not a descendant"), "{error}");
    fixture.assert_anchor_liveness(false, true);
    assert!(fixture.marker_path().is_file());
    for _ in 0..3 {
        let retry = bridge::reconcile_direct_launch(&fixture.paths, Some(&fixture.intent))
            .expect_err("durable quarantine must reject every retry");
        assert!(retry.contains("permanently quarantined"), "{retry}");
        fixture.assert_anchor_liveness(false, true);
        assert!(
            fixture.paths.launch.is_file(),
            "quarantined launch was retired"
        );
    }
}
