// executor_bridge tests: risk-aware reviewer provider selection.

use super::support_invocation::{implementation_proof_fixture, reviewer_request};
use crate::commands::autonomous::executor_bridge as bridge;
use autospec_core::autonomous::review_policy::{
    classify_review_requirements, ReviewPolicyInput, ReviewRequirements,
};
use std::collections::BTreeMap;
use std::ffi::OsString;

fn requirements(input: ReviewPolicyInput) -> ReviewRequirements {
    classify_review_requirements(&input)
}

fn harness_config(kinds: &[bridge::HarnessKind]) -> bridge::HarnessConfig {
    bridge::HarnessConfig {
        aliases: kinds
            .iter()
            .map(|kind| bridge::HarnessAlias {
                kind: *kind,
                binary: "/usr/bin/true".to_string(),
                approval_alias: String::new(),
                display_name: kind.as_str().to_string(),
            })
            .collect(),
        opencode_adapter: None,
    }
}

#[test]
fn review_provider_normal_retains_the_implementer_provider() {
    // Break caught: normal review paying diversity cost despite the current provider being valid.
    let policy = bridge::resolve_review_policy(
        &harness_config(&[bridge::HarnessKind::Claude, bridge::HarnessKind::Codex]),
        requirements(ReviewPolicyInput::default()),
        bridge::HarnessKind::Codex,
        &BTreeMap::new(),
    )
    .expect("normal reviewer policy");

    assert_eq!(policy.reviewer_harness, bridge::HarnessKind::Codex);
    assert!(!policy.provider_diversified);
    assert_eq!(policy.selection_reason, "normal:implementer-provider");
}

#[test]
fn review_provider_integration_prefers_a_different_provider() {
    // Break caught: integration review reusing the implementer when an independent provider exists.
    let policy = bridge::resolve_review_policy(
        &harness_config(&[bridge::HarnessKind::Codex, bridge::HarnessKind::Claude]),
        requirements(ReviewPolicyInput {
            changed_paths: vec!["scripts/autospec-daemon.sh".to_string()],
            ..ReviewPolicyInput::default()
        }),
        bridge::HarnessKind::Codex,
        &BTreeMap::new(),
    )
    .expect("provider-diverse integration review");

    assert_eq!(policy.reviewer_harness, bridge::HarnessKind::Claude);
    assert!(policy.provider_diversified);
    assert_eq!(policy.selection_reason, "risk:provider-diversified");
}

#[test]
fn review_provider_integration_records_same_provider_high_reasoning_fallback() {
    // Break caught: a permitted integration fallback being mistaken for provider diversity.
    let policy = bridge::resolve_review_policy(
        &harness_config(&[bridge::HarnessKind::Codex]),
        requirements(ReviewPolicyInput {
            logical_component_count: 2,
            ..ReviewPolicyInput::default()
        }),
        bridge::HarnessKind::Codex,
        &BTreeMap::new(),
    )
    .expect("same-provider integration fallback");

    assert_eq!(policy.reviewer_harness, bridge::HarnessKind::Codex);
    assert!(!policy.provider_diversified);
    assert_eq!(
        policy.selection_reason,
        "risk:same-provider-high-reasoning-fallback"
    );
    assert_eq!(
        policy.requirements.reviewer_reasoning,
        autospec_core::autonomous::review_policy::ReviewReasoning::High
    );
}

#[test]
fn review_provider_does_not_mistake_opencode_for_a_distinct_provider() {
    let error = bridge::resolve_review_policy(
        &harness_config(&[bridge::HarnessKind::Codex, bridge::HarnessKind::OpenCode]),
        requirements(ReviewPolicyInput {
            critical_boundary: true,
            ..ReviewPolicyInput::default()
        }),
        bridge::HarnessKind::Codex,
        &BTreeMap::new(),
    )
    .expect_err("a harness name cannot prove provider independence");

    assert!(error.contains("alternate provider"), "{error}");
}

#[test]
fn review_provider_critical_without_an_alternate_fails_closed() {
    // Break caught: critical work silently degrading to same-provider self-review.
    let error = bridge::resolve_review_policy(
        &harness_config(&[bridge::HarnessKind::Codex]),
        requirements(ReviewPolicyInput {
            critical_boundary: true,
            ..ReviewPolicyInput::default()
        }),
        bridge::HarnessKind::Codex,
        &BTreeMap::new(),
    )
    .expect_err("critical review requires another provider");

    assert!(error.contains("critical"), "{error}");
    assert!(error.contains("alternate provider"), "{error}");
}

#[test]
fn review_provider_unstructured_command_cannot_authorize_production_review() {
    // Break caught: arbitrary stdout containing LGTM bypassing the structured reviewer adapter.
    let (fixture, mut state, _snapshot, _) = implementation_proof_fixture("review-override");
    state.phase = bridge::BridgePhase::CiPassed;
    state.head_oid = Some("a".repeat(40));
    let request = reviewer_request(&state, fixture.root.join("state/invocation.json"));
    let env = BTreeMap::from([(
        "AUTOSPEC_EXECUTOR_REVIEW_COMMAND".to_string(),
        OsString::from("/usr/bin/printf LGTM"),
    )]);

    let error = bridge::resolve_independent_reviewer(
        &request,
        &state,
        &env,
        &fixture.root.join("review-artifacts"),
    )
    .expect_err("unstructured production review must fail closed");

    assert!(error.contains("unstructured"), "{error}");
    assert!(error.contains("cannot authorize"), "{error}");
}
