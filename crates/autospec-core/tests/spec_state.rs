use autospec_core::state::{SpecLifecycle, SpecRunState};
use autospec_core::validation::{ValidationRegistry, ValidationStatus};

#[test]
fn spec_state_allows_valid_transitions() {
    let mut lifecycle = SpecLifecycle::new("v65-spec-state-validation");

    lifecycle
        .transition_to(SpecRunState::Ready)
        .expect("planned -> ready");
    lifecycle
        .transition_to(SpecRunState::Running)
        .expect("ready -> running");
    lifecycle
        .transition_to(SpecRunState::Passed)
        .expect("running -> passed");

    assert_eq!(lifecycle.state, SpecRunState::Passed);
}

#[test]
fn spec_state_rejects_invalid_transitions() {
    let mut lifecycle = SpecLifecycle::new("v65-spec-state-validation");

    let error = lifecycle
        .transition_to(SpecRunState::Passed)
        .expect_err("planned -> passed should fail");

    assert!(error.contains("invalid transition"));
    assert_eq!(lifecycle.state, SpecRunState::Planned);
}

#[test]
fn spec_state_records_deferred_and_superseded_metadata() {
    let deferred = SpecLifecycle::new("v65-spec-state-validation")
        .deferred("waiting on V64 proof")
        .expect("deferred reason is valid");
    let superseded = SpecLifecycle::new("v65-spec-state-validation")
        .superseded_by("v66-autonomous-execution-queue")
        .expect("replacement id is valid");

    assert_eq!(deferred.state, SpecRunState::Deferred);
    assert_eq!(
        deferred.deferred_reason.as_deref(),
        Some("waiting on V64 proof")
    );
    assert_eq!(superseded.state, SpecRunState::Superseded);
    assert_eq!(
        superseded.superseded_by.as_deref(),
        Some("v66-autonomous-execution-queue")
    );
}

#[test]
fn validation_registry_runs_shell_command_and_captures_status() {
    let mut registry = ValidationRegistry::new();
    registry.register("smoke", "printf ok", ".", 5, true);

    let result = registry.run("smoke").expect("registered command runs");

    assert_eq!(result.status, ValidationStatus::Passed);
    assert_eq!(result.stdout, "ok");
    assert!(result.stderr.is_empty());
}
