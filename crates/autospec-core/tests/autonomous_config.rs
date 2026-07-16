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
another_policy:
  values:
    - ignored
"#,
    )
    .expect("unrelated configuration is ignored");

    assert_eq!(absent, AutonomousConfig::default());
    assert_eq!(unrelated, AutonomousConfig::default());
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
    ] {
        let error = AutonomousConfig::parse(source).expect_err(&format!("{name} must fail closed"));
        assert!(
            !error.is_empty(),
            "{name} must produce a useful parse diagnostic"
        );
    }
}
