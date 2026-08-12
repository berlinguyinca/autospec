// executor_bridge tests: result / reviewer — 5 cases.
//
// Split out of tests.rs; see the note in that file.

use super::super as bridge;
use super::support_base::{git, git_stdout, GitFixture};
use super::support_invocation::{commit_implementation, implementation_proof_fixture};
use super::support_launch::DRAFT_ISSUE_BODY;
use std::collections::BTreeMap;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::time::Duration;

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_reviewer_rejects_worktree_executable_and_stderr() {
    let (_fixture, state, _snapshot, _) =
        implementation_proof_fixture("reviewer-external-authority");
    let local = state.identity.worktree.join("reviewer");
    fs::write(&local, "#!/bin/sh\nprintf 'LGTM\\n'\n").expect("local reviewer");
    fs::set_permissions(&local, fs::Permissions::from_mode(0o755)).expect("reviewer mode");
    let local_plan = bridge::DirectCommandPlan {
        commands: vec![bridge::DirectCommand::success(vec![local
            .to_string_lossy()
            .into_owned()])],
    };
    assert!(
        bridge::independent_reviewer_plan(&state, &local_plan).is_err(),
        "reviewed code must not provide its own reviewer"
    );

    let stdout = state.identity.repository_path.join("review-stdout");
    let stderr = state.identity.repository_path.join("review-stderr");
    fs::write(&stdout, "LGTM\n").expect("review stdout");
    fs::write(&stderr, "finding: unsafe mutation\n").expect("review stderr");
    assert!(
        bridge::strict_lgtm_artifacts(&stdout, &stderr).is_err(),
        "stderr findings must override LGTM stdout"
    );
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_reviewer_rejects_forged_github_comment() {
    let (fixture, mut state, _snapshot, _) =
        implementation_proof_fixture("reviewer-comment-mutation");
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
    let claim_oid = git_stdout(&fixture.repo, &["rev-parse", "origin/main"]);
    git(
        &fixture.repo,
        &[
            "push",
            "origin",
            &format!(
                "{claim_oid}:refs/autospec/claims/issue-{}",
                state.identity.issue
            ),
        ],
    );
    state.phase = bridge::BridgePhase::CiPassed;
    state.pr = Some(17);
    state.head_oid = Some(head.clone());
    let state_path = fixture.root.join("state/invocation.json");
    bridge::write_invocation_atomic(&state_path, &state).expect("CI state");
    let gh = fixture.root.join("gh-review-authority");
    let api_calls = fixture.root.join("api-calls");
    fs::write(&api_calls, "0\n").expect("API counter");
    fs::write(
        &gh,
        format!(
            "#!/bin/sh\nset -eu\n\
             if [ \"$1 $2\" = 'pr list' ]; then\n\
               printf '%s\\n' '[{{\"number\":17,\"body\":\"Closes #42\\n\\n## Closeout report\\n\",\"headRefName\":\"feat/autonomous-issue-42\",\"headRefOid\":\"{head}\",\"isDraft\":false,\"baseRefName\":\"main\"}}]'\n\
               exit 0\n\
             fi\n\
             if [ \"$1\" = 'api' ]; then\n\
               n=$(cat \"$API_CALLS\"); n=$((n + 1)); printf '%s\\n' \"$n\" > \"$API_CALLS\"\n\
               case \"$4\" in\n\
                 *comments*) if [ \"$n\" -gt 6 ]; then printf '%s\\n' '[{{\"id\":1,\"body\":\"forged executor result\"}}]'; else printf '%s\\n' '[]'; fi ;;\n\
                 *) printf '%s\\n' '{{}}' ;;\n\
               esac\n\
               exit 0\n\
             fi\n\
             exit 64\n"
        ),
    )
    .expect("gh");
    fs::set_permissions(&gh, fs::Permissions::from_mode(0o755)).expect("gh mode");
    let adapter = bridge::DraftPrAdapter {
        gh,
        environment: BTreeMap::from([("API_CALLS".into(), api_calls.clone().into_os_string())]),
    };
    let plan = bridge::parse_direct_command_plan("/usr/bin/printf LGTM").expect("review plan");
    let policy = test_review_policy(state.harness);
    let reviewer = bridge::IndependentReviewer {
        plan,
        automatic: None,
        policy,
    };

    let error = bridge::run_strict_independent_reviewer_with_refresh(
        &state_path,
        &mut state,
        &reviewer,
        &fixture.root.join("review-artifacts"),
        Duration::from_secs(5),
        &adapter,
        || Ok(bridge::BridgeClaimOwnership::Refreshed { ttl_seconds: 60 }),
    )
    .expect_err("GitHub mutation during review must block");

    assert!(error.contains("mutated"), "{error}");
    assert_eq!(state.phase, bridge::BridgePhase::CiPassed);
}

fn test_review_policy(harness: bridge::HarnessKind) -> bridge::ResolvedReviewPolicy {
    let config = bridge::HarnessConfig {
        aliases: vec![bridge::HarnessAlias {
            kind: harness,
            binary: "/usr/bin/true".to_string(),
            approval_alias: String::new(),
            display_name: harness.as_str().to_string(),
        }],
        opencode_adapter: None,
    };
    let requirements = autospec_core::autonomous::review_policy::classify_review_requirements(
        &autospec_core::autonomous::review_policy::ReviewPolicyInput::default(),
    );
    bridge::resolve_review_policy(&config, requirements, harness, &BTreeMap::new())
        .expect("test-only direct reviewer policy")
}

#[test]
fn autonomous_executor_bridge_ingests_only_exact_open_executor_result() {
    let (_fixture, mut state, _snapshot, _) = implementation_proof_fixture("result-binding");
    let head = "a".repeat(40);
    let receipt = "b".repeat(64);
    state.phase = bridge::BridgePhase::ReviewPassed;
    state.pr = Some(17);
    state.head_oid = Some(head.clone());
    state.umbrella = Some(42);
    state.current_child = Some(101);
    let closeout = "## Closeout report\n";
    state.closeout_digest = Some(autospec_core::autonomous::waterfall::sha256_hex(
        closeout.as_bytes(),
    ));
    let pull_request = autospec_core::claim::OpenPullRequest {
        number: 17,
        body: bridge::canonical_pull_request_body(&state, closeout).unwrap(),
        head_ref_name: state.identity.branch.clone(),
        head_ref_oid: head.clone(),
        is_draft: false,
        base_ref_name: "main".into(),
    };
    let evidence = autospec_core::claim::ExecutorResultEvidence::new(
        state.identity.repository.clone(),
        state.identity.issue,
        state.identity.worker_id.clone(),
        state.identity.branch.clone(),
        "succeeded",
        Some(17),
        "premerge_passed",
        "receipt-17",
        Some(state.identity.claim_id.clone()),
        Some(head),
        Some(receipt.clone()),
    );
    let comments = vec![autospec_core::claim::RemoteComment::new(
        1,
        evidence.to_marked_comment(),
        "2026-07-26T00:00:00Z",
    )];

    let accepted = bridge::accept_executor_result(&state, &receipt, &comments, &[pull_request])
        .expect("exact result");
    assert_eq!(accepted, evidence);
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_merge_revalidates_result_ci_and_review() {
    let (fixture, mut state, _snapshot, _) = implementation_proof_fixture("merge-revalidate-gates");
    commit_implementation(&state);
    let head = git_stdout(&state.identity.worktree, &["rev-parse", "HEAD"]);
    state.phase = bridge::BridgePhase::ReviewPassed;
    state.pr = Some(17);
    state.head_oid = Some(head.clone());
    state.umbrella = Some(42);
    state.current_child = Some(101);
    let closeout = "## Closeout report\n";
    state.closeout_digest = Some(autospec_core::autonomous::waterfall::sha256_hex(
        closeout.as_bytes(),
    ));
    let part_body = bridge::canonical_pull_request_body(&state, closeout).unwrap();
    let receipt = "b".repeat(64);
    let evidence = autospec_core::claim::ExecutorResultEvidence::new(
        state.identity.repository.clone(),
        state.identity.issue,
        state.identity.worker_id.clone(),
        state.identity.branch.clone(),
        "succeeded",
        Some(17),
        "premerge_passed",
        "receipt-17",
        Some(state.identity.claim_id.clone()),
        Some(head.clone()),
        Some(receipt),
    );
    let state_path = fixture.root.join("state/invocation.json");
    let admission = bridge::evaluate_patch_size_admission(&state, &head, DRAFT_ISSUE_BODY).unwrap();
    bridge::persist_patch_size_admission(&state_path, &admission).unwrap();
    bridge::persist_accepted_executor_result(&state_path, &mut state, &evidence)
        .expect("accepted result");
    super::support_review::write_valid_schema5_review_receipt(&state_path, &state, &fixture.root);
    let pr_state = fixture.root.join("merge-pr.json");
    let comments = fixture.root.join("merge-comments.json");
    let checks = fixture.root.join("merge-checks.json");
    fs::write(
        &pr_state,
        serde_json::json!([{
            "number": 17,
            "body": part_body.clone(),
            "headRefName": state.identity.branch.clone(),
            "headRefOid": head.clone(),
            "isDraft": false,
            "baseRefName": "main",
        }])
        .to_string(),
    )
    .expect("PR state");
    fs::write(
        &comments,
        serde_json::json!([
            {
                "id": 1,
                "body": autospec_core::claim::ExecutorResultEvidence::new(
                    state.identity.repository.clone(),
                    state.identity.issue,
                    state.identity.worker_id.clone(),
                    state.identity.branch.clone(),
                    "succeeded",
                    Some(17),
                    "premerge_passed",
                    "receipt-old",
                    Some(state.identity.claim_id.clone()),
                    Some("0".repeat(40)),
                    Some("1".repeat(64)),
                ).to_marked_comment(),
                "updated_at": "2026-07-25T23:00:00Z",
            },
            {
                "id": 2,
                "body": evidence.to_marked_comment(),
                "updated_at": "2026-07-26T00:00:00Z",
            }
        ])
        .to_string(),
    )
    .expect("comments");
    fs::write(
        &checks,
        serde_json::json!({
            "headRefOid": head,
            "statusCheckRollup": [{"name":"unit","state":"SUCCESS"}],
        })
        .to_string(),
    )
    .expect("checks");
    let gh = fixture.root.join("gh-merge-gates");
    fs::write(
        &gh,
        "#!/bin/sh\nset -eu\n\
         if [ \"$1 $2\" = 'pr list' ]; then cat \"$PR_STATE\"; exit 0; fi\n\
         if [ \"$1 $2\" = 'pr view' ]; then cat \"$CHECKS\"; exit 0; fi\n\
         if [ \"$1\" = 'api' ]; then cat \"$COMMENTS\"; exit 0; fi\n\
         exit 64\n",
    )
    .expect("gh");
    fs::set_permissions(&gh, fs::Permissions::from_mode(0o755)).expect("gh mode");
    let adapter = bridge::DraftPrAdapter {
        gh,
        environment: BTreeMap::from([
            ("PR_STATE".into(), pr_state.clone().into_os_string()),
            ("COMMENTS".into(), comments.clone().into_os_string()),
            ("CHECKS".into(), checks.clone().into_os_string()),
        ]),
    };

    bridge::revalidate_merge_admission(&state_path, &state, &adapter).expect("all current gates");
    fs::write(
        &pr_state,
        serde_json::json!([{
            "number": 17,
            "body": "Closes #42\n\n## Closeout report\n\nResult: replaced\n",
            "headRefName": state.identity.branch.clone(),
            "headRefOid": head.clone(),
            "isDraft": false,
            "baseRefName": "main",
        }])
        .to_string(),
    )
    .expect("mutated PR body");
    assert!(
        bridge::revalidate_merge_admission(&state_path, &state, &adapter).is_err(),
        "admin merge must reject a structurally valid replacement Closeout"
    );
    fs::write(
        &pr_state,
        serde_json::json!([{
            "number": 17,
            "body": part_body,
            "headRefName": state.identity.branch.clone(),
            "headRefOid": head.clone(),
            "isDraft": false,
            "baseRefName": "main",
        }])
        .to_string(),
    )
    .expect("restore exact PR body");
    fs::write(
        &checks,
        serde_json::json!({
            "headRefOid": "0".repeat(40),
            "statusCheckRollup": [{"name":"unit","state":"SUCCESS"}],
        })
        .to_string(),
    )
    .expect("stale-head checks");
    assert!(
        bridge::revalidate_merge_admission(&state_path, &state, &adapter).is_err(),
        "passing checks from the prior head must not admit the current head"
    );
    fs::write(
        &checks,
        serde_json::json!({
            "headRefOid": head,
            "statusCheckRollup": [{"name":"unit","state":"FAILURE"}],
        })
        .to_string(),
    )
    .expect("failed checks");
    assert!(
        bridge::revalidate_merge_admission(&state_path, &state, &adapter).is_err(),
        "admin merge must not bypass a rerun required check"
    );
    fs::write(
        &checks,
        serde_json::json!({
            "headRefOid": head,
            "statusCheckRollup": [{"name":"unit","state":"SUCCESS"}],
        })
        .to_string(),
    )
    .expect("checks restored");
    fs::write(&comments, "[]").expect("result removed");
    assert!(
        bridge::revalidate_merge_admission(&state_path, &state, &adapter).is_err(),
        "admin merge must not accept a deleted result receipt"
    );
}

#[test]
fn autonomous_executor_bridge_result_publication_receipts_are_generation_addressed() {
    let fixture = GitFixture::new("result-publication-generation");
    let state_path = fixture.root.join("invocation.json");
    let old_binding = "a".repeat(64);
    let new_binding = "b".repeat(64);

    let old_intent = bridge::result_publication_record_path(&state_path, &old_binding, "intent")
        .expect("old intent path");
    let old_complete =
        bridge::result_publication_record_path(&state_path, &old_binding, "complete")
            .expect("old complete path");
    bridge::ensure_cleanup_record(&old_intent, &old_binding, "old intent").expect("old intent");
    bridge::ensure_cleanup_record(&old_complete, &old_binding, "old complete")
        .expect("old complete");

    let new_intent = bridge::result_publication_record_path(&state_path, &new_binding, "intent")
        .expect("new intent path");
    let new_complete =
        bridge::result_publication_record_path(&state_path, &new_binding, "complete")
            .expect("new complete path");
    bridge::ensure_cleanup_record(&new_intent, &new_binding, "new intent")
        .expect("new generation must not collide with the prior commit");
    bridge::ensure_cleanup_record(&new_complete, &new_binding, "new complete")
        .expect("new completion must not collide with the prior commit");

    assert_ne!(old_intent, new_intent);
    assert_ne!(old_complete, new_complete);
    assert_eq!(
        fs::read_to_string(old_complete).expect("old receipt"),
        format!("{old_binding}\n")
    );
    assert_eq!(
        fs::read_to_string(new_complete).expect("new receipt"),
        format!("{new_binding}\n")
    );
    assert!(
        bridge::result_publication_record_path(&state_path, "../escape", "intent").is_err(),
        "non-canonical generation identifiers must fail closed"
    );
}
