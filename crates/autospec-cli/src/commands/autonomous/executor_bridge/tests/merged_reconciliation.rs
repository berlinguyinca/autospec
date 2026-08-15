// executor_bridge tests: merged / reconciliation — 5 cases.
//
// Split out of tests.rs; see the note in that file.

use super::super::ResolvedBase;
use super::support_base::{git, git_stdout, test_environment, DetachedForkedCleanup};
use super::support_invocation::{commit_implementation, implementation_proof_fixture};
use super::support_launch::DRAFT_ISSUE_BODY;
use crate::commands::autonomous::executor_bridge as bridge;
use std::collections::BTreeMap;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::process::Command;
use std::time::{Duration, Instant};

#[test]
fn autonomous_executor_bridge_remote_snapshot_ignores_claim_ledger_refs() {
    let document = format!(
        "{}\trefs/heads/main\n{}\trefs/autospec/claims/issue-42\n",
        "a".repeat(40),
        "b".repeat(40)
    );

    assert_eq!(
        bridge::parse_bridge_remote_refs(&document).expect("claim ledger is separate authority"),
        BTreeMap::from([("refs/heads/main".to_string(), "a".repeat(40))])
    );
    assert!(
        bridge::parse_bridge_remote_refs(&format!(
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
        bridge::parse_observed_merge(document, 17, &"a".repeat(40), "main",).expect("merged"),
        "b".repeat(40)
    );
    assert!(bridge::parse_observed_merge(
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
    let _environment = test_environment();
    let (fixture, mut state, _snapshot, closeout) =
        implementation_proof_fixture("merged-existing-worktree-entrypoint");
    commit_implementation(&state);
    let persisted_head = git_stdout(&state.identity.worktree, &["rev-parse", "HEAD"]);
    state.phase = bridge::BridgePhase::DraftCreated;
    state.pr = Some(17);
    state.head_oid = Some(persisted_head.clone());
    state.closeout_path = Some(fs::canonicalize(&closeout).expect("canonical closeout"));
    let closeout_body = fs::read_to_string(&closeout).expect("read closeout");
    state.closeout_digest = Some(bridge::sha256_hex(closeout_body.as_bytes()));
    state.identity.runtime_environment_dir = None;
    state.identity.runtime_session_id = None;
    state.supervisor = None;
    state.process = None;
    state.draft_process = None;
    bridge::record_worktree_creation_identity(
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
    bridge::write_invocation_atomic(&state_path, &state).expect("persist stale draft state");
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
            "body": bridge::canonical_pull_request_body(&state, &closeout_body).unwrap(),
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
    let request = bridge::ExecutorBridgeRequest {
        repository: state.identity.repository.clone(),
        repository_path: state.identity.repository_path.clone(),
        issue: state.identity.issue,
        issue_title: "Retire merged executor".to_string(),
        issue_body: DRAFT_ISSUE_BODY.to_string(),
        serialization_reasons: Vec::new(),
        worker_id: state.identity.worker_id.clone(),
        claim_id: state.identity.claim_id.clone(),
        invocation_id: state.identity.invocation_id.clone(),
        state_path: state_path.clone(),
        event_log: fixture.root.join("events.jsonl"),
    };

    let outcome = bridge::run_executor_bridge_with_codex_probe(&request, |_| {
        panic!("merged recovery must precede Codex probing")
    });
    let error = outcome.expect_err("failpoint stops after merged reconciliation");
    assert!(
        error.to_string().contains("injected executor crash"),
        "{error}"
    );
    let durable = bridge::PersistedInvocation::from_json(
        &fs::read_to_string(&state_path).expect("read reconciled invocation"),
    )
    .expect("parse reconciled invocation");
    assert_eq!(durable.phase, bridge::BridgePhase::Merged);
    assert_eq!(durable.head_oid.as_deref(), Some(merged_head.as_str()));
    assert_eq!(
        durable.terminal_result.as_deref(),
        Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
    );
    assert!(bridge::cleanup_record_path(&state_path, "merged-reconciliation").exists());
    assert!(
        state.identity.worktree.exists(),
        "failpoint must precede cleanup"
    );
    let receipt = bridge::run_executor_bridge_with_codex_probe(&request, |_| {
        panic!("merged restart must finalize before Codex probing")
    })
    .expect("restart finalizes the reconciled merge");
    let complete = bridge::PersistedInvocation::from_json(
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
        bridge::BridgeRunStatus::Merged {
            pull_request: 17,
            ref head_oid,
            ref merge_oid,
        } if head_oid == &merged_head && merge_oid == &"b".repeat(40)
    ));
    assert_eq!(complete.phase, bridge::BridgePhase::Complete);
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
    state.phase = bridge::BridgePhase::DraftCreated;
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
    let adapter = bridge::DraftPrAdapter {
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
        "body": bridge::canonical_pull_request_body(&state, closeout).unwrap(),
    });
    for phase in [
        bridge::BridgePhase::Merged,
        bridge::BridgePhase::CleanupPending,
        bridge::BridgePhase::Complete,
    ] {
        state.phase = phase;
        for (body, accepted) in [("Closes #101", true), ("Closes #42", false)] {
            fs::write(&observation, exact.to_string().replace("Closes #101", body)).unwrap();
            assert_eq!(
                bridge::revalidate_live_canonical_pull_request(&state, &adapter).is_ok(),
                accepted
            );
        }
    }
    state.phase = bridge::BridgePhase::DraftCreated;
    fs::write(&observation, exact.to_string()).expect("exact observation");
    let state_path = fixture.root.join("state/exact.json");
    bridge::write_invocation_atomic(&state_path, &state).expect("persist pre-reconciliation");
    let mut reconciled = state.clone();
    assert!(bridge::reconcile_exact_merged_invocation_with_refresh(
        &state_path,
        &mut reconciled,
        &adapter,
        || Ok(bridge::BridgeClaimOwnership::Refreshed { ttl_seconds: 60 }),
    )
    .expect("exact merged reconciliation"));
    assert_eq!(reconciled.phase, bridge::BridgePhase::Merged);
    assert_eq!(reconciled.head_oid.as_deref(), Some(merged_head.as_str()));
    assert_eq!(
        reconciled.terminal_result.as_deref(),
        Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
    );
    bridge::validate_merged_reconciliation_record(&state_path, &reconciled)
        .expect("bound reconciliation record");
    let mut rebound = reconciled.clone();
    rebound.current_child = Some(102);
    assert!(bridge::validate_merged_reconciliation_record(&state_path, &rebound).is_err());

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
        let outcome = bridge::reconcile_exact_merged_invocation_with_refresh(
            &candidate_path,
            &mut candidate,
            &adapter,
            || Ok(bridge::BridgeClaimOwnership::Refreshed { ttl_seconds: 60 }),
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

    let reconciliation_path = bridge::cleanup_record_path(&state_path, "merged-reconciliation");
    let mut changed_record: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(&reconciliation_path).expect("read reconciliation"),
    )
    .expect("parse reconciliation");
    changed_record["persisted_head"] = serde_json::json!(state.identity.base_oid);
    fs::write(&reconciliation_path, changed_record.to_string())
        .expect("change reconciliation record");
    assert!(
        bridge::validate_merged_reconciliation_record(&state_path, &reconciled).is_err(),
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
    let error = bridge::reconcile_exact_merged_invocation_with_refresh(
        &nonancestor_path,
        &mut nonancestor,
        &adapter,
        || Ok(bridge::BridgeClaimOwnership::Refreshed { ttl_seconds: 60 }),
    )
    .expect_err("non-ancestor merged head must fail closed");
    assert!(error.to_string().contains("not contained"), "{error}");
    assert_eq!(nonancestor, state);
    assert!(!nonancestor_path.exists());

    fs::write(&observation, exact.to_string()).expect("restore exact observation");
    let mut lost = state.clone();
    let lost_path = fixture.root.join("state/ownership-lost.json");
    let error = bridge::reconcile_exact_merged_invocation_with_refresh(
        &lost_path,
        &mut lost,
        &adapter,
        || Ok(bridge::BridgeClaimOwnership::Lost),
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
    let mut cleanup = DetachedForkedCleanup::new(child.id()).expect("arm live executor cleanup");
    let deadline = Instant::now() + Duration::from_secs(2);
    let identity = loop {
        if let Some(identity) =
            bridge::observe_process_identity(child.id(), &bridge::argv_digest(&args))
                .expect("observe live executor")
        {
            if identity.executable == executable
                && identity.argv_digest == bridge::argv_digest(&args)
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
        !bridge::executor_terminal_processes_are_quiescent(&state)
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
