//! AAR spec sections 18 and 19: policy assembly, versioning and explanations.

use autospec_core::aar::classify::{
    classify, ClassificationInput, Complexity, Risk, TaskClass, TaskClassification,
};
use autospec_core::aar::inferweave::LatencyPriority;
use autospec_core::aar::policy::{
    decide, decide_for_classification, role_capabilities, PolicyConfig, POLICY_SCHEMA_VERSION,
};
use autospec_core::aar::profile::{
    CapabilityScores, ModelProfile, ModelProfileRegistry, ProfileObservations,
};
use autospec_core::aar::reasoning::{ReasoningBudget, ReasoningLimits};
use autospec_core::aar::topology::{AgentRole, SeparationPolicy};

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
    assert!(decision
        .explain()
        .contains("No profile in registry empty-v1"));
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
    assert!(decision
        .policy
        .topology
        .contains(AgentRole::SecurityReviewer));
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

fn profile(
    model_id: &str,
    quantization: &str,
    scores: CapabilityScores,
    context_window: u64,
) -> ModelProfile {
    ModelProfile {
        model_id: model_id.to_string(),
        model_version: "1".to_string(),
        quantization: quantization.to_string(),
        backend: "vllm".to_string(),
        hardware_class: "rtx4090".to_string(),
        model_class: "coding-local".to_string(),
        provider: "inferweave".to_string(),
        context_window,
        supports_vision: false,
        supports_web: false,
        max_concurrent_sessions: 3,
        cost_per_1k_prompt_micros: 0,
        cost_per_1k_output_micros: 0,
        is_local: true,
        scores,
        observations: ProfileObservations::default(),
        profile_version: 1,
    }
}

/// A Low-risk bugfix: exactly the two-role topology (implementer, reviewer).
fn low_bugfix() -> TaskClassification {
    let mut classification = classify(
        &ClassificationInput::new(
            "Fix the flaky lease renewal test",
            "It fails under load. Reproduce with the fixture.",
        )
        .with_paths(["crates/autospec-core/src/claim/lease.rs"]),
    );
    classification.complexity = Complexity::Low;
    classification.risk = Risk::Low;
    classification.task_class = TaskClass::Bugfix;
    classification
}

fn registry_config(profiles: Vec<ModelProfile>) -> PolicyConfig {
    PolicyConfig {
        registry: ModelProfileRegistry::new("test-v1", profiles),
        minimum_capability_score: 0.4,
        ..PolicyConfig::default()
    }
}

/// One RoleAssignment per topology role: the producer takes the best coding
/// instance, the reviewer is pinned to a different instance, and the two
/// sessions are distinct (a shared session would be a shared context).
#[test]
fn a_two_role_decision_pins_producer_and_reviewer_to_different_instances() {
    let strong_coder = profile(
        "alpha",
        "q4_k_m",
        CapabilityScores {
            coding: 0.95,
            tool_use: 0.95,
            review: 0.4,
            repository_reasoning: 0.5,
            ..CapabilityScores::uniform(0.5)
        },
        32_768,
    );
    let strong_reviewer = profile(
        "beta",
        "bf16",
        CapabilityScores {
            coding: 0.5,
            tool_use: 0.5,
            review: 0.95,
            repository_reasoning: 0.9,
            ..CapabilityScores::uniform(0.5)
        },
        32_768,
    );
    let coder_key = strong_coder.key();
    let reviewer_key = strong_reviewer.key();

    let decision = decide_for_classification(
        low_bugfix(),
        &registry_config(vec![strong_coder, strong_reviewer]),
    )
    .expect("two profiles can hold the two roles");

    assert_eq!(decision.assignments.len(), 2, "one assignment per role");
    let by_role = |role: AgentRole| {
        decision
            .assignments
            .iter()
            .find(|assignment| assignment.role == role)
            .expect("every topology role is assigned")
    };
    let implementer = by_role(AgentRole::Implementer);
    let reviewer = by_role(AgentRole::Reviewer);

    assert_eq!(
        implementer.model_key, coder_key,
        "producer takes the best coder"
    );
    assert_eq!(
        reviewer.model_key, reviewer_key,
        "reviewer takes a different instance"
    );
    assert_eq!(implementer.model_class, "coding-local");
    assert_ne!(
        implementer.session_id, reviewer.session_id,
        "sessions must not share"
    );
    assert!(
        implementer.session_id.starts_with("aar-"),
        "{}",
        implementer.session_id
    );
    assert!(implementer.session_id.ends_with("-implementer"));
    assert!(reviewer.session_id.ends_with("-reviewer"));
}

/// A reviewer cannot sit on the producer's own instance: with exactly one
/// eligible profile the decision is an error naming both roles.
#[test]
fn a_two_role_decision_with_one_eligible_profile_is_an_error_naming_both_roles() {
    let only = profile(
        "alpha",
        "q4_k_m",
        CapabilityScores {
            coding: 0.95,
            tool_use: 0.95,
            review: 0.9,
            repository_reasoning: 0.9,
            ..CapabilityScores::uniform(0.5)
        },
        32_768,
    );
    let key = only.key();

    let error = decide_for_classification(low_bugfix(), &registry_config(vec![only]))
        .expect_err("one profile cannot both implement and review");

    assert!(error.contains("reviewer"), "{error}");
    assert!(error.contains("implementer"), "{error}");
    assert!(error.contains(&key), "{error}");
}

/// Trivial work stays single-agent and still gets its one assignment.
#[test]
fn a_single_agent_decision_gets_exactly_one_assignment() {
    let only = profile("alpha", "q4_k_m", CapabilityScores::uniform(0.8), 32_768);
    let key = only.key();
    let mut classification = low_bugfix();
    classification.complexity = Complexity::Trivial;

    let decision = decide_for_classification(classification, &registry_config(vec![only]))
        .expect("a single eligible profile routes trivial work");

    assert_eq!(decision.assignments.len(), 1, "single-agent topology");
    assert_eq!(decision.assignments[0].role, AgentRole::Implementer);
    assert_eq!(decision.assignments[0].model_key, key);
    assert!(decision.assignments[0].session_id.ends_with("-implementer"));
}

/// A reviewer judges the structured handoff, never the producer's working
/// context: when only the big window fits the producer's projected context,
/// the smaller instance is still eligible for the review and separation
/// holds instead of erroring.
#[test]
fn a_large_task_keeps_separation_when_only_the_big_window_fits_the_producer() {
    let paths: Vec<String> = (0..14)
        .map(|index| format!("crates/autospec-core/src/feature/part_{index}.rs"))
        .collect();
    let decision = decide(
        &ClassificationInput::new("Implement the report export surface", "Add the exporter.")
            .with_paths(paths),
        &config(),
    )
    .expect("the review does not need the producer's projected context");

    assert!(decision.policy.complexity >= Complexity::High);
    let by_role = |role: AgentRole| {
        decision
            .assignments
            .iter()
            .find(|assignment| assignment.role == role)
            .expect("role must be assigned")
    };
    let implementer = by_role(AgentRole::Implementer);
    let reviewer = by_role(AgentRole::Reviewer);

    assert!(
        implementer.model_key.contains("q4_k_m"),
        "{}",
        implementer.model_key
    );
    assert!(
        reviewer.model_key.contains("bf16"),
        "{}",
        reviewer.model_key
    );
    assert_ne!(implementer.model_key, reviewer.model_key);
}

/// When the separation policy forbids planner/reviewer sharing and both end
/// up on the only planning-grade instance, the decision is rejected rather
/// than silently weakened.
#[test]
fn a_planner_and_reviewer_forced_onto_one_instance_are_rejected_when_sharing_is_disabled() {
    let coder = profile(
        "alpha",
        "q4_k_m",
        CapabilityScores {
            coding: 0.95,
            tool_use: 0.95,
            planning: 0.05,
            review: 0.4,
            repository_reasoning: 0.5,
            ..CapabilityScores::uniform(0.5)
        },
        32_768,
    );
    let planner = profile(
        "beta",
        "bf16",
        CapabilityScores {
            coding: 0.5,
            tool_use: 0.5,
            planning: 1.0,
            review: 0.95,
            repository_reasoning: 0.9,
            ..CapabilityScores::uniform(0.5)
        },
        32_768,
    );
    let config = PolicyConfig {
        registry: ModelProfileRegistry::new("test-v1", vec![coder, planner]),
        minimum_capability_score: 0.4,
        projected_context_growth: 0,
        separation: SeparationPolicy {
            allow_planner_reviewer_sharing: false,
        },
        ..PolicyConfig::default()
    };
    let mut classification = low_bugfix();
    classification.complexity = Complexity::High;

    let error = decide_for_classification(classification, &config)
        .expect_err("planner and reviewer share the only planning-grade instance");

    assert!(error.contains("planner"), "{error}");
    assert!(error.contains("reviewer"), "{error}");
    assert!(error.contains("sharing is disabled"), "{error}");
}
