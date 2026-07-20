use std::collections::BTreeSet;

use autospec_core::autonomous::config::AutonomousConfig;

#[test]
fn parses_the_supported_main_health_configuration() {
    let config = AutonomousConfig::parse(
        r#"
# Repository-owned policy
main_health:
  branch: master_ai
  ignore_checks:
    - Unit Tests
    - "Lint"
"#,
    )
    .expect("supported configuration parses");

    assert_eq!(config.main_health.branch.as_deref(), Some("master_ai"));
    assert_eq!(
        config.main_health.ignore_checks,
        BTreeSet::from(["Lint".to_string(), "Unit Tests".to_string()])
    );
}

#[test]
fn absent_or_unrelated_policy_preserves_default_main_health_configuration() {
    let absent = AutonomousConfig::parse("").expect("empty configuration parses");
    let unrelated = AutonomousConfig::parse(
        r#"
growth:
  enabled: true
  main_health:
    enabled: true
another_policy:
  values:
    - ignored
"#,
    )
    .expect("unrelated configuration is ignored");

    assert_eq!(absent, AutonomousConfig::default());
    assert_eq!(unrelated, AutonomousConfig::default());
    assert!(absent.tier4.sources.is_empty());
    assert!(unrelated.tier4.sources.is_empty());
}

#[test]
fn rejects_invalid_relevant_main_health_shapes() {
    for (name, source) in [
        (
            "duplicate branch",
            "main_health:\n  branch: main\n  branch: trunk\n",
        ),
        (
            "duplicate ignored checks",
            "main_health:\n  ignore_checks:\n    - ci\n  ignore_checks:\n    - lint\n",
        ),
        (
            "scalar ignored checks",
            "main_health:\n  ignore_checks: ci\n",
        ),
        ("empty branch", "main_health:\n  branch:   \n"),
        (
            "empty ignored check",
            "main_health:\n  ignore_checks:\n    - \n",
        ),
        (
            "inline ignored checks",
            "main_health:\n  ignore_checks: [ci]\n",
        ),
        (
            "nested ignored check",
            "main_health:\n  ignore_checks:\n    name: ci\n",
        ),
        (
            "unknown relevant field",
            "main_health:\n  regex_ignore: ci.*\n",
        ),
        ("malformed indentation", "main_health:\n    branch: main\n"),
        (
            "indented main health block",
            "  main_health:\n    branch: main\n",
        ),
        (
            "nested list entry",
            "main_health:\n  ignore_checks:\n    - - Unit Tests\n",
        ),
        (
            "mapped list entry",
            "main_health:\n  ignore_checks:\n    - name: Unit Tests\n",
        ),
        (
            "mapped list entry with terminal separator",
            "main_health:\n  ignore_checks:\n    - name:\n",
        ),
        ("nested branch value", "main_health:\n  branch: - nested\n"),
    ] {
        let error = AutonomousConfig::parse(source).expect_err(&format!("{name} must fail closed"));
        assert!(
            !error.is_empty(),
            "{name} must produce a useful parse diagnostic"
        );
    }
}

#[test]
fn canonical_effective_policy_digests_are_isolated_per_repository() {
    let first = AutonomousConfig::parse(
        "main_health:\n  branch: main\n  ignore_checks:\n    - Unit Tests\n",
    )
    .expect("first repository config parses");
    let reordered = AutonomousConfig::parse(
        "main_health:\n  ignore_checks:\n    - Unit Tests\n  branch: main\n",
    )
    .expect("reordered repository config parses");
    let second = AutonomousConfig::parse(
        "main_health:\n  branch: release\n  ignore_checks:\n    - E2E Tests\n",
    )
    .expect("second repository config parses");

    let first_digest = first
        .main_health
        .effective_policy_digest("main")
        .expect("first digest");
    assert_eq!(
        first_digest,
        reordered
            .main_health
            .effective_policy_digest("main")
            .expect("reordered digest"),
        "YAML field order must not change the canonical effective policy"
    );
    assert_ne!(
        first_digest,
        second
            .main_health
            .effective_policy_digest("release")
            .expect("second digest"),
        "repository policies must retain distinct identities in one process"
    );
}

#[test]
fn effective_policy_digest_rejects_an_empty_resolved_branch() {
    let config = AutonomousConfig::default();

    assert_eq!(
        config
            .main_health
            .effective_policy_digest("  ")
            .expect_err("empty branch must fail"),
        "effective main-health branch must not be empty"
    );
}
