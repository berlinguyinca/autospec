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
