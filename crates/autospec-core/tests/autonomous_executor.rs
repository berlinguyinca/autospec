use autospec_core::autonomous::executor::ExecutorInvocation;

fn valid() -> ExecutorInvocation {
    ExecutorInvocation {
        repo: "test/repo".into(),
        issue: 42,
        worker_id: "worker-42".into(),
        branch: "autonomous/issue-42".into(),
        claim_id: "claim-42".into(),
        invocation_id: "42-claim-42".into(),
        expected_commit: "a".repeat(40),
    }
}

#[test]
fn invocation_identity_accepts_bound_commit() {
    assert!(valid().validate().is_ok());
}

#[test]
fn invocation_identity_rejects_invalid_commit_and_empty_fields() {
    let mut invocation = valid();
    invocation.expected_commit = "not-a-commit".into();
    assert!(invocation.validate().is_err());
    invocation = valid();
    invocation.claim_id.clear();
    assert!(invocation.validate().is_err());
}
