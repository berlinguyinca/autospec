// executor_bridge tests: integration-smoke review evidence.

use super::super as bridge;
use super::support_base::GitFixture;
use super::support_invocation::supervision_state;
use autospec_core::autonomous::review_policy::{
    classify_review_requirements, ReviewPolicyInput, ReviewRequirements,
};
use std::time::Duration;

fn integration_requirements() -> ReviewRequirements {
    classify_review_requirements(&ReviewPolicyInput {
        serialization_reasons: vec!["priority:high".to_string()],
        ..ReviewPolicyInput::default()
    })
}

#[test]
fn integration_shaped_issue_without_integration_smoke_fails_before_review() {
    let body = "### Primary smoke test (inner loop)\n\n```bash\n/usr/bin/true\n```\n";

    let error = bridge::parse_required_integration_smoke(body, &integration_requirements())
        .expect_err("integration-shaped work must carry integration evidence");

    assert!(error.contains("integration smoke"), "{error}");
}

#[test]
fn duplicate_integration_smoke_headings_fail_as_ambiguous() {
    let body = concat!(
        "### Primary smoke test (inner loop)\n\n```bash\n/usr/bin/true\n```\n\n",
        "### Integration smoke test (pre-merge)\n\n```bash\n/usr/bin/true\n```\n\n",
        "### Integration smoke test (pre-merge)\n\n```bash\n/usr/bin/true\n```\n",
    );

    let error = bridge::parse_required_integration_smoke(body, &integration_requirements())
        .expect_err("duplicate integration smoke headings must fail closed");

    assert!(error.contains("exactly one"), "{error}");
}

#[test]
fn integration_primary_smoke_accepts_a_repository_integration_test() {
    let body = "### Primary smoke test (inner loop)\n\n```bash\n/usr/bin/true tests/integration/review-policy.rs\n```\n";

    let plan = bridge::parse_required_integration_smoke(body, &integration_requirements())
        .expect("qualifying primary compatibility smoke")
        .expect("integration smoke plan");

    assert_eq!(
        plan.commands[0].argv,
        vec!["/usr/bin/true", "tests/integration/review-policy.rs"]
    );
}

#[test]
fn failing_integration_smoke_blocks_ci_passed_transition() {
    let fixture = GitFixture::new("integration-smoke-failure");
    let mut state = supervision_state(&fixture);
    state.phase = bridge::BridgePhase::DraftCreated;
    let body = "### Integration smoke test (pre-merge)\n\n```bash\n/usr/bin/false\n```\n";
    let plan = bridge::parse_required_integration_smoke(body, &integration_requirements())
        .expect("strict integration smoke")
        .expect("required plan");

    let error = bridge::execute_required_integration_smoke(
        &state,
        &plan,
        &fixture.root.join("integration-failure"),
        None,
        Duration::from_secs(5),
    )
    .expect_err("failing integration evidence must block admission");

    assert!(error.contains("exit status 1"), "{error}");
    assert_eq!(state.phase, bridge::BridgePhase::DraftCreated);
}

#[test]
fn passing_integration_smoke_is_bound_into_premerge_evidence() {
    let fixture = GitFixture::new("integration-smoke-binding");
    let mut state = supervision_state(&fixture);
    state.phase = bridge::BridgePhase::DraftCreated;
    let requirements = integration_requirements();
    let body =
        "### Integration smoke test (pre-merge)\n\n```bash\n/usr/bin/printf integration-ok\n```\n";
    let plan = bridge::parse_required_integration_smoke(body, &requirements)
        .expect("strict integration smoke")
        .expect("required plan");
    let artifact_root = fixture.root.join("integration-pass");
    let observations = bridge::execute_required_integration_smoke(
        &state,
        &plan,
        &artifact_root,
        None,
        Duration::from_secs(5),
    )
    .expect("passing integration evidence");

    let binding =
        bridge::bind_integration_smoke_evidence(&requirements, &artifact_root, &observations)
            .expect("commit-bound integration evidence");

    assert!(bridge::canonical_sha256(&binding.requirements_digest));
    assert!(bridge::canonical_sha256(&binding.evidence_digest));
    assert_eq!(binding.command_records.len(), 1);
    assert!(binding.command_records[0].ends_with("command-000.json"));
}
