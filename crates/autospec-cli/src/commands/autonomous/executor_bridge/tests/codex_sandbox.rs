// executor_bridge tests: codex / sandbox — 5 cases.
//
// Split out of tests.rs; see the note in that file.

use crate::commands::autonomous::executor_bridge as bridge;
use super::super::{resolve_base, BridgePhase};
use super::support_base::{DetachedSupervisorCleanup, GitFixture, TEST_SEQUENCE, git, git_stdout, test_environment};
use super::support_invocation::{detach_harness_for_adoption, supervision_state};
use std::collections::BTreeMap;
use std::fs;
use std::process::Command;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

#[test]
fn autonomous_executor_bridge_codex_sandbox_entrypoint_retries_pruned_worktree_repair() {
    let _environment = test_environment();
    let fixture = GitFixture::new("entrypoint-pruned-worktree-repair");
    let base = resolve_base(&fixture.repo, &BTreeMap::new()).expect("resolve base");
    let scope = format!(
        "entrypoint_repair_{}_{}",
        std::process::id(),
        TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    );
    let worktree = bridge::provision_issue_worktree_for_claim(
        &fixture.repo,
        &scope,
        42,
        &base,
        Some(("claim-42", "invocation-42")),
    )
    .expect("provision issue worktree");
    let mut state = supervision_state(&fixture);
    state.phase = BridgePhase::ImplementationComplete;
    state.identity.worktree = worktree.path.clone();
    state.identity.base_ref = base.base_ref.clone();
    state.identity.base_oid = base.base_oid.clone();
    state.identity.runtime_environment_dir = None;
    state.identity.runtime_session_id = None;
    let scope_root = worktree.path.parent().expect("scope root");
    let state_path = scope_root.join("entrypoint-state.json");
    bridge::write_invocation_atomic(&state_path, &state).expect("persist invocation");
    fs::remove_dir_all(&worktree.path).expect("simulate disappeared worktree");
    let config = fixture.root.join("empty-config");
    fs::create_dir_all(&config).expect("empty config");
    let previous_config = std::env::var_os("AUTOSPEC_CONFIG_DIR");
    std::env::set_var("AUTOSPEC_CONFIG_DIR", &config);
    let request = bridge::ExecutorBridgeRequest {
        repository: state.identity.repository.clone(),
        repository_path: fixture.repo.clone(),
        issue: state.identity.issue,
        issue_title: "Repair executor worktree".to_string(),
        issue_body: "## Goal\n\nRepair the exact executor worktree.".to_string(),
        serialization_reasons: Vec::new(),
        worker_id: state.identity.worker_id.clone(),
        claim_id: state.identity.claim_id.clone(),
        invocation_id: state.identity.invocation_id.clone(),
        state_path: state_path.clone(),
        event_log: scope_root.join("events.jsonl"),
    };

    bridge::WORKTREE_REPAIR_FAILPOINT.store(1, Ordering::SeqCst);
    let interrupted =
        bridge::run_executor_bridge(&request).expect_err("interrupt entrypoint repair");
    assert!(interrupted
        .to_string()
        .contains("injected executor worktree repair crash"));
    let retry = bridge::run_executor_bridge(&request)
        .expect_err("stop after entrypoint recovery at implementation proof");
    assert!(
        retry
            .to_string()
            .contains("implementation HEAD is unchanged"),
        "{retry}"
    );
    assert!(worktree.path.is_dir());
    assert!(!scope_root.join("issue-42.repair-intent.json").exists());

    match previous_config {
        Some(value) => std::env::set_var("AUTOSPEC_CONFIG_DIR", value),
        None => std::env::remove_var("AUTOSPEC_CONFIG_DIR"),
    }
    let worktree_path = worktree.path.to_str().expect("worktree path");
    git(
        &fixture.repo,
        &["worktree", "remove", "--force", worktree_path],
    );
    let _ = fs::remove_dir_all(scope_root);
}

#[test]
fn autonomous_executor_bridge_codex_sandbox_entrypoint_cleanup_ignores_missing_codex() {
    let _environment = test_environment();
    let fixture = GitFixture::new("entrypoint-cleanup-missing-codex");
    let base = resolve_base(&fixture.repo, &BTreeMap::new()).expect("resolve base");
    let scope = format!(
        "entrypoint_cleanup_{}_{}",
        std::process::id(),
        TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    );
    let worktree = bridge::provision_issue_worktree_for_claim(
        &fixture.repo,
        &scope,
        42,
        &base,
        Some(("claim-42", "invocation-42")),
    )
    .expect("provision issue worktree");
    let mut state = supervision_state(&fixture);
    state.phase = BridgePhase::CleanupPending;
    state.identity.worktree = worktree.path.clone();
    state.identity.base_ref = base.base_ref.clone();
    state.identity.base_oid = base.base_oid.clone();
    state.identity.runtime_environment_dir = None;
    state.identity.runtime_session_id = None;
    state.pr = Some(17);
    state.head_oid = Some(git_stdout(&worktree.path, &["rev-parse", "HEAD"]));
    state.terminal_result = Some("b".repeat(40));
    let scope_root = worktree.path.parent().expect("scope root");
    let state_path = scope_root.join("cleanup-state.json");
    bridge::write_invocation_atomic(&state_path, &state).expect("persist cleanup invocation");
    bridge::write_private_create_once(
        &bridge::zero_effect_recovery_marker_path(&state_path),
        b"{\"schema\":1,\"binding\":\"stale-terminal-marker\"}\n",
        "test stale terminal zero-effect marker",
    )
    .expect("persist stale terminal marker");
    bridge::ensure_cleanup_record(
        &bridge::cleanup_record_path(&state_path, "worktree-intent"),
        &bridge::cleanup_binding(&state),
        "test executor worktree cleanup intent",
    )
    .expect("persist worktree cleanup intent");
    let worktree_path = worktree.path.to_str().expect("worktree path");
    git(&fixture.repo, &["worktree", "remove", worktree_path]);
    let aliases = fixture.root.join("missing-codex-aliases.tsv");
    fs::write(&aliases, "codex\t/definitely/missing/codex\t\tCodex CLI\n")
        .expect("write missing Codex alias");
    let previous_aliases = std::env::var_os("AUTOSPEC_HARNESS_RUNTIME_ALIASES");
    std::env::set_var("AUTOSPEC_HARNESS_RUNTIME_ALIASES", &aliases);
    let request = bridge::ExecutorBridgeRequest {
        repository: state.identity.repository.clone(),
        repository_path: fixture.repo.clone(),
        issue: state.identity.issue,
        issue_title: "Finish executor cleanup".to_string(),
        issue_body: "## Goal\n\nFinish the durable executor cleanup.".to_string(),
        serialization_reasons: Vec::new(),
        worker_id: state.identity.worker_id.clone(),
        claim_id: state.identity.claim_id.clone(),
        invocation_id: state.identity.invocation_id.clone(),
        state_path: state_path.clone(),
        event_log: scope_root.join("cleanup-events.jsonl"),
    };

    let _ = bridge::run_executor_bridge(&request);
    let durable = bridge::PersistedInvocation::from_json(
        &fs::read_to_string(&state_path).expect("read completed cleanup state"),
    )
    .expect("parse completed cleanup state");
    assert_eq!(durable.phase, BridgePhase::Complete);
    assert!(!worktree.path.exists());

    match previous_aliases {
        Some(value) => std::env::set_var("AUTOSPEC_HARNESS_RUNTIME_ALIASES", value),
        None => std::env::remove_var("AUTOSPEC_HARNESS_RUNTIME_ALIASES"),
    }
    let _ = fs::remove_dir_all(scope_root);
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_codex_sandbox_entrypoint_pending_sidecar_cleanup_skips_missing_harness(
) {
    let _environment = test_environment();
    let fixture = GitFixture::new("entrypoint-pending-sidecar-recovery");
    let base = resolve_base(&fixture.repo, &BTreeMap::new()).expect("resolve base");
    let scope = format!(
        "entrypoint_pending_sidecar_{}_{}",
        std::process::id(),
        TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    );
    let worktree = bridge::provision_issue_worktree_for_claim(
        &fixture.repo,
        &scope,
        42,
        &base,
        Some(("claim-42", "invocation-42")),
    )
    .expect("provision issue worktree");
    let mut state = supervision_state(&fixture);
    state.identity.worktree = worktree.path.clone();
    state.identity.base_ref = base.base_ref.clone();
    state.identity.base_oid = base.base_oid.clone();
    state.identity.runtime_environment_dir = None;
    state.identity.runtime_session_id = None;
    let scope_root = worktree.path.parent().expect("scope root");
    let state_path = scope_root.join("pending-sidecar-recovery-state.json");
    let event_log = scope_root.join("pending-sidecar-recovery-events.jsonl");
    let _ = detach_harness_for_adoption(
        &fixture,
        &state_path,
        &mut state,
        "while :; do /usr/bin/sleep 1; done",
    );
    let supervisor = state.supervisor.clone().expect("durable supervisor");
    let harness = state.process.clone().expect("durable harness");
    let _cleanup = DetachedSupervisorCleanup(supervisor.clone());
    state.phase = BridgePhase::Pending;
    state.supervisor = None;
    state.process = None;
    state.remote_snapshot_digest = Some("a".repeat(64));
    bridge::write_invocation_atomic(&state_path, &state)
        .expect("persist sidecar-only Pending state");
    let aliases = fixture.root.join("missing-codex-aliases.tsv");
    fs::write(&aliases, "codex\t/definitely/missing/codex\t\tCodex CLI\n")
        .expect("write missing Codex alias");
    let previous_aliases = std::env::var_os("AUTOSPEC_HARNESS_RUNTIME_ALIASES");
    std::env::set_var("AUTOSPEC_HARNESS_RUNTIME_ALIASES", &aliases);
    let request = bridge::ExecutorBridgeRequest {
        repository: state.identity.repository.clone(),
        repository_path: fixture.repo.clone(),
        issue: state.identity.issue,
        issue_title: "Clean the pending executor sidecar".to_string(),
        issue_body: "## Goal\n\nClean the exact pending executor sidecar.".to_string(),
        serialization_reasons: Vec::new(),
        worker_id: state.identity.worker_id.clone(),
        claim_id: state.identity.claim_id.clone(),
        invocation_id: state.identity.invocation_id.clone(),
        state_path: state_path.clone(),
        event_log,
    };

    let mut probe_calls = 0;
    let outcome = bridge::run_executor_bridge_with_codex_probe(&request, |_| {
        probe_calls += 1;
        Err("injected failing Codex sandbox probe".to_string())
    });
    let supervisor_live =
        bridge::cleanup_instance_is_live(&supervisor).expect("inspect pending supervisor");
    let harness_live = bridge::cleanup_instance_is_live(&harness).expect("inspect pending harness");

    match previous_aliases {
        Some(value) => std::env::set_var("AUTOSPEC_HARNESS_RUNTIME_ALIASES", value),
        None => std::env::remove_var("AUTOSPEC_HARNESS_RUNTIME_ALIASES"),
    }
    assert_eq!(probe_calls, 0, "missing harness must not be probed");
    assert!(
        outcome.is_err(),
        "fixture intentionally stops after cleanup"
    );
    assert!(
        !supervisor_live && !harness_live,
        "sidecar-only Pending executor was stranded before harness resolution"
    );
    let worktree_path = worktree.path.to_str().expect("worktree path");
    git(
        &fixture.repo,
        &["worktree", "remove", "--force", worktree_path],
    );
    let _ = fs::remove_dir_all(scope_root);
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_codex_sandbox_entrypoint_interrupted_partial_cleanup_skips_failing_probe(
) {
    let _environment = test_environment();
    let fixture = GitFixture::new("entrypoint-interrupted-partial-recovery");
    let base = resolve_base(&fixture.repo, &BTreeMap::new()).expect("resolve base");
    let scope = format!(
        "entrypoint_interrupted_partial_{}_{}",
        std::process::id(),
        TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    );
    let worktree = bridge::provision_issue_worktree_for_claim(
        &fixture.repo,
        &scope,
        42,
        &base,
        Some(("claim-42", "invocation-42")),
    )
    .expect("provision issue worktree");
    let mut state = supervision_state(&fixture);
    state.identity.worktree = worktree.path.clone();
    state.identity.base_ref = base.base_ref.clone();
    state.identity.base_oid = base.base_oid.clone();
    state.identity.runtime_environment_dir = None;
    state.identity.runtime_session_id = None;
    let scope_root = worktree.path.parent().expect("scope root");
    let state_path = scope_root.join("interrupted-partial-recovery-state.json");
    let event_log = scope_root.join("interrupted-partial-recovery-events.jsonl");
    let _ = detach_harness_for_adoption(
        &fixture,
        &state_path,
        &mut state,
        "while :; do /usr/bin/sleep 1; done",
    );
    let supervisor = state.supervisor.clone().expect("durable supervisor");
    let harness = state.process.clone().expect("durable harness");
    let _cleanup = DetachedSupervisorCleanup(supervisor.clone());
    state.phase = BridgePhase::Interrupted;
    state.process = None;
    bridge::write_invocation_atomic(&state_path, &state)
        .expect("persist partial Interrupted state");
    let aliases = fixture.root.join("codex-aliases.tsv");
    fs::write(&aliases, "codex\t/bin/true\t\tCodex CLI\n").expect("write Codex alias");
    let previous_aliases = std::env::var_os("AUTOSPEC_HARNESS_RUNTIME_ALIASES");
    let previous_claim = std::env::var_os("AUTOSPEC_TEST_EXACT_EVIDENCE_CLAIM");
    std::env::set_var("AUTOSPEC_HARNESS_RUNTIME_ALIASES", &aliases);
    std::env::set_var("AUTOSPEC_TEST_EXACT_EVIDENCE_CLAIM", "1");
    let request = bridge::ExecutorBridgeRequest {
        repository: state.identity.repository.clone(),
        repository_path: fixture.repo.clone(),
        issue: state.identity.issue,
        issue_title: "Clean the interrupted executor".to_string(),
        issue_body: "## Goal\n\nClean the exact interrupted executor.".to_string(),
        serialization_reasons: Vec::new(),
        worker_id: state.identity.worker_id.clone(),
        claim_id: state.identity.claim_id.clone(),
        invocation_id: state.identity.invocation_id.clone(),
        state_path: state_path.clone(),
        event_log: event_log.clone(),
    };

    let mut probe_calls = 0;
    let outcome = bridge::run_executor_bridge_with_codex_probe(&request, |_| {
        probe_calls += 1;
        Err("injected failing Codex sandbox probe".to_string())
    });
    let supervisor_live =
        bridge::cleanup_instance_is_live(&supervisor).expect("inspect interrupted supervisor");
    let harness_live =
        bridge::cleanup_instance_is_live(&harness).expect("inspect interrupted harness");

    assert_eq!(
        probe_calls, 0,
        "partial recovery must precede a fresh Codex probe"
    );
    assert!(
        outcome.is_err(),
        "fixture intentionally stops after cleanup"
    );
    assert!(
        !supervisor_live && !harness_live,
        "partial Interrupted executor was stranded before Codex probing"
    );
    let sinks = bridge::output_sink_paths(&state_path, &state.identity.invocation_id)
        .expect("executor output sinks");
    assert_eq!(
        fs::metadata(&sinks.exit_status)
            .expect("preallocated exit sink")
            .len(),
        16
    );
    assert_eq!(
        bridge::read_executor_exit_status(&sinks.exit_status).expect("empty exit sink"),
        None
    );

    let mut retry_probe_calls = 0;
    let retry = bridge::run_executor_bridge_with_codex_probe(&request, |_| {
        retry_probe_calls += 1;
        Ok(bridge::CodexSandboxPolicy::Default)
    });
    let events = fs::read_to_string(&event_log).expect("read retry events");
    assert_eq!(
        retry_probe_calls, 1,
        "empty preallocated exit sink must allow fresh Codex resolution"
    );
    assert!(retry.is_err(), "unchanged fixture HEAD stops after launch");
    assert!(
        events.contains("\"event\":\"child_started\""),
        "retry never launched the fresh harness: outcome={retry:?} events={events}"
    );
    match previous_aliases {
        Some(value) => std::env::set_var("AUTOSPEC_HARNESS_RUNTIME_ALIASES", value),
        None => std::env::remove_var("AUTOSPEC_HARNESS_RUNTIME_ALIASES"),
    }
    match previous_claim {
        Some(value) => std::env::set_var("AUTOSPEC_TEST_EXACT_EVIDENCE_CLAIM", value),
        None => std::env::remove_var("AUTOSPEC_TEST_EXACT_EVIDENCE_CLAIM"),
    }
    let worktree_path = worktree.path.to_str().expect("worktree path");
    git(
        &fixture.repo,
        &["worktree", "remove", "--force", worktree_path],
    );
    let _ = fs::remove_dir_all(scope_root);
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_codex_sandbox_entrypoint_live_recovery_skips_failing_probe() {
    let _environment = test_environment();
    let fixture = GitFixture::new("entrypoint-live-recovery");
    let base = resolve_base(&fixture.repo, &BTreeMap::new()).expect("resolve base");
    let scope = format!(
        "entrypoint_live_recovery_{}_{}",
        std::process::id(),
        TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    );
    let worktree = bridge::provision_issue_worktree_for_claim(
        &fixture.repo,
        &scope,
        42,
        &base,
        Some(("claim-42", "invocation-42")),
    )
    .expect("provision issue worktree");
    let mut state = supervision_state(&fixture);
    state.identity.worktree = worktree.path.clone();
    state.identity.base_ref = base.base_ref.clone();
    state.identity.base_oid = base.base_oid.clone();
    state.identity.runtime_environment_dir = None;
    state.identity.runtime_session_id = None;
    let scope_root = worktree.path.parent().expect("scope root");
    let state_path = scope_root.join("live-recovery-state.json");
    let event_log = scope_root.join("live-recovery-events.jsonl");
    bridge::write_invocation_atomic(&state_path, &state).expect("persist invocation");
    let ready = scope_root.join("child-ready");
    let release = scope_root.join("release-child");

    let mut launcher = Command::new(std::env::current_exe().expect("test executable"))
        .args([
            "--exact",
            "commands::autonomous::executor_bridge::tests::branch_predecessor::autonomous_executor_bridge_codex_sandbox_entrypoint_live_recovery_helper",
            "--nocapture",
        ])
        .env("AUTOSPEC_TEST_RECOVERY_STATE", &state_path)
        .env("AUTOSPEC_TEST_RECOVERY_EVENTS", &event_log)
        .env("AUTOSPEC_TEST_RECOVERY_READY", &ready)
        .env("AUTOSPEC_TEST_RECOVERY_RELEASE", &release)
        .spawn()
        .expect("spawn recovery fixture launcher");
    let deadline = Instant::now() + Duration::from_secs(5);
    let durable = loop {
        if let Ok(body) = fs::read_to_string(&state_path) {
            if let Ok(candidate) = bridge::PersistedInvocation::from_json(&body) {
                if candidate.phase == BridgePhase::Implementing
                    && candidate.supervisor.is_some()
                    && candidate.process.is_some()
                    && ready.is_file()
                {
                    break candidate;
                }
            }
        }
        assert!(
            Instant::now() < deadline,
            "fixture did not persist a live implementing identity"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    launcher.kill().expect("crash fixture launcher");
    launcher.wait().expect("reap fixture launcher");

    let aliases = fixture.root.join("aliases.tsv");
    fs::write(&aliases, "codex\t/bin/false\t\tCodex CLI\n").expect("write alias table");
    let previous_aliases = std::env::var_os("AUTOSPEC_HARNESS_RUNTIME_ALIASES");
    let previous_claim = std::env::var_os("AUTOSPEC_TEST_EXACT_EVIDENCE_CLAIM");
    std::env::set_var("AUTOSPEC_HARNESS_RUNTIME_ALIASES", &aliases);
    std::env::set_var("AUTOSPEC_TEST_EXACT_EVIDENCE_CLAIM", "1");
    let request = bridge::ExecutorBridgeRequest {
        repository: durable.identity.repository.clone(),
        repository_path: fixture.repo.clone(),
        issue: durable.identity.issue,
        issue_title: "Adopt the live executor".to_string(),
        issue_body: "## Goal\n\nAdopt the exact durable executor process.".to_string(),
        serialization_reasons: Vec::new(),
        worker_id: durable.identity.worker_id.clone(),
        claim_id: durable.identity.claim_id.clone(),
        invocation_id: durable.identity.invocation_id.clone(),
        state_path: state_path.clone(),
        event_log: event_log.clone(),
    };
    let release_for_thread = release.clone();
    let releaser = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(250));
        fs::write(release_for_thread, b"release\n").expect("release adopted child");
    });
    let mut probe_calls = 0;
    let outcome = bridge::run_executor_bridge_with_codex_probe(&request, |_| {
        probe_calls += 1;
        Err("injected failing Codex sandbox probe".to_string())
    });
    releaser.join().expect("release thread");

    assert_eq!(probe_calls, 0, "live recovery must not run a fresh probe");
    let error = outcome.expect_err("fixture must stop after recovered supervision");
    assert!(
        !error
            .to_string()
            .contains("injected failing Codex sandbox probe"),
        "{error}"
    );
    let events = fs::read_to_string(&event_log).expect("read recovery events");
    assert!(events.contains("\"event\":\"child_adopted\""), "{events}");
    for identity in [
        durable.supervisor.as_ref().expect("durable supervisor"),
        durable.process.as_ref().expect("durable child"),
    ] {
        assert!(
            !bridge::cleanup_instance_is_live(identity).expect("inspect recovered identity"),
            "recovered executor identity was orphaned: {identity:?}"
        );
    }

    match previous_aliases {
        Some(value) => std::env::set_var("AUTOSPEC_HARNESS_RUNTIME_ALIASES", value),
        None => std::env::remove_var("AUTOSPEC_HARNESS_RUNTIME_ALIASES"),
    }
    match previous_claim {
        Some(value) => std::env::set_var("AUTOSPEC_TEST_EXACT_EVIDENCE_CLAIM", value),
        None => std::env::remove_var("AUTOSPEC_TEST_EXACT_EVIDENCE_CLAIM"),
    }
    let worktree_path = worktree.path.to_str().expect("worktree path");
    git(
        &fixture.repo,
        &["worktree", "remove", "--force", worktree_path],
    );
    let _ = fs::remove_dir_all(scope_root);
}
