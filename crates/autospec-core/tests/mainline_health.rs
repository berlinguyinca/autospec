use autospec_core::autonomous::mainline_health::{
    resolve_health_branch, CheckEvidence, HealthBranchInput, MainlineHealthDiagnostic,
    MainlineHealthOutcome,
};

#[test]
fn explicit_health_branch_wins_over_default_branch() {
    let resolved = resolve_health_branch(&HealthBranchInput {
        explicit_branch: Some("release-candidate".to_string()),
        default_branch: Some("trunk".to_string()),
    })
    .expect("explicit branch resolves");

    assert_eq!(resolved.branch, "release-candidate");
    assert_eq!(resolved.source.as_str(), "explicit");
}

#[test]
fn default_branch_resolution_does_not_silently_fall_back_to_main() {
    let error = resolve_health_branch(&HealthBranchInput {
        explicit_branch: None,
        default_branch: None,
    })
    .expect_err("missing default branch must be a typed diagnostic");

    assert_eq!(error, MainlineHealthDiagnostic::DefaultBranchMissing);
}

#[test]
fn failed_required_check_blocks_mainline_health_admission() {
    let observation = autospec_core::autonomous::mainline_health::evaluate_health(
        "trunk",
        true,
        vec![CheckEvidence::required("ci", "completed", Some("failure"))],
    );

    assert_eq!(observation.branch, "trunk");
    assert_eq!(observation.outcome, MainlineHealthOutcome::Halt);
    assert_eq!(
        observation.diagnostic,
        MainlineHealthDiagnostic::RequiredCheckFailed
    );
}

#[test]
fn pending_required_check_blocks_mainline_health_admission() {
    let observation = autospec_core::autonomous::mainline_health::evaluate_health(
        "trunk",
        true,
        vec![CheckEvidence::required("ci", "in_progress", None)],
    );

    assert_eq!(observation.outcome, MainlineHealthOutcome::Wait);
    assert_eq!(
        observation.diagnostic,
        MainlineHealthDiagnostic::RequiredCheckPending
    );
}
