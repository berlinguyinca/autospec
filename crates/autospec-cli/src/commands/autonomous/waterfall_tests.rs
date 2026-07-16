use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use autospec_core::autonomous::no_work::{DryReason, NoWorkTier};
use autospec_core::autonomous::waterfall::{
    FunnelCounts, SealedEvidence, TierReceipt, TierStatus, WaterfallState,
};

use super::waterfall::{
    StoreAcquisition, Tier15EvidenceArtifact, Tier1EvidenceArtifact, WaterfallStore,
};

static ROOT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

const EVIDENCE_DIGEST: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

struct TempRoot {
    path: PathBuf,
}

impl TempRoot {
    fn new() -> Self {
        let sequence = ROOT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time is after the Unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "autospec-waterfall-store-{}-{nanos}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("temporary root");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn receipt(status: TierStatus) -> TierReceipt {
    TierReceipt::new(
        "owner/repo",
        1,
        NoWorkTier::Tier1,
        "queue-v1",
        10,
        11,
        status,
        FunnelCounts::new(3, 2, 1, 1, 1).expect("funnel counts"),
        vec![SealedEvidence::new("evidence/queue.json", EVIDENCE_DIGEST).expect("sealed evidence")],
    )
    .expect("receipt")
}

fn receipt_with_evidence(status: TierStatus, evidence: SealedEvidence) -> TierReceipt {
    TierReceipt::new(
        "owner/repo",
        1,
        NoWorkTier::Tier1,
        "queue-v1",
        10,
        11,
        status,
        FunnelCounts::new(0, 0, 0, 0, 0).expect("funnel counts"),
        vec![evidence],
    )
    .expect("receipt")
}

fn tier_one_point_five_receipt_with_evidence(
    status: TierStatus,
    evidence: SealedEvidence,
) -> TierReceipt {
    TierReceipt::new(
        "owner/repo",
        1,
        NoWorkTier::Tier1_5,
        "tier15-read-only-v1",
        12,
        13,
        status,
        FunnelCounts::new(3, 2, 2, 1, 1).expect("funnel counts"),
        vec![evidence],
    )
    .expect("receipt")
}

fn acquire(root: &TempRoot) -> WaterfallStore {
    match WaterfallStore::acquire(root.path(), "owner/repo").expect("store acquisition") {
        StoreAcquisition::Acquired(store) => store,
        StoreAcquisition::Held => panic!("fresh temporary store must be available"),
    }
}

#[test]
fn concurrent_cursor_ownership_returns_typed_held_without_writing_state() {
    let root = TempRoot::new();
    let first = acquire(&root);

    assert!(matches!(
        WaterfallStore::acquire(root.path(), "owner/repo").expect("contended acquisition"),
        StoreAcquisition::Held
    ));
    assert!(!first.state_path().exists());
}

#[test]
fn store_atomically_replaces_state_without_overwriting_sealed_receipts() {
    let root = TempRoot::new();
    let store = acquire(&root);
    let exhausted = receipt(TierStatus::Exhausted {
        reason: DryReason::NoProposalsGenerated,
    });
    let advancing = receipt_with_evidence(
        TierStatus::Exhausted {
            reason: DryReason::NoProposalsGenerated,
        },
        store
            .persist_tier1_evidence(
                1,
                Tier1EvidenceArtifact::ReadyPage,
                "{\"schema\":1,\"kind\":\"ready_page\"}\n",
            )
            .expect("persist Tier 1 ready-page evidence"),
    );

    let tier_one_point_five_exhausted = tier_one_point_five_receipt_with_evidence(
        TierStatus::Exhausted {
            reason: DryReason::NoProposalsGenerated,
        },
        store
            .persist_tier15_evidence(
                1,
                Tier15EvidenceArtifact::Observation,
                "{\"schema\":1,\"kind\":\"tier15_observation\"}\n",
            )
            .expect("persist Tier 1.5 observation evidence"),
    );

    store
        .persist_receipt(&advancing)
        .expect("first receipt write");
    store
        .persist_receipt(&advancing)
        .expect("same sealed receipt is idempotent");
    assert!(
        store.persist_receipt(&exhausted).is_err(),
        "a sealed receipt cannot be overwritten"
    );
    store
        .persist_receipt(&tier_one_point_five_exhausted)
        .expect("next-tier receipt write");
    let receipt_path = store.receipt_path(&advancing).expect("receipt path");
    let body = fs::read_to_string(&receipt_path).expect("receipt body");
    assert!(body.contains("\"kind\":\"exhausted\""));
    assert!(!receipt_path
        .parent()
        .expect("receipt parent")
        .read_dir()
        .expect("receipt directory")
        .filter_map(Result::ok)
        .any(|entry| entry.file_name().to_string_lossy().ends_with(".tmp")));

    let first_state = WaterfallState::new("owner/repo", 1, NoWorkTier::Tier1)
        .expect("state")
        .record_receipt(&advancing)
        .expect("advancing receipt is retained");
    store
        .persist_state(&first_state)
        .expect("first atomic state write");
    let state = first_state
        .record_receipt(&tier_one_point_five_exhausted)
        .expect("exhausted receipt advances cursor");
    store
        .persist_state(&state)
        .expect("state replacement write");
    assert_eq!(
        store.load_state().expect("read replaced state"),
        Some(state)
    );

    fs::write(&receipt_path, exhausted.to_json()).expect("tamper sealed receipt");
    assert!(
        store.load_state().is_err(),
        "state cannot accept a different receipt digest"
    );
    fs::remove_file(&receipt_path).expect("remove referenced receipt");
    assert!(
        store.load_state().is_err(),
        "state cannot advance past a missing receipt"
    );

    fs::write(store.state_path(), "{\"schema\":1,\"repo\":\"other/repo\"}").expect("tamper state");
    assert!(store.load_state().is_err(), "hostile state fails closed");
}

#[test]
fn store_rejects_a_cursor_when_its_tier_one_evidence_artifact_is_tampered() {
    let root = TempRoot::new();
    let store = acquire(&root);
    let evidence = store
        .persist_tier1_evidence(
            1,
            Tier1EvidenceArtifact::ReadyPage,
            "{\"schema\":1,\"kind\":\"ready_page\"}\n",
        )
        .expect("persist Tier 1 evidence");
    let receipt = receipt_with_evidence(
        TierStatus::Exhausted {
            reason: DryReason::NoProposalsGenerated,
        },
        evidence,
    );
    store.persist_receipt(&receipt).expect("persist receipt");
    let state = WaterfallState::new("owner/repo", 1, NoWorkTier::Tier1)
        .expect("state")
        .record_receipt(&receipt)
        .expect("advance cursor");
    store.persist_state(&state).expect("persist cursor");

    fs::write(
        root.path().join("waterfall/1/tier1/ready-page.json"),
        "{\"schema\":1,\"kind\":\"tampered\"}\n",
    )
    .expect("tamper Tier 1 evidence");

    assert!(
        store.load_state().is_err(),
        "a cursor must not trust a receipt whose sealed evidence bytes changed"
    );
}

#[test]
fn store_seals_tier_one_point_five_observation_and_failure_evidence() {
    let root = TempRoot::new();
    let store = acquire(&root);

    let observation = store
        .persist_tier15_evidence(
            1,
            Tier15EvidenceArtifact::Observation,
            "{\"schema\":1,\"kind\":\"tier15_observation\"}\n",
        )
        .expect("persist Tier 1.5 observation");
    let failure = store
        .persist_tier15_evidence(
            1,
            Tier15EvidenceArtifact::ReadFailure,
            "{\"schema\":1,\"kind\":\"read_failure\"}\n",
        )
        .expect("persist Tier 1.5 failure");

    assert_eq!(
        observation.reference,
        "waterfall/1/tier1_5/observation.json"
    );
    assert_eq!(failure.reference, "waterfall/1/tier1_5/read-failure.json");
}

#[test]
fn store_rejects_a_cursor_when_tier_one_point_five_evidence_is_tampered() {
    let root = TempRoot::new();
    let store = acquire(&root);
    let tier_one_evidence = store
        .persist_tier1_evidence(
            1,
            Tier1EvidenceArtifact::ReadyPage,
            "{\"schema\":1,\"kind\":\"ready_page\"}\n",
        )
        .expect("persist Tier 1 evidence");
    let tier_one_receipt = receipt_with_evidence(
        TierStatus::Exhausted {
            reason: DryReason::NoProposalsGenerated,
        },
        tier_one_evidence,
    );
    store
        .persist_receipt(&tier_one_receipt)
        .expect("persist Tier 1 receipt");
    let tier_one_point_five_state = WaterfallState::new("owner/repo", 1, NoWorkTier::Tier1)
        .expect("state")
        .record_receipt(&tier_one_receipt)
        .expect("advance to Tier 1.5");

    let evidence = store
        .persist_tier15_evidence(
            1,
            Tier15EvidenceArtifact::Observation,
            "{\"schema\":1,\"kind\":\"tier15_observation\"}\n",
        )
        .expect("persist Tier 1.5 evidence");
    let receipt = tier_one_point_five_receipt_with_evidence(
        TierStatus::Exhausted {
            reason: DryReason::NoProposalsGenerated,
        },
        evidence,
    );
    store
        .persist_receipt(&receipt)
        .expect("persist Tier 1.5 receipt");
    let state = tier_one_point_five_state
        .record_receipt(&receipt)
        .expect("advance to Tier 2");
    store.persist_state(&state).expect("persist cursor");

    fs::write(
        root.path().join("waterfall/1/tier1_5/observation.json"),
        "{\"schema\":1,\"kind\":\"tampered\"}\n",
    )
    .expect("tamper Tier 1.5 evidence");

    assert!(
        store.load_state().is_err(),
        "a cursor must reject tampered Tier 1.5 sealed evidence"
    );
}
