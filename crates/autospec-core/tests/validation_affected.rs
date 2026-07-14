use autospec_core::validation::affected::AffectedSet;

#[test]
fn validation_affected_routes_direct_validation_sources_to_always_run() {
    let affected = AffectedSet::from_paths(["crates/autospec-core/src/validation/plan.rs"]);

    assert_eq!(
        affected.changed_paths,
        vec!["crates/autospec-core/src/validation/plan.rs"]
    );
    assert!(affected.includes_check("always-run"));
    assert_eq!(affected.rules.len(), 1);
}

#[test]
fn validation_affected_routes_autospec_run_skill_checks() {
    let affected = AffectedSet::from_paths(["skills/autospec-run/SKILL.md"]);

    assert!(affected.includes_check("skill:autospec-run"));
    assert_eq!(affected.checks(), vec!["skill:autospec-run"]);
}

#[test]
fn validation_affected_docs_only_skips_rust_lint() {
    let affected = AffectedSet::from_paths(["docs/specs/runtime-policy.md"]);

    assert!(affected.includes_check("docs"));
    assert!(!affected.includes_check("rust:lint"));
}
