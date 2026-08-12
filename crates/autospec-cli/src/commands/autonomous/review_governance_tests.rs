use super::*;

#[test]
fn lifecycle_atomic_temp_paths_are_unique_per_write() {
    let path = Path::new("/tmp/lifecycle.json");

    assert_ne!(atomic_temporary_path(path), atomic_temporary_path(path));
}

#[test]
fn selected_issue_serialization_reasons_reach_the_executor_request() {
    let root = std::env::temp_dir().join("autospec-review-reasons-threading");
    let layout = RunLayout {
        state_dir: root.join("state"),
        log_dir: root.join("log"),
        scope: "test_repo".to_string(),
        repo: "test/repo".to_string(),
    };
    let reasons = vec!["priority:high".to_string(), "reasoning:deep".to_string()];

    let request = ExecutorRequest::for_selected(
        &layout,
        "/tmp/repo",
        42,
        "Risky issue",
        "## Goal\n\nExercise risk threading.",
        &reasons,
        "worker-42",
        "feat/autonomous-issue-42",
        "claim-42",
    )
    .expect("selected executor request");

    assert_eq!(request.bridge.serialization_reasons, reasons);
}
