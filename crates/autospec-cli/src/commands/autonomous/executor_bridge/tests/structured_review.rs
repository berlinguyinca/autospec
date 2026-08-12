// executor_bridge tests: strict semantic reviewer verdicts and schema-5 receipts.

use super::super as bridge;
use super::support_invocation::{implementation_proof_fixture, reviewer_request};
use autospec_core::autonomous::review_policy::{classify_review_requirements, ReviewPolicyInput};
use std::fs;
use std::process::Command;

const COMMIT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn review_json_with_citations(commit: &str, citations: &[String]) -> String {
    let citations = serde_json::to_string(citations).expect("review citations");
    format!(
        r#"{{"schema":1,"commit":"{commit}","verdict":"lgtm","surfaces_examined":["src/auth.rs"],"tests_examined":["tests/auth.rs"],"integration_paths_checked":{citations},"blocking_findings":[]}}"#
    )
}

fn integration_citations() -> Vec<String> {
    vec![
        "requirements-digest:requirements-digest".to_string(),
        "integration-evidence-digest:evidence-digest".to_string(),
        "integration-record:.autospec/evidence/premerge/lane/attempts/generation/qa/integration/command-000.json".to_string(),
    ]
}

fn review_json(commit: &str) -> String {
    review_json_with_citations(commit, &integration_citations())
}

#[test]
fn structured_review_accepts_exact_semantic_evidence() {
    // Break caught: a complete exact-commit verdict being rejected before normalization.
    let citations = integration_citations();
    let verdict = bridge::parse_review_verdict(&review_json(COMMIT), COMMIT, &citations)
        .expect("valid structured review");

    assert_eq!(verdict.commit, COMMIT);
    assert_eq!(verdict.verdict, "lgtm");
    assert_eq!(verdict.surfaces_examined, ["src/auth.rs"]);
    assert_eq!(verdict.tests_examined, ["tests/auth.rs"]);
    assert_eq!(verdict.integration_paths_checked, citations);
    assert!(verdict.blocking_findings.is_empty());
}

#[test]
fn structured_review_rejects_wrong_commit() {
    // Break caught: a valid review for an earlier commit authorizing the current commit.
    let wrong = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    let error = bridge::parse_review_verdict(&review_json(wrong), COMMIT, &integration_citations())
        .expect_err("wrong commit must fail closed");

    assert!(error.contains("commit mismatch"), "{error}");
}

#[test]
fn structured_review_rejects_unknown_keys() {
    // Break caught: an invented reviewer field bypassing the closed evidence schema.
    let body = review_json(COMMIT).replace(
        r#","blocking_findings":[]}"#,
        r#","blocking_findings":[],"confidence":"high"}"#,
    );
    let error = bridge::parse_review_verdict(&body, COMMIT, &integration_citations())
        .expect_err("unknown key must fail closed");

    assert!(error.contains("unexpected field"), "{error}");
}

#[test]
fn structured_review_rejects_empty_surfaces() {
    // Break caught: a reviewer authorizing code it did not identify as examined.
    let body = review_json(COMMIT).replace(r#"["src/auth.rs"]"#, "[]");
    let error = bridge::parse_review_verdict(&body, COMMIT, &integration_citations())
        .expect_err("empty surfaces must fail closed");

    assert!(error.contains("surfaces_examined"), "{error}");
}

#[test]
fn structured_review_rejects_empty_tests() {
    // Break caught: a reviewer authorizing code without identifying test evidence.
    let body = review_json(COMMIT).replace(r#"["tests/auth.rs"]"#, "[]");
    let error = bridge::parse_review_verdict(&body, COMMIT, &integration_citations())
        .expect_err("empty tests must fail closed");

    assert!(error.contains("tests_examined"), "{error}");
}

#[test]
fn structured_review_requires_integration_paths_when_policy_does() {
    // Break caught: integration-shaped work receiving only file-level review evidence.
    let body = review_json_with_citations(COMMIT, &[]);
    let error = bridge::parse_review_verdict(&body, COMMIT, &integration_citations())
        .expect_err("missing integration paths must fail closed");

    assert!(error.contains("integration evidence"), "{error}");
}

#[test]
fn structured_review_rejects_unbound_integration_path_claims() {
    // Break caught: arbitrary reviewer prose satisfying integration evidence admission.
    let body = review_json_with_citations(COMMIT, &["login -> session".to_string()]);
    let error = bridge::parse_review_verdict(&body, COMMIT, &integration_citations())
        .expect_err("unbound integration path must fail closed");

    assert!(error.contains("integration evidence"), "{error}");
}

#[test]
fn independent_reviewer_prompt_includes_policy_components_and_immutable_evidence() {
    // Break caught: reviewer approval without receiving the policy or smoke artifacts it cites.
    let (fixture, mut state, _snapshot, _) = implementation_proof_fixture("review-bound-prompt");
    state.head_oid = Some(COMMIT.to_string());
    let request = reviewer_request(&state, fixture.root.join("state/invocation.json"));
    let requirements = classify_review_requirements(&ReviewPolicyInput {
        has_producer_surface: true,
        has_consumer_surface: true,
        ..ReviewPolicyInput::default()
    });
    let requirements_digest = bridge::canonical_review_requirements_digest(&requirements);
    let evidence_digest = format!("sha256:{}", "b".repeat(64));
    let inventory = bridge::ExecutorReviewInventory {
        changed_paths: vec![
            "crates/example/src/event_consumer.rs".to_string(),
            "crates/example/src/event_producer.rs".to_string(),
        ],
        logical_components: vec!["crates/example".to_string()],
        producer_surfaces: vec!["crates/example/src/event_producer.rs".to_string()],
        consumer_surfaces: vec!["crates/example/src/event_consumer.rs".to_string()],
    };
    let evidence = bridge::BoundReviewEvidence {
        commit: COMMIT.to_string(),
        inventory,
        requirements_digest: requirements_digest.clone(),
        integration_evidence_digest: Some(evidence_digest.clone()),
        integration_command_records: vec!["qa/integration/command-000.json".to_string()],
    };
    let policy = bridge::ResolvedReviewPolicy {
        requirements,
        reviewer_harness: bridge::HarnessKind::Codex,
        provider_diversified: false,
        selection_reason: "risk:same-provider-high-reasoning-fallback".to_string(),
    };
    let prompt = bridge::bound_independent_reviewer_prompt(&request, &state, &policy, &evidence)
        .expect("review prompt");

    for required in [
        requirements_digest.as_str(),
        evidence_digest.as_str(),
        "crates/example",
        "crates/example/src/event_producer.rs",
        "crates/example/src/event_consumer.rs",
        "qa/integration/command-000.json",
    ] {
        assert!(prompt.contains(required), "missing {required}: {prompt}");
    }
}

#[test]
fn structured_review_rejects_blocking_findings() {
    // Break caught: an lgtm label overriding a nonempty list of blocking findings.
    let body = review_json(COMMIT).replace(
        r#""blocking_findings":[]"#,
        r#""blocking_findings":["race remains"]"#,
    );
    let error = bridge::parse_review_verdict(&body, COMMIT, &integration_citations())
        .expect_err("blocking findings must fail closed");

    assert!(error.contains("blocking_findings"), "{error}");
}

#[cfg(unix)]
#[test]
fn structured_review_normalizer_emits_legacy_lgtm_after_json_validation() {
    // Break caught: valid structured evidence not reaching the legacy exact-LGTM state machine.
    let root = super::support_base::test_root("structured-normalizer-valid");
    let harness = root.join("reviewer");
    super::support_base::write_executable(
        &harness,
        &format!("#!/bin/sh\nprintf '%s\\n' '{}'\n", review_json(COMMIT)),
    );
    let artifact_root = root.join("review-artifacts");
    bridge::ensure_private_directory(&artifact_root).unwrap();
    let invocation = bridge::ValidatedInvocation {
        program: fs::canonicalize(&harness).unwrap(),
        argv_zero: None,
        args: Vec::new(),
        current_dir: root.clone(),
        environment_overrides: Vec::new(),
    };
    let automatic = bridge::prepare_bound_reviewer_normalizer(
        bridge::HarnessKind::Claude,
        &invocation,
        &artifact_root,
        COMMIT,
        &integration_citations(),
    )
    .expect("structured normalizer");

    let output = Command::new("/bin/sh")
        .arg(&automatic.normalizer)
        .current_dir(&root)
        .output()
        .expect("run structured normalizer");
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"LGTM\n");
    assert_eq!(
        fs::read_to_string(automatic.result).unwrap().trim(),
        review_json(COMMIT)
    );
}

#[cfg(unix)]
#[test]
fn structured_review_normalizer_rejects_wrong_commit_before_lgtm() {
    // Break caught: normalizer emitting LGTM for structured evidence bound to another commit.
    let root = super::support_base::test_root("structured-normalizer-wrong-commit");
    let harness = root.join("reviewer");
    super::support_base::write_executable(
        &harness,
        &format!(
            "#!/bin/sh\nprintf '%s\\n' '{}'\n",
            review_json("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
        ),
    );
    let artifact_root = root.join("review-artifacts");
    bridge::ensure_private_directory(&artifact_root).unwrap();
    let invocation = bridge::ValidatedInvocation {
        program: fs::canonicalize(&harness).unwrap(),
        argv_zero: None,
        args: Vec::new(),
        current_dir: root.clone(),
        environment_overrides: Vec::new(),
    };
    let automatic = bridge::prepare_bound_reviewer_normalizer(
        bridge::HarnessKind::Claude,
        &invocation,
        &artifact_root,
        COMMIT,
        &integration_citations(),
    )
    .expect("structured normalizer");

    let output = Command::new("/bin/sh")
        .arg(&automatic.normalizer)
        .current_dir(&root)
        .output()
        .expect("run structured normalizer");
    assert!(!output.status.success(), "{output:?}");
    assert_ne!(output.stdout, b"LGTM\n");
}

#[cfg(unix)]
#[test]
fn structured_review_normalizer_rejects_unbound_integration_citations() {
    // Break caught: the trusted normalizer accepting plausible but unbound integration prose.
    let root = super::support_base::test_root("structured-normalizer-unbound-integration");
    let harness = root.join("reviewer");
    let unbound = review_json_with_citations(COMMIT, &["login -> session".to_string()]);
    super::support_base::write_executable(
        &harness,
        &format!("#!/bin/sh\nprintf '%s\\n' '{}'\n", unbound),
    );
    let artifact_root = root.join("review-artifacts");
    bridge::ensure_private_directory(&artifact_root).unwrap();
    let invocation = bridge::ValidatedInvocation {
        program: fs::canonicalize(&harness).unwrap(),
        argv_zero: None,
        args: Vec::new(),
        current_dir: root.clone(),
        environment_overrides: Vec::new(),
    };
    let automatic = bridge::prepare_bound_reviewer_normalizer(
        bridge::HarnessKind::Claude,
        &invocation,
        &artifact_root,
        COMMIT,
        &integration_citations(),
    )
    .expect("structured normalizer");

    let output = Command::new("/bin/sh")
        .arg(&automatic.normalizer)
        .current_dir(&root)
        .output()
        .expect("run structured normalizer");

    assert!(!output.status.success(), "{output:?}");
    assert_ne!(output.stdout, b"LGTM\n");
}
