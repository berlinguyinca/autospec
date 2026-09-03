//! AAR spec sections 18 and 19: policy assembly, versioning and explanations.

use autospec_core::aar::classify::{ClassificationInput, Complexity, Risk, TaskClass};
use autospec_core::aar::inferweave::LatencyPriority;
use autospec_core::aar::policy::{
    decide, role_capabilities, PolicyConfig, POLICY_SCHEMA_VERSION,
};
use autospec_core::aar::profile::ModelProfileRegistry;
use autospec_core::aar::reasoning::{ReasoningBudget, ReasoningLimits};
use autospec_core::aar::topology::AgentRole;

fn config() -> PolicyConfig {
    PolicyConfig {
        registry: ModelProfileRegistry::starter(),
        minimum_capability_score: 0.4,
        ..PolicyConfig::default()
    }
}

/// Acceptance criterion 1: classify a work item and produce a policy.
#[test]
fn a_work_item_produces_a_complete_execution_policy() {
    let input = ClassificationInput::new(
        "Fix panic in the queue parser on empty specs",
        "The parser panics. Reproduce with the empty fixture and fix it.",
    )
    .with_paths(["crates/autospec-core/src/execution/queue_parser.rs"]);

    let decision = decide(&input, &config()).expect("config is valid");

    assert_eq!(decision.policy.task_class, TaskClass::Bugfix);
    assert!(decision.policy.topology.contains(AgentRole::Implementer));
    assert!(!decision.policy.context.include_full_history);
    assert_eq!(decision.policy.editing.max_edit_lines, 150);
    assert!(decision.policy.stop.stop_on_acceptance_met);
    assert!(!decision.policy.escalation.chain.is_empty());
    assert!(decision.policy.sampling.is_some());
}

/// Acceptance criterion 13: decisions are policy-versioned and auditable.
#[test]
fn every_decision_records_its_policy_version_and_candidates() {
    let decision = decide(
        &ClassificationInput::new("Fix the crash", "It panics.").with_paths(["src/a.rs"]),
        &config(),
    )
    .expect("config is valid");

    let record = decision.record();

    assert_eq!(record.schema_version, POLICY_SCHEMA_VERSION);
    assert_eq!(record.policy_version, "aar-v1");
    assert_eq!(record.registry_version, "starter-v1");
    assert!(!record.candidate_models.is_empty() || !record.rejected_models.is_empty());
    assert!(!record.rationale.is_empty());
    assert!(!record.classification_evidence.is_empty());
}

#[test]
fn a_custom_policy_version_is_carried_onto_the_decision() {
    let decision = decide(
        &ClassificationInput::new("Fix the crash", "It panics.").with_paths(["src/a.rs"]),
        &PolicyConfig {
            policy_version: "aar-2026-09-02".to_string(),
            ..config()
        },
    )
    .expect("config is valid");

    assert_eq!(decision.policy_version, "aar-2026-09-02");
    assert!(decision.explain().contains("aar-2026-09-02"));
}

#[test]
fn the_decision_serializes_to_json_for_the_dashboard_api() {
    let decision = decide(
        &ClassificationInput::new("Fix the crash in the parser", "It panics.")
            .with_paths(["src/parser.rs"]),
        &config(),
    )
    .expect("config is valid");

    let json = decision.to_json().expect("serializes");

    for key in [
        "\"policy_version\"",
        "\"task_class\"",
        "\"reasoning_budget\"",
        "\"selected_model\"",
        "\"retrieval_ladder\"",
        "\"escalation_chain\"",
        "\"rationale\"",
    ] {
        assert!(json.contains(key), "decision json must carry {key}");
    }
}

/// Acceptance criterion 12: the selected profile is explainable.
#[test]
fn the_explanation_names_the_model_budget_task_shape_and_separation() {
    let decision = decide(
        &ClassificationInput::new(
            "Fix panic in the queue parser",
            "Medium bugfix touching the parser and its tests.",
        )
        .with_paths([
            "crates/autospec-core/src/execution/queue_parser.rs",
            "crates/autospec-core/tests/execution_queue.rs",
            "crates/autospec-core/src/execution/queue.rs",
            "crates/autospec-core/src/execution/result.rs",
        ]),
        &config(),
    )
    .expect("config is valid");

    let explanation = decision.explain();

    assert!(explanation.contains("qwen3.8-27b"));
    assert!(explanation.contains("reasoning"));
    assert!(explanation.contains("bugfix"));
    assert!(explanation.contains("Separation-of-duty requirements remain satisfied."));
}

#[test]
fn an_unroutable_request_explains_that_no_model_was_eligible() {
    let decision = decide(
        &ClassificationInput::new("Review the screenshots", "Compare the rendered pages.")
            .with_labels(["type:ui"])
            .with_paths(["src/App.tsx"]),
        &PolicyConfig {
            registry: ModelProfileRegistry::new("empty-v1", Vec::new()),
            ..config()
        },
    )
    .expect("config is valid");

    assert_eq!(decision.selected_model(), None);
    assert!(decision.explain().contains("No profile in registry empty-v1"));
}

#[test]
fn the_context_requirement_grows_with_the_retrieval_budget() {
    let small = decide(
        &ClassificationInput::new("Fix typo", "One line.").with_paths(["docs/a.md"]),
        &config(),
    )
    .expect("config is valid");
    let large = decide(
        &ClassificationInput::new("Implement the exporter", "Broad change.")
            .with_estimated_files(24),
        &config(),
    )
    .expect("config is valid");

    assert!(
        small.policy.model_requirements.minimum_context_free
            < large.policy.model_requirements.minimum_context_free
    );
}

#[test]
fn the_capability_request_asks_for_capabilities_not_a_node() {
    let decision = decide(
        &ClassificationInput::new("Fix the crash", "It panics.").with_paths(["src/a.rs"]),
        &PolicyConfig {
            latency_priority: LatencyPriority::Latency,
            ..config()
        },
    )
    .expect("config is valid");

    let request = &decision.capability_request;

    assert_eq!(request.model_class, "coding-local");
    assert!(request.session_affinity);
    assert_eq!(request.latency_priority, LatencyPriority::Latency);
    assert!(request.seat.projected_growth_tokens > 0);
    assert_eq!(
        request.required_free_context(),
        decision.policy.model_requirements.minimum_context_free
    );
}

#[test]
fn a_low_confidence_classification_raises_the_reasoning_budget() {
    let vague = decide(
        &ClassificationInput::new("Handle the thing", "Make it work."),
        &config(),
    )
    .expect("config is valid");

    assert!(vague.record().needs_tie_breaker);
    assert!(vague.policy.reasoning.budget > ReasoningBudget::Tiny);
}

#[test]
fn an_invalid_config_is_rejected_before_any_decision_is_made() {
    let error = decide(
        &ClassificationInput::new("x", "y"),
        &PolicyConfig {
            reasoning_limits: ReasoningLimits {
                tiny: 9_000,
                ..ReasoningLimits::default()
            },
            ..config()
        },
    )
    .unwrap_err();

    assert!(error.contains("reasoning limits must increase"));
}

#[test]
fn an_out_of_range_capability_score_is_rejected() {
    let error = decide(
        &ClassificationInput::new("x", "y"),
        &PolicyConfig {
            minimum_capability_score: 2.0,
            ..config()
        },
    )
    .unwrap_err();

    assert!(error.contains("minimum_capability_score"));
}

#[test]
fn a_blank_policy_version_is_rejected() {
    let error = decide(
        &ClassificationInput::new("x", "y"),
        &PolicyConfig {
            policy_version: "   ".to_string(),
            ..config()
        },
    )
    .unwrap_err();

    assert!(error.contains("requires a version"));
}

#[test]
fn critical_risk_work_carries_a_security_reviewer_and_a_larger_budget() {
    let decision = decide(
        &ClassificationInput::new(
            "Rework the credential helper",
            "Change how the token is stored.",
        )
        .with_paths(["crates/autospec-cli/src/commands/security/credential.rs"]),
        &config(),
    )
    .expect("config is valid");

    assert_eq!(decision.policy.risk, Risk::Critical);
    assert!(decision.policy.topology.contains(AgentRole::SecurityReviewer));
    assert!(decision.policy.reasoning.budget >= ReasoningBudget::Complex);
}

#[test]
fn trivial_work_stays_single_agent_with_the_tiny_budget() {
    let decision = decide(
        &ClassificationInput::new("Fix typo in the install guide", "One line: copy the fix.")
            .with_paths(["docs/install.md"]),
        &config(),
    )
    .expect("config is valid");

    assert_eq!(decision.policy.complexity, Complexity::Trivial);
    assert!(decision.policy.topology.is_single_agent());
    assert_eq!(decision.policy.reasoning.budget, ReasoningBudget::Tiny);
}

#[test]
fn every_role_declares_the_capabilities_it_needs() {
    for role in [
        AgentRole::Coordinator,
        AgentRole::Explorer,
        AgentRole::Planner,
        AgentRole::Implementer,
        AgentRole::Tester,
        AgentRole::Reviewer,
        AgentRole::DocumentationWriter,
        AgentRole::UiEvaluator,
        AgentRole::SecurityReviewer,
        AgentRole::PerformanceReviewer,
    ] {
        assert!(
            !role_capabilities(role).is_empty(),
            "{} must declare capabilities",
            role.as_str()
        );
    }
}
