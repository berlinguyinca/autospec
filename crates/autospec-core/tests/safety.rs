use autospec_core::safety::{redact_secrets, SafetyPolicy, UnsafeOperation};

#[test]
fn safety_blocks_unsafe_operations_by_default() {
    let policy = SafetyPolicy::default();

    let error = policy
        .check("git reset --hard HEAD")
        .expect_err("destructive git should be blocked");

    assert_eq!(error.operation, UnsafeOperation::DestructiveGit);
}

#[test]
fn safety_redacts_secret_like_values() {
    let input = "token ghp_123456789012345678901234567890123456 and key AKIA1234567890ABCDEF";

    let redacted = redact_secrets(input);

    assert!(!redacted.contains("ghp_123456789012345678901234567890123456"));
    assert!(!redacted.contains("AKIA1234567890ABCDEF"));
    assert!(redacted.contains("[REDACTED_GITHUB_TOKEN]"));
    assert!(redacted.contains("[REDACTED_AWS_KEY]"));
}
