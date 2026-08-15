// executor_bridge tests: schema-5 structured review receipts.

use super::support_base::GitFixture;
use super::support_invocation::implementation_proof_fixture;
use crate::commands::autonomous::executor_bridge as bridge;
use autospec_core::autonomous::review_policy::{classify_review_requirements, ReviewPolicyInput};
use std::fs;
use std::path::{Path, PathBuf};

const COMMIT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn review_json_with_citations(commit: &str, citations: &[String]) -> String {
    let citations = serde_json::to_string(citations).expect("review citations");
    format!(
        r#"{{"schema":1,"commit":"{commit}","verdict":"lgtm","surfaces_examined":["src/auth.rs"],"tests_examined":["tests/auth.rs"],"integration_paths_checked":{citations},"blocking_findings":[]}}"#
    )
}

struct ReceiptFixture {
    _fixture: GitFixture,
    state: bridge::PersistedInvocation,
    state_path: PathBuf,
    receipt_path: PathBuf,
    paths: [PathBuf; 6],
    verdict: String,
    policy: bridge::ResolvedReviewPolicy,
    evidence: bridge::BoundReviewEvidence,
}

fn receipt_fixture(name: &str) -> ReceiptFixture {
    let (fixture, mut state, _snapshot, _) = implementation_proof_fixture(name);
    state.head_oid = Some(COMMIT.to_string());
    let state_path = fixture.root.join("state/invocation.json");
    let root = fixture.root.join("review-artifacts");
    fs::create_dir_all(&root).expect("review artifacts");
    let paths = [
        root.join("outer.stdout"),
        root.join("outer.stderr"),
        root.join("normalizer.sh"),
        root.join("inner.stdout"),
        root.join("inner.stderr"),
        root.join("result.json"),
    ];
    let verdict = review_json_with_citations(COMMIT, &[]);
    for (path, body) in [
        (&paths[0], b"LGTM\n".as_slice()),
        (&paths[1], b"".as_slice()),
        (&paths[2], b"#!/bin/sh\n".as_slice()),
        (&paths[3], b"review transcript\n".as_slice()),
        (&paths[4], b"".as_slice()),
        (&paths[5], verdict.as_bytes()),
    ] {
        bridge::write_private_create_once(path, body, "review fixture").expect("private artifact");
    }
    let receipt_path = bridge::review_receipt_path(&state_path, &state).expect("receipt path");
    let requirements = classify_review_requirements(&ReviewPolicyInput::default());
    let policy = bridge::ResolvedReviewPolicy {
        requirements: requirements.clone(),
        reviewer_harness: bridge::HarnessKind::Codex,
        provider_diversified: false,
        selection_reason: "normal:implementer-provider".to_string(),
    };
    let evidence = bridge::load_bound_review_evidence(
        &state,
        &requirements,
        bridge::executor_review_inventory(&state).expect("review inventory"),
    )
    .expect("bound review evidence");
    ReceiptFixture {
        _fixture: fixture,
        state,
        state_path,
        receipt_path,
        paths,
        verdict,
        policy,
        evidence,
    }
}

fn manual_policy_digest(selection_reason: &str) -> String {
    let requirements_digest = manual_requirements_digest();
    autospec_core::autonomous::waterfall::sha256_hex(
        format!(
            "review-policy-v1\0{}\0codex\0false\0{selection_reason}",
            requirements_digest
        )
        .as_bytes(),
    )
}

fn manual_requirements_digest() -> String {
    autospec_core::autonomous::waterfall::sha256_hex(
        b"normal\0standard\0false\0false\0false\0false\0",
    )
}

fn manual_verdict_digest(surfaces: &[&str]) -> String {
    autospec_core::autonomous::waterfall::sha256_hex(
        format!(
            "review-verdict-v1\01\0{COMMIT}\0lgtm\0{}\0tests/auth.rs\0\0",
            surfaces.join("\0")
        )
        .as_bytes(),
    )
}

fn schema5_receipt(fixture: &ReceiptFixture) -> serde_json::Value {
    let digest = |path: &Path| bridge::private_reviewer_artifact_digest(path).unwrap();
    serde_json::json!({
        "schema": 5,
        "binding": bridge::review_binding(&fixture.state).unwrap(),
        "stdout_path": fixture.paths[0],
        "stdout_digest": autospec_core::autonomous::waterfall::sha256_hex(b"LGTM\n"),
        "stderr_path": fixture.paths[1],
        "stderr_digest": autospec_core::autonomous::waterfall::sha256_hex(b""),
        "normalizer_path": fixture.paths[2], "normalizer_digest": digest(&fixture.paths[2]),
        "inner_stdout_path": fixture.paths[3], "inner_stdout_digest": digest(&fixture.paths[3]),
        "inner_stderr_path": fixture.paths[4], "inner_stderr_digest": digest(&fixture.paths[4]),
        "result_path": fixture.paths[5], "result_digest": digest(&fixture.paths[5]),
        "review_commit": COMMIT,
        "review_risk": "normal",
        "reviewer_harness": "codex",
        "reviewer_reasoning": "standard",
        "integration_shaped": false,
        "require_integration_smoke": false,
        "prefer_provider_diversity": false,
        "require_provider_diversity": false,
        "review_reasons": [],
        "provider_diversified": false,
        "selection_reason": "normal:implementer-provider",
        "requirements_digest": manual_requirements_digest(),
        "policy_digest": manual_policy_digest("normal:implementer-provider"),
        "changed_paths": fixture.evidence.inventory.changed_paths,
        "logical_components": fixture.evidence.inventory.logical_components,
        "producer_surfaces": fixture.evidence.inventory.producer_surfaces,
        "consumer_surfaces": fixture.evidence.inventory.consumer_surfaces,
        "integration_evidence_digest": fixture.evidence.integration_evidence_digest,
        "integration_command_records": fixture.evidence.integration_command_records,
        "review_context_digest": bridge::canonical_review_context_digest(&fixture.policy, &fixture.evidence),
        "verdict_schema": 1,
        "verdict": "lgtm",
        "surfaces_examined": ["src/auth.rs"],
        "tests_examined": ["tests/auth.rs"],
        "integration_paths_checked": [],
        "blocking_findings": [],
        "verdict_digest": manual_verdict_digest(&["src/auth.rs"]),
    })
}

fn write_receipt(fixture: &ReceiptFixture, receipt: serde_json::Value) {
    bridge::write_private_create_once(
        &fixture.receipt_path,
        format!("{receipt}\n").as_bytes(),
        "review receipt fixture",
    )
    .expect("write receipt");
}

#[test]
fn structured_review_receipt_accepts_exact_schema5_evidence() {
    // Break caught: an internally consistent semantic receipt being unrecoverable after a crash.
    let fixture = receipt_fixture("receipt-schema5-valid");
    write_receipt(&fixture, schema5_receipt(&fixture));

    bridge::validate_review_receipt(&fixture.state_path, &fixture.state)
        .expect("valid schema-5 review receipt");
}

#[test]
fn receipt_rejects_policy_digest_drift() {
    // Break caught: receipt policy fields changing without invalidating reviewer authority.
    let fixture = receipt_fixture("receipt-policy-drift");
    let mut receipt = schema5_receipt(&fixture);
    receipt["selection_reason"] = serde_json::json!("risk:same-provider-high-reasoning-fallback");
    write_receipt(&fixture, receipt);

    let error = bridge::validate_review_receipt(&fixture.state_path, &fixture.state)
        .expect_err("policy digest drift must fail closed");
    assert!(error.contains("policy digest mismatch"), "{error}");
}

#[test]
fn receipt_rejects_verdict_digest_drift() {
    // Break caught: receipt semantic evidence changing without invalidating the verdict.
    let fixture = receipt_fixture("receipt-verdict-drift");
    let mut receipt = schema5_receipt(&fixture);
    receipt["surfaces_examined"] = serde_json::json!(["src/other.rs"]);
    write_receipt(&fixture, receipt);

    let error = bridge::validate_review_receipt(&fixture.state_path, &fixture.state)
        .expect_err("verdict digest drift must fail closed");
    assert!(error.contains("verdict digest mismatch"), "{error}");
}

#[test]
fn receipt_rejects_review_context_digest_drift() {
    // Break caught: changed component inventory retaining authority from the original review.
    let fixture = receipt_fixture("receipt-context-drift");
    let mut receipt = schema5_receipt(&fixture);
    receipt["logical_components"] = serde_json::json!(["crates/other"]);
    write_receipt(&fixture, receipt);

    let error = bridge::validate_review_receipt(&fixture.state_path, &fixture.state)
        .expect_err("review context digest drift must fail closed");
    assert!(error.contains("context digest mismatch"), "{error}");
}

#[test]
fn structured_review_receipt_rejects_inner_artifact_drift() {
    // Break caught: a normalized verdict surviving replacement of its private harness evidence.
    let fixture = receipt_fixture("receipt-inner-artifact-drift");
    write_receipt(&fixture, schema5_receipt(&fixture));
    fs::write(&fixture.paths[4], "changed transport trace\n").expect("tamper inner stderr");

    let error = bridge::validate_review_receipt(&fixture.state_path, &fixture.state)
        .expect_err("changed inner evidence must invalidate receipt");
    assert!(
        error.contains("inner_stderr_path digest mismatch"),
        "{error}"
    );
}

#[test]
fn structured_review_legacy_receipt_recovers_to_ci_passed_for_rereview() {
    // Break caught: a legacy unstructured LGTM receipt retaining merge authority after upgrade.
    let mut fixture = receipt_fixture("receipt-legacy-rereview");
    fixture.state.phase = bridge::BridgePhase::ReviewPassed;
    let receipt = serde_json::json!({
        "schema": 4,
        "binding": bridge::review_binding(&fixture.state).unwrap(),
        "stdout_path": fixture.paths[0],
        "stdout_digest": autospec_core::autonomous::waterfall::sha256_hex(b"LGTM\n"),
        "stderr_path": fixture.paths[1],
        "stderr_digest": autospec_core::autonomous::waterfall::sha256_hex(b""),
        "normalizer_path": fixture.paths[2],
        "normalizer_digest": bridge::private_reviewer_artifact_digest(&fixture.paths[2]).unwrap(),
        "inner_stdout_path": fixture.paths[3],
        "inner_stdout_digest": bridge::private_reviewer_artifact_digest(&fixture.paths[3]).unwrap(),
        "inner_stderr_path": fixture.paths[4],
        "inner_stderr_digest": bridge::private_reviewer_artifact_digest(&fixture.paths[4]).unwrap(),
        "result_path": fixture.paths[5],
        "result_digest": bridge::private_reviewer_artifact_digest(&fixture.paths[5]).unwrap(),
    });
    write_receipt(&fixture, receipt);

    assert!(
        !bridge::recover_existing_review_receipt(&fixture.state_path, &mut fixture.state)
            .expect("legacy receipt recovery")
    );
    assert_eq!(fixture.state.phase, bridge::BridgePhase::CiPassed);
    assert!(!fixture.receipt_path.exists());
}
