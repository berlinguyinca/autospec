use std::fs;

use autospec_core::autonomous::no_work::{DryReason, NoWorkTier};
use autospec_core::autonomous::waterfall::{
    sha256_hex, FunnelCounts, SealedEvidence, TierReceipt, TierStatus,
};
use autospec_core::explore::specialists::{StrictCollectorError, StrictCollectorErrorCode};

use super::tier2::{strict_collector_failure, Tier2Scan};
use super::tier2_receipts::{record_tier2, Tier2Progress};
use super::tier2_receipts_tests::{
    observation, proposal, seed_tier_two_cursor, store, survives, TempRoot, REPO,
};
use super::waterfall::Tier2EvidenceArtifact;

#[test]
fn tier2_replays_pre_cursor_receipts_and_ignores_unreferenced_disk_files() {
    let root = TempRoot::new();
    seed_tier_two_cursor(&root);
    let before =
        fs::read(root.path().join("waterfall/waterfall-state.json")).expect("Tier 2 state");
    assert_eq!(
        record_tier2(
            root.path(),
            REPO,
            Tier2Scan::Complete(observation(Vec::new(), Vec::new()))
        )
        .expect("receipt"),
        Tier2Progress::Advanced
    );
    fs::write(root.path().join("waterfall/waterfall-state.json"), before)
        .expect("restore pre-cursor state");
    fs::write(
        root.path()
            .join("waterfall/waterfall/1/tier2/unreferenced.json"),
        "{\"extra\":true}\n",
    )
    .expect("extra disk file");
    assert_eq!(
        record_tier2(root.path(), REPO, Tier2Scan::NotRun).expect("unreferenced file ignored"),
        Tier2Progress::Advanced
    );
}

#[test]
fn tier2_replay_rejects_missing_or_tampered_referenced_artifacts() {
    for missing in [false, true] {
        let root = TempRoot::new();
        seed_tier_two_cursor(&root);
        assert_eq!(
            record_tier2(
                root.path(),
                REPO,
                Tier2Scan::Complete(observation(Vec::new(), Vec::new()))
            )
            .expect("receipt"),
            Tier2Progress::Advanced
        );
        let path = root
            .path()
            .join("waterfall/waterfall/1/tier2/collector.json");
        if missing {
            fs::remove_file(path).expect("remove evidence");
        } else {
            fs::write(path, "{\"schema\":1,\"kind\":\"tampered\"}\n").expect("tamper evidence");
        }
        assert!(record_tier2(root.path(), REPO, Tier2Scan::NotRun).is_err());
        assert!(
            store(&root).load_state().is_err(),
            "a completed Tier 2 cursor must reject a missing or tampered receipt artifact"
        );
    }
}

#[test]
fn tier2_replay_rejects_misordered_or_extra_receipt_references() {
    for extra in [false, true] {
        let root = TempRoot::new();
        seed_tier_two_cursor(&root);
        assert_eq!(
            record_tier2(
                root.path(),
                REPO,
                Tier2Scan::Complete(observation(vec![proposal("one")], vec![survives("one")]))
            )
            .expect("produced receipt"),
            Tier2Progress::Produced(1)
        );
        let store = store(&root);
        let receipt = store
            .load_receipt(1, NoWorkTier::Tier2)
            .expect("receipt")
            .expect("sealed receipt");
        let mut evidence = receipt.evidence().to_vec();
        if extra {
            evidence.push(evidence[0].clone());
        } else {
            evidence.swap(0, 1);
        }
        let malformed = receipt_with(
            TierStatus::Produced { count: 1 },
            FunnelCounts::new(1, 1, 1, 1, 1).expect("funnel"),
            evidence,
        );
        fs::write(
            store.receipt_path(&receipt).expect("receipt path"),
            format!("{}\n", malformed.to_json()),
        )
        .expect("replace receipt");
        drop(store);
        assert!(record_tier2(root.path(), REPO, Tier2Scan::NotRun).is_err());
    }
}

#[test]
fn tier2_replay_rejects_rehashed_whitespace_or_escape_variants() {
    for replacement in [
        ("whitespace", "\"schema\": 1"),
        ("unnecessary unicode escape", "\"stable_key\":\"\\u006fne\""),
    ] {
        let root = TempRoot::new();
        seed_tier_two_cursor(&root);
        assert_eq!(
            record_tier2(
                root.path(),
                REPO,
                Tier2Scan::Complete(observation(vec![proposal("one")], vec![survives("one")]))
            )
            .expect("receipt"),
            Tier2Progress::Produced(1)
        );
        let store = store(&root);
        let prior = store
            .load_receipt(1, NoWorkTier::Tier2)
            .expect("receipt")
            .expect("sealed receipt");
        let original = fs::read_to_string(
            root.path()
                .join("waterfall")
                .join(&prior.evidence()[4].reference),
        )
        .expect("collector evidence");
        let contents = if replacement.0 == "whitespace" {
            original.replacen("\"schema\":1", replacement.1, 1)
        } else {
            original.replacen("\"stable_key\":\"one\"", replacement.1, 1)
        };
        assert_ne!(contents, original, "test variant must modify the artifact");
        let replacement_receipt = replace_evidence(&root, &store, &prior, 4, contents);
        let verification = store.verify_tier2_evidence(1, &replacement_receipt);
        assert!(
            verification.is_err(),
            "{}/rehashed document must fail Tier 2 evidence verification",
            replacement.0
        );
        drop(store);
        assert!(
            record_tier2(root.path(), REPO, Tier2Scan::NotRun).is_err(),
            "{}/rehashed document must not become replayable evidence",
            replacement.0
        );
    }
}

#[test]
fn tier2_replay_rejects_terminal_status_that_contradicts_the_funnel() {
    let root = TempRoot::new();
    seed_tier_two_cursor(&root);
    assert_eq!(
        record_tier2(
            root.path(),
            REPO,
            Tier2Scan::Complete(observation(vec![proposal("one")], vec![survives("one")]))
        )
        .expect("produced receipt"),
        Tier2Progress::Produced(1)
    );
    let receipt_store = store(&root);
    let prior = receipt_store
        .load_receipt(1, NoWorkTier::Tier2)
        .expect("receipt")
        .expect("sealed receipt");
    let forged = receipt_with(
        TierStatus::Exhausted {
            reason: DryReason::NoProposalsGenerated,
        },
        prior.funnel().clone(),
        prior.evidence().to_vec(),
    );
    fs::write(
        receipt_store.receipt_path(&prior).expect("receipt path"),
        format!("{}\n", forged.to_json()),
    )
    .expect("replace receipt");
    drop(receipt_store);
    assert!(record_tier2(root.path(), REPO, Tier2Scan::NotRun).is_err());
    assert_eq!(
        store(&root)
            .load_state()
            .expect("state")
            .expect("cursor")
            .current_tier(),
        NoWorkTier::Tier2
    );
}

#[test]
fn tier2_replay_rejects_blank_or_oversized_failure_details() {
    for detail in [" ".to_string(), "é".repeat(241)] {
        let root = TempRoot::new();
        seed_tier_two_cursor(&root);
        assert!(matches!(
            record_tier2(
                root.path(),
                REPO,
                Tier2Scan::Failed(strict_collector_failure(StrictCollectorError {
                    code: StrictCollectorErrorCode::ReadFile,
                    detail: "collector read failed".to_string(),
                }))
            ),
            Ok(Tier2Progress::Failed(_))
        ));
        let store = store(&root);
        let prior = store
            .load_receipt(1, NoWorkTier::Tier2)
            .expect("receipt")
            .expect("sealed receipt");
        let original = fs::read_to_string(
            root.path()
                .join("waterfall")
                .join(&prior.evidence()[0].reference),
        )
        .expect("failure evidence");
        let contents = original.replacen(
            "\"detail\":\"collector read failed\"",
            &format!("\"detail\":\"{detail}\""),
            1,
        );
        replace_evidence(&root, &store, &prior, 0, contents);
        drop(store);
        assert!(record_tier2(root.path(), REPO, Tier2Scan::NotRun).is_err());
    }
}

#[test]
fn tier2_persistence_conflict_writes_no_receipt_and_retains_cursor() {
    let root = TempRoot::new();
    seed_tier_two_cursor(&root);
    let conflict = root
        .path()
        .join("waterfall/waterfall/1/tier2/collector.json");
    fs::create_dir_all(conflict.parent().expect("artifact parent")).expect("artifact parent");
    fs::write(conflict, "{\"schema\":1,\"kind\":\"foreign\"}\n").expect("conflicting artifact");

    assert!(record_tier2(
        root.path(),
        REPO,
        Tier2Scan::Complete(observation(Vec::new(), Vec::new()))
    )
    .is_err());
    let store = store(&root);
    assert!(store
        .load_receipt(1, NoWorkTier::Tier2)
        .expect("receipt lookup")
        .is_none());
    assert_eq!(
        store
            .load_state()
            .expect("state")
            .expect("cursor")
            .current_tier(),
        NoWorkTier::Tier2
    );
}

#[test]
fn tier2_replay_validates_manual_roi_rank_failure_prefix() {
    let root = TempRoot::new();
    seed_tier_two_cursor(&root);
    assert_eq!(
        record_tier2(
            root.path(),
            REPO,
            Tier2Scan::Complete(observation(vec![proposal("one")], vec![survives("one")]))
        )
        .expect("produced receipt"),
        Tier2Progress::Produced(1)
    );
    let initial_store = store(&root);
    let prior = initial_store
        .load_receipt(1, NoWorkTier::Tier2)
        .expect("receipt")
        .expect("sealed receipt");
    let mut evidence = prior.evidence()[..4].to_vec();
    let predecessor = evidence
        .last()
        .expect("verification evidence")
        .digest
        .clone();
    let failure = initial_store
        .persist_tier2_evidence(
            1,
            Tier2EvidenceArtifact::Failure,
            &format!(
                "{{\"schema\":1,\"kind\":\"tier2_failure\",\"predecessor_digest\":\"{predecessor}\",\"stage\":\"roi_rank\",\"code\":\"invalid_ranking\",\"status_reason\":\"tier2_roi_rank_invalid_ranking\",\"detail\":\"rank policy could not complete\",\"funnel\":{{\"observed\":1,\"deduplicated\":1,\"verified\":1,\"roi_approved\":0,\"ranked\":0}}}}\n"
            ),
        )
        .expect("manual failure evidence");
    evidence.push(failure);
    let failed = receipt_with(
        TierStatus::Failed {
            reason: "tier2_roi_rank_invalid_ranking".to_string(),
        },
        FunnelCounts::new(1, 1, 1, 0, 0).expect("funnel"),
        evidence,
    );
    fs::write(
        initial_store.receipt_path(&prior).expect("receipt path"),
        format!("{}\n", failed.to_json()),
    )
    .expect("replace receipt");
    drop(initial_store);
    assert_eq!(
        record_tier2(root.path(), REPO, Tier2Scan::NotRun).expect("manual receipt replay"),
        Tier2Progress::Failed("tier2_roi_rank_invalid_ranking".to_string())
    );

    let forged = fs::read_to_string(root.path().join("waterfall/waterfall/1/tier2/failure.json"))
        .expect("failure evidence")
        .replacen(
            "\"roi_approved\":0,\"ranked\":0",
            "\"roi_approved\":1,\"ranked\":1",
            1,
        );
    let receipt_store = store(&root);
    replace_evidence(&root, &receipt_store, &failed, 4, forged);
    drop(receipt_store);
    assert!(record_tier2(root.path(), REPO, Tier2Scan::NotRun).is_err());
}

fn receipt_with(
    status: TierStatus,
    funnel: FunnelCounts,
    evidence: Vec<SealedEvidence>,
) -> TierReceipt {
    TierReceipt::new(
        REPO,
        1,
        NoWorkTier::Tier2,
        "rust-tier2-local-receipts-v1",
        1,
        1,
        status,
        funnel,
        evidence,
    )
    .expect("syntactically sealed receipt")
}

fn replace_evidence(
    root: &TempRoot,
    store: &super::waterfall::WaterfallStore,
    prior: &TierReceipt,
    index: usize,
    contents: String,
) -> TierReceipt {
    let mut evidence = prior.evidence().to_vec();
    let reference = evidence[index].reference.clone();
    evidence[index] = SealedEvidence::new(&reference, sha256_hex(contents.as_bytes()))
        .expect("rehashed evidence");
    let replacement = receipt_with(prior.status().clone(), prior.funnel().clone(), evidence);
    fs::write(root.path().join("waterfall").join(reference), contents).expect("replace evidence");
    fs::write(
        store.receipt_path(prior).expect("receipt path"),
        format!("{}\n", replacement.to_json()),
    )
    .expect("replace receipt");
    replacement
}
