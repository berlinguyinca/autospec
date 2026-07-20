use autospec_core::claim::{
    claim_losing_worker_comment_id, executor_wait_failure_relinquishes_claim,
    lowest_marked_comment, select_run_state, ExecutorResultEvidence, RemoteComment, RunStateRecord,
};

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
fn wait_failure_evidence_relinquishes_only_the_exact_claim_generation() {
    let legacy_claim = RunStateRecord::new(
        "testorg/testrepo",
        42,
        "worker-a",
        "claimed",
        "feat/test",
        "",
        "claimed",
        Vec::new(),
        "2026-01-01T00:00:00Z",
        "2026-01-01T00:00:00Z",
        10_800,
    );
    let claim = legacy_claim.clone().with_claim_id("claim-generation-a");
    let exact = ExecutorResultEvidence::new(
        "testorg/testrepo",
        42,
        "worker-a",
        "feat/test",
        "failed",
        None,
        "implementer_wait_failed",
        "implementer-wait-failed:claim-generation-a:session-7",
    );
    let prior_generation = ExecutorResultEvidence::new(
        "testorg/testrepo",
        42,
        "worker-a",
        "feat/test",
        "failed",
        None,
        "implementer_wait_failed",
        "implementer-wait-failed:claim-generation-prior:session-6",
    );

    assert!(executor_wait_failure_relinquishes_claim(
        &[RemoteComment::new(
            101,
            exact.to_marked_comment(),
            "2026-01-01T00:00:01Z",
        )],
        &claim,
    ));
    assert!(!executor_wait_failure_relinquishes_claim(
        &[RemoteComment::new(
            101,
            prior_generation.to_marked_comment(),
            "2026-01-01T00:00:01Z",
        )],
        &claim,
    ));
    assert!(!executor_wait_failure_relinquishes_claim(
        &[RemoteComment::new(
            101,
            exact.to_marked_comment(),
            "2026-01-01T00:00:01Z",
        )],
        &legacy_claim,
    ));
}
