use std::collections::BTreeSet;

use autospec_core::autonomous::mainline_health::{
    apply_ignored_checks, check_run_evidence, evaluate_health, evaluate_health_with_baseline,
    resolve_health_branch, CheckEvidence, HealthBaseline, HealthBranchInput,
    MainlineHealthDiagnostic, MainlineHealthOutcome,
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
fn baseline_tolerates_known_red_and_halts_new_red() {
    let mut baseline = BTreeSet::new();
    baseline.insert("legacy-red".to_string());
    let known = evaluate_health_with_baseline(
        "main",
        true,
        vec![CheckEvidence::required(
            "legacy-red",
            "completed",
            Some("failure"),
        )],
        HealthBaseline::Ready(baseline.clone()),
    );
    assert_eq!(known.outcome, MainlineHealthOutcome::Continue);
    assert_eq!(known.newly_red_checks, Vec::<String>::new());
    let new = evaluate_health_with_baseline(
        "main",
        true,
        vec![CheckEvidence::required(
            "new-red",
            "completed",
            Some("failure"),
        )],
        HealthBaseline::Ready(baseline),
    );
    assert_eq!(new.outcome, MainlineHealthOutcome::Halt);
    assert_eq!(new.diagnostic, MainlineHealthDiagnostic::NewCheckFailed);
    assert_eq!(new.newly_red_checks, vec!["new-red"]);
}

#[test]
fn baseline_failures_are_explicit_and_repository_sets_are_isolated() {
    let stale = evaluate_health_with_baseline("main", true, Vec::new(), HealthBaseline::Stale);
    assert_eq!(stale.diagnostic, MainlineHealthDiagnostic::BaselineStale);
    let failed = evaluate_health_with_baseline("main", true, Vec::new(), HealthBaseline::Failed);
    assert_eq!(
        failed.diagnostic,
        MainlineHealthDiagnostic::BaselineReadFailed
    );
    let mut a = BTreeSet::new();
    a.insert("a".to_string());
    let mut b = BTreeSet::new();
    b.insert("b".to_string());
    let ha = evaluate_health_with_baseline("main", true, Vec::new(), HealthBaseline::Ready(a));
    let hb = evaluate_health_with_baseline("main", true, Vec::new(), HealthBaseline::Ready(b));
    assert_eq!(ha.baseline_checks, vec!["a"]);
    assert_eq!(hb.baseline_checks, vec!["b"]);
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

#[test]
fn a_wait_receipt_distinguishes_unreachable_github_from_a_pending_check() {
    // Both outcomes are Wait and both surface to the operator as the same bare
    // {"decision":"park","reason":"health_wait"}. The receipt is what tells them apart:
    // "keep waiting, CI is running" versus "I cannot reach GitHub at all".
    let unreachable = autospec_core::autonomous::mainline_health::MainlineHealth::diagnostic(
        "main",
        autospec_core::autonomous::mainline_health::MainlineHealthOutcome::Wait,
        autospec_core::autonomous::mainline_health::MainlineHealthDiagnostic::GhApiFailed,
    );
    let pending = autospec_core::autonomous::mainline_health::evaluate_health(
        "main",
        true,
        vec![CheckEvidence::required("ci", "in_progress", None)],
    );

    let unreachable_receipt = unreachable.to_json("owner/repo");
    let pending_receipt = pending.to_json("owner/repo");

    assert!(unreachable_receipt.contains("\"outcome\":\"wait\""));
    assert!(pending_receipt.contains("\"outcome\":\"wait\""));
    assert!(
        unreachable_receipt.contains("\"diagnostic\":\"gh-api-failed\""),
        "unreachable GitHub must be attributable, got: {unreachable_receipt}"
    );
    assert!(
        pending_receipt.contains("\"diagnostic\":\"required-check-pending\""),
        "a genuinely pending check must stay distinguishable, got: {pending_receipt}"
    );
    assert_ne!(
        unreachable_receipt, pending_receipt,
        "the two wait causes must not be indistinguishable"
    );
}

#[test]
fn a_repo_with_no_ci_at_all_continues_rather_than_waiting() {
    // A repo that deliberately runs no CI reports state=pending/total_count=0 on the legacy
    // endpoint forever, and zero check-runs. That must not be read as "CI is pending".
    let health = autospec_core::autonomous::mainline_health::evaluate_health("main", true, vec![]);

    assert_eq!(
        health.outcome,
        autospec_core::autonomous::mainline_health::MainlineHealthOutcome::Continue
    );
    assert!(health
        .to_json("owner/repo")
        .contains("\"diagnostic\":\"no-required-checks\""));
}
