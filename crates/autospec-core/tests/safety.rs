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
    let github_token = ["gh", "p_", "123456789012345678901234567890123456"].concat();
    let input = format!("token {github_token} and key AKIA1234567890ABCDEF");

    let redacted = redact_secrets(&input);

    assert!(!redacted.contains(&github_token));
    assert!(!redacted.contains("AKIA1234567890ABCDEF"));
    assert!(redacted.contains("[REDACTED_GITHUB_TOKEN]"));
    assert!(redacted.contains("[REDACTED_AWS_KEY]"));
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
