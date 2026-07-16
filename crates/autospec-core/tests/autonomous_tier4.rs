use autospec_core::autonomous::config::Tier4SourceDescriptor;
use autospec_core::autonomous::no_work::DryReason;
use autospec_core::autonomous::tier4::{
    evaluate_tier4, Tier4Candidate, Tier4Failure, Tier4FailureCode, Tier4GeneratedCandidates,
    Tier4Input, Tier4RoiPolicy, Tier4SourceEnvelope, Tier4SourceFact, Tier4SourcePolicy,
    Tier4Stage, Tier4StageResult, Tier4Verification, Tier4VerifierVerdicts, DISABLED_REASON,
    TIER4_RANK_LIMIT, TIER4_SCHEMA,
};

fn descriptor(id: &str) -> Tier4SourceDescriptor {
    Tier4SourceDescriptor {
        id: id.to_string(),
        host: format!("{id}.example.test"),
        path: format!("/{id}"),
        max_bytes: 1_024,
        deadline_millis: 1_000,
    }
}

fn policy(ids: &[&str]) -> Tier4SourcePolicy {
    Tier4SourcePolicy {
        schema_version: TIER4_SCHEMA,
        policy_identity: "checked-in-policy-v1".to_string(),
        descriptors: ids.iter().map(|id| descriptor(id)).collect(),
    }
}

fn source(id: &str, facts: &[&str]) -> Tier4SourceEnvelope {
    Tier4SourceEnvelope {
        schema_version: TIER4_SCHEMA,
        producer_identity: "test-source-adapter".to_string(),
        producer_protocol_version: "v1".to_string(),
        source_id: id.to_string(),
        byte_length: 128,
        body_sha256: "a".repeat(64),
        facts: facts
            .iter()
            .map(|key| Tier4SourceFact {
                fact_key: (*key).to_string(),
                fact_type: "release".to_string(),
                value: format!("fact for {key}"),
            })
            .collect(),
    }
}

fn candidate(key: &str, source_id: &str, fact_key: &str) -> Tier4Candidate {
    Tier4Candidate {
        stable_key: key.to_string(),
        source_id: source_id.to_string(),
        fact_key: fact_key.to_string(),
        title: format!("Investigate {key}"),
        rationale: format!("typed fact {fact_key} supports {key}"),
    }
}

fn generated(candidates: Vec<Tier4Candidate>) -> Tier4GeneratedCandidates {
    Tier4GeneratedCandidates {
        schema_version: TIER4_SCHEMA,
        generator_identity: "test-generator".to_string(),
        generator_protocol_version: "v1".to_string(),
        candidates,
    }
}

fn accepted(key: &str, roi_millis: u16) -> Tier4Verification {
    Tier4Verification::Accepted {
        stable_key: key.to_string(),
        roi_millis,
        reason: "verified against typed source evidence".to_string(),
    }
}

fn rejected(key: &str) -> Tier4Verification {
    Tier4Verification::Rejected {
        stable_key: key.to_string(),
        reason: "not actionable".to_string(),
    }
}

fn verifier(verdicts: Vec<Tier4Verification>) -> Tier4VerifierVerdicts {
    Tier4VerifierVerdicts {
        schema_version: TIER4_SCHEMA,
        verifier_identity: "test-verifier".to_string(),
        verifier_protocol_version: "v1".to_string(),
        verdicts,
    }
}

fn enabled(
    sources: Vec<Tier4SourceEnvelope>,
    candidates: Vec<Tier4Candidate>,
    verdicts: Vec<Tier4Verification>,
) -> Tier4Input {
    Tier4Input::Enabled {
        source_policy: policy(
            &sources
                .iter()
                .map(|source| source.source_id.as_str())
                .collect::<Vec<_>>(),
        ),
        sources: sources
            .into_iter()
            .map(Tier4StageResult::Complete)
            .collect(),
        generated: Tier4StageResult::Complete(generated(candidates)),
        verifier: Tier4StageResult::Complete(verifier(verdicts)),
        roi_policy: Tier4RoiPolicy::v1(),
    }
}

fn assert_failure(
    result: Result<autospec_core::autonomous::tier4::Tier4Evaluation, Tier4Failure>,
    stage: Tier4Stage,
    code: Tier4FailureCode,
) -> Tier4Failure {
    let failure = result.expect_err("invalid Tier 4 input must fail closed");
    assert_eq!((failure.stage(), failure.code()), (stage, code));
    assert!(
        failure.documents().is_some(),
        "evaluator failures are sealed"
    );
    failure
}

#[test]
fn disabled_policy_is_exact_and_has_no_observation() {
    let evaluation = evaluate_tier4(Tier4Input::DisabledByCheckedInPolicy).expect("policy result");

    assert_eq!(evaluation.observation(), None);
    assert_eq!(
        evaluation.not_run_reason(),
        Some("tier4_external_discovery_disabled_by_checked_in_policy")
    );
    assert_eq!(
        DISABLED_REASON,
        "tier4_external_discovery_disabled_by_checked_in_policy"
    );
}

#[test]
fn source_policy_revalidates_descriptors_and_requires_ordered_complete_coverage() {
    let mut invalid_policy = policy(&["alpha"]);
    invalid_policy.descriptors[0].host = "HTTPS://alpha.example.test".to_string();
    let invalid = Tier4Input::Enabled {
        source_policy: invalid_policy,
        sources: Vec::new(),
        generated: Tier4StageResult::Complete(generated(Vec::new())),
        verifier: Tier4StageResult::Complete(verifier(Vec::new())),
        roi_policy: Tier4RoiPolicy::v1(),
    };
    assert_failure(
        evaluate_tier4(invalid),
        Tier4Stage::SourcePolicy,
        Tier4FailureCode::InvalidSourcePolicy,
    );

    let mut numeric_host_policy = policy(&["alpha"]);
    numeric_host_policy.descriptors[0].host = "0x7f.0x0.0x0.0x1".to_string();
    let numeric_host = Tier4Input::Enabled {
        source_policy: numeric_host_policy,
        sources: Vec::new(),
        generated: Tier4StageResult::Complete(generated(Vec::new())),
        verifier: Tier4StageResult::Complete(verifier(Vec::new())),
        roi_policy: Tier4RoiPolicy::v1(),
    };
    assert_failure(
        evaluate_tier4(numeric_host),
        Tier4Stage::SourcePolicy,
        Tier4FailureCode::InvalidSourcePolicy,
    );

    let ordered_policy = policy(&["alpha", "beta"]);
    let out_of_order = Tier4Input::Enabled {
        source_policy: ordered_policy,
        sources: vec![
            Tier4StageResult::Complete(source("beta", &["b"])),
            Tier4StageResult::Complete(source("alpha", &["a"])),
        ],
        generated: Tier4StageResult::Complete(generated(Vec::new())),
        verifier: Tier4StageResult::Complete(verifier(Vec::new())),
        roi_policy: Tier4RoiPolicy::v1(),
    };
    let failure = assert_failure(
        evaluate_tier4(out_of_order),
        Tier4Stage::Sources,
        Tier4FailureCode::InvalidSourceCoverage,
    );
    assert!(failure.partial_evidence().has_source_policy());
    assert!(!failure.partial_evidence().has_sources());
}

#[test]
fn sources_validate_sealed_digest_body_cap_and_fact_limit() {
    let mut empty = source("alpha", &[]);
    empty.byte_length = 0;
    let empty_result = evaluate_tier4(enabled(vec![empty], Vec::new(), Vec::new()))
        .expect("a sealed empty source remains a completed source stage");
    assert_eq!(
        empty_result
            .observation()
            .expect("completed observation")
            .terminal_dry_reason(),
        Some(DryReason::NoProposalsGenerated)
    );

    let mut oversized = source("alpha", &["a"]);
    oversized.byte_length = 1_025;
    assert_failure(
        evaluate_tier4(enabled(vec![oversized], Vec::new(), Vec::new())),
        Tier4Stage::Sources,
        Tier4FailureCode::InvalidSourceEnvelope,
    );

    let mut invalid_digest = source("alpha", &["a"]);
    invalid_digest.body_sha256 = "A".repeat(64);
    assert_failure(
        evaluate_tier4(enabled(vec![invalid_digest], Vec::new(), Vec::new())),
        Tier4Stage::Sources,
        Tier4FailureCode::InvalidSourceEnvelope,
    );

    let keys = (0..129)
        .map(|index| format!("fact-{index}"))
        .collect::<Vec<_>>();
    let mut fact_limited = source("alpha", &[]);
    fact_limited.facts = keys
        .iter()
        .map(|key| Tier4SourceFact {
            fact_key: key.clone(),
            fact_type: "release".to_string(),
            value: "bounded typed fact".to_string(),
        })
        .collect();
    assert_failure(
        evaluate_tier4(enabled(vec![fact_limited], Vec::new(), Vec::new())),
        Tier4Stage::Sources,
        Tier4FailureCode::InvalidSourceFact,
    );
}

#[test]
fn candidates_must_reference_observed_facts_and_deduplicate_by_matching_semantics() {
    let unknown_fact = candidate("unknown", "alpha", "missing");
    assert_failure(
        evaluate_tier4(enabled(
            vec![source("alpha", &["a"])],
            vec![unknown_fact],
            vec![accepted("unknown", 900)],
        )),
        Tier4Stage::Generator,
        Tier4FailureCode::InvalidCandidate,
    );

    let first = candidate("same", "alpha", "a");
    let mut conflict = candidate("same", "alpha", "b");
    conflict.title = "A different semantic candidate".to_string();
    assert_failure(
        evaluate_tier4(enabled(
            vec![source("alpha", &["a", "b"])],
            vec![first, conflict],
            vec![accepted("same", 900)],
        )),
        Tier4Stage::Deduplicator,
        Tier4FailureCode::DuplicateConflict,
    );

    let matching_first = candidate("same", "alpha", "a");
    let mut matching_second = candidate("same", "alpha", "b");
    matching_second.rationale = matching_first.rationale.clone();
    let evaluation = evaluate_tier4(enabled(
        vec![source("alpha", &["a", "b"])],
        vec![matching_first, matching_second],
        vec![accepted("same", 900)],
    ))
    .expect("matching duplicate candidates are deterministic");
    let observation = evaluation.observation().expect("completed observation");
    assert_eq!(observation.funnel().observed, 2);
    assert_eq!(observation.funnel().deduplicated, 1);
    assert_eq!(observation.deduplication().groups.len(), 1);
}

#[test]
fn verifier_coverage_and_completed_empty_paths_are_closed() {
    assert_failure(
        evaluate_tier4(enabled(
            vec![source("alpha", &["a"])],
            vec![candidate("one", "alpha", "a")],
            Vec::new(),
        )),
        Tier4Stage::Verifier,
        Tier4FailureCode::InvalidVerdictCoverage,
    );

    let generated_empty = evaluate_tier4(enabled(
        vec![source("alpha", &["a"])],
        Vec::new(),
        Vec::new(),
    ))
    .expect("completed empty generation is dry");
    assert_eq!(
        generated_empty
            .observation()
            .expect("observation")
            .terminal_dry_reason(),
        Some(DryReason::NoProposalsGenerated)
    );

    let rejected = evaluate_tier4(enabled(
        vec![source("alpha", &["a"])],
        vec![candidate("one", "alpha", "a")],
        vec![rejected("one")],
    ))
    .expect("completed verifier rejection is dry");
    assert_eq!(
        rejected
            .observation()
            .expect("observation")
            .terminal_dry_reason(),
        Some(DryReason::VerificationRejected)
    );

    let roi_filtered = evaluate_tier4(enabled(
        vec![source("alpha", &["a"])],
        vec![candidate("one", "alpha", "a")],
        vec![accepted("one", 499)],
    ))
    .expect("completed ROI rejection is dry");
    assert_eq!(
        roi_filtered
            .observation()
            .expect("observation")
            .terminal_dry_reason(),
        Some(DryReason::RoiFiltered)
    );
}

#[test]
fn roi_ranking_is_descending_stable_and_capped_with_exact_funnel_counts() {
    let keys = (0..12)
        .map(|index| format!("fact-{index:02}"))
        .collect::<Vec<_>>();
    let candidates = keys
        .iter()
        .map(|key| candidate(&format!("candidate-{key}"), "alpha", key))
        .collect::<Vec<_>>();
    let verdicts = keys
        .iter()
        .enumerate()
        .map(|(index, key)| {
            accepted(
                &format!("candidate-{key}"),
                if index < 2 { 900 } else { 800 },
            )
        })
        .collect::<Vec<_>>();

    let evaluation = evaluate_tier4(enabled(
        vec![source(
            "alpha",
            &keys.iter().map(String::as_str).collect::<Vec<_>>(),
        )],
        candidates,
        verdicts,
    ))
    .expect("valid candidates rank deterministically");
    let observation = evaluation.observation().expect("observation");
    assert_eq!(
        (
            observation.funnel().observed,
            observation.funnel().deduplicated,
            observation.funnel().verified,
            observation.funnel().roi_approved,
            observation.funnel().ranked,
        ),
        (12, 12, 12, 12, TIER4_RANK_LIMIT)
    );
    assert_eq!(observation.ranked().len(), TIER4_RANK_LIMIT as usize);
    assert_eq!(observation.ranked()[0].stable_key, "candidate-fact-00");
    assert_eq!(observation.ranked()[1].stable_key, "candidate-fact-01");
    assert!(observation
        .ranked()
        .windows(2)
        .all(|pair| pair[0].roi_millis >= pair[1].roi_millis));
}

#[test]
fn malformed_stage_results_and_roi_policy_are_sealed_failures() {
    let missing_sources = Tier4Input::Enabled {
        source_policy: policy(&["alpha"]),
        sources: vec![Tier4StageResult::Missing],
        generated: Tier4StageResult::Complete(generated(Vec::new())),
        verifier: Tier4StageResult::Complete(verifier(Vec::new())),
        roi_policy: Tier4RoiPolicy::v1(),
    };
    assert_failure(
        evaluate_tier4(missing_sources),
        Tier4Stage::Sources,
        Tier4FailureCode::MissingStageResult,
    );

    let invalid_roi = Tier4Input::Enabled {
        source_policy: policy(&["alpha"]),
        sources: vec![Tier4StageResult::Complete(source("alpha", &["a"]))],
        generated: Tier4StageResult::Complete(generated(vec![candidate("one", "alpha", "a")])),
        verifier: Tier4StageResult::Complete(verifier(vec![accepted("one", 900)])),
        roi_policy: Tier4RoiPolicy {
            threshold_millis: 499,
            scale_millis: 1_000,
        },
    };
    assert_failure(
        evaluate_tier4(invalid_roi),
        Tier4Stage::RoiRank,
        Tier4FailureCode::InvalidRoiPolicy,
    );
}
