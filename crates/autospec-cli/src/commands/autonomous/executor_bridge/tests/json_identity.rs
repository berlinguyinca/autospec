// executor_bridge tests: json / identity — 13 cases.
//
// Split out of tests.rs; see the note in that file.

use crate::commands::autonomous::executor_bridge as bridge;
#[cfg(target_os = "linux")]
use super::super::{BridgePhase, MutationSnapshot, SupervisionOutcome};
use super::super::{
    build_implementer_prompt, write_invocation_atomic, PersistedInvocation, ProcessIdentity,
};
#[cfg(target_os = "linux")]
use super::support_base::test_environment;
use super::support_base::{GitFixture, test_root};
#[cfg(target_os = "linux")]
use super::support_invocation::{
    detach_harness_for_adoption, supervision_config, supervision_state,
};
use super::support_invocation::persisted_invocation;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
#[cfg(target_os = "linux")]
use std::time::Duration;

#[test]
fn autonomous_executor_bridge_migrates_only_exact_legacy_generation_proof() {
    let root = test_root("legacy-generation-proof");
    let state_dir = root.join("executor");
    bridge::ensure_private_directory(&state_dir).expect("private executor state");
    let lease = crate::commands::claim::ClaimLease {
        issue: 42,
        repo: "owner/repo".to_string(),
        worker_id: "worker-1".to_string(),
        branch: "feat/autonomous-issue-42".to_string(),
        claim_id: "claim-42".to_string(),
        session_id: None,
    };
    let generation = &bridge::sha256_hex(lease.claim_id.as_bytes())[..16];
    let state_path = state_dir.join(format!("issue-42-{generation}.json"));
    let mut state = persisted_invocation();
    state.identity.invocation_id = "42-claim-42".to_string();
    write_invocation_atomic(&state_path, &state).expect("legacy active invocation");

    assert!(bridge::legacy_bridge_proves_claim(&state_dir, &lease)
        .expect("exact active generation proof"));

    state.identity.claim_id = "foreign-claim".to_string();
    bridge::write_private_atomic(
        &state_path,
        state.to_json().expect("serialize foreign proof").as_bytes(),
        "foreign legacy invocation",
    )
    .expect("replace legacy proof");
    let error = bridge::legacy_bridge_proves_claim(&state_dir, &lease)
        .expect_err("foreign generation cannot migrate");
    assert!(error.contains("authoritative claim"), "{error}");

    fs::remove_file(&state_path).expect("remove active proof");
    let receipt = bridge::BridgeRunReceipt {
        repository: lease.repo.clone(),
        issue: lease.issue,
        worker_id: lease.worker_id.clone(),
        branch: lease.branch.clone(),
        claim_id: lease.claim_id.clone(),
        invocation_id: "42-claim-42".to_string(),
        status: bridge::BridgeRunStatus::Merged {
            pull_request: 17,
            head_oid: "a".repeat(40),
            merge_oid: "b".repeat(40),
        },
    };
    bridge::write_private_create_once(
        &state_dir.join(format!("issue-42-{generation}.terminal.json")),
        format!("{}\n", receipt.to_json()).as_bytes(),
        "legacy terminal receipt",
    )
    .expect("terminal proof");
    assert!(bridge::legacy_bridge_proves_claim(&state_dir, &lease)
        .expect("exact completed generation proof"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn autonomous_executor_bridge_exact_invocation_probe_rejects_foreign_state() {
    let root = test_root("exact-invocation-probe");
    let state_dir = root.join("executor");
    bridge::ensure_private_directory(&state_dir).expect("private executor state");
    let lease = crate::commands::claim::ClaimLease {
        issue: 42,
        repo: "owner/repo".to_string(),
        worker_id: "worker-1".to_string(),
        branch: "feat/autonomous-issue-42".to_string(),
        claim_id: "claim-42".to_string(),
        session_id: None,
    };
    assert!(
        !bridge::exact_invocation_exists(&state_dir, &lease).expect("missing exact invocation"),
        "absence is the only state that permits pre-invocation retry"
    );

    let generation = &bridge::sha256_hex(lease.claim_id.as_bytes())[..16];
    let state_path = state_dir.join(format!("issue-42-{generation}.json"));
    let mut foreign = persisted_invocation();
    foreign.identity.invocation_id = "42-claim-42".to_string();
    foreign.identity.worker_id = "foreign-worker".to_string();
    write_invocation_atomic(&state_path, &foreign).expect("persist foreign invocation");

    assert!(bridge::exact_invocation_exists(&state_dir, &lease)
        .expect_err("foreign invocation must fail closed")
        .contains("does not match"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn autonomous_executor_bridge_keeps_provisioning_transport_failures_transient() {
    let failure = bridge::bridge_provision_failure(
        "TRANSIENT: fetch executor base: network unavailable".to_string(),
    );
    assert_eq!(failure.kind, bridge::BridgeFailureKind::Transient);
    assert!(failure.detail.contains("network unavailable"));
}

#[test]
fn autonomous_executor_bridge_persists_supervisor_birth_separately_from_harness() {
    // Break caught: a restarted conductor only knowing the short-lived harness PID and losing
    // the stable subreaper/process-group anchor after the harness exits.
    let expected = persisted_invocation();
    let value: serde_json::Value =
        serde_json::from_str(&expected.to_json().expect("serialize invocation"))
            .expect("parse invocation JSON");

    assert!(
        value.get("supervisor").is_some(),
        "supervisor birth identity must be a distinct strict persisted field"
    );
    assert_ne!(value["supervisor"], value["process"]);
}

#[test]
fn autonomous_executor_bridge_persisted_json_rejects_unknown_fields() {
    // Break caught: a newer or foreign state document being partially adopted.
    let expected = persisted_invocation();
    let mut value: serde_json::Value =
        serde_json::from_str(&expected.to_json().expect("serialize invocation"))
            .expect("parse test JSON");
    value
        .as_object_mut()
        .expect("invocation object")
        .insert("foreign".to_string(), serde_json::json!(true));

    let error = PersistedInvocation::from_json(&value.to_string())
        .expect_err("unknown fields must fail closed");

    assert!(
        error.contains("unexpected field"),
        "unexpected error: {error}"
    );
}

#[test]
fn autonomous_executor_bridge_persisted_json_requires_supported_schema() {
    // Break caught: missing, unsupported, or wrapped schema versions being adopted.
    let expected = persisted_invocation();
    let mut value: serde_json::Value =
        serde_json::from_str(&expected.to_json().expect("serialize invocation"))
            .expect("parse test JSON");
    value
        .as_object_mut()
        .expect("invocation object")
        .remove("schema");
    let error = PersistedInvocation::from_json(&value.to_string())
        .expect_err("missing schema must fail closed");
    assert!(error.contains("missing field"), "unexpected error: {error}");

    for (unsupported, expected_message) in [
        (2_u64, "unsupported invocation schema"),
        (4_294_967_297, "out of range"),
    ] {
        let mut value: serde_json::Value =
            serde_json::from_str(&expected.to_json().expect("serialize invocation"))
                .expect("parse test JSON");
        value
            .as_object_mut()
            .expect("invocation object")
            .insert("schema".to_string(), serde_json::json!(unsupported));
        let error = PersistedInvocation::from_json(&value.to_string())
            .expect_err("unsupported schema must fail closed");
        assert!(
            error.contains(expected_message),
            "unexpected error: {error}"
        );
    }
}

#[test]
fn autonomous_executor_bridge_persisted_json_rejects_process_identity_overflow() {
    // Break caught: oversized process IDs wrapping onto a different live process.
    let expected = persisted_invocation();
    for field in ["pid", "process_group"] {
        let mut value: serde_json::Value =
            serde_json::from_str(&expected.to_json().expect("serialize invocation"))
                .expect("parse test JSON");
        value
            .get_mut("process")
            .and_then(serde_json::Value::as_object_mut)
            .expect("process object")
            .insert(field.to_string(), serde_json::json!(u64::MAX));
        let error = PersistedInvocation::from_json(&value.to_string())
            .expect_err("oversized process identity must fail closed");
        assert!(error.contains("out of range"), "unexpected error: {error}");
    }
}

#[test]
fn autonomous_executor_bridge_process_identity_requires_every_component() {
    // Break caught: signaling a reused PID whose executable or boot/start identity differs.
    let expected = persisted_invocation().process.expect("process identity");
    let mut observed = expected.clone();
    assert!(expected.matches(&observed));

    observed.boot_id = "other-boot".to_string();
    assert!(!expected.matches(&observed));
    observed = expected.clone();
    observed.argv_digest = "c".repeat(64);
    assert!(!expected.matches(&observed));
    observed = expected.clone();
    observed.start_identity = "457".to_string();
    assert!(!expected.matches(&observed));
}

#[test]
fn autonomous_executor_bridge_live_harness_identity_allows_only_argv_mutation() {
    // Break caught: a harness self-reexec keeping its immutable lifetime and executable was
    // mistaken for PID reuse solely because its observable argv changed.
    let expected = persisted_invocation().process.expect("process identity");
    let mut observed = expected.clone();
    observed.argv_digest = "d".repeat(64);
    assert!(expected.matches_live_harness(&observed));

    for mutate in [
        |identity: &mut ProcessIdentity| identity.pid += 1,
        |identity: &mut ProcessIdentity| identity.process_group += 1,
        |identity: &mut ProcessIdentity| identity.executable = PathBuf::from("/other"),
        |identity: &mut ProcessIdentity| identity.boot_id = "other-boot".to_string(),
        |identity: &mut ProcessIdentity| identity.start_identity = "other-start".to_string(),
    ] {
        let mut changed = observed.clone();
        mutate(&mut changed);
        assert!(!expected.matches_live_harness(&changed));
    }
}

#[test]
fn autonomous_executor_bridge_prompt_binds_local_only_authority() {
    let invocation = persisted_invocation();
    let closeout = Path::new("/safe/worktree/.autospec/closeout.json");

    let prompt = build_implementer_prompt(
        &invocation.identity,
        "Executor bridge stalls",
        "Implement the exact acceptance criteria.",
        closeout,
    )
    .expect("build bounded prompt");

    for required in [
        "owner/repo",
        "issue #42",
        "claim-42",
        "feat/autonomous-issue-42",
        "/safe/worktree",
        "refs/remotes/origin/main",
        "Implement the exact acceptance criteria.",
        "/safe/worktree/.autospec/closeout.json",
        "MUST NOT push",
        "MUST NOT create, edit, ready, close, or merge a pull request",
        "MUST NOT mutate remote Git or GitHub state",
        "MUST NOT create local commits or replace the worktree's Git metadata",
        "Autospec Rust owns local commits",
    ] {
        assert!(
            prompt.contains(required),
            "prompt omitted required binding: {required}"
        );
    }
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_exit_record_requires_complete_synced_fence() {
    let fixture = GitFixture::new("exit-record-fence");
    let exit_record = fixture.root.join("executor.exit");
    fs::write(&exit_record, [0_u8; 16]).expect("zero exit record");
    fs::set_permissions(&exit_record, fs::Permissions::from_mode(0o600))
        .expect("private exit record");

    assert_eq!(
        bridge::read_live_executor_exit_status(&exit_record).expect("live pending record"),
        None,
        "a prompt without its durable record cannot authorize success"
    );
    assert_eq!(
        bridge::read_executor_exit_status(&exit_record).expect("dead incomplete record"),
        None,
        "an absent durable record cannot become recovery truth"
    );

    let mut partial = [0_u8; 16];
    partial[..4].copy_from_slice(&0_i32.to_ne_bytes());
    partial[4..6].copy_from_slice(b"EX");
    fs::write(&exit_record, partial).expect("partial exit record");
    assert_eq!(
        bridge::read_live_executor_exit_status(&exit_record).expect("live partial record"),
        None,
        "a partial record remains pending while the exact supervisor is live"
    );
    assert!(bridge::read_executor_exit_status(&exit_record)
        .expect_err("dead partial record must fail closed")
        .contains("malformed"));

    let mut precommit = [0_u8; 16];
    precommit[..4].copy_from_slice(&0_i32.to_ne_bytes());
    precommit[4..8].copy_from_slice(b"EXIT");
    fs::write(&exit_record, precommit).expect("precommit exit record");
    assert_eq!(
        bridge::read_live_executor_exit_status(&exit_record).expect("live precommit record"),
        None
    );
    assert!(bridge::read_executor_exit_status(&exit_record)
        .expect_err("dead precommit record must fail closed")
        .contains("malformed"));

    let mut mismatch = precommit;
    mismatch[8..12].copy_from_slice(&7_i32.to_ne_bytes());
    mismatch[12..].copy_from_slice(b"DONE");
    fs::write(&exit_record, mismatch).expect("mismatched exit record");
    assert_eq!(
        bridge::read_live_executor_exit_status(&exit_record).expect("live mismatch record"),
        None
    );
    assert!(bridge::read_executor_exit_status(&exit_record)
        .expect_err("dead mismatch record must fail closed")
        .contains("malformed"));

    let mut complete = [0_u8; 16];
    complete[..4].copy_from_slice(&7_i32.to_ne_bytes());
    complete[4..8].copy_from_slice(b"EXIT");
    complete[8..12].copy_from_slice(&7_i32.to_ne_bytes());
    complete[12..].copy_from_slice(b"DONE");
    fs::write(&exit_record, complete).expect("complete exit record");
    assert_eq!(
        bridge::read_executor_exit_status(&exit_record).expect("complete exit record"),
        Some(7)
    );
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_adopts_daemonizing_success_and_failure() {
    let _environment = test_environment();
    for exit_code in [0, 7] {
        let fixture = GitFixture::new(&format!("adopt-daemon-{exit_code}"));
        let mut state = supervision_state(&fixture);
        let state_path = fixture.root.join("state/invocation.json");
        let event_log = fixture.root.join("log/executor.jsonl");
        let descendant_pid = fixture.root.join("descendant.pid");
        let script = format!(
            "sleep 0.1; sleep 30 & child=$!; printf '%s\\n' \"$child\" > '{}'; printf 'adopted-{exit_code}\\n'; exit {exit_code}",
            descendant_pid.display()
        );
        let validated = detach_harness_for_adoption(&fixture, &state_path, &mut state, &script);
        let snapshot = MutationSnapshot::capture(&fixture.repo, &state.identity.branch)
            .expect("adoption snapshot");

        let error = bridge::supervise_validated_harness(
            &state_path,
            &event_log,
            &mut state,
            &validated,
            &snapshot,
            supervision_config(2_000),
        )
        .expect_err("adopted retained writer must not authorize terminal exit");

        assert!(error.contains("output-complete timeout"), "{error}");
        assert_eq!(state.phase, BridgePhase::Interrupted);
        assert!(state.supervisor.is_none());
        assert!(state.process.is_none());
        let descendant = fs::read_to_string(&descendant_pid)
            .expect("daemon identity")
            .trim()
            .to_string();
        for _ in 0..40 {
            if !Path::new(&format!("/proc/{descendant}")).exists() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            !Path::new(&format!("/proc/{descendant}")).exists(),
            "adopted daemon survived exact pidfd cleanup"
        );
        let events = fs::read_to_string(&event_log).expect("adopted events");
        assert!(events.contains("\"event\":\"child_adopted\""), "{events}");
        assert!(!events.contains("\"event\":\"child_exited\""), "{events}");
        assert!(events.contains(&format!("adopted-{exit_code}")), "{events}");
    }
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_adopted_restart_waits_for_delayed_stderr_tail() {
    let _environment = test_environment();
    let fixture = GitFixture::new("adopt-delayed-tail");
    let mut state = supervision_state(&fixture);
    let state_path = fixture.root.join("state/invocation.json");
    let event_log = fixture.root.join("log/executor.jsonl");
    let validated = detach_harness_for_adoption(
        &fixture,
        &state_path,
        &mut state,
        "(sleep 0.2; printf 'adopted-delayed-tail\\n' >&2) & exit 0",
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
    .expect("adopted delayed tail reaches durable completion");
    let events = fs::read_to_string(event_log).expect("adopted delayed events");

    assert_eq!(outcome, SupervisionOutcome::Exited { exit_code: 0 });
    assert!(events.contains("adopted-delayed-tail"), "{events}");
    assert!(events.contains("\"event\":\"child_exited\""), "{events}");
}
