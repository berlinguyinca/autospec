use std::fs;

use autospec_core::autonomous::no_work::{DryReason, NoWorkTier};
use autospec_core::autonomous::tier3::Tier3Stage;
use autospec_core::autonomous::waterfall::{
    sha256_hex, FunnelCounts, SealedEvidence, TierReceipt, TierStatus,
};

use super::tier2_receipts_tests::{store, TempRoot, REPO};
use super::tier3::Tier3Scan;
use super::tier3_receipts::{record_tier3, Tier3Progress};
use super::tier3_receipts_tests::{finding, observation, seed_tier_three_cursor};

#[test]
fn tier3_replay_rejects_missing_tampered_extra_and_misordered_artifacts() {
    for mutation in ["missing", "tampered", "extra", "misordered"] {
        let root = TempRoot::new();
        seed_tier_three_cursor(&root);
        let receipt = produced(&root);
        let receipt_store = store(&root);
        match mutation {
            "missing" => fs::remove_file(root.path().join("waterfall").join(&receipt.evidence()[0].reference)).expect("remove artifact"),
            "tampered" => fs::write(root.path().join("waterfall").join(&receipt.evidence()[0].reference), "{\"schema\":1,\"kind\":\"tier3_architecture\",\"adapter_version\":\"tampered\",\"rule_version\":\"rules-v1\",\"findings\":[]}\n").expect("tamper artifact"),
            "extra" | "misordered" => {
                let mut evidence = receipt.evidence().to_vec();
                if mutation == "extra" { evidence.push(evidence[0].clone()); } else { evidence.swap(0, 1); }
                let replacement = receipt_with(receipt.status().clone(), receipt.funnel().clone(), evidence);
                fs::write(receipt_store.receipt_path(&receipt).expect("path"), format!("{}\n", replacement.to_json())).expect("replace receipt");
            }
            _ => unreachable!("fixed mutation list"),
        }
        drop(receipt_store);
        assert!(
            record_tier3(root.path(), REPO, Tier3Scan::NotRun).is_err(),
            "{mutation} evidence must not replay"
        );
    }
}

#[test]
fn tier3_replay_precedes_cursor_write_and_ignores_unreferenced_evidence() {
    let root = TempRoot::new();
    seed_tier_three_cursor(&root);
    let before = fs::read(root.path().join("waterfall/waterfall-state.json")).expect("cursor");
    assert_eq!(
        record_tier3(
            root.path(),
            REPO,
            Tier3Scan::Complete(observation(Vec::new()))
        )
        .expect("metadata receipt"),
        Tier3Progress::Advanced
    );
    fs::write(root.path().join("waterfall/waterfall-state.json"), before).expect("restore cursor");
    fs::write(
        root.path()
            .join("waterfall/waterfall/1/tier3/unreferenced.json"),
        "{\"extra\":true}\n",
    )
    .expect("unreferenced evidence");
    assert_eq!(
        record_tier3(root.path(), REPO, Tier3Scan::NotRun).expect("receipt replay"),
        Tier3Progress::Advanced
    );
}

#[test]
fn tier3_disabled_receipt_rejects_changed_policy_identity_funnel_and_artifact_set() {
    for mutation in ["producer", "funnel", "extra", "policy"] {
        let root = TempRoot::new();
        seed_tier_three_cursor(&root);
        assert!(matches!(
            record_tier3(root.path(), REPO, Tier3Scan::NotRun),
            Ok(Tier3Progress::NotRun(_))
        ));
        let receipt_store = store(&root);
        let prior = receipt_store
            .load_receipt(1, NoWorkTier::Tier3)
            .expect("receipt")
            .expect("sealed");
        let mut evidence = prior.evidence().to_vec();
        let (producer, funnel) = match mutation {
            "producer" => ("forged-policy-v1", zero()),
            "funnel" => (
                prior.producer_version(),
                FunnelCounts::new(1, 1, 1, 1, 1).expect("funnel"),
            ),
            "extra" => {
                evidence.push(evidence[0].clone());
                (prior.producer_version(), zero())
            }
            "policy" => {
                let contents = "{\"schema\":1,\"kind\":\"tier3_policy\",\"mode\":\"disabled\",\"reason\":\"forged\",\"policy_source\":\"checked_in\"}\n";
                evidence[0] =
                    SealedEvidence::new(&evidence[0].reference, sha256_hex(contents.as_bytes()))
                        .expect("digest");
                fs::write(
                    root.path().join("waterfall").join(&evidence[0].reference),
                    contents,
                )
                .expect("replace policy");
                (prior.producer_version(), zero())
            }
            _ => unreachable!("fixed mutation list"),
        };
        let replacement = TierReceipt::new(
            REPO,
            1,
            NoWorkTier::Tier3,
            producer,
            1,
            1,
            prior.status().clone(),
            funnel,
            evidence,
        )
        .expect("syntactic receipt");
        fs::write(
            receipt_store.receipt_path(&prior).expect("path"),
            format!("{}\n", replacement.to_json()),
        )
        .expect("replace receipt");
        drop(receipt_store);
        assert!(
            record_tier3(root.path(), REPO, Tier3Scan::NotRun).is_err(),
            "{mutation} disabled receipt must fail"
        );
    }
}

#[test]
fn tier3_replay_rejects_empty_findings_that_discard_sealed_adapter_facts() {
    let root = TempRoot::new();
    seed_tier_three_cursor(&root);
    let prior = produced(&root);
    let receipt_store = store(&root);
    let predecessor = prior.evidence()[2].digest.clone();
    let contents = format!(
        "{{\"schema\":1,\"kind\":\"tier3_findings\",\"predecessor_digest\":\"{predecessor}\",\"rank_limit\":10,\"funnel\":{{\"observed\":0,\"deduplicated\":0,\"verified\":0,\"roi_approved\":0,\"ranked\":0}},\"deduplicated\":[],\"ranked\":[]}}\n"
    );
    let mut evidence = prior.evidence().to_vec();
    evidence[3] = SealedEvidence::new(&evidence[3].reference, sha256_hex(contents.as_bytes()))
        .expect("findings digest");
    let exhausted = receipt_with(
        TierStatus::Exhausted {
            reason: DryReason::NoMetadataFindings,
        },
        zero(),
        evidence,
    );
    fs::write(
        root.path()
            .join("waterfall")
            .join(&prior.evidence()[3].reference),
        contents,
    )
    .expect("replace findings");
    fs::write(
        receipt_store.receipt_path(&prior).expect("path"),
        format!("{}\n", exhausted.to_json()),
    )
    .expect("replace receipt");
    drop(receipt_store);
    assert!(
        record_tier3(root.path(), REPO, Tier3Scan::NotRun).is_err(),
        "findings cannot erase facts sealed by completed adapters"
    );
}

#[test]
fn tier3_replay_rejects_rehashed_nested_key_order_variants() {
    let root = TempRoot::new();
    seed_tier_three_cursor(&root);
    let prior = produced(&root);
    let receipt_store = store(&root);
    let path = root
        .path()
        .join("waterfall")
        .join(&prior.evidence()[3].reference);
    let original = fs::read_to_string(&path).expect("findings evidence");
    let contents = original.replacen(
        "\"observed\":1,\"deduplicated\":1",
        "\"deduplicated\":1,\"observed\":1",
        1,
    );
    assert_ne!(
        contents, original,
        "nested key order fixture must change evidence"
    );
    let mut evidence = prior.evidence().to_vec();
    evidence[3] = SealedEvidence::new(&evidence[3].reference, sha256_hex(contents.as_bytes()))
        .expect("findings digest");
    let replacement = receipt_with(prior.status().clone(), prior.funnel().clone(), evidence);
    fs::write(path, contents).expect("replace findings");
    fs::write(
        receipt_store.receipt_path(&prior).expect("path"),
        format!("{}\n", replacement.to_json()),
    )
    .expect("replace receipt");
    drop(receipt_store);
    assert!(
        record_tier3(root.path(), REPO, Tier3Scan::NotRun).is_err(),
        "rehashed nested key order must not replay"
    );
}

#[test]
fn tier3_complete_and_failed_receipts_reject_forged_producer_identity() {
    let root = TempRoot::new();
    seed_tier_three_cursor(&root);
    let prior = produced(&root);
    let receipt_store = store(&root);
    let forged = TierReceipt::new(
        REPO,
        1,
        NoWorkTier::Tier3,
        "forged-tier3-producer-v1",
        1,
        1,
        prior.status().clone(),
        prior.funnel().clone(),
        prior.evidence().to_vec(),
    )
    .expect("syntactic receipt");
    fs::write(
        receipt_store.receipt_path(&prior).expect("path"),
        format!("{}\n", forged.to_json()),
    )
    .expect("replace receipt");
    drop(receipt_store);
    assert!(record_tier3(root.path(), REPO, Tier3Scan::NotRun).is_err());

    let failed_root = TempRoot::new();
    seed_tier_three_cursor(&failed_root);
    let failure = super::tier3_receipts_failure_prefix_tests::failure(Tier3Stage::Architecture);
    assert!(matches!(
        record_tier3(failed_root.path(), REPO, Tier3Scan::Failed(failure)),
        Ok(Tier3Progress::Failed(_))
    ));
    let failure_store = store(&failed_root);
    let failure_receipt = failure_store
        .load_receipt(1, NoWorkTier::Tier3)
        .expect("receipt")
        .expect("sealed failure");
    let forged_failure = TierReceipt::new(
        REPO,
        1,
        NoWorkTier::Tier3,
        "forged-tier3-producer-v1",
        1,
        1,
        failure_receipt.status().clone(),
        failure_receipt.funnel().clone(),
        failure_receipt.evidence().to_vec(),
    )
    .expect("syntactic failure receipt");
    fs::write(
        failure_store.receipt_path(&failure_receipt).expect("path"),
        format!("{}\n", forged_failure.to_json()),
    )
    .expect("replace failure receipt");
    drop(failure_store);
    assert!(record_tier3(failed_root.path(), REPO, Tier3Scan::NotRun).is_err());
}

#[test]
fn tier3_completed_cursor_rejects_every_non_metadata_exhaustion_terminal_status() {
    let root = TempRoot::new();
    seed_tier_three_cursor(&root);
    let produced = produced(&root);
    assert_rejected_completed_state(&root, produced.clone());

    let not_run = receipt_with(
        TierStatus::NotRun {
            reason: "disabled-by-policy".to_string(),
        },
        zero(),
        produced.evidence().to_vec(),
    );
    assert_rejected_completed_state(&root, not_run);
    let failed = receipt_with(
        TierStatus::Failed {
            reason: "tier3_architecture_invalid_finding".to_string(),
        },
        zero(),
        produced.evidence().to_vec(),
    );
    assert_rejected_completed_state(&root, failed);
    let other_dry = receipt_with(
        TierStatus::Exhausted {
            reason: DryReason::NoProposalsGenerated,
        },
        zero(),
        produced.evidence().to_vec(),
    );
    assert_rejected_completed_state(&root, other_dry);
}

fn produced(root: &TempRoot) -> TierReceipt {
    assert_eq!(
        record_tier3(
            root.path(),
            REPO,
            Tier3Scan::Complete(observation(vec![finding()]))
        )
        .expect("produced"),
        Tier3Progress::Produced(1)
    );
    store(root)
        .load_receipt(1, NoWorkTier::Tier3)
        .expect("receipt")
        .expect("sealed receipt")
}

fn assert_rejected_completed_state(root: &TempRoot, receipt: TierReceipt) {
    let receipt_store = store(root);
    let original = fs::read(receipt_store.state_path()).expect("original cursor");
    let state = receipt_store
        .load_state()
        .expect("state")
        .expect("cursor")
        .record_receipt(&receipt)
        .expect("generic state records terminal receipt");
    fs::write(receipt_store.state_path(), format!("{}\n", state.to_json()))
        .expect("forge state file");
    drop(receipt_store);
    assert!(
        store(root).load_state().is_err(),
        "CLI state replay must block a forged Tier 3 cursor"
    );
    fs::write(root.path().join("waterfall/waterfall-state.json"), original)
        .expect("restore cursor for next forged state");
}

fn receipt_with(
    status: TierStatus,
    funnel: FunnelCounts,
    evidence: Vec<SealedEvidence>,
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
