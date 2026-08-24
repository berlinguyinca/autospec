// executor_bridge tests: runtime / close — 7 cases.
//
// Split out of tests.rs; see the note in that file.

use super::support_base::{git, git_stdout, test_environment, write_executable};
use super::support_invocation::{
    commit_implementation, implementation_proof_fixture, session_record_ids,
};
use crate::commands::autonomous::executor_bridge as bridge;
use std::collections::BTreeMap;
use std::fs;
use std::sync::atomic::Ordering;

#[test]
fn autonomous_executor_bridge_runtime_close_recovers_after_receipt_gap() {
    let _environment = test_environment();
    let (fixture, mut state, _snapshot, _) = implementation_proof_fixture("runtime-close-recovery");
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
    let runtime = bridge::runtime_session_adapter(&state.identity.worktree)
        .expect("runtime adapter")
        .expect("runtime manifest");
    state.identity.runtime_session_id = Some(runtime.session_id.clone());
    let state_path = fixture.root.join("state/invocation.json");
    bridge::write_invocation_atomic(&state_path, &state).expect("runtime state");

    bridge::RUNTIME_CLOSE_FAILPOINT.store(1, Ordering::SeqCst);
    let error = bridge::finalize_failed_executor(
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
        bridge::read_failure_cleanup_intent(&state_path, &state)
            .expect("restart reads exact cleanup intent")
            .reason,
        "implementation failed"
    );
    assert!(
        bridge::cleanup_record_path(&state_path, "runtime-intent.json").exists(),
        "close intent must precede runtime mutation"
    );
    assert!(!bridge::cleanup_record_path(&state_path, "runtime-complete").exists());

    bridge::close_owned_runtime(&state_path, &state, None)
        .expect("restart proves absence and writes receipt");
    assert!(bridge::cleanup_record_path(&state_path, "runtime-complete").exists());
    assert!(session_record_ids(&state_root).is_empty());
    match previous {
        Some(value) => std::env::set_var("AGENT_ENV_STATE_ROOT", value),
        None => std::env::remove_var("AGENT_ENV_STATE_ROOT"),
    }
}

#[test]
fn autonomous_executor_bridge_reattaches_after_error_and_derives_cleanup_intent() {
    let _environment = test_environment();
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
    let runtime = bridge::runtime_session_adapter(&state.identity.worktree)
        .expect("runtime adapter")
        .expect("runtime manifest");
    let environment_dir = runtime.environment_dir().to_path_buf();
    let session_id = runtime.session_id.clone();
    state.phase = bridge::BridgePhase::CleanupPending;
    state.identity.runtime_environment_dir = Some(environment_dir.clone());
    state.identity.runtime_session_id = Some(session_id.clone());
    let state_path = fixture.root.join("state/invocation.json");
    bridge::write_invocation_atomic(&state_path, &state).expect("runtime state");

    drop(runtime);
    assert!(
        !bridge::cleanup_record_path(&state_path, "runtime-intent.json").exists(),
        "the crash window precedes cleanup intent"
    );
    fs::remove_file(autospec.join("runtime.yml"))
        .expect("remove mutable manifest before bridge reattach");
    let reattached = bridge::reattach_runtime_session_adapter(
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

    bridge::close_owned_runtime(&state_path, &state, None)
        .expect("cleanup from persisted original manifest and binding");
    assert!(bridge::cleanup_record_path(&state_path, "runtime-intent.json").exists());
    assert!(bridge::cleanup_record_path(&state_path, "runtime-complete").exists());
    match previous {
        Some(value) => std::env::set_var("AGENT_ENV_STATE_ROOT", value),
        None => std::env::remove_var("AGENT_ENV_STATE_ROOT"),
    }
}

#[test]
fn autonomous_executor_bridge_runtime_close_retries_partial_teardown_failure() {
    let _environment = test_environment();
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
    let runtime = bridge::runtime_session_adapter(&state.identity.worktree)
        .expect("runtime adapter")
        .expect("runtime manifest");
    state.identity.runtime_session_id = Some(runtime.session_id.clone());
    let state_path = fixture.root.join("state/invocation.json");
    bridge::write_invocation_atomic(&state_path, &state).expect("runtime state");

    let error = bridge::close_owned_runtime(&state_path, &state, Some(runtime))
        .expect_err("first teardown fails");
    assert!(error.contains("42") || error.contains("runtime"), "{error}");
    assert!(!bridge::cleanup_record_path(&state_path, "runtime-complete").exists());

    bridge::close_owned_runtime(&state_path, &state, None)
        .expect("restart retries authoritative teardown");
    assert!(bridge::cleanup_record_path(&state_path, "runtime-complete").exists());
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
    state.phase = bridge::BridgePhase::CleanupPending;
    state.identity.runtime_environment_dir = None;
    state.identity.runtime_session_id = None;
    let state_path = fixture.root.join("state/invocation.json");
    bridge::write_invocation_atomic(&state_path, &state).expect("cleanup state");
    git(
        &state.identity.repository_path,
        &[
            "worktree",
            "remove",
            state.identity.worktree.to_str().expect("worktree"),
        ],
    );

    let error = bridge::finalize_merged_executor(&state_path, &mut state, None)
        .expect_err("missing intent must fail closed");
    assert!(error.contains("before a durable removal intent"), "{error}");
}

#[test]
fn autonomous_executor_bridge_requires_owned_runtime_cleanup_proof() {
    let (fixture, mut state, _snapshot, _) = implementation_proof_fixture("cleanup-runtime-proof");
    state.phase = bridge::BridgePhase::CleanupPending;
    state.identity.runtime_session_id = Some("runtime-owned".into());
    let state_path = fixture.root.join("state/invocation.json");
    bridge::write_invocation_atomic(&state_path, &state).expect("cleanup state");

    let error = bridge::finalize_merged_executor(&state_path, &mut state, None)
        .expect_err("runtime receipt is mandatory");
    assert!(error.contains("runtime session"), "{error}");
    assert!(state.identity.worktree.exists());
}

#[test]
fn autonomous_executor_bridge_failure_budget_selects_queue_or_needs_human() {
    assert_eq!(
        bridge::failure_disposition(true, false),
        crate::commands::claim::BridgeClaimDisposition::Retryable
    );
    assert_eq!(
        bridge::failure_disposition(true, true),
        crate::commands::claim::BridgeClaimDisposition::NeedsHuman
    );
    assert_eq!(
        bridge::failure_disposition(false, false),
        crate::commands::claim::BridgeClaimDisposition::NeedsHuman
    );
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_closes_integration_issue() {
    let (fixture, mut state, _snapshot, _) =
        implementation_proof_fixture("close-integration-issue");
    state.phase = bridge::BridgePhase::Merged;
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
    let adapter = bridge::DraftPrAdapter {
        gh,
        environment: BTreeMap::from([
            ("ISSUE_CALLS".into(), calls.clone().into_os_string()),
            ("ISSUE_STATE".into(), issue_state.clone().into_os_string()),
        ]),
    };

    bridge::close_merged_integration_issue(&state, &adapter).expect("close continuation");
    let call_log = fs::read_to_string(&calls).expect("calls");
    assert_eq!(call_log.matches("issue close 101").count(), 1, "{call_log}");
    assert!(!call_log.contains("issue close 42"), "{call_log}");
    assert_eq!(state.phase, bridge::BridgePhase::Merged);

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
    bridge::close_merged_integration_issue(&state, &adapter)
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
    bridge::close_merged_integration_issue(&state, &adapter).expect("close legacy issue");
    let legacy_calls = fs::read_to_string(&calls).expect("legacy calls");
    assert_eq!(
        legacy_calls.matches("issue close 42").count(),
        1,
        "{legacy_calls}"
    );
    assert!(!legacy_calls.contains("issue close 101"), "{legacy_calls}");

    fs::write(&calls, "").expect("clear default-branch calls");
    state.identity.base_ref = "origin/main".into();
    bridge::close_merged_integration_issue(&state, &adapter).expect("default branch merge");
    assert_eq!(fs::read_to_string(&calls).expect("default calls"), "");

    state.identity.base_ref = "origin/integration".into();
    let mut mismatched = adapter.clone();
    mismatched
        .environment
        .insert("RETURN_NUMBER".into(), "99".into());
    assert!(bridge::close_merged_integration_issue(&state, &mismatched).is_err());
    assert_eq!(state.phase, bridge::BridgePhase::Merged);

    let mut failing = adapter.clone();
    failing.environment.insert("CLOSE_FAIL".into(), "1".into());
    fs::write(&issue_state, "OPEN\n").expect("reopen failed-close issue");
    assert!(bridge::close_merged_integration_issue(&state, &failing).is_err());
    assert_eq!(state.phase, bridge::BridgePhase::Merged);

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
    let error = bridge::close_merged_integration_issue(&state, &adapter)
        .expect_err("rewritten integration tip must fail closed");
    assert!(format!("{error:?}").contains("not an ancestor"));
    assert_eq!(fs::read_to_string(&calls).expect("rewritten-tip calls"), "");
}

#[cfg(target_os = "linux")]
#[test]
fn executor_supervision_live_harness_retains_executor_worktree() {
    // Break caught: the executor worktree was removed while its harness, Cargo, and test
    // descendants stayed alive with deleted working directories. Spec section 13.3
    // precondition 9 requires termination proof before deletion.
    let _environment = test_environment();
    let (fixture, mut state, _snapshot, _) = implementation_proof_fixture("cleanup-live-harness");
    commit_implementation(&state);
    state.phase = bridge::BridgePhase::CleanupPending;
    state.identity.runtime_environment_dir = None;
    state.identity.runtime_session_id = None;
    let mut harness = std::process::Command::new("/bin/sh")
        .args(["-c", "sleep 30"])
        .spawn()
        .expect("spawn live harness");
    let birth = bridge::observe_process_birth(harness.id())
        .expect("observe live harness")
        .expect("live harness birth");
    state.process = Some(bridge::ProcessIdentity {
        pid: birth.pid,
        process_group: birth.process_group,
        executable: std::path::PathBuf::from("/bin/sh"),
        argv_digest: "cleanup-live-harness".to_string(),
        boot_id: birth.boot_id,
        start_identity: birth.start_identity,
    });
    let state_path = fixture.root.join("state/invocation.json");
    bridge::write_invocation_atomic(&state_path, &state).expect("cleanup state");

    let failure = bridge::finalize_merged_executor(&state_path, &mut state, None)
        .expect_err("a live known child must retain the worktree");
    let _ = harness.kill();
    let _ = harness.wait();

    assert!(failure.contains("still uses"), "{failure}");
    assert!(state.identity.worktree.exists(), "worktree must survive");
    assert!(
        bridge::registered_worktree_paths(&state.identity.repository_path)
            .expect("registered worktrees")
            .iter()
            .any(|path| path == &state.identity.worktree),
        "worktree registration must survive"
    );
}
