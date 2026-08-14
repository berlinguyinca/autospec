// executor_bridge tests: terminal / label — 9 cases.
//
// Split out of tests.rs; see the note in that file.

use super::support_base::{test_environment, write_executable, zero_effect_classifier_fixture};
use super::support_invocation::implementation_proof_fixture;
use crate::commands::autonomous::executor_bridge as bridge;
use std::fs;
use std::path::PathBuf;

#[test]
fn autonomous_executor_bridge_zero_effect_marker_survives_repair_and_transfer_crashes() {
    let environment = test_environment();
    let (_fixture, state, state_path, _) =
        zero_effect_classifier_fixture("zero-effect-crash-windows", false, true);

    environment.zero_effect_recovery(bridge::ZeroEffectRecoveryFailpoint::AfterRepair);
    let repair_crash = bridge::prepare_zero_effect_recovery(&state_path, &state)
        .expect_err("interrupt after repaired worktree");
    environment.zero_effect_recovery(bridge::ZeroEffectRecoveryFailpoint::None);
    assert!(repair_crash.contains("after repair"), "{repair_crash}");
    assert!(state.identity.worktree.is_dir());
    assert!(
        bridge::recoverable_zero_effect_completion_for_state(&state_path, &state)
            .expect("recognize marked repaired state")
    );
    assert!(bridge::prepare_zero_effect_recovery(&state_path, &state)
        .expect("resume after repair crash"));

    environment.zero_effect_recovery(bridge::ZeroEffectRecoveryFailpoint::AfterTransfer);
    let transfer_crash = bridge::prepare_zero_effect_retry(&state_path, &state, None)
        .expect_err("interrupt after available transfer");
    environment.zero_effect_recovery(bridge::ZeroEffectRecoveryFailpoint::None);
    assert!(
        transfer_crash.contains("after transfer"),
        "{transfer_crash}"
    );
    assert!(
        bridge::recoverable_zero_effect_completion_for_state(&state_path, &state)
            .expect("recognize marked transferred state")
    );
    bridge::prepare_zero_effect_retry(&state_path, &state, None)
        .expect("resume after transfer crash");

    let (_runtime_fixture, mut runtime_state, runtime_state_path, _) =
        zero_effect_classifier_fixture("zero-effect-runtime-crash", false, true);
    runtime_state.identity.runtime_environment_dir = Some(
        runtime_state
            .identity
            .worktree
            .parent()
            .expect("runtime scope root")
            .join("runtime-environment"),
    );
    runtime_state.identity.runtime_session_id = Some("runtime-session".to_string());
    bridge::write_invocation_atomic(&runtime_state_path, &runtime_state)
        .expect("persist runtime-bound invocation");
    assert!(
        bridge::prepare_zero_effect_recovery(&runtime_state_path, &runtime_state)
            .expect("prepare runtime-bound recovery")
    );
    bridge::ensure_cleanup_record(
        &bridge::cleanup_record_path(&runtime_state_path, "runtime-complete"),
        &bridge::cleanup_binding(&runtime_state),
        "test runtime cleanup receipt",
    )
    .expect("persist completed runtime cleanup");
    assert!(
        !bridge::recovery_needs_runtime(&runtime_state, true),
        "marked zero-effect recovery must not reattach a released runtime"
    );
    environment.zero_effect_recovery(bridge::ZeroEffectRecoveryFailpoint::AfterRuntimeClose);
    let runtime_crash =
        bridge::prepare_zero_effect_retry(&runtime_state_path, &runtime_state, None)
            .expect_err("interrupt after durable runtime close");
    environment.zero_effect_recovery(bridge::ZeroEffectRecoveryFailpoint::None);
    assert!(
        runtime_crash.contains("after runtime close"),
        "{runtime_crash}"
    );
    assert!(bridge::recoverable_zero_effect_completion_for_state(
        &runtime_state_path,
        &runtime_state
    )
    .expect("recognize runtime-bound marked state"));
    bridge::prepare_zero_effect_retry(&runtime_state_path, &runtime_state, None)
        .expect("resume after runtime cleanup crash");

    let marker = bridge::zero_effect_recovery_marker_path(&state_path);
    fs::write(&marker, "{\"schema\":1,\"binding\":\"foreign\"}\n").expect("tamper recovery marker");
    assert!(
        bridge::recoverable_zero_effect_completion_for_state(&state_path, &state)
            .expect_err("marker mismatch remains fail-closed")
            .contains("identity mismatch")
    );
}

#[cfg(unix)]
fn run_complete_terminal_label_fixture(
    name: &str,
    label_mode: &str,
) -> (
    Result<bridge::BridgeRunReceipt, bridge::BridgeRunFailure>,
    bool,
    String,
    String,
) {
    let _environment = test_environment();
    let (fixture, mut state, state_path, _) = zero_effect_classifier_fixture(name, false, true);
    bridge::ensure_zero_effect_recovery_marker(&state_path, &state)
        .expect("persist zero-effect recovery marker");
    state.phase = bridge::BridgePhase::Complete;
    state.terminal_result = Some("retryable:executor_zero_effect_completion".to_string());
    bridge::write_invocation_atomic(&state_path, &state).expect("persist Complete invocation");

    let claimed = autospec_core::claim::RunStateRecord::new(
        state.identity.repository.clone(),
        state.identity.issue,
        state.identity.worker_id.clone(),
        "claimed",
        state.identity.branch.clone(),
        "",
        "claimed",
        Vec::new(),
        "2026-07-27T00:00:00Z",
        "2026-07-27T00:00:00Z",
        1,
    )
    .with_claim_id(state.identity.claim_id.clone());
    assert!(crate::commands::claim::advance_claim_ref_for_test(
        &state.identity.repository_path,
        &claimed,
    )
    .expect("seed claimed generation"));
    let released = autospec_core::claim::RunStateRecord::new(
        state.identity.repository.clone(),
        state.identity.issue,
        state.identity.worker_id.clone(),
        "released",
        state.identity.branch.clone(),
        "",
        "retryable_released",
        Vec::new(),
        "2026-07-27T00:00:00Z",
        "2026-07-27T00:00:00Z",
        2,
    )
    .with_claim_id(state.identity.claim_id.clone());
    assert!(crate::commands::claim::advance_claim_ref_for_test(
        &state.identity.repository_path,
        &released,
    )
    .expect("seed released claim"));

    let bin = fixture.root.join("bin");
    let arguments = fixture.root.join("terminal-label-arguments");
    fs::create_dir(&bin).expect("create fake gh directory");
    write_executable(
        &bin.join("gh"),
        r#"#!/bin/sh
set -eu
if [ "$1 $2" = "issue view" ]; then
  printf '%s\n' "$@" > "$GH_TERMINAL_LABEL_ARGUMENTS"
  projected=0
  if [ "${8:-}" = "--jq" ] &&
 [ "${9:-}" = '{labels: [.labels[] | {name: .name}]}' ]; then
projected=1
  fi
  case "$GH_TERMINAL_LABEL_MODE" in
metadata)
  if [ "$projected" -eq 1 ]; then
    printf '%s\n' '{"labels":[{"name":"bug"}]}'
  else
    printf '%s\n' '{"labels":[{"id":"LA_kwDO","name":"bug","description":"real metadata","color":"d73a4a"}]}'
  fi
  ;;
forbidden)
  if [ "$projected" -eq 1 ]; then
    printf '%s\n' '{"labels":[{"name":"in-progress-by-bot"}]}'
  else
    printf '%s\n' '{"labels":[{"id":"LA_lock","name":"in-progress-by-bot","description":"claim owner","color":"ededed"}]}'
  fi
  ;;
malformed)
  printf '%s\n' '{"labels":[{"name":7}]}'
  ;;
*)
  exit 65
  ;;
  esac
  exit 0
fi
exit 64
"#,
    );
    let previous_path = std::env::var_os("PATH");
    let previous_claim_remote = std::env::var_os("AUTOSPEC_CLAIM_GIT_REMOTE");
    let previous_label_mode = std::env::var_os("GH_TERMINAL_LABEL_MODE");
    let previous_label_arguments = std::env::var_os("GH_TERMINAL_LABEL_ARGUMENTS");
    std::env::set_var("PATH", format!("{}:/usr/bin:/bin", bin.display()));
    std::env::set_var("AUTOSPEC_CLAIM_GIT_REMOTE", fixture.root.join("remote.git"));
    std::env::set_var("GH_TERMINAL_LABEL_MODE", label_mode);
    std::env::set_var("GH_TERMINAL_LABEL_ARGUMENTS", &arguments);
    let request = bridge::ExecutorBridgeRequest {
        repository: state.identity.repository.clone(),
        repository_path: state.identity.repository_path.clone(),
        issue: state.identity.issue,
        issue_title: "Normalize terminal labels".to_string(),
        issue_body: "## Goal\n\nNormalize terminal labels.".to_string(),
        serialization_reasons: Vec::new(),
        worker_id: state.identity.worker_id.clone(),
        claim_id: state.identity.claim_id.clone(),
        invocation_id: state.identity.invocation_id.clone(),
        state_path: state_path.clone(),
        event_log: fixture.root.join("events.jsonl"),
    };
    let repository = request.repository.clone();

    let result = bridge::run_executor_bridge(&request);
    let receipt_exists = state_path.with_extension("terminal.json").is_file();
    let arguments = fs::read_to_string(arguments).expect("read terminal label arguments");
    for (key, previous) in [
        ("PATH", previous_path),
        ("AUTOSPEC_CLAIM_GIT_REMOTE", previous_claim_remote),
        ("GH_TERMINAL_LABEL_MODE", previous_label_mode),
        ("GH_TERMINAL_LABEL_ARGUMENTS", previous_label_arguments),
    ] {
        match previous {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }
    }
    (result, receipt_exists, arguments, repository)
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_terminal_label_projects_real_github_metadata() {
    // Break caught: removing the jq projection sends color/id/description into strict parsing.
    let (result, receipt_exists, arguments, repository) =
        run_complete_terminal_label_fixture("terminal-label-metadata", "metadata");

    let receipt = result.expect("real GitHub label metadata must publish the terminal receipt");
    assert!(matches!(
        receipt.status,
        bridge::BridgeRunStatus::Retryable { ref reason }
            if reason == "executor_zero_effect_completion"
    ));
    assert!(receipt_exists, "terminal receipt was not published");
    assert_eq!(
        arguments,
        format!(
            "issue\nview\n42\n--repo\n{repository}\n--json\nlabels\n--jq\n\
             {{labels: [.labels[] | {{name: .name}}]}}\n"
        )
    );
}

#[cfg(all(not(target_os = "linux"), unix))]
#[test]
fn released_predecessor_advances_through_executor_on_supported_host() {
    // Break caught: restoring the non-Linux admission stub rejects an exact released
    // predecessor before its already-complete invocation can publish a terminal receipt.
    let (result, receipt_exists, _, _) =
        run_complete_terminal_label_fixture("portable-released-predecessor", "metadata");

    let outcome = result.expect("portable autonomous run");
    assert!(matches!(
        outcome.status,
        bridge::BridgeRunStatus::Retryable { ref reason }
            if reason == "executor_zero_effect_completion"
    ));
    assert!(
        receipt_exists,
        "portable terminal receipt was not published"
    );
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_terminal_label_forbidden_owner_remains_fail_closed() {
    // Break caught: projection accidentally drops label names or bypasses ownership blocking.
    let (result, receipt_exists, _, _) =
        run_complete_terminal_label_fixture("terminal-label-forbidden", "forbidden");

    let error = result.expect_err("in-progress-by-bot must block terminal receipt publication");
    assert_eq!(
        error.detail,
        "terminal executor receipt still owns in-progress-by-bot"
    );
    assert!(matches!(
        error.kind,
        bridge::BridgeFailureKind::InvariantNeedsHuman
    ));
    assert!(!receipt_exists, "forbidden label wrote a terminal receipt");
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_terminal_label_malformed_projection_remains_fail_closed() {
    // Break caught: normalized output is accepted without exact type validation.
    let (result, receipt_exists, _, _) =
        run_complete_terminal_label_fixture("terminal-label-malformed", "malformed");

    let error = result.expect_err("malformed normalized labels must fail closed");
    assert_eq!(error.detail, "name must be a string");
    assert!(matches!(
        error.kind,
        bridge::BridgeFailureKind::InvariantNeedsHuman
    ));
    assert!(!receipt_exists, "malformed labels wrote a terminal receipt");
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_publishes_complete_receipt_before_marker_recovery() {
    let _environment = test_environment();
    let (fixture, mut state, state_path, _) =
        zero_effect_classifier_fixture("complete-before-marker", false, true);
    bridge::ensure_zero_effect_recovery_marker(&state_path, &state)
        .expect("persist zero-effect recovery marker");
    state.phase = bridge::BridgePhase::Complete;
    state.terminal_result = Some("retryable:executor_zero_effect_completion".to_string());
    bridge::write_invocation_atomic(&state_path, &state).expect("persist Complete invocation");

    let claimed = autospec_core::claim::RunStateRecord::new(
        state.identity.repository.clone(),
        state.identity.issue,
        state.identity.worker_id.clone(),
        "claimed",
        state.identity.branch.clone(),
        "",
        "claimed",
        Vec::new(),
        "2026-07-27T00:00:00Z",
        "2026-07-27T00:00:00Z",
        1,
    )
    .with_claim_id(state.identity.claim_id.clone());
    assert!(crate::commands::claim::advance_claim_ref_for_test(
        &state.identity.repository_path,
        &claimed,
    )
    .expect("seed claimed generation"));
    let released = autospec_core::claim::RunStateRecord::new(
        state.identity.repository.clone(),
        state.identity.issue,
        state.identity.worker_id.clone(),
        "released",
        state.identity.branch.clone(),
        "",
        "retryable_released",
        Vec::new(),
        "2026-07-27T00:00:00Z",
        "2026-07-27T00:00:00Z",
        2,
    )
    .with_claim_id(state.identity.claim_id.clone());
    assert!(crate::commands::claim::advance_claim_ref_for_test(
        &state.identity.repository_path,
        &released,
    )
    .expect("seed released claim"));
    let bin = fixture.root.join("bin");
    fs::create_dir(&bin).expect("create fake gh directory");
    write_executable(
        &bin.join("gh"),
        "#!/bin/sh\n\
         if [ \"$1 $2\" = \"issue view\" ]; then printf '%s\\n' '{\"labels\":[]}'; fi\n\
         exit 0\n",
    );
    let previous_path = std::env::var_os("PATH");
    let previous_claim_remote = std::env::var_os("AUTOSPEC_CLAIM_GIT_REMOTE");
    std::env::set_var("PATH", format!("{}:/usr/bin:/bin", bin.display()));
    std::env::set_var("AUTOSPEC_CLAIM_GIT_REMOTE", fixture.root.join("remote.git"));
    let request = bridge::ExecutorBridgeRequest {
        repository: state.identity.repository.clone(),
        repository_path: state.identity.repository_path.clone(),
        issue: state.identity.issue,
        issue_title: "Publish terminal receipt".to_string(),
        issue_body: "## Goal\n\nPublish the completed receipt.".to_string(),
        serialization_reasons: Vec::new(),
        worker_id: state.identity.worker_id.clone(),
        claim_id: state.identity.claim_id.clone(),
        invocation_id: state.identity.invocation_id.clone(),
        state_path: state_path.clone(),
        event_log: fixture.root.join("events.jsonl"),
    };

    let first = bridge::run_executor_bridge(&request)
        .expect("publish receipt from exact Complete invocation");
    let second = bridge::run_executor_bridge(&request).expect("reuse exact terminal receipt");
    match previous_path {
        Some(path) => std::env::set_var("PATH", path),
        None => std::env::remove_var("PATH"),
    }
    match previous_claim_remote {
        Some(remote) => std::env::set_var("AUTOSPEC_CLAIM_GIT_REMOTE", remote),
        None => std::env::remove_var("AUTOSPEC_CLAIM_GIT_REMOTE"),
    }

    assert_eq!(first, second, "startup must publish one idempotent receipt");
    assert!(matches!(
        first.status,
        bridge::BridgeRunStatus::Retryable { ref reason }
            if reason == "executor_zero_effect_completion"
    ));
    assert!(
        state_path.with_extension("terminal.json").is_file(),
        "Complete startup did not publish its terminal receipt"
    );
}

#[test]
fn autonomous_executor_bridge_tampered_implementation_marker_remains_fail_closed() {
    let (_fixture, state, state_path, _) =
        zero_effect_classifier_fixture("implementation-marker-tamper", false, true);
    bridge::ensure_zero_effect_recovery_marker(&state_path, &state)
        .expect("persist zero-effect recovery marker");
    fs::write(
        bridge::zero_effect_recovery_marker_path(&state_path),
        "{\"schema\":1,\"binding\":\"foreign\"}\n",
    )
    .expect("tamper zero-effect recovery marker");
    let request = bridge::ExecutorBridgeRequest {
        repository: state.identity.repository.clone(),
        repository_path: state.identity.repository_path.clone(),
        issue: state.identity.issue,
        issue_title: "Reject marker tamper".to_string(),
        issue_body: "## Goal\n\nReject the changed recovery marker.".to_string(),
        serialization_reasons: Vec::new(),
        worker_id: state.identity.worker_id.clone(),
        claim_id: state.identity.claim_id.clone(),
        invocation_id: state.identity.invocation_id.clone(),
        state_path,
        event_log: PathBuf::from("/tmp/unused-zero-effect-events.jsonl"),
    };

    let error = bridge::run_executor_bridge(&request)
        .expect_err("ImplementationComplete marker mismatch must fail closed");
    assert!(error.to_string().contains("identity mismatch"), "{error}");
}

#[test]
fn autonomous_executor_bridge_terminal_phases_bypass_zero_effect_recovery() {
    for phase in [
        bridge::BridgePhase::Merged,
        bridge::BridgePhase::CleanupPending,
        bridge::BridgePhase::Complete,
    ] {
        assert!(
            !bridge::startup_needs_zero_effect_recovery(phase),
            "{phase:?} must use terminal recovery instead of zero-effect classification"
        );
    }
    assert!(
        bridge::startup_needs_zero_effect_recovery(bridge::BridgePhase::ImplementationComplete),
        "ImplementationComplete marker evidence must remain fail-closed"
    );
}

#[test]
fn autonomous_executor_bridge_resumes_failure_after_terminal_claim_transition_crash() {
    let environment = test_environment();
    let (fixture, mut state, _snapshot, _) = implementation_proof_fixture("terminal-failure-crash");
    state.phase = bridge::BridgePhase::ImplementationComplete;
    state.identity.runtime_environment_dir = None;
    state.identity.runtime_session_id = None;
    let state_path = fixture.root.join("state/terminal-failure.json");
    bridge::write_invocation_atomic(&state_path, &state).expect("persist failure state");
    let scope_root = state
        .identity
        .worktree
        .parent()
        .expect("executor scope root")
        .to_path_buf();
    let issue = state.identity.issue;
    bridge::ensure_active_worktree_ownership(
        &state.identity.repository_path,
        &scope_root,
        issue,
        &state.identity.worktree,
        &state.identity.branch,
        &state.identity.claim_id,
        &state.identity.invocation_id,
    )
    .expect("seed active worktree ownership");

    environment.zero_effect_recovery(bridge::ZeroEffectRecoveryFailpoint::AfterClaimTransition);
    let interrupted = bridge::finalize_failed_executor_with_transition(
        &state_path,
        &mut state,
        None,
        None,
        true,
        false,
        "executor_zero_effect_completion",
        |_, disposition| {
            assert_eq!(disposition, bridge::BridgeClaimDisposition::Retryable);
            let transfer: serde_json::Value = serde_json::from_str(
                &fs::read_to_string(bridge::ownership_transfer_path(&scope_root, issue))
                    .expect("read ownership offered before claim release"),
            )
            .expect("parse ownership offered before claim release");
            assert_eq!(
                transfer["state"], "available",
                "retryable failure must offer retained worktree ownership before release"
            );
            Ok(bridge::BridgeClaimTransition::Transitioned)
        },
    )
    .expect_err("interrupt after terminal claim transition");
    environment.zero_effect_recovery(bridge::ZeroEffectRecoveryFailpoint::None);
    assert!(
        interrupted.to_string().contains("after claim transition"),
        "{interrupted}"
    );
    let durable = bridge::PersistedInvocation::from_json(
        &fs::read_to_string(&state_path).expect("read interrupted state"),
    )
    .expect("parse interrupted state");
    assert_eq!(durable.phase, bridge::BridgePhase::ImplementationComplete);
    assert_eq!(
        bridge::read_failure_cleanup_intent(&state_path, &durable)
            .expect("read exact failure cleanup intent")
            .reason,
        "executor_zero_effect_completion"
    );

    bridge::finalize_failed_executor_with_transition(
        &state_path,
        &mut state,
        None,
        None,
        true,
        false,
        "executor_zero_effect_completion",
        |_, _| Ok(bridge::BridgeClaimTransition::Transitioned),
    )
    .expect("resume terminal failure finalization");
    assert_eq!(state.phase, bridge::BridgePhase::Complete);
    assert_eq!(
        state.terminal_result.as_deref(),
        Some("retryable:executor_zero_effect_completion")
    );
}

#[test]
fn autonomous_executor_bridge_terminal_failure_recovery_requires_runtime_receipt() {
    let (fixture, mut state, _snapshot, _) =
        implementation_proof_fixture("terminal-runtime-receipt");
    state.phase = bridge::BridgePhase::ImplementationComplete;
    state.identity.claim_id = "claim-runtime".to_string();
    state.identity.invocation_id = "42-claim-runtime".to_string();
    state.identity.runtime_environment_dir = Some(fixture.root.join("runtime-environment"));
    state.identity.runtime_session_id = Some("runtime-session".to_string());
    let state_dir = fixture.root.join("state/executor");
    let generation = &bridge::sha256_hex(state.identity.claim_id.as_bytes())[..16];
    let state_path = state_dir.join(format!("issue-42-{generation}.json"));
    bridge::write_invocation_atomic(&state_path, &state).expect("persist runtime failure");
    bridge::ensure_failure_cleanup_intent(
        &state_path,
        &state,
        true,
        false,
        "executor_zero_effect_completion",
    )
    .expect("persist failure cleanup intent");
    let lease = crate::commands::claim::ClaimLease {
        issue: 42,
        repo: state.identity.repository.clone(),
        worker_id: state.identity.worker_id.clone(),
        branch: state.identity.branch.clone(),
        claim_id: state.identity.claim_id.clone(),
        session_id: None,
    };
    let receipt = bridge::cleanup_record_path(&state_path, "runtime-complete");

    let error = bridge::recover_terminal_failure_identity(&state_dir, &lease)
        .expect_err("missing runtime receipt remains fail-closed");
    assert!(error.contains("runtime receipt"), "{error}");
    assert!(
        !receipt.exists(),
        "read-only terminal recovery created a runtime receipt"
    );
}
