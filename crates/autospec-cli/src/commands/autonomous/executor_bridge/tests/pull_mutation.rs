// executor_bridge tests: pull / mutation — 9 cases.
//
// Split out of tests.rs; see the note in that file.

use super::super::{supervise_harness, BridgePhase, MutationSnapshot, SupervisionOutcome};
use super::support_base::{
    git, git_stdout, test_environment, write_executable, zero_effect_classifier_fixture,
    GitFixture,
};
use super::support_invocation::{
    commit_implementation, implementation_proof_fixture, shell_invocation, supervision_config,
    supervision_state,
};
use super::support_launch::{
    adapter_path, draft_pr_adapter_fixture, prepared_draft_transaction, DRAFT_ISSUE_BODY,
};
use crate::commands::autonomous::executor_bridge as bridge;
use std::fs;
use std::time::Duration;

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_excludes_server_managed_pull_refs_from_mutation_ownership() {
    // Break caught: GitHub refs/pull advertisements making the operator-owned ref proof unusable.
    let mut prepared = prepared_draft_transaction("draft-server-managed-refs");
    let remote = prepared.fixture.root.join("remote.git");
    git(
        &prepared.fixture.root,
        &[
            "--git-dir",
            remote.to_str().expect("remote path"),
            "update-ref",
            "refs/pull/1/head",
            &prepared.state.identity.base_oid,
        ],
    );
    prepared.adapter.environment.insert(
        "GH_MUTATE_PULL_REF".into(),
        prepared.state.identity.base_oid.clone().into(),
    );

    let pull_request = prepared
        .publish()
        .expect("server-managed pull refs must be excluded");

    assert_eq!(pull_request, 17);
    assert_eq!(prepared.state.phase, BridgePhase::DraftCreated);
    assert_eq!(
        git_stdout(
            &prepared.fixture.root,
            &[
                "--git-dir",
                remote.to_str().expect("remote path"),
                "rev-parse",
                "refs/pull/17/head",
            ],
        ),
        prepared.state.identity.base_oid
    );
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_snapshot_is_create_once_and_full_identity_bound() {
    // Break caught: a same-invocation recapture blessing a new remote baseline.
    let (fixture, mut state, _, _) = implementation_proof_fixture("snapshot-create-once");
    let state_path = fixture.root.join("state/invocation.json");
    state.phase = BridgePhase::Pending;
    bridge::write_invocation_atomic(&state_path, &state).expect("pending invocation");
    let adapter = draft_pr_adapter_fixture(&fixture, &state_path, "[]");

    let first =
        bridge::RemoteMutationSnapshot::capture_and_persist(&state_path, &mut state, &adapter)
            .expect("first prelaunch snapshot");
    let persisted = fs::read_to_string(&state_path).expect("persisted invocation");
    assert!(
        persisted.contains("\"remote_snapshot_digest\":\""),
        "invocation must bind the snapshot digest"
    );
    state.remote_snapshot_digest = None;
    bridge::write_invocation_atomic(&state_path, &state)
        .expect("simulate crash before digest binding");
    let recovered =
        bridge::RemoteMutationSnapshot::capture_and_persist(&state_path, &mut state, &adapter)
            .expect("adopt create-once snapshot after binding crash");
    assert_eq!(recovered, first);
    assert!(state.remote_snapshot_digest.is_some());

    let error =
        bridge::RemoteMutationSnapshot::capture_and_persist(&state_path, &mut state, &adapter)
            .expect_err("snapshot recapture must fail closed");
    assert!(
        error.contains("exists") || error.contains("once"),
        "{error}"
    );

    state.identity.worker_id = "foreign-worker".to_string();
    let error = bridge::RemoteMutationSnapshot::load(&state_path, &state)
        .expect_err("full invocation identity mismatch must fail closed");
    assert!(
        error.contains("identity") || error.contains("digest"),
        "{error}"
    );
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_snapshot_admits_exact_adopted_base_merge() {
    let (fixture, mut state, state_path, _) =
        zero_effect_classifier_fixture("snapshot-adopted-base-merge", false, false);
    let implementation = state.identity.worktree.join("implementation.txt");
    fs::write(&implementation, "preserved implementation\n").expect("write implementation");
    git(&state.identity.worktree, &["add", "implementation.txt"]);
    git(
        &state.identity.worktree,
        &["commit", "-m", "test: preserve adopted implementation"],
    );
    let transfer_head = git_stdout(&state.identity.worktree, &["rev-parse", "HEAD"]);
    bridge::ensure_active_worktree_ownership(
        &state.identity.repository_path,
        state.identity.worktree.parent().expect("scope root"),
        state.identity.issue,
        &state.identity.worktree,
        &state.identity.branch,
        &state.identity.claim_id,
        &state.identity.invocation_id,
    )
    .expect("record exact adopted transfer");

    let seed = fixture.root.join("seed");
    git(&seed, &["checkout", "main"]);
    fs::write(seed.join("base-advance.txt"), "advanced base\n").expect("advance base");
    git(&seed, &["add", "base-advance.txt"]);
    git(&seed, &["commit", "-m", "test: advance adopted base"]);
    git(&seed, &["push", "origin", "main"]);
    let advanced_base = git_stdout(&seed, &["rev-parse", "HEAD"]);
    git(&state.identity.worktree, &["fetch", "origin", "main"]);
    git(
        &state.identity.worktree,
        &["merge", "--no-ff", "--no-edit", &advanced_base],
    );
    let merged_head = git_stdout(&state.identity.worktree, &["rev-parse", "HEAD"]);
    assert_eq!(
        git_stdout(&state.identity.worktree, &["show", "-s", "--format=%P", "HEAD"]),
        format!("{transfer_head} {advanced_base}")
    );

    let snapshot_path = bridge::remote_snapshot_path(&state_path);
    fs::remove_file(&snapshot_path).expect("remove old prelaunch snapshot");
    state.phase = BridgePhase::Pending;
    state.identity.base_oid = advanced_base;
    state.remote_snapshot_digest = None;
    bridge::write_invocation_atomic(&state_path, &state).expect("persist successor invocation");
    let adapter = draft_pr_adapter_fixture(&fixture, &state_path, "[]");

    bridge::RemoteMutationSnapshot::capture_and_persist(&state_path, &mut state, &adapter)
        .expect("exact adopted base merge is a valid prelaunch HEAD");
    let snapshot: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(snapshot_path).expect("read successor snapshot"),
    )
    .expect("parse successor snapshot");
    assert_eq!(snapshot["identity"]["local_head"], merged_head);
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_snapshot_recovery_rejects_foreign_and_malformed_files() {
    // Break caught: Pending crash recovery binding a pre-existing snapshot without validation.
    for variant in ["foreign", "malformed"] {
        let (fixture, mut state, _, _) =
            implementation_proof_fixture(&format!("snapshot-recovery-{variant}"));
        let state_path = fixture.root.join("state/invocation.json");
        state.phase = BridgePhase::Pending;
        bridge::write_invocation_atomic(&state_path, &state).expect("pending invocation");
        let adapter = draft_pr_adapter_fixture(&fixture, &state_path, "[]");
        bridge::RemoteMutationSnapshot::capture_and_persist(&state_path, &mut state, &adapter)
            .expect("initial snapshot");
        state.remote_snapshot_digest = None;
        bridge::write_invocation_atomic(&state_path, &state)
            .expect("simulate digest-binding crash");
        let snapshot_path = state_path.with_extension("prelaunch-remote.json");
        if variant == "foreign" {
            let mut value: serde_json::Value =
                serde_json::from_str(&fs::read_to_string(&snapshot_path).unwrap()).unwrap();
            value["identity"]["worker_id"] = "foreign-worker".into();
            fs::write(&snapshot_path, value.to_string()).expect("foreign snapshot");
        } else {
            fs::write(&snapshot_path, "{malformed").expect("malformed snapshot");
        }

        let error =
            bridge::RemoteMutationSnapshot::capture_and_persist(&state_path, &mut state, &adapter)
                .expect_err("invalid existing snapshot must not be rebound");

        assert!(
            error.contains("identity") || error.contains("parse"),
            "{variant}: {error}"
        );
        assert!(state.remote_snapshot_digest.is_none());
    }
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_proof_recovery_preserves_phase_and_artifact_digest() {
    // Break caught: recovery reconstructing an in-memory body or regressing a mutation phase.
    let (fixture, mut state, snapshot, closeout) =
        implementation_proof_fixture("proof-durable-recovery");
    let state_path = fixture.root.join("state/invocation.json");
    commit_implementation(&state);
    bridge::prove_implementation(&state_path, &mut state, &snapshot, &closeout)
        .expect("initial proof");
    let persisted = fs::read_to_string(&state_path).expect("persisted proof");
    assert!(persisted.contains("\"closeout_path\":\""), "{persisted}");
    assert!(persisted.contains("\"closeout_digest\":\""), "{persisted}");

    state.phase = BridgePhase::BranchPushed;
    bridge::write_invocation_atomic(&state_path, &state).expect("mutation phase");
    let recovered = bridge::prove_implementation(&state_path, &mut state, &snapshot, &closeout)
        .expect("phase-preserving proof recovery");

    assert_eq!(state.phase, BridgePhase::BranchPushed);
    assert_eq!(recovered.head_oid, state.head_oid.unwrap());
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_rejects_a_forged_closeout_body_at_mutation_boundary() {
    // Break caught: a caller constructing an exact-head proof with an unvalidated PR body.
    let mut prepared = prepared_draft_transaction("proof-forged-closeout");
    let forged = bridge::ImplementationProof {
        head_oid: prepared.proof.head_oid.clone(),
        closeout_body: prepared
            .proof
            .closeout_body
            .replace("Result: shipped", "Result: forged"),
    };

    let error = bridge::push_and_create_draft(
        &prepared.state_path,
        &mut prepared.state,
        &forged,
        "Implement issue",
        DRAFT_ISSUE_BODY,
        &prepared.adapter,
    )
    .expect_err("forged proof body must fail before mutation");

    assert!(error.contains("digest"), "{error}");
    let calls = fs::read_to_string(prepared.fixture.root.join("gh-calls")).expect("gh calls");
    assert_eq!(calls.matches("pr create").count(), 0);
    assert!(git_stdout(
        &prepared.fixture.root,
        &[
            "--git-dir",
            prepared.fixture.root.join("remote.git").to_str().unwrap(),
            "show-ref",
        ],
    )
    .lines()
    .all(|line| !line.ends_with(&format!("refs/heads/{}", prepared.state.identity.branch))));
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_rejects_saturated_pull_request_inventory() {
    // Break caught: a 100-row gh result silently hiding additional open pull requests.
    let (fixture, mut state, _, _) = implementation_proof_fixture("draft-pr-saturation");
    let state_path = fixture.root.join("state/invocation.json");
    state.phase = BridgePhase::Pending;
    bridge::write_invocation_atomic(&state_path, &state).expect("pending invocation");
    let adapter = draft_pr_adapter_fixture(&fixture, &state_path, "[]");
    let pull_requests = (1..=100)
        .map(|number| {
            serde_json::json!({
                "number": number,
                "body": format!("Closes #{number}"),
                "headRefName": format!("foreign-{number}"),
                "headRefOid": "ffffffffffffffffffffffffffffffffffffffff",
                "isDraft": true,
                "baseRefName": "main"
            })
        })
        .collect::<Vec<_>>();
    fs::write(
        adapter_path(&adapter, "GH_PR_STATE"),
        serde_json::to_string(&pull_requests).unwrap(),
    )
    .expect("saturated PR fixture");

    let error =
        bridge::RemoteMutationSnapshot::capture_and_persist(&state_path, &mut state, &adapter)
            .expect_err("saturated PR inventory must fail closed");

    assert!(
        error.contains("100") || error.contains("saturat"),
        "{error}"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_clean_supervision_restores_prior_subreaper_state() {
    let _environment = test_environment();
    nix::sys::prctl::set_child_subreaper(false).expect("clear fixture subreaper state");
    let fixture = GitFixture::new("subreaper-restore");
    let mut state = supervision_state(&fixture);
    let snapshot =
        MutationSnapshot::capture(&fixture.repo, &state.identity.branch).expect("snapshot");

    let outcome = supervise_harness(
        &fixture.root.join("state/invocation.json"),
        &fixture.root.join("log/executor.jsonl"),
        &mut state,
        &shell_invocation(&fixture.repo, "exit 0"),
        &snapshot,
        supervision_config(500),
    )
    .expect("clean child supervision");
    let mut observed = 0_i32;
    // SECURITY-REVIEW: independent #2598 reviewer LGTM; read-only process-state probe.
    // SAFETY: PR_GET_CHILD_SUBREAPER writes one integer to the supplied valid pointer.
    let get_result = unsafe {
        nix::libc::prctl(
            nix::libc::PR_GET_CHILD_SUBREAPER,
            std::ptr::addr_of_mut!(observed),
            0,
            0,
            0,
        )
    };
    nix::sys::prctl::set_child_subreaper(false).expect("clean RED subreaper state");

    assert_eq!(outcome, SupervisionOutcome::Exited { exit_code: 0 });
    assert_eq!(get_result, 0, "read child-subreaper state");
    assert_eq!(
        observed, 0,
        "fully-cleaned supervision leaked process-global subreaper ownership"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_clean_supervision_preserves_enabled_subreaper_state() {
    let _environment = test_environment();
    nix::sys::prctl::set_child_subreaper(true).expect("enable fixture subreaper state");
    let fixture = GitFixture::new("subreaper-preserve");
    let mut state = supervision_state(&fixture);
    let snapshot =
        MutationSnapshot::capture(&fixture.repo, &state.identity.branch).expect("snapshot");

    let outcome = supervise_harness(
        &fixture.root.join("state/invocation.json"),
        &fixture.root.join("log/executor.jsonl"),
        &mut state,
        &shell_invocation(&fixture.repo, "exit 0"),
        &snapshot,
        supervision_config(500),
    )
    .expect("clean child supervision");
    let mut observed = 0_i32;
    // SECURITY-REVIEW: independent #2598 reviewer LGTM; read-only process-state probe.
    // SAFETY: PR_GET_CHILD_SUBREAPER writes one integer to the supplied valid pointer.
    let get_result = unsafe {
        nix::libc::prctl(
            nix::libc::PR_GET_CHILD_SUBREAPER,
            std::ptr::addr_of_mut!(observed),
            0,
            0,
            0,
        )
    };
    nix::sys::prctl::set_child_subreaper(false).expect("clean enabled subreaper fixture");

    assert_eq!(outcome, SupervisionOutcome::Exited { exit_code: 0 });
    assert_eq!(get_result, 0, "read child-subreaper state");
    assert_eq!(
        observed, 1,
        "fully-cleaned supervision did not preserve the prior enabled subreaper state"
    );
}

#[test]
fn autonomous_executor_bridge_stops_completion_drain_after_ttl_one_claim_takeover() {
    let _environment = test_environment();
    // Break caught: dense completion output crossing TTL after the exact claim was replaced.
    let fixture = GitFixture::new("supervise-claim-takeover");
    let mut state = supervision_state(&fixture);
    let snapshot =
        MutationSnapshot::capture(&fixture.repo, &state.identity.branch).expect("snapshot");
    let state_path = fixture.root.join("state/invocation.json");
    let event_log = fixture.root.join("log/executor.jsonl");
    let bin = fixture.root.join("bin");
    let comments = fixture.root.join("comments.json");
    let posts = fixture.root.join("posts");
    let gh_log = fixture.root.join("gh.log");
    fs::create_dir(&bin).expect("claim fixture bin");
    fs::write(&posts, "0\n").expect("claim post counter");
    let claimed = autospec_core::claim::RunStateRecord::new(
        "owner/repo",
        42,
        "worker-1",
        "claimed",
        "feat/autonomous-issue-42",
        "",
        "claimed",
        Vec::new(),
        "2026-07-14T00:00:00Z",
        "2026-07-14T00:00:00Z",
        1,
    )
    .with_claim_id("claim-42");
    assert!(
        crate::commands::claim::advance_claim_ref_for_test(&fixture.repo, &claimed)
            .expect("seed authoritative claim ref")
    );
    fs::write(
        &comments,
        serde_json::json!([{
            "id": 100,
            "updated_at": "2026-07-14T00:00:00Z",
            "body": claimed.to_marked_comment()
        }])
        .to_string(),
    )
    .expect("claim fixture");
    write_executable(
        &bin.join("gh"),
        r#"#!/bin/sh
set -eu
printf 'CALL\n' >> "$AUTOSPEC_BRIDGE_GH_LOG"
printf '%s\n' "$@" >> "$AUTOSPEC_BRIDGE_GH_LOG"
if [ "$1" = api ] && [ "$2" = repos/owner/repo/issues/42/comments ]; then
  cat "$AUTOSPEC_BRIDGE_COMMENTS"
  exit 0
fi
if [ "$1" = issue ] && [ "$2" = comment ]; then
  body=''
  shift 2
  while [ "$#" -gt 0 ]; do
case "$1" in
  --body) body="$2"; shift 2 ;;
  *) shift ;;
esac
  done
  count=$(cat "$AUTOSPEC_BRIDGE_POSTS")
  count=$((count + 1))
  printf '%s\n' "$count" > "$AUTOSPEC_BRIDGE_POSTS"
  if [ "$count" -eq 1 ]; then
jq --arg body "$body" \
  '. + [{id:101,updated_at:"2030-01-01T00:00:00Z",body:$body}]' \
  "$AUTOSPEC_BRIDGE_COMMENTS" > "$AUTOSPEC_BRIDGE_COMMENTS.tmp"
  else
jq --arg takeover "$AUTOSPEC_BRIDGE_TAKEOVER" --arg body "$body" \
  '. + [{id:102,updated_at:"2030-01-01T00:00:01Z",body:$takeover},{id:103,updated_at:"2030-01-01T00:00:02Z",body:$body}]' \
  "$AUTOSPEC_BRIDGE_COMMENTS" > "$AUTOSPEC_BRIDGE_COMMENTS.tmp"
  fi
  mv "$AUTOSPEC_BRIDGE_COMMENTS.tmp" "$AUTOSPEC_BRIDGE_COMMENTS"
  exit 0
fi
exit 19
"#,
    );
    let takeover_record = autospec_core::claim::RunStateRecord::new(
        "owner/repo",
        42,
        "worker-2",
        "claimed",
        "feat/takeover",
        "",
        "claimed",
        Vec::new(),
        "2030-01-01T00:00:01Z",
        "2030-01-01T00:00:01Z",
        1,
    )
    .with_claim_id("claim-takeover");
    let takeover_link = [
        "<!-- autospec-",
        "run-state-link parent=101 generation=takeover-generation -->",
    ]
    .concat();
    let takeover = format!("{takeover_link}\n{}", takeover_record.to_marked_comment());
    let original_path = std::env::var_os("PATH");
    let original_lease = std::env::var_os("AUTOSPEC_CLAIM_LEASE_SECONDS");
    let original_claim_remote = std::env::var_os("AUTOSPEC_CLAIM_GIT_REMOTE");
    let original_claim_state = std::env::var_os("AUTOSPEC_CLAIM_GIT_STATE_DIR");
    std::env::set_var(
        "PATH",
        format!(
            "{}:{}",
            bin.display(),
            original_path
                .as_deref()
                .unwrap_or_default()
                .to_string_lossy()
        ),
    );
    std::env::set_var("AUTOSPEC_BRIDGE_GH_LOG", &gh_log);
    std::env::set_var("AUTOSPEC_BRIDGE_COMMENTS", &comments);
    std::env::set_var("AUTOSPEC_BRIDGE_POSTS", &posts);
    std::env::set_var("AUTOSPEC_BRIDGE_TAKEOVER", &takeover);
    std::env::set_var("AUTOSPEC_CLAIM_RETRY_SLEEP_MS", "0");
    std::env::set_var("AUTOSPEC_CLAIM_LEASE_SECONDS", "999999");
    std::env::set_var("AUTOSPEC_CLAIM_GIT_REMOTE", fixture.root.join("remote.git"));
    std::env::set_var(
        "AUTOSPEC_CLAIM_GIT_STATE_DIR",
        fixture.root.join("claim-state"),
    );
    std::env::set_var("AUTOSPEC_TEST_COMPLETION_DRAIN_DELAY_MS", "1100");
    let drain_marker = fixture.root.join("completion-drain.entered");
    std::env::set_var("AUTOSPEC_TEST_COMPLETION_DRAIN_MARKER", &drain_marker);

    let takeover_repo = fixture.repo.clone();
    let takeover_for_thread = takeover_record.clone();
    let takeover_thread = std::thread::spawn(move || {
        for _ in 0..500 {
            if drain_marker.exists() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(drain_marker.exists(), "completion drain did not start");
        crate::commands::claim::advance_claim_ref_for_test(&takeover_repo, &takeover_for_thread)
            .expect("publish claim takeover")
    });
    let outcome = bridge::supervise_harness_with_claim_renewal(
        &state_path,
        &event_log,
        &mut state,
        &shell_invocation(
            &fixture.repo,
            "yes x | head -c 32768; printf 'completion-marker\\n'",
        ),
        &snapshot,
        supervision_config(2_000),
        Duration::from_millis(20),
    );
    assert!(takeover_thread.join().expect("takeover publisher"));

    match original_path {
        Some(path) => std::env::set_var("PATH", path),
        None => std::env::remove_var("PATH"),
    }
    match original_lease {
        Some(value) => std::env::set_var("AUTOSPEC_CLAIM_LEASE_SECONDS", value),
        None => std::env::remove_var("AUTOSPEC_CLAIM_LEASE_SECONDS"),
    }
    match original_claim_remote {
        Some(value) => std::env::set_var("AUTOSPEC_CLAIM_GIT_REMOTE", value),
        None => std::env::remove_var("AUTOSPEC_CLAIM_GIT_REMOTE"),
    }
    match original_claim_state {
        Some(value) => std::env::set_var("AUTOSPEC_CLAIM_GIT_STATE_DIR", value),
        None => std::env::remove_var("AUTOSPEC_CLAIM_GIT_STATE_DIR"),
    }
    for name in [
        "AUTOSPEC_BRIDGE_GH_LOG",
        "AUTOSPEC_BRIDGE_COMMENTS",
        "AUTOSPEC_BRIDGE_POSTS",
        "AUTOSPEC_BRIDGE_TAKEOVER",
        "AUTOSPEC_CLAIM_RETRY_SLEEP_MS",
        "AUTOSPEC_TEST_COMPLETION_DRAIN_DELAY_MS",
        "AUTOSPEC_TEST_COMPLETION_DRAIN_MARKER",
    ] {
        std::env::remove_var(name);
    }

    assert_eq!(
        outcome.expect("claim loss is a supervised outcome"),
        SupervisionOutcome::OwnershipLost
    );
    assert_eq!(state.phase, BridgePhase::Interrupted);
    assert!(state.supervisor.is_none());
    assert!(state.process.is_none());
    let events = fs::read_to_string(event_log).expect("claim loss event");
    assert!(events.contains("\"event\":\"claim_ownership_lost\""));
    let calls = fs::read_to_string(gh_log).expect("claim gh calls");
    assert!(
        calls.matches("issue\ncomment\n42").count() >= 1,
        "authoritative ttl=1 must renew before env ttl=999999 expires: {calls}"
    );
    assert!(
        !calls.contains("\nPATCH\n"),
        "run-state is append-only: {calls}"
    );
}
