use autospec_core::claim::render_safety_review_section;
use autospec_core::safety::{
    evaluate_issue_promotion, evaluate_issue_promotion_with_trusted_actors, redact_secrets,
    IssuePromotionPayload, IssuePromotionSafetyDecision, SafetyPolicy, UnsafeOperation,
};

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
    let github_token = ["gh", "p_", "123456789012345678901234567890123456"].concat();
    let input = format!("token {github_token} and key AKIA1234567890ABCDEF");

    let redacted = redact_secrets(&input);

    assert!(!redacted.contains(&github_token));
    assert!(!redacted.contains("AKIA1234567890ABCDEF"));
    assert!(redacted.contains("[REDACTED_GITHUB_TOKEN]"));
    assert!(redacted.contains("[REDACTED_AWS_KEY]"));
}

#[test]
fn issue_promotion_safety_gate_allows_safe_payload_and_marks_it_eligible() {
    let decision = evaluate_issue_promotion(IssuePromotionPayload::new(
        1890,
        "Add a typed issue promotion command",
        format!(
            "## Goal\nAdd `autospec issue promote`.\n\n{}",
            render_safety_review_section(autospec_core::claim::SafetyReviewDecision::Pass)
        ),
        "berlinguyinca",
        vec!["safety:reviewed".to_string()],
    ));

    assert_eq!(decision.safety_decision, IssuePromotionSafetyDecision::Pass);
    assert!(
        decision.auto_implement,
        "auto-implement is granted only after the final payload passes safety"
    );
    assert!(
        decision.eligible,
        "a safely promoted payload should be eligible for queue admission"
    );
    assert!(decision
        .final_labels
        .iter()
        .any(|label| label == "auto-implement"));
    assert!(decision.blocked_by_reason.is_empty());
}

#[test]
fn issue_promotion_safety_gate_returns_ambiguous_without_auto_implement() {
    let decision = evaluate_issue_promotion(IssuePromotionPayload::new(
        1891,
        "Clean production data",
        format!(
            "## Goal\nClean old data in production.\n\n{}",
            render_safety_review_section(autospec_core::claim::SafetyReviewDecision::Pass)
        ),
        "contributor",
        vec!["safety:reviewed".to_string()],
    ));

    assert_eq!(
        decision.safety_decision,
        IssuePromotionSafetyDecision::Ambiguous
    );
    assert!(!decision.auto_implement);
    assert!(!decision.eligible);
    assert!(!decision
        .final_labels
        .iter()
        .any(|label| label == "auto-implement"));
    assert!(decision.blocked_by_reason.is_empty());
}

#[test]
fn issue_promotion_safety_gate_groups_blocked_payloads_by_inner_safety_reason() {
    let decision = evaluate_issue_promotion(IssuePromotionPayload::new(
        1892,
        "Implement safe queue work",
        "## Goal\nImplement a harmless local queue change.",
        "contributor",
        Vec::new(),
    ));

    assert_eq!(
        decision.safety_decision,
        IssuePromotionSafetyDecision::Blocked
    );
    assert!(!decision.auto_implement);
    assert!(!decision.eligible);
    assert_eq!(
        decision
            .blocked_by_reason
            .get("missing_safety_reviewed")
            .copied(),
        Some(1)
    );
}

#[test]
fn issue_promotion_safety_gate_fails_closed_when_verdict_is_indeterminate() {
    let decision = evaluate_issue_promotion(IssuePromotionPayload::new(
        1893,
        "Implement safe queue work",
        "## Goal\nImplement a harmless local queue change.\n\n<!-- autospec-safety:begin -->",
        "contributor",
        vec!["safety:reviewed".to_string()],
    ));

    assert_eq!(
        decision.safety_decision,
        IssuePromotionSafetyDecision::Indeterminate
    );
    assert!(!decision.auto_implement);
    assert!(!decision.eligible);
    assert_eq!(
        decision
            .blocked_by_reason
            .get("invalid_safety_markers")
            .copied(),
        Some(1)
    );
}

#[test]
fn issue_promotion_safety_gate_accepts_a_reviewed_markerless_current_body() {
    let decision = evaluate_issue_promotion(IssuePromotionPayload::new(
        1895,
        "Implement safe queue work",
        "## Goal\nImplement a harmless local queue change.",
        "contributor",
        vec!["safety:reviewed".to_string()],
    ));

    assert_eq!(decision.safety_decision, IssuePromotionSafetyDecision::Pass);
    assert!(decision.auto_implement);
    assert!(decision.eligible);
}

#[test]
fn issue_promotion_safety_gate_uses_configured_trusted_actors() {
    let decision = evaluate_issue_promotion_with_trusted_actors(
        IssuePromotionPayload::new(
            1894,
            "Reset test database",
            format!(
                "## Goal\n\nDelete the local test database and repopulate it from fixtures.\n\nOnly test, local, and fixture data are in scope. Production is out of scope.\n\n{}",
                render_safety_review_section(autospec_core::claim::SafetyReviewDecision::Pass)
            ),
            "release-operator",
            vec!["safety:reviewed".to_string()],
        ),
        &["berlinguyinca", "release-operator"],
    );

    assert_eq!(decision.safety_decision, IssuePromotionSafetyDecision::Pass);
    assert!(decision.auto_implement);
    assert!(decision.eligible);
}

#[test]
fn safety_redacts_transport_diagnostics_across_credential_forms() {
    let github_pat = ["github", "_pat_11AA22BB33CC44DD55EE66FF77GG88HH99"].concat();
    let github_oauth = ["gh", "o_123456789012345678901234567890123456"].concat();
    let diagnostic = format!(
        "{github_pat} {github_oauth} Authorization: Bearer bearer-value api_key=key-value token:token-value user:hunter2 -----BEGIN PRIVATE KEY----- private-material -----END PRIVATE KEY-----"
    );

    let redacted = redact_secrets(&diagnostic);

    for secret in [
        github_pat.as_str(),
        github_oauth.as_str(),
        "bearer-value",
        "key-value",
        "token-value",
        "hunter2",
        "private-material",
    ] {
        assert!(!redacted.contains(secret), "leaked {secret}: {redacted}");
    }
}

#[test]
fn session_start_git_exclude_creates_missing_exclude_when_info_dir_exists() {
    let repo = unique_temp_repo("session-start-exclude");
    std::fs::create_dir_all(repo.join(".git/info")).expect("create .git/info");

    let outcome = autospec_core::safety::prepare_session_start_git_exclude(&repo)
        .expect("missing exclude file should be created, not treated as hook failure");

    assert_eq!(
        outcome,
        autospec_core::safety::SessionStartGitExcludeOutcome::Created
    );
    assert!(
        repo.join(".git/info/exclude").is_file(),
        "SessionStart should create .git/info/exclude when only the file is missing"
    );

    std::fs::remove_dir_all(repo).expect("remove temp repo");
}

#[test]
fn session_start_git_exclude_skips_missing_info_dir_without_dispatch_error() {
    let repo = unique_temp_repo("session-start-missing-info");
    std::fs::create_dir_all(repo.join(".git")).expect("create .git");

    let outcome = autospec_core::safety::prepare_session_start_git_exclude(&repo)
        .expect("missing .git/info should be non-fatal for SessionStart");

    match outcome {
        autospec_core::safety::SessionStartGitExcludeOutcome::SkippedMissingInfoDir {
            debug_reason,
        } => {
            assert!(debug_reason.contains(".git/info"));
            assert!(!debug_reason.contains("native_hook_dispatch_error"));
        }
        other => panic!("expected missing-info skip, got {other:?}"),
    }
    assert!(
        !repo.join(".git/info/exclude").exists(),
        "SessionStart should not create .git/info/exclude when .git/info is absent"
    );

    std::fs::remove_dir_all(repo).expect("remove temp repo");
}

fn unique_temp_repo(name: &str) -> std::path::PathBuf {
    static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    let mut path = std::env::temp_dir();
    path.push(format!(
        "autospec-{name}-{}-{}",
        std::process::id(),
        NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&path).expect("create temp repo root");
    path
}
