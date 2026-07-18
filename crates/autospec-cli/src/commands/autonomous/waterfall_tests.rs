use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use autospec_core::autonomous::no_work::{DryReason, NoWorkTier};
use autospec_core::autonomous::waterfall::{
    FunnelCounts, SealedEvidence, TierReceipt, TierStatus, WaterfallState,
};

use super::waterfall::{
    retry_transient_lock, StoreAcquisition, Tier15EvidenceArtifact, Tier1EvidenceArtifact,
    WaterfallStore,
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
        "rust-foreground-tier1-v1",
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
        "rust-foreground-tier1-v1",
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
        "rust-tier1_5-read-only-v1",
        12,
        13,
        status,
        FunnelCounts::new(3, 2, 2, 0, 0).expect("funnel counts"),
        vec![evidence],
    )
    .expect("receipt")
}

fn acquire(root: &TempRoot) -> WaterfallStore {
    let acquisition = retry_transient_lock(
        || WaterfallStore::acquire(root.path(), "owner/repo"),
        |result| matches!(result, Ok(StoreAcquisition::Held)),
    );
    match acquisition.expect("store acquisition") {
        StoreAcquisition::Acquired(store) => store,
        StoreAcquisition::Held => panic!("test store remained locked for one second"),
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
fn cloned_lock_descriptor_blocks_until_its_last_owner_closes() {
    let root = TempRoot::new();
    let first = acquire(&root);
    let inherited_lock = first.clone_lock_for_test();
    drop(first);
    assert!(matches!(
        WaterfallStore::acquire(root.path(), "owner/repo").expect("cloned-lock acquisition"),
        StoreAcquisition::Held
    ));

    let waterfall_root = root.path().to_path_buf();
    let (acquired_tx, acquired_rx) = mpsc::channel();
    let acquisition = thread::spawn(move || {
        let result = WaterfallStore::acquire_for_receipts(waterfall_root, "owner/repo", None);
        acquired_tx.send(result).expect("send receipt acquisition");
    });
    assert!(matches!(
        acquired_rx.recv_timeout(Duration::from_millis(25)),
        Err(RecvTimeoutError::Timeout)
    ));

    drop(inherited_lock);
    let acquired = acquired_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("receipt acquisition completed after inherited lock closed");
    assert!(matches!(acquired, Ok(StoreAcquisition::Acquired(_))));
    acquisition.join().expect("receipt acquisition thread");
}

#[test]
fn receipt_acquisition_returns_held_after_bounded_retry_deadline() {
    let root = TempRoot::new();
    let first = acquire(&root);
    let started = Instant::now();

    let acquisition = WaterfallStore::acquire_for_receipts(root.path(), "owner/repo", None)
        .expect("bounded receipt acquisition");

    assert!(matches!(acquisition, StoreAcquisition::Held));
    assert!(started.elapsed() >= Duration::from_secs(1));
    assert!(started.elapsed() < Duration::from_secs(3));
    drop(first);
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
                "{\"schema\":1,\"kind\":\"ready_page\",\"gate_counts\":{\"open\":0,\"candidate\":0,\"reviewed\":0,\"blocked\":0,\"ready\":0,\"claimed\":0,\"selected\":0},\"worker_cap\":{\"active_count\":0,\"remaining\":1,\"reached\":false}}\n",
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
                "{\"schema\":1,\"kind\":\"tier15_observation\",\"open_observed\":2,\"open_deduplicated\":1,\"closed_observed\":1,\"budget\":1,\"decisions\":[{\"number\":7,\"classification\":\"unlabeled\",\"decision\":\"held\",\"reason\":\"thin_intent\"}]}\n",
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
            "{\"schema\":1,\"kind\":\"ready_page\",\"gate_counts\":{\"open\":0,\"candidate\":0,\"reviewed\":0,\"blocked\":0,\"ready\":0,\"claimed\":0,\"selected\":0},\"worker_cap\":{\"active_count\":0,\"remaining\":1,\"reached\":false}}\n",
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
fn store_rejects_self_consistent_tier_one_receipt_with_wrong_producer() {
    let root = TempRoot::new();
    let store = acquire(&root);
    let evidence = store
        .persist_tier1_evidence(
            1,
            Tier1EvidenceArtifact::ReadyPage,
            "{\"schema\":1,\"kind\":\"ready_page\",\"gate_counts\":{\"open\":0,\"candidate\":0,\"reviewed\":0,\"blocked\":0,\"ready\":0,\"claimed\":0,\"selected\":0},\"worker_cap\":{\"active_count\":0,\"remaining\":1,\"reached\":false}}\n",
        )
        .expect("persist canonical Tier 1 evidence");
    let forged = TierReceipt::new(
        "owner/repo",
        1,
        NoWorkTier::Tier1,
        "forged-tier1-producer",
        1,
        1,
        TierStatus::Exhausted {
            reason: DryReason::NoProposalsGenerated,
        },
        FunnelCounts::new(0, 0, 0, 0, 0).expect("counts"),
        vec![evidence],
    )
    .expect("self-consistent forged receipt");
    store
        .persist_receipt(&forged)
        .expect("persist forged receipt");
    let state = WaterfallState::new("owner/repo", 1, NoWorkTier::Tier1)
        .expect("state")
        .record_receipt(&forged)
        .expect("forge advances core cursor");

    assert!(
        store.persist_state(&state).is_err(),
        "Tier 1 replay must bind the native producer and evidence semantics"
    );
}

#[test]
fn store_rejects_self_consistent_tier15_funnel_forgery() {
    let root = TempRoot::new();
    let store = acquire(&root);
    let tier_one_evidence = store
        .persist_tier1_evidence(
            1,
            Tier1EvidenceArtifact::ReadyPage,
            "{\"schema\":1,\"kind\":\"ready_page\",\"gate_counts\":{\"open\":0,\"candidate\":0,\"reviewed\":0,\"blocked\":0,\"ready\":0,\"claimed\":0,\"selected\":0},\"worker_cap\":{\"active_count\":0,\"remaining\":1,\"reached\":false}}\n",
        )
        .expect("Tier 1 evidence");
    let tier_one = TierReceipt::new(
        "owner/repo",
        1,
        NoWorkTier::Tier1,
        "rust-foreground-tier1-v1",
        1,
        1,
        TierStatus::Exhausted {
            reason: DryReason::NoProposalsGenerated,
        },
        FunnelCounts::new(0, 0, 0, 0, 0).expect("counts"),
        vec![tier_one_evidence],
    )
    .expect("Tier 1 receipt");
    store.persist_receipt(&tier_one).expect("Tier 1 receipt");
    let tier15_state = WaterfallState::new("owner/repo", 1, NoWorkTier::Tier1)
        .expect("state")
        .record_receipt(&tier_one)
        .expect("Tier 1.5 state");
    let observation = store
        .persist_tier15_evidence(
            1,
            Tier15EvidenceArtifact::Observation,
            "{\"schema\":1,\"kind\":\"tier15_observation\",\"open_observed\":1,\"open_deduplicated\":1,\"closed_observed\":0,\"budget\":1,\"decisions\":[]}\n",
        )
        .expect("Tier 1.5 evidence");
    let forged = TierReceipt::new(
        "owner/repo",
        1,
        NoWorkTier::Tier1_5,
        "rust-tier1_5-read-only-v1",
        2,
        2,
        TierStatus::Exhausted {
            reason: DryReason::NoProposalsGenerated,
        },
        FunnelCounts::new(0, 0, 0, 0, 0).expect("forged counts"),
        vec![observation],
    )
    .expect("self-consistent forged receipt");
    store.persist_receipt(&forged).expect("forged receipt");
    let state = tier15_state
        .record_receipt(&forged)
        .expect("forged Tier 1.5 cursor");

    assert!(
        store.persist_state(&state).is_err(),
        "Tier 1.5 replay must reconstruct its funnel from canonical evidence"
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
            "{\"schema\":1,\"kind\":\"ready_page\",\"gate_counts\":{\"open\":0,\"candidate\":0,\"reviewed\":0,\"blocked\":0,\"ready\":0,\"claimed\":0,\"selected\":0},\"worker_cap\":{\"active_count\":0,\"remaining\":1,\"reached\":false}}\n",
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
            "{\"schema\":1,\"kind\":\"tier15_observation\",\"open_observed\":2,\"open_deduplicated\":1,\"closed_observed\":1,\"budget\":1,\"decisions\":[{\"number\":7,\"classification\":\"unlabeled\",\"decision\":\"held\",\"reason\":\"thin_intent\"}]}\n",
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
