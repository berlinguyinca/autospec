//! AAR spec section 4: model capability profiles and their resolution.

use autospec_core::aar::classify::Capability;
use autospec_core::aar::profile::{
    CapabilityScores, ModelProfile, ModelProfileRegistry, ModelRequirements, ProfileObservations,
};

fn profile(model_id: &str, quantization: &str, coding: f64, context_window: u64) -> ModelProfile {
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
        scores: CapabilityScores {
            coding,
            ..CapabilityScores::uniform(0.7)
        },
        observations: ProfileObservations::default(),
        profile_version: 1,
    }
}

#[test]
fn profile_key_distinguishes_quantizations_of_one_model() {
    let quantized = profile("qwen", "q4_k_m", 0.7, 65_536);
    let full = profile("qwen", "bf16", 0.7, 65_536);

    assert_ne!(quantized.key(), full.key());
    assert!(quantized.key().contains("q4_k_m"));
}

#[test]
fn production_outcomes_adjust_but_do_not_replace_the_benchmark() {
    let mut candidate = profile("qwen", "q4_k_m", 0.80, 65_536);
    candidate.observations = ProfileObservations {
        tasks: 5,
        successes: 0,
    };

    let blended = candidate.blended_score(Capability::Coding);

    // Five failures must move the score, but not all the way to zero: the
    // benchmark still carries most of the weight at this sample size.
    assert!(blended < 0.80, "production outcomes must move the score");
    assert!(
        blended > 0.55,
        "five samples must not erase a benchmark, got {blended}"
    );
}

#[test]
fn a_large_production_sample_dominates_the_benchmark() {
    let mut candidate = profile("qwen", "q4_k_m", 0.9, 65_536);
    candidate.observations = ProfileObservations {
        tasks: 500,
        successes: 250,
    };

    let blended = candidate.blended_score(Capability::Coding);

    assert!(
        (blended - 0.5).abs() < 0.05,
        "500 samples at 50% should pull the score near 0.5, got {blended}"
    );
}

#[test]
fn a_profile_with_no_observations_keeps_its_benchmark_score() {
    let candidate = profile("qwen", "q4_k_m", 0.73, 65_536);

    assert_eq!(candidate.blended_score(Capability::Coding), 0.73);
}

#[test]
fn resolution_rejects_a_model_whose_window_cannot_hold_the_request() {
    let registry = ModelProfileRegistry::new(
        "test-v1",
        vec![
            profile("small", "q4_k_m", 0.9, 8_000),
            profile("large", "q4_k_m", 0.7, 128_000),
        ],
    );
    let requirements = ModelRequirements {
        minimum_context_free: 32_000,
        required_capabilities: vec![Capability::Coding],
        minimum_capability_score: 0.5,
        ..ModelRequirements::default()
    };

    let resolution = registry.resolve(&requirements);

    assert_eq!(resolution.matches.len(), 1);
    assert_eq!(resolution.best().map(|p| p.model_id.clone()).as_deref(), Some("large"));
    assert!(resolution.rejections[0].reason.contains("context window"));
}

#[test]
fn resolution_rejects_a_model_below_the_minimum_capability_score() {
    let registry = ModelProfileRegistry::new("test-v1", vec![profile("weak", "q4_k_m", 0.2, 65_536)]);
    let requirements = ModelRequirements {
        required_capabilities: vec![Capability::Coding],
        minimum_capability_score: 0.6,
        ..ModelRequirements::default()
    };

    let resolution = registry.resolve(&requirements);

    assert!(resolution.matches.is_empty());
    assert!(resolution.rejections[0].reason.contains("coding score"));
}

#[test]
fn vision_and_web_requirements_are_hard_filters() {
    let registry = ModelProfileRegistry::starter();

    let vision = registry.resolve(&ModelRequirements {
        model_class: "coding-local".to_string(),
        requires_vision: true,
        required_capabilities: vec![Capability::Coding],
        minimum_capability_score: 0.4,
        minimum_context_free: 1_000,
        ..ModelRequirements::default()
    });

    assert!(
        vision.matches.is_empty(),
        "no local starter profile supports vision"
    );
    let local_rejections: Vec<&str> = vision
        .rejections
        .iter()
        .filter(|rejection| rejection.key.contains("qwen"))
        .map(|rejection| rejection.reason.as_str())
        .collect();
    assert_eq!(local_rejections.len(), 2);
    assert!(local_rejections
        .iter()
        .all(|reason| reason.contains("vision")));
}

#[test]
fn the_allowlist_excludes_every_other_model() {
    let registry = ModelProfileRegistry::new(
        "test-v1",
        vec![
            profile("wanted", "q4_k_m", 0.7, 65_536),
            profile("unwanted", "q4_k_m", 0.95, 65_536),
        ],
    );
    let requirements = ModelRequirements {
        model_allowlist: vec!["wanted".to_string()],
        required_capabilities: vec![Capability::Coding],
        minimum_capability_score: 0.5,
        minimum_context_free: 1_000,
        ..ModelRequirements::default()
    };

    let resolution = registry.resolve(&requirements);

    assert_eq!(resolution.matches.len(), 1);
    assert_eq!(resolution.matches[0].profile.model_id, "wanted");
}

#[test]
fn recording_an_outcome_updates_the_profile_observations() {
    let mut registry = ModelProfileRegistry::new("test-v1", vec![profile("qwen", "q4_k_m", 0.8, 65_536)]);
    let key = registry.profiles()[0].key();

    registry.record_outcome(&key, true).expect("profile exists");
    registry.record_outcome(&key, false).expect("profile exists");

    let observations = registry.get(&key).expect("profile exists").observations;
    assert_eq!(observations.tasks, 2);
    assert_eq!(observations.successes, 1);
    assert_eq!(observations.success_rate(), Some(0.5));
}

#[test]
fn recording_an_outcome_for_an_unknown_profile_is_an_error() {
    let mut registry = ModelProfileRegistry::new("test-v1", Vec::new());

    let error = registry.record_outcome("missing", true).unwrap_err();

    assert!(error.contains("unknown model profile"));
}

#[test]
fn local_profiles_are_preferred_when_the_requirements_ask_for_it() {
    let mut cloud = profile("cloud", "none", 0.99, 200_000);
    cloud.is_local = false;
    let registry = ModelProfileRegistry::new("test-v1", vec![cloud, profile("local", "q4_k_m", 0.7, 65_536)]);
    let requirements = ModelRequirements {
        prefer_local: true,
        required_capabilities: vec![Capability::Coding],
        minimum_capability_score: 0.5,
        minimum_context_free: 1_000,
        ..ModelRequirements::default()
    };

    let resolution = registry.resolve(&requirements);

    assert_eq!(
        resolution.best().map(|profile| profile.model_id.clone()).as_deref(),
        Some("local"),
        "prefer_local must beat a higher capability score"
    );
}

#[test]
fn weakest_capability_reports_the_binding_constraint() {
    let mut candidate = profile("qwen", "q4_k_m", 0.9, 65_536);
    candidate.scores.vision = 0.1;

    let (capability, score) = candidate
        .weakest_capability(&[Capability::Coding, Capability::Vision])
        .expect("capabilities requested");

    assert_eq!(capability, Capability::Vision);
    assert_eq!(score, 0.1);
}

#[test]
fn cost_estimation_scales_with_token_counts() {
    let mut candidate = profile("cloud", "none", 0.9, 200_000);
    candidate.cost_per_1k_prompt_micros = 3_000;
    candidate.cost_per_1k_output_micros = 15_000;

    assert_eq!(candidate.estimated_cost_micros(10_000, 2_000), 30_000 + 30_000);
}
