use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
#[cfg(unix)]
use std::net::{TcpListener, TcpStream};
#[cfg(unix)]
use std::os::unix::fs::{symlink, PermissionsExt};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use autospec_core::runtime_env::{EnvironmentLifecycle, EnvironmentOwner};
#[cfg(target_os = "linux")]
use nix::sys::signal::Signal;

use super::{
    build_implementer_prompt, provision_issue_worktree, recover_invocation, resolve_base,
    runtime_session_adapter, supervise_harness, validate_trusted_ownership,
    write_invocation_atomic, BridgeIdentity, BridgePhase, ExecutorBridgeRequest, HarnessConfig,
    HarnessInvocation, HarnessKind, MutationSnapshot, PersistedInvocation, ProcessIdentity,
    ResolvedBase, SupervisionConfig, SupervisionOutcome, CLAUDE_BUILTIN_TOOLS,
    CLAUDE_FORBIDDEN_TOOLS, CLAUDE_LOCAL_TOOLS,
};

mod support_base;
use support_base::*;
mod support_invocation;
use support_invocation::*;
mod support_launch;
use support_launch::*;
mod attempt_generation;
mod worktree_post;
mod license_checker;
mod descendant_spawn;
mod full_suite;
mod reviewer_runtime;
mod draft_release;
mod json_identity;
mod quarantine_nested;
mod dispatcher_temporary;
mod generation_input;
mod cleanup_reap;
mod closeout_repairs;
mod snapshot_identity;
mod branch_predecessor;
mod runtime_fixture;
mod terminal_label;
mod reviewer_automatic;
mod prunable_zero;
mod codex_permission;
mod identity_reviewer;
mod repair_implementation;
mod closeout_harness;
mod pull_mutation;
mod scope_root;
mod production_entry;
mod commit_rust;
mod codex_sandbox;
mod closeout_remote;
mod sidecar_launch;
mod restart_direct;
mod adoption_cleanup;
mod ready_harness;
mod rust_commit;
mod result_reviewer;
mod remote_base;
mod sync_integration;
mod harness_supervisor;
mod continuation_event;
mod cleanup_restart;
mod ordered_publication;
mod proxy_direct;
mod attempt_retirement;

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_plan_shrink_cleans_removed_trailing_index() {
    // Break caught: cleanup preflight enumerating only the new shorter plan and abandoning a
    // live interrupted tree owned by a removed trailing command index.
    let fixture = GitFixture::new("direct-plan-shrink-cleanup");
    let artifact_root = fixture.root.join("evidence");
    let marker = fixture.root.join("trailing.pid");
    let old_command = format!(
        "/usr/bin/true && /usr/bin/python3 -c 'from pathlib import Path; import os,time; Path(\"{}\").write_text(str(os.getpid())); time.sleep(30)'",
        marker.display()
    );
    let parent = Command::new(std::env::current_exe().expect("test executable"))
        .args([
            "--exact",
            "commands::autonomous::executor_bridge::tests::autonomous_executor_bridge_parent_crash_helper",
            "--nocapture",
        ])
        .env("AUTOSPEC_TEST_CRASH_REPO", &fixture.repo)
        .env("AUTOSPEC_TEST_CRASH_ARTIFACT", &artifact_root)
        .env("AUTOSPEC_TEST_CRASH_COMMAND", &old_command)
        .spawn()
        .expect("crash-parent process");
    let launch = artifact_root.join("command-001.launch.json");
    let mut cleanup = DirectCrashFixtureCleanup::new(parent, launch.clone());
    let deadline = Instant::now() + Duration::from_secs(5);
    while (!marker.is_file() || !launch.is_file()) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    let value: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&launch).expect("trailing launch"))
            .expect("trailing launch JSON");
    let supervisor =
        super::parse_process_identity(value["supervisor"].clone(), "supervisor fixture")
            .expect("supervisor identity");
    let harness = super::parse_process_identity(value["process"].clone(), "harness fixture")
        .expect("harness identity");
    cleanup.arm(supervisor.clone(), harness.clone());
    cleanup.crash_parent();
    let new_plan =
        super::parse_direct_command_plan("/usr/bin/true").expect("shortened recovery plan");

    let observed = super::execute_direct_plan(
        &fixture.repo,
        &new_plan,
        &artifact_root,
        None,
        Duration::from_secs(5),
    )
    .expect("shortened plan proceeds after all-index cleanup");

    assert_eq!(observed.len(), 1);
    assert!(
        super::observe_process_birth(supervisor.pid)
            .expect("observe removed supervisor")
            .is_none(),
        "removed trailing supervisor survived plan-shrink cleanup"
    );
    assert!(
        super::observe_process_birth(harness.pid)
            .expect("observe removed harness")
            .is_none(),
        "removed trailing harness survived plan-shrink cleanup"
    );
    assert!(!launch.exists());
    assert!(
        artifact_root.join("command-001.intent.json").is_file(),
        "removed command intent must remain immutable diagnostic context"
    );
    cleanup.disarm();
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_restart_adopts_live_harness_after_supervisor_death() {
    let fixture = GitFixture::new("direct-dead-supervisor");
    let artifact_root = fixture.root.join("evidence");
    let marker = fixture.root.join("command.pid");
    let command = format!(
        "/usr/bin/python3 -c 'from pathlib import Path; import os,time; p=Path(\"{}\"); first=not p.exists(); p.write_text(str(os.getpid())); time.sleep(30) if first else None'",
        marker.display()
    );
    let mut parent = Command::new(std::env::current_exe().expect("test executable"))
        .args([
            "--exact",
            "commands::autonomous::executor_bridge::tests::autonomous_executor_bridge_parent_crash_helper",
            "--nocapture",
        ])
        .env("AUTOSPEC_TEST_CRASH_REPO", &fixture.repo)
        .env("AUTOSPEC_TEST_CRASH_ARTIFACT", &artifact_root)
        .env("AUTOSPEC_TEST_CRASH_COMMAND", &command)
        .spawn()
        .expect("crash-parent process");
    let launch = artifact_root.join("command-000.launch.json");
    let deadline = Instant::now() + Duration::from_secs(5);
    while (!marker.is_file() || !launch.is_file()) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    let launch_value: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&launch).expect("durable launch record"))
            .expect("launch JSON");
    let supervisor = launch_value["supervisor"]["pid"]
        .as_i64()
        .and_then(|pid| i32::try_from(pid).ok())
        .expect("supervisor PID");
    let harness = launch_value["process"]["pid"]
        .as_i64()
        .and_then(|pid| i32::try_from(pid).ok())
        .expect("harness PID");
    nix::sys::signal::kill(nix::unistd::Pid::from_raw(supervisor), Signal::SIGKILL)
        .expect("kill stable supervisor only");
    let deadline = Instant::now() + Duration::from_secs(2);
    while Path::new(&format!("/proc/{supervisor}")).exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        Path::new(&format!("/proc/{harness}")).exists(),
        "harness must remain live after supervisor death"
    );
    parent.kill().expect("crash command parent");
    parent.wait().expect("reap crashed command parent");

    let plan = super::parse_direct_command_plan(&command).expect("recovery plan");
    let recovered = super::execute_direct_plan(
        &fixture.repo,
        &plan,
        &artifact_root,
        None,
        Duration::from_secs(5),
    )
    .expect("restart must adopt the exact live harness and retry");

    assert!(!Path::new(&format!("/proc/{harness}")).exists());
    assert!(!launch.exists());
    assert_eq!(recovered[0].terminal, super::AttemptTerminal::Exited(0));
}

#[cfg(target_os = "linux")]
fn assert_exec_replaced_direct_harness_recovers(
    fixture: &GitFixture,
    command: &str,
    marker: &Path,
) {
    let artifact_root = fixture.root.join("evidence");
    let parent = Command::new(std::env::current_exe().expect("test executable"))
        .args([
            "--exact",
            "commands::autonomous::executor_bridge::tests::autonomous_executor_bridge_parent_crash_helper",
            "--nocapture",
        ])
        .env("AUTOSPEC_TEST_CRASH_REPO", &fixture.repo)
        .env("AUTOSPEC_TEST_CRASH_ARTIFACT", &artifact_root)
        .env("AUTOSPEC_TEST_CRASH_COMMAND", command)
        .spawn()
        .expect("crash-parent process");
    let launch = artifact_root.join("command-000.launch.json");
    let mut cleanup = DirectCrashFixtureCleanup::new(parent, launch.clone());
    let deadline = Instant::now() + Duration::from_secs(5);
    while (!marker.is_file() || !launch.is_file()) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    let launch_value: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&launch).expect("durable launch record"))
            .expect("launch JSON");
    let supervisor =
        super::parse_process_identity(launch_value["supervisor"].clone(), "supervisor fixture")
            .expect("supervisor identity");
    let harness =
        super::parse_process_identity(launch_value["process"].clone(), "harness fixture")
            .expect("harness identity");
    cleanup.arm(supervisor.clone(), harness.clone());
    let deadline = Instant::now() + Duration::from_secs(5);
    let observed = loop {
        let Some(observed) = super::observe_process_identity(harness.pid, &harness.argv_digest)
            .expect("observe exec-replaced harness")
        else {
            assert!(
                Instant::now() < deadline,
                "exec-replaced harness disappeared during identity transition"
            );
            std::thread::sleep(Duration::from_millis(10));
            continue;
        };
        if observed.executable != harness.executable
            || observed.argv_digest != harness.argv_digest
        {
            break observed;
        }
        assert!(
            Instant::now() < deadline,
            "fixture harness never replaced its declared exec identity"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(harness.same_birth(&observed));

    let owned =
        super::OwnedProcess::capture(&supervisor.birth()).expect("capture exact supervisor");
    owned
        .signal(Signal::SIGKILL)
        .expect("kill stable supervisor");
    let deadline = Instant::now() + Duration::from_secs(2);
    while super::observe_process_birth(supervisor.pid)
        .expect("observe supervisor exit")
        .is_some()
        && Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        super::observe_process_birth(harness.pid)
            .expect("observe surviving harness")
            .is_some(),
        "exec-replaced harness must survive supervisor death"
    );
    cleanup.crash_parent();

    let plan = super::parse_direct_command_plan(command).expect("recovery plan");
    let recovered = super::execute_direct_plan(
        &fixture.repo,
        &plan,
        &artifact_root,
        None,
        Duration::from_secs(5),
    )
    .expect("restart must clean exact-birth harness and retry");

    assert!(
        super::observe_process_birth(harness.pid)
            .expect("post-recovery harness")
            .is_none(),
        "exec-replaced harness survived restart cleanup"
    );
    assert!(!launch.exists());
    assert_eq!(recovered[0].terminal, super::AttemptTerminal::Exited(0));
    cleanup.disarm();
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_restart_cleans_shebang_harness_after_supervisor_death() {
    // Break caught: cleanup recovery requiring the declared script path after Linux replaces
    // the live harness identity with its shebang interpreter.
    let fixture = GitFixture::new("direct-dead-supervisor-shebang");
    let marker = fixture.root.join("command.pid");
    let script = fixture.root.join("fixture-command");
    write_executable(
        &script,
        &format!(
            "#!/bin/sh\nif [ ! -e '{}' ]; then printf '%s' \"$$\" > '{}'; while :; do sleep 1; done; fi\n",
            marker.display(),
            marker.display()
        ),
    );

    assert_exec_replaced_direct_harness_recovers(
        &fixture,
        script.to_str().expect("script path"),
        &marker,
    );
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_restart_cleans_immediate_exec_harness_after_supervisor_death() {
    // Break caught: cleanup recovery rejecting a same-birth harness after the declared shell
    // immediately replaces itself with another executable.
    let fixture = GitFixture::new("direct-dead-supervisor-immediate-exec");
    let marker = fixture.root.join("command.pid");
    let command = format!(
        "/bin/sh -c 'if [ ! -e \"{}\" ]; then printf %s \"$$\" > \"{}\"; exec /usr/bin/sleep 30; fi'",
        marker.display(),
        marker.display()
    );

    assert_exec_replaced_direct_harness_recovers(&fixture, &command, &marker);
}

#[test]
fn autonomous_executor_bridge_every_required_scanner_fails_closed_when_missing() {
    for missing in ["gitleaks", "semgrep", "trivy", "license-checker"] {
        let mut paths = BTreeMap::new();
        for scanner in ["gitleaks", "semgrep", "trivy", "license-checker"] {
            if scanner != missing {
                paths.insert(scanner.to_string(), PathBuf::from("/usr/bin/true"));
            }
        }
        let error = super::ScannerExecutables::from_paths(paths)
            .expect_err("missing required scanner must fail closed");
        assert!(error.contains(missing), "{missing}: {error}");
    }
}

#[test]
fn autonomous_executor_bridge_every_degraded_or_failing_scanner_blocks() {
    let fixture = GitFixture::new("scanner-status");
    for scanner in ["gitleaks", "semgrep", "trivy", "license-checker"] {
        let degraded =
            super::validate_scanner_result(scanner, 0, b"", b"scanner degraded fallback")
                .expect_err("degraded scanner output must fail closed");
        assert!(degraded.contains(scanner), "{degraded}");

        let failed = super::validate_scanner_result(scanner, 1, b"{}", b"")
            .expect_err("failing scanner must fail closed");
        assert!(failed.contains(scanner), "{failed}");

        let generic = super::validate_scanner_result(scanner, 0, b"{}", b"")
            .expect_err("generic JSON must not impersonate scanner-native evidence");
        assert!(generic.contains(scanner), "{scanner}: {generic}");
    }
    drop(fixture);
}

#[test]
fn autonomous_executor_bridge_scanner_results_require_native_clean_schemas() {
    let clean = [
        ("gitleaks", br#"[]"#.as_slice()),
        (
            "semgrep",
            br#"{"results":[],"errors":[],"paths":{"scanned":["feature.js"],"skipped":[]},"version":"1.0"}"#
                .as_slice(),
        ),
        (
            "trivy",
            br#"{"SchemaVersion":2,"Results":[{"Target":".","Vulnerabilities":[],"Misconfigurations":[],"Secrets":[]}]}"#
                .as_slice(),
        ),
        (
            "license-checker",
            br#"{"fixture@1.0.0":{"licenses":"MIT","repository":"https://example.invalid"}}"#
                .as_slice(),
        ),
    ];
    for (scanner, output) in clean {
        super::validate_scanner_result(scanner, 0, output, b"")
            .unwrap_or_else(|error| panic!("{scanner} native clean output rejected: {error}"));
    }

    let findings = [
        ("gitleaks", br#"[{"RuleID":"secret"}]"#.as_slice()),
        (
            "semgrep",
            br#"{"results":[{"check_id":"rule"}],"errors":[],"paths":{"scanned":["feature.js"],"skipped":[]}}"#
                .as_slice(),
        ),
        (
            "trivy",
            br#"{"Results":[{"Target":".","Vulnerabilities":[{"VulnerabilityID":"CVE-1"}]}]}"#
                .as_slice(),
        ),
        (
            "license-checker",
            br#"{"fixture@1.0.0":{"licenses":"GPL-3.0"}}"#.as_slice(),
        ),
    ];
    for (scanner, output) in findings {
        let error = super::validate_scanner_result(scanner, 0, output, b"")
            .expect_err("native finding must block");
        assert!(error.contains(scanner), "{scanner}: {error}");
        assert!(error.contains("reported"), "{scanner}: {error}");
    }
    let semgrep_finding = br#"{"results":[{"check_id":"rule"}],"errors":[],"paths":{"scanned":["feature.js"],"skipped":[]}}"#;
    let error = super::validate_scanner_result("semgrep", 1, semgrep_finding, b"")
        .expect_err("Semgrep finding exit must reach native JSON validation");
    assert!(error.contains("reported findings"), "{error}");
    let error = super::validate_scanner_result(
        "semgrep",
        1,
        br#"{"results":[],"errors":[],"paths":{"scanned":["feature.js"],"skipped":[]}}"#,
        b"",
    )
    .expect_err("empty Semgrep JSON must not legitimize a non-zero exit");
    assert!(error.contains("exit status 1"), "{error}");
    let error = super::validate_scanner_result(
        "semgrep",
        0,
        br#"{"results":[],"errors":[],"paths":{"scanned":[],"skipped":[{"path":"large.rs","reason":"exceeded_size_limit"}]}}"#,
        b"",
    )
    .expect_err("a successful Semgrep process must not hide skipped changed files");
    assert!(
        error.contains("scanned no files") || error.contains("skipped files"),
        "{error}"
    );

    let wrong_shape = [
        ("gitleaks", br#"{"results":[],"errors":[]}"#.as_slice()),
        ("semgrep", br#"[]"#.as_slice()),
        ("trivy", br#"{"results":[],"errors":[]}"#.as_slice()),
        (
            "license-checker",
            br#"{"results":[],"errors":[]}"#.as_slice(),
        ),
    ];
    for (scanner, output) in wrong_shape {
        let error = super::validate_scanner_result(scanner, 0, output, b"")
            .expect_err("another tool's JSON shape must not be accepted");
        assert!(error.contains(scanner), "{scanner}: {error}");
    }
}

#[test]
fn autonomous_executor_bridge_changed_paths_preserve_git_status_classes() {
    let changed = super::parse_changed_paths(
        b"A\0added\0D\0deleted\0M\0modified\0R100\0old\0new\0C100\0source\0copy\0T\0typed\0",
    )
    .expect("parse Git name-status output");

    for path in [
        "added", "deleted", "modified", "old", "new", "copy", "typed",
    ] {
        assert!(changed.all.contains(path), "missing changed path {path}");
    }
    for path in ["added", "new", "copy"] {
        assert!(changed.added.contains(path), "missing added path {path}");
    }
    for path in ["deleted", "old"] {
        assert!(
            changed.deleted.contains(path),
            "missing deleted path {path}"
        );
    }
    assert!(changed.type_changed.contains("typed"));
    assert!(!changed.all.contains("source"));
}

#[test]
fn autonomous_executor_bridge_changed_paths_reject_malformed_name_status() {
    for (output, expected) in [
        (b"X\0path\0".as_slice(), "status is unsupported"),
        (b"R100\0old\0".as_slice(), "output is truncated"),
        (b"\0path\0".as_slice(), "empty status"),
        (b"\xff\0path\0".as_slice(), "status is not valid UTF-8"),
        (b"M\0\xff\0".as_slice(), "path is not valid UTF-8"),
    ] {
        let error = super::parse_changed_paths(output)
            .expect_err("malformed Git name-status output must fail");
        assert!(error.contains(expected), "{error}");
    }
}

#[test]
fn autonomous_executor_bridge_npm_dependency_inputs_follow_manifest_policy() {
    // Break caught: treating scripts as graph input, or treating any other top-level
    // manifest field as irrelevant, weakens the conservative npm classification policy.
    let fixture = GitFixture::new("npm-dependency-inputs-manifest-policy");
    fs::write(
        fixture.repo.join("package.json"),
        r#"{"scripts":{"test":"old"}}"#,
    )
    .expect("baseline manifest");
    git(&fixture.repo, &["add", "package.json"]);
    git(&fixture.repo, &["commit", "-m", "baseline manifest"]);
    let base_oid = git_stdout(&fixture.repo, &["rev-parse", "HEAD"]);

    for (field, current, expected) in [
        ("scripts", r#"{"scripts":{"test":"new"}}"#, false),
        (
            "dependencies",
            r#"{"scripts":{"test":"old"},"dependencies":{"fixture":"1"}}"#,
            true,
        ),
        (
            "devDependencies",
            r#"{"scripts":{"test":"old"},"devDependencies":{"fixture":"1"}}"#,
            true,
        ),
        (
            "optionalDependencies",
            r#"{"scripts":{"test":"old"},"optionalDependencies":{"fixture":"1"}}"#,
            true,
        ),
        (
            "peerDependencies",
            r#"{"scripts":{"test":"old"},"peerDependencies":{"fixture":"1"}}"#,
            true,
        ),
        (
            "peerDependenciesMeta",
            r#"{"scripts":{"test":"old"},"peerDependenciesMeta":{"fixture":{"optional":true}}}"#,
            true,
        ),
        (
            "overrides",
            r#"{"scripts":{"test":"old"},"overrides":{"fixture":"2"}}"#,
            true,
        ),
        (
            "version",
            r#"{"scripts":{"test":"old"},"version":"2.0.0"}"#,
            true,
        ),
        (
            "unknown",
            r#"{"scripts":{"test":"old"},"autospecUnknown":true}"#,
            true,
        ),
    ] {
        fs::write(fixture.repo.join("package.json"), current).expect("current manifest");
        git(&fixture.repo, &["add", "package.json"]);
        git(&fixture.repo, &["commit", "-m", field]);
        let changed = super::changed_paths_since_base(&fixture.repo, &base_oid)
            .expect("changed manifest paths");

        assert_eq!(
            super::npm_dependency_inputs_changed(&fixture.repo, &base_oid, &changed)
                .expect("manifest classification"),
            expected,
            "{field}"
        );
    }
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_npm_dependency_inputs_preserve_git_status_classes() {
    // Break caught: dropping a lockfile identity on modify, delete, rename, copy, or
    // type change can misclassify a runtime dependency graph change as irrelevant.
    for lockfile in [
        "package-lock.json",
        "npm-shrinkwrap.json",
        "pnpm-lock.yaml",
        "yarn.lock",
    ] {
        let fixture = GitFixture::new(&format!("npm-dependency-inputs-{lockfile}"));
        fs::write(fixture.repo.join(lockfile), "baseline\n").expect("baseline lockfile");
        git(&fixture.repo, &["add", lockfile]);
        git(&fixture.repo, &["commit", "-m", "baseline lockfile"]);
        let base_oid = git_stdout(&fixture.repo, &["rev-parse", "HEAD"]);
        fs::write(fixture.repo.join(lockfile), "changed\n").expect("changed lockfile");
        git(&fixture.repo, &["add", lockfile]);
        git(&fixture.repo, &["commit", "-m", "change lockfile"]);
        let changed = super::changed_paths_since_base(&fixture.repo, &base_oid)
            .expect("changed lockfile paths");

        assert!(
            super::npm_dependency_inputs_changed(&fixture.repo, &base_oid, &changed)
                .expect("lockfile classification"),
            "{lockfile}"
        );
    }

    let deleted = GitFixture::new("npm-dependency-inputs-deleted");
    fs::write(deleted.repo.join("package.json"), r#"{"name":"fixture"}"#)
        .expect("baseline manifest");
    git(&deleted.repo, &["add", "package.json"]);
    git(&deleted.repo, &["commit", "-m", "baseline manifest"]);
    let base_oid = git_stdout(&deleted.repo, &["rev-parse", "HEAD"]);
    git(&deleted.repo, &["rm", "package.json"]);
    git(&deleted.repo, &["commit", "-m", "delete manifest"]);
    let changed = super::changed_paths_since_base(&deleted.repo, &base_oid)
        .expect("deleted manifest paths");
    assert!(
        super::npm_dependency_inputs_changed(&deleted.repo, &base_oid, &changed)
            .expect("deleted manifest classification")
    );

    let renamed = GitFixture::new("npm-dependency-inputs-renamed");
    fs::write(renamed.repo.join("package-lock.json"), "{}\n").expect("baseline lockfile");
    git(&renamed.repo, &["add", "package-lock.json"]);
    git(&renamed.repo, &["commit", "-m", "baseline lockfile"]);
    let base_oid = git_stdout(&renamed.repo, &["rev-parse", "HEAD"]);
    git(
        &renamed.repo,
        &["mv", "package-lock.json", "archived-lock.json"],
    );
    git(&renamed.repo, &["commit", "-m", "rename lockfile"]);
    let changed = super::changed_paths_since_base(&renamed.repo, &base_oid)
        .expect("renamed lockfile paths");
    assert!(
        super::npm_dependency_inputs_changed(&renamed.repo, &base_oid, &changed)
            .expect("renamed lockfile classification")
    );

    let copied = GitFixture::new("npm-dependency-inputs-copied");
    fs::write(copied.repo.join("source-lock.json"), "{}\n").expect("baseline source");
    git(&copied.repo, &["add", "source-lock.json"]);
    git(&copied.repo, &["commit", "-m", "baseline source"]);
    let base_oid = git_stdout(&copied.repo, &["rev-parse", "HEAD"]);
    fs::copy(
        copied.repo.join("source-lock.json"),
        copied.repo.join("package-lock.json"),
    )
    .expect("copy lockfile");
    git(&copied.repo, &["add", "package-lock.json"]);
    git(&copied.repo, &["commit", "-m", "copy lockfile"]);
    let changed = super::changed_paths_since_base(&copied.repo, &base_oid)
        .expect("copied lockfile paths");
    assert!(
        super::npm_dependency_inputs_changed(&copied.repo, &base_oid, &changed)
            .expect("copied lockfile classification")
    );

    let typed = GitFixture::new("npm-dependency-inputs-type-changed");
    fs::write(typed.repo.join("package-lock.json"), "{}\n").expect("baseline lockfile");
    git(&typed.repo, &["add", "package-lock.json"]);
    git(&typed.repo, &["commit", "-m", "baseline lockfile"]);
    let base_oid = git_stdout(&typed.repo, &["rev-parse", "HEAD"]);
    fs::remove_file(typed.repo.join("package-lock.json")).expect("remove lockfile");
    symlink("README.md", typed.repo.join("package-lock.json")).expect("symlink lockfile");
    git(&typed.repo, &["add", "package-lock.json"]);
    git(&typed.repo, &["commit", "-m", "type change lockfile"]);
    let changed = super::changed_paths_since_base(&typed.repo, &base_oid)
        .expect("type-changed lockfile paths");
    assert!(changed.type_changed.contains("package-lock.json"));
    assert!(
        super::npm_dependency_inputs_changed(&typed.repo, &base_oid, &changed)
            .expect("type-changed lockfile classification")
    );
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_npm_dependency_inputs_fail_closed_on_bad_evidence() {
    // Break caught: malformed, unsafe, unreadable, or unattributable package.json
    // evidence silently becoming an absent manifest and weakening classification.
    let fixture = GitFixture::new("npm-dependency-inputs-bad-evidence");
    fs::write(fixture.repo.join("package.json"), r#"{"name":"fixture"}"#)
        .expect("baseline manifest");
    git(&fixture.repo, &["add", "package.json"]);
    git(&fixture.repo, &["commit", "-m", "baseline manifest"]);
    let base_oid = git_stdout(&fixture.repo, &["rev-parse", "HEAD"]);

    fs::write(fixture.repo.join("package.json"), b"{").expect("malformed manifest");
    git(&fixture.repo, &["add", "package.json"]);
    git(&fixture.repo, &["commit", "-m", "malformed manifest"]);
    let changed = super::changed_paths_since_base(&fixture.repo, &base_oid)
        .expect("malformed manifest paths");
    let error = super::npm_dependency_inputs_changed(&fixture.repo, &base_oid, &changed)
        .expect_err("malformed JSON must fail closed");
    assert!(error.contains("parse current package.json:"), "{error}");

    fs::write(fixture.repo.join("package.json"), "[]\n").expect("non-object manifest");
    git(&fixture.repo, &["add", "package.json"]);
    git(&fixture.repo, &["commit", "-m", "non-object manifest"]);
    let changed = super::changed_paths_since_base(&fixture.repo, &base_oid)
        .expect("non-object manifest paths");
    assert_eq!(
        super::npm_dependency_inputs_changed(&fixture.repo, &base_oid, &changed)
            .expect_err("non-object JSON must fail closed"),
        "current package.json is not a JSON object"
    );

    fs::write(fixture.repo.join("package.json"), r#"{"name":"changed"}"#)
        .expect("changed manifest");
    git(&fixture.repo, &["add", "package.json"]);
    git(&fixture.repo, &["commit", "-m", "changed manifest"]);
    let changed = super::changed_paths_since_base(&fixture.repo, &base_oid)
        .expect("changed manifest paths");
    let manifest = fixture.repo.join("package.json");
    let mut permissions = fs::metadata(&manifest)
        .expect("manifest metadata")
        .permissions();
    permissions.set_mode(0o000);
    fs::set_permissions(&manifest, permissions).expect("make manifest unreadable");
    let result = super::npm_dependency_inputs_changed(&fixture.repo, &base_oid, &changed);
    let mut permissions = fs::metadata(&manifest)
        .expect("manifest metadata")
        .permissions();
    permissions.set_mode(0o600);
    fs::set_permissions(&manifest, permissions).expect("restore manifest permissions");
    let error = result.expect_err("unreadable current manifest must fail closed");
    assert!(error.contains("read current package.json:"), "{error}");

    let error = super::npm_dependency_inputs_changed(&fixture.repo, "missing-base", &changed)
        .expect_err("missing base evidence must fail closed");
    assert!(error.contains("read base package.json:"), "{error}");

    fs::remove_file(&manifest).expect("remove regular manifest");
    symlink("README.md", &manifest).expect("unsafe manifest symlink");
    git(&fixture.repo, &["add", "package.json"]);
    git(&fixture.repo, &["commit", "-m", "symlink manifest"]);
    let changed = super::changed_paths_since_base(&fixture.repo, &base_oid)
        .expect("symlink manifest paths");
    let error = super::npm_dependency_inputs_changed(&fixture.repo, &base_oid, &changed)
        .expect_err("unsafe manifest symlink must fail closed");
    assert!(error.contains("current package.json is unsafe:"), "{error}");
    assert!(error.contains("path contains a symlink"), "{error}");
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_npm_dependency_inputs_reject_manifest_swap_before_open() {
    // Break caught: validating package.json by path and reopening it lets a symlink swap
    // redirect the classifier to attacker-controlled dependency data.
    let fixture = GitFixture::new("npm-dependency-inputs-open-swap");
    let manifest = fixture.repo.join("package.json");
    fs::write(
        &manifest,
        r#"{"dependencies":{"fixture":"1"},"scripts":{"test":"old"}}"#,
    )
    .expect("baseline manifest");
    git(&fixture.repo, &["add", "package.json"]);
    git(&fixture.repo, &["commit", "-m", "baseline manifest"]);
    let base_oid = git_stdout(&fixture.repo, &["rev-parse", "HEAD"]);
    fs::write(
        &manifest,
        r#"{"dependencies":{"fixture":"1"},"scripts":{"test":"new"}}"#,
    )
    .expect("scripts-only manifest");
    git(&fixture.repo, &["add", "package.json"]);
    git(&fixture.repo, &["commit", "-m", "scripts-only change"]);
    let changed = super::changed_paths_since_base(&fixture.repo, &base_oid)
        .expect("changed manifest paths");
    super::NPM_MANIFEST_OPEN_FAILPOINT.store(1, Ordering::SeqCst);
    let repo = fixture.repo.clone();
    let classifier =
        thread::spawn(move || super::npm_dependency_inputs_changed(&repo, &base_oid, &changed));
    let deadline = Instant::now() + Duration::from_secs(5);
    while super::NPM_MANIFEST_OPEN_FAILPOINT.load(Ordering::SeqCst) != 2 {
        assert!(
            Instant::now() < deadline,
            "classifier did not reach open boundary"
        );
        thread::yield_now();
    }
    fs::rename(&manifest, fixture.repo.join("package.original.json"))
        .expect("move validated manifest");
    let attacker = fixture.root.join("attacker-package.json");
    fs::write(&attacker, r#"{"dependencies":{"fixture":"2"}}"#).expect("attacker manifest");
    symlink(&attacker, &manifest).expect("replace manifest with symlink");
    super::NPM_MANIFEST_OPEN_FAILPOINT.store(3, Ordering::SeqCst);

    let error = classifier
        .join()
        .expect("classifier thread")
        .expect_err("manifest swap must fail closed");
    assert!(error.contains("current package.json"), "{error}");
    super::NPM_MANIFEST_OPEN_FAILPOINT.store(0, Ordering::SeqCst);
}

#[test]
fn autonomous_executor_bridge_restart_reruns_all_scanner_results() {
    let _environment = TEST_ENVIRONMENT.lock().expect("test environment lock");
    // Break caught: successful scanner records recovered from disk becoming authoritative
    // security evidence without re-executing each scanner in the current process.
    let fixture = GitFixture::new("scanner-recovery");
    let bin = fixture.root.join("scanner-bin");
    fs::create_dir_all(&bin).expect("scanner bin");
    let mut paths = BTreeMap::new();
    for (scanner, output) in [
        ("gitleaks", "[]"),
        (
            "semgrep",
            r#"{"results":[],"errors":[],"paths":{"scanned":["feature.js"],"skipped":[]}}"#,
        ),
        ("trivy", r#"{"Results":[{"Target":"."}]}"#),
        ("license-checker", r#"{"fixture@1.0.0":{"licenses":"MIT"}}"#),
    ] {
        let executable = bin.join(scanner);
        let count = bin.join(format!("{scanner}.count"));
        let result_logic = if scanner == "gitleaks" {
            format!(
                "report=''\nwhile [ \"$#\" -gt 0 ]; do if [ \"$1\" = --report-path ]; then report=\"$2\"; shift 2; else shift; fi; done\nprintf '%s' '{}' > \"$report\"\n",
                output
            )
        } else {
            format!("printf '%s' '{}'\n", output)
        };
        write_executable(
            &executable,
            &format!(
                "#!/bin/sh\nset -eu\nn=0\n[ ! -f '{}' ] || n=$(cat '{}')\nn=$((n+1))\nprintf '%s' \"$n\" > '{}'\n{}",
                count.display(),
                count.display(),
                count.display(),
                result_logic
            ),
        );
        paths.insert(scanner.to_string(), executable);
    }
    let scanners = super::ScannerExecutables::from_paths(paths).expect("scanner paths");
    let artifact_root = fixture.root.join("scanner-evidence");

    let first = super::run_required_scanners(
        &fixture.repo,
        &git_stdout(&fixture.repo, &["rev-parse", "HEAD"]),
        &artifact_root,
        &scanners,
        None,
        Duration::from_secs(5),
    )
    .expect("first scanner pass");
    let second = super::run_required_scanners(
        &fixture.repo,
        &git_stdout(&fixture.repo, &["rev-parse", "HEAD"]),
        &artifact_root,
        &scanners,
        None,
        Duration::from_secs(5),
    )
    .expect("adopt scanner pass");

    assert_eq!(first.len(), second.len());
    for scanner in ["gitleaks", "semgrep", "trivy", "license-checker"] {
        assert_eq!(
            fs::read_to_string(bin.join(format!("{scanner}.count"))).expect("scanner count"),
            "2",
            "{scanner} did not rerun after its durable terminal record"
        );
    }
}

#[test]
fn autonomous_executor_bridge_policy_digest_prevents_command_replay() {
    // Break caught: an identical scanner argv recovering a terminal record created under
    // different generated-policy content.
    let fixture = GitFixture::new("scanner-policy-command-identity");
    let artifact_root = fixture.root.join("command-evidence");
    let command = |digest: &str| {
        let mut command = super::DirectCommand::success(vec!["/usr/bin/true".to_string()]);
        command.identity_digest = Some(digest.to_string());
        super::DirectCommandPlan {
            commands: vec![command],
        }
    };

    super::execute_direct_plan(
        &fixture.repo,
        &command(&"a".repeat(64)),
        &artifact_root,
        None,
        Duration::from_secs(5),
    )
    .expect("first policy-bound command");
    let error = super::execute_direct_plan(
        &fixture.repo,
        &command(&"b".repeat(64)),
        &artifact_root,
        None,
        Duration::from_secs(5),
    )
    .expect_err("different policy digest must not replay the old terminal record");

    assert!(error.contains("invocation intent"), "{error}");
}

#[test]
fn autonomous_executor_bridge_scanner_argv_is_direct_and_fail_closed() {
    let worktree = Path::new("/safe/worktree");
    let config = Path::new("/safe/evidence/gitleaks-policy.toml");
    let report = Path::new("/safe/evidence/gitleaks.json");
    let expected = [
        (
            "gitleaks",
            vec![
                "/scanner/gitleaks",
                "detect",
                "--no-git",
                "--no-banner",
                "--redact",
                "--source",
                "/safe/worktree",
                "--config",
                "/safe/evidence/gitleaks-policy.toml",
                "--report-format",
                "json",
                "--report-path",
                "/safe/evidence/gitleaks.json",
            ],
        ),
        (
            "semgrep",
            vec![
                "/scanner/semgrep",
                "scan",
                "--config",
                "p/default",
                "--metrics",
                "off",
                "--error",
                "--json",
                "--verbose",
                "--max-target-bytes",
                "0",
                "--timeout",
                "0",
                "--timeout-threshold",
                "0",
                "--baseline-commit",
                "base-oid",
                "/safe/worktree",
            ],
        ),
        (
            "trivy",
            vec![
                "/scanner/trivy",
                "fs",
                "--quiet",
                "--format",
                "json",
                "--exit-code",
                "1",
                "/safe/worktree",
            ],
        ),
        (
            "license-checker",
            vec![
                "/scanner/license-checker",
                "--json",
                "--production",
                "--start",
                "/safe/worktree",
            ],
        ),
    ];
    for (scanner, argv) in expected {
        assert_eq!(
            super::scanner_command(
                scanner,
                Path::new(argv[0]),
                worktree,
                "base-oid",
                config,
                report,
            )
            .expect("scanner command")
            .argv,
            argv
        );
    }
}

#[test]
fn autonomous_executor_bridge_scanner_command_semgrep_is_private_and_baseline_scoped() {
    // Break caught: `--config auto --metrics off` is rejected by Semgrep before scanning,
    // while an unscoped repository scan also blocks feature work on pre-existing findings.
    let command = super::scanner_command(
        "semgrep",
        Path::new("/scanner/semgrep"),
        Path::new("/safe/worktree"),
        "base-oid",
        Path::new("/safe/evidence/gitleaks-policy.toml"),
        Path::new("/safe/evidence/gitleaks.json"),
    )
    .expect("Semgrep command");

    assert!(
        command
            .argv
            .windows(2)
            .any(|pair| pair == ["--config", "p/default"]),
        "{:?}",
        command.argv
    );
    assert!(
        command
            .argv
            .iter()
            .any(|argument| argument == "--baseline-commit"),
        "{:?}",
        command.argv
    );
    assert!(
        command
            .argv
            .windows(2)
            .any(|pair| pair == ["--metrics", "off"]),
        "{:?}",
        command.argv
    );
    assert!(
        command
            .argv
            .windows(2)
            .any(|pair| pair == ["--max-target-bytes", "0"]),
        "{:?}",
        command.argv
    );
    assert_eq!(command.accepted_exit_codes, vec![0, 1]);
}

#[test]
fn autonomous_executor_bridge_scanner_command_semgrep_baseline_is_diff_scoped() {
    let _environment = TEST_ENVIRONMENT.lock().expect("test environment lock");
    // Break caught: a repository-wide scan blocking a feature on findings already present
    // in its claimed base commit, instead of evaluating only feature-introduced findings.
    let fixture = GitFixture::new("semgrep-baseline");
    let rule = fixture.root.join("semgrep-rule.yml");
    fs::write(
        &rule,
        r#"rules:
  - id: autospec-test-dangerous-call
languages:
  - generic
message: deterministic test finding
severity: ERROR
pattern-regex: dangerous_call
"#,
    )
    .expect("deterministic Semgrep rule");
    fs::write(fixture.repo.join("old.js"), "dangerous_call('old');\n")
        .expect("pre-existing finding");
    git(&fixture.repo, &["add", "old.js"]);
    git(&fixture.repo, &["commit", "-m", "baseline finding"]);
    let base_oid = git_stdout(&fixture.repo, &["rev-parse", "HEAD"]);
    fs::write(
        fixture.repo.join("feature.js"),
        "safe_call('feature');\n".repeat(60_000),
    )
    .expect("clean feature larger than Semgrep's default 1 MB limit");
    git(&fixture.repo, &["add", "feature.js"]);
    git(&fixture.repo, &["commit", "-m", "clean feature"]);
    let semgrep = super::resolve_direct_executable(&fixture.repo, "semgrep")
        .expect("real Semgrep")
        .program;
    let scan = |artifact: &str| {
        let mut command = super::DirectCommand::success(vec![
            semgrep.display().to_string(),
            "scan".to_string(),
            "--config".to_string(),
            rule.display().to_string(),
            "--metrics".to_string(),
            "off".to_string(),
            "--error".to_string(),
            "--json".to_string(),
            "--verbose".to_string(),
            "--max-target-bytes".to_string(),
            "0".to_string(),
            "--timeout".to_string(),
            "0".to_string(),
            "--timeout-threshold".to_string(),
            "0".to_string(),
            "--baseline-commit".to_string(),
            base_oid.clone(),
            ".".to_string(),
        ]);
        command.accepted_exit_codes = vec![0, 1];
        let observed = super::execute_direct_plan(
            &fixture.repo,
            &super::DirectCommandPlan {
                commands: vec![command],
            },
            &fixture.root.join(artifact),
            None,
            Duration::from_secs(30),
        )
        .expect("Semgrep process observation");
        let command = &observed[0];
        let stdout = fs::read(&command.stdout_path).expect("Semgrep JSON");
        let stderr = fs::read(&command.stderr_path).expect("Semgrep diagnostics");
        (
            command.exit_code().expect("Semgrep exit status"),
            stdout,
            stderr,
        )
    };

    let (exit_status, stdout, stderr) = scan("clean-scan");
    super::validate_scanner_result("semgrep", exit_status, &stdout, &stderr).unwrap_or_else(
        |error| {
            panic!(
                "pre-existing finding is outside the feature diff: {error}; {}",
                String::from_utf8_lossy(&stderr)
            )
        },
    );

    fs::write(
        fixture.repo.join("feature.js"),
        "dangerous_call('feature');\n",
    )
    .expect("new finding");
    git(&fixture.repo, &["add", "feature.js"]);
    git(
        &fixture.repo,
        &["commit", "-m", "introduce feature finding"],
    );
    let (exit_status, stdout, stderr) = scan("finding-scan");
    let error = super::validate_scanner_result("semgrep", exit_status, &stdout, &stderr)
        .expect_err("feature-introduced finding must block");
    assert!(error.contains("reported findings"), "{error}");
}

#[test]
fn autonomous_executor_bridge_command_artifact_digest_and_commit_tamper_block() {
    let fixture = GitFixture::new("command-evidence-tamper");
    let artifacts = fixture.root.join("evidence");
    let plan =
        super::parse_direct_command_plan("/usr/bin/printf exact").expect("direct command plan");
    let records = super::execute_direct_plan(
        &fixture.repo,
        &plan,
        &artifacts,
        None,
        Duration::from_secs(5),
    )
    .expect("observed command");
    super::validate_observed_command(&fixture.repo, &records[0])
        .expect("untampered observation");

    fs::write(&records[0].stdout_path, "tampered").expect("tamper command output");
    let error = super::validate_observed_command(&fixture.repo, &records[0])
        .expect_err("artifact digest tamper must fail");
    assert!(error.contains("digest"), "{error}");

    git(&fixture.repo, &["commit", "--allow-empty", "-m", "drift"]);
    let error = super::validate_observed_command(&fixture.repo, &records[0])
        .expect_err("commit drift must fail");
    assert!(error.contains("commit"), "{error}");
}

#[test]
fn autonomous_executor_bridge_observed_results_are_the_only_typed_pass_authority() {
    let fixture = GitFixture::new("typed-evidence");
    let commit = git_stdout(&fixture.repo, &["rev-parse", "HEAD"]);
    let lane = super::PremergeLaneIdentity::new(
        "test/repo",
        42,
        "worker-42",
        "claim-42",
        "main",
        commit.clone(),
    )
    .expect("typed lane");
    let mut qa = Vec::new();
    let mut scanners = Vec::new();
    for (index, scanner) in ["gitleaks", "semgrep", "trivy", "license-checker"]
        .into_iter()
        .enumerate()
    {
        let output = match scanner {
            "gitleaks" => "[]",
            "semgrep" => {
                r#"{"results":[],"errors":[],"paths":{"scanned":["feature.js"],"skipped":[]}}"#
            }
            "trivy" => r#"{"Results":[{"Target":"."}]}"#,
            "license-checker" => r#"{"fixture@1.0.0":{"licenses":"MIT"}}"#,
            _ => unreachable!(),
        };
        let plan = super::parse_direct_command_plan(&format!("/usr/bin/printf '{output}'"))
            .expect("native scanner JSON observation command");
        let records = super::execute_direct_plan(
            &fixture.repo,
            &plan,
            &fixture.root.join(format!("observed-{index}")),
            None,
            Duration::from_secs(5),
        )
        .expect("real observed process");
        if index == 0 {
            qa.push(records[0].clone());
        }
        scanners.push(super::ObservedScanner {
            name: scanner.to_string(),
            base_oid: git_stdout(&fixture.repo, &["rev-parse", "HEAD"]),
            command: records[0].clone(),
            result_path: records[0].stdout_path.clone(),
            result_digest: records[0].stdout_digest.clone(),
        });
    }

    let complete = super::typed_evidence_from_observed(
        &fixture.repo,
        &commit,
        &lane,
        Ok(&qa),
        Ok(&scanners),
        Some("PASS"),
        1_800_000_000,
    );
    assert!(matches!(complete.0.verdict, super::EvidenceVerdict::Pass));
    assert!(matches!(complete.1.verdict, super::EvidenceVerdict::Pass));

    let missing = super::typed_evidence_from_observed(
        &fixture.repo,
        &commit,
        &lane,
        Ok(&qa),
        Ok(&scanners[..3]),
        Some("PASS"),
        1_800_000_001,
    );
    assert!(
        !matches!(missing.1.verdict, super::EvidenceVerdict::Pass),
        "fabricated model Pass must not upgrade missing scanner evidence"
    );

    let failed = super::typed_evidence_from_observed(
        &fixture.repo,
        &commit,
        &lane,
        Err("full suite failed"),
        Ok(&scanners),
        Some("PASS"),
        1_800_000_002,
    );
    assert!(
        !matches!(failed.0.verdict, super::EvidenceVerdict::Pass),
        "fabricated model Pass must not upgrade failed QA evidence"
    );
    let lint_failed = super::typed_evidence_from_observed(
        &fixture.repo,
        &commit,
        &lane,
        Ok(&qa),
        Err("implementation lint failed"),
        Some("PASS"),
        1_800_000_003,
    );
    assert!(
        !matches!(lint_failed.1.verdict, super::EvidenceVerdict::Pass),
        "implementation-lint failure must block security Pass"
    );
}

#[test]
fn autonomous_executor_bridge_remote_snapshot_ignores_claim_ledger_refs() {
    let document = format!(
        "{}\trefs/heads/main\n{}\trefs/autospec/claims/issue-42\n",
        "a".repeat(40),
        "b".repeat(40)
    );

    assert_eq!(
        super::parse_bridge_remote_refs(&document).expect("claim ledger is separate authority"),
        BTreeMap::from([("refs/heads/main".to_string(), "a".repeat(40))])
    );
    assert!(
        super::parse_bridge_remote_refs(&format!(
            "{}\trefs/autospec/unowned/issue-42\n",
            "c".repeat(40)
        ))
        .is_err(),
        "only the exact claim-ledger namespace may be excluded"
    );
}

#[test]
fn autonomous_executor_bridge_requires_observed_exact_merged_state() {
    let document = r#"{
        "number":17,
        "state":"MERGED",
        "isDraft":false,
        "headRefOid":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "baseRefName":"main",
        "mergeCommit":{"oid":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}
    }"#;
    assert_eq!(
        super::parse_observed_merge(document, 17, &"a".repeat(40), "main",).expect("merged"),
        "b".repeat(40)
    );
    assert!(super::parse_observed_merge(
        &document.replace("\"MERGED\"", "\"OPEN\""),
        17,
        &"a".repeat(40),
        "main",
    )
    .is_err());
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_reconciles_merged_existing_worktree_before_stale_proof() {
    let _environment = TEST_ENVIRONMENT.lock().expect("test environment lock");
    let (fixture, mut state, _snapshot, closeout) =
        implementation_proof_fixture("merged-existing-worktree-entrypoint");
    commit_implementation(&state);
    let persisted_head = git_stdout(&state.identity.worktree, &["rev-parse", "HEAD"]);
    state.phase = super::BridgePhase::DraftCreated;
    state.pr = Some(17);
    state.head_oid = Some(persisted_head.clone());
    state.closeout_path = Some(fs::canonicalize(&closeout).expect("canonical closeout"));
    let closeout_body = fs::read_to_string(&closeout).expect("read closeout");
    state.closeout_digest = Some(super::sha256_hex(closeout_body.as_bytes()));
    state.identity.runtime_environment_dir = None;
    state.identity.runtime_session_id = None;
    state.supervisor = None;
    state.process = None;
    state.draft_process = None;
    super::record_worktree_creation_identity(
        &state.identity.repository_path,
        &state.identity.branch,
        &ResolvedBase {
            base_ref: state.identity.base_ref.clone(),
            base_oid: state.identity.base_oid.clone(),
            explore_mode: false,
        },
    )
    .expect("record worktree creation identity");
    let claimed = autospec_core::claim::RunStateRecord::new(
        state.identity.repository.clone(),
        state.identity.issue,
        state.identity.worker_id.clone(),
        "claimed",
        state.identity.branch.clone(),
        "",
        "claimed",
        Vec::new(),
        "2026-08-04T00:00:00Z",
        "2026-08-04T00:00:00Z",
        999_999,
    )
    .with_claim_id(state.identity.claim_id.clone());
    assert!(crate::commands::claim::advance_claim_ref_for_test(
        &state.identity.repository_path,
        &claimed,
    )
    .expect("seed claimed generation"));

    fs::write(
        state.identity.worktree.join("reviewer-follow-up.txt"),
        "reviewer follow-up\n",
    )
    .expect("reviewer follow-up");
    git(&state.identity.worktree, &["add", "reviewer-follow-up.txt"]);
    git(
        &state.identity.worktree,
        &["commit", "-m", "test: reviewer follow-up"],
    );
    let merged_head = git_stdout(&state.identity.worktree, &["rev-parse", "HEAD"]);

    let state_path = fixture.root.join("state/invocation.json");
    super::write_invocation_atomic(&state_path, &state).expect("persist stale draft state");
    let observation = fixture.root.join("merged-observation.json");
    fs::write(
        &observation,
        serde_json::json!({
            "number": 17,
            "state": "MERGED",
            "isDraft": false,
            "headRefName": state.identity.branch,
            "headRefOid": merged_head,
            "baseRefName": "main",
            "mergeCommit": {"oid": "b".repeat(40)},
            "body": super::canonical_pull_request_body(&state, &closeout_body).unwrap(),
        })
        .to_string(),
    )
    .expect("merged observation");
    let gh = fixture.root.join("gh");
    fs::write(
        &gh,
        format!(
            "#!/bin/sh\nset -eu\n\
             if [ \"$1 $2\" = 'pr view' ]; then cat '{}'; exit 0; fi\n\
             if [ \"$1 $2\" = 'issue view' ]; then printf '%s\\n' '{{\"labels\":[]}}'; exit 0; fi\n\
             if [ \"$1 $2\" = 'issue edit' ] || [ \"$1 $2\" = 'issue comment' ]; then exit 0; fi\n\
             if [ \"$1\" = 'api' ]; then printf '%s\\n' '[]'; exit 0; fi\n\
             exit 64\n",
            observation.display()
        ),
    )
    .expect("gh fixture");
    fs::set_permissions(&gh, fs::Permissions::from_mode(0o755)).expect("gh mode");
    let failpoint = fixture.root.join("merged-reconciliation-failpoint");
    let previous_path = std::env::var_os("PATH");
    let previous_claim = std::env::var_os("AUTOSPEC_TEST_EXACT_EVIDENCE_CLAIM");
    let previous_failpoint = std::env::var_os("AUTOSPEC_TEST_MERGED_RECONCILIATION_FAIL_ONCE");
    let previous_claim_remote = std::env::var_os("AUTOSPEC_CLAIM_GIT_REMOTE");
    let previous_claim_state = std::env::var_os("AUTOSPEC_CLAIM_GIT_STATE_DIR");
    let previous_retry_sleep = std::env::var_os("AUTOSPEC_CLAIM_RETRY_SLEEP_MS");
    std::env::set_var(
        "PATH",
        format!(
            "{}:{}",
            fixture.root.display(),
            previous_path
                .as_deref()
                .unwrap_or_default()
                .to_string_lossy()
        ),
    );
    std::env::set_var("AUTOSPEC_TEST_EXACT_EVIDENCE_CLAIM", "1");
    std::env::set_var("AUTOSPEC_TEST_MERGED_RECONCILIATION_FAIL_ONCE", &failpoint);
    std::env::set_var("AUTOSPEC_CLAIM_GIT_REMOTE", fixture.root.join("remote.git"));
    std::env::set_var(
        "AUTOSPEC_CLAIM_GIT_STATE_DIR",
        fixture.root.join("claim-state"),
    );
    std::env::set_var("AUTOSPEC_CLAIM_RETRY_SLEEP_MS", "0");
    let request = super::ExecutorBridgeRequest {
        repository: state.identity.repository.clone(),
        repository_path: state.identity.repository_path.clone(),
        issue: state.identity.issue,
        issue_title: "Retire merged executor".to_string(),
        issue_body: DRAFT_ISSUE_BODY.to_string(),
        worker_id: state.identity.worker_id.clone(),
        claim_id: state.identity.claim_id.clone(),
        invocation_id: state.identity.invocation_id.clone(),
        state_path: state_path.clone(),
        event_log: fixture.root.join("events.jsonl"),
    };

    let outcome = super::run_executor_bridge_with_codex_probe(&request, |_| {
        panic!("merged recovery must precede Codex probing")
    });
    let error = outcome.expect_err("failpoint stops after merged reconciliation");
    assert!(
        error.to_string().contains("injected executor crash"),
        "{error}"
    );
    let durable = super::PersistedInvocation::from_json(
        &fs::read_to_string(&state_path).expect("read reconciled invocation"),
    )
    .expect("parse reconciled invocation");
    assert_eq!(durable.phase, super::BridgePhase::Merged);
    assert_eq!(durable.head_oid.as_deref(), Some(merged_head.as_str()));
    assert_eq!(
        durable.terminal_result.as_deref(),
        Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
    );
    assert!(super::cleanup_record_path(&state_path, "merged-reconciliation").exists());
    assert!(
        state.identity.worktree.exists(),
        "failpoint must precede cleanup"
    );
    let receipt = super::run_executor_bridge_with_codex_probe(&request, |_| {
        panic!("merged restart must finalize before Codex probing")
    })
    .expect("restart finalizes the reconciled merge");
    let complete = super::PersistedInvocation::from_json(
        &fs::read_to_string(&state_path).expect("read completed invocation"),
    )
    .expect("parse completed invocation");

    for (key, previous) in [
        ("PATH", previous_path),
        ("AUTOSPEC_TEST_EXACT_EVIDENCE_CLAIM", previous_claim),
        (
            "AUTOSPEC_TEST_MERGED_RECONCILIATION_FAIL_ONCE",
            previous_failpoint,
        ),
        ("AUTOSPEC_CLAIM_GIT_REMOTE", previous_claim_remote),
        ("AUTOSPEC_CLAIM_GIT_STATE_DIR", previous_claim_state),
        ("AUTOSPEC_CLAIM_RETRY_SLEEP_MS", previous_retry_sleep),
    ] {
        match previous {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }
    }

    assert!(matches!(
        receipt.status,
        super::BridgeRunStatus::Merged {
            pull_request: 17,
            ref head_oid,
            ref merge_oid,
        } if head_oid == &merged_head && merge_oid == &"b".repeat(40)
    ));
    assert_eq!(complete.phase, super::BridgePhase::Complete);
    assert!(!state.identity.worktree.exists());
    assert!(state_path.with_extension("terminal.json").is_file());
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_merged_reconciliation_is_exact_and_fail_closed() {
    let (fixture, mut state, _snapshot, _) =
        implementation_proof_fixture("merged-reconciliation-exact");
    commit_implementation(&state);
    let persisted_head = git_stdout(&state.identity.worktree, &["rev-parse", "HEAD"]);
    fs::write(
        state.identity.worktree.join("reviewer-follow-up.txt"),
        "reviewer follow-up\n",
    )
    .expect("reviewer follow-up");
    git(&state.identity.worktree, &["add", "reviewer-follow-up.txt"]);
    git(
        &state.identity.worktree,
        &["commit", "-m", "reviewer follow-up"],
    );
    let merged_head = git_stdout(&state.identity.worktree, &["rev-parse", "HEAD"]);
    state.phase = super::BridgePhase::DraftCreated;
    state.pr = Some(17);
    state.head_oid = Some(persisted_head.clone());
    state.supervisor = None;
    state.process = None;
    state.draft_process = None;
    state.umbrella = Some(42);
    state.current_child = Some(101);
    let closeout = "## Closeout report\n";
    state.closeout_digest = Some(autospec_core::autonomous::waterfall::sha256_hex(
        closeout.as_bytes(),
    ));
    let observation = fixture.root.join("merged-observation.json");
    let gh = fixture.root.join("gh-merged-reconciliation");
    fs::write(&gh, "#!/bin/sh\nset -eu\ncat \"$MERGED_OBSERVATION\"\n").expect("gh");
    fs::set_permissions(&gh, fs::Permissions::from_mode(0o755)).expect("gh mode");
    let adapter = super::DraftPrAdapter {
        gh,
        environment: BTreeMap::from([(
            "MERGED_OBSERVATION".into(),
            observation.clone().into_os_string(),
        )]),
    };
    let exact = serde_json::json!({
        "number": 17,
        "state": "MERGED",
        "isDraft": false,
        "headRefName": state.identity.branch,
        "headRefOid": merged_head,
        "baseRefName": "main",
        "mergeCommit": {"oid": "b".repeat(40)},
        "body": super::canonical_pull_request_body(&state, closeout).unwrap(),
    });
    for phase in [
        super::BridgePhase::Merged,
        super::BridgePhase::CleanupPending,
        super::BridgePhase::Complete,
    ] {
        state.phase = phase;
        for (body, accepted) in [("Closes #101", true), ("Closes #42", false)] {
            fs::write(&observation, exact.to_string().replace("Closes #101", body)).unwrap();
            assert_eq!(
                super::revalidate_live_canonical_pull_request(&state, &adapter).is_ok(),
                accepted
            );
        }
    }
    state.phase = super::BridgePhase::DraftCreated;
    fs::write(&observation, exact.to_string()).expect("exact observation");
    let state_path = fixture.root.join("state/exact.json");
    super::write_invocation_atomic(&state_path, &state).expect("persist pre-reconciliation");
    let mut reconciled = state.clone();
    assert!(super::reconcile_exact_merged_invocation_with_refresh(
        &state_path,
        &mut reconciled,
        &adapter,
        || Ok(super::BridgeClaimOwnership::Refreshed { ttl_seconds: 60 }),
    )
    .expect("exact merged reconciliation"));
    assert_eq!(reconciled.phase, super::BridgePhase::Merged);
    assert_eq!(reconciled.head_oid.as_deref(), Some(merged_head.as_str()));
    assert_eq!(
        reconciled.terminal_result.as_deref(),
        Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
    );
    super::validate_merged_reconciliation_record(&state_path, &reconciled)
        .expect("bound reconciliation record");
    let mut rebound = reconciled.clone();
    rebound.current_child = Some(102);
    assert!(super::validate_merged_reconciliation_record(&state_path, &rebound).is_err());

    for (name, mutated) in [
        ("open", serde_json::json!({"state": "OPEN"})),
        ("draft", serde_json::json!({"isDraft": true})),
        ("number", serde_json::json!({"number": 18})),
        (
            "branch",
            serde_json::json!({"headRefName": "feat/autonomous-issue-99"}),
        ),
        ("base", serde_json::json!({"baseRefName": "release"})),
        (
            "body",
            serde_json::json!({"body": format!("Closes #42\n\n{closeout}")}),
        ),
        ("head-oid", serde_json::json!({"headRefOid": "not-an-oid"})),
        (
            "merge-oid",
            serde_json::json!({"mergeCommit": {"oid": "not-an-oid"}}),
        ),
        (
            "local-head",
            serde_json::json!({"headRefOid": persisted_head}),
        ),
    ] {
        let mut document = exact.clone();
        let object = document.as_object_mut().expect("observation object");
        for (key, value) in mutated.as_object().expect("mutation object") {
            object.insert(key.clone(), value.clone());
        }
        fs::write(&observation, document.to_string()).expect("mutated observation");
        let mut candidate = state.clone();
        let candidate_path = fixture.root.join(format!("state/{name}.json"));
        let outcome = super::reconcile_exact_merged_invocation_with_refresh(
            &candidate_path,
            &mut candidate,
            &adapter,
            || Ok(super::BridgeClaimOwnership::Refreshed { ttl_seconds: 60 }),
        );
        if name == "open" {
            assert!(!outcome.expect("open PR is not terminal"));
        } else {
            assert!(outcome.is_err(), "{name} must fail closed");
        }
        assert_eq!(candidate, state, "{name} must not mutate invocation state");
        assert!(
            !candidate_path.exists(),
            "{name} must not publish terminal state"
        );
    }

    let reconciliation_path = super::cleanup_record_path(&state_path, "merged-reconciliation");
    let mut changed_record: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(&reconciliation_path).expect("read reconciliation"),
    )
    .expect("parse reconciliation");
    changed_record["persisted_head"] = serde_json::json!(state.identity.base_oid);
    fs::write(&reconciliation_path, changed_record.to_string())
        .expect("change reconciliation record");
    assert!(
        super::validate_merged_reconciliation_record(&state_path, &reconciled).is_err(),
        "changed persisted evidence must fail closed"
    );

    let base = state.identity.base_oid.clone();
    let tree = git_stdout(
        &state.identity.repository_path,
        &["rev-parse", &format!("{base}^{{tree}}")],
    );
    let divergent = git_stdout(
        &state.identity.repository_path,
        &["commit-tree", &tree, "-p", &base, "-m", "divergent head"],
    );
    git(
        &state.identity.repository_path,
        &[
            "update-ref",
            &format!("refs/heads/{}", state.identity.branch),
            &divergent,
            &merged_head,
        ],
    );
    let mut divergent_observation = exact.clone();
    divergent_observation["headRefOid"] = serde_json::json!(divergent);
    fs::write(&observation, divergent_observation.to_string()).expect("divergent observation");
    let mut nonancestor = state.clone();
    let nonancestor_path = fixture.root.join("state/nonancestor.json");
    let error = super::reconcile_exact_merged_invocation_with_refresh(
        &nonancestor_path,
        &mut nonancestor,
        &adapter,
        || Ok(super::BridgeClaimOwnership::Refreshed { ttl_seconds: 60 }),
    )
    .expect_err("non-ancestor merged head must fail closed");
    assert!(error.to_string().contains("not contained"), "{error}");
    assert_eq!(nonancestor, state);
    assert!(!nonancestor_path.exists());

    fs::write(&observation, exact.to_string()).expect("restore exact observation");
    let mut lost = state.clone();
    let lost_path = fixture.root.join("state/ownership-lost.json");
    let error = super::reconcile_exact_merged_invocation_with_refresh(
        &lost_path,
        &mut lost,
        &adapter,
        || Ok(super::BridgeClaimOwnership::Lost),
    )
    .expect_err("claim takeover must block reconciliation");
    assert!(error.to_string().contains("ownership"), "{error}");
    assert_eq!(lost, state);
    assert!(!lost_path.exists());
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_merged_reconciliation_waits_for_exact_live_process() {
    let (_fixture, mut state, _snapshot, _) =
        implementation_proof_fixture("merged-reconciliation-live-process");
    let args = vec!["30".to_string()];
    let executable = fs::canonicalize("/usr/bin/sleep").expect("sleep executable");
    let mut child = Command::new(&executable)
        .arg("30")
        .process_group(0)
        .spawn()
        .expect("spawn live executor fixture");
    let mut cleanup =
        DetachedForkedCleanup::new(child.id()).expect("arm live executor cleanup");
    let deadline = Instant::now() + Duration::from_secs(2);
    let identity = loop {
        if let Some(identity) =
            super::observe_process_identity(child.id(), &super::argv_digest(&args))
                .expect("observe live executor")
        {
            if identity.executable == executable
                && identity.argv_digest == super::argv_digest(&args)
                && identity.process_group == identity.pid
            {
                break identity;
            }
        }
        assert!(Instant::now() < deadline, "live executor was not observed");
        std::thread::sleep(Duration::from_millis(1));
    };
    cleanup.confirm_identity(identity.clone());
    state.supervisor = Some(identity);

    assert!(
        !super::executor_terminal_processes_are_quiescent(&state)
            .expect("inspect exact live process"),
        "remote terminal truth must not retire a generation while its exact process is live"
    );
    assert!(
        child
            .try_wait()
            .expect("inspect live executor fixture")
            .is_none(),
        "the quiescence gate must not mutate the live process"
    );
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_claim_takeover_blocks_admin_merge() {
    let (fixture, mut state, _snapshot, _) = implementation_proof_fixture("merge-takeover");
    commit_implementation(&state);
    let head = git_stdout(&state.identity.worktree, &["rev-parse", "HEAD"]);
    git(
        &state.identity.worktree,
        &[
            "push",
            "origin",
            &format!("{head}:refs/heads/{}", state.identity.branch),
        ],
    );
    state.phase = super::BridgePhase::ResultAccepted;
    state.pr = Some(17);
    state.head_oid = Some(head.clone());
    let closeout = "## Closeout report\n";
    state.closeout_digest = Some(autospec_core::autonomous::waterfall::sha256_hex(
        closeout.as_bytes(),
    ));
    let body =
        serde_json::to_string(&super::canonical_pull_request_body(&state, closeout).unwrap())
            .unwrap();
    let state_path = fixture.root.join("state/invocation.json");
    super::write_invocation_atomic(&state_path, &state).expect("accepted state");
    let gh = fixture.root.join("gh-merge");
    let calls = fixture.root.join("merge-calls");
    fs::write(
        &gh,
        format!("#!/bin/sh\nset -eu\nprintf '%s\\n' \"$*\" >> \"$GH_CALLS\"\n\
         if [ \"$1 $2\" = 'pr view' ]; then\n\
         printf '%s\\n' '{{\"number\":17,\"state\":\"OPEN\",\"isDraft\":false,\"headRefOid\":\"{head}\",\"baseRefName\":\"main\",\"mergeCommit\":null,\"body\":{body}}}'\n\
         exit 0\nfi\nexit 64\n"),
    )
    .expect("gh");
    fs::set_permissions(&gh, fs::Permissions::from_mode(0o755)).expect("gh mode");
    let adapter = super::DraftPrAdapter {
        gh,
        environment: BTreeMap::from([("GH_CALLS".into(), calls.clone().into_os_string())]),
    };

    let error = super::admin_squash_merge_exact_with_refresh_and_admission(
        &state_path,
        &mut state,
        &adapter,
        || Ok(super::BridgeClaimOwnership::Lost),
        || Ok(()),
    )
    .expect_err("takeover blocks merge");
    assert!(error.contains("ownership"), "{error}");
    assert!(!fs::read_to_string(calls)
        .expect("calls")
        .contains("pr merge"));
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_merge_failure_resumes_from_accepted_evidence() {
    let (fixture, mut state, _snapshot, _) = implementation_proof_fixture("merge-retry");
    commit_implementation(&state);
    let head = git_stdout(&state.identity.worktree, &["rev-parse", "HEAD"]);
    git(
        &state.identity.worktree,
        &[
            "push",
            "origin",
            &format!("{head}:refs/heads/{}", state.identity.branch),
        ],
    );
    state.phase = super::BridgePhase::ResultAccepted;
    state.pr = Some(17);
    state.head_oid = Some(head.clone());
    let closeout = "## Closeout report\n";
    state.closeout_digest = Some(autospec_core::autonomous::waterfall::sha256_hex(
        closeout.as_bytes(),
    ));
    let body =
        serde_json::to_string(&super::canonical_pull_request_body(&state, closeout).unwrap())
            .unwrap();
    let state_path = fixture.root.join("state/invocation.json");
    super::write_invocation_atomic(&state_path, &state).expect("accepted state");
    let gh = fixture.root.join("gh-merge-retry");
    let fail_once = fixture.root.join("fail-once");
    let merged = fixture.root.join("merged");
    fs::write(&fail_once, "").expect("fail marker");
    fs::write(
        &gh,
        format!(
            "#!/bin/sh\nset -eu\n\
             if [ \"$1 $2\" = 'pr view' ]; then\n\
               if [ -e \"$MERGED\" ]; then printf '%s\\n' '{{\"number\":17,\"state\":\"MERGED\",\"isDraft\":false,\"headRefOid\":\"{head}\",\"baseRefName\":\"main\",\"mergeCommit\":{{\"oid\":\"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\"}},\"body\":{body}}}';\n\
               else printf '%s\\n' '{{\"number\":17,\"state\":\"OPEN\",\"isDraft\":false,\"headRefOid\":\"{head}\",\"baseRefName\":\"main\",\"mergeCommit\":null,\"body\":{body}}}'; fi\n\
               exit 0\n\
             fi\n\
             if [ \"$1 $2\" = 'pr merge' ]; then\n\
               case \" $* \" in *\" --match-head-commit {head} \"*) ;; *) exit 74 ;; esac\n\
               if [ -e \"$FAIL_ONCE\" ]; then rm \"$FAIL_ONCE\"; exit 73; fi\n\
               touch \"$MERGED\"; exit 0\n\
             fi\n\
             exit 64\n"
        ),
    )
    .expect("gh");
    fs::set_permissions(&gh, fs::Permissions::from_mode(0o755)).expect("gh mode");
    let adapter = super::DraftPrAdapter {
        gh,
        environment: BTreeMap::from([
            ("FAIL_ONCE".into(), fail_once.into_os_string()),
            ("MERGED".into(), merged.into_os_string()),
        ]),
    };

    super::admin_squash_merge_exact_with_refresh_and_admission(
        &state_path,
        &mut state,
        &adapter,
        || Ok(super::BridgeClaimOwnership::Refreshed { ttl_seconds: 60 }),
        || Ok(()),
    )
    .expect_err("first merge fails");
    assert_eq!(state.phase, super::BridgePhase::MergeRequested);
    let merge_oid = super::admin_squash_merge_exact_with_refresh_and_admission(
        &state_path,
        &mut state,
        &adapter,
        || Ok(super::BridgeClaimOwnership::Refreshed { ttl_seconds: 60 }),
        || Ok(()),
    )
    .expect("retry observes merge");
    assert_eq!(merge_oid, "b".repeat(40));
    assert_eq!(state.phase, super::BridgePhase::Merged);
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_recovers_merged_pr_after_branch_deletion_and_base_advance() {
    let (fixture, mut state, _snapshot, _) =
        implementation_proof_fixture("merge-observed-after-delete");
    commit_implementation(&state);
    let head = git_stdout(&state.identity.worktree, &["rev-parse", "HEAD"]);
    git(
        &state.identity.worktree,
        &[
            "push",
            "origin",
            &format!("{head}:refs/heads/{}", state.identity.branch),
        ],
    );
    state.phase = super::BridgePhase::MergeRequested;
    state.pr = Some(17);
    state.head_oid = Some(head.clone());
    let closeout = "## Closeout report\n";
    state.closeout_digest = Some(autospec_core::autonomous::waterfall::sha256_hex(
        closeout.as_bytes(),
    ));
    let body =
        serde_json::to_string(&super::canonical_pull_request_body(&state, closeout).unwrap())
            .unwrap();
    let state_path = fixture.root.join("state/invocation.json");
    super::write_invocation_atomic(&state_path, &state).expect("requested state");
    git(
        &fixture.root.join("seed"),
        &["push", "origin", &format!(":{}", state.identity.branch)],
    );
    fs::write(fixture.root.join("seed/merged.txt"), "merged\n").expect("base advance");
    git(&fixture.root.join("seed"), &["add", "merged.txt"]);
    git(
        &fixture.root.join("seed"),
        &["commit", "-m", "merged result"],
    );
    git(&fixture.root.join("seed"), &["push", "origin", "main"]);
    let gh = fixture.root.join("gh-merged-observation");
    fs::write(
        &gh,
        format!(
            "#!/bin/sh\nset -eu\n\
             if [ \"$1 $2\" = 'pr view' ]; then\n\
               printf '%s\\n' '{{\"number\":17,\"state\":\"MERGED\",\"isDraft\":false,\"headRefOid\":\"{head}\",\"baseRefName\":\"main\",\"mergeCommit\":{{\"oid\":\"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\"}},\"body\":{body}}}'\n\
               exit 0\n\
             fi\n\
             exit 64\n"
        ),
    )
    .expect("gh");
    fs::set_permissions(&gh, fs::Permissions::from_mode(0o755)).expect("gh mode");
    let adapter = super::DraftPrAdapter {
        gh,
        environment: BTreeMap::new(),
    };

    assert_eq!(
        super::admin_squash_merge_exact_with_refresh(&state_path, &mut state, &adapter, || Ok(
            super::BridgeClaimOwnership::Refreshed { ttl_seconds: 60 }
        ))
        .expect("merged observation must win over deleted refs"),
        "b".repeat(40)
    );
    assert_eq!(state.phase, super::BridgePhase::Merged);
}

#[test]
fn autonomous_executor_bridge_pr_size_base_drift_recomputes_exact_admission() {
    let (fixture, mut state, _snapshot, _) = implementation_proof_fixture("merge-base-drift");
    commit_implementation(&state);
    let original_head = git_stdout(&state.identity.worktree, &["rev-parse", "HEAD"]);
    git(
        &state.identity.worktree,
        &[
            "push",
            "origin",
            &format!("{original_head}:refs/heads/{}", state.identity.branch),
        ],
    );
    state.phase = super::BridgePhase::ReviewPassed;
    state.pr = Some(17);
    state.head_oid = Some(original_head.clone());
    let state_path = fixture.root.join("state/invocation.json");
    super::write_invocation_atomic(&state_path, &state).expect("reviewed state");
    let original =
        super::evaluate_patch_size_admission(&state, &original_head, DRAFT_ISSUE_BODY)
            .expect("original admission");
    super::persist_patch_size_admission(&state_path, &original).expect("original receipt");

    fs::write(fixture.root.join("seed/base-drift.txt"), "drift\n").expect("base drift");
    git(&fixture.root.join("seed"), &["add", "base-drift.txt"]);
    git(&fixture.root.join("seed"), &["commit", "-m", "base drift"]);
    git(&fixture.root.join("seed"), &["push", "origin", "main"]);

    assert!(
        super::reconcile_base_drift_with_refresh(&state_path, &mut state, || {
            Ok(super::BridgeClaimOwnership::Refreshed { ttl_seconds: 60 })
        })
        .expect("reconcile"),
        "drift must regenerate the lane"
    );
    assert_eq!(state.phase, super::BridgePhase::DraftCreated);
    assert_ne!(state.identity.base_oid, original_head);
    assert_ne!(state.head_oid.as_deref(), Some(original_head.as_str()));
    let admission = super::validate_patch_size_admission(&state_path, &state)
        .expect("drifted base/head have fresh patch-size evidence");
    assert_eq!(admission.base_oid, state.identity.base_oid);
    assert_eq!(Some(admission.head_oid.as_str()), state.head_oid.as_deref());
    assert_ne!(admission.base_oid, original.base_oid);
    assert_ne!(admission.head_oid, original.head_oid);
    assert_ne!(admission.evaluation_digest, original.evaluation_digest);
    let ancestor = Command::new("git")
        .args([
            "merge-base",
            "--is-ancestor",
            &state.identity.base_oid,
            state.head_oid.as_deref().expect("new head"),
        ])
        .current_dir(&state.identity.worktree)
        .status()
        .expect("ancestry");
    assert!(ancestor.success());
}

#[test]
fn autonomous_executor_bridge_reconciles_draft_base_before_premerge_evidence() {
    // Break caught: stale branches running scanners and the full suite before they receive
    // process fixes already merged to their integration base.
    let _environment = TEST_ENVIRONMENT.lock().expect("test environment lock");
    let previous_claim = std::env::var_os("AUTOSPEC_TEST_EXACT_EVIDENCE_CLAIM");
    std::env::set_var("AUTOSPEC_TEST_EXACT_EVIDENCE_CLAIM", "1");
    let (fixture, mut state, _snapshot, _) =
        implementation_proof_fixture("draft-premerge-base-drift");
    commit_implementation(&state);
    let original_head = git_stdout(&state.identity.worktree, &["rev-parse", "HEAD"]);
    git(
        &state.identity.worktree,
        &[
            "push",
            "origin",
            &format!("{original_head}:refs/heads/{}", state.identity.branch),
        ],
    );
    state.phase = super::BridgePhase::DraftCreated;
    state.pr = Some(17);
    state.head_oid = Some(original_head.clone());
    let state_path = fixture.root.join("state/invocation.json");
    super::write_invocation_atomic(&state_path, &state).expect("draft state");
    let admission =
        super::evaluate_patch_size_admission(&state, &original_head, DRAFT_ISSUE_BODY)
            .expect("original admission");
    super::persist_patch_size_admission(&state_path, &admission)
        .expect("original admission receipt");

    fs::write(fixture.root.join("seed/premerge-fix.txt"), "base fix\n").expect("base drift");
    git(&fixture.root.join("seed"), &["add", "premerge-fix.txt"]);
    git(
        &fixture.root.join("seed"),
        &["commit", "-m", "premerge fix"],
    );
    git(&fixture.root.join("seed"), &["push", "origin", "main"]);
    let updated_base = git_stdout(&fixture.root.join("seed"), &["rev-parse", "HEAD"]);
    let request = super::ExecutorBridgeRequest {
        repository: state.identity.repository.clone(),
        repository_path: state.identity.repository_path.clone(),
        issue: state.identity.issue,
        issue_title: "Reconcile stale implementation".to_string(),
        issue_body: DRAFT_ISSUE_BODY.to_string(),
        worker_id: state.identity.worker_id.clone(),
        claim_id: state.identity.claim_id.clone(),
        invocation_id: state.identity.invocation_id.clone(),
        state_path: state_path.clone(),
        event_log: fixture.root.join("events.jsonl"),
    };
    let proof = super::ImplementationProof {
        head_oid: original_head.clone(),
        closeout_body: String::new(),
    };

    let result = super::ensure_premerge_and_review(
        &request,
        &BTreeMap::new(),
        &super::DraftPrAdapter::github_cli(),
        &mut state,
        &proof,
        None,
    );
    match previous_claim {
        Some(value) => std::env::set_var("AUTOSPEC_TEST_EXACT_EVIDENCE_CLAIM", value),
        None => std::env::remove_var("AUTOSPEC_TEST_EXACT_EVIDENCE_CLAIM"),
    }

    assert_eq!(
        result.expect("base drift must precede scanner resolution"),
        None
    );
    assert_eq!(state.phase, super::BridgePhase::DraftCreated);
    assert_eq!(state.identity.base_oid, updated_base);
    assert_ne!(state.head_oid.as_deref(), Some(original_head.as_str()));
    assert!(
        !state.identity.worktree.join(".autospec/evidence").exists(),
        "premerge evidence started before the stale branch was updated"
    );
}

#[test]
fn autonomous_executor_bridge_base_drift_invalidates_accepted_and_requested_results() {
    for phase in [
        super::BridgePhase::ResultAccepted,
        super::BridgePhase::MergeRequested,
    ] {
        let (fixture, mut state, _snapshot, _) =
            implementation_proof_fixture(&format!("late-drift-{phase:?}"));
        commit_implementation(&state);
        let original_head = git_stdout(&state.identity.worktree, &["rev-parse", "HEAD"]);
        git(
            &state.identity.worktree,
            &[
                "push",
                "origin",
                &format!("{original_head}:refs/heads/{}", state.identity.branch),
            ],
        );
        state.phase = phase;
        state.pr = Some(17);
        state.head_oid = Some(original_head);
        state.terminal_result = Some("accepted-result".into());
        let state_path = fixture.root.join("state/invocation.json");
        super::write_invocation_atomic(&state_path, &state).expect("accepted state");
        fs::write(
            fixture.root.join("seed/late-drift.txt"),
            format!("{phase:?}\n"),
        )
        .expect("base drift");
        git(&fixture.root.join("seed"), &["add", "late-drift.txt"]);
        git(&fixture.root.join("seed"), &["commit", "-m", "late drift"]);
        git(&fixture.root.join("seed"), &["push", "origin", "main"]);

        assert!(
            super::reconcile_base_drift_with_refresh(&state_path, &mut state, || Ok(
                super::BridgeClaimOwnership::Refreshed { ttl_seconds: 60 }
            ))
            .expect("late drift must regenerate")
        );
        assert_eq!(state.phase, super::BridgePhase::DraftCreated);
        assert_eq!(state.terminal_result, None);
    }
}

#[test]
fn autonomous_executor_bridge_claim_takeover_blocks_base_drift_push() {
    let (fixture, mut state, _snapshot, _) = implementation_proof_fixture("drift-takeover");
    commit_implementation(&state);
    let original_head = git_stdout(&state.identity.worktree, &["rev-parse", "HEAD"]);
    git(
        &state.identity.worktree,
        &[
            "push",
            "origin",
            &format!("{original_head}:refs/heads/{}", state.identity.branch),
        ],
    );
    state.phase = super::BridgePhase::ReviewPassed;
    state.pr = Some(17);
    state.head_oid = Some(original_head.clone());
    let state_path = fixture.root.join("state/invocation.json");
    super::write_invocation_atomic(&state_path, &state).expect("reviewed state");
    let durable_before = fs::read(&state_path).expect("durable state");
    let local_head_before = git_stdout(
        &state.identity.worktree,
        &["rev-parse", "--verify", "HEAD^{commit}"],
    );
    fs::write(fixture.root.join("seed/takeover.txt"), "drift\n").expect("base drift");
    git(&fixture.root.join("seed"), &["add", "takeover.txt"]);
    git(&fixture.root.join("seed"), &["commit", "-m", "base drift"]);
    git(&fixture.root.join("seed"), &["push", "origin", "main"]);

    let error = super::reconcile_base_drift_with_refresh(&state_path, &mut state, || {
        Ok(super::BridgeClaimOwnership::Lost)
    })
    .expect_err("takeover blocks push");
    assert!(error.contains("ownership"), "{error}");
    let issue_ref = format!("refs/heads/{}", state.identity.branch);
    assert_eq!(
        super::remote_head_refs(&state.identity.repository_path)
            .expect("remote refs")
            .get(&issue_ref),
        Some(&original_head)
    );
    assert_eq!(
        git_stdout(
            &state.identity.worktree,
            &["rev-parse", "--verify", "HEAD^{commit}"]
        ),
        local_head_before
    );
    assert_eq!(fs::read(state_path).expect("durable state"), durable_before);
}

#[test]
fn autonomous_executor_bridge_recovers_crash_after_owned_base_merge() {
    let (fixture, mut state, _snapshot, _) = implementation_proof_fixture("drift-crash");
    commit_implementation(&state);
    let original_head = git_stdout(&state.identity.worktree, &["rev-parse", "HEAD"]);
    git(
        &state.identity.worktree,
        &[
            "push",
            "origin",
            &format!("{original_head}:refs/heads/{}", state.identity.branch),
        ],
    );
    state.phase = super::BridgePhase::ReviewPassed;
    state.pr = Some(17);
    state.head_oid = Some(original_head);
    let state_path = fixture.root.join("state/invocation.json");
    super::write_invocation_atomic(&state_path, &state).expect("reviewed state");
    fs::write(fixture.root.join("seed/crash.txt"), "drift\n").expect("base drift");
    git(&fixture.root.join("seed"), &["add", "crash.txt"]);
    git(&fixture.root.join("seed"), &["commit", "-m", "base drift"]);
    git(&fixture.root.join("seed"), &["push", "origin", "main"]);

    super::BASE_DRIFT_FAILPOINT.store(1, Ordering::SeqCst);
    let error = super::reconcile_base_drift_with_refresh(&state_path, &mut state, || {
        Ok(super::BridgeClaimOwnership::Refreshed { ttl_seconds: 60 })
    })
    .expect_err("injected crash");
    assert!(error.contains("injected crash"), "{error}");
    let durable = super::PersistedInvocation::from_json(
        &fs::read_to_string(&state_path).expect("durable invocation"),
    )
    .expect("durable pre-merge binding remains");
    assert_eq!(durable.phase, super::BridgePhase::ReviewPassed);

    assert!(
        super::reconcile_base_drift_with_refresh(&state_path, &mut state, || {
            Ok(super::BridgeClaimOwnership::Refreshed { ttl_seconds: 60 })
        })
        .expect("adopt exact merge")
    );
    assert_eq!(state.phase, super::BridgePhase::DraftCreated);
}

#[test]
fn autonomous_executor_bridge_cleanup_resumes_after_owned_worktree_removal() {
    let (fixture, mut state, _snapshot, _) = implementation_proof_fixture("cleanup-resume");
    commit_implementation(&state);
    state.phase = super::BridgePhase::CleanupPending;
    state.identity.runtime_environment_dir = None;
    state.identity.runtime_session_id = None;
    let state_path = fixture.root.join("state/invocation.json");
    super::write_invocation_atomic(&state_path, &state).expect("cleanup state");
    let intent = super::cleanup_record_path(&state_path, "worktree-intent");
    super::ensure_cleanup_record(
        &intent,
        &super::cleanup_binding(&state),
        "test removal intent",
    )
    .expect("durable intent");
    git(
        &state.identity.repository_path,
        &[
            "worktree",
            "remove",
            state.identity.worktree.to_str().expect("worktree"),
        ],
    );

    super::finalize_merged_executor(&state_path, &mut state, None).expect("resume cleanup");
    assert_eq!(state.phase, super::BridgePhase::Complete);
    assert!(super::cleanup_record_path(&state_path, "worktree-complete").exists());
}

#[test]
fn autonomous_executor_bridge_runtime_close_recovers_after_receipt_gap() {
    let _environment = TEST_ENVIRONMENT.lock().expect("test environment lock");
    let (fixture, mut state, _snapshot, _) =
        implementation_proof_fixture("runtime-close-recovery");
    let autospec = state.identity.worktree.join(".autospec");
    fs::create_dir_all(&autospec).expect("runtime manifest directory");
    fs::write(
        state.identity.worktree.join("runtime-up.py"),
        "import http.server, os\npid=os.fork()\nif not pid:\n http.server.HTTPServer(('127.0.0.1', int(os.environ['AGENT_FRONTEND_PORT'])), http.server.SimpleHTTPRequestHandler).serve_forever()\nelse:\n open('runtime.pid','w').write(str(pid))\n",
    )
    .expect("runtime up");
    fs::write(
        state.identity.worktree.join("runtime-down.py"),
        "import os, signal\npid=int(open('runtime.pid').read())\nos.kill(pid, signal.SIGTERM)\nos.remove('runtime.pid')\n",
    )
    .expect("runtime down");
    fs::write(
        autospec.join("runtime.yml"),
        "version: 1\ndefault_mode: local\nmodes:\n  local:\n    command: python3 runtime-up.py\n    down: python3 runtime-down.py\n",
    )
    .expect("runtime manifest");
    let state_root = fixture.root.join("runtime-state");
    let previous = std::env::var_os("AGENT_ENV_STATE_ROOT");
    std::env::set_var("AGENT_ENV_STATE_ROOT", &state_root);
    let runtime = super::runtime_session_adapter(&state.identity.worktree)
        .expect("runtime adapter")
        .expect("runtime manifest");
    state.identity.runtime_session_id = Some(runtime.session_id.clone());
    let state_path = fixture.root.join("state/invocation.json");
    super::write_invocation_atomic(&state_path, &state).expect("runtime state");

    super::RUNTIME_CLOSE_FAILPOINT.store(1, Ordering::SeqCst);
    let error = super::finalize_failed_executor(
        &state_path,
        &mut state,
        Some(runtime),
        None,
        true,
        false,
        "implementation failed",
    )
    .expect_err("injected failure-finalization receipt gap");
    assert!(error.to_string().contains("injected crash"), "{error}");
    assert_eq!(
        super::read_failure_cleanup_intent(&state_path, &state)
            .expect("restart reads exact cleanup intent")
            .reason,
        "implementation failed"
    );
    assert!(
        super::cleanup_record_path(&state_path, "runtime-intent.json").exists(),
        "close intent must precede runtime mutation"
    );
    assert!(!super::cleanup_record_path(&state_path, "runtime-complete").exists());

    super::close_owned_runtime(&state_path, &state, None)
        .expect("restart proves absence and writes receipt");
    assert!(super::cleanup_record_path(&state_path, "runtime-complete").exists());
    assert!(session_record_ids(&state_root).is_empty());
    match previous {
        Some(value) => std::env::set_var("AGENT_ENV_STATE_ROOT", value),
        None => std::env::remove_var("AGENT_ENV_STATE_ROOT"),
    }
}

#[test]
fn autonomous_executor_bridge_reattaches_after_error_and_derives_cleanup_intent() {
    let _environment = TEST_ENVIRONMENT.lock().expect("test environment lock");
    let (fixture, mut state, _snapshot, _) =
        implementation_proof_fixture("runtime-reattach-cleanup-intent");
    let autospec = state.identity.worktree.join(".autospec");
    fs::create_dir_all(&autospec).expect("runtime manifest directory");
    fs::write(
        state.identity.worktree.join("runtime-up.py"),
        "import http.server, os\npid=os.fork()\nif not pid:\n http.server.HTTPServer(('127.0.0.1', int(os.environ['AGENT_FRONTEND_PORT'])), http.server.SimpleHTTPRequestHandler).serve_forever()\nelse:\n open('runtime.pid','w').write(str(pid))\n",
    )
    .expect("runtime up");
    fs::write(
        state.identity.worktree.join("runtime-down.py"),
        "import os, signal\npid=int(open('runtime.pid').read())\nos.kill(pid, signal.SIGTERM)\nos.remove('runtime.pid')\n",
    )
    .expect("runtime down");
    fs::write(
        autospec.join("runtime.yml"),
        "version: 1\ndefault_mode: local\nmodes:\n  local:\n    command: python3 runtime-up.py\n    down: python3 runtime-down.py\n",
    )
    .expect("runtime manifest");
    let state_root = fixture.root.join("runtime-state");
    let previous = std::env::var_os("AGENT_ENV_STATE_ROOT");
    std::env::set_var("AGENT_ENV_STATE_ROOT", &state_root);
    let runtime = super::runtime_session_adapter(&state.identity.worktree)
        .expect("runtime adapter")
        .expect("runtime manifest");
    let environment_dir = runtime.environment_dir().to_path_buf();
    let session_id = runtime.session_id.clone();
    state.phase = super::BridgePhase::CleanupPending;
    state.identity.runtime_environment_dir = Some(environment_dir.clone());
    state.identity.runtime_session_id = Some(session_id.clone());
    let state_path = fixture.root.join("state/invocation.json");
    super::write_invocation_atomic(&state_path, &state).expect("runtime state");

    drop(runtime);
    assert!(
        !super::cleanup_record_path(&state_path, "runtime-intent.json").exists(),
        "the crash window precedes cleanup intent"
    );
    fs::remove_file(autospec.join("runtime.yml"))
        .expect("remove mutable manifest before bridge reattach");
    let reattached = super::reattach_runtime_session_adapter(
        &state.identity.worktree,
        &environment_dir,
        &session_id,
    )
    .expect("reattach exact durable runtime from its private snapshot")
    .expect("persisted runtime binding");
    drop(reattached);
    fs::write(
        autospec.join("runtime.yml"),
        "version: 1\ndefault_mode: local\nmodes:\n  local:\n    command: /bin/false\n    down: /bin/false\n",
    )
    .expect("replace live manifest after persisted runtime binding");

    super::close_owned_runtime(&state_path, &state, None)
        .expect("cleanup from persisted original manifest and binding");
    assert!(super::cleanup_record_path(&state_path, "runtime-intent.json").exists());
    assert!(super::cleanup_record_path(&state_path, "runtime-complete").exists());
    match previous {
        Some(value) => std::env::set_var("AGENT_ENV_STATE_ROOT", value),
        None => std::env::remove_var("AGENT_ENV_STATE_ROOT"),
    }
}

#[test]
fn autonomous_executor_bridge_runtime_close_retries_partial_teardown_failure() {
    let _environment = TEST_ENVIRONMENT.lock().expect("test environment lock");
    let (fixture, mut state, _snapshot, _) =
        implementation_proof_fixture("runtime-close-partial-retry");
    let autospec = state.identity.worktree.join(".autospec");
    fs::create_dir_all(&autospec).expect("runtime manifest directory");
    fs::write(
        state.identity.worktree.join("runtime-up.py"),
        "import http.server, os\npid=os.fork()\nif not pid:\n http.server.HTTPServer(('127.0.0.1', int(os.environ['AGENT_FRONTEND_PORT'])), http.server.SimpleHTTPRequestHandler).serve_forever()\nelse:\n open('runtime.pid','w').write(str(pid))\n",
    )
    .expect("runtime up");
    fs::write(
        state.identity.worktree.join("runtime-down.py"),
        "import os, signal\ntry:\n open('down-attempted','x').write('1')\n raise RuntimeError('42')\nexcept FileExistsError:\n pass\npid=int(open('runtime.pid').read())\nos.kill(pid, signal.SIGTERM)\nos.remove('runtime.pid')\n",
    )
    .expect("runtime down");
    fs::write(
        autospec.join("runtime.yml"),
        "version: 1\ndefault_mode: local\nmodes:\n  local:\n    command: python3 runtime-up.py\n    down: python3 runtime-down.py\n",
    )
    .expect("runtime manifest");
    let state_root = fixture.root.join("runtime-state");
    let previous = std::env::var_os("AGENT_ENV_STATE_ROOT");
    std::env::set_var("AGENT_ENV_STATE_ROOT", &state_root);
    let runtime = super::runtime_session_adapter(&state.identity.worktree)
        .expect("runtime adapter")
        .expect("runtime manifest");
    state.identity.runtime_session_id = Some(runtime.session_id.clone());
    let state_path = fixture.root.join("state/invocation.json");
    super::write_invocation_atomic(&state_path, &state).expect("runtime state");

    let error = super::close_owned_runtime(&state_path, &state, Some(runtime))
        .expect_err("first teardown fails");
    assert!(error.contains("42") || error.contains("runtime"), "{error}");
    assert!(!super::cleanup_record_path(&state_path, "runtime-complete").exists());

    super::close_owned_runtime(&state_path, &state, None)
        .expect("restart retries authoritative teardown");
    assert!(super::cleanup_record_path(&state_path, "runtime-complete").exists());
    assert!(session_record_ids(&state_root).is_empty());
    match previous {
        Some(value) => std::env::set_var("AGENT_ENV_STATE_ROOT", value),
        None => std::env::remove_var("AGENT_ENV_STATE_ROOT"),
    }
}

#[test]
fn autonomous_executor_bridge_rejects_missing_worktree_without_prior_intent() {
    let (fixture, mut state, _snapshot, _) = implementation_proof_fixture("cleanup-no-intent");
    commit_implementation(&state);
    state.phase = super::BridgePhase::CleanupPending;
    state.identity.runtime_environment_dir = None;
    state.identity.runtime_session_id = None;
    let state_path = fixture.root.join("state/invocation.json");
    super::write_invocation_atomic(&state_path, &state).expect("cleanup state");
    git(
        &state.identity.repository_path,
        &[
            "worktree",
            "remove",
            state.identity.worktree.to_str().expect("worktree"),
        ],
    );

    let error = super::finalize_merged_executor(&state_path, &mut state, None)
        .expect_err("missing intent must fail closed");
    assert!(error.contains("before a durable removal intent"), "{error}");
}

#[test]
fn autonomous_executor_bridge_requires_owned_runtime_cleanup_proof() {
    let (fixture, mut state, _snapshot, _) =
        implementation_proof_fixture("cleanup-runtime-proof");
    state.phase = super::BridgePhase::CleanupPending;
    state.identity.runtime_session_id = Some("runtime-owned".into());
    let state_path = fixture.root.join("state/invocation.json");
    super::write_invocation_atomic(&state_path, &state).expect("cleanup state");

    let error = super::finalize_merged_executor(&state_path, &mut state, None)
        .expect_err("runtime receipt is mandatory");
    assert!(error.contains("runtime session"), "{error}");
    assert!(state.identity.worktree.exists());
}

#[test]
fn autonomous_executor_bridge_failure_budget_selects_queue_or_needs_human() {
    assert_eq!(
        super::failure_disposition(true, false),
        crate::commands::claim::BridgeClaimDisposition::Retryable
    );
    assert_eq!(
        super::failure_disposition(true, true),
        crate::commands::claim::BridgeClaimDisposition::NeedsHuman
    );
    assert_eq!(
        super::failure_disposition(false, false),
        crate::commands::claim::BridgeClaimDisposition::NeedsHuman
    );
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_closes_integration_issue() {
    let (fixture, mut state, _snapshot, _) =
        implementation_proof_fixture("close-integration-issue");
    state.phase = super::BridgePhase::Merged;
    state.terminal_result = Some(git_stdout(&fixture.repo, &["rev-parse", "origin/main"]));
    state.identity.base_ref = "origin/integration".into();
    state.current_child = Some(101);
    git(
        &fixture.repo,
        &[
            "--git-dir",
            fixture
                .root
                .join("remote.git")
                .to_str()
                .expect("remote path"),
            "update-ref",
            "refs/heads/integration",
            state.terminal_result.as_deref().expect("merge OID"),
        ],
    );

    let calls = fixture.root.join("issue-calls");
    let issue_state = fixture.root.join("issue-state");
    fs::write(&issue_state, "OPEN\n").expect("issue state");
    let gh = fixture.root.join("gh-close-issue");
    write_executable(
        &gh,
        r#"#!/bin/sh
set -eu
printf '%s\n' "$*" >> "$ISSUE_CALLS"
number=${RETURN_NUMBER:-$3}
case "$1 $2" in
  "issue view") printf '{"number":%s,"state":"%s"}\n' "$number" "$(cat "$ISSUE_STATE")" ;;
  "issue close")
[ "${CLOSE_FAIL:-0}" = 0 ] || exit 65
printf '%s\n' CLOSED > "$ISSUE_STATE"
;;
  *) exit 64 ;;
esac
"#,
    );
    let adapter = super::DraftPrAdapter {
        gh,
        environment: BTreeMap::from([
            ("ISSUE_CALLS".into(), calls.clone().into_os_string()),
            ("ISSUE_STATE".into(), issue_state.clone().into_os_string()),
        ]),
    };

    super::close_merged_integration_issue(&state, &adapter).expect("close continuation");
    let call_log = fs::read_to_string(&calls).expect("calls");
    assert_eq!(call_log.matches("issue close 101").count(), 1, "{call_log}");
    assert!(!call_log.contains("issue close 42"), "{call_log}");
    assert_eq!(state.phase, super::BridgePhase::Merged);

    fs::write(fixture.repo.join("integration-advance"), "next\n")
        .expect("integration advance fixture");
    git(&fixture.repo, &["add", "integration-advance"]);
    git(&fixture.repo, &["commit", "-m", "advance integration"]);
    git(
        &fixture.repo,
        &["push", "origin", "HEAD:refs/heads/integration"],
    );
    fs::write(&calls, "").expect("clear advanced-tip calls");
    fs::write(&issue_state, "OPEN\n").expect("reopen advanced-tip issue");
    super::close_merged_integration_issue(&state, &adapter)
        .expect("descendant integration tip preserves closure proof");
    let advanced_calls = fs::read_to_string(&calls).expect("advanced-tip calls");
    assert_eq!(
        advanced_calls.matches("issue close 101").count(),
        1,
        "{advanced_calls}"
    );

    fs::write(&calls, "").expect("clear calls");
    fs::write(&issue_state, "OPEN\n").expect("reopen legacy issue");
    state.current_child = None;
    super::close_merged_integration_issue(&state, &adapter).expect("close legacy issue");
    let legacy_calls = fs::read_to_string(&calls).expect("legacy calls");
    assert_eq!(
        legacy_calls.matches("issue close 42").count(),
        1,
        "{legacy_calls}"
    );
    assert!(!legacy_calls.contains("issue close 101"), "{legacy_calls}");

    fs::write(&calls, "").expect("clear default-branch calls");
    state.identity.base_ref = "origin/main".into();
    super::close_merged_integration_issue(&state, &adapter).expect("default branch merge");
    assert_eq!(fs::read_to_string(&calls).expect("default calls"), "");

    state.identity.base_ref = "origin/integration".into();
    let mut mismatched = adapter.clone();
    mismatched
        .environment
        .insert("RETURN_NUMBER".into(), "99".into());
    assert!(super::close_merged_integration_issue(&state, &mismatched).is_err());
    assert_eq!(state.phase, super::BridgePhase::Merged);

    let mut failing = adapter.clone();
    failing.environment.insert("CLOSE_FAIL".into(), "1".into());
    fs::write(&issue_state, "OPEN\n").expect("reopen failed-close issue");
    assert!(super::close_merged_integration_issue(&state, &failing).is_err());
    assert_eq!(state.phase, super::BridgePhase::Merged);

    let rewrite_tree = git_stdout(&fixture.repo, &["rev-parse", "HEAD^{tree}"]);
    let rewritten_integration = git_stdout(
        &fixture.repo,
        &["commit-tree", &rewrite_tree, "-m", "rewrite integration"],
    );
    git(
        &fixture.repo,
        &[
            "push",
            "--force",
            "origin",
            &format!("{rewritten_integration}:refs/heads/integration"),
        ],
    );
    fs::write(&calls, "").expect("clear rewritten-tip calls");
    let error = super::close_merged_integration_issue(&state, &adapter)
        .expect_err("rewritten integration tip must fail closed");
    assert!(format!("{error:?}").contains("not an ancestor"));
    assert_eq!(fs::read_to_string(&calls).expect("rewritten-tip calls"), "");
}
