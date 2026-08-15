// executor_bridge tests: attempt / retirement — 9 cases.
//
// Split out of tests.rs; see the note in that file.

use super::super::HarnessInvocation;
use super::support_base::{test_environment, GitFixture};
use super::support_invocation::shell_invocation;
use super::support_launch::direct_launch_supervisor_pid;
use crate::commands::autonomous::executor_bridge as bridge;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::Duration;

#[test]
fn autonomous_executor_bridge_runtime_infrastructure_error_is_terminal_evidence() {
    let _environment = test_environment();
    // Break caught: a runtime adapter error returning before command-000.json is persisted.
    let fixture = GitFixture::new("runtime-terminal-error");
    let artifact_root = fixture.root.join("evidence");
    let runtime = bridge::DirectRuntimeAdapter {
        repo: fixture.repo.clone(),
        session_id: "closed-runtime-session".into(),
        environment_dir: fixture.root.join("runtime"),
        session: std::cell::RefCell::new(None),
    };
    let plan = bridge::parse_direct_command_plan("/usr/bin/true").expect("direct plan");

    let error = bridge::execute_direct_plan(
        &fixture.repo,
        &plan,
        &artifact_root,
        Some(&runtime),
        Duration::from_secs(5),
    )
    .expect_err("closed runtime must be typed infrastructure evidence");
    let record: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(artifact_root.join("command-000.json"))
            .expect("terminal infrastructure record"),
    )
    .expect("typed command record");

    assert!(error.contains("infrastructure"), "{error}");
    assert_eq!(record["terminal"]["kind"], "infrastructure_failed");
    assert!(artifact_root.join("command-000.intent.json").is_file());
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_retries_repaired_supervisor_resolution_failure() {
    let environment = test_environment();
    let fixture = GitFixture::new("direct-repaired-supervisor-resolution");
    let artifact_root = fixture.root.join("evidence");
    let plan = bridge::parse_direct_command_plan("/usr/bin/true").expect("direct plan");

    environment.launch(bridge::LaunchFailpoint::ParentReadiness);
    bridge::execute_direct_plan(
        &fixture.repo,
        &plan,
        &artifact_root,
        None,
        Duration::from_secs(5),
    )
    .expect_err("first attempt records cleanup-proven infrastructure failure");
    environment.launch(bridge::LaunchFailpoint::None);
    let record_path = artifact_root.join("command-000.json");
    let observed = bridge::read_observed_command_record(&fixture.repo, &record_path)
        .expect("read infrastructure record");
    let repaired = bridge::observed_command_document(
        &observed.attempt_id,
        &observed.commit_oid,
        observed.runtime_session_id.as_deref(),
        &observed.executable,
        &observed.argv,
        &observed.process_executable,
        &observed.process_argv,
        &bridge::AttemptTerminal::InfrastructureFailed(
            "canonicalize executor supervisor executable: No such file or directory (os error 2)"
                .to_string(),
        ),
        &observed.stdout_path,
        &observed.stdout_digest,
        &observed.stderr_path,
        &observed.stderr_digest,
    );
    fs::write(&record_path, repaired).expect("persist repaired failure fixture");

    let recovered = bridge::execute_direct_plan(
        &fixture.repo,
        &plan,
        &artifact_root,
        None,
        Duration::from_secs(5),
    )
    .expect("healthy supervisor resolution starts a fresh attempt");

    assert_eq!(recovered[0].terminal, bridge::AttemptTerminal::Exited(0));
    assert!(fs::read_dir(&artifact_root)
        .expect("failure archive")
        .flatten()
        .any(|entry| entry.file_name().to_string_lossy().contains(".archive-")));
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_cleanup_failure_retains_identity_until_restart_reconciles() {
    let environment = test_environment();
    let fixture = GitFixture::new("direct-cleanup-quarantine");
    let artifact_root = fixture.root.join("evidence");
    let plan = bridge::parse_direct_command_plan("/usr/bin/true").expect("direct plan");

    environment.cleanup(bridge::LaunchFailpoint::CleanupSignal);
    let first = bridge::execute_direct_plan(
        &fixture.repo,
        &plan,
        &artifact_root,
        None,
        Duration::from_secs(5),
    );
    environment.cleanup(bridge::LaunchFailpoint::None);
    let first = first.expect_err("failed cleanup must be typed and quarantined");
    let launch = artifact_root.join("command-000.launch.json");
    let supervisor = direct_launch_supervisor_pid(&launch);
    let harness = serde_json::from_str::<serde_json::Value>(
        &fs::read_to_string(&launch).expect("durable direct launch identity"),
    )
    .expect("direct launch JSON")["process"]["pid"]
        .as_u64()
        .and_then(|pid| u32::try_from(pid).ok())
        .expect("direct launch harness PID");
    assert!(first.contains("cleanup"), "{first}");
    assert!(Path::new(&format!("/proc/{supervisor}")).exists());

    let retry = bridge::execute_direct_plan(
        &fixture.repo,
        &plan,
        &artifact_root,
        None,
        Duration::from_secs(5),
    )
    .expect("reconciled cleanup starts a fresh attempt");

    assert_eq!(retry[0].terminal, bridge::AttemptTerminal::Exited(0));
    assert!(!launch.exists());
    assert!(!Path::new(&format!("/proc/{supervisor}")).exists());
    assert!(!Path::new(&format!("/proc/{harness}")).exists());
    assert!(fs::read_dir(&artifact_root)
        .expect("cleanup archives")
        .flatten()
        .any(|entry| entry.file_name().to_string_lossy().contains(".archive-")));
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_resumes_failure_archive_before_one_fresh_attempt() {
    let environment = test_environment();
    for boundary in [
        bridge::LaunchFailpoint::ArchiveAfterManifest,
        bridge::LaunchFailpoint::ArchiveMidMove,
        bridge::LaunchFailpoint::ArchiveBeforeComplete,
    ] {
        let fixture = GitFixture::new(&format!("direct-archive-{boundary:?}"));
        let artifact_root = fixture.root.join("evidence");
        let plan = bridge::parse_direct_command_plan("/usr/bin/true").expect("direct plan");
        environment.cleanup(bridge::LaunchFailpoint::CleanupSignal);
        let first = bridge::execute_direct_plan(
            &fixture.repo,
            &plan,
            &artifact_root,
            None,
            Duration::from_secs(5),
        );
        environment.cleanup(bridge::LaunchFailpoint::None);
        first.expect_err("first attempt leaves cleanup quarantine");

        environment.launch(boundary);
        let interrupted = bridge::execute_direct_plan(
            &fixture.repo,
            &plan,
            &artifact_root,
            None,
            Duration::from_secs(5),
        );
        environment.launch(bridge::LaunchFailpoint::None);
        interrupted.expect_err("archive transaction failpoint interrupts rollover");

        let recovered = bridge::execute_direct_plan(
            &fixture.repo,
            &plan,
            &artifact_root,
            None,
            Duration::from_secs(5),
        )
        .expect("restart completes archive and runs one fresh attempt");
        assert_eq!(recovered[0].terminal, bridge::AttemptTerminal::Exited(0));
        let archives = fs::read_dir(&artifact_root)
            .expect("archive directory")
            .flatten()
            .filter(|entry| entry.file_name().to_string_lossy().contains(".archive-"))
            .collect::<Vec<_>>();
        assert_eq!(
            archives.len(),
            1,
            "one immutable archive per failed attempt"
        );
        assert!(archives[0].path().join("complete").is_file());
        assert!(archives[0].path().join("command-000.json").is_file());
        assert!(!artifact_root.join("command-000.archive.pending").exists());
    }
}

#[test]
fn autonomous_executor_bridge_retirement_resumes_every_delete_boundary() {
    let environment = test_environment();
    // Break caught: a crash after cleanup proof deleting launch ownership without leaving a
    // durable transaction that restart can finish.
    for boundary in [
        bridge::LaunchFailpoint::RetireAfterProof,
        bridge::LaunchFailpoint::RetireMidDelete,
        bridge::LaunchFailpoint::RetireAfterLaunchDelete,
    ] {
        let fixture = GitFixture::new(&format!("direct-retire-{boundary:?}"));
        let artifact_root = fixture.root.join("evidence");
        fs::create_dir_all(&artifact_root).expect("artifact root");
        fs::set_permissions(&artifact_root, fs::Permissions::from_mode(0o700))
            .expect("private artifact root");
        let paths = bridge::direct_attempt_paths(&artifact_root, 0);
        for path in [
            &paths.launch,
            &paths.sinks.supervisor_identity,
            &paths.sinks.stdout,
            &paths.sinks.stderr,
            &paths.sinks.stdout_writer_cursor,
            &paths.sinks.stderr_writer_cursor,
            &paths.sinks.stdout_reader_cursor,
            &paths.sinks.stderr_reader_cursor,
            &paths.sinks.exit_status,
        ] {
            fs::write(path, b"owned\n").expect("retirement artifact");
            fs::set_permissions(path, fs::Permissions::from_mode(0o600))
                .expect("private retirement artifact");
        }

        environment.launch(boundary);
        let attempt_id = bridge::new_direct_attempt_id_candidate().expect("attempt id");
        let interrupted = bridge::retire_direct_launch(&paths, &attempt_id);
        environment.launch(bridge::LaunchFailpoint::None);
        interrupted.expect_err("retirement failpoint must interrupt transaction");
        bridge::retire_direct_launch(&paths, &attempt_id).expect("restart resumes retirement");

        assert!(!paths.launch.exists());
        assert!(!paths.sinks.supervisor_identity.exists());
        assert!(
            !paths.record.with_extension("retire.pending").exists(),
            "retirement pending pointer survived commit"
        );
    }
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_complete_retirement_recovers_without_pending_pointer() {
    let environment = test_environment();
    // Break caught: pending removal followed by parent-sync failure losing the only
    // cleanup-proven locator and preventing the typed failure archive/fresh retry.
    let fixture = GitFixture::new("direct-retire-pointer-cleanup");
    let artifact_root = fixture.root.join("evidence");
    let plan = bridge::parse_direct_command_plan("/usr/bin/true").expect("direct plan");
    environment.cleanup(bridge::LaunchFailpoint::CleanupSignal);
    let first = bridge::execute_direct_plan(
        &fixture.repo,
        &plan,
        &artifact_root,
        None,
        Duration::from_secs(5),
    );
    environment.cleanup(bridge::LaunchFailpoint::None);
    first.expect_err("first attempt leaves cleanup quarantine");

    environment.launch(bridge::LaunchFailpoint::RetireAfterPendingRemoval);
    let interrupted = bridge::execute_direct_plan(
        &fixture.repo,
        &plan,
        &artifact_root,
        None,
        Duration::from_secs(5),
    );
    environment.launch(bridge::LaunchFailpoint::None);
    interrupted.expect_err("retirement loses pending before final parent sync");
    let failed_record: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(artifact_root.join("command-000.json"))
            .expect("cleanup-failed command record"),
    )
    .expect("cleanup-failed record JSON");
    let failed_attempt_id = failed_record["attempt_id"]
        .as_str()
        .expect("cleanup-failed attempt id")
        .to_string();
    assert!(!artifact_root.join("command-000.retire.pending").exists());
    let completed_retirement = fs::read_dir(&artifact_root)
        .expect("retirement transactions")
        .flatten()
        .find(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("command-000.retire-")
                && entry.path().join("complete").is_file()
        })
        .expect("completed retirement");
    let retirement_commit: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(completed_retirement.path().join("complete"))
            .expect("retirement commit"),
    )
    .expect("retirement commit JSON");
    assert_eq!(
        retirement_commit["attempt_id"].as_str(),
        Some(failed_attempt_id.as_str()),
        "retirement and the later cleanup-failed terminal record must bind one attempt"
    );

    let recovered = bridge::execute_direct_plan(
        &fixture.repo,
        &plan,
        &artifact_root,
        None,
        Duration::from_secs(5),
    )
    .expect("complete retirement locator archives failure and retries");

    assert_eq!(recovered[0].terminal, bridge::AttemptTerminal::Exited(0));
    assert_ne!(
        recovered[0].attempt_id, failed_attempt_id,
        "fresh execution after archived cleanup failure must use a new attempt identity"
    );
    assert!(fs::read_dir(&artifact_root)
        .expect("failure archive")
        .flatten()
        .any(|entry| entry.file_name().to_string_lossy().contains(".archive-")));
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_live_sidecar_beats_stale_completed_retirement() {
    let _environment = test_environment();
    // Break caught: a completed retirement from an older attempt suppressing cleanup of a
    // newer live supervisor sidecar for the same command index.
    let fixture = GitFixture::new("direct-stale-retirement-live-sidecar");
    let artifact_root = fixture.root.join("evidence");
    fs::create_dir_all(&artifact_root).expect("artifact root");
    fs::set_permissions(&artifact_root, fs::Permissions::from_mode(0o700))
        .expect("private artifact root");
    let paths = bridge::direct_attempt_paths(&artifact_root, 0);
    let old_attempt_id = bridge::reserve_direct_attempt_id(&paths).expect("old attempt id");
    fs::write(&paths.launch, b"old ownership\n").expect("old launch");
    fs::set_permissions(&paths.launch, fs::Permissions::from_mode(0o600))
        .expect("private old launch");
    bridge::retire_direct_launch(&paths, &old_attempt_id).expect("retire old attempt");

    let invocation = shell_invocation(&fixture.repo, "exec /usr/bin/sleep 30");
    let validated = bridge::validate_invocation(
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
        &fixture.repo.canonicalize().expect("canonical fixture repo"),
    )
    .expect("validate live sidecar harness");
    let new_attempt_id = bridge::reserve_direct_attempt_id(&paths).expect("new attempt id");
    assert_ne!(old_attempt_id, new_attempt_id);
    let mut argv = vec![validated.program.display().to_string()];
    argv.extend(validated.args.clone());
    let intent = bridge::direct_intent_document(
        &new_attempt_id,
        &bridge::git_stdout(&fixture.repo, &["rev-parse", "--verify", "HEAD^{commit}"])
            .expect("fixture commit"),
        None,
        &validated.program,
        &argv,
    );
    bridge::write_private_create_once(
        &paths.intent,
        intent.as_bytes(),
        "current direct intent fixture",
    )
    .expect("write current intent");
    let mut child = bridge::spawn_blocked_harness(&validated, &paths.sinks, Some(&new_attempt_id))
        .expect("spawn current sidecar harness");
    let supervisor_pid = child.supervisor_birth().pid;
    child
        .release_launch_barrier()
        .expect("release current harness");
    drop(child);

    assert!(
        bridge::reconcile_direct_launch(&paths, Some(&intent)).expect("reconcile current sidecar")
    );
    assert!(
        bridge::observe_process_birth(supervisor_pid)
            .expect("observe cleaned supervisor")
            .is_none(),
        "new live sidecar was suppressed by stale retirement proof"
    );
    assert!(!paths.sinks.supervisor_identity.exists());
}

#[test]
fn autonomous_executor_bridge_terminal_record_attempt_must_match_intent() {
    let _environment = test_environment();
    // Break caught: a syntactically valid terminal record for another attempt being accepted
    // and archived solely because its argv and output digests were self-consistent.
    let fixture = GitFixture::new("direct-terminal-attempt-binding");
    let artifact_root = fixture.root.join("evidence");
    let plan = bridge::parse_direct_command_plan("/usr/bin/true").expect("direct plan");
    bridge::execute_direct_plan(
        &fixture.repo,
        &plan,
        &artifact_root,
        None,
        Duration::from_secs(5),
    )
    .expect("initial successful attempt");
    let record_path = artifact_root.join("command-000.json");
    let mut record: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&record_path).expect("terminal record"))
            .expect("terminal record JSON");
    record["attempt_id"] = serde_json::Value::String(
        bridge::new_direct_attempt_id_candidate().expect("foreign attempt id"),
    );
    fs::write(&record_path, record.to_string()).expect("replace terminal attempt id");

    let error = bridge::execute_direct_plan(
        &fixture.repo,
        &plan,
        &artifact_root,
        None,
        Duration::from_secs(5),
    )
    .expect_err("foreign terminal attempt must not authorize recovery");

    assert!(error.contains("resolved invocation intent"), "{error}");
}

#[test]
fn autonomous_executor_bridge_attempt_id_collision_uses_durable_reservation() {
    // Break caught: a restored clock/PID/process sequence reproducing a historical attempt ID
    // and making an old completed retirement look authoritative for a later execution.
    let fixture = GitFixture::new("direct-attempt-id-reservation");
    let artifact_root = fixture.root.join("evidence");
    fs::create_dir_all(&artifact_root).expect("artifact root");
    fs::set_permissions(&artifact_root, fs::Permissions::from_mode(0o700))
        .expect("private artifact root");
    let paths = bridge::direct_attempt_paths(&artifact_root, 0);
    let collision = autospec_core::autonomous::waterfall::sha256_hex(b"restored-generator");
    let fallback = autospec_core::autonomous::waterfall::sha256_hex(b"fresh-generator");
    let historical =
        bridge::reserve_direct_attempt_id_candidates(&paths, std::iter::once(collision.clone()))
            .expect("reserve historical attempt");
    fs::write(&paths.launch, b"historical ownership\n").expect("historical launch");
    fs::set_permissions(&paths.launch, fs::Permissions::from_mode(0o600))
        .expect("private historical launch");
    bridge::retire_direct_launch(&paths, &historical).expect("complete historical retirement");

    let current =
        bridge::reserve_direct_attempt_id_candidates(&paths, [collision.clone(), fallback.clone()])
            .expect("retry deterministic collision");

    assert_eq!(historical, collision);
    assert_eq!(current, fallback);
    assert_ne!(current, historical);
    let reservations = bridge::direct_attempt_reservation_directory(&paths);
    assert!(reservations.join(&historical).is_file());
    assert!(reservations.join(&current).is_file());
}
