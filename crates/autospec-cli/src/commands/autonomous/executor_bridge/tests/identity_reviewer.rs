// executor_bridge tests: identity / reviewer — 9 cases.
//
// Split out of tests.rs; see the note in that file.

use super::support_base::{test_environment, write_executable, GitFixture};
use super::support_launch::{
    automatic_review_command, direct_failure_archive_count, direct_launch_supervisor_pid,
    rewrite_direct_terminal_as_signal,
};
use crate::commands::autonomous::executor_bridge as bridge;
#[cfg(target_os = "linux")]
use nix::sys::signal::Signal;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::Duration;

#[test]
fn autonomous_executor_bridge_installed_tool_retries_spawn_failure() {
    let _environment = test_environment();
    // Break caught: SpawnFailed records without a logical intent poisoning the command index
    // forever once the missing dependency becomes available.
    let fixture = GitFixture::new("direct-install-after-spawn-failure");
    let artifact_root = fixture.root.join("evidence");
    let executable = fixture.root.join("installed-later");
    let runtime = bridge::DirectRuntimeAdapter {
        repo: fixture.repo.canonicalize().expect("runtime repo"),
        session_id: "runtime-install-session".to_string(),
        environment_dir: fixture
            .repo
            .canonicalize()
            .expect("runtime repo")
            .join(".autospec-test-runtime-adapter"),
        session: std::cell::RefCell::new(None),
    };
    let plan = bridge::parse_direct_command_plan(&executable.display().to_string())
        .expect("missing executable plan");
    bridge::execute_direct_plan(
        &fixture.repo,
        &plan,
        &artifact_root,
        Some(&runtime),
        Duration::from_secs(5),
    )
    .expect_err("missing executable records SpawnFailed");
    let failed_record: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(artifact_root.join("command-000.json")).expect("spawn failure record"),
    )
    .expect("spawn failure JSON");
    let failed_intent: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(artifact_root.join("command-000.intent.json"))
            .expect("unresolved intent"),
    )
    .expect("unresolved intent JSON");
    assert_eq!(failed_intent["resolution"], "unresolved");
    assert_eq!(failed_record["attempt_id"], failed_intent["attempt_id"]);
    assert_eq!(
        failed_record["runtime_session_id"],
        "runtime-install-session"
    );
    assert_eq!(
        failed_intent["runtime_session_id"],
        "runtime-install-session"
    );

    write_executable(&executable, "#!/bin/sh\nexit 0\n");
    let recovered = bridge::execute_direct_plan(
        &fixture.repo,
        &plan,
        &artifact_root,
        Some(&runtime),
        Duration::from_secs(5),
    )
    .expect("installed executable runs under a fresh attempt");

    assert_eq!(recovered[0].terminal, bridge::AttemptTerminal::Exited(0));
    assert_ne!(
        recovered[0].attempt_id,
        failed_record["attempt_id"]
            .as_str()
            .expect("failed attempt id")
    );
    assert!(fs::read_dir(&artifact_root)
        .expect("failure archive")
        .flatten()
        .any(|entry| {
            entry.file_name().to_string_lossy().contains(".archive-")
                && entry.path().join("command-000.json").is_file()
                && entry.path().join("command-000.intent.json").is_file()
        }));
}

#[test]
fn autonomous_executor_bridge_identical_failures_use_distinct_archives() {
    // Break caught: digest-only archive names colliding when two attempts fail identically.
    let fixture = GitFixture::new("direct-identical-failure-archives");
    let artifact_root = fixture.root.join("evidence");
    fs::create_dir_all(&artifact_root).expect("artifact root");
    fs::set_permissions(&artifact_root, fs::Permissions::from_mode(0o700))
        .expect("private artifact root");
    let paths = bridge::direct_attempt_paths(&artifact_root, 0);
    for _ in 0..2 {
        for path in [&paths.record, &paths.stdout, &paths.stderr, &paths.intent] {
            fs::write(path, b"identical\n").expect("identical failure artifact");
            fs::set_permissions(path, fs::Permissions::from_mode(0o600))
                .expect("private identical artifact");
        }
        bridge::archive_reconciled_direct_failure(&paths)
            .expect("archive identical failure independently");
    }
    let archives = fs::read_dir(&artifact_root)
        .expect("archive root")
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().contains(".archive-"))
        .collect::<Vec<_>>();
    assert_eq!(archives.len(), 2);
    assert!(archives
        .iter()
        .all(|entry| entry.path().join("complete").is_file()));
}

#[test]
fn automatic_reviewer_identity_change_retries_signaled_failure_once() {
    let _environment = test_environment();
    let fixture = GitFixture::new("review-identity-retry");
    let evidence = fixture.root.join("review-evidence");
    let capture = fixture.root.join("review-capture");
    let executable = fixture.root.join("reviewer");
    let marker = fixture.root.join("reviewer-ran");
    write_executable(
        &executable,
        &format!(
            "#!/bin/sh\nif [ ! -f '{}' ]; then : > '{}'; exit 1; fi\nexit 0\n",
            marker.display(),
            marker.display()
        ),
    );
    let old = automatic_review_command(&executable, &capture, &"a".repeat(64));
    let new = automatic_review_command(&executable, &capture, &"b".repeat(64));

    let first = bridge::execute_direct_plan(
        &fixture.repo,
        &bridge::DirectCommandPlan {
            commands: vec![old],
        },
        &evidence,
        None,
        Duration::from_secs(5),
    )
    .expect_err("old reviewer must leave a durable failure");
    assert!(first.contains("exit status 1"), "{first}");
    rewrite_direct_terminal_as_signal(&evidence, 25);
    let recovered = bridge::execute_direct_plan(
        &fixture.repo,
        &bridge::DirectCommandPlan {
            commands: vec![new],
        },
        &evidence,
        None,
        Duration::from_secs(5),
    )
    .expect("changed automatic-review identity retries once");

    assert_eq!(recovered[0].terminal, bridge::AttemptTerminal::Exited(0));
    assert_eq!(direct_failure_archive_count(&evidence), 1);
    let paths = bridge::direct_attempt_paths(&evidence, 0);
    assert!(!bridge::changed_automatic_reviewer_failure(
        &fixture.repo.canonicalize().expect("canonical repo"),
        &paths,
        &automatic_review_command(&executable, &capture, &"b".repeat(64)),
        None,
    )
    .expect("restart inspects completed retry"));
    assert_eq!(direct_failure_archive_count(&evidence), 1);
    assert_eq!(
        fs::read_dir(bridge::direct_attempt_reservation_directory(&paths))
            .expect("attempt reservations")
            .flatten()
            .count(),
        2,
        "restart must not reserve a third automatic-review attempt"
    );
    let archive = fs::read_dir(&evidence)
        .expect("failure archive")
        .flatten()
        .find(|entry| entry.file_name().to_string_lossy().contains(".archive-"))
        .expect("archived failed review")
        .path();
    for name in [
        "command-000.json",
        "command-000.intent.json",
        "command-000.stdout",
        "command-000.stderr",
        "complete",
    ] {
        assert!(archive.join(name).is_file(), "missing archived {name}");
    }
    assert!(fs::read_dir(&evidence)
        .expect("launch retirement evidence")
        .flatten()
        .any(
            |entry| entry.file_name().to_string_lossy().contains(".retire-")
                && entry.path().join("complete").is_file()
        ));
}

#[test]
fn automatic_reviewer_identity_change_same_identity_keeps_terminal_failure() {
    let _environment = test_environment();
    let fixture = GitFixture::new("review-identity-same");
    let evidence = fixture.root.join("review-evidence");
    let capture = fixture.root.join("review-capture");
    let executable = fixture.root.join("reviewer");
    let count = fixture.root.join("reviewer-count");
    write_executable(
        &executable,
        &format!(
            "#!/bin/sh\nn=0; [ ! -f '{}' ] || n=$(cat '{}'); n=$((n+1)); printf '%s' \"$n\" > '{}'; exit 1\n",
            count.display(),
            count.display(),
            count.display()
        ),
    );
    let command = || automatic_review_command(&executable, &capture, &"a".repeat(64));
    let run = || {
        bridge::execute_direct_plan(
            &fixture.repo,
            &bridge::DirectCommandPlan {
                commands: vec![command()],
            },
            &evidence,
            None,
            Duration::from_secs(5),
        )
    };

    run().expect_err("first reviewer signal persists");
    rewrite_direct_terminal_as_signal(&evidence, 25);
    let second = run().expect_err("same reviewer identity must not retry");

    assert!(second.contains("signal 25"), "{second}");
    assert_eq!(fs::read_to_string(count).expect("reviewer count"), "1");
    assert_eq!(direct_failure_archive_count(&evidence), 0);
}

#[test]
fn automatic_reviewer_identity_change_never_retries_success() {
    let _environment = test_environment();
    let fixture = GitFixture::new("review-success-no-retry");
    let evidence = fixture.root.join("review-evidence");
    let capture = fixture.root.join("review-capture");
    let command = automatic_review_command(Path::new("/usr/bin/true"), &capture, &"a".repeat(64));
    bridge::execute_direct_plan(
        &fixture.repo,
        &bridge::DirectCommandPlan {
            commands: vec![command.clone()],
        },
        &evidence,
        None,
        Duration::from_secs(5),
    )
    .expect("successful automatic review");
    let paths = bridge::direct_attempt_paths(&evidence, 0);
    let changed = automatic_review_command(Path::new("/usr/bin/true"), &capture, &"b".repeat(64));

    assert!(!bridge::changed_automatic_reviewer_failure(
        &fixture.repo.canonicalize().expect("canonical repo"),
        &paths,
        &changed,
        None,
    )
    .expect("inspect successful review"));
    assert_eq!(direct_failure_archive_count(&evidence), 0);
}

#[test]
fn automatic_reviewer_identity_change_recovers_ci_passed_review() {
    let _environment = test_environment();
    let fixture = GitFixture::new("issue-52-ci-passed-review");
    let evidence = fixture.root.join("receipts/52/ci_passed/review");
    let capture = fixture.root.join("review-capture");
    fs::create_dir_all(&evidence).expect("legacy review evidence");
    fs::set_permissions(&evidence, fs::Permissions::from_mode(0o700))
        .expect("private legacy review evidence");
    let executable = evidence.join("review-normalizer.sh");
    let repaired = fixture.root.join("reviewer-repaired");
    write_executable(
        &executable,
        &format!(
            "#!/bin/sh\n[ -f '{}' ] || exit 1\nexit 0\n",
            repaired.display()
        ),
    );
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700))
        .expect("private legacy reviewer");
    let mut first = automatic_review_command(&executable, &capture, &"a".repeat(64));
    first.identity_digest = None;
    bridge::execute_direct_plan(
        &fixture.repo,
        &bridge::DirectCommandPlan {
            commands: vec![first],
        },
        &evidence,
        None,
        Duration::from_secs(5),
    )
    .expect_err("old ci_passed review terminal");
    rewrite_direct_terminal_as_signal(&evidence, 25);
    fs::write(&repaired, b"repaired\n").expect("repair reviewer");
    let changed = automatic_review_command(&executable, &capture, &"b".repeat(64));

    let recovered = bridge::execute_direct_plan(
        &fixture.repo,
        &bridge::DirectCommandPlan {
            commands: vec![changed],
        },
        &evidence,
        None,
        Duration::from_secs(5),
    )
    .expect("ci_passed restart reaches repaired review");

    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].terminal, bridge::AttemptTerminal::Exited(0));
    assert_eq!(direct_failure_archive_count(&evidence), 1);
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_spawn_cleanup_failure_persists_quarantine_identity() {
    let environment = test_environment();
    let fixture = GitFixture::new("direct-spawn-quarantine");
    let artifact_root = fixture.root.join("evidence");
    let plan = bridge::parse_direct_command_plan("/usr/bin/true").expect("direct plan");

    environment.launch(bridge::LaunchFailpoint::NeverReady);
    environment.cleanup(bridge::LaunchFailpoint::CleanupSignal);
    let first = bridge::execute_direct_plan(
        &fixture.repo,
        &plan,
        &artifact_root,
        None,
        Duration::from_secs(5),
    );
    environment.launch(bridge::LaunchFailpoint::None);
    environment.cleanup(bridge::LaunchFailpoint::None);
    let first = first.expect_err("spawn cleanup failure must be quarantined");
    let launch = artifact_root.join("command-000.launch.json");
    let supervisor = direct_launch_supervisor_pid(&launch);
    assert!(first.contains("cleanup"), "{first}");
    assert!(Path::new(&format!("/proc/{supervisor}")).exists());

    let retry = bridge::execute_direct_plan(
        &fixture.repo,
        &plan,
        &artifact_root,
        None,
        Duration::from_secs(5),
    )
    .expect("reconciled spawn cleanup starts a fresh attempt");

    assert_eq!(retry[0].terminal, bridge::AttemptTerminal::Exited(0));
    assert!(!launch.exists(), "{retry:?}");
    assert!(!Path::new(&format!("/proc/{supervisor}")).exists());
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_descendant_capture_failure_cannot_retire_identity() {
    let environment = test_environment();
    // Break caught: a terminal exact harness omitted from the live descendant map being
    // misclassified as foreign instead of retained for exact reaping during cleanup.
    let fixture = GitFixture::new("direct-descendant-capture-quarantine");
    let artifact_root = fixture.root.join("evidence");
    let descendant = fixture.root.join("descendant.pid");
    let plan = bridge::parse_direct_command_plan(&format!(
        "/usr/bin/python3 -c 'from pathlib import Path; import os,time; p=Path(\"{}\"); first=not p.exists(); pid=os.fork() if first else None; p.write_text(str(pid)) if pid else (time.sleep(30) if pid == 0 else None)'",
        descendant.display()
    ))
    .expect("descendant plan");

    environment.cleanup(bridge::LaunchFailpoint::DescendantCapture);
    let first = bridge::execute_direct_plan(
        &fixture.repo,
        &plan,
        &artifact_root,
        None,
        Duration::from_secs(5),
    );
    environment.cleanup(bridge::LaunchFailpoint::None);
    let first = first.expect_err("uncaptured live descendant must fail cleanup closed");
    let launch = artifact_root.join("command-000.launch.json");
    let supervisor = direct_launch_supervisor_pid(&launch);
    assert!(first.contains("cleanup"), "{first}");
    assert!(Path::new(&format!("/proc/{supervisor}")).exists());

    let retry = bridge::execute_direct_plan(
        &fixture.repo,
        &plan,
        &artifact_root,
        None,
        Duration::from_secs(5),
    )
    .expect("reconciled descendant cleanup starts a fresh attempt");
    let descendant = fs::read_to_string(descendant)
        .expect("descendant pid")
        .trim()
        .to_string();

    assert_eq!(retry[0].terminal, bridge::AttemptTerminal::Exited(0));
    assert!(!launch.exists());
    assert!(!Path::new(&format!("/proc/{supervisor}")).exists());
    assert!(!Path::new(&format!("/proc/{descendant}")).exists());
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_success_with_descendant_is_typed_cleanup_failure() {
    let _environment = test_environment();
    // Break caught: a successful unisolated leader returning Pass while its child survives.
    let fixture = GitFixture::new("unisolated-descendant");
    let artifact_root = fixture.root.join("evidence");
    let descendant = fixture.root.join("descendant.pid");
    let plan = bridge::parse_direct_command_plan(&format!(
        "/usr/bin/python3 -c 'import os,time; pid=os.fork(); open(\"{}\",\"w\").write(str(pid)) if pid else time.sleep(30)'",
        descendant.display()
    ))
    .expect("descendant plan");

    let result = bridge::execute_direct_plan(
        &fixture.repo,
        &plan,
        &artifact_root,
        None,
        Duration::from_secs(5),
    );
    let descendant_pid = fs::read_to_string(&descendant)
        .expect("descendant pid")
        .parse::<i32>()
        .expect("descendant pid integer");
    let survived = nix::sys::signal::kill(nix::unistd::Pid::from_raw(descendant_pid), None).is_ok();
    if survived {
        let _ =
            nix::sys::signal::kill(nix::unistd::Pid::from_raw(-descendant_pid), Signal::SIGKILL);
    }
    let error = result.expect_err("surviving descendant must fail cleanup");
    let record: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(artifact_root.join("command-000.json")).expect("typed cleanup record"),
    )
    .expect("typed command record");

    assert!(error.contains("cleanup"), "{error}");
    assert_eq!(record["terminal"]["kind"], "cleanup_failed");
    assert!(!survived, "unisolated descendant survived group cleanup");
}
