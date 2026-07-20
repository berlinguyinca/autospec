use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use autospec_core::autonomous::tier2::{
    evaluate_tier2, evaluate_tier2_with_policy, Tier2Complexity, Tier2Evaluation,
    Tier2ExclusionPolicy, Tier2Failure, Tier2FailureCode, Tier2GeneratedProposals, Tier2Input,
    Tier2Proposal, Tier2RoiPolicy, Tier2Severity, Tier2Source, Tier2Stage, Tier2StageResult,
    Tier2Verification, Tier2VerifierVerdicts, DISABLED_REASON,
};
use autospec_core::explore::specialists::{
    DetectedDomain, FileLineEvidence, StrictCollectorEvidence,
};

fn row(file: &str, line: usize, matched: &str) -> FileLineEvidence {
    FileLineEvidence {
        file: file.to_string(),
        line,
        r#match: matched.to_string(),
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
            evidence: vec![row("Cargo.toml", 1, "trading")],
        }],
    }
}

fn proposal(key: &str) -> Tier2Proposal {
    Tier2Proposal {
        stable_key: key.to_string(),
        title: format!("feat: {key}"),
        source: Tier2Source::StrictLocalSpecialist,
        evidence: vec![row("Cargo.toml", 1, "trading")],
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

fn survives(key: &str) -> Tier2Verification {
    Tier2Verification::Survived {
        stable_key: key.to_string(),
        reason: "bounded evidence remains actionable".to_string(),
    }
}

fn refutes(key: &str) -> Tier2Verification {
    Tier2Verification::Refuted {
        stable_key: key.to_string(),
        reason: "evidence does not establish a gap".to_string(),
    }
}

fn verdicts(rows: Vec<Tier2Verification>) -> Tier2VerifierVerdicts {
    Tier2VerifierVerdicts {
        verifier_identity: "test-verifier".to_string(),
        verifier_protocol_version: "v1".to_string(),
        verdicts: rows,
    }
}

fn enabled(proposals: Vec<Tier2Proposal>, verdict_rows: Vec<Tier2Verification>) -> Tier2Input {
    enabled_with_collector(collector(), proposals, verdict_rows)
}

fn enabled_with_collector(
    collector: StrictCollectorEvidence,
    proposals: Vec<Tier2Proposal>,
    verdict_rows: Vec<Tier2Verification>,
) -> Tier2Input {
    Tier2Input::Enabled {
        collector: Tier2StageResult::Complete(collector),
        generator: Tier2StageResult::Complete(generated(proposals)),
        verifier: Tier2StageResult::Complete(verdicts(verdict_rows)),
        roi_policy: Tier2RoiPolicy::v1(),
    }
}

#[test]
fn default_exclusion_policy_filters_vendor_descendants_and_records_pollution() {
    let mut supplied = collector();
    supplied.domains[0].score = 3;
    supplied.domains[0].evidence.extend([
        row("node_modules/pkg/index.js", 1, "trading"),
        row("node_modules/pkg/nested/mod.js", 2, "trading"),
    ]);
    supplied.domains[0]
        .evidence
        .sort_by(|left, right| left.file.cmp(&right.file).then(left.line.cmp(&right.line)));
    let evaluation = evaluate_tier2(enabled_with_collector(
        supplied,
        vec![proposal("clean")],
        vec![survives("clean")],
    ))
    .expect("default policy filters supplied vendor evidence");
    let observation = evaluation
        .observation()
        .expect("complete filtered observation");

    assert!(observation.collector().domains[0]
        .evidence
        .iter()
        .all(|row| !row.file.starts_with("node_modules/")));
    assert_eq!(observation.exclusion_report().excluded_path_count(), 2);
    assert_eq!(observation.exclusion_report().pollution_findings().len(), 2);
    assert!(observation
        .evidence_json()
        .contains("\"finding\":\"prohibited_vendor_path\""));
    let collector_receipt = observation
        .documents()
        .collector_json()
        .expect("collector receipt");
    assert!(collector_receipt.contains(observation.exclusion_report().policy_digest()));
    assert!(collector_receipt.contains("\"excluded_path_count\":2"));
    for excluded in [".git", ".next", "dist", "build", "coverage", "node_modules"] {
        assert!(Tier2ExclusionPolicy::default().excludes_component(excluded));
    }
}

#[test]
fn repository_exclusion_additions_are_deterministic_and_digest_bound() {
    let first = Tier2ExclusionPolicy::with_repository_additions(["generated", "artifacts"])
        .expect("valid checked-in additions");
    let second = Tier2ExclusionPolicy::with_repository_additions(["artifacts", "generated"])
        .expect("order-independent checked-in additions");

    assert_eq!(first, second);
    assert_eq!(first.digest(), second.digest());
    assert_eq!(first.digest().len(), 64);
    assert!(first.excludes_component("generated"));
    assert!(Tier2ExclusionPolicy::with_repository_additions(["../outside"]).is_err());
}

#[test]
fn tier2_exclusion_policy_rejects_evidence_traversal_outside_repository_root() {
    let mut supplied = collector();
    supplied.domains[0].evidence = vec![row("../outside/Cargo.toml", 1, "trading")];

    let failure = evaluate_tier2_with_policy(
        enabled_with_collector(supplied, Vec::new(), Vec::new()),
        Tier2ExclusionPolicy::default(),
    )
    .expect_err("traversal must fail before policy filtering");

    assert_eq!(failure.code(), Tier2FailureCode::PathEscapesRoot);
    assert!(failure.detail().contains("outside repository root"));
}

fn failure(stage: Tier2Stage, code: Tier2FailureCode) -> Tier2Failure {
    Tier2Failure::new(stage, code, "typed stage failure").expect("bounded typed failure")
}

#[test]
fn disabled_policy_is_exact_and_has_no_observation() {
    let evaluation = evaluate_tier2(Tier2Input::DisabledByCheckedInPolicy).expect("policy result");

    assert_eq!(evaluation.observation(), None);
    assert_eq!(evaluation.not_run_reason(), Some(DISABLED_REASON));
    assert_eq!(
        evaluation.evidence_json(),
        format!("{{\"schema\":1,\"kind\":\"tier2_evaluation\",\"result\":\"not_run\",\"reason\":\"{DISABLED_REASON}\"}}\n")
    );
}

#[test]
fn stage_validation_returns_failures_before_empty_generator_results() {
    let expected = failure(
        Tier2Stage::Verifier,
        Tier2FailureCode::InvalidVerdictCoverage,
    );
    let result = evaluate_tier2(Tier2Input::Enabled {
        collector: Tier2StageResult::Complete(collector()),
        generator: Tier2StageResult::Complete(generated(Vec::new())),
        verifier: Tier2StageResult::Failed(expected.clone()),
        roi_policy: Tier2RoiPolicy::v1(),
    });

    let returned = result.expect_err("failed verifier must not become a dry result");
    assert_eq!(returned.stage(), expected.stage());
    assert_eq!(returned.code(), expected.code());
    assert_eq!(returned.detail(), expected.detail());
    let missing = evaluate_tier2(Tier2Input::Enabled {
        collector: Tier2StageResult::Missing,
        generator: Tier2StageResult::Complete(generated(Vec::new())),
        verifier: Tier2StageResult::Complete(verdicts(Vec::new())),
        roi_policy: Tier2RoiPolicy::v1(),
    })
    .expect_err("missing collector must fail closed");
    assert_eq!(
        (missing.stage(), missing.code()),
        (Tier2Stage::Collector, Tier2FailureCode::MissingStageResult)
    );
}

#[test]
fn collector_and_proposal_validation_fail_closed() {
    let mut invalid_collector = collector();
    invalid_collector.schema_version = 2;
    assert_eq!(
        evaluate_tier2(Tier2Input::Enabled {
            collector: Tier2StageResult::Complete(invalid_collector),
            generator: Tier2StageResult::Complete(generated(Vec::new())),
            verifier: Tier2StageResult::Complete(verdicts(Vec::new())),
            roi_policy: Tier2RoiPolicy::v1(),
        })
        .expect_err("wrong collector schema must fail")
        .stage(),
        Tier2Stage::Collector
    );

    let mut invalid = proposal("outside");
    invalid.evidence = vec![row("src/main.rs", 4, "outside")];
    let error = evaluate_tier2(enabled(vec![invalid], vec![survives("outside")]))
        .expect_err("proposal evidence must come from the collector");
    assert_eq!(
        (error.stage(), error.code()),
        (Tier2Stage::Generator, Tier2FailureCode::InvalidProposal)
    );
}

#[test]
fn empty_complete_generator_is_a_valid_zero_count_observation() {
    let evaluation =
        evaluate_tier2(enabled(Vec::new(), Vec::new())).expect("empty complete stages");
    let observation = evaluation.observation().expect("complete observation");

    assert_eq!(observation.funnel().observed, 0);
    assert_eq!(observation.funnel().deduplicated, 0);
    assert_eq!(observation.funnel().verified, 0);
    assert_eq!(observation.funnel().roi_approved, 0);
    assert_eq!(observation.funnel().ranked, 0);
}

#[test]
fn deduplication_selects_the_stable_best_winner_and_rejects_conflicts() {
    let mut first = proposal("zeta");
    first.title = "feat: Normalize evidence".to_string();
    first.confidence_millis = 500;
    let mut second = proposal("alpha");
    second.title = "fix: normalize evidence".to_string();
    second.confidence_millis = 800;
    let evaluation = evaluate_tier2(enabled(
        vec![first.clone(), second.clone()],
        vec![survives("alpha")],
    ))
    .expect("same evidence and consumer may deduplicate");
    let group = &evaluation
        .observation()
        .expect("observation")
        .deduplication()
        .groups[0];
    assert_eq!(group.winner_key, "alpha");
    assert_eq!(group.candidate_keys, vec!["alpha", "zeta"]);
    assert_eq!(group.suppressed_keys, vec!["zeta"]);

    second.named_consumer = "different-consumer".to_string();
    let error = evaluate_tier2(enabled(vec![first, second], vec![survives("alpha")]))
        .expect_err("different consumer makes a duplicate group conflicting");
    assert_eq!(
        (error.stage(), error.code()),
        (
            Tier2Stage::Deduplicator,
            Tier2FailureCode::DuplicateConflict
        )
    );
}

#[test]
fn verifier_requires_one_valid_verdict_for_each_winner() {
    let missing = evaluate_tier2(enabled(vec![proposal("one")], Vec::new()))
        .expect_err("missing verdict must fail");
    assert_eq!(
        (missing.stage(), missing.code()),
        (
            Tier2Stage::Verifier,
            Tier2FailureCode::InvalidVerdictCoverage
        )
    );

    let duplicate = evaluate_tier2(enabled(
        vec![proposal("one")],
        vec![survives("one"), refutes("one")],
    ))
    .expect_err("duplicate verdict must fail");
    assert_eq!(
        (duplicate.stage(), duplicate.code()),
        (
            Tier2Stage::Verifier,
            Tier2FailureCode::InvalidVerdictCoverage
        )
    );
}

#[test]
fn refutation_roi_and_rank_cap_produce_monotonic_distinct_outcomes() {
    let refuted = evaluate_tier2(enabled(vec![proposal("one")], vec![refutes("one")]))
        .expect("complete refutation observation");
    assert_eq!(
        refuted.observation().expect("observation").funnel().ranked,
        0
    );

    let mut candidates = (0..6)
        .map(|index| {
            let mut item = proposal(&format!("candidate-{index}"));
            item.title = format!("candidate {index}");
            item.confidence_millis = 600 + index as u16;
            item
        })
        .collect::<Vec<_>>();
    candidates[0].severity = Tier2Severity::High;
    let survived = candidates
        .iter()
        .map(|item| survives(&item.stable_key))
        .collect();
    let cap = evaluate_tier2(enabled(candidates.clone(), survived)).expect("six valid survivors");
    let cap_observation = cap.observation().expect("observation");
    assert_eq!(
        (
            cap_observation.funnel().roi_approved,
            cap_observation.funnel().ranked
        ),
        (6, 5)
    );
    assert_eq!(
        cap_observation.ranked()[0].proposal.severity,
        Tier2Severity::High
    );
    assert_eq!(cap_observation.ranked()[0].rank, 1);

    let roi_filtered = evaluate_tier2(Tier2Input::Enabled {
        collector: Tier2StageResult::Complete(collector()),
        generator: Tier2StageResult::Complete(generated(candidates)),
        verifier: Tier2StageResult::Complete(verdicts(
            (0..6)
                .map(|index| survives(&format!("candidate-{index}")))
                .collect(),
        )),
        roi_policy: Tier2RoiPolicy::new(BTreeSet::new()),
    })
    .expect("empty injected permission set is a valid ROI policy");
    let funnel = roi_filtered.observation().expect("observation").funnel();
    assert_eq!(
        (funnel.verified, funnel.roi_approved, funnel.ranked),
        (6, 0, 0)
    );
}

#[test]
fn failed_stages_preserve_only_validated_predecessor_evidence() {
    let error = evaluate_tier2(Tier2Input::Enabled {
        collector: Tier2StageResult::Complete(collector()),
        generator: Tier2StageResult::Failed(failure(
            Tier2Stage::Generator,
            Tier2FailureCode::InvalidProposal,
        )),
        verifier: Tier2StageResult::Complete(verdicts(Vec::new())),
        roi_policy: Tier2RoiPolicy::v1(),
    })
    .expect_err("generator failure must retain the validated collector only");

    assert_eq!(error.partial_evidence().funnel().observed, 0);
    let digest = "a".repeat(64);
    let documents = error.documents().expect("evaluated failure is sealed");
    let collector_json = documents
        .collector_json()
        .expect("validated collector document");
    assert!(collector_json.contains("strict-local-v1"));
    let failure_json = documents
        .failure_json(Some(&digest))
        .expect("generator failure has a predecessor digest");
    let failure_json = failure_json.expect("failure document");
    assert!(failure_json.contains("\"stage\":\"generator\""));
    assert!(documents.failure_json(None).is_err());
    assert!(documents
        .generated_json(&digest)
        .expect("valid digest")
        .is_none());
}

#[test]
fn failure_details_are_bounded_by_unicode_scalars() {
    let scalar = "é".repeat(240);
    assert!(Tier2Failure::new(
        Tier2Stage::Verifier,
        Tier2FailureCode::InvalidVerdictCoverage,
        scalar
    )
    .is_ok());
    assert!(Tier2Failure::new(
        Tier2Stage::Verifier,
        Tier2FailureCode::InvalidVerdictCoverage,
        " "
    )
    .is_err());
    assert!(Tier2Failure::new(
        Tier2Stage::Verifier,
        Tier2FailureCode::InvalidVerdictCoverage,
        "é".repeat(241)
    )
    .is_err());
}

#[test]
fn canonical_json_is_stable_across_valid_input_ordering() {
    let left = proposal("left");
    let mut right = proposal("right");
    right.title = "right independent candidate".to_string();
    let first = evaluate_tier2(enabled(
        vec![left.clone(), right.clone()],
        vec![survives("left"), survives("right")],
    ))
    .expect("first ordering");
    let second = evaluate_tier2(enabled(
        vec![right, left],
        vec![survives("right"), survives("left")],
    ))
    .expect("second ordering");

    assert!(first.evidence_json().ends_with('\n'));
    assert_eq!(
        first
            .evidence_json()
            .bytes()
            .filter(|byte| *byte == b'\n')
            .count(),
        1
    );
    assert_eq!(first.evidence_json(), second.evidence_json());
}

#[test]
fn tier2_source_has_no_io_legacy_or_mutation_authority() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for source in [
        root.join("src/autonomous/tier2.rs"),
        root.join("src/autonomous/tier2/model.rs"),
        root.join("src/autonomous/tier2/funnel.rs"),
        root.join("src/autonomous/tier2/funnel_validation.rs"),
        root.join("src/autonomous/tier2/evidence.rs"),
        root.join("src/autonomous/tier2/partial.rs"),
    ] {
        let contents = fs::read_to_string(&source).expect("read pure Tier 2 source");
        let git_child = ["\"", "g", "h "].concat();
        for forbidden in [
            "std::fs",
            "std::env",
            "std::process",
            "Command::new",
            "std::net",
            "WaterfallStore",
            "scan_specialists",
            "load_or_derive",
            "AUTOSPEC_SPECIALIST_LLM_STUB_OUTPUT",
            "queue::",
            "claim::",
            "autospec-explore",
            git_child.as_str(),
            "curl",
            "bash",
            "sh -c",
        ] {
            assert!(
                !contents.contains(forbidden),
                "{} retains {forbidden}",
                source.display()
            );
        }
    }
}

#[test]
fn evaluation_variants_remain_closed() {
    let evaluation = evaluate_tier2(enabled(vec![proposal("one")], vec![survives("one")]))
        .expect("complete evaluation");
    assert!(matches!(evaluation, Tier2Evaluation::Complete(_)));
    assert_eq!(
        Tier2Source::StrictLocalSpecialist.as_str(),
        "strict_local_specialist"
    );
    assert_eq!(Tier2Severity::Critical.rank(), 0);
    assert_eq!(Tier2Complexity::Large.units(), 4);
}
