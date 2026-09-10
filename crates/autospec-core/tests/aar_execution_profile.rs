//! Model-agnostic execution profiles with enforceable budgets (issue #3317).

use autospec_core::aar::execution_profile::{default_registry, ExecutionBudgets, ExecutionProfile};

fn profile(name: &str, family: &str, model: &str, budgets: ExecutionBudgets) -> ExecutionProfile {
    ExecutionProfile {
        name: name.to_string(),
        family: family.to_string(),
        model: model.to_string(),
        budgets,
    }
}

fn budgets(reasoning_tokens: u32, hard_context_limit: u32, max_turns: u32) -> ExecutionBudgets {
    ExecutionBudgets {
        reasoning_tokens,
        hard_context_limit,
        max_turns,
        max_repairs: 1,
        wall_clock_ms: 1_800_000,
    }
}

#[test]
fn execution_profile_fast_enforces_six_turns_and_24000_context() {
    let registry = default_registry();
    let fast = registry
        .get("qwen38-fast")
        .expect("built-in registry must define qwen38-fast");

    fast.validate()
        .expect("qwen38-fast must pass schema validation");
    assert_eq!(
        fast.budgets.max_turns, 6,
        "qwen38-fast must enforce 6 turns"
    );
    assert_eq!(
        fast.budgets.hard_context_limit, 24_000,
        "qwen38-fast must enforce a hard context limit of 24000"
    );
    assert_eq!(fast.tier(), "fast");
}

#[test]
fn execution_profile_coding_enforces_ten_turns_and_32768_context() {
    let registry = default_registry();
    let coding = registry
        .get("qwen38-coding")
        .expect("built-in registry must define qwen38-coding");

    coding
        .validate()
        .expect("qwen38-coding must pass schema validation");
    assert_eq!(
        coding.budgets.max_turns, 10,
        "qwen38-coding must enforce 10 turns"
    );
    assert_eq!(
        coding.budgets.hard_context_limit, 32_768,
        "qwen38-coding must enforce a hard context limit of 32768"
    );
    assert_eq!(coding.tier(), "coding");
}

#[test]
fn execution_profile_deep_enforces_six_turns_and_65536_context() {
    let registry = default_registry();
    let deep = registry
        .get("qwen38-deep")
        .expect("built-in registry must define qwen38-deep");

    deep.validate()
        .expect("qwen38-deep must pass schema validation");
    assert_eq!(
        deep.budgets.max_turns, 6,
        "qwen38-deep must enforce 6 turns"
    );
    assert_eq!(
        deep.budgets.hard_context_limit, 65_536,
        "qwen38-deep must enforce a hard context limit of 65536"
    );
    assert_eq!(deep.tier(), "deep");
}

#[test]
fn execution_profile_maps_qwen38_tiers_to_qwen38_27b_without_provider_details() {
    let registry = default_registry();

    for name in ["qwen38-fast", "qwen38-coding", "qwen38-deep"] {
        let profile = registry.get(name).unwrap();
        assert_eq!(profile.family, "qwen3.8");
        assert_eq!(profile.model, "qwen3.8-27b");
    }
}

#[test]
fn execution_profile_family_mismatch_fails_validation() {
    // A profile named for the qwen3.8 family but declared with any other
    // family must fail schema validation, whatever the model is.
    let mismatched = profile(
        "qwen38-fast",
        "qwen3.9",
        "qwen3.9-27b",
        budgets(512, 24_000, 6),
    );

    let error = mismatched
        .validate()
        .expect_err("family mismatch must fail");
    assert!(
        error.contains("qwen3.9"),
        "the error must name the offending family, got: {error}"
    );

    let claude_family = profile(
        "qwen38-fast",
        "claude4",
        "qwen3.8-27b",
        budgets(512, 24_000, 6),
    );
    assert!(claude_family.validate().is_err());
}

#[test]
fn execution_profile_unknown_tier_fails_validation() {
    let unknown = profile(
        "qwen38-turbo",
        "qwen3.8",
        "qwen3.8-27b",
        budgets(512, 24_000, 6),
    );
    assert!(unknown.validate().is_err());
}

#[test]
fn execution_profile_model_outside_family_fails_validation() {
    let wrong_model = profile(
        "qwen38-fast",
        "qwen3.8",
        "qwen3.9-27b",
        budgets(512, 24_000, 6),
    );
    assert!(wrong_model.validate().is_err());
}

#[test]
fn execution_profile_zero_budget_fails_validation() {
    let zero_context = profile("qwen38-fast", "qwen3.8", "qwen3.8-27b", budgets(512, 0, 6));
    assert!(zero_context.validate().is_err());

    let zero_turns = profile(
        "qwen38-fast",
        "qwen3.8",
        "qwen3.8-27b",
        budgets(512, 24_000, 0),
    );
    assert!(zero_turns.validate().is_err());
}

#[test]
fn execution_profile_registry_rejects_duplicate_names() {
    let fast = profile(
        "qwen38-fast",
        "qwen3.8",
        "qwen3.8-27b",
        budgets(512, 24_000, 6),
    );
    let registry = autospec_core::aar::execution_profile::ExecutionProfileRegistry::new(
        "execution-profiles-v1",
        vec![fast.clone(), fast],
    );

    let error = registry.validate().expect_err("duplicate names must fail");
    assert!(error.contains("qwen38-fast"));
}

#[test]
fn execution_profile_example_config_validates() {
    let raw = include_str!("../../../tests/fixtures/execution-profiles/example-config.json");
    let registry: autospec_core::aar::execution_profile::ExecutionProfileRegistry =
        serde_json::from_str(raw).expect("example config must deserialize");

    registry
        .validate()
        .expect("example config must pass schema validation");

    let fast = registry.get("qwen38-fast").unwrap();
    assert_eq!(fast.budgets.max_turns, 6);
    assert_eq!(fast.budgets.hard_context_limit, 24_000);

    let coding = registry.get("qwen38-coding").unwrap();
    assert_eq!(coding.budgets.max_turns, 10);
    assert_eq!(coding.budgets.hard_context_limit, 32_768);

    let deep = registry.get("qwen38-deep").unwrap();
    assert_eq!(deep.budgets.max_turns, 6);
    assert_eq!(deep.budgets.hard_context_limit, 65_536);
}

#[test]
fn execution_profile_session_metadata_matches_fixture() {
    let registry = default_registry();
    let fast = registry
        .get("qwen38-fast")
        .expect("built-in registry must define qwen38-fast");

    let metadata = serde_json::to_value(fast.session_metadata()).unwrap();
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/execution-profiles/qwen38-fast-session-metadata.json"
    ))
    .expect("fixture must be valid JSON");

    assert_eq!(
        metadata, fixture,
        "the effective profile must be emitted to session metadata unchanged"
    );
}
