use autospec_core::claim::{
    claim_losing_worker_comment_id, executor_result_evidence_exists, lowest_marked_comment,
    parse_open_pull_requests_json, select_run_state, ExecutorResultEvidence, RemoteComment,
    RunStateRecord,
};

const RECEIPT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn marked_comment(id: u64, worker_id: &str) -> RemoteComment {
    let record = RunStateRecord::new(
        "testorg/testrepo",
        42,
        worker_id,
        "claimed",
        "feat/claim-race",
        "",
        "claimed",
        Vec::new(),
        "2026-01-01T00:00:00Z",
        "2026-01-01T00:00:00Z",
        10_800,
    );
    RemoteComment::new(id, record.to_marked_comment(), "2026-01-01T00:00:00Z")
}

#[test]
fn lowest_comment_id_wins_even_when_api_order_is_descending() {
    let comments = vec![
        marked_comment(101, "worker-b"),
        marked_comment(100, "worker-a"),
    ];

    let selected = select_run_state(&comments, "testorg/testrepo", 42).expect("run state");

    assert_eq!(selected.comment_id, 100);
    assert_eq!(selected.record.worker_id, "worker-a");
    assert_eq!(
        lowest_marked_comment(&comments).map(|comment| comment.id),
        Some(100)
    );
}

#[test]
fn higher_id_worker_loses_and_self_cleanup_targets_only_that_comment() {
    let comments = vec![
        marked_comment(101, "worker-b"),
        marked_comment(100, "worker-a"),
    ];

    assert_eq!(
        claim_losing_worker_comment_id(&comments, "worker-b"),
        Some(101)
    );
    assert_eq!(claim_losing_worker_comment_id(&comments, "worker-a"), None);
}

#[test]
fn dotted_worker_id_cleanup_uses_literal_equality_not_regex_matching() {
    let comments = vec![
        marked_comment(100, "winner-a"),
        marked_comment(101, "mac.lan:bob:monitor:1"),
        marked_comment(102, "macXlan:bob:monitor:1"),
    ];

    assert_eq!(
        claim_losing_worker_comment_id(&comments, "mac.lan:bob:monitor:1"),
        Some(101)
    );
    assert_eq!(
        claim_losing_worker_comment_id(&comments, "macXlan:bob:monitor:1"),
        Some(102)
    );
}

#[test]
fn executor_result_pull_request_requires_the_head_commit_oid() {
    let pull_requests = parse_open_pull_requests_json(
        r#"[{"number":17,"body":"Closes #42","headRefName":"feat/claim","headRefOid":"0123456789abcdef0123456789abcdef01234567"}]"#,
    )
    .expect("parse exact open pull request evidence");

    assert_eq!(
        pull_requests[0].head_ref_oid,
        "0123456789abcdef0123456789abcdef01234567"
    );
    assert!(parse_open_pull_requests_json(
        r#"[{"number":17,"body":"Closes #42","headRefName":"feat/claim"}]"#
    )
    .is_err());
}

#[test]
fn executor_result_evidence_is_bound_to_one_claim_generation_commit_and_receipt() {
    let exact = ExecutorResultEvidence::new(
        "testorg/testrepo",
        42,
        "worker-a",
        "feat/test",
        "succeeded",
        Some(17),
        "executor_succeeded",
        "result-17",
        Some("claim-generation-a".to_string()),
        Some("0123456789abcdef0123456789abcdef01234567".to_string()),
        Some(RECEIPT.to_string()),
    );
    let successor = ExecutorResultEvidence::new(
        "testorg/testrepo",
        42,
        "worker-a",
        "feat/test",
        "succeeded",
        Some(17),
        "executor_succeeded",
        "result-17",
        Some("claim-generation-b".to_string()),
        Some("0123456789abcdef0123456789abcdef01234567".to_string()),
        Some(RECEIPT.to_string()),
    );
    let comments = [RemoteComment::new(
        101,
        exact.to_marked_comment(),
        "2026-01-01T00:00:01Z",
    )];

    assert!(executor_result_evidence_exists(&comments, &exact));
    assert!(!executor_result_evidence_exists(&comments, &successor));
}
