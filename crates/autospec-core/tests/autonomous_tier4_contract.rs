use autospec_core::autonomous::config::Tier4SourceDescriptor;
use autospec_core::autonomous::tier4::{
    evaluate_tier4, Tier4Candidate, Tier4GeneratedCandidates, Tier4Input, Tier4RoiPolicy,
    Tier4SourceEnvelope, Tier4SourceFact, Tier4SourcePolicy, Tier4StageResult, Tier4Verification,
    Tier4VerifierVerdicts, TIER4_SCHEMA,
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

fn source() -> Tier4SourceEnvelope {
    Tier4SourceEnvelope {
        schema_version: TIER4_SCHEMA,
        producer_identity: "typed-source-v1".to_string(),
        producer_protocol_version: "v1".to_string(),
        source_id: "release-feed".to_string(),
        byte_length: 17,
        body_sha256: "b".repeat(64),
        facts: vec![
            Tier4SourceFact {
                fact_key: "release-1".to_string(),
                fact_type: "release".to_string(),
                value: "version 1.2.3".to_string(),
            },
            Tier4SourceFact {
                fact_key: "release-2".to_string(),
                fact_type: "release".to_string(),
                value: "version 1.2.4".to_string(),
            },
        ],
    }
}

fn complete() -> autospec_core::autonomous::tier4::Tier4Evaluation {
    complete_with_source(source())
}

fn complete_with_source(
    source: Tier4SourceEnvelope,
) -> autospec_core::autonomous::tier4::Tier4Evaluation {
    evaluate_tier4(Tier4Input::Enabled {
        source_policy: policy(),
        sources: vec![Tier4StageResult::Complete(source)],
        generated: Tier4StageResult::Complete(Tier4GeneratedCandidates {
            schema_version: TIER4_SCHEMA,
            generator_identity: "typed-generator-v1".to_string(),
            generator_protocol_version: "v1".to_string(),
            candidates: vec![Tier4Candidate {
                stable_key: "upgrade-1-2-3".to_string(),
                source_id: "release-feed".to_string(),
                fact_key: "release-1".to_string(),
                title: "Evaluate version 1.2.3".to_string(),
                rationale: "sealed source fact reports a release".to_string(),
            }],
        }),
        verifier: Tier4StageResult::Complete(Tier4VerifierVerdicts {
            schema_version: TIER4_SCHEMA,
            verifier_identity: "typed-verifier-v1".to_string(),
            verifier_protocol_version: "v1".to_string(),
            verdicts: vec![Tier4Verification::Accepted {
                stable_key: "upgrade-1-2-3".to_string(),
                roi_millis: 900,
                reason: "benefit exceeds fixed threshold".to_string(),
            }],
        }),
        roi_policy: Tier4RoiPolicy::v1(),
    })
    .expect("complete Tier 4 fixture")
}

fn assert_canonical(document: &str) {
    assert!(document.ends_with('\n'));
    assert!(!document[..document.len() - 1].contains(['\n', '\r', '\t']));
    assert!(document.starts_with("{\"schema\":1,"));
}

#[test]
fn documents_are_canonical_and_chain_each_completed_stage() {
    let observation = complete().observation().expect("observation").clone();
    let documents = observation.documents();

    let source_policy = documents
        .source_policy_json()
        .expect("source-policy document");
    assert_canonical(&source_policy);
    assert!(source_policy.contains("\"kind\":\"tier4_source_policy\""));
    assert!(!source_policy.contains("predecessor_digest"));

    let source_policy_digest = sha256_hex(source_policy.as_bytes());
    let sources = documents
        .sources_json(&source_policy_digest)
        .expect("sources document result")
        .expect("sources document");
    let sources_digest = sha256_hex(sources.as_bytes());
    let generated = documents
        .generated_json(&sources_digest)
        .expect("generated document result")
        .expect("generated document");
    let generated_digest = sha256_hex(generated.as_bytes());
    let dedup = documents
        .dedup_json(&generated_digest)
        .expect("dedup document result")
        .expect("dedup document");
    let dedup_digest = sha256_hex(dedup.as_bytes());
    let verification = documents
        .verification_json(&dedup_digest)
        .expect("verification document result")
        .expect("verification document");
    let verification_digest = sha256_hex(verification.as_bytes());
    let roi_rank = documents
        .roi_rank_json(&verification_digest)
        .expect("rank document result")
        .expect("rank document");
    for (document, predecessor) in [
        (&sources, &source_policy_digest),
        (&generated, &sources_digest),
        (&dedup, &generated_digest),
        (&verification, &dedup_digest),
        (&roi_rank, &verification_digest),
    ] {
        assert_canonical(document);
        assert!(document.contains(&format!("\"predecessor_digest\":\"{predecessor}\"")));
    }
    assert!(sources.contains("\"body_sha256\""));
    assert!(!sources.contains("raw_body"));
    assert!(generated.contains("\"source_id\":\"release-feed\""));
    assert!(dedup.contains("\"stable_key\":\"upgrade-1-2-3\""));
    assert!(verification.contains("\"result\":\"accepted\""));
    assert!(roi_rank.contains("\"rank\":1"));
    assert!(documents.sources_json(WRONG_DIGEST).is_err());
    assert!(documents.generated_json(WRONG_DIGEST).is_err());
    assert!(documents.dedup_json(WRONG_DIGEST).is_err());
    assert!(documents.verification_json(WRONG_DIGEST).is_err());
    assert!(documents.roi_rank_json(WRONG_DIGEST).is_err());
}

#[test]
fn source_document_is_canonical_across_equivalent_fact_ordering() {
    let ordered = complete()
        .observation()
        .expect("ordered observation")
        .clone();
    let mut reordered_source = source();
    reordered_source.facts.reverse();
    let reordered = complete_with_source(reordered_source)
        .observation()
        .expect("reordered observation")
        .clone();

    let ordered_documents = ordered.documents();
    let ordered_policy = ordered_documents
        .source_policy_json()
        .expect("ordered policy document");
    let ordered_sources = ordered_documents
        .sources_json(&sha256_hex(ordered_policy.as_bytes()))
        .expect("ordered source result")
        .expect("ordered source document");
    let reordered_documents = reordered.documents();
    let reordered_policy = reordered_documents
        .source_policy_json()
        .expect("reordered policy document");
    let reordered_sources = reordered_documents
        .sources_json(&sha256_hex(reordered_policy.as_bytes()))
        .expect("reordered source result")
        .expect("reordered source document");

    assert_eq!(ordered_policy, reordered_policy);
    assert_eq!(ordered_sources, reordered_sources);
}

#[test]
fn failure_documents_only_allow_a_null_predecessor_for_source_policy_failure() {
    let source_policy_failure = evaluate_tier4(Tier4Input::Enabled {
        source_policy: Tier4SourcePolicy {
            schema_version: TIER4_SCHEMA,
            policy_identity: "invalid".to_string(),
            descriptors: Vec::new(),
        },
        sources: Vec::new(),
        generated: Tier4StageResult::Complete(Tier4GeneratedCandidates {
            schema_version: TIER4_SCHEMA,
            generator_identity: "generator".to_string(),
            generator_protocol_version: "v1".to_string(),
            candidates: Vec::new(),
        }),
        verifier: Tier4StageResult::Complete(Tier4VerifierVerdicts {
            schema_version: TIER4_SCHEMA,
            verifier_identity: "verifier".to_string(),
            verifier_protocol_version: "v1".to_string(),
            verdicts: Vec::new(),
        }),
        roi_policy: Tier4RoiPolicy::v1(),
    })
    .expect_err("invalid policy fails");
    let policy_documents = source_policy_failure.documents().expect("sealed failure");
    let policy_failure = policy_documents
        .failure_json(None)
        .expect("null predecessor for source policy");
    assert_canonical(&policy_failure);
    assert!(policy_failure.contains("\"predecessor_digest\":null"));
    assert!(policy_documents.failure_json(Some(WRONG_DIGEST)).is_err());

    let source_failure = evaluate_tier4(Tier4Input::Enabled {
        source_policy: policy(),
        sources: vec![Tier4StageResult::Missing],
        generated: Tier4StageResult::Complete(Tier4GeneratedCandidates {
            schema_version: TIER4_SCHEMA,
            generator_identity: "generator".to_string(),
            generator_protocol_version: "v1".to_string(),
            candidates: Vec::new(),
        }),
        verifier: Tier4StageResult::Complete(Tier4VerifierVerdicts {
            schema_version: TIER4_SCHEMA,
            verifier_identity: "verifier".to_string(),
            verifier_protocol_version: "v1".to_string(),
            verdicts: Vec::new(),
        }),
        roi_policy: Tier4RoiPolicy::v1(),
    })
    .expect_err("missing source stage fails");
    let source_documents = source_failure.documents().expect("sealed failure");
    let source_policy = source_documents
        .source_policy_json()
        .expect("completed source policy document");
    let source_policy_digest = sha256_hex(source_policy.as_bytes());
    assert!(source_documents.failure_json(None).is_err());
    assert!(source_documents.failure_json(Some(WRONG_DIGEST)).is_err());
    let failure = source_documents
        .failure_json(Some(&source_policy_digest))
        .expect("source failure requires prior digest");
    assert_canonical(&failure);
    assert!(failure.contains(&format!(
        "\"predecessor_digest\":\"{source_policy_digest}\""
    )));
}
