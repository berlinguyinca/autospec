use std::fs;

use autospec_core::autonomous::no_work::{DryReason, NoWorkTier};
use autospec_core::autonomous::waterfall::{FunnelCounts, TierReceipt, TierStatus};

use super::tier2_receipts_tests::{store, TempRoot, REPO};
use super::tier4::Tier4Scan;
use super::tier4_receipts::{record_tier4, Tier4Progress};
use super::tier4_receipts_tests::{produced_observation, seed_tier_four_cursor};

#[test]
fn retained_tier4_history_is_replayed_from_the_prior_pass() {
    let root = TempRoot::new();
    seed_tier_four_cursor(&root);
    assert_eq!(
        record_tier4(
            root.path(),
            REPO,
            Tier4Scan::Complete(super::tier4_receipts_tests::observation(
                super::tier4_receipts_tests::source(&[]),
                Vec::new(),
                Vec::new(),
            )),
        )
        .expect("dry Tier 4 receipt"),
        Tier4Progress::Advanced
    );
    let history = root.path().join("waterfall/waterfall/1/tier1.json");
    fs::remove_file(history).expect("remove retained pass-one receipt");
    assert!(store(&root).load_state().is_err());
}

#[test]
fn forged_non_advancing_tier4_state_is_rejected() {
    for status in [
        TierStatus::Produced { count: 1 },
        TierStatus::NotRun {
            reason: "forged-not-run".to_string(),
        },
        TierStatus::Failed {
            reason: "tier4_generator_invalid_candidate".to_string(),
        },
        TierStatus::Blocked {
            reason: "forged-block".to_string(),
        },
        TierStatus::Exhausted {
            reason: DryReason::Deduplicated,
        },
    ] {
        let root = TempRoot::new();
        seed_tier_four_cursor(&root);
        assert_eq!(
            record_tier4(
                root.path(),
                REPO,
                Tier4Scan::Complete(super::tier4_receipts_tests::observation(
                    super::tier4_receipts_tests::source(&[]),
                    Vec::new(),
                    Vec::new(),
                )),
            )
            .expect("valid dry receipt"),
            Tier4Progress::Advanced
        );
        let receipt_store = store(&root);
        let prior = receipt_store
            .load_receipt(1, NoWorkTier::Tier4)
            .expect("receipt")
            .expect("sealed receipt");
        let forged = TierReceipt::new(
            REPO,
            1,
            NoWorkTier::Tier4,
            "rust-tier4-external-discovery-receipts-v1",
            1,
            1,
            status,
            FunnelCounts::new(0, 0, 0, 0, 0).expect("zero funnel"),
            prior.evidence().to_vec(),
        )
        .expect("syntactic forged receipt");
        fs::write(
            receipt_store.receipt_path(&prior).expect("receipt path"),
            format!("{}\n", forged.to_json()),
        )
        .expect("replace Tier 4 receipt");
        let state_path = receipt_store.state_path().to_path_buf();
        let state = fs::read_to_string(&state_path).expect("state document");
        fs::write(
            &state_path,
            state.replacen(prior.digest(), forged.digest(), 1),
        )
        .expect("forge retained state digest");
        drop(receipt_store);
        assert!(store(&root).load_state().is_err());
    }
}

#[test]
fn produced_tier4_receipt_replays_without_advancing_or_mutating_history() {
    let root = TempRoot::new();
    seed_tier_four_cursor(&root);
    assert_eq!(
        record_tier4(
            root.path(),
            REPO,
            Tier4Scan::Complete(produced_observation()),
        )
        .expect("produced receipt"),
        Tier4Progress::Produced(1)
    );
    assert_eq!(
        record_tier4(root.path(), REPO, Tier4Scan::NotRun).expect("produced replay"),
        Tier4Progress::Produced(1)
    );
    assert_eq!(
        store(&root)
            .load_state()
            .expect("state")
            .expect("cursor")
            .current_tier(),
        NoWorkTier::Tier4
    );
}
