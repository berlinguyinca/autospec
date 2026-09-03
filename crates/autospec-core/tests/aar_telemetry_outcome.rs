//! AAR spec sections 14 and 15: telemetry records and outcome optimization.

use autospec_core::aar::classify::TaskClass;
use autospec_core::aar::outcome::{
    apply_policy_override, recommend, score_outcome, ExecutionOutcome, HardPolicy, OutcomeSignals,
    ProfileStats, QualityThreshold,
};
use autospec_core::aar::telemetry::{
    ExecutionTelemetry, FailureCategory, ReviewOutcome, REDACTED, TELEMETRY_SCHEMA_VERSION,
};

fn telemetry() -> ExecutionTelemetry {
    ExecutionTelemetry {
        task_id: "task-1".to_string(),
        spec_id: "spec-1".to_string(),
        repository: "berlinguyinca/autospec".to_string(),
        base_revision: "af8591e".to_string(),
        role: "implementer".to_string(),
        harness: "pi".to_string(),
        model_id: "qwen3.8-27b".to_string(),
        node_id: "node-1".to_string(),
        prompt_tokens: 10_000,
        cached_prompt_tokens: 8_000,
        new_prefill_tokens: 2_000,
        output_tokens: 900,
        tests_run: 12,
        tests_passed: 12,
        review_outcome: ReviewOutcome::Approved,
        success: true,
        ..ExecutionTelemetry::default()
    }
}

#[test]
fn a_record_carries_its_schema_version() {
    assert_eq!(telemetry().schema_version, TELEMETRY_SCHEMA_VERSION);
}

/// Section 11 requires total, cached and newly-prefilled tokens to be
/// distinguishable; a record where they cannot add up is rejected.
#[test]
fn token_accounting_must_add_up() {
    assert!(telemetry().validate().is_ok());

    let inconsistent = ExecutionTelemetry {
        cached_prompt_tokens: 8_000,
        new_prefill_tokens: 5_000,
        ..telemetry()
    };

    assert!(inconsistent
        .validate()
        .unwrap_err()
        .contains("!= prompt_tokens"));
}

#[test]
fn the_cache_hit_rate_is_derivable_from_the_record() {
    assert_eq!(telemetry().cache_hit_rate(), 0.8);
    assert_eq!(
        ExecutionTelemetry::default().cache_hit_rate(),
        0.0,
        "an empty prompt must not divide by zero"
    );
}

#[test]
fn a_successful_record_may_not_carry_a_failure_category() {
    let contradictory = ExecutionTelemetry {
        failure_category: FailureCategory::TestsFailed,
        ..telemetry()
    };

    assert!(contradictory
        .validate()
        .unwrap_err()
        .contains("failure category"));
}

#[test]
fn a_failed_record_must_carry_a_failure_category() {
    let unexplained = ExecutionTelemetry {
        success: false,
        ..telemetry()
    };

    assert!(unexplained
        .validate()
        .unwrap_err()
        .contains("must carry a failure category"));
}

#[test]
fn more_tests_passing_than_ran_is_rejected() {
    let impossible = ExecutionTelemetry {
        tests_run: 3,
        tests_passed: 5,
        ..telemetry()
    };

    assert!(impossible.validate().is_err());
}

#[test]
fn free_text_is_redacted_before_export() {
    let record = ExecutionTelemetry {
        success: false,
        failure_category: FailureCategory::TestsFailed,
        failure_detail: "panicked at src/secret_path.rs: token=abc".to_string(),
        ..telemetry()
    };

    let redacted = record.redacted();

    assert_eq!(redacted.failure_detail, REDACTED);
    assert!(!redacted.to_json_line().expect("serializes").contains("abc"));
}

#[test]
fn a_record_serializes_to_one_jsonl_line() {
    let line = telemetry().to_json_line().expect("serializes");

    assert!(line.ends_with('\n'));
    assert_eq!(line.matches('\n').count(), 1);
    assert!(line.contains("\"model_id\":\"qwen3.8-27b\""));
    assert!(line.contains("\"review_outcome\":\"approved\""));
}

#[test]
fn a_clean_success_scores_near_the_top() {
    let score = score_outcome(
        &ExecutionOutcome {
            success: true,
            tests_passed: true,
            review_passed: true,
            prompt_tokens: 10_000,
            cached_prompt_tokens: 9_000,
            ..ExecutionOutcome::default()
        },
        &OutcomeSignals {
            acceptance_criteria_total: 4,
            acceptance_criteria_met: 4,
            ..OutcomeSignals::default()
        },
    );

    assert_eq!(score.quality, 1.0);
    assert!(score.efficiency > 0.9);
}

#[test]
fn regressions_and_human_corrections_reduce_quality() {
    let clean = score_outcome(
        &ExecutionOutcome {
            success: true,
            tests_passed: true,
            review_passed: true,
            ..ExecutionOutcome::default()
        },
        &OutcomeSignals::default(),
    );
    let messy = score_outcome(
        &ExecutionOutcome {
            success: true,
            tests_passed: true,
            review_passed: true,
            ..ExecutionOutcome::default()
        },
        &OutcomeSignals {
            regressions_introduced: 1,
            human_corrections: 2,
            ..OutcomeSignals::default()
        },
    );

    assert!(messy.quality < clean.quality);
    assert!(messy
        .reasons
        .iter()
        .any(|reason| reason.contains("regression")));
}

#[test]
fn retries_reduce_efficiency() {
    let score = score_outcome(
        &ExecutionOutcome {
            success: true,
            retries: 3,
            ..ExecutionOutcome::default()
        },
        &OutcomeSignals::default(),
    );

    assert!(score.efficiency < 0.6);
}

/// The Wilson lower bound is what stops a lucky three-run profile from
/// outranking a hundred-run one.
#[test]
fn a_small_perfect_sample_ranks_below_a_large_good_one() {
    let lucky = ProfileStats {
        profile_key: "lucky".to_string(),
        task_class: TaskClass::Bugfix,
        reasoning_budget: "normal".to_string(),
        samples: 3,
        successes: 3,
        mean_latency_ms: 1_000,
        mean_cost_micros: 0,
    };
    let proven = ProfileStats {
        profile_key: "proven".to_string(),
        samples: 100,
        successes: 92,
        ..lucky.clone()
    };

    assert!(lucky.success_lower_bound() < proven.success_lower_bound());
}

#[test]
fn the_cheapest_profile_meeting_the_threshold_is_recommended() {
    let base = ProfileStats {
        profile_key: String::new(),
        task_class: TaskClass::Bugfix,
        reasoning_budget: "normal".to_string(),
        samples: 100,
        successes: 92,
        mean_latency_ms: 5_000,
        mean_cost_micros: 0,
    };
    let candidates = [
        ProfileStats {
            profile_key: "expensive-cloud".to_string(),
            successes: 99,
            mean_cost_micros: 50_000,
            mean_latency_ms: 2_000,
            ..base.clone()
        },
        ProfileStats {
            profile_key: "cheap-local".to_string(),
            mean_cost_micros: 0,
            ..base.clone()
        },
    ];

    let recommendation = recommend(&candidates, &QualityThreshold::default());

    assert_eq!(recommendation.profile_key.as_deref(), Some("cheap-local"));
    assert!(recommendation
        .rationale
        .iter()
        .any(|reason| reason.contains("cheapest configuration")));
}

#[test]
fn an_under_sampled_profile_is_not_recommended() {
    let candidates = [ProfileStats {
        profile_key: "new".to_string(),
        task_class: TaskClass::Bugfix,
        reasoning_budget: "normal".to_string(),
        samples: 2,
        successes: 2,
        mean_latency_ms: 100,
        mean_cost_micros: 0,
    }];

    let recommendation = recommend(&candidates, &QualityThreshold::default());

    assert_eq!(recommendation.profile_key, None);
    assert!(recommendation.rationale[0].contains("below minimum"));
}

#[test]
fn a_profile_below_the_quality_bar_is_not_recommended_however_cheap() {
    let candidates = [ProfileStats {
        profile_key: "free-but-bad".to_string(),
        task_class: TaskClass::Bugfix,
        reasoning_budget: "tiny".to_string(),
        samples: 100,
        successes: 40,
        mean_latency_ms: 10,
        mean_cost_micros: 0,
    }];

    let recommendation = recommend(&candidates, &QualityThreshold::default());

    assert_eq!(recommendation.profile_key, None);
}

/// Hard policy always overrides a learned recommendation.
#[test]
fn a_denied_profile_is_dropped_from_the_recommendation() {
    let candidates = [ProfileStats {
        profile_key: "denied".to_string(),
        task_class: TaskClass::Bugfix,
        reasoning_budget: "normal".to_string(),
        samples: 100,
        successes: 95,
        mean_latency_ms: 100,
        mean_cost_micros: 0,
    }];
    let recommendation = recommend(&candidates, &QualityThreshold::default());

    let overridden = apply_policy_override(
        recommendation,
        &HardPolicy {
            denied_profiles: vec!["denied".to_string()],
            ..HardPolicy::default()
        },
    );

    assert_eq!(overridden.profile_key, None);
    assert!(overridden.overridden_by_policy);
}

#[test]
fn a_pinned_profile_replaces_the_learned_recommendation() {
    let candidates = [ProfileStats {
        profile_key: "learned".to_string(),
        task_class: TaskClass::Bugfix,
        reasoning_budget: "normal".to_string(),
        samples: 100,
        successes: 95,
        mean_latency_ms: 100,
        mean_cost_micros: 0,
    }];
    let recommendation = recommend(&candidates, &QualityThreshold::default());

    let overridden = apply_policy_override(
        recommendation,
        &HardPolicy {
            pinned_profile: Some("operator-choice".to_string()),
            pinned_reasoning_budget: Some("complex".to_string()),
            ..HardPolicy::default()
        },
    );

    assert_eq!(overridden.profile_key.as_deref(), Some("operator-choice"));
    assert_eq!(overridden.reasoning_budget.as_deref(), Some("complex"));
    assert!(overridden.overridden_by_policy);
}

#[test]
fn a_policy_that_agrees_with_the_recommendation_is_not_marked_as_an_override() {
    let candidates = [ProfileStats {
        profile_key: "agreed".to_string(),
        task_class: TaskClass::Bugfix,
        reasoning_budget: "normal".to_string(),
        samples: 100,
        successes: 95,
        mean_latency_ms: 100,
        mean_cost_micros: 0,
    }];
    let recommendation = recommend(&candidates, &QualityThreshold::default());

    let overridden = apply_policy_override(
        recommendation,
        &HardPolicy {
            pinned_profile: Some("agreed".to_string()),
            pinned_reasoning_budget: Some("normal".to_string()),
            ..HardPolicy::default()
        },
    );

    assert!(!overridden.overridden_by_policy);
}
