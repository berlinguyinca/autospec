use std::fs;
use std::path::Path;

use autospec_core::autonomous::tier3::{
    evaluate_tier3, Tier3AdapterEvidence, Tier3Failure, Tier3FailureCode, Tier3Finding,
    Tier3FindingKind, Tier3Input, Tier3Severity, Tier3Stage, Tier3StageResult, DISABLED_REASON,
    TIER3_RANK_LIMIT,
};

fn finding(
    kind: Tier3FindingKind,
    severity: Tier3Severity,
    rule_id: &str,
    path: &str,
    line: u64,
    message: &str,
) -> Tier3Finding {
    Tier3Finding {
        kind,
        severity,
        rule_id: rule_id.to_string(),
        path: path.to_string(),
        line,
        message: message.to_string(),
    }
}

fn evidence(kind: Tier3FindingKind, findings: Vec<Tier3Finding>) -> Tier3AdapterEvidence {
    Tier3AdapterEvidence {
        schema_version: 1,
        adapter_version: format!("test-{}-adapter", kind.as_str()),
        rule_version: "rules-v1".to_string(),
        findings,
    }
}

fn enabled(
    architecture: Vec<Tier3Finding>,
    coverage: Vec<Tier3Finding>,
    debt: Vec<Tier3Finding>,
) -> Tier3Input {
    Tier3Input::Enabled {
        architecture: Tier3StageResult::Complete(evidence(
            Tier3FindingKind::Architecture,
            architecture,
        )),
        coverage: Tier3StageResult::Complete(evidence(Tier3FindingKind::Coverage, coverage)),
        debt: Tier3StageResult::Complete(evidence(Tier3FindingKind::Debt, debt)),
    }
}

fn failure(stage: Tier3Stage, code: Tier3FailureCode) -> Tier3Failure {
    Tier3Failure::new(stage, code, "typed stage failure").expect("bounded failure")
}

#[test]
fn disabled_policy_is_exact_and_has_no_observation() {
    let evaluation = evaluate_tier3(Tier3Input::DisabledByCheckedInPolicy).expect("policy result");

    assert_eq!(evaluation.observation(), None);
    assert_eq!(
        DISABLED_REASON,
        "tier3_metadata_disabled_by_checked_in_policy"
    );
    assert_eq!(
        evaluation.not_run_reason(),
        Some("tier3_metadata_disabled_by_checked_in_policy")
    );
}

#[test]
fn failure_stages_and_codes_are_closed_and_receipt_parseable() {
    assert_eq!(Tier3Stage::Architecture.as_str(), "architecture");
    assert_eq!(Tier3Stage::Coverage.as_str(), "coverage");
    assert_eq!(Tier3Stage::Debt.as_str(), "debt");
    assert_eq!(Tier3Stage::Ranking.as_str(), "ranking");

    for code in [
        Tier3FailureCode::MissingStageResult,
        Tier3FailureCode::InvalidAdapterEvidence,
        Tier3FailureCode::InvalidFinding,
        Tier3FailureCode::WrongFindingKind,
        Tier3FailureCode::NonCanonicalOrder,
        Tier3FailureCode::DuplicateConflict,
        Tier3FailureCode::InvalidRanking,
        Tier3FailureCode::CountOverflow,
    ] {
        assert_eq!(Tier3FailureCode::parse(code.as_str()), Ok(code));
    }
    assert!(Tier3FailureCode::parse("invented_code").is_err());
    assert_eq!(
        failure(Tier3Stage::Debt, Tier3FailureCode::InvalidFinding).status_reason(),
        "tier3_debt_invalid_finding"
    );
    assert_eq!(
        Tier3Failure::parse_status_reason("tier3_debt_invalid_finding"),
        Ok((Tier3Stage::Debt, Tier3FailureCode::InvalidFinding))
    );
    for malformed in [
        "tier3_unknown_invalid_finding",
        "tier3_debt_unknown_code",
        "tier3_debt_invalid_finding_extra",
        "tier3_debt",
    ] {
        assert!(Tier3Failure::parse_status_reason(malformed).is_err());
    }
}

#[test]
fn stages_fail_or_remain_missing_before_later_empty_evidence() {
    let result = evaluate_tier3(Tier3Input::Enabled {
        architecture: Tier3StageResult::Complete(evidence(
            Tier3FindingKind::Architecture,
            Vec::new(),
        )),
        coverage: Tier3StageResult::Failed(failure(
            Tier3Stage::Ranking,
            Tier3FailureCode::CountOverflow,
        )),
        debt: Tier3StageResult::Complete(evidence(Tier3FindingKind::Debt, Vec::new())),
    })
    .expect_err("failed coverage must win over later empty evidence");
    assert_eq!(result.stage(), Tier3Stage::Coverage);
    assert_eq!(result.code(), Tier3FailureCode::CountOverflow);
    assert!(result.partial_evidence().has_architecture());
    assert!(!result.partial_evidence().has_coverage());

    let missing = evaluate_tier3(Tier3Input::Enabled {
        architecture: Tier3StageResult::Missing,
        coverage: Tier3StageResult::Complete(evidence(Tier3FindingKind::Coverage, Vec::new())),
        debt: Tier3StageResult::Complete(evidence(Tier3FindingKind::Debt, Vec::new())),
    })
    .expect_err("missing architecture must fail closed");
    assert_eq!(
        (missing.stage(), missing.code()),
        (
            Tier3Stage::Architecture,
            Tier3FailureCode::MissingStageResult
        )
    );
}

#[test]
fn invalid_fields_wrong_kind_and_noncanonical_stage_records_fail_closed() {
    let invalid_path = finding(
        Tier3FindingKind::Architecture,
        Tier3Severity::High,
        "architecture.path",
        "../outside.rs",
        1,
        "path escapes repository root",
    );
    let error = evaluate_tier3(enabled(vec![invalid_path], Vec::new(), Vec::new()))
        .expect_err("escaping paths must fail");
    assert_eq!(
        (error.stage(), error.code()),
        (Tier3Stage::Architecture, Tier3FailureCode::InvalidFinding)
    );

    let drive_relative = finding(
        Tier3FindingKind::Architecture,
        Tier3Severity::High,
        "architecture.path",
        "C:outside.rs",
        1,
        "drive-relative path escapes repository root",
    );
    let error = evaluate_tier3(enabled(vec![drive_relative], Vec::new(), Vec::new()))
        .expect_err("drive-relative paths must fail");
    assert_eq!(error.code(), Tier3FailureCode::InvalidFinding);

    let wrong_kind = finding(
        Tier3FindingKind::Debt,
        Tier3Severity::High,
        "architecture.kind",
        "src/lib.rs",
        1,
        "wrong adapter kind",
    );
    let error = evaluate_tier3(enabled(vec![wrong_kind], Vec::new(), Vec::new()))
        .expect_err("architecture adapters cannot submit debt findings");
    assert_eq!(error.code(), Tier3FailureCode::WrongFindingKind);

    let later = finding(
        Tier3FindingKind::Architecture,
        Tier3Severity::Low,
        "architecture.zeta",
        "src/z.rs",
        1,
        "later canonical record",
    );
    let earlier = finding(
        Tier3FindingKind::Architecture,
        Tier3Severity::Critical,
        "architecture.alpha",
        "src/a.rs",
        1,
        "earlier canonical record",
    );
    let error = evaluate_tier3(enabled(vec![later, earlier], Vec::new(), Vec::new()))
        .expect_err("adapter records must be canonically sorted");
    assert_eq!(error.code(), Tier3FailureCode::NonCanonicalOrder);
}

#[test]
fn duplicate_and_conflicting_finding_keys_are_rejected() {
    let row = finding(
        Tier3FindingKind::Architecture,
        Tier3Severity::Medium,
        "architecture.duplicate",
        "src/lib.rs",
        2,
        "duplicate finding",
    );
    let error = evaluate_tier3(enabled(vec![row.clone(), row], Vec::new(), Vec::new()))
        .expect_err("duplicates are not canonical input");
    assert_eq!(error.code(), Tier3FailureCode::NonCanonicalOrder);

    let first = finding(
        Tier3FindingKind::Architecture,
        Tier3Severity::High,
        "architecture.conflict",
        "src/lib.rs",
        3,
        "first message",
    );
    let second = finding(
        Tier3FindingKind::Architecture,
        Tier3Severity::High,
        "architecture.conflict",
        "src/lib.rs",
        3,
        "second message",
    );
    let error = evaluate_tier3(enabled(vec![first, second], Vec::new(), Vec::new()))
        .expect_err("one finding key cannot have conflicting evidence");
    assert_eq!(error.code(), Tier3FailureCode::DuplicateConflict);
}

#[test]
fn empty_complete_evidence_is_valid_and_ranked_results_obey_the_cap() {
    let empty = evaluate_tier3(enabled(Vec::new(), Vec::new(), Vec::new()))
        .expect("empty complete metadata remains an observation");
    let empty_funnel = empty.observation().expect("observation").funnel();
    assert_eq!(
        (
            empty_funnel.observed,
            empty_funnel.deduplicated,
            empty_funnel.verified,
            empty_funnel.roi_approved,
            empty_funnel.ranked,
        ),
        (0, 0, 0, 0, 0)
    );

    let architecture = (0..12)
        .map(|index| {
            finding(
                Tier3FindingKind::Architecture,
                if index == 0 {
                    Tier3Severity::Critical
                } else {
                    Tier3Severity::Low
                },
                &format!("architecture.{index:02}"),
                &format!("src/{index:02}.rs"),
                1,
                "bounded finding",
            )
        })
        .collect();
    let evaluation = evaluate_tier3(enabled(architecture, Vec::new(), Vec::new()))
        .expect("canonical findings rank deterministically");
    let observation = evaluation.observation().expect("observation");
    assert_eq!(observation.funnel().observed, 12);
    assert_eq!(observation.funnel().deduplicated, 12);
    assert_eq!(observation.funnel().verified, 12);
    assert_eq!(observation.funnel().roi_approved, 12);
    assert_eq!(observation.funnel().ranked, TIER3_RANK_LIMIT);
    assert_eq!(observation.ranked()[0].rule_id, "architecture.00");
    assert_eq!(observation.ranked().len() as u64, TIER3_RANK_LIMIT);
}

#[test]
fn sealed_documents_have_canonical_kinds_and_predecessor_rules() {
    let architecture = finding(
        Tier3FindingKind::Architecture,
        Tier3Severity::High,
        "architecture.boundary",
        "src/lib.rs",
        5,
        "architecture boundary is unsealed",
    );
    let coverage = finding(
        Tier3FindingKind::Coverage,
        Tier3Severity::Medium,
        "coverage.branch",
        "tests/lib.rs",
        8,
        "branch lacks direct coverage",
    );
    let debt = finding(
        Tier3FindingKind::Debt,
        Tier3Severity::Low,
        "debt.duplicate",
        "src/lib.rs",
        9,
        "duplicate control flow",
    );
    let evaluation = evaluate_tier3(enabled(vec![architecture], vec![coverage], vec![debt]))
        .expect("valid evidence");
    let digest = "a".repeat(64);
    let documents = evaluation.observation().expect("observation").documents();
    assert_eq!(
        documents.architecture_json().expect("architecture document"),
        "{\"schema\":1,\"kind\":\"tier3_architecture\",\"adapter_version\":\"test-architecture-adapter\",\"rule_version\":\"rules-v1\",\"findings\":[{\"kind\":\"architecture\",\"severity\":\"high\",\"rule_id\":\"architecture.boundary\",\"path\":\"src/lib.rs\",\"line\":5,\"message\":\"architecture boundary is unsealed\"}]}\n"
    );
    assert_eq!(
        documents
        .coverage_json(&digest)
        .expect("sealed coverage digest")
        .expect("coverage document"),
        "{\"schema\":1,\"kind\":\"tier3_coverage\",\"predecessor_digest\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",\"adapter_version\":\"test-coverage-adapter\",\"rule_version\":\"rules-v1\",\"findings\":[{\"kind\":\"coverage\",\"severity\":\"medium\",\"rule_id\":\"coverage.branch\",\"path\":\"tests/lib.rs\",\"line\":8,\"message\":\"branch lacks direct coverage\"}]}\n"
    );
    assert_eq!(
        documents
        .debt_json(&digest)
        .expect("sealed debt digest")
        .expect("debt document"),
        "{\"schema\":1,\"kind\":\"tier3_debt\",\"predecessor_digest\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",\"adapter_version\":\"test-debt-adapter\",\"rule_version\":\"rules-v1\",\"findings\":[{\"kind\":\"debt\",\"severity\":\"low\",\"rule_id\":\"debt.duplicate\",\"path\":\"src/lib.rs\",\"line\":9,\"message\":\"duplicate control flow\"}]}\n"
    );
    assert_eq!(
        documents
        .findings_json(&digest)
        .expect("sealed findings digest")
        .expect("findings document"),
        "{\"schema\":1,\"kind\":\"tier3_findings\",\"predecessor_digest\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",\"rank_limit\":10,\"funnel\":{\"observed\":3,\"deduplicated\":3,\"verified\":3,\"roi_approved\":3,\"ranked\":3},\"deduplicated\":[{\"kind\":\"architecture\",\"severity\":\"high\",\"rule_id\":\"architecture.boundary\",\"path\":\"src/lib.rs\",\"line\":5,\"message\":\"architecture boundary is unsealed\"},{\"kind\":\"coverage\",\"severity\":\"medium\",\"rule_id\":\"coverage.branch\",\"path\":\"tests/lib.rs\",\"line\":8,\"message\":\"branch lacks direct coverage\"},{\"kind\":\"debt\",\"severity\":\"low\",\"rule_id\":\"debt.duplicate\",\"path\":\"src/lib.rs\",\"line\":9,\"message\":\"duplicate control flow\"}],\"ranked\":[{\"kind\":\"architecture\",\"severity\":\"high\",\"rule_id\":\"architecture.boundary\",\"path\":\"src/lib.rs\",\"line\":5,\"message\":\"architecture boundary is unsealed\"},{\"kind\":\"coverage\",\"severity\":\"medium\",\"rule_id\":\"coverage.branch\",\"path\":\"tests/lib.rs\",\"line\":8,\"message\":\"branch lacks direct coverage\"},{\"kind\":\"debt\",\"severity\":\"low\",\"rule_id\":\"debt.duplicate\",\"path\":\"src/lib.rs\",\"line\":9,\"message\":\"duplicate control flow\"}]}\n"
    );

    let failure = evaluate_tier3(Tier3Input::Enabled {
        architecture: Tier3StageResult::Complete(evidence(
            Tier3FindingKind::Architecture,
            Vec::new(),
        )),
        coverage: Tier3StageResult::Failed(failure(
            Tier3Stage::Architecture,
            Tier3FailureCode::InvalidAdapterEvidence,
        )),
        debt: Tier3StageResult::Complete(evidence(Tier3FindingKind::Debt, Vec::new())),
    })
    .expect_err("coverage failure");
    let documents = failure.documents().expect("sealed failure documents");
    assert!(documents.architecture_json().is_some());
    assert!(documents.coverage_json(&digest).expect("digest").is_none());
    assert!(documents.failure_json(None).is_err());
    assert_eq!(
        documents
        .failure_json(Some(&digest))
        .expect("coverage failure predecessor")
        .expect("failure document"),
        "{\"schema\":1,\"kind\":\"tier3_failure\",\"predecessor_digest\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",\"stage\":\"coverage\",\"code\":\"invalid_adapter_evidence\",\"status_reason\":\"tier3_coverage_invalid_adapter_evidence\",\"detail\":\"typed stage failure\",\"funnel\":{\"observed\":0,\"deduplicated\":0,\"verified\":0,\"roi_approved\":0,\"ranked\":0}}\n"
    );
}

#[test]
fn tier3_pure_source_has_no_direct_io_or_legacy_authority() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for source in [
        root.join("src/autonomous/tier3.rs"),
        root.join("src/autonomous/tier3/model.rs"),
        root.join("src/autonomous/tier3/evaluate.rs"),
        root.join("src/autonomous/tier3/evidence.rs"),
    ] {
        let contents = fs::read_to_string(&source).expect("read pure Tier 3 source");
        let git_child = ["\"", "g", "h "].concat();
        for forbidden in [
            "std::fs",
            "std::env",
            "std::process",
            "Command::new",
            "std::net",
            "WaterfallStore",
            "scan_specialists",
            "autospec-explore",
            git_child.as_str(),
            "curl",
            "bash",
            "sh -c",
            "queue::",
            "claim::",
        ] {
            assert!(
                !contents.contains(forbidden),
                "{} retains {forbidden}",
                source.display()
            );
        }
    }
}
