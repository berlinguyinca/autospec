use autospec_core::autonomous::no_work::NoWorkTier;
use autospec_core::autonomous::tier2::{
    evaluate_tier2, Tier2Failure, Tier2FailureCode, Tier2GeneratedProposals, Tier2Input,
    Tier2RoiPolicy, Tier2Stage, Tier2StageResult,
};
use autospec_core::explore::specialists::{StrictCollectorError, StrictCollectorErrorCode};

use super::tier2::{strict_collector_failure, Tier2Scan};
use super::tier2_receipts::{record_tier2, Tier2Progress};
use super::tier2_receipts_tests::{
    collector, deduplicator_failure, proposal, seed_tier_two_cursor, TempRoot, REPO,
};

#[test]
fn tier2_failed_receipts_keep_exact_collector_to_verifier_prefixes() {
    assert_prefix(
        strict_collector_failure(StrictCollectorError {
            code: StrictCollectorErrorCode::ReadFile,
            detail: "collector read failed".to_string(),
        }),
        &["failure.json"],
    );
    assert_prefix(
        stage_failure(Tier2Stage::Generator),
        &["collector.json", "failure.json"],
    );
    assert_prefix(
        deduplicator_failure(),
        &["collector.json", "generated.json", "failure.json"],
    );
    assert_prefix(
        stage_failure(Tier2Stage::Verifier),
        &[
            "collector.json",
            "generated.json",
            "dedup.json",
            "failure.json",
        ],
    );
}

fn assert_prefix(failure: Tier2Failure, expected: &[&str]) {
    let root = TempRoot::new();
    seed_tier_two_cursor(&root);
    let expected_reason = failure.status_reason();
    assert_eq!(
        record_tier2(root.path(), REPO, Tier2Scan::Failed(failure)).expect("failed receipt"),
        Tier2Progress::Failed(expected_reason.clone())
    );
    let store = super::tier2_receipts_tests::store(&root);
    let receipt = store
        .load_receipt(1, NoWorkTier::Tier2)
        .expect("receipt")
        .expect("sealed receipt");
    assert_eq!(
        receipt
            .evidence()
            .iter()
            .map(|item| item.reference.rsplit('/').next().expect("file name"))
            .collect::<Vec<_>>(),
        expected
    );
    drop(store);
    assert_eq!(
        record_tier2(root.path(), REPO, Tier2Scan::NotRun).expect("failed receipt replay"),
        Tier2Progress::Failed(expected_reason)
    );
}

fn stage_failure(stage: Tier2Stage) -> Tier2Failure {
    let raw = Tier2Failure::new(
        stage,
        Tier2FailureCode::InvalidProposal,
        "injected stage failure",
    )
    .expect("bounded failure");
    let input = match stage {
        Tier2Stage::Generator => Tier2Input::Enabled {
            collector: Tier2StageResult::Complete(collector()),
            generator: Tier2StageResult::Failed(raw),
            verifier: Tier2StageResult::Missing,
            roi_policy: Tier2RoiPolicy::v1(),
        },
        Tier2Stage::Verifier => Tier2Input::Enabled {
            collector: Tier2StageResult::Complete(collector()),
            generator: Tier2StageResult::Complete(Tier2GeneratedProposals {
                generator_identity: "test-generator".to_string(),
                generator_protocol_version: "v1".to_string(),
                proposals: vec![proposal("verified")],
            }),
            verifier: Tier2StageResult::Failed(raw),
            roi_policy: Tier2RoiPolicy::v1(),
        },
        _ => panic!("test only injects generator or verifier failures"),
    };
    evaluate_tier2(input).expect_err("injected stage failure must be sealed")
}
