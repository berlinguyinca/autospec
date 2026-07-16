use autospec_core::autonomous::no_work::NoWorkTier;
use autospec_core::autonomous::tier4::{Tier4Failure, Tier4FailureCode, Tier4Stage};

use super::tier2_receipts_tests::{store, TempRoot, REPO};
use super::tier4::Tier4Scan;
use super::tier4_receipts::{record_tier4, Tier4Progress};
use super::tier4_receipts_tests::{generator_failure, seed_tier_four_cursor};

#[test]
fn tier4_generator_failure_retains_only_its_completed_evidence_prefix() {
    let root = TempRoot::new();
    seed_tier_four_cursor(&root);
    assert_eq!(
        record_tier4(root.path(), REPO, Tier4Scan::Failed(generator_failure()),)
            .expect("sealed failure receipt"),
        Tier4Progress::Failed("tier4_generator_invalid_candidate".to_string())
    );
    let receipt = store(&root)
        .load_receipt(1, NoWorkTier::Tier4)
        .expect("receipt")
        .expect("sealed receipt");
    assert_eq!(
        receipt
            .evidence()
            .iter()
            .map(|evidence| evidence.reference.as_str())
            .collect::<Vec<_>>(),
        vec![
            "waterfall/1/tier4/source_policy.json",
            "waterfall/1/tier4/sources.json",
            "waterfall/1/tier4/failure.json",
        ]
    );
    assert_eq!(
        record_tier4(root.path(), REPO, Tier4Scan::NotRun).expect("failure replay"),
        Tier4Progress::Failed("tier4_generator_invalid_candidate".to_string())
    );
}

#[test]
fn tier4_unsealed_failure_cannot_create_a_receipt() {
    let root = TempRoot::new();
    seed_tier_four_cursor(&root);
    let failure = Tier4Failure::new(
        Tier4Stage::Sources,
        Tier4FailureCode::InvalidSourceEnvelope,
        "raw failure has no evaluated partial evidence",
    )
    .expect("unsealed failure shape");
    assert!(record_tier4(root.path(), REPO, Tier4Scan::Failed(failure)).is_err());
    assert!(store(&root)
        .load_receipt(1, NoWorkTier::Tier4)
        .expect("receipt lookup")
        .is_none());
}
