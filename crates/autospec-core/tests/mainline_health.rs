use std::collections::BTreeSet;

use autospec_core::autonomous::mainline_health::{
    apply_ignored_checks, check_run_evidence, evaluate_health, resolve_health_branch,
    CheckEvidence, HealthBranchInput, MainlineHealthDiagnostic, MainlineHealthOutcome,
};

#[test]
fn explicit_health_branch_wins_over_default_branch() {
    let resolved = resolve_health_branch(&HealthBranchInput {
        explicit_branch: Some("release-candidate".to_string()),
        configured_branch: Some("master_ai".to_string()),
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
        configured_branch: None,
        default_branch: None,
    })
    .expect_err("missing default branch must be a typed diagnostic");

    assert_eq!(error, MainlineHealthDiagnostic::DefaultBranchMissing);
}

#[test]
fn configured_health_branch_wins_when_cli_override_is_absent() {
    let resolved = resolve_health_branch(&HealthBranchInput {
        explicit_branch: None,
        configured_branch: Some("master_ai".to_string()),
        default_branch: Some("trunk".to_string()),
    })
    .expect("configured branch resolves");

    assert_eq!(resolved.branch, "master_ai");
    assert_eq!(resolved.source.as_str(), "configured");
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

#[test]
fn check_run_fixtures_classify_pending_and_terminal_verdicts() {
    let cases = [
        (
            r#"{"check_runs":[{"name":"ci","status":"COMPLETED","conclusion":""}]}"#,
            MainlineHealthOutcome::Wait,
        ),
        (
            r#"{"check_runs":[{"name":"ci","status":"IN_PROGRESS","conclusion":null}]}"#,
            MainlineHealthOutcome::Wait,
        ),
        (
            r#"{"check_runs":[{"name":"ci","status":"COMPLETED","conclusion":"success"}]}"#,
            MainlineHealthOutcome::Continue,
        ),
        (
            r#"{"check_runs":[{"name":"ci","status":"COMPLETED","conclusion":"failure"}]}"#,
            MainlineHealthOutcome::Halt,
        ),
        (
            r#"{"check_runs":[{"name":"ci","status":"COMPLETED","conclusion":"cancelled"}]}"#,
            MainlineHealthOutcome::Halt,
        ),
        (
            r#"{"check_runs":[{"name":"ci","status":"COMPLETED","conclusion":"timed_out"}]}"#,
            MainlineHealthOutcome::Halt,
        ),
        (
            r#"{"check_runs":[{"name":"ci","status":"COMPLETED","conclusion":"action_required"}]}"#,
            MainlineHealthOutcome::Halt,
        ),
        (
            r#"{"check_runs":[{"name":"waiting","status":"COMPLETED","conclusion":""},{"name":"failed","status":"COMPLETED","conclusion":"failure"}]}"#,
            MainlineHealthOutcome::Halt,
        ),
    ];

    for (fixture, expected) in cases {
        let checks = check_run_evidence(fixture).expect("valid check-run fixture");
        assert_eq!(
            evaluate_health("main", true, checks).outcome,
            expected,
            "fixture: {fixture}"
        );
    }
}

#[test]
fn exact_ignored_checks_remain_evidence_but_do_not_block_admission() {
    let ignored = BTreeSet::from(["Unit Tests".to_string()]);
    let evidence = apply_ignored_checks(
        vec![
            CheckEvidence::required("Unit Tests", "completed", Some("failure")),
            CheckEvidence::required("Lint", "completed", Some("success")),
        ],
        &ignored,
    );

    assert!(!evidence[0].required, "ignored check becomes advisory");
    assert_eq!(evidence[0].status, "completed");
    assert_eq!(evidence[0].conclusion.as_deref(), Some("failure"));
    assert!(evidence[1].required, "unmatched evidence stays required");

    let observation =
        autospec_core::autonomous::mainline_health::evaluate_health("trunk", true, evidence);
    assert_eq!(observation.outcome, MainlineHealthOutcome::Continue);
}

#[test]
fn ignored_pending_checks_are_advisory_but_near_matches_still_block() {
    let ignored = BTreeSet::from(["Unit Tests".to_string()]);
    let ignored_pending = apply_ignored_checks(
        vec![CheckEvidence::required("Unit Tests", "in_progress", None)],
        &ignored,
    );
    let near_match_pending = apply_ignored_checks(
        vec![CheckEvidence::required("Unit Test", "in_progress", None)],
        &ignored,
    );

    assert_eq!(
        autospec_core::autonomous::mainline_health::evaluate_health(
            "trunk",
            true,
            ignored_pending,
        )
        .outcome,
        MainlineHealthOutcome::Continue
    );
    assert_eq!(
        autospec_core::autonomous::mainline_health::evaluate_health(
            "trunk",
            true,
            near_match_pending,
        )
        .outcome,
        MainlineHealthOutcome::Wait
    );
}

#[test]
fn ignored_checks_do_not_relax_unmatched_failure() {
    let evidence = apply_ignored_checks(
        vec![CheckEvidence::required(
            "Lint",
            "completed",
            Some("failure"),
        )],
        &BTreeSet::from(["Unit Tests".to_string()]),
    );

    assert_eq!(
        autospec_core::autonomous::mainline_health::evaluate_health("trunk", true, evidence)
            .outcome,
        MainlineHealthOutcome::Halt
    );
}

#[test]
fn health_receipt_can_bind_the_effective_policy_digest() {
    let health = autospec_core::autonomous::mainline_health::evaluate_health(
        "main",
        true,
        vec![CheckEvidence::required("ci", "completed", Some("success"))],
    );

    let receipt =
        health.to_json_with_policy_digest("owner/repo", "autospec-main-health-policy-v1:abc123");

    assert!(
        receipt.contains("\"effective_policy_digest\":\"autospec-main-health-policy-v1:abc123\"")
    );
    assert_eq!(
        receipt.matches("\"effective_policy_digest\"").count(),
        1,
        "the receipt must carry exactly one policy binding"
    );
}
