use autospec_core::autonomous::blast_radius::{
    classify_paths, default_legacy_registry, parse_fenced_surfaces,
};

#[test]
fn blast_radius_classifies_autospec_policy_config_as_fenced_from_config() {
    let registry = parse_fenced_surfaces(
        r#"
fenced_surfaces:
  - id: autospec-policy-config
    severity: fenced
    reason: AutoSpec policy config edits can change autonomous safety policy.
    paths:
      - ".autospec/**"
"#,
    )
    .expect("fenced surface registry parses");

    let classification = classify_paths([".autospec/autospec.yml"], &registry);

    assert_eq!(classification.decision, "quarantine");
    assert_eq!(classification.reason.as_deref(), Some("fenced_surface"));
    assert_eq!(classification.label, "blast:fenced");
    assert!(classification.fenced);
    assert_eq!(classification.paths, [".autospec/autospec.yml"]);
    assert_eq!(classification.fenced_matches.len(), 1);
    assert_eq!(
        classification.fenced_matches[0].surface,
        "autospec-policy-config"
    );
    assert_eq!(classification.fenced_matches[0].pattern, ".autospec/**");
}

#[test]
fn blast_radius_legacy_fallback_also_fences_autospec_policy_config() {
    let registry = default_legacy_registry();

    let classification = classify_paths([".autospec/autospec.yml"], &registry);

    assert_eq!(classification.decision, "quarantine");
    assert_eq!(classification.label, "blast:fenced");
    assert!(classification.fenced);
    assert_eq!(classification.fenced_matches[0].pattern, ".autospec/**");
}

/// The `public-api-contracts` fence must protect real API surface without
/// quarantining every Rust change. This repo keeps all source under `crates/`,
/// so a blanket `crates/**` pattern fenced test-only diffs too and stalled the
/// autonomous drain. Guards the shipped config, not just the matcher.
#[test]
fn public_api_fence_covers_api_surface_but_not_crate_test_files() {
    let config = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.autospec/autospec.yml"),
    )
    .expect("repo .autospec/autospec.yml is readable");
    let registry = parse_fenced_surfaces(&config).expect("shipped registry parses");

    let fenced_by_public_api = |path: &str| {
        classify_paths([path], &registry)
            .fenced_matches
            .iter()
            .any(|m| m.surface == "public-api-contracts")
    };

    // Real public API surface stays fenced.
    for path in ["crates/autospec-core/src/lib.rs", "Cargo.toml", "Cargo.lock"] {
        assert!(
            fenced_by_public_api(path),
            "{path} must remain fenced by public-api-contracts"
        );
    }

    // Test-only files carry no downstream API contract.
    for path in [
        "crates/autospec-cli/tests/autonomous_conductor_commands.rs",
        "crates/autospec-cli/src/commands/autonomous/executor_bridge/tests/attempt_generation.rs",
    ] {
        assert!(
            !fenced_by_public_api(path),
            "{path} must NOT be fenced by public-api-contracts"
        );
    }
}
