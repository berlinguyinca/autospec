use autospec_core::agent::{render_handoff_prompt, AgentResult, AgentTask, SafeModePolicy};
use autospec_core::execution::{AgentOutcome, FailureKind, IngestedAgentResult};

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
fn agent_contracts_parse_strict_results_and_render_canonical_json() {
    let result = AgentResult::new(
        "implemented V67\nwith evidence",
        vec!["crates/autospec-core/src/agent/mod.rs".to_string()],
        "cargo test --all agent_contracts\tpassed",
        vec!["waiting for external review".to_string()],
        "ready for review",
    );

    let json = result.to_json();
    let parsed = AgentResult::from_json(&json).expect("canonical result parses");

    assert_eq!(parsed, result);
    assert_eq!(parsed.to_json(), json);
    assert!(AgentResult::from_json(
        "{\"result\":\"ok\",\"files_changed\":[],\"validation\":\"check\",\"blockers\":[],\"handoff\":\"done\",\"extra\":true}"
    )
    .is_err());
}

#[test]
fn agent_contracts_require_explicit_outcome_evidence_without_reading_prose() {
    let without_validation = AgentResult::new("passed apparently", Vec::new(), "", Vec::new(), "");
    let without_blocker = AgentResult::new("blocked apparently", Vec::new(), "", Vec::new(), "");

    assert!(IngestedAgentResult::new_at(
        "run-v67",
        "v67-agent-integration-contracts",
        "result-1",
        AgentOutcome::Passed,
        without_validation,
        100,
    )
    .is_err());
    assert!(IngestedAgentResult::new_at(
        "run-v67",
        "v67-agent-integration-contracts",
        "result-2",
        AgentOutcome::Blocked,
        without_blocker,
        100,
    )
    .is_err());

    let failed = IngestedAgentResult::new_at(
        "run-v67",
        "v67-agent-integration-contracts",
        "result-3",
        AgentOutcome::Failed {
            failure_kind: FailureKind::Validation,
        },
        AgentResult::new(
            "the tests failed",
            Vec::new(),
            "cargo test: exit 1",
            Vec::new(),
            "",
        ),
        100,
    )
    .expect("explicit failed outcome is valid");

    assert_eq!(failed.outcome.as_str(), "failed");
    assert_eq!(
        failed.to_json(),
        IngestedAgentResult::from_json(&failed.to_json())
            .unwrap()
            .to_json()
    );
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
fn agent_contracts_safe_mode_also_checks_the_rendered_validation_command() {
    let policy = SafeModePolicy::default();
    let task = AgentTask::new(
        "v67-agent-integration-contracts",
        "Run the documented check",
        "git push --force origin main",
    );

    let error = policy
        .check(&task)
        .expect_err("destructive validation command should be blocked");

    assert!(error.contains("destructive git"));
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
