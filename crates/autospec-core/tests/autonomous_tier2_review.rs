use autospec_core::autonomous::tier2::{
    evaluate_tier2, StrictCollectorEvidence, Tier2Complexity, Tier2Failure, Tier2FailureCode,
    Tier2GeneratedProposals, Tier2Input, Tier2Proposal, Tier2RoiPolicy, Tier2Severity, Tier2Source,
    Tier2Stage, Tier2StageResult, Tier2Verification, Tier2VerifierVerdicts, TIER2_RANK_LIMIT,
};
use autospec_core::explore::specialists::{DetectedDomain, FileLineEvidence};
use std::fs;
use std::path::Path;

fn evidence() -> FileLineEvidence {
    FileLineEvidence {
        file: "Cargo.toml".to_string(),
        line: 1,
        r#match: "trading".to_string(),
    }
}

fn collector() -> StrictCollectorEvidence {
    StrictCollectorEvidence {
        schema_version: 1,
        collector_version: "strict-local-v1".to_string(),
        canonical_repo_scope: "/repo".to_string(),
        domains: vec![DetectedDomain {
            name: "trading".to_string(),
            score: 1,
            evidence: vec![evidence()],
        }],
    }
}

fn proposal(key: &str) -> Tier2Proposal {
    Tier2Proposal {
        stable_key: key.to_string(),
        title: format!("feat: {key}"),
        source: Tier2Source::StrictLocalSpecialist,
        evidence: vec![evidence()],
        severity: Tier2Severity::Medium,
        confidence_millis: 800,
        complexity: Tier2Complexity::Small,
        named_consumer: "maintainer".to_string(),
    }
}

fn generated(proposals: Vec<Tier2Proposal>) -> Tier2GeneratedProposals {
    Tier2GeneratedProposals {
        generator_identity: "test-generator".to_string(),
        generator_protocol_version: "v1".to_string(),
        proposals,
    }
}

fn verifier(key: &str) -> Tier2VerifierVerdicts {
    Tier2VerifierVerdicts {
        verifier_identity: "test-verifier".to_string(),
        verifier_protocol_version: "v1".to_string(),
        verdicts: vec![Tier2Verification::Survived {
            stable_key: key.to_string(),
            reason: "bounded evidence remains actionable".to_string(),
        }],
    }
}

fn failure(stage: Tier2Stage, code: Tier2FailureCode) -> Tier2Failure {
    Tier2Failure::new(stage, code, "injected typed failure").expect("bounded failure")
}

fn enabled(proposals: Vec<Tier2Proposal>, verdicts: Vec<Tier2Verification>) -> Tier2Input {
    Tier2Input::Enabled {
        collector: Tier2StageResult::Complete(collector()),
        generator: Tier2StageResult::Complete(generated(proposals)),
        verifier: Tier2StageResult::Complete(Tier2VerifierVerdicts {
            verifier_identity: "test-verifier".to_string(),
            verifier_protocol_version: "v1".to_string(),
            verdicts,
        }),
        roi_policy: Tier2RoiPolicy::v1(),
    }
}

fn assert_slot(error: Tier2Failure, stage: Tier2Stage, detail: &str) {
    assert_eq!(error.stage(), stage);
    assert_eq!(error.detail(), detail);
}

#[test]
fn injected_failure_stages_are_normalized_to_the_enclosing_slot() {
    let collector_error = evaluate_tier2(Tier2Input::Enabled {
        collector: Tier2StageResult::Failed(failure(
            Tier2Stage::Verifier,
            Tier2FailureCode::InvalidVerdictCoverage,
        )),
        generator: Tier2StageResult::Complete(generated(Vec::new())),
        verifier: Tier2StageResult::Complete(Tier2VerifierVerdicts {
            verifier_identity: "test-verifier".to_string(),
            verifier_protocol_version: "v1".to_string(),
            verdicts: Vec::new(),
        }),
        roi_policy: Tier2RoiPolicy::v1(),
    })
    .expect_err("collector slot fails closed");
    assert!(collector_error
        .documents()
        .expect("evaluated failure is sealed")
        .collector_json()
        .is_none());
    assert_slot(
        collector_error,
        Tier2Stage::Collector,
        "injected typed failure",
    );

    let generator_error = evaluate_tier2(Tier2Input::Enabled {
        collector: Tier2StageResult::Complete(collector()),
        generator: Tier2StageResult::Failed(failure(
            Tier2Stage::RoiRank,
            Tier2FailureCode::InvalidRanking,
        )),
        verifier: Tier2StageResult::Complete(Tier2VerifierVerdicts {
            verifier_identity: "test-verifier".to_string(),
            verifier_protocol_version: "v1".to_string(),
            verdicts: Vec::new(),
        }),
        roi_policy: Tier2RoiPolicy::v1(),
    })
    .expect_err("generator slot fails closed");
    assert!(generator_error
        .documents()
        .expect("evaluated failure is sealed")
        .collector_json()
        .is_some());
    assert_slot(
        generator_error,
        Tier2Stage::Generator,
        "injected typed failure",
    );

    let verifier_error = evaluate_tier2(Tier2Input::Enabled {
        collector: Tier2StageResult::Complete(collector()),
        generator: Tier2StageResult::Complete(generated(vec![proposal("one")])),
        verifier: Tier2StageResult::Failed(failure(
            Tier2Stage::Collector,
            Tier2FailureCode::InvalidCollectorSchema,
        )),
        roi_policy: Tier2RoiPolicy::v1(),
    })
    .expect_err("verifier slot fails closed");
    let documents = verifier_error
        .documents()
        .expect("evaluated failure is sealed");
    assert!(documents
        .deduplication_json(&"a".repeat(64))
        .unwrap()
        .is_some());
    assert!(documents
        .verification_json(&"a".repeat(64))
        .unwrap()
        .is_none());
    assert_slot(
        verifier_error,
        Tier2Stage::Verifier,
        "injected typed failure",
    );
}

#[test]
fn opaque_documents_render_metadata_and_reject_invalid_raw_inputs() {
    let digest = "a".repeat(64);
    let evaluation = evaluate_tier2(Tier2Input::Enabled {
        collector: Tier2StageResult::Complete(collector()),
        generator: Tier2StageResult::Complete(generated(vec![proposal("one")])),
        verifier: Tier2StageResult::Complete(verifier("one")),
        roi_policy: Tier2RoiPolicy::v1(),
    })
    .expect("complete evidence");
    let documents = evaluation.observation().expect("observation").documents();
    let dedup = documents
        .deduplication_json(&digest)
        .expect("valid digest")
        .expect("dedup document");
    assert!(dedup.contains("\"normalization_version\":1"));
    let roi_rank = documents
        .roi_rank_json(&digest)
        .expect("valid digest")
        .expect("ROI/rank document");
    assert!(roi_rank.contains(&format!("\"rank_limit\":{TIER2_RANK_LIMIT}")));
    assert!(roi_rank.contains("\"candidates\":["));
    assert!(roi_rank.contains("\"proposal\":{"));

    let invalid = evaluate_tier2(Tier2Input::Enabled {
        collector: Tier2StageResult::Complete(collector()),
        generator: Tier2StageResult::Complete(Tier2GeneratedProposals {
            generator_identity: " ".to_string(),
            generator_protocol_version: "v1".to_string(),
            proposals: vec![proposal("one")],
        }),
        verifier: Tier2StageResult::Complete(verifier("one")),
        roi_policy: Tier2RoiPolicy::v1(),
    })
    .expect_err("invalid raw generator cannot become validated evidence");
    let documents = invalid.documents().expect("evaluated failure is sealed");
    assert!(documents.collector_json().is_some());
    assert!(documents
        .generated_json(&digest)
        .expect("valid digest")
        .is_none());

    assert!(
        failure(Tier2Stage::Generator, Tier2FailureCode::InvalidProposal)
            .documents()
            .is_none()
    );
}

#[test]
fn collector_proposal_and_verdict_bounds_fail_before_any_receipt() {
    let mut unordered = collector();
    unordered.domains.push(DetectedDomain {
        name: "alpha".to_string(),
        score: 1,
        evidence: vec![evidence()],
    });
    let mut blank_name = collector();
    blank_name.domains[0].name = " ".to_string();
    let mut escaping_evidence = collector();
    escaping_evidence.domains[0].evidence[0].file = "../Cargo.toml".to_string();
    for invalid in [unordered, blank_name, escaping_evidence] {
        assert_eq!(
            evaluate_tier2(Tier2Input::Enabled {
                collector: Tier2StageResult::Complete(invalid),
                generator: Tier2StageResult::Complete(generated(Vec::new())),
                verifier: Tier2StageResult::Complete(Tier2VerifierVerdicts {
                    verifier_identity: "test-verifier".to_string(),
                    verifier_protocol_version: "v1".to_string(),
                    verdicts: Vec::new(),
                }),
                roi_policy: Tier2RoiPolicy::v1(),
            })
            .expect_err("invalid collector must fail")
            .stage(),
            Tier2Stage::Collector
        );
    }

    let mut blank_key = proposal("one");
    blank_key.stable_key = " ".to_string();
    let mut oversized_title = proposal("two");
    oversized_title.title = "é".repeat(201);
    let mut excessive_confidence = proposal("three");
    excessive_confidence.confidence_millis = 1001;
    for invalid in [blank_key, oversized_title, excessive_confidence] {
        assert_eq!(
            evaluate_tier2(enabled(
                vec![invalid],
                vec![Tier2Verification::Survived {
                    stable_key: "one".to_string(),
                    reason: "bounded evidence remains actionable".to_string(),
                }],
            ))
            .expect_err("invalid proposal must fail")
            .stage(),
            Tier2Stage::Generator
        );
    }

    let unknown = Tier2Verification::Survived {
        stable_key: "unknown".to_string(),
        reason: "bounded evidence remains actionable".to_string(),
    };
    let blank = Tier2Verification::Survived {
        stable_key: "one".to_string(),
        reason: " ".to_string(),
    };
    let oversized = Tier2Verification::Survived {
        stable_key: "one".to_string(),
        reason: "é".repeat(241),
    };
    for invalid in [unknown, blank, oversized] {
        assert_eq!(
            evaluate_tier2(enabled(vec![proposal("one")], vec![invalid]))
                .expect_err("invalid verdict must fail")
                .stage(),
            Tier2Stage::Verifier
        );
    }
}

#[test]
fn tie_breaks_refutations_and_stage_precedence_remain_deterministic() {
    let mut zeta = proposal("zeta");
    zeta.title = "fix: same candidate".to_string();
    let mut alpha = proposal("alpha");
    alpha.title = "feat: same candidate".to_string();
    let observation = evaluate_tier2(enabled(
        vec![zeta, alpha],
        vec![Tier2Verification::Survived {
            stable_key: "alpha".to_string(),
            reason: "bounded evidence remains actionable".to_string(),
        }],
    ))
    .expect("stable duplicate winner")
    .observation()
    .expect("observation")
    .clone();
    assert_eq!(observation.deduplication().groups[0].winner_key, "alpha");

    let mut beta = proposal("beta");
    beta.title = "beta distinct".to_string();
    let mut alpha = proposal("alpha");
    alpha.title = "alpha distinct".to_string();
    let ranked_evaluation = evaluate_tier2(enabled(
        vec![beta.clone(), alpha.clone()],
        vec![
            Tier2Verification::Survived {
                stable_key: "alpha".to_string(),
                reason: "survived".to_string(),
            },
            Tier2Verification::Survived {
                stable_key: "beta".to_string(),
                reason: "survived".to_string(),
            },
        ],
    ))
    .expect("rank tie is deterministic");
    let ranked = ranked_evaluation.observation().expect("observation");
    assert_eq!(
        ranked
            .ranked()
            .iter()
            .map(|proposal| proposal.stable_key.as_str())
            .collect::<Vec<_>>(),
        vec!["alpha", "beta"]
    );
    let refuted_evaluation = evaluate_tier2(enabled(
        vec![beta, alpha],
        vec![
            Tier2Verification::Refuted {
                stable_key: "alpha".to_string(),
                reason: "refuted".to_string(),
            },
            Tier2Verification::Refuted {
                stable_key: "beta".to_string(),
                reason: "refuted".to_string(),
            },
        ],
    ))
    .expect("refuted candidates remain a complete observation");
    let all_refuted = refuted_evaluation.observation().expect("observation");
    assert_eq!(
        (
            all_refuted.funnel().observed,
            all_refuted.funnel().deduplicated,
            all_refuted.funnel().verified,
            all_refuted.funnel().roi_approved,
            all_refuted.funnel().ranked,
        ),
        (2, 2, 0, 0, 0)
    );

    let first = evaluate_tier2(Tier2Input::Enabled {
        collector: Tier2StageResult::Failed(failure(
            Tier2Stage::RoiRank,
            Tier2FailureCode::InvalidRanking,
        )),
        generator: Tier2StageResult::Failed(failure(
            Tier2Stage::RoiRank,
            Tier2FailureCode::InvalidRanking,
        )),
        verifier: Tier2StageResult::Failed(failure(
            Tier2Stage::RoiRank,
            Tier2FailureCode::InvalidRanking,
        )),
        roi_policy: Tier2RoiPolicy::v1(),
    })
    .expect_err("first failed stage wins");
    assert_eq!(first.stage(), Tier2Stage::Collector);
}

#[test]
fn all_stage_documents_are_canonical_and_digest_bound() {
    let digest = "a".repeat(64);
    let invalid_digest = "A".repeat(64);
    let evaluation = evaluate_tier2(enabled(
        vec![proposal("one")],
        vec![Tier2Verification::Survived {
            stable_key: "one".to_string(),
            reason: "bounded evidence remains actionable".to_string(),
        }],
    ))
    .expect("complete evidence");
    let documents = evaluation.observation().expect("observation").documents();
    assert!(documents
        .collector_json()
        .expect("collector")
        .ends_with('\n'));
    for document in [
        documents.generated_json(&digest),
        documents.deduplication_json(&digest),
        documents.verification_json(&digest),
        documents.roi_rank_json(&digest),
    ] {
        assert!(document
            .expect("valid digest")
            .expect("stage document")
            .ends_with('\n'));
    }
    assert!(documents.generated_json(&invalid_digest).is_err());
    assert!(documents.deduplication_json(&invalid_digest).is_err());
    assert!(documents.verification_json(&invalid_digest).is_err());
    assert!(documents.roi_rank_json(&invalid_digest).is_err());

    let failed = evaluate_tier2(Tier2Input::Enabled {
        collector: Tier2StageResult::Complete(collector()),
        generator: Tier2StageResult::Failed(failure(
            Tier2Stage::Verifier,
            Tier2FailureCode::InvalidVerdictCoverage,
        )),
        verifier: Tier2StageResult::Complete(Tier2VerifierVerdicts {
            verifier_identity: "test-verifier".to_string(),
            verifier_protocol_version: "v1".to_string(),
            verdicts: Vec::new(),
        }),
        roi_policy: Tier2RoiPolicy::v1(),
    })
    .expect_err("failed stage stays sealed");
    let failure_document = failed
        .documents()
        .expect("evaluated failure")
        .failure_json(Some(&digest))
        .expect("valid digest")
        .expect("failure document");
    assert!(failure_document.ends_with('\n'));

    let evidence_source =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/autonomous/tier2/evidence.rs");
    assert!(
        !fs::read_to_string(evidence_source)
            .expect("read evidence implementation")
            .contains("pub fn render_tier2_"),
        "raw model renderer must not be public"
    );
}
