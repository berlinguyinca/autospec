use autospec_core::runtime_policy::{
    classify_path, is_supported_runtime_path, Runtime, RuntimeClass,
};

#[test]
fn runtime_policy_covers_r0_to_r4_variants() {
    let classes = [
        RuntimeClass::R0,
        RuntimeClass::R1,
        RuntimeClass::R2,
        RuntimeClass::R3,
        RuntimeClass::R4,
    ];

    assert_eq!(
        classes.map(|class| class.as_str()),
        ["R0", "R1", "R2", "R3", "R4"]
    );
}

#[test]
fn runtime_policy_marks_stateful_shell_helpers_as_r1() {
    let verdict = classify_path("scripts/lint-issue.sh");

    assert_eq!(verdict.runtime, Runtime::Shell);
    assert_eq!(verdict.class, RuntimeClass::R1);
    assert_eq!(
        verdict.reasons,
        vec!["stateful platform behavior belongs in Rust core".to_string()]
    );
}

#[test]
fn runtime_policy_marks_fab_python_as_r4_exception() {
    let verdict = classify_path("skills/autospec-fab/scripts/stage_cfd.py");

    assert_eq!(verdict.runtime, Runtime::Python);
    assert_eq!(verdict.class, RuntimeClass::R4);
    assert!(verdict.reasons[0].contains("exception"));
}

#[test]
fn runtime_policy_keeps_install_scripts_as_r0_wrappers() {
    let verdict = classify_path("skills/autospec-run/install.sh");

    assert_eq!(verdict.runtime, Runtime::Shell);
    assert_eq!(verdict.class, RuntimeClass::R0);
}

#[test]
fn runtime_policy_defaults_unknown_helpers_to_r2() {
    let verdict = classify_path("docs/specs/example.md");

    assert_eq!(verdict.runtime, Runtime::Unknown);
    assert_eq!(verdict.class, RuntimeClass::R2);
}

#[test]
fn runtime_policy_exposes_the_runtime_paths_supported_by_the_classifier() {
    assert!(is_supported_runtime_path(
        "skills/autospec-run/tests/watchdog_claim_timeout.bats"
    ));
    assert!(is_supported_runtime_path("packages/example/go.mod"));
    assert!(!is_supported_runtime_path("docs/specs/example.md"));
}
