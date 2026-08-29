// executor_bridge tests: restart / direct — 8 cases.
//
// Split out of tests.rs; see the note in that file.

use super::support_base::{
    test_environment, write_executable, DirectCrashFixtureCleanup, GitFixture,
};
use crate::commands::autonomous::executor_bridge as bridge;
#[cfg(target_os = "linux")]
use nix::sys::signal::Signal;
use std::collections::BTreeMap;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

const CLEANUP_BEFORE_VALIDATION_TEST: &str = "commands::autonomous::executor_bridge::tests::restart_direct::autonomous_executor_bridge_cleanup_precedes_executable_validation";
const CLEANUP_BEFORE_VALIDATION_RECEIPT: &str = "AUTOSPEC_TEST_CLEANUP_BEFORE_VALIDATION_RECEIPT";

#[test]
fn autonomous_executor_bridge_requires_exact_normalized_evidence_headings() {
    for heading in [
        "### Primary smoke test notes",
        "### Primary smoke test (inner loop) appendix",
        "#### Primary smoke test (inner loop)",
    ] {
        let body = format!("{heading}\n\n```\n/usr/bin/true\n```\n");
        assert!(
            bridge::parse_primary_smoke(&body).is_err(),
            "near-match heading was executable authority: {heading}"
        );
    }
    let exact = "  ###   PRIMARY   SMOKE TEST (INNER LOOP)  \n\n```\n/usr/bin/true\n```\n";
    assert!(bridge::parse_primary_smoke(exact).is_ok());

    let fixture = GitFixture::new("exact-full-heading");
    let near = "### Operator/full verification notes\n\n```\n/usr/bin/true\n```\n";
    assert!(
        bridge::resolve_full_suite(&fixture.repo, near, &[], &BTreeMap::new()).is_err(),
        "near-match Operator/full heading became executable authority"
    );
}

#[test]
fn autonomous_executor_bridge_executes_direct_segments_and_stops_on_first_failure() {
    let _environment = test_environment();
    let fixture = GitFixture::new("direct-qa");
    let artifact_root = fixture.root.join("evidence");
    let stopped_marker = fixture.root.join("must-not-run");
    let plan = bridge::parse_direct_command_plan(&format!(
        "/usr/bin/printf first && /usr/bin/false && /usr/bin/touch {}",
        stopped_marker.display()
    ))
    .expect("direct plan");

    let error = bridge::execute_direct_plan(
        &fixture.repo,
        &plan,
        &artifact_root,
        None,
        Duration::from_secs(5),
    )
    .expect_err("second direct command must fail");

    assert!(error.contains("exit status 1"), "{error}");
    assert!(!stopped_marker.exists(), "later segment must not execute");
    let stdout = fs::read(artifact_root.join("command-000.stdout")).expect("first stdout artifact");
    assert_eq!(stdout, b"first");
    assert!(artifact_root.join("command-001.stderr").is_file());
    let failed_record: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(artifact_root.join("command-001.json")).expect("failed command record"),
    )
    .expect("typed failed command record");
    assert_eq!(failed_record["terminal"]["kind"], "exited");
    assert_eq!(failed_record["terminal"]["code"], 1);
}

#[test]
fn autonomous_executor_bridge_restart_reruns_completed_disk_attempt() {
    let _environment = test_environment();
    // Break caught: a successful record recovered from disk being treated as fresh runtime
    // evidence and authorizing Pass without executing the command in this process.
    let fixture = GitFixture::new("direct-recovery");
    let artifact_root = fixture.root.join("evidence");
    let count = fixture.root.join("count");
    let plan = bridge::parse_direct_command_plan(&format!(
        "/usr/bin/python3 -c 'from pathlib import Path; p=Path(\"{}\"); p.write_text(str(int(p.read_text())+1) if p.exists() else \"1\")'",
        count.display()
    ))
    .expect("recovery plan");

    let first = bridge::execute_direct_plan(
        &fixture.repo,
        &plan,
        &artifact_root,
        None,
        Duration::from_secs(5),
    )
    .expect("first attempt");
    let second = bridge::execute_direct_plan(
        &fixture.repo,
        &plan,
        &artifact_root,
        None,
        Duration::from_secs(5),
    )
    .expect("rerun completed disk attempt");

    assert_eq!(fs::read_to_string(&count).expect("execution count"), "2");
    assert_eq!(first[0].terminal, bridge::AttemptTerminal::Exited(0));
    assert_eq!(second[0].terminal, bridge::AttemptTerminal::Exited(0));
    assert!(fs::read_dir(&artifact_root)
        .expect("diagnostic archive")
        .flatten()
        .any(|entry| entry.file_name().to_string_lossy().contains(".archive-")));
}

#[test]
fn autonomous_executor_bridge_restart_recovers_partial_output_before_terminal_record() {
    let _environment = test_environment();
    let fixture = GitFixture::new("direct-partial-recovery");
    let artifact_root = fixture.root.join("evidence");
    fs::create_dir_all(&artifact_root).expect("artifact root");
    fs::set_permissions(&artifact_root, fs::Permissions::from_mode(0o700))
        .expect("private artifact root");
    let partial = artifact_root.join("command-000.stdout");
    fs::write(&partial, "partial").expect("partial stdout");
    fs::set_permissions(&partial, fs::Permissions::from_mode(0o600))
        .expect("private partial stdout");
    let plan = bridge::parse_direct_command_plan("/usr/bin/printf recovered").expect("direct plan");

    let observed = bridge::execute_direct_plan(
        &fixture.repo,
        &plan,
        &artifact_root,
        None,
        Duration::from_secs(5),
    )
    .expect("recover interrupted attempt");

    assert_eq!(
        fs::read(&observed[0].stdout_path).expect("recovered stdout"),
        b"recovered"
    );
    assert!(artifact_root.join("command-000.json").is_file());
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_parent_crash_helper() {
    let _environment = test_environment();
    let Some(repo) = std::env::var_os("AUTOSPEC_TEST_CRASH_REPO") else {
        return;
    };
    let artifact_root =
        PathBuf::from(std::env::var_os("AUTOSPEC_TEST_CRASH_ARTIFACT").expect("artifact"));
    let command = std::env::var("AUTOSPEC_TEST_CRASH_COMMAND").expect("crash helper command");
    let plan = bridge::parse_direct_command_plan(&command).expect("crash helper plan");
    let _ = bridge::execute_direct_plan(
        Path::new(&repo),
        &plan,
        &artifact_root,
        None,
        Duration::from_secs(60),
    );
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_restart_reaps_exact_crashed_parent_group_before_retry() {
    let _environment = test_environment();
    // Break caught: recovery deleting partial output while the command started by the dead
    // parent is still running.
    let fixture = GitFixture::new("direct-parent-crash");
    let artifact_root = fixture.root.join("evidence");
    let marker = fixture.root.join("command.pid");
    let command = format!(
        "/usr/bin/python3 -c 'from pathlib import Path; import os,time; p=Path(\"{}\"); first=not p.exists(); p.write_text(str(os.getpid())); time.sleep(30) if first else None'",
        marker.display()
    );
    let mut parent = Command::new(std::env::current_exe().expect("test executable"))
        .args([
            "--exact",
            "commands::autonomous::executor_bridge::tests::restart_direct::autonomous_executor_bridge_parent_crash_helper",
            "--nocapture",
        ])
        .env("AUTOSPEC_TEST_CRASH_REPO", &fixture.repo)
        .env("AUTOSPEC_TEST_CRASH_ARTIFACT", &artifact_root)
        .env("AUTOSPEC_TEST_CRASH_COMMAND", &command)
        .spawn()
        .expect("crash-parent process");
    let deadline = Instant::now() + Duration::from_secs(5);
    while !marker.is_file() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    let old_pid = fs::read_to_string(&marker)
        .expect("old command published pid")
        .parse::<i32>()
        .expect("old command pid");
    let launch_was_durable = artifact_root.join("command-000.launch.json").is_file();
    parent.kill().expect("crash command parent");
    parent.wait().expect("reap crashed command parent");

    let plan = bridge::parse_direct_command_plan(&command).expect("recovery plan");
    let recovered = bridge::execute_direct_plan(
        &fixture.repo,
        &plan,
        &artifact_root,
        None,
        Duration::from_secs(5),
    )
    .expect("recovery must reap the old group before retry");
    let old_group_survived =
        nix::sys::signal::kill(nix::unistd::Pid::from_raw(old_pid), None).is_ok();
    if old_group_survived {
        let _ = nix::sys::signal::kill(nix::unistd::Pid::from_raw(-old_pid), Signal::SIGKILL);
    }

    assert!(
        launch_was_durable,
        "child did work before its exact launch identity was durable"
    );
    assert!(!old_group_survived, "recovery left the old command alive");
    assert_eq!(recovered[0].terminal, bridge::AttemptTerminal::Exited(0));
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_direct_supervisor_reaps_adopted_children() {
    let _environment = test_environment();
    let fixture = GitFixture::new("direct-live-adopted-reap");
    let artifact_root = fixture.root.join("evidence");
    let receipt_path = fixture.root.join("cleanup-before-validation.receipt");
    let executable = std::env::current_exe().expect("current test executable");
    let plan = bridge::DirectCommandPlan {
        commands: vec![bridge::DirectCommand::success(vec![
            "/usr/bin/env".to_string(),
            format!(
                "{CLEANUP_BEFORE_VALIDATION_RECEIPT}={}",
                receipt_path.display()
            ),
            executable.display().to_string(),
            "--ignored".to_string(),
            "--exact".to_string(),
            CLEANUP_BEFORE_VALIDATION_TEST.to_string(),
            "--test-threads=1".to_string(),
        ])],
    };

    let observed = bridge::execute_direct_plan(
        &fixture.repo,
        &plan,
        &artifact_root,
        None,
        Duration::from_secs(15),
    )
    .unwrap_or_else(|error| {
        let stdout = fs::read_to_string(artifact_root.join("command-000.stdout"))
            .unwrap_or_else(|read_error| format!("<cannot read stdout: {read_error}>"));
        let stderr = fs::read_to_string(artifact_root.join("command-000.stderr"))
            .unwrap_or_else(|read_error| format!("<cannot read stderr: {read_error}>"));
        panic!(
            "nested process-cleanup test failed under direct supervision: {error}; stdout={stdout} stderr={stderr}"
        );
    });

    assert_eq!(observed[0].terminal, bridge::AttemptTerminal::Exited(0));
    assert_eq!(
        fs::read_to_string(&receipt_path).expect("nested cleanup test receipt"),
        format!("{CLEANUP_BEFORE_VALIDATION_TEST}\n"),
        "nested cleanup test did not publish its exact completion receipt"
    );
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "launched in isolation by the adopted-children supervision test"]
fn autonomous_executor_bridge_cleanup_precedes_executable_validation() {
    let _environment = test_environment();
    // Break caught: a missing/replaced current executable returning before an old live
    // quarantined tree is reconciled from its independently persisted intent and launch.
    let fixture = GitFixture::new("direct-cleanup-before-validation");
    let artifact_root = fixture.root.join("evidence");
    let marker = fixture.root.join("running");
    let executable = fixture.root.join("ephemeral-command");
    write_executable(
        &executable,
        &format!(
            "#!/bin/sh\nprintf '%s' \"$$\" > '{}'\nexec /bin/sleep 30\n",
            marker.display()
        ),
    );
    let command = executable.display().to_string();
    let parent = Command::new(std::env::current_exe().expect("test executable"))
        .args([
            "--exact",
            "commands::autonomous::executor_bridge::tests::restart_direct::autonomous_executor_bridge_parent_crash_helper",
            "--nocapture",
        ])
        .env("AUTOSPEC_TEST_CRASH_REPO", &fixture.repo)
        .env("AUTOSPEC_TEST_CRASH_ARTIFACT", &artifact_root)
        .env("AUTOSPEC_TEST_CRASH_COMMAND", &command)
        .spawn()
        .expect("crash-parent process");
    let launch = artifact_root.join("command-000.launch.json");
    let mut cleanup = DirectCrashFixtureCleanup::new(parent, launch.clone());
    let deadline = Instant::now() + Duration::from_secs(5);
    while (!marker.is_file() || !launch.is_file()) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    let value: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&launch).expect("durable launch"))
            .expect("launch JSON");
    let supervisor =
        bridge::parse_process_identity(value["supervisor"].clone(), "supervisor fixture")
            .expect("supervisor identity");
    let harness = bridge::parse_process_identity(value["process"].clone(), "harness fixture")
        .expect("harness identity");
    cleanup.arm(supervisor.clone(), harness.clone());
    cleanup.crash_parent();
    fs::remove_file(&executable).expect("remove current executable before restart");
    let plan = bridge::parse_direct_command_plan(&command).expect("recovery plan");

    let error = bridge::execute_direct_plan(
        &fixture.repo,
        &plan,
        &artifact_root,
        None,
        Duration::from_secs(5),
    )
    .expect_err("missing current executable still fails after cleanup");

    assert!(error.contains("executable"), "{error}");
    let reap_deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < reap_deadline
        && (bridge::observe_process_birth(supervisor.pid)
            .expect("observe old supervisor")
            .is_some()
            || bridge::observe_process_birth(harness.pid)
                .expect("observe old harness")
                .is_some())
    {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        bridge::observe_process_birth(supervisor.pid)
            .expect("observe old supervisor")
            .is_none(),
        "supervisor survived pre-validation cleanup"
    );
    assert!(
        bridge::observe_process_birth(harness.pid)
            .expect("observe old harness")
            .is_none(),
        "harness survived pre-validation cleanup"
    );
    assert!(!launch.exists(), "retired launch identity survived cleanup");
    if let Some(path) = std::env::var_os(CLEANUP_BEFORE_VALIDATION_RECEIPT) {
        fs::write(path, format!("{CLEANUP_BEFORE_VALIDATION_TEST}\n"))
            .expect("publish exact cleanup helper receipt");
    }
    cleanup.disarm();
}
