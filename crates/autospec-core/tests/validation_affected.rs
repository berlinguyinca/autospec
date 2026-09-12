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

#[test]
fn validation_affected_web_only_patch_is_ungated_not_default() {
    // Regression (#3790): a TypeScript-only patch must not be reported as
    // covered by the Rust gate set. The old `global:default` fallback let a
    // change like this "pass" on the strength of the Rust suite; it must be
    // reported as ungated instead, with no rule standing in for a gate.
    let affected = AffectedSet::from_paths([
        "apps/web/app/page.tsx",
        "apps/web/client/fixtures.ts",
        "apps/web/client/server-data.ts",
        "apps/web/components/panels/ClusterDashboard.tsx",
        "apps/web/presentation/freshness.ts",
        "apps/web/presentation/gpus.ts",
        "apps/web/tests/dashboard.test.tsx",
    ]);

    assert!(affected.rules.is_empty());
    assert!(!affected.includes_check("rust:lint"));
    assert!(!affected.includes_check("global:default"));
    assert!(affected.has_ungated());
    assert_eq!(
        affected.ungated_paths, affected.changed_paths,
        "every web-only path has no declared gate covering it"
    );
}

#[test]
fn validation_affected_mixed_docs_and_ungated_patch_names_both() {
    // The InferWeave #48 patch shape: a root CHANGELOG.md plus web sources.
    // The declared docs gate covers the changelog; the web paths stay ungated
    // and the set is still not reportable as validated.
    let affected = AffectedSet::from_paths([
        "CHANGELOG.md",
        "apps/web/app/page.tsx",
        "apps/web/tests/dashboard.test.tsx",
    ]);

    assert!(affected.includes_check("docs"));
    assert!(!affected.includes_check("rust:lint"));
    assert!(affected.has_ungated());
    assert_eq!(
        affected.ungated_paths,
        vec!["apps/web/app/page.tsx", "apps/web/tests/dashboard.test.tsx"]
    );
}

#[test]
fn validation_affected_change_spanning_two_toolchains_selects_both_gate_sets() {
    // A change touching both Rust sources and docs selects both gate sets;
    // each declared gate is named, nothing is ungated.
    let affected = AffectedSet::from_paths([
        "crates/autospec-cli/src/main.rs",
        "docs/specs/gate-selection.md",
    ]);

    assert!(affected.includes_check("rust:lint"));
    assert!(affected.includes_check("docs"));
    assert!(!affected.has_ungated());
    assert!(affected.ungated_paths.is_empty());
}
