//! AAR spec section 5: adaptive reasoning budgets and sampling profiles.

use autospec_core::aar::classify::{classify, ClassificationInput, Complexity, Risk, TaskClass};
use autospec_core::aar::reasoning::{
    select_reasoning, ReasoningBudget, ReasoningContext, ReasoningHistory, ReasoningLimits,
    SamplingProfile, SamplingRegistry, MIN_SAMPLES_FOR_BUDGET_COMPARISON,
};

fn classification(complexity: Complexity, risk: Risk, class: TaskClass) -> autospec_core::aar::classify::TaskClassification {
    let mut classification = classify(&ClassificationInput::new("placeholder", "placeholder"));
    classification.complexity = complexity;
    classification.risk = risk;
    classification.task_class = class;
    classification
}

#[test]
fn spec_default_budgets_are_the_documented_token_counts() {
    let limits = ReasoningLimits::default();

    assert_eq!(limits.tokens(ReasoningBudget::Tiny), 512);
    assert_eq!(limits.tokens(ReasoningBudget::Normal), 2_048);
    assert_eq!(limits.tokens(ReasoningBudget::Complex), 4_096);
    assert_eq!(limits.tokens(ReasoningBudget::Exceptional), 8_192);
}

#[test]
fn a_non_monotonic_limit_ladder_is_rejected() {
    let limits = ReasoningLimits {
        tiny: 4_096,
        normal: 2_048,
        complex: 4_096,
        exceptional: 8_192,
    };

    assert!(limits.validate().is_err());
}

#[test]
fn trivial_work_gets_the_tiny_budget() {
    let selection = select_reasoning(
        &classification(Complexity::Trivial, Risk::Low, TaskClass::Docs),
        &ReasoningContext::default(),
        &ReasoningHistory::default(),
        &ReasoningLimits::default(),
    );

    assert_eq!(selection.budget, ReasoningBudget::Tiny);
    assert_eq!(selection.tokens, 512);
}

#[test]
fn critical_risk_raises_the_budget_to_complex() {
    let selection = select_reasoning(
        &classification(Complexity::Low, Risk::Critical, TaskClass::Bugfix),
        &ReasoningContext::default(),
        &ReasoningHistory::default(),
        &ReasoningLimits::default(),
    );

    assert_eq!(selection.budget, ReasoningBudget::Complex);
    assert!(selection
        .reasons
        .iter()
        .any(|reason| reason.contains("critical risk")));
}

#[test]
fn each_retry_raises_the_budget_one_step() {
    let selection = select_reasoning(
        &classification(Complexity::Low, Risk::Low, TaskClass::Bugfix),
        &ReasoningContext {
            retries: 2,
            ..ReasoningContext::default()
        },
        &ReasoningHistory::default(),
        &ReasoningLimits::default(),
    );

    assert_eq!(selection.budget, ReasoningBudget::Exceptional);
}

#[test]
fn reviewer_rejection_raises_the_budget() {
    let base = select_reasoning(
        &classification(Complexity::Low, Risk::Low, TaskClass::Feature),
        &ReasoningContext::default(),
        &ReasoningHistory::default(),
        &ReasoningLimits::default(),
    );
    let after_rework = select_reasoning(
        &classification(Complexity::Low, Risk::Low, TaskClass::Feature),
        &ReasoningContext {
            reviewer_rejected: true,
            ..ReasoningContext::default()
        },
        &ReasoningHistory::default(),
        &ReasoningLimits::default(),
    );

    assert!(after_rework.budget > base.budget);
}

/// The spec's negative rule: AAR must not assume more reasoning is better.
#[test]
fn a_larger_budget_without_measured_benefit_is_lowered() {
    let mut history = ReasoningHistory::default();
    for _ in 0..MIN_SAMPLES_FOR_BUDGET_COMPARISON {
        history.record(ReasoningBudget::Normal, true);
        history.record(ReasoningBudget::Complex, true);
    }

    let selection = select_reasoning(
        &classification(Complexity::High, Risk::Low, TaskClass::Feature),
        &ReasoningContext::default(),
        &history,
        &ReasoningLimits::default(),
    );

    assert_eq!(
        selection.budget,
        ReasoningBudget::Normal,
        "equal measured success must select the cheaper budget: {:?}",
        selection.reasons
    );
    assert!(selection
        .reasons
        .iter()
        .any(|reason| reason.contains("not materially better")));
}

#[test]
fn a_larger_budget_with_measured_benefit_is_kept() {
    let mut history = ReasoningHistory::default();
    for index in 0..20 {
        history.record(ReasoningBudget::Normal, index < 8);
        history.record(ReasoningBudget::Complex, true);
    }

    let selection = select_reasoning(
        &classification(Complexity::High, Risk::Low, TaskClass::Feature),
        &ReasoningContext::default(),
        &history,
        &ReasoningLimits::default(),
    );

    assert_eq!(selection.budget, ReasoningBudget::Complex);
}

/// Under-sampled evidence must not move the budget in either direction.
#[test]
fn a_handful_of_samples_does_not_count_as_evidence() {
    let mut history = ReasoningHistory::default();
    for _ in 0..3 {
        history.record(ReasoningBudget::Tiny, true);
    }

    assert!(!history.materially_better(ReasoningBudget::Complex, ReasoningBudget::Tiny));
    assert_eq!(history.cheapest_adequate(), None);
}

#[test]
fn low_classification_confidence_raises_the_budget_one_step() {
    let raised = select_reasoning(
        &classification(Complexity::Low, Risk::Low, TaskClass::Feature),
        &ReasoningContext {
            low_confidence: true,
            ..ReasoningContext::default()
        },
        &ReasoningHistory::default(),
        &ReasoningLimits::default(),
    );

    assert_eq!(raised.budget, ReasoningBudget::Complex);
}

/// A cost or latency preference may lower an unproven budget, but only where
/// the work is neither complex nor high-risk: cheapness never overrides the
/// reasons the budget was raised in the first place.
#[test]
fn a_cost_preference_lowers_an_unproven_budget_on_ordinary_work() {
    let lowered = select_reasoning(
        &classification(Complexity::High, Risk::Low, TaskClass::Feature),
        &ReasoningContext::default(),
        &ReasoningHistory::default(),
        &ReasoningLimits::default(),
    );
    assert_eq!(lowered.budget, ReasoningBudget::Complex);

    let cost_sensitive = select_reasoning(
        &classification(Complexity::Medium, Risk::Low, TaskClass::Feature),
        &ReasoningContext {
            cost_sensitive: true,
            ..ReasoningContext::default()
        },
        &ReasoningHistory::default(),
        &ReasoningLimits::default(),
    );

    assert_eq!(cost_sensitive.budget, ReasoningBudget::Normal);
}

#[test]
fn a_cost_preference_does_not_lower_the_budget_on_high_risk_work() {
    let selection = select_reasoning(
        &classification(Complexity::Medium, Risk::Critical, TaskClass::Feature),
        &ReasoningContext {
            cost_sensitive: true,
            latency_sensitive: true,
            ..ReasoningContext::default()
        },
        &ReasoningHistory::default(),
        &ReasoningLimits::default(),
    );

    assert_eq!(selection.budget, ReasoningBudget::Complex);
}

#[test]
fn budget_escalation_stops_at_the_ceiling() {
    assert_eq!(
        ReasoningBudget::Complex.escalate(),
        Some(ReasoningBudget::Exceptional)
    );
    assert_eq!(ReasoningBudget::Exceptional.escalate(), None);
}

#[test]
fn sampling_profiles_validate_their_ranges() {
    assert!(SamplingProfile::qwen_thinking().validate().is_ok());
    assert!(SamplingProfile::qwen_instruct().validate().is_ok());

    let invalid = SamplingProfile {
        temperature: 5.0,
        ..SamplingProfile::qwen_thinking()
    };
    assert!(invalid.validate().is_err());
}

#[test]
fn sampling_profiles_are_versioned_by_identity() {
    let profile = SamplingProfile::qwen_thinking();

    assert_eq!(profile.identity(), "qwen-thinking@v1");
}

#[test]
fn thinking_profiles_serve_budgets_above_tiny() {
    let registry = SamplingRegistry::default_registry();

    assert_eq!(
        registry.for_budget(ReasoningBudget::Tiny).map(|p| p.name.as_str()),
        Some("qwen-instruct")
    );
    assert_eq!(
        registry.for_budget(ReasoningBudget::Complex).map(|p| p.name.as_str()),
        Some("qwen-thinking")
    );
}
