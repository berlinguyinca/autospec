// executor_bridge tests: descendant / spawn — 13 cases.
//
// Split out of tests.rs; see the note in that file.

use super::super::{
    supervise_harness, BridgePhase, HarnessInvocation, MutationSnapshot, PersistedInvocation,
    ProcessIdentity, SupervisionOutcome,
};
use super::support_base::{test_environment, GitFixture};
use super::support_invocation::{
    detach_harness_for_adoption, persisted_invocation, shell_invocation, supervision_config,
    supervision_state,
};
use crate::commands::autonomous::executor_bridge as bridge;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_releases_fork_serialization_after_exact_exec() { // linter:allow-SECURITY test fn name ends in _exec( — Rust test function, not a builtin call
    let _environment = test_environment();
    let fixture = GitFixture::new("fork-lock-release-after-exec");
    let invocation = shell_invocation(&fixture.repo, "exec /usr/bin/sleep 30");
    let validated = bridge::validate_invocation(
        &HarnessInvocation {
            program: invocation.program.canonicalize().expect("canonical shell"),
            supervised_executable: invocation
                .program
                .canonicalize()
                .expect("canonical supervised shell"),
            args: invocation.args,
            current_dir: invocation
                .current_dir
                .canonicalize()
                .expect("canonical fixture repo"),
            requires_mutation_snapshots: false,
        },
        &fixture.repo.canonicalize().expect("canonical fixture repo"),
    )
    .expect("validate sleeping harness");
    let sinks = bridge::output_sink_paths(
        &fixture.root.join("state/invocation.json"),
        "fork-lock-release",
    )
    .expect("output sinks");
    let mut child =
        bridge::spawn_blocked_harness(&validated, &sinks, None).expect("spawn sleeping harness");

    child.release_launch_barrier().expect("release harness");
    assert!(
        bridge::test_fork_lifecycle_is_available(),
        "an execed harness must not serialize unrelated test forks until process exit"
    );
    child.terminate().expect("terminate sleeping harness");
}

#[test]
fn autonomous_executor_bridge_pending_and_interrupted_phases_round_trip_nonterminally() {
    for phase in [BridgePhase::Pending, BridgePhase::Interrupted] {
        let mut expected = persisted_invocation();
        expected.phase = phase;
        expected.process = None;
        expected.terminal_result = None;
        let recovered =
            PersistedInvocation::from_json(&expected.to_json().expect("serialize phase"))
                .expect("recover nonterminal phase");
        assert_eq!(recovered.phase, phase);
        assert!(recovered.terminal_result.is_none());
    }
}

#[test]
fn autonomous_executor_bridge_spawn_state_failure_cleans_owned_child() {
    let environment = test_environment();
    let fixture = GitFixture::new("supervise-state-failure");
    let mut state = supervision_state(&fixture);
    let snapshot =
        MutationSnapshot::capture(&fixture.repo, &state.identity.branch).expect("snapshot");
    let child_pid = fixture.root.join("child.pid");
    let invocation = shell_invocation(
        &fixture.repo,
        &format!(
            "printf '%s\\n' \"$$\" > '{}'; sleep 30",
            child_pid.display()
        ),
    );

    environment.launch(bridge::LaunchFailpoint::PersistAfterSpawn);
    let error = supervise_harness(
        &fixture.root.join("state/invocation.json"),
        &fixture.root.join("log/executor.jsonl"),
        &mut state,
        &invocation,
        &snapshot,
        supervision_config(2_000),
    )
    .expect_err("injected state persistence failure");
    environment.launch(bridge::LaunchFailpoint::None);

    assert!(error.contains("injected"));
    assert!(
        !child_pid.exists(),
        "barrier released before durable process identity"
    );
}

#[test]
fn autonomous_executor_bridge_spawn_log_failure_cleans_without_releasing_child() {
    let environment = test_environment();
    let fixture = GitFixture::new("supervise-log-failure");
    let mut state = supervision_state(&fixture);
    let snapshot =
        MutationSnapshot::capture(&fixture.repo, &state.identity.branch).expect("snapshot");
    let child_pid = fixture.root.join("child.pid");
    let invocation = shell_invocation(
        &fixture.repo,
        &format!(
            "printf '%s\\n' \"$$\" > '{}'; sleep 30",
            child_pid.display()
        ),
    );

    environment.launch(bridge::LaunchFailpoint::LogAfterSpawn);
    let error = supervise_harness(
        &fixture.root.join("state/invocation.json"),
        &fixture.root.join("log/executor.jsonl"),
        &mut state,
        &invocation,
        &snapshot,
        supervision_config(2_000),
    )
    .expect_err("injected log failure");
    environment.launch(bridge::LaunchFailpoint::None);

    assert!(error.contains("injected"));
    assert!(!child_pid.exists(), "barrier released after log failure");
    assert_eq!(state.phase, BridgePhase::Interrupted);
    assert!(
        state.process.is_none(),
        "proven cleanup retained stale harness ownership"
    );
    assert!(
        state.supervisor.is_none(),
        "proven cleanup retained stale supervisor ownership"
    );
}

#[test]
fn autonomous_executor_bridge_never_ready_handshake_times_out_and_reaps() {
    let environment = test_environment();
    // Break caught: a post-fork child hanging before readiness blocks the conductor forever.
    let fixture = GitFixture::new("supervise-never-ready");
    let mut state = supervision_state(&fixture);
    let snapshot =
        MutationSnapshot::capture(&fixture.repo, &state.identity.branch).expect("snapshot");
    let started = Instant::now();

    environment.launch(bridge::LaunchFailpoint::NeverReady);
    let error = supervise_harness(
        &fixture.root.join("state/invocation.json"),
        &fixture.root.join("log/executor.jsonl"),
        &mut state,
        &shell_invocation(&fixture.repo, "sleep 30"),
        &snapshot,
        supervision_config(2_000),
    )
    .expect_err("never-ready child must time out");
    environment.launch(bridge::LaunchFailpoint::None);

    assert!(
        error.contains("readiness timeout"),
        "unexpected error: {error}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "ready handshake exceeded its test deadline"
    );
}

#[test]
fn autonomous_executor_bridge_never_close_exec_status_times_out_and_reaps() {
    let environment = test_environment();
    // Break caught: a post-barrier child retaining the exec-status descriptor blocks forever.
    let fixture = GitFixture::new("supervise-never-exec");
    let mut state = supervision_state(&fixture);
    let snapshot =
        MutationSnapshot::capture(&fixture.repo, &state.identity.branch).expect("snapshot");
    let state_path = fixture.root.join("state/invocation.json");
    let started = Instant::now();

    environment.launch(bridge::LaunchFailpoint::NeverCloseExecStatus);
    let error = supervise_harness(
        &state_path,
        &fixture.root.join("log/executor.jsonl"),
        &mut state,
        &shell_invocation(&fixture.repo, "sleep 30"),
        &snapshot,
        supervision_config(2_000),
    )
    .expect_err("never-closing exec status must time out");
    environment.launch(bridge::LaunchFailpoint::None);

    assert!(
        error.contains("exec-status timeout"),
        "unexpected error: {error}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "exec-status handshake exceeded its test deadline"
    );
    let persisted = PersistedInvocation::from_json(
        &fs::read_to_string(state_path).expect("persisted timeout state"),
    )
    .expect("strict timeout state");
    assert_eq!(persisted.phase, BridgePhase::Interrupted);
    assert!(persisted.process.is_none());
}

#[test]
fn autonomous_executor_bridge_fast_exit_is_observed_after_durable_handshake() {
    let _environment = test_environment();
    let fixture = GitFixture::new("supervise-fast-exit");
    let mut state = supervision_state(&fixture);
    let snapshot =
        MutationSnapshot::capture(&fixture.repo, &state.identity.branch).expect("snapshot");

    let outcome = supervise_harness(
        &fixture.root.join("state/invocation.json"),
        &fixture.root.join("log/executor.jsonl"),
        &mut state,
        &shell_invocation(&fixture.repo, "exit 0"),
        &snapshot,
        supervision_config(2_000),
    )
    .expect("fast child remains attributable");

    assert_eq!(outcome, SupervisionOutcome::Exited { exit_code: 0 });
    let events = fs::read_to_string(fixture.root.join("log/executor.jsonl")).expect("events");
    assert!(events.contains("\"event\":\"child_started\""));
    assert!(events.contains("\"event\":\"child_exited\""));
}

#[test]
fn autonomous_executor_bridge_process_only_restart_proves_and_cleans_parent_supervisor() {
    let _environment = test_environment();
    let fixture = GitFixture::new("supervise-legacy-parent");
    let mut state = supervision_state(&fixture);
    let state_path = fixture.root.join("state/invocation.json");
    let event_log = fixture.root.join("log/executor.jsonl");
    let validated = detach_harness_for_adoption(
        &fixture,
        &state_path,
        &mut state,
        "exec >/dev/null 2>&1; while :; do sleep 1; done",
    );
    let supervisor_pid = state.supervisor.as_ref().expect("supervisor").pid;
    state.supervisor = None;
    bridge::write_invocation_atomic(&state_path, &state).expect("persist schema-1 state");
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
    .expect("schema-1 restart adopts the proven stable supervisor");

    assert_eq!(outcome, SupervisionOutcome::Stalled);
    assert!(
        bridge::observe_process_birth(supervisor_pid)
            .expect("observe cleaned supervisor")
            .is_none(),
        "legacy parent supervisor survived restart cleanup"
    );
}

#[test]
fn autonomous_executor_bridge_dead_legacy_recovery_retains_permanent_quarantine() {
    let _environment = test_environment();
    let fixture = GitFixture::new("supervise-dead-recovery");
    let mut state = supervision_state(&fixture);
    state.phase = BridgePhase::Implementing;
    state.process = Some(ProcessIdentity {
        pid: u32::MAX - 1,
        process_group: u32::MAX - 1,
        executable: PathBuf::from("/bin/sh"),
        argv_digest: "a".repeat(64),
        boot_id: fs::read_to_string("/proc/sys/kernel/random/boot_id")
            .expect("boot id")
            .trim()
            .to_string(),
        start_identity: "1".to_string(),
    });
    let snapshot =
        MutationSnapshot::capture(&fixture.repo, &state.identity.branch).expect("snapshot");
    let launches = fixture.root.join("unexpected-launch");

    let error = supervise_harness(
        &fixture.root.join("state/invocation.json"),
        &fixture.root.join("log/executor.jsonl"),
        &mut state,
        &shell_invocation(
            &fixture.repo,
            &format!("printf launched > '{}'", launches.display()),
        ),
        &snapshot,
        supervision_config(500),
    )
    .expect_err("dead recovery must stop at the local-classification boundary");

    assert!(error.contains("quarantin"), "{error}");
    assert_eq!(state.phase, BridgePhase::Interrupted);
    assert!(state.process.is_some());
    assert!(!launches.exists(), "dead child was blindly relaunched");
}

#[test]
fn autonomous_executor_bridge_inherited_pipe_writer_blocks_success_and_is_cleaned() {
    let _environment = test_environment();
    let fixture = GitFixture::new("supervise-daemon-success");
    let mut state = supervision_state(&fixture);
    let snapshot =
        MutationSnapshot::capture(&fixture.repo, &state.identity.branch).expect("snapshot");
    let descendant_pid = fixture.root.join("descendant.pid");
    let script = format!(
        "sleep 30 & child=$!; printf '%s\\n' \"$child\" > '{}'; exit 0",
        descendant_pid.display()
    );

    let error = supervise_harness(
        &fixture.root.join("state/invocation.json"),
        &fixture.root.join("log/executor.jsonl"),
        &mut state,
        &shell_invocation(&fixture.repo, &script),
        &snapshot,
        supervision_config(2_000),
    )
    .expect_err("retained output writer must block terminal success");

    assert!(error.contains("output-complete timeout"), "{error}");
    assert_eq!(state.phase, BridgePhase::Interrupted);
    let descendant = fs::read_to_string(descendant_pid)
        .expect("descendant pid")
        .trim()
        .to_string();
    assert!(
        !Path::new(&format!("/proc/{descendant}")).exists(),
        "background descendant survived successful leader exit"
    );
    let events = fs::read_to_string(fixture.root.join("log/executor.jsonl")).expect("events");
    assert!(
        events.contains("\"event\":\"child_supervision_error\""),
        "{events}"
    );
}

#[test]
fn autonomous_executor_bridge_closed_stdio_descendant_blocks_terminal_success() {
    let _environment = test_environment();
    let fixture = GitFixture::new("supervise-closed-stdio-descendant");
    let mut state = supervision_state(&fixture);
    let snapshot =
        MutationSnapshot::capture(&fixture.repo, &state.identity.branch).expect("snapshot");
    let descendant_pid = fixture.root.join("descendant.pid");
    let script = format!(
        "sleep 30 >/dev/null 2>&1 & child=$!; printf '%s\\n' \"$child\" > '{}'; exit 0",
        descendant_pid.display()
    );

    let error = supervise_harness(
        &fixture.root.join("state/invocation.json"),
        &fixture.root.join("log/executor.jsonl"),
        &mut state,
        &shell_invocation(&fixture.repo, &script),
        &snapshot,
        supervision_config(2_000),
    )
    .expect_err("closed-stdio descendant must block whole-tree completion");

    assert!(error.contains("output-complete timeout"), "{error}");
    let descendant = fs::read_to_string(descendant_pid)
        .expect("closed-stdio descendant pid")
        .trim()
        .to_string();
    assert!(!Path::new(&format!("/proc/{descendant}")).exists());
    let events = fs::read_to_string(fixture.root.join("log/executor.jsonl")).expect("events");
    assert!(!events.contains("\"event\":\"child_exited\""), "{events}");
}

#[test]
fn autonomous_executor_bridge_bounds_oversized_unterminated_output() {
    let _environment = test_environment();
    let fixture = GitFixture::new("supervise-bounded-output");
    let mut state = supervision_state(&fixture);
    let snapshot =
        MutationSnapshot::capture(&fixture.repo, &state.identity.branch).expect("snapshot");

    let outcome = supervise_harness(
        &fixture.root.join("state/invocation.json"),
        &fixture.root.join("log/executor.jsonl"),
        &mut state,
        &shell_invocation(&fixture.repo, "head -c 131072 /dev/zero | tr '\\0' x"),
        &snapshot,
        supervision_config(2_000),
    )
    .expect("bounded output supervision");

    assert_eq!(outcome, SupervisionOutcome::Exited { exit_code: 0 });
    let events = fs::read_to_string(fixture.root.join("log/executor.jsonl")).expect("events");
    assert!(
        events.len() < 32_768,
        "event log was not bounded: {}",
        events.len()
    );
    assert!(events.contains("\"truncated\":true"), "{events}");
}

#[test]
fn autonomous_executor_bridge_failing_leader_cleans_background_descendant() {
    let _environment = test_environment();
    let fixture = GitFixture::new("supervise-daemon-failure");
    let mut state = supervision_state(&fixture);
    let snapshot =
        MutationSnapshot::capture(&fixture.repo, &state.identity.branch).expect("snapshot");
    let descendant_pid = fixture.root.join("descendant.pid");

    let error = supervise_harness(
        &fixture.root.join("state/invocation.json"),
        &fixture.root.join("log/executor.jsonl"),
        &mut state,
        &shell_invocation(
            &fixture.repo,
            &format!(
                "sleep 30 & child=$!; printf '%s\\n' \"$child\" > '{}'; exit 7",
                descendant_pid.display()
            ),
        ),
        &snapshot,
        supervision_config(2_000),
    )
    .expect_err("prompt-only failing leader is not durable terminal authority");

    assert!(error.contains("prompt-only exit code 7"), "{error}");
    assert_eq!(state.phase, BridgePhase::Interrupted);
    let descendant = fs::read_to_string(descendant_pid)
        .expect("descendant pid")
        .trim()
        .to_string();
    assert!(!Path::new(&format!("/proc/{descendant}")).exists());
    let events = fs::read_to_string(fixture.root.join("log/executor.jsonl")).expect("events");
    assert!(
        events.contains("\"event\":\"child_supervision_error\""),
        "{events}"
    );
    assert!(!events.contains("\"event\":\"child_exited\""), "{events}");
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_freeze_captures_descendant_forked_in_cleanup_window() {
    let environment = test_environment();
    let fixture = GitFixture::new("cleanup-fork-race");
    let mut state = supervision_state(&fixture);
    let snapshot =
        MutationSnapshot::capture(&fixture.repo, &state.identity.branch).expect("snapshot");
    let late_pid = fixture.root.join("late-descendant.pid");
    let script = format!(
        "while :; do state=$(sed -E 's/^[0-9]+ \\(.*\\) ([A-Z]).*/\\1/' /proc/$PPID/stat); if [ \"$state\" = T ]; then sleep 30 & printf '%s\\n' \"$!\" > '{}'; break; fi; sleep 0.001; done; while :; do sleep 1; done",
        late_pid.display()
    );

    environment.cleanup(bridge::LaunchFailpoint::CleanupFreezeWindow);
    let outcome = supervise_harness(
        &fixture.root.join("state/invocation.json"),
        &fixture.root.join("log/executor.jsonl"),
        &mut state,
        &shell_invocation(&fixture.repo, &script),
        &snapshot,
        supervision_config(100),
    )
    .expect("stalled harness cleanup");
    environment.cleanup(bridge::LaunchFailpoint::None);

    assert_eq!(outcome, SupervisionOutcome::Stalled);
    assert!(
        late_pid.exists(),
        "fixture never synchronized a fork into the frozen cleanup window"
    );
    let pid = fs::read_to_string(late_pid)
        .expect("late descendant PID")
        .trim()
        .to_string();
    assert!(
        !Path::new(&format!("/proc/{pid}")).exists(),
        "descendant forked during cleanup survived"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn executor_supervision_descendant_capture_reserves_descriptor_headroom() {
    // Break caught: a wide real process tree exhausted RLIMIT_NOFILE while descendant pidfds
    // were retained, so the fail-closed path itself had no descriptor left. Capture must stop
    // with the reserve still free instead of walking into EMFILE.
    let _environment = test_environment();
    let mut leader = std::process::Command::new("/bin/sh")
        .args([
            "-c",
            "for _ in 1 2 3 4 5 6 7 8 9 10 11 12; do /bin/sleep 30 & done; wait",
        ])
        .spawn()
        .expect("spawn descendant tree leader");
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut observed = 0usize;
    while Instant::now() < deadline {
        observed = bridge::process_table_entries()
            .expect("read process table")
            .into_iter()
            .filter(|(parent, _)| *parent == leader.id())
            .count();
        if observed >= 12 {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(observed >= 12, "descendant tree never materialized: {observed}");

    let mut set =
        bridge::OwnedProcessSet::from_forked_child(leader.id()).expect("capture tree leader");
    let ceiling = bridge::open_descriptor_count().expect("open descriptor count") + 36;
    bridge::set_descriptor_limit_override(ceiling);
    let constrained = set.capture_descendants_while_leader_live();
    let free = bridge::free_descriptor_slots().expect("free descriptor slots");
    bridge::set_descriptor_limit_override(0);

    let error = constrained.expect_err("a constrained descriptor budget must fail closed");
    assert!(error.contains("descriptor budget"), "{error}");
    assert!(
        free >= 32,
        "descendant capture must leave at least 32 descriptor slots free, saw {free}"
    );

    let mut guard = bridge::AdoptedProcessGuard::new(set);
    guard
        .processes_mut()
        .capture_descendants_while_leader_live()
        .expect("recapture the unconstrained tree");
    guard.terminate().expect("terminate the descendant tree");
    let _ = leader.wait();
}
