use autospec_core::autonomous::config::Tier4SourceDescriptor;
use autospec_core::autonomous::tier4::{
    evaluate_tier4, Tier4Candidate, Tier4Failure, Tier4FailureCode, Tier4GeneratedCandidates,
    Tier4Input, Tier4RoiPolicy, Tier4SourceEnvelope, Tier4SourceFact, Tier4SourcePolicy,
    Tier4Stage, Tier4StageResult, Tier4Verification, Tier4VerifierVerdicts, TIER4_SCHEMA,
};
use autospec_core::autonomous::waterfall::sha256_hex;

const WRONG_DIGEST: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn policy() -> Tier4SourcePolicy {
    Tier4SourcePolicy {
        schema_version: TIER4_SCHEMA,
        policy_identity: "checked-in-policy-v1".to_string(),
        descriptors: vec![Tier4SourceDescriptor {
            id: "release-feed".to_string(),
            host: "api.example.test".to_string(),
            path: "/v1/releases".to_string(),
            max_bytes: 65_536,
            deadline_millis: 5_000,
        }],
    }
}

fn source(facts: &[&str]) -> Tier4SourceEnvelope {
    Tier4SourceEnvelope {
        schema_version: TIER4_SCHEMA,
        producer_identity: "typed-source-v1".to_string(),
        producer_protocol_version: "v1".to_string(),
        source_id: "release-feed".to_string(),
        byte_length: 17,
        body_sha256: "b".repeat(64),
        facts: facts
            .iter()
            .map(|fact_key| Tier4SourceFact {
                fact_key: (*fact_key).to_string(),
                fact_type: "release".to_string(),
                value: format!("version for {fact_key}"),
            })
            .collect(),
    }
}

fn candidate(stable_key: &str, fact_key: &str) -> Tier4Candidate {
    Tier4Candidate {
        stable_key: stable_key.to_string(),
        source_id: "release-feed".to_string(),
        fact_key: fact_key.to_string(),
        title: format!("Evaluate {stable_key}"),
        rationale: "sealed source fact supports this candidate".to_string(),
    }
}

fn generated(candidates: Vec<Tier4Candidate>) -> Tier4GeneratedCandidates {
    Tier4GeneratedCandidates {
        schema_version: TIER4_SCHEMA,
        generator_identity: "typed-generator-v1".to_string(),
        generator_protocol_version: "v1".to_string(),
        candidates,
    }
}

fn verifier(verdicts: Vec<Tier4Verification>) -> Tier4VerifierVerdicts {
    Tier4VerifierVerdicts {
        schema_version: TIER4_SCHEMA,
        verifier_identity: "typed-verifier-v1".to_string(),
        verifier_protocol_version: "v1".to_string(),
        verdicts,
    }
}

fn accepted(stable_key: &str) -> Tier4Verification {
    Tier4Verification::Accepted {
        stable_key: stable_key.to_string(),
        roi_millis: 900,
        reason: "benefit exceeds fixed threshold".to_string(),
    }
}

fn input(
    source: Tier4StageResult<Tier4SourceEnvelope>,
    generated: Tier4StageResult<Tier4GeneratedCandidates>,
    verifier: Tier4StageResult<Tier4VerifierVerdicts>,
    roi_policy: Tier4RoiPolicy,
) -> Tier4Input {
    Tier4Input::Enabled {
        source_policy: policy(),
        sources: vec![source],
        generated,
        verifier,
        roi_policy,
    }
}

fn injected_failure() -> Tier4Failure {
    Tier4Failure::new(
        Tier4Stage::SourcePolicy,
        Tier4FailureCode::InvalidCandidate,
        "injected stage failure",
    )
    .expect("valid injected failure")
}

fn assert_prefix(failure: &Tier4Failure, stage: Tier4Stage, expected: [bool; 5]) {
    assert_eq!(failure.stage(), stage);
    let partial = failure.partial_evidence();
    assert_eq!(
        [
            partial.has_source_policy(),
            partial.has_sources(),
            partial.has_generated(),
            partial.has_deduplication(),
            partial.has_verification(),
        ],
        expected
    );

    let documents = failure.documents().expect("sealed failure documents");
    let policy = documents.source_policy_json().expect("policy prefix");
    let first_digest = sha256_hex(policy.as_bytes());
    let mut predecessor = first_digest.clone();
    if partial.has_sources() {
        let sources = documents
            .sources_json(&predecessor)
            .expect("sources result")
            .expect("sources prefix");
        predecessor = sha256_hex(sources.as_bytes());
    }
    if partial.has_generated() {
        let generated = documents
            .generated_json(&predecessor)
            .expect("generated result")
            .expect("generated prefix");
        predecessor = sha256_hex(generated.as_bytes());
    }
    if partial.has_deduplication() {
        let dedup = documents
            .dedup_json(&predecessor)
            .expect("dedup result")
            .expect("dedup prefix");
        predecessor = sha256_hex(dedup.as_bytes());
    }
    if partial.has_verification() {
        let verification = documents
            .verification_json(&predecessor)
            .expect("verification result")
            .expect("verification prefix");
        predecessor = sha256_hex(verification.as_bytes());
    }
    let document = documents
        .failure_json(Some(&predecessor))
        .expect("failure document uses latest completed prefix");
    assert!(document.contains(&format!("\"predecessor_digest\":\"{predecessor}\"")));
    assert!(document.contains(&format!("\"stage\":\"{}\"", stage.as_str())));
    assert!(documents.failure_json(Some(WRONG_DIGEST)).is_err());
    if predecessor != first_digest {
        assert!(documents.failure_json(Some(&first_digest)).is_err());
    }
}

#[test]
fn stage_failures_and_internal_failures_seal_exact_completed_prefixes() {
    let source_failure = evaluate_tier4(input(
        Tier4StageResult::Failed(injected_failure()),
        Tier4StageResult::Complete(generated(vec![candidate("one", "a")])),
        Tier4StageResult::Complete(verifier(vec![accepted("one")])),
        Tier4RoiPolicy::v1(),
    ))
    .expect_err("injected source failure");
    assert_prefix(
        &source_failure,
        Tier4Stage::Sources,
        [true, false, false, false, false],
    );

    let generator_failure = evaluate_tier4(input(
        Tier4StageResult::Complete(source(&["a"])),
        Tier4StageResult::Failed(injected_failure()),
        Tier4StageResult::Complete(verifier(vec![accepted("one")])),
        Tier4RoiPolicy::v1(),
    ))
    .expect_err("injected generator failure");
    assert_prefix(
        &generator_failure,
        Tier4Stage::Generator,
        [true, true, false, false, false],
    );

    let mut conflicting = candidate("one", "b");
    conflicting.title = "different candidate semantics".to_string();
    let dedup_failure = evaluate_tier4(input(
        Tier4StageResult::Complete(source(&["a", "b"])),
        Tier4StageResult::Complete(generated(vec![candidate("one", "a"), conflicting])),
        Tier4StageResult::Complete(verifier(vec![accepted("one")])),
        Tier4RoiPolicy::v1(),
    ))
    .expect_err("deduplication conflict");
    assert_prefix(
        &dedup_failure,
        Tier4Stage::Deduplicator,
        [true, true, true, false, false],
    );

    let verifier_failure = evaluate_tier4(input(
        Tier4StageResult::Complete(source(&["a"])),
        Tier4StageResult::Complete(generated(vec![candidate("one", "a")])),
        Tier4StageResult::Failed(injected_failure()),
        Tier4RoiPolicy::v1(),
    ))
    .expect_err("injected verifier failure");
    assert_prefix(
        &verifier_failure,
        Tier4Stage::Verifier,
        [true, true, true, true, false],
    );

    let roi_failure = evaluate_tier4(input(
        Tier4StageResult::Complete(source(&["a"])),
        Tier4StageResult::Complete(generated(vec![candidate("one", "a")])),
        Tier4StageResult::Complete(verifier(vec![accepted("one")])),
        Tier4RoiPolicy {
            threshold_millis: 499,
            scale_millis: 1_000,
        },
    ))
    .expect_err("invalid ROI policy");
    assert_prefix(
        &roi_failure,
        Tier4Stage::RoiRank,
        [true, true, true, true, true],
    );
}
