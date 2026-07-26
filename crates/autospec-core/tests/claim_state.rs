use autospec_core::claim::{
    evaluate_merge_ready_claim_recovery, find_reconcilable_pull_request,
    parse_open_pull_requests_json, parse_remote_comments_json, select_run_state,
    terminal_merged_comment_exists, ClaimRecoveryBlock, ClaimRecoveryDecision,
    ExecutorResultEvidence, OpenPullRequest, RemoteComment, RequiredCheck, RunStateRecord,
};

const BEGIN: &str = "<!-- autospec-run-state:begin -->";
const END: &str = "<!-- autospec-run-state:end -->";

fn marked(body: &str) -> String {
    format!("{BEGIN}\n{body}\n{END}")
}

fn record(worker_id: &str) -> String {
    format!(
        r#"{{"schema":1,"repo":"testorg/testrepo","issue":42,"worker_id":"{worker_id}","state":"claimed","branch":"feat/test","pr":"","step":"claimed","paths":["crates/autospec-core/src/claim/mod.rs"],"claimed_at":"2026-07-14T00:00:00Z","updated_at":"2026-07-14T00:00:00Z","ttl_seconds":10800}}"#
    )
}

fn recovery_claim() -> RunStateRecord {
    RunStateRecord::new(
        "testorg/testrepo",
        42,
        "worker-a",
        "claimed",
        "feat/test",
        "",
        "executor_succeeded",
        Vec::new(),
        "2026-07-14T00:00:00Z",
        "2026-07-14T00:00:00Z",
        10_800,
    )
    .with_claim_id("claim-a")
}

fn recovery_evidence() -> ExecutorResultEvidence {
    ExecutorResultEvidence::new(
        "testorg/testrepo",
        42,
        "worker-a",
        "feat/test",
        "succeeded",
        Some(75),
        "executor_succeeded",
        "result-75",
        Some("claim-a".to_string()),
        Some("7575757575757575757575757575757575757575".to_string()),
        Some("a".repeat(64)),
    )
}

fn recovery_pull_request() -> OpenPullRequest {
    OpenPullRequest {
        number: 75,
        body: "Closes #42\n\n## Closeout report\n\nshipped".to_string(),
        head_ref_name: "feat/test".to_string(),
        head_ref_oid: "7575757575757575757575757575757575757575".to_string(),
    }
}

#[test]
fn selects_the_lowest_marked_comment_id_not_api_order() {
    let comments = vec![
        RemoteComment::new(101, marked(&record("worker-b")), "2026-07-14T00:01:00Z"),
        RemoteComment::new(100, marked(&record("worker-a")), "2026-07-14T00:00:00Z"),
    ];

    let selected = select_run_state(&comments, "testorg/testrepo", 42)
        .expect("the lowest marked comment is selected");

    assert_eq!(selected.comment_id, 100);
    assert_eq!(selected.record.worker_id, "worker-a");
}

#[test]
fn ignores_a_lowest_marked_comment_that_is_not_bound_to_the_requested_issue() {
    let comments = vec![
        RemoteComment::new(
            100,
            marked(&record("worker-a").replace("\"issue\":42", "\"issue\":43")),
            "2026-07-14T00:00:00Z",
        ),
        RemoteComment::new(101, marked(&record("worker-b")), "2026-07-14T00:01:00Z"),
    ];

    assert!(select_run_state(&comments, "testorg/testrepo", 42).is_none());
}

#[test]
fn run_state_record_round_trips_the_schema_one_contract() {
    let record = RunStateRecord::new(
        "testorg/testrepo",
        42,
        "worker-a",
        "worktree_ready",
        "feat/test",
        "99",
        "worktree_ready",
        vec!["crates/autospec-core/src/claim/mod.rs".to_string()],
        "2026-07-14T00:00:00Z",
        "2026-07-14T00:01:00Z",
        7_200,
    );

    let parsed = RunStateRecord::parse_json(&record.to_json()).expect("record parses");

    assert_eq!(parsed, record);
}

#[test]
fn parses_the_minimal_historical_schema_one_claim_without_treating_it_as_unowned() {
    let parsed = RunStateRecord::parse_json(
        r#"{"schema":1,"repo":"testorg/testrepo","issue":42,"worker_id":"worker-a","state":"claimed","claimed_at":"2026-07-14T00:00:00Z"}"#,
    )
    .expect("historical schema-one claim parses");

    assert_eq!(parsed.worker_id, "worker-a");
    assert_eq!(parsed.state, "claimed");
    assert_eq!(parsed.branch, "");
    assert_eq!(parsed.step, "claimed");
    assert_eq!(parsed.paths, Vec::<String>::new());
    assert_eq!(parsed.updated_at, "2026-07-14T00:00:00Z");
    assert_eq!(parsed.ttl_seconds, 10_800);
}

#[test]
fn rejects_duplicate_and_unknown_schema_fields() {
    let duplicate = record("worker-a").replace(
        "\"ttl_seconds\":10800",
        "\"ttl_seconds\":10800,\"ttl_seconds\":10800",
    );
    assert!(RunStateRecord::parse_json(&duplicate).is_err());

    let unknown_schema = record("worker-a").replace("\"schema\":1", "\"schema\":2");
    assert!(RunStateRecord::parse_json(&unknown_schema).is_err());
}

#[test]
fn parses_the_projected_github_comment_shape_without_accepting_unknown_fields() {
    let comments = parse_remote_comments_json(
        r#"[{"id":100,"body":"state","updated_at":"2026-07-14T00:00:00Z"}]"#,
    )
    .expect("projected GitHub comments parse");
    assert_eq!(
        comments,
        vec![RemoteComment::new(100, "state", "2026-07-14T00:00:00Z")]
    );

    assert!(parse_remote_comments_json(r#"[{"id":100,"body":"state","extra":true}]"#).is_err());
}

#[test]
fn selects_the_lowest_linked_open_pr_with_exactly_one_closeout_report() {
    let pull_requests = parse_open_pull_requests_json(
        r#"[
          {"number":77,"body":"Fixes #42\n\n## Closeout report\n\n## Closeout report","headRefName":"feat/77","headRefOid":"7777777777777777777777777777777777777777"},
          {"number":75,"body":"Closes #42\n\n## Closeout report\n\nshipped","headRefName":"feat/75","headRefOid":"7575757575757575757575757575757575757575"},
          {"number":74,"body":"Fixes #420\n\n## Closeout report","headRefName":"feat/74","headRefOid":"7474747474747474747474747474747474747474"}
        ]"#,
    )
    .expect("projected pull request list parses");

    let selected = find_reconcilable_pull_request(&pull_requests, 42)
        .expect("the lower valid linked pull request is selected");
    assert_eq!(selected.number, 75);
}

#[test]
fn recognizes_only_a_valid_merged_terminal_record() {
    let comments = vec![
        RemoteComment::new(
            100,
            "<!-- autospec-run-terminal:begin -->\n{\"state\": \"merged\"}\n<!-- autospec-run-terminal:end -->",
            "2026-07-14T00:00:00Z",
        ),
        RemoteComment::new(
            101,
            "<!-- autospec-run-terminal:begin -->\n{\"state\": \"claimed\"}\n<!-- autospec-run-terminal:end -->",
            "2026-07-14T00:00:00Z",
        ),
    ];
    assert!(terminal_merged_comment_exists(&comments));

    let malformed = vec![RemoteComment::new(
        100,
        "<!-- autospec-run-terminal:begin -->\n{\"state\": \"merged\"\n<!-- autospec-run-terminal:end -->",
        "2026-07-14T00:00:00Z",
    )];
    assert!(!terminal_merged_comment_exists(&malformed));
}

#[test]
fn recovers_an_expired_merge_ready_claim_with_exact_identity() {
    let decision = evaluate_merge_ready_claim_recovery(
        &recovery_claim(),
        &recovery_evidence(),
        &recovery_pull_request(),
        &[RequiredCheck::new("CI", "SUCCESS")],
        false,
    );

    assert_eq!(
        decision,
        ClaimRecoveryDecision::Recover { pull_request: 75 }
    );
}

#[test]
fn blocks_recovery_when_any_claim_identity_field_differs() {
    let claim = recovery_claim().with_claim_id("claim-a");
    let pull_request = recovery_pull_request();
    let checks = [RequiredCheck::new("CI", "SUCCESS")];

    for evidence in [
        ExecutorResultEvidence {
            repo: "other/repo".to_string(),
            ..recovery_evidence()
        },
        ExecutorResultEvidence {
            issue: 43,
            ..recovery_evidence()
        },
        ExecutorResultEvidence {
            worker_id: "worker-b".to_string(),
            ..recovery_evidence()
        },
        ExecutorResultEvidence {
            pr: Some(76),
            ..recovery_evidence()
        },
    ] {
        assert_eq!(
            evaluate_merge_ready_claim_recovery(&claim, &evidence, &pull_request, &checks, false,),
            ClaimRecoveryDecision::Blocked(ClaimRecoveryBlock::IdentityMismatch)
        );
    }
}

#[test]
fn merge_ready_recovery_rejects_mismatched_claim_id() {
    let claim = recovery_claim().with_claim_id("claim-b");
    let evidence = recovery_evidence();

    assert_eq!(
        evaluate_merge_ready_claim_recovery(
            &claim,
            &evidence,
            &recovery_pull_request(),
            &[RequiredCheck::new("CI", "SUCCESS")],
            false,
        ),
        ClaimRecoveryDecision::Blocked(ClaimRecoveryBlock::IdentityMismatch)
    );
}

#[test]
fn blocks_recovery_while_the_prior_worker_lease_is_live() {
    assert_eq!(
        evaluate_merge_ready_claim_recovery(
            &recovery_claim(),
            &recovery_evidence(),
            &recovery_pull_request(),
            &[RequiredCheck::new("CI", "SUCCESS")],
            true,
        ),
        ClaimRecoveryDecision::Blocked(ClaimRecoveryBlock::LiveLease)
    );
}

#[test]
fn blocks_recovery_for_missing_pending_or_failing_required_checks() {
    let claim = recovery_claim();
    let evidence = recovery_evidence();
    let pull_request = recovery_pull_request();

    for (checks, reason) in [
        (Vec::new(), ClaimRecoveryBlock::MissingRequiredChecks),
        (
            vec![RequiredCheck::new("CI", "PENDING")],
            ClaimRecoveryBlock::RequiredCheckPending,
        ),
        (
            vec![RequiredCheck::new("CI", "FAILURE")],
            ClaimRecoveryBlock::RequiredCheckFailed,
        ),
    ] {
        assert_eq!(
            evaluate_merge_ready_claim_recovery(&claim, &evidence, &pull_request, &checks, false,),
            ClaimRecoveryDecision::Blocked(reason)
        );
    }
}
