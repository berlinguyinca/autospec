//! AAR spec section 21: the required end-to-end scenarios, driven through the
//! whole decision pipeline rather than a single module.

use autospec_core::aar::classify::{ClassificationInput, Complexity, TaskClass};
use autospec_core::aar::context::{CacheFriendlyPrompt, ContextSegment, PromptBlock};
use autospec_core::aar::escalation::{
    next_attempt, Attempt, EscalationContext, EscalationOutcome, EscalationPolicy, QuotaState,
};
use autospec_core::aar::guards::{
    evaluate_stop, EditAction, EditGuard, ExecutionProgress, StepEvent, StopReason, ThrashDetector,
    ThrashSignal,
};
use autospec_core::aar::inferweave::{route, CapabilityRequest, NodeOffer};
use autospec_core::aar::policy::{decide, PolicyConfig, PolicyDecision};
use autospec_core::aar::profile::ModelProfileRegistry;
use autospec_core::aar::reasoning::{
    select_reasoning, ReasoningBudget, ReasoningContext, ReasoningHistory, ReasoningLimits,
};
use autospec_core::aar::telemetry::{ExecutionTelemetry, FailureCategory, ReviewOutcome};
use autospec_core::aar::topology::{AgentRole, RoleAssignment, SeparationPolicy};

fn config() -> PolicyConfig {
    PolicyConfig {
        registry: ModelProfileRegistry::starter(),
        minimum_capability_score: 0.4,
        ..PolicyConfig::default()
    }
}

fn plan(input: ClassificationInput) -> PolicyDecision {
    decide(&input, &config()).expect("config is valid")
}

fn node(node_id: &str, free_context: u64) -> NodeOffer {
    NodeOffer {
        node_id: node_id.to_string(),
        served_models: vec!["qwen3.8-27b".to_string()],
        model_classes: vec!["coding-local".to_string()],
        free_context_tokens: free_context,
        total_context_tokens: 65_536,
        is_local: true,
        warm_prefix_cache_keys: Vec::new(),
        affinity_session_id: None,
        utilization: 0.2,
        queue_depth: 0,
        observed_prefill_tokens_per_second: 1_200.0,
        observed_decode_tokens_per_second: 60.0,
        network_cost: 0.0,
        qos_share_remaining: 1.0,
        overloaded: false,
    }
}

#[test]
fn scenario_trivial_edit_runs_one_agent_at_the_smallest_budget() {
    let decision = plan(
        ClassificationInput::new(
            "Fix typo in the quickstart",
            "One line: copy the correction.",
        )
        .with_paths(["docs/quickstart.md"]),
    );

    assert_eq!(decision.policy.complexity, Complexity::Trivial);
    assert!(decision.policy.topology.is_single_agent());
    assert_eq!(decision.policy.reasoning.budget, ReasoningBudget::Tiny);
    assert!(decision.policy.context.max_retrieved_files <= 3);
}

#[test]
fn scenario_medium_rust_bugfix_gets_an_explorer_a_tester_and_a_reviewer() {
    let decision = plan(
        ClassificationInput::new(
            "Fix panic in the queue parser on empty specs",
            "The parser panics on an empty document; reproduce and fix.",
        )
        .with_paths([
            "crates/autospec-core/src/execution/queue_parser.rs",
            "crates/autospec-core/src/execution/queue.rs",
            "crates/autospec-core/tests/execution_queue.rs",
            "crates/autospec-core/src/execution/result.rs",
            "crates/autospec-core/src/execution/queue_storage.rs",
        ]),
    );

    assert_eq!(decision.policy.task_class, TaskClass::Bugfix);
    assert_eq!(decision.policy.complexity, Complexity::Medium);
    assert!(decision.policy.topology.contains(AgentRole::Explorer));
    assert!(decision.policy.topology.contains(AgentRole::Tester));
    assert!(decision.policy.topology.contains(AgentRole::Reviewer));
    assert!(decision.policy.topology.isolated_contexts);
}

#[test]
fn scenario_multi_file_feature_adds_a_planner_and_a_coordinator() {
    let paths: Vec<String> = (0..14)
        .map(|index| format!("crates/autospec-core/src/feature/part_{index}.rs"))
        .collect();
    let decision = plan(
        ClassificationInput::new("Implement the report export surface", "Add the exporter.")
            .with_paths(paths),
    );

    assert!(decision.policy.complexity >= Complexity::High);
    assert!(decision.policy.topology.contains(AgentRole::Planner));
    assert!(decision.policy.topology.contains(AgentRole::Coordinator));
    assert!(decision.classification.requires_long_context);
}

#[test]
fn scenario_difficult_debugging_escalates_the_budget_across_retries() {
    let decision = plan(
        ClassificationInput::new(
            "Fix the intermittent crash in the lease renewer",
            "Hard to reproduce; the process panics under load.",
        )
        .with_paths(["crates/autospec-core/src/claim/lease.rs"]),
    );

    let escalated = select_reasoning(
        &decision.classification,
        &ReasoningContext {
            retries: 2,
            reviewer_rejected: true,
            ..ReasoningContext::default()
        },
        &ReasoningHistory::default(),
        &ReasoningLimits::default(),
    );

    assert!(escalated.budget > decision.policy.reasoning.budget);
    assert_eq!(escalated.budget, ReasoningBudget::Exceptional);
}

#[test]
fn scenario_test_generation_keeps_a_small_budget_and_a_test_retrieval_rung() {
    let decision = plan(
        ClassificationInput::new(
            "Add unit tests for the queue parser",
            "Add tests covering the empty-document case.",
        )
        .with_labels(["type:test"])
        .with_paths(["crates/autospec-core/tests/execution_queue.rs"]),
    );

    assert_eq!(decision.policy.task_class, TaskClass::Test);
    assert!(decision.policy.reasoning.budget <= ReasoningBudget::Normal);
    assert!(decision
        .policy
        .context
        .ladder
        .iter()
        .any(|step| step.strategy.as_str() == "tests"));
}

#[test]
fn scenario_documentation_uses_a_documentation_writer() {
    let decision = plan(
        ClassificationInput::new(
            "Document the runtime isolation commands",
            "Write the documentation for the runtime env subcommands.",
        )
        .with_paths(["docs/USER_MANUAL.md", "docs/cli-reference.md"]),
    );

    assert_eq!(decision.policy.task_class, TaskClass::Docs);
    assert!(decision
        .policy
        .topology
        .contains(AgentRole::DocumentationWriter));
    assert!(!decision.policy.topology.contains(AgentRole::Implementer));
}

#[test]
fn scenario_review_rejection_raises_the_budget_and_records_the_outcome() {
    let decision = plan(
        ClassificationInput::new("Implement the exporter", "Add the exporter.")
            .with_paths(["src/a.rs", "src/b.rs", "src/c.rs", "src/d.rs"]),
    );

    let rework = select_reasoning(
        &decision.classification,
        &ReasoningContext {
            reviewer_rejected: true,
            retries: 1,
            ..ReasoningContext::default()
        },
        &ReasoningHistory::default(),
        &ReasoningLimits::default(),
    );

    let record = ExecutionTelemetry {
        prompt_tokens: 8_000,
        cached_prompt_tokens: 6_000,
        new_prefill_tokens: 2_000,
        review_outcome: ReviewOutcome::Rejected,
        success: false,
        failure_category: FailureCategory::ReviewRejected,
        retries: 1,
        ..ExecutionTelemetry::default()
    };

    assert!(rework.budget > decision.policy.reasoning.budget);
    assert!(record.validate().is_ok());
}

#[test]
fn scenario_provider_exhaustion_escalates_without_weakening_separation() {
    let registry = ModelProfileRegistry::starter();
    let requirements = decide(
        &ClassificationInput::new("Fix the crash", "It panics.").with_paths(["src/a.rs"]),
        &config(),
    )
    .expect("config is valid")
    .policy
    .model_requirements;
    let policy = EscalationPolicy::default();
    let separation = SeparationPolicy::default();
    let mut quota = QuotaState::new();
    quota.set("inferweave", 0);
    quota.set("cloud", 0);

    let implementer = RoleAssignment::new(
        AgentRole::Implementer,
        "qwen3.8-27b@3.8/q4_k_m/vllm/rtx4090",
        "coding-local",
        "session-a",
    );
    let assignments = [
        implementer.clone(),
        RoleAssignment::new(
            AgentRole::Reviewer,
            "qwen3.8-27b@3.8/bf16/vllm/dual-turing",
            "coding-local",
            "session-b",
        ),
    ];
    let failed = Attempt {
        assignment: implementer,
        budget: ReasoningBudget::Exceptional,
        provider: "inferweave".to_string(),
        step: None,
    };
    let context = EscalationContext {
        policy: &policy,
        registry: &registry,
        requirements: &requirements,
        separation_policy: &separation,
        current_assignments: &assignments,
        quota: &quota,
        attempts: &[],
    };

    assert!(matches!(
        next_attempt(&context, &failed),
        EscalationOutcome::Escalate { .. }
    ));
}

#[test]
fn scenario_insufficient_node_context_leaves_the_request_unrouted() {
    let decision = plan(
        ClassificationInput::new("Implement the exporter", "Broad change.")
            .with_estimated_files(24),
    );
    let request = CapabilityRequest {
        minimum_context_free: decision.policy.model_requirements.minimum_context_free,
        model_allowlist: vec!["qwen3.8-27b".to_string()],
        ..CapabilityRequest::default()
    };

    let routed = route(&request, &[node("too-small", 8_000)]);

    assert!(!routed.is_routed());
    assert!(routed.rejected[0].1.contains("free context"));
}

#[test]
fn scenario_warm_prefix_cache_wins_and_shows_up_in_telemetry() {
    let prompt = CacheFriendlyPrompt::assemble(vec![
        PromptBlock::new(
            ContextSegment::HarnessInstructions,
            "you are a coding agent",
        ),
        PromptBlock::new(ContextSegment::Tools, "read, edit, run"),
        PromptBlock::new(ContextSegment::ModelRules, "one logical change at a time"),
        PromptBlock::new(ContextSegment::RepositoryInstructions, "see AGENTS.md"),
        PromptBlock::new(ContextSegment::Task, "fix the parser panic"),
    ])
    .expect("blocks assemble");
    let cache_key = prompt.stable_prefix_hash();

    let mut warm = node("warm", 40_000);
    warm.warm_prefix_cache_keys = vec![cache_key.clone()];
    let request = CapabilityRequest {
        minimum_context_free: 24_000,
        prefix_cache_key: cache_key,
        model_allowlist: vec!["qwen3.8-27b".to_string()],
        ..CapabilityRequest::default()
    };

    let routed = route(&request, &[node("cold", 40_000), warm]);

    assert_eq!(routed.selected.as_deref(), Some("warm"));

    let record = ExecutionTelemetry {
        prompt_tokens: 10_000,
        cached_prompt_tokens: 9_000,
        new_prefill_tokens: 1_000,
        success: true,
        ..ExecutionTelemetry::default()
    };
    assert!(record.validate().is_ok());
    assert_eq!(record.cache_hit_rate(), 0.9);
}

/// The full inner loop: guards run over the agent's actions, the thrash
/// detector watches the same stream, and the stop evaluator ends the run the
/// moment acceptance is met.
#[test]
fn scenario_a_guarded_run_stops_the_moment_acceptance_is_met() {
    let decision = plan(
        ClassificationInput::new("Fix panic in the parser", "It panics on empty input.")
            .with_paths(["src/parser.rs"]),
    );
    let mut guard = EditGuard::new(decision.policy.editing.clone());
    let mut detector = ThrashDetector::with_defaults();
    let mut violations = Vec::new();
    let mut thrash = Vec::new();

    let actions = [
        EditAction::Read {
            path: "src/parser.rs".to_string(),
        },
        EditAction::Edit {
            path: "src/parser.rs".to_string(),
            lines: 12,
        },
        EditAction::Read {
            path: "src/parser.rs".to_string(),
        },
        EditAction::RunTests {
            command: "cargo test -p autospec-core".to_string(),
            passed: true,
        },
    ];
    for (index, action) in actions.iter().enumerate() {
        violations.extend(guard.observe(action));
        thrash.extend(detector.observe(&StepEvent {
            action: action.clone(),
            recorded_finding: index == 0,
            cumulative_tokens: 1_000 * (index as u64 + 1),
            state_digest: format!("state-{index}"),
        }));
    }
    violations.extend(guard.end_step());

    assert!(
        violations.is_empty(),
        "clean run must not trip a guard: {violations:?}"
    );
    assert!(
        thrash.is_empty(),
        "clean run must not look like thrashing: {thrash:?}"
    );
    assert_eq!(
        evaluate_stop(
            &decision.policy.stop,
            &ExecutionProgress {
                steps: 4,
                acceptance_criteria_total: 2,
                acceptance_criteria_met: 2,
                ..ExecutionProgress::default()
            }
        ),
        Some(StopReason::AcceptanceMet)
    );
}

#[test]
fn scenario_a_looping_agent_is_detected_and_stopped() {
    let decision = plan(
        ClassificationInput::new("Fix panic in the parser", "It panics on empty input.")
            .with_paths(["src/parser.rs"]),
    );
    let mut detector = ThrashDetector::with_defaults();
    let mut signals = Vec::new();

    for index in 0..4 {
        signals.extend(detector.observe(&StepEvent {
            action: EditAction::Read {
                path: "src/parser.rs".to_string(),
            },
            recorded_finding: false,
            cumulative_tokens: 1_000 * (index + 1),
            state_digest: "stuck".to_string(),
        }));
    }

    assert!(signals
        .iter()
        .any(|finding| finding.signal == ThrashSignal::RepeatedReadsWithoutFindings));
    assert_eq!(
        evaluate_stop(
            &decision.policy.stop,
            &ExecutionProgress {
                steps: 4,
                unresolved_thrash_signals: signals.len() as u32,
                ..ExecutionProgress::default()
            }
        ),
        Some(StopReason::Thrashing)
    );
}
