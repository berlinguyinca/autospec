// executor_bridge tests: cleanup / reap — 8 cases.
//
// Split out of tests.rs; see the note in that file.

use crate::commands::autonomous::executor_bridge as bridge;
use super::super::{
    supervise_harness, BridgePhase, MutationSnapshot, PersistedInvocation, SupervisionOutcome,
};
use super::support_base::{DetachedForkedCleanup, GitFixture, observe_spawned_identity, reap_fixture_child_within, test_environment};
use super::support_invocation::{
    detach_harness_for_adoption, shell_invocation, supervision_config, supervision_state,
};
use std::fs::{self, OpenOptions};
use std::process::Command;
use std::sync::atomic::Ordering;
use std::time::Duration;

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_missing_adopted_journal_fails_without_truncation() {
    let _environment = test_environment();
    let fixture = GitFixture::new("missing-adopted-journal");
    let mut state = supervision_state(&fixture);
    let state_path = fixture.root.join("state/invocation.json");
    let event_log = fixture.root.join("log/executor.jsonl");
    let validated = detach_harness_for_adoption(
        &fixture,
        &state_path,
        &mut state,
        "printf 'journal-evidence\\n'; sleep 30",
    );
    let sinks =
        bridge::output_sink_paths(&state_path, &state.identity.invocation_id).expect("sinks");
    for _ in 0..100 {
        let writer = OpenOptions::new()
            .read(true)
            .open(&sinks.stdout_writer_cursor)
            .expect("writer cursor");
        if bridge::read_output_cursor(&writer)
            .expect("writer position")
            .total
            > 0
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let stdout_before = fs::read(&sinks.stdout).expect("surviving ring");
    fs::remove_file(&sinks.stderr_reader_cursor).expect("remove one journal member");
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
    .expect_err("incomplete adopted journal must fail closed");

    assert!(error.contains("journal"), "{error}");
    assert!(!sinks.stderr_reader_cursor.exists());
    assert_eq!(
        fs::read(&sinks.stdout).expect("surviving ring"),
        stdout_before
    );
    assert!(fs::read_to_string(event_log)
        .expect("structured journal failure")
        .contains("\"event\":\"child_supervision_error\""));
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_cleanup_failure_keeps_durable_ownership() {
    let environment = test_environment();
    let fixture = GitFixture::new("cleanup-quarantine");
    let mut state = supervision_state(&fixture);
    let state_path = fixture.root.join("state/invocation.json");
    let event_log = fixture.root.join("log/executor.jsonl");
    let validated = detach_harness_for_adoption(
        &fixture,
        &state_path,
        &mut state,
        "printf 'progress\\n'; sleep 30",
    );
    let snapshot =
        MutationSnapshot::capture(&fixture.repo, &state.identity.branch).expect("snapshot");
    environment.launch(bridge::LaunchFailpoint::AdoptedPoll);
    environment.cleanup(bridge::LaunchFailpoint::CleanupSignal);
    let error = bridge::supervise_validated_harness(
        &state_path,
        &event_log,
        &mut state,
        &validated,
        &snapshot,
        supervision_config(500),
    )
    .expect_err("cleanup failure must quarantine");
    environment.launch(bridge::LaunchFailpoint::None);
    environment.cleanup(bridge::LaunchFailpoint::None);

    assert!(error.contains("cleanup"), "{error}");
    let durable = PersistedInvocation::from_json(
        &fs::read_to_string(&state_path).expect("durable quarantine"),
    )
    .expect("strict quarantine");
    assert_eq!(durable.phase, BridgePhase::Interrupted);
    assert!(durable.supervisor.is_some());
    assert!(durable.process.is_some());

    let cleanup = bridge::supervise_validated_harness(
        &state_path,
        &event_log,
        &mut state,
        &validated,
        &snapshot,
        supervision_config(100),
    )
    .expect("later exact cleanup succeeds");
    assert_eq!(cleanup, SupervisionOutcome::Stalled);
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_direct_poll_error_is_structured_and_cleared() {
    let environment = test_environment();
    for failpoint in [
        bridge::LaunchFailpoint::PostReturnIdentity,
        bridge::LaunchFailpoint::DirectSetup,
        bridge::LaunchFailpoint::DirectPoll,
    ] {
        let fixture = GitFixture::new(&format!("direct-error-{failpoint:?}"));
        let mut state = supervision_state(&fixture);
        let state_path = fixture.root.join("state/invocation.json");
        let event_log = fixture.root.join("log/executor.jsonl");
        let snapshot =
            MutationSnapshot::capture(&fixture.repo, &state.identity.branch).expect("snapshot");
        environment.launch(failpoint);
        let error = supervise_harness(
            &state_path,
            &event_log,
            &mut state,
            &shell_invocation(&fixture.repo, "printf 'progress\\n'; sleep 30"),
            &snapshot,
            supervision_config(500),
        )
        .expect_err("direct supervision failure");
        environment.launch(bridge::LaunchFailpoint::None);

        assert!(error.contains("direct-"), "{error}");
        assert!(state.supervisor.is_none());
        assert!(state.process.is_none());
        assert!(fs::read_to_string(event_log)
            .expect("direct structured failure")
            .contains("\"event\":\"child_supervision_error\""));
    }
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_parent_setup_failures_reap_supervisor_and_harness() {
    let environment = test_environment();
    for failpoint in [
        bridge::LaunchFailpoint::ParentAfterPidfd,
        bridge::LaunchFailpoint::ParentHarnessCapture,
        bridge::LaunchFailpoint::ParentBirthRefresh,
    ] {
        let fixture = GitFixture::new(&format!("parent-setup-{failpoint:?}"));
        let mut state = supervision_state(&fixture);
        let snapshot =
            MutationSnapshot::capture(&fixture.repo, &state.identity.branch).expect("snapshot");
        bridge::LAST_SPAWN_SUPERVISOR.store(0, Ordering::SeqCst);
        bridge::LAST_SPAWN_HARNESS.store(0, Ordering::SeqCst);
        environment.launch(failpoint);
        let error = supervise_harness(
            &fixture.root.join("state/invocation.json"),
            &fixture.root.join("log/executor.jsonl"),
            &mut state,
            &shell_invocation(&fixture.repo, "sleep 30"),
            &snapshot,
            supervision_config(500),
        )
        .expect_err("parent setup failpoint");
        environment.launch(bridge::LaunchFailpoint::None);
        assert!(error.contains("parent-"), "{error}");

        for pid in [
            bridge::LAST_SPAWN_SUPERVISOR.load(Ordering::SeqCst),
            bridge::LAST_SPAWN_HARNESS.load(Ordering::SeqCst),
        ] {
            if pid == 0 {
                continue;
            }
            for _ in 0..100 {
                if bridge::observe_process_birth(pid)
                    .expect("observe failed launch")
                    .is_none()
                {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            assert!(
                bridge::observe_process_birth(pid)
                    .expect("final failed launch observation")
                    .is_none(),
                "post-fork setup failure leaked PID {pid}"
            );
        }
    }
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_capture_failure_retries_interrupted_exact_reap() {
    let environment = test_environment();
    let fixture = GitFixture::new("capture-reap-interrupted");
    let mut state = supervision_state(&fixture);
    let state_path = fixture.root.join("state/invocation.json");
    let snapshot =
        MutationSnapshot::capture(&fixture.repo, &state.identity.branch).expect("snapshot");
    bridge::LAST_SPAWN_SUPERVISOR.store(0, Ordering::SeqCst);
    environment.parent_capture(true);
    environment.parent_reap(bridge::ParentReapFailpoint::InterruptedOnce);

    let error = supervise_harness(
        &state_path,
        &fixture.root.join("log/executor.jsonl"),
        &mut state,
        &shell_invocation(&fixture.repo, "sleep 30"),
        &snapshot,
        supervision_config(100),
    )
    .expect_err("capture failure after fork");
    environment.parent_capture(false);
    environment.parent_reap(bridge::ParentReapFailpoint::None);

    let supervisor = bridge::LAST_SPAWN_SUPERVISOR.load(Ordering::SeqCst);
    assert!(error.contains("capture"), "{error}");
    assert_eq!(state.phase, BridgePhase::Interrupted);
    assert!(state.supervisor.is_none());
    assert!(state.process.is_none());
    assert!(
        bridge::observe_process_birth(supervisor)
            .expect("observe reaped supervisor")
            .is_none(),
        "EINTR retry did not reap the exact direct child"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_capture_and_reap_failure_retains_exact_quarantine() {
    let environment = test_environment();
    nix::sys::prctl::set_child_subreaper(false).expect("clear fixture subreaper");
    let fixture = GitFixture::new("capture-reap-quarantine");
    let mut state = supervision_state(&fixture);
    let state_path = fixture.root.join("state/invocation.json");
    let launches = fixture.root.join("launches");
    let snapshot =
        MutationSnapshot::capture(&fixture.repo, &state.identity.branch).expect("snapshot");
    bridge::LAST_SPAWN_SUPERVISOR.store(0, Ordering::SeqCst);
    environment.parent_capture(true);
    environment.parent_reap(bridge::ParentReapFailpoint::Failure);

    let error = supervise_harness(
        &state_path,
        &fixture.root.join("log/executor.jsonl"),
        &mut state,
        &shell_invocation(
            &fixture.repo,
            &format!("printf launched > '{}'", launches.display()),
        ),
        &snapshot,
        supervision_config(100),
    )
    .expect_err("capture plus exact reap failure");
    let durable = PersistedInvocation::from_json(
        &fs::read_to_string(&state_path).expect("durable capture quarantine"),
    )
    .expect("strict capture quarantine");
    let supervisor = bridge::LAST_SPAWN_SUPERVISOR.load(Ordering::SeqCst);
    let mut subreaper = 0_i32;
    // SECURITY-REVIEW: independent #2598 reviewer LGTM; read-only process-state probe.
    // SAFETY: PR_GET_CHILD_SUBREAPER writes one integer to the supplied valid pointer.
    let get_result = unsafe {
        nix::libc::prctl(
            nix::libc::PR_GET_CHILD_SUBREAPER,
            std::ptr::addr_of_mut!(subreaper),
            0,
            0,
            0,
        )
    };

    environment.parent_capture(false);
    environment.parent_reap(bridge::ParentReapFailpoint::None);
    // Bounded: see reap_fixture_child_within. An unbounded wait here is what turned a wrong
    // PID into a suite-wide hang (#2981), and this was the only reap in the tree without a
    // bound. The result is deliberately ignored — the assertions below are the real check.
    let _ = reap_fixture_child_within(supervisor, Duration::from_secs(10));
    nix::sys::prctl::set_child_subreaper(false).expect("restore fixture subreaper");

    assert!(error.contains("quarantin"), "{error}");
    assert_eq!(durable.phase, BridgePhase::Interrupted);
    assert_eq!(
        durable
            .supervisor
            .as_ref()
            .expect("durable exact supervisor")
            .pid,
        supervisor
    );
    assert!(durable.process.is_none());
    assert_eq!(get_result, 0);
    assert_eq!(
        subreaper, 1,
        "unproven exact reap restored subreaper ownership"
    );
    assert!(!launches.exists(), "capture failure released the harness");
    assert!(
        bridge::observe_process_birth(supervisor)
            .expect("final supervisor observation")
            .is_none(),
        "fixture cleanup left the supervisor"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_parent_cleanup_failure_persists_quarantine() {
    let environment = test_environment();
    for (parent_failpoint, harness_identity_expected) in [
        (bridge::LaunchFailpoint::ParentHarnessPidRead, false),
        (bridge::LaunchFailpoint::ParentHarnessBirth, false),
        (bridge::LaunchFailpoint::ParentHarnessPidfd, true),
        (bridge::LaunchFailpoint::ParentReadiness, true),
        (bridge::LaunchFailpoint::JournalCreate, true),
        (bridge::LaunchFailpoint::JournalWrite, true),
        (bridge::LaunchFailpoint::JournalSync, true),
        (bridge::LaunchFailpoint::JournalRename, true),
        (bridge::LaunchFailpoint::JournalDirectorySync, true),
    ] {
        for cleanup_failpoint in [
            bridge::LaunchFailpoint::CleanupSignal,
            bridge::LaunchFailpoint::CleanupLiveness,
        ] {
            let fixture = GitFixture::new(&format!(
                "parent-quarantine-{parent_failpoint:?}-{cleanup_failpoint:?}"
            ));
            let mut state = supervision_state(&fixture);
            let state_path = fixture.root.join("state/invocation.json");
            let event_log = fixture.root.join("log/executor.jsonl");
            let snapshot =
                MutationSnapshot::capture(&fixture.repo, &state.identity.branch).expect("snapshot");
            bridge::LAST_SPAWN_SUPERVISOR.store(0, Ordering::SeqCst);
            environment.launch(parent_failpoint);
            environment.cleanup(cleanup_failpoint);
            let error = supervise_harness(
                &state_path,
                &event_log,
                &mut state,
                &shell_invocation(&fixture.repo, "sleep 30"),
                &snapshot,
                supervision_config(100),
            )
            .expect_err("parent setup cleanup failure");
            let durable = PersistedInvocation::from_json(
                &fs::read_to_string(&state_path).expect("durable parent quarantine"),
            )
            .expect("strict parent quarantine");
            environment.launch(bridge::LaunchFailpoint::None);
            environment.cleanup(bridge::LaunchFailpoint::None);

            let supervisor_pid = bridge::LAST_SPAWN_SUPERVISOR.load(Ordering::SeqCst);
            if supervisor_pid != 0
                && bridge::observe_process_birth(supervisor_pid)
                    .expect("observe RED supervisor")
                    .is_some()
            {
                let mut owned = bridge::OwnedProcessSet::from_forked_child(supervisor_pid)
                    .expect("capture RED supervisor");
                owned.terminate().expect("clean RED supervisor tree");
            }

            assert!(error.contains("cleanup"), "{error}");
            assert_eq!(durable.phase, BridgePhase::Interrupted);
            assert!(
                durable.supervisor.is_some(),
                "cleanup failure erased the captured supervisor identity"
            );
            assert_eq!(
                durable.process.is_some(),
                harness_identity_expected,
                "cleanup failure persisted the wrong captured harness identity at {parent_failpoint:?}"
            );
            assert!(fs::read_to_string(event_log)
                .expect("parent cleanup event")
                .contains("\"event\":\"child_supervision_error\""));
        }
    }
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_fixture_cleanup_is_armed_before_identity_observation_panics() {
    let _environment = test_environment();
    let mut child = Command::new("/bin/sh")
        .args(["-c", "sleep 30 & wait"])
        .spawn()
        .expect("spawn unstable-observation fixture");
    let pid = child.id();
    let cleanup = DetachedForkedCleanup::new(pid).expect("arm immediate exact-child cleanup");
    let wrong_args = vec!["-c".to_string(), "identity-never-matches".to_string()];

    let panic = std::panic::catch_unwind(|| {
        let _ = observe_spawned_identity(pid, &wrong_args);
    });
    assert!(panic.is_err(), "fixture observation unexpectedly succeeded");
    drop(cleanup);
    let _ = child.wait();

    for _ in 0..100 {
        if bridge::observe_process_birth(pid)
            .expect("observe fixture after panic")
            .is_none()
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        bridge::observe_process_birth(pid)
            .expect("final fixture observation")
            .is_none(),
        "observer panic leaked the spawned fixture"
    );
}

/// A fault armed by a test that dies must not outlive it.
///
/// Arming is a bare store, so before the guard reset, a test panicking between arming and
/// disarming left the fault set for whatever launched next — surfacing as that test's bug, in
/// another file, with nothing connecting the two. This holds no guard itself: the panicking
/// closure takes it, and the mutex is not reentrant.
#[test]
fn autonomous_executor_bridge_environment_guard_disarms_after_a_panic() {
    let none = bridge::LaunchFailpoint::None as u8;
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let outcome = std::panic::catch_unwind(|| {
        let environment = test_environment();
        environment.launch(bridge::LaunchFailpoint::ParentAfterPidfd);
        panic!("deliberate: dies between arming and disarming");
    });
    std::panic::set_hook(previous);

    assert!(outcome.is_err(), "the closure must actually unwind");
    let _environment = test_environment();
    assert_eq!(
        bridge::LAUNCH_FAILPOINT.load(Ordering::SeqCst),
        none,
        "a panicking armer left its failpoint set"
    );
}

/// Arming must go through the guard, and that has to be checked rather than trusted.
///
/// The methods on TestEnvironment cannot be reached without holding the mutex, but the free
/// `set_*_failpoint` functions are still visible to every test module — a descendant can name
/// an ancestor's private items. Eight tests already called them directly (#2989) and one hung
/// the whole suite. This fails the moment a ninth does.
#[test]
fn autonomous_executor_bridge_failpoints_are_armed_only_through_the_guard() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/commands/autonomous/executor_bridge/tests");
    let mut offenders = Vec::new();
    for entry in fs::read_dir(&root).expect("read test modules") {
        let path = entry.expect("test module entry").path();
        if path.file_name().is_some_and(|name| name == "support_base.rs") {
            continue; // the guard methods themselves live here
        }
        let body = fs::read_to_string(&path).expect("read test module");
        for (number, line) in body.lines().enumerate() {
            if line.contains(concat!("bridge::", "set_")) && line.contains("failpoint(") {
                offenders.push(format!("{}:{}", path.display(), number + 1));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "arm through the environment guard, not the free setter: {offenders:?}"
    );
}
