use autospec_core::execution::{ExecutionQueue, FailureKind, QueueStatus};

#[test]
fn execution_queue_resumes_first_incomplete_entry() {
    let mut queue = ExecutionQueue::new(
        "run-v66",
        vec![
            "v65-spec-state-validation".to_string(),
            "v66-autonomous-execution-queue".to_string(),
        ],
    );
    queue
        .mark_passed("v65-spec-state-validation")
        .expect("known spec");

    let next = queue.next_incomplete().expect("second entry remains");

    assert_eq!(next.spec_id, "v66-autonomous-execution-queue");
    assert_eq!(next.status, QueueStatus::Pending);
}

#[test]
fn execution_queue_enforces_retry_limit() {
    let mut queue = ExecutionQueue::new(
        "run-v66",
        vec!["v66-autonomous-execution-queue".to_string()],
    );

    queue
        .record_failure("v66-autonomous-execution-queue", FailureKind::Validation, 1)
        .expect("first failure records");
    let error = queue
        .record_failure("v66-autonomous-execution-queue", FailureKind::Validation, 1)
        .expect_err("second failure exceeds retry limit");

    assert!(error.contains("retry limit exceeded"));
    assert_eq!(
        queue
            .entry("v66-autonomous-execution-queue")
            .unwrap()
            .status,
        QueueStatus::Blocked
    );
}

#[test]
fn execution_queue_renders_handoff_and_report() {
    let mut queue = ExecutionQueue::new(
        "run-v66",
        vec![
            "v65-spec-state-validation".to_string(),
            "v66-autonomous-execution-queue".to_string(),
        ],
    );
    queue
        .mark_passed("v65-spec-state-validation")
        .expect("known spec");
    queue
        .block("v66-autonomous-execution-queue", "validation failed")
        .expect("known spec");

    let handoff = queue
        .handoff_markdown("v66-autonomous-execution-queue")
        .unwrap();
    let report = queue.final_report_markdown();

    assert!(handoff.contains("# Blocked Spec: v66-autonomous-execution-queue"));
    assert!(handoff.contains("validation failed"));
    assert!(report.contains("passed: 1"));
    assert!(report.contains("blocked: 1"));
}
