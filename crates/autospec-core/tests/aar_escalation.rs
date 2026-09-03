//! AAR spec section 13: fallback, escalation and usage-aware routing.

use autospec_core::aar::classify::Capability;
use autospec_core::aar::escalation::{
    next_attempt, Attempt, EscalationContext, EscalationOutcome, EscalationPolicy, EscalationStep,
    QuotaState,
};
use autospec_core::aar::profile::{
    CapabilityScores, ModelProfile, ModelProfileRegistry, ModelRequirements, ProfileObservations,
};
use autospec_core::aar::reasoning::ReasoningBudget;
use autospec_core::aar::topology::{AgentRole, RoleAssignment, SeparationPolicy};

fn profile(model_id: &str, class: &str, provider: &str, local: bool) -> ModelProfile {
    ModelProfile {
        model_id: model_id.to_string(),
        model_version: "1".to_string(),
        quantization: "q4_k_m".to_string(),
        backend: "vllm".to_string(),
        hardware_class: "rtx4090".to_string(),
        model_class: class.to_string(),
        provider: provider.to_string(),
        context_window: 131_072,
        supports_vision: false,
        supports_web: false,
        max_concurrent_sessions: 3,
        cost_per_1k_prompt_micros: 0,
        cost_per_1k_output_micros: 0,
        is_local: local,
        scores: CapabilityScores::uniform(0.8),
        observations: ProfileObservations::default(),
        profile_version: 1,
    }
}

fn registry() -> ModelProfileRegistry {
    ModelProfileRegistry::new(
        "test-v1",
        vec![
            profile("primary", "coding-local", "inferweave", true),
            profile("alternate", "coding-local", "inferweave", true),
            profile("large", "coding-local-large", "inferweave", true),
            profile("cloud", "coding-cloud", "cloud", false),
        ],
    )
}

fn requirements() -> ModelRequirements {
    ModelRequirements {
        model_class: "coding-local".to_string(),
        required_capabilities: vec![Capability::Coding],
        minimum_capability_score: 0.5,
        minimum_context_free: 24_000,
        prefer_local: true,
        ..ModelRequirements::default()
    }
}

fn key(model_id: &str) -> String {
    format!("{model_id}@1/q4_k_m/vllm/rtx4090")
}

fn failed_attempt(step: Option<EscalationStep>, budget: ReasoningBudget) -> Attempt {
    Attempt {
        assignment: RoleAssignment::new(
            AgentRole::Implementer,
            key("primary"),
            "coding-local",
            "session-a",
        ),
        budget,
        provider: "inferweave".to_string(),
        step,
    }
}

#[test]
fn the_first_fallback_keeps_the_model_and_raises_the_budget() {
    let registry = registry();
    let requirements = requirements();
    let policy = EscalationPolicy::default();
    let separation = SeparationPolicy::default();
    let quota = QuotaState::new();
    let failed = failed_attempt(None, ReasoningBudget::Normal);
    let assignments = [failed.assignment.clone()];
    let context = EscalationContext {
        policy: &policy,
        registry: &registry,
        requirements: &requirements,
        separation_policy: &separation,
        current_assignments: &assignments,
        quota: &quota,
        attempts: &[],
    };

    match next_attempt(&context, &failed) {
        EscalationOutcome::Retry {
            step,
            assignment,
            budget,
            ..
        } => {
            assert_eq!(step, EscalationStep::SameModelLargerBudget);
            assert_eq!(assignment.model_key, key("primary"));
            assert_eq!(budget, ReasoningBudget::Complex);
        }
        other => panic!("expected a retry, got {other:?}"),
    }
}

#[test]
fn at_the_budget_ceiling_the_chain_moves_to_an_alternate_model_in_class() {
    let registry = registry();
    let requirements = requirements();
    let policy = EscalationPolicy::default();
    let separation = SeparationPolicy::default();
    let quota = QuotaState::new();
    let failed = failed_attempt(None, ReasoningBudget::Exceptional);
    let assignments = [failed.assignment.clone()];
    let context = EscalationContext {
        policy: &policy,
        registry: &registry,
        requirements: &requirements,
        separation_policy: &separation,
        current_assignments: &assignments,
        quota: &quota,
        attempts: &[],
    };

    match next_attempt(&context, &failed) {
        EscalationOutcome::Retry { step, assignment, .. } => {
            assert_eq!(step, EscalationStep::AlternateModelInClass);
            assert_eq!(assignment.model_key, key("alternate"));
        }
        other => panic!("expected a retry, got {other:?}"),
    }
}

/// Usage-aware routing: quota is checked before assignment, not after failure.
#[test]
fn a_provider_without_capacity_is_skipped_before_assignment() {
    let registry = registry();
    let requirements = requirements();
    let policy = EscalationPolicy::default();
    let separation = SeparationPolicy::default();
    let mut quota = QuotaState::new();
    quota.set("inferweave", 0);
    let failed = failed_attempt(None, ReasoningBudget::Exceptional);
    let assignments = [failed.assignment.clone()];
    let context = EscalationContext {
        policy: &policy,
        registry: &registry,
        requirements: &requirements,
        separation_policy: &separation,
        current_assignments: &assignments,
        quota: &quota,
        attempts: &[],
    };

    match next_attempt(&context, &failed) {
        EscalationOutcome::Retry { assignment, provider, .. } => {
            assert_eq!(provider, "cloud");
            assert!(assignment.model_key.contains("cloud"));
        }
        other => panic!("expected the cloud fallback, got {other:?}"),
    }
}

/// The spec's hard rule: a quota or capacity failure must not weaken
/// separation of duties.
#[test]
fn a_fallback_that_would_break_separation_is_skipped_not_taken() {
    let registry = ModelProfileRegistry::new(
        "test-v1",
        vec![
            profile("primary", "coding-local", "inferweave", true),
            // The only alternate in class is the reviewer's model.
            profile("reviewer-model", "coding-local", "inferweave", true),
        ],
    );
    let requirements = requirements();
    let policy = EscalationPolicy {
        chain: vec![
            EscalationStep::AlternateModelInClass,
            EscalationStep::HumanEscalation,
        ],
        ..EscalationPolicy::default()
    };
    let separation = SeparationPolicy::default();
    let quota = QuotaState::new();
    let failed = failed_attempt(None, ReasoningBudget::Exceptional);
    let assignments = [
        failed.assignment.clone(),
        RoleAssignment::new(
            AgentRole::Reviewer,
            key("reviewer-model"),
            "coding-local",
            "session-b",
        ),
    ];
    let context = EscalationContext {
        policy: &policy,
        registry: &registry,
        requirements: &requirements,
        separation_policy: &separation,
        current_assignments: &assignments,
        quota: &quota,
        attempts: &[],
    };

    match next_attempt(&context, &failed) {
        EscalationOutcome::Escalate { rationale } => {
            assert!(
                rationale
                    .iter()
                    .any(|reason| reason.contains("separation of duties")),
                "rationale must say separation blocked the fallback: {rationale:?}"
            );
        }
        other => panic!("expected escalation rather than a separation-breaking retry, got {other:?}"),
    }
}

#[test]
fn the_chain_climbs_to_a_higher_model_class_when_the_class_is_exhausted() {
    let registry = ModelProfileRegistry::new(
        "test-v1",
        vec![
            profile("primary", "coding-local", "inferweave", true),
            profile("large", "coding-local-large", "inferweave", true),
        ],
    );
    let requirements = requirements();
    let policy = EscalationPolicy::default();
    let separation = SeparationPolicy::default();
    let quota = QuotaState::new();
    let failed = failed_attempt(
        Some(EscalationStep::SameModelLargerBudget),
        ReasoningBudget::Exceptional,
    );
    let assignments = [failed.assignment.clone()];
    let context = EscalationContext {
        policy: &policy,
        registry: &registry,
        requirements: &requirements,
        separation_policy: &separation,
        current_assignments: &assignments,
        quota: &quota,
        attempts: &[],
    };

    match next_attempt(&context, &failed) {
        EscalationOutcome::Retry { step, assignment, .. } => {
            assert_eq!(step, EscalationStep::HigherModelClass);
            assert_eq!(assignment.model_class, "coding-local-large");
        }
        other => panic!("expected a class climb, got {other:?}"),
    }
}

#[test]
fn exhausting_max_attempts_escalates_to_a_human() {
    let registry = registry();
    let requirements = requirements();
    let policy = EscalationPolicy {
        max_attempts: 2,
        ..EscalationPolicy::default()
    };
    let separation = SeparationPolicy::default();
    let quota = QuotaState::new();
    let failed = failed_attempt(None, ReasoningBudget::Normal);
    let assignments = [failed.assignment.clone()];
    let attempts = [failed.clone(), failed.clone()];
    let context = EscalationContext {
        policy: &policy,
        registry: &registry,
        requirements: &requirements,
        separation_policy: &separation,
        current_assignments: &assignments,
        quota: &quota,
        attempts: &attempts,
    };

    match next_attempt(&context, &failed) {
        EscalationOutcome::Escalate { rationale } => {
            assert!(rationale[0].contains("max_attempts"));
        }
        other => panic!("expected escalation, got {other:?}"),
    }
}

#[test]
fn provider_exhaustion_with_no_remaining_capacity_escalates_to_a_human() {
    let registry = registry();
    let requirements = requirements();
    let policy = EscalationPolicy::default();
    let separation = SeparationPolicy::default();
    let mut quota = QuotaState::new();
    quota.set("inferweave", 0);
    quota.set("cloud", 0);
    let failed = failed_attempt(None, ReasoningBudget::Exceptional);
    let assignments = [failed.assignment.clone()];
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
fn quota_consumption_decrements_only_a_tracked_provider() {
    let mut quota = QuotaState::new();
    quota.set("cloud", 1);

    assert!(quota.has_capacity("untracked"));
    quota.consume("cloud");

    assert!(!quota.has_capacity("cloud"));
    assert!(quota.has_capacity("untracked"));
}
