use crate::commands::autonomous::executor_bridge as bridge;
use autospec_core::autonomous::review_policy::{classify_review_requirements, ReviewPolicyInput};
use std::fs;
use std::path::Path;

pub(super) fn valid_review_json(commit: &str) -> String {
    format!(
        r#"{{"schema":1,"commit":"{commit}","verdict":"lgtm","surfaces_examined":["src/auth.rs"],"tests_examined":["tests/auth.rs"],"integration_paths_checked":[],"blocking_findings":[]}}"#
    )
}

pub(super) fn write_valid_schema5_review_receipt(
    state_path: &Path,
    state: &bridge::PersistedInvocation,
    root: &Path,
) {
    let commit = state.head_oid.as_deref().expect("review head");
    let artifacts = root.join("review-artifacts");
    fs::create_dir_all(&artifacts).expect("review artifacts");
    let stdout = artifacts.join("outer.stdout");
    let stderr = artifacts.join("outer.stderr");
    let normalizer = artifacts.join("normalizer.sh");
    let inner_stdout = artifacts.join("inner.stdout");
    let inner_stderr = artifacts.join("inner.stderr");
    let result = artifacts.join("result.json");
    let verdict_body = valid_review_json(commit);
    for (path, body) in [
        (&stdout, b"LGTM\n".as_slice()),
        (&stderr, b"".as_slice()),
        (&normalizer, b"#!/bin/sh\n".as_slice()),
        (&inner_stdout, b"review transcript\n".as_slice()),
        (&inner_stderr, b"".as_slice()),
        (&result, verdict_body.as_bytes()),
    ] {
        bridge::write_private_create_once(path, body, "review fixture").expect("private artifact");
    }
    let requirements = classify_review_requirements(&ReviewPolicyInput::default());
    let policy = bridge::ResolvedReviewPolicy {
        requirements: requirements.clone(),
        reviewer_harness: bridge::HarnessKind::Codex,
        provider_diversified: false,
        selection_reason: "normal:implementer-provider".to_string(),
    };
    let verdict = bridge::parse_review_verdict(&verdict_body, commit, &[]).expect("verdict");
    let evidence = bridge::load_bound_review_evidence(
        state,
        &requirements,
        bridge::executor_review_inventory(state).expect("review inventory"),
    )
    .expect("bound review evidence");
    let digest = |path: &Path| bridge::private_reviewer_artifact_digest(path).unwrap();
    let receipt = serde_json::json!({
        "schema": 5, "binding": bridge::review_binding(state).unwrap(),
        "stdout_path": stdout, "stdout_digest": digest(&stdout),
        "stderr_path": stderr, "stderr_digest": digest(&stderr),
        "normalizer_path": normalizer, "normalizer_digest": digest(&normalizer),
        "inner_stdout_path": inner_stdout, "inner_stdout_digest": digest(&inner_stdout),
        "inner_stderr_path": inner_stderr, "inner_stderr_digest": digest(&inner_stderr),
        "result_path": result, "result_digest": digest(&result),
        "review_commit": commit, "review_risk": "normal", "reviewer_harness": "codex",
        "reviewer_reasoning": "standard", "integration_shaped": false,
        "require_integration_smoke": false, "prefer_provider_diversity": false,
        "require_provider_diversity": false, "review_reasons": [],
        "provider_diversified": false, "selection_reason": policy.selection_reason,
        "requirements_digest": bridge::canonical_review_requirements_digest(&requirements),
        "policy_digest": bridge::canonical_review_policy_digest(&policy),
        "changed_paths": evidence.inventory.changed_paths,
        "logical_components": evidence.inventory.logical_components,
        "producer_surfaces": evidence.inventory.producer_surfaces,
        "consumer_surfaces": evidence.inventory.consumer_surfaces,
        "integration_evidence_digest": evidence.integration_evidence_digest,
        "integration_command_records": evidence.integration_command_records,
        "review_context_digest": bridge::canonical_review_context_digest(&policy, &evidence),
        "verdict_schema": verdict.schema, "verdict": verdict.verdict,
        "surfaces_examined": verdict.surfaces_examined, "tests_examined": verdict.tests_examined,
        "integration_paths_checked": verdict.integration_paths_checked,
        "blocking_findings": verdict.blocking_findings,
        "verdict_digest": bridge::canonical_review_verdict_digest(&verdict),
    });
    let receipt_path = bridge::review_receipt_path(state_path, state).expect("review path");
    bridge::write_private_create_once(
        &receipt_path,
        format!("{receipt}\n").as_bytes(),
        "schema-5 review receipt",
    )
    .expect("review receipt");
}
