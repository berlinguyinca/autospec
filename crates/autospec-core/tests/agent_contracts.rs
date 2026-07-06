use autospec_core::agent::{render_handoff_prompt, AgentResult, AgentTask, SafeModePolicy};

#[test]
fn agent_contracts_result_normalizes_required_fields() {
    let result = AgentResult::new(
        "implemented V67",
        vec!["crates/autospec-core/src/agent/mod.rs".to_string()],
        "cargo test --all agent_contracts",
        Vec::new(),
        "ready for review",
    );

    let json = result.to_json();

    assert!(json.contains("\"result\":\"implemented V67\""));
    assert!(json.contains("\"files_changed\""));
    assert!(json.contains("\"validation\":\"cargo test --all agent_contracts\""));
}

#[test]
fn agent_contracts_safe_mode_blocks_destructive_operations_by_default() {
    let policy = SafeModePolicy::default();
    let task = AgentTask::new(
        "v67-agent-integration-contracts",
        "rm -rf /tmp/autospec-target",
        "cargo test --all agent_contracts",
    );

    let error = policy
        .check(&task)
        .expect_err("destructive operation should be blocked");

    assert!(error.contains("filesystem deletion"));
}

#[test]
fn agent_contracts_handoff_prompt_includes_spec_and_validation() {
    let task = AgentTask::new(
        "v67-agent-integration-contracts",
        "Implement agent contracts",
        "cargo test --all agent_contracts",
    );

    let prompt = render_handoff_prompt("codex", &task);

    assert!(prompt.contains("v67-agent-integration-contracts"));
    assert!(prompt.contains("cargo test --all agent_contracts"));
    assert!(prompt.contains("Codex"));
}
