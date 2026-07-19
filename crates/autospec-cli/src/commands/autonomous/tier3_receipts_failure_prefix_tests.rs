use std::fs;

use autospec_core::autonomous::no_work::NoWorkTier;
use autospec_core::autonomous::tier3::{
    evaluate_tier3, Tier3AdapterEvidence, Tier3Failure, Tier3FailureCode, Tier3FindingKind,
    Tier3Input, Tier3Stage, Tier3StageResult,
};
use autospec_core::autonomous::waterfall::{FunnelCounts, TierReceipt, TierStatus};

use super::tier2_receipts_tests::{store, TempRoot, REPO};
use super::tier3::Tier3Scan;
use super::tier3_receipts::{record_tier3, Tier3Progress};
use super::tier3_receipts_tests::seed_tier_three_cursor;
use super::waterfall::Tier3EvidenceArtifact;

fn adapter(kind: Tier3FindingKind) -> Tier3AdapterEvidence {
    Tier3AdapterEvidence {
        schema_version: 1,
        adapter_version: format!("test-{}-adapter", kind.as_str()),
        rule_version: "rules-v1".to_string(),
        findings: Vec::new(),
    }
}

pub(super) fn failure(stage: Tier3Stage) -> Tier3Failure {
    let raw = Tier3Failure::new(
        stage,
        Tier3FailureCode::InvalidFinding,
        "injected typed failure",
    )
    .expect("bounded failure");
    let input = match stage {
        Tier3Stage::Architecture => Tier3Input::Enabled {
            architecture: Tier3StageResult::Failed(raw),
            coverage: Tier3StageResult::Missing,
            debt: Tier3StageResult::Missing,
        },
        Tier3Stage::Coverage => Tier3Input::Enabled {
            architecture: Tier3StageResult::Complete(adapter(Tier3FindingKind::Architecture)),
            coverage: Tier3StageResult::Failed(raw),
            debt: Tier3StageResult::Missing,
        },
        Tier3Stage::Debt => Tier3Input::Enabled {
            architecture: Tier3StageResult::Complete(adapter(Tier3FindingKind::Architecture)),
            coverage: Tier3StageResult::Complete(adapter(Tier3FindingKind::Coverage)),
            debt: Tier3StageResult::Failed(raw),
        },
        Tier3Stage::Ranking => panic!("ranking is sealed from a manual receipt fixture"),
    };
    evaluate_tier3(input).expect_err("injected failure must be sealed")
}

#[test]
fn tier3_failed_receipts_keep_exact_architecture_to_debt_prefixes() {
    for (stage, expected) in [
        (Tier3Stage::Architecture, vec!["failure.json"]),
        (
            Tier3Stage::Coverage,
            vec!["architecture.json", "failure.json"],
        ),
        (
            Tier3Stage::Debt,
            vec!["architecture.json", "coverage.json", "failure.json"],
        ),
    ] {
        let root = TempRoot::new();
        seed_tier_three_cursor(&root);
        let failure = failure(stage);
        let expected_reason = failure.status_reason();
        assert_eq!(
            record_tier3(root.path(), REPO, Tier3Scan::Failed(failure)).expect("failure receipt"),
            Tier3Progress::Failed(expected_reason.clone())
        );
        let receipt = store(&root)
            .load_receipt(1, NoWorkTier::Tier3)
            .expect("receipt")
            .expect("sealed receipt");
        assert_eq!(
            receipt
                .evidence()
                .iter()
                .map(|item| item.reference.rsplit('/').next().expect("name"))
                .collect::<Vec<_>>(),
            expected
        );
        assert_eq!(
            record_tier3(root.path(), REPO, Tier3Scan::NotRun).expect("failure replay"),
            Tier3Progress::Failed(expected_reason)
        );
    }
}

#[test]
fn tier3_manual_ranking_failure_replays_and_rejects_malformed_status_or_funnel() {
    let root = TempRoot::new();
    seed_tier_three_cursor(&root);
    let prior = record_empty_produced(&root);
    let receipt_store = store(&root);
    let mut evidence = prior.evidence()[..3].to_vec();
    let predecessor = evidence.last().expect("debt evidence").digest.clone();
    evidence.push(
        receipt_store
            .persist_tier3_evidence(
                1,
                Tier3EvidenceArtifact::Failure,
                &format!(
                    "{{\"schema\":1,\"kind\":\"tier3_failure\",\"predecessor_digest\":\"{predecessor}\",\"stage\":\"ranking\",\"code\":\"invalid_ranking\",\"status_reason\":\"tier3_ranking_invalid_ranking\",\"detail\":\"rank policy could not complete\",\"funnel\":{{\"observed\":0,\"deduplicated\":0,\"verified\":0,\"roi_approved\":0,\"ranked\":0}}}}\n"
                ),
            )
            .expect("manual failure evidence"),
    );
    let failed = receipt_with(
        TierStatus::Failed {
            reason: "tier3_ranking_invalid_ranking".to_string(),
        },
        zero(),
        evidence,
    );
    fs::write(
        receipt_store.receipt_path(&prior).expect("path"),
        format!("{}\n", failed.to_json()),
    )
    .expect("replace receipt");
    drop(receipt_store);
    assert_eq!(
        record_tier3(root.path(), REPO, Tier3Scan::NotRun).expect("ranking failure replay"),
        Tier3Progress::Failed("tier3_ranking_invalid_ranking".to_string())
    );

    let malformed = receipt_with(
        TierStatus::Failed {
            reason: "tier3_ranking_invalid_ranking_extra".to_string(),
        },
        zero(),
        failed.evidence().to_vec(),
    );
    let receipt_store = store(&root);
    fs::write(
        receipt_store.receipt_path(&failed).expect("path"),
        format!("{}\n", malformed.to_json()),
    )
    .expect("replace receipt");
    drop(receipt_store);
    assert!(record_tier3(root.path(), REPO, Tier3Scan::NotRun).is_err());

    let nonzero = receipt_with(
        failed.status().clone(),
        FunnelCounts::new(1, 1, 1, 1, 1).expect("nonzero funnel"),
        failed.evidence().to_vec(),
    );
    let receipt_store = store(&root);
    fs::write(
        receipt_store.receipt_path(&failed).expect("path"),
        format!("{}\n", nonzero.to_json()),
    )
    .expect("replace receipt");
    drop(receipt_store);
    assert!(record_tier3(root.path(), REPO, Tier3Scan::NotRun).is_err());
}

fn record_empty_produced(root: &TempRoot) -> TierReceipt {
    let observation =
        super::tier3_receipts_tests::observation(vec![super::tier3_receipts_tests::finding()]);
    assert_eq!(
        record_tier3(root.path(), REPO, Tier3Scan::Complete(observation))
            .expect("produced receipt"),
        Tier3Progress::Produced(1)
    );
    store(root)
        .load_receipt(1, NoWorkTier::Tier3)
        .expect("receipt")
        .expect("sealed receipt")
}

fn receipt_with(
    status: TierStatus,
    funnel: FunnelCounts,
    evidence: Vec<autospec_core::autonomous::waterfall::SealedEvidence>,
) -> TierReceipt {
    TierReceipt::new(
        REPO,
        1,
        NoWorkTier::Tier3,
        "rust-tier3-metadata-receipts-v1",
        1,
        1,
        status,
        funnel,
        evidence,
    )
    .expect("syntactic receipt")
}

fn zero() -> FunnelCounts {
    FunnelCounts::new(0, 0, 0, 0, 0).expect("zero funnel")
}
