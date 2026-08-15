// executor_bridge tests: base / merge — 9 cases.
//
// Split out of tests.rs; see the note in that file.

use super::support_base::{git, git_stdout, test_environment};
use super::support_invocation::{commit_implementation, implementation_proof_fixture};
use super::support_launch::DRAFT_ISSUE_BODY;
use crate::commands::autonomous::executor_bridge as bridge;
use std::collections::BTreeMap;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::process::Command;
use std::sync::atomic::Ordering;

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
    state.phase = bridge::BridgePhase::ResultAccepted;
    state.pr = Some(17);
    state.head_oid = Some(head.clone());
    let closeout = "## Closeout report\n";
    state.closeout_digest = Some(autospec_core::autonomous::waterfall::sha256_hex(
        closeout.as_bytes(),
    ));
    let body =
        serde_json::to_string(&bridge::canonical_pull_request_body(&state, closeout).unwrap())
            .unwrap();
    let state_path = fixture.root.join("state/invocation.json");
    bridge::write_invocation_atomic(&state_path, &state).expect("accepted state");
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
    let adapter = bridge::DraftPrAdapter {
        gh,
        environment: BTreeMap::from([("GH_CALLS".into(), calls.clone().into_os_string())]),
    };

    let error = bridge::admin_squash_merge_exact_with_refresh_and_admission(
        &state_path,
        &mut state,
        &adapter,
        || Ok(bridge::BridgeClaimOwnership::Lost),
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
    state.phase = bridge::BridgePhase::ResultAccepted;
    state.pr = Some(17);
    state.head_oid = Some(head.clone());
    let closeout = "## Closeout report\n";
    state.closeout_digest = Some(autospec_core::autonomous::waterfall::sha256_hex(
        closeout.as_bytes(),
    ));
    let body =
        serde_json::to_string(&bridge::canonical_pull_request_body(&state, closeout).unwrap())
            .unwrap();
    let state_path = fixture.root.join("state/invocation.json");
    bridge::write_invocation_atomic(&state_path, &state).expect("accepted state");
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
    let adapter = bridge::DraftPrAdapter {
        gh,
        environment: BTreeMap::from([
            ("FAIL_ONCE".into(), fail_once.into_os_string()),
            ("MERGED".into(), merged.into_os_string()),
        ]),
    };

    bridge::admin_squash_merge_exact_with_refresh_and_admission(
        &state_path,
        &mut state,
        &adapter,
        || Ok(bridge::BridgeClaimOwnership::Refreshed { ttl_seconds: 60 }),
        || Ok(()),
    )
    .expect_err("first merge fails");
    assert_eq!(state.phase, bridge::BridgePhase::MergeRequested);
    let merge_oid = bridge::admin_squash_merge_exact_with_refresh_and_admission(
        &state_path,
        &mut state,
        &adapter,
        || Ok(bridge::BridgeClaimOwnership::Refreshed { ttl_seconds: 60 }),
        || Ok(()),
    )
    .expect("retry observes merge");
    assert_eq!(merge_oid, "b".repeat(40));
    assert_eq!(state.phase, bridge::BridgePhase::Merged);
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
    state.phase = bridge::BridgePhase::MergeRequested;
    state.pr = Some(17);
    state.head_oid = Some(head.clone());
    let closeout = "## Closeout report\n";
    state.closeout_digest = Some(autospec_core::autonomous::waterfall::sha256_hex(
        closeout.as_bytes(),
    ));
    let body =
        serde_json::to_string(&bridge::canonical_pull_request_body(&state, closeout).unwrap())
            .unwrap();
    let state_path = fixture.root.join("state/invocation.json");
    bridge::write_invocation_atomic(&state_path, &state).expect("requested state");
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
    let adapter = bridge::DraftPrAdapter {
        gh,
        environment: BTreeMap::new(),
    };

    assert_eq!(
        bridge::admin_squash_merge_exact_with_refresh(&state_path, &mut state, &adapter, || Ok(
            bridge::BridgeClaimOwnership::Refreshed { ttl_seconds: 60 }
        ))
        .expect("merged observation must win over deleted refs"),
        "b".repeat(40)
    );
    assert_eq!(state.phase, bridge::BridgePhase::Merged);
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
    state.phase = bridge::BridgePhase::ReviewPassed;
    state.pr = Some(17);
    state.head_oid = Some(original_head.clone());
    let state_path = fixture.root.join("state/invocation.json");
    bridge::write_invocation_atomic(&state_path, &state).expect("reviewed state");
    let original = bridge::evaluate_patch_size_admission(&state, &original_head, DRAFT_ISSUE_BODY)
        .expect("original admission");
    bridge::persist_patch_size_admission(&state_path, &original).expect("original receipt");

    fs::write(fixture.root.join("seed/base-drift.txt"), "drift\n").expect("base drift");
    git(&fixture.root.join("seed"), &["add", "base-drift.txt"]);
    git(&fixture.root.join("seed"), &["commit", "-m", "base drift"]);
    git(&fixture.root.join("seed"), &["push", "origin", "main"]);

    assert!(
        bridge::reconcile_base_drift_with_refresh(&state_path, &mut state, || {
            Ok(bridge::BridgeClaimOwnership::Refreshed { ttl_seconds: 60 })
        })
        .expect("reconcile"),
        "drift must regenerate the lane"
    );
    assert_eq!(state.phase, bridge::BridgePhase::DraftCreated);
    assert_ne!(state.identity.base_oid, original_head);
    assert_ne!(state.head_oid.as_deref(), Some(original_head.as_str()));
    let admission = bridge::validate_patch_size_admission(&state_path, &state)
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
    let _environment = test_environment();
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
    state.phase = bridge::BridgePhase::DraftCreated;
    state.pr = Some(17);
    state.head_oid = Some(original_head.clone());
    let state_path = fixture.root.join("state/invocation.json");
    bridge::write_invocation_atomic(&state_path, &state).expect("draft state");
    let admission = bridge::evaluate_patch_size_admission(&state, &original_head, DRAFT_ISSUE_BODY)
        .expect("original admission");
    bridge::persist_patch_size_admission(&state_path, &admission)
        .expect("original admission receipt");

    fs::write(fixture.root.join("seed/premerge-fix.txt"), "base fix\n").expect("base drift");
    git(&fixture.root.join("seed"), &["add", "premerge-fix.txt"]);
    git(
        &fixture.root.join("seed"),
        &["commit", "-m", "premerge fix"],
    );
    git(&fixture.root.join("seed"), &["push", "origin", "main"]);
    let updated_base = git_stdout(&fixture.root.join("seed"), &["rev-parse", "HEAD"]);
    let request = bridge::ExecutorBridgeRequest {
        repository: state.identity.repository.clone(),
        repository_path: state.identity.repository_path.clone(),
        issue: state.identity.issue,
        issue_title: "Reconcile stale implementation".to_string(),
        issue_body: DRAFT_ISSUE_BODY.to_string(),
        serialization_reasons: Vec::new(),
        worker_id: state.identity.worker_id.clone(),
        claim_id: state.identity.claim_id.clone(),
        invocation_id: state.identity.invocation_id.clone(),
        state_path: state_path.clone(),
        event_log: fixture.root.join("events.jsonl"),
    };
    let proof = bridge::ImplementationProof {
        head_oid: original_head.clone(),
        closeout_body: String::new(),
    };

    let result = bridge::ensure_premerge_and_review(
        &request,
        &BTreeMap::new(),
        &bridge::DraftPrAdapter::github_cli(),
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
    assert_eq!(state.phase, bridge::BridgePhase::DraftCreated);
    assert_eq!(state.identity.base_oid, updated_base);
    assert_ne!(state.head_oid.as_deref(), Some(original_head.as_str()));
    assert!(
        !state.identity.worktree.join(".autospec/evidence").exists(),
        "premerge evidence started before the stale branch was updated"
    );
}

#[test]
fn autonomous_executor_bridge_base_drift_invalidates_accepted_and_requested_results() {
    let _environment = test_environment();
    for phase in [
        bridge::BridgePhase::ResultAccepted,
        bridge::BridgePhase::MergeRequested,
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
        bridge::write_invocation_atomic(&state_path, &state).expect("accepted state");
        fs::write(
            fixture.root.join("seed/late-drift.txt"),
            format!("{phase:?}\n"),
        )
        .expect("base drift");
        git(&fixture.root.join("seed"), &["add", "late-drift.txt"]);
        git(&fixture.root.join("seed"), &["commit", "-m", "late drift"]);
        git(&fixture.root.join("seed"), &["push", "origin", "main"]);

        assert!(
            bridge::reconcile_base_drift_with_refresh(&state_path, &mut state, || Ok(
                bridge::BridgeClaimOwnership::Refreshed { ttl_seconds: 60 }
            ))
            .expect("late drift must regenerate")
        );
        assert_eq!(state.phase, bridge::BridgePhase::DraftCreated);
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
    state.phase = bridge::BridgePhase::ReviewPassed;
    state.pr = Some(17);
    state.head_oid = Some(original_head.clone());
    let state_path = fixture.root.join("state/invocation.json");
    bridge::write_invocation_atomic(&state_path, &state).expect("reviewed state");
    let durable_before = fs::read(&state_path).expect("durable state");
    let local_head_before = git_stdout(
        &state.identity.worktree,
        &["rev-parse", "--verify", "HEAD^{commit}"],
    );
    fs::write(fixture.root.join("seed/takeover.txt"), "drift\n").expect("base drift");
    git(&fixture.root.join("seed"), &["add", "takeover.txt"]);
    git(&fixture.root.join("seed"), &["commit", "-m", "base drift"]);
    git(&fixture.root.join("seed"), &["push", "origin", "main"]);

    let error = bridge::reconcile_base_drift_with_refresh(&state_path, &mut state, || {
        Ok(bridge::BridgeClaimOwnership::Lost)
    })
    .expect_err("takeover blocks push");
    assert!(error.contains("ownership"), "{error}");
    let issue_ref = format!("refs/heads/{}", state.identity.branch);
    assert_eq!(
        bridge::remote_head_refs(&state.identity.repository_path)
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
    let _environment = test_environment();
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
    state.phase = bridge::BridgePhase::ReviewPassed;
    state.pr = Some(17);
    state.head_oid = Some(original_head);
    let state_path = fixture.root.join("state/invocation.json");
    bridge::write_invocation_atomic(&state_path, &state).expect("reviewed state");
    fs::write(fixture.root.join("seed/crash.txt"), "drift\n").expect("base drift");
    git(&fixture.root.join("seed"), &["add", "crash.txt"]);
    git(&fixture.root.join("seed"), &["commit", "-m", "base drift"]);
    git(&fixture.root.join("seed"), &["push", "origin", "main"]);

    bridge::BASE_DRIFT_FAILPOINT.store(1, Ordering::SeqCst);
    let error = bridge::reconcile_base_drift_with_refresh(&state_path, &mut state, || {
        Ok(bridge::BridgeClaimOwnership::Refreshed { ttl_seconds: 60 })
    })
    .expect_err("injected crash");
    assert!(error.contains("injected crash"), "{error}");
    let durable = bridge::PersistedInvocation::from_json(
        &fs::read_to_string(&state_path).expect("durable invocation"),
    )
    .expect("durable pre-merge binding remains");
    assert_eq!(durable.phase, bridge::BridgePhase::ReviewPassed);

    assert!(
        bridge::reconcile_base_drift_with_refresh(&state_path, &mut state, || {
            Ok(bridge::BridgeClaimOwnership::Refreshed { ttl_seconds: 60 })
        })
        .expect("adopt exact merge")
    );
    assert_eq!(state.phase, bridge::BridgePhase::DraftCreated);
}

#[test]
fn autonomous_executor_bridge_cleanup_resumes_after_owned_worktree_removal() {
    let (fixture, mut state, _snapshot, _) = implementation_proof_fixture("cleanup-resume");
    commit_implementation(&state);
    state.phase = bridge::BridgePhase::CleanupPending;
    state.identity.runtime_environment_dir = None;
    state.identity.runtime_session_id = None;
    let state_path = fixture.root.join("state/invocation.json");
    bridge::write_invocation_atomic(&state_path, &state).expect("cleanup state");
    let intent = bridge::cleanup_record_path(&state_path, "worktree-intent");
    bridge::ensure_cleanup_record(
        &intent,
        &bridge::cleanup_binding(&state),
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

    bridge::finalize_merged_executor(&state_path, &mut state, None).expect("resume cleanup");
    assert_eq!(state.phase, bridge::BridgePhase::Complete);
    assert!(bridge::cleanup_record_path(&state_path, "worktree-complete").exists());
}
