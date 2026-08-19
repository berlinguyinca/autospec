use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use autospec_core::autonomous::no_work::{DryReason, NoWorkTier};
use autospec_core::autonomous::tier15::Tier15Observation;
use autospec_core::autonomous::waterfall::{FunnelCounts, SealedEvidence, TierReceipt, TierStatus};

use super::resilience::{with_current_lifecycle_lease, ConductorLease};
use super::tier15::Tier15Scan;
use super::waterfall::{
    StoreAcquisition, Tier15EvidenceArtifact, WaterfallStore, WaterfallStoreError,
};

const PRODUCER_VERSION: &str = "rust-tier1_5-read-only-v1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Tier15Progress {
    Pending,
    Advanced,
    Produced(u64),
    Failed(String),
}

pub(super) enum ReceiptPreflight<T> {
    Replayed(T),
    NeedsCollection,
}

pub(super) fn replay_tier15_with_lease(
    state_root: &Path,
    repo: &str,
    lease: &ConductorLease,
) -> Result<ReceiptPreflight<Tier15Progress>, String> {
    with_current_lifecycle_lease(lease, || replay_tier15_fenced(state_root, repo))
}

pub(super) fn record_tier15_with_lease(
    state_root: &Path,
    repo: &str,
    lease: &ConductorLease,
    scan: Tier15Scan,
) -> Result<Tier15Progress, String> {
    with_current_lifecycle_lease(lease, || record_tier15_fenced(state_root, repo, scan))
}

fn record_tier15_fenced(
    state_root: &Path,
    repo: &str,
    scan: Tier15Scan,
) -> Result<Tier15Progress, String> {
    let root = state_root.join("waterfall");
    let store =
        match WaterfallStore::acquire_for_receipts(&root, repo, None).map_err(store_error)? {
            StoreAcquisition::Acquired(store) => store,
            StoreAcquisition::Held => return Ok(Tier15Progress::Pending),
        };
    let Some(state) = store.load_state().map_err(store_error)? else {
        return Ok(Tier15Progress::Pending);
    };
    if state.current_tier() != NoWorkTier::Tier1_5 {
        return Ok(Tier15Progress::Pending);
    }
    if let Some(receipt) = existing_receipt(&store, &state)? {
        return settle_receipt(&store, &state, receipt);
    }

    let (status, funnel, artifact, contents) = match scan {
        Tier15Scan::Complete(observation) => observation_receipt_parts(&observation)?,
        Tier15Scan::Failed(reason) => (
            TierStatus::Failed {
                reason: reason.clone(),
            },
            FunnelCounts::new(0, 0, 0, 0, 0)?,
            Tier15EvidenceArtifact::ReadFailure,
            read_failure_document(&reason),
        ),
    };
    let evidence = store
        .persist_tier15_evidence(state.next_pass_id(), artifact, &contents)
        .map_err(store_error)?;
    let receipt = receipt(&state, status, funnel, evidence)?;
    store.persist_receipt(&receipt).map_err(store_error)?;
    settle_receipt(&store, &state, receipt)
}

fn replay_tier15_fenced(
    state_root: &Path,
    repo: &str,
) -> Result<ReceiptPreflight<Tier15Progress>, String> {
    let store = match WaterfallStore::acquire_for_receipts(state_root.join("waterfall"), repo, None)
        .map_err(store_error)?
    {
        StoreAcquisition::Acquired(store) => store,
        StoreAcquisition::Held => return Ok(ReceiptPreflight::Replayed(Tier15Progress::Pending)),
    };
    let Some(state) = store.load_state().map_err(store_error)? else {
        return Ok(ReceiptPreflight::Replayed(Tier15Progress::Pending));
    };
    if state.current_tier() != NoWorkTier::Tier1_5 {
        return Ok(ReceiptPreflight::Replayed(Tier15Progress::Pending));
    }
    match existing_receipt(&store, &state)? {
        Some(receipt) => settle_receipt(&store, &state, receipt).map(ReceiptPreflight::Replayed),
        None => Ok(ReceiptPreflight::NeedsCollection),
    }
}

fn existing_receipt(
    store: &WaterfallStore,
    state: &autospec_core::autonomous::waterfall::WaterfallState,
) -> Result<Option<TierReceipt>, String> {
    let Some(receipt) = store
        .load_receipt(state.next_pass_id(), NoWorkTier::Tier1_5)
        .map_err(store_error)?
    else {
        return Ok(None);
    };
    store
        .verify_tier15_evidence(state.next_pass_id(), &receipt)
        .map_err(store_error)?;
    Ok(Some(receipt))
}

fn settle_receipt(
    store: &WaterfallStore,
    state: &autospec_core::autonomous::waterfall::WaterfallState,
    receipt: TierReceipt,
) -> Result<Tier15Progress, String> {
    match receipt.status() {
        TierStatus::Exhausted { .. } => {
            let advanced = state.clone().record_receipt(&receipt)?;
            store.persist_state(&advanced).map_err(store_error)?;
            Ok(Tier15Progress::Advanced)
        }
        TierStatus::Produced { count } => Ok(Tier15Progress::Produced(*count)),
        TierStatus::Failed { reason } => Ok(Tier15Progress::Failed(reason.clone())),
        status => Err(format!(
            "Tier 1.5 receipt has unexpected {} status",
            status.as_str()
        )),
    }
}

fn observation_receipt_parts(
    observation: &Tier15Observation,
) -> Result<(TierStatus, FunnelCounts, Tier15EvidenceArtifact, String), String> {
    let open_observed = funnel_count(observation.open_observed())?;
    let open_deduplicated = funnel_count(observation.open_deduplicated())?;
    let closed_observed = funnel_count(observation.closed_observed())?;
    let observed = open_observed
        .checked_add(closed_observed)
        .ok_or_else(|| "Tier 1.5 observed funnel count overflowed".to_string())?;
    let deduplicated = open_deduplicated
        .checked_add(closed_observed)
        .ok_or_else(|| "Tier 1.5 deduplicated funnel count overflowed".to_string())?;
    let produced = funnel_count(observation.produced_count())?;
    let status = if produced == 0 {
        TierStatus::Exhausted {
            reason: DryReason::NoProposalsGenerated,
        }
    } else {
        TierStatus::Produced { count: produced }
    };
    Ok((
        status,
        FunnelCounts::new(observed, deduplicated, deduplicated, produced, produced)?,
        Tier15EvidenceArtifact::Observation,
        observation.evidence_json(),
    ))
}

fn funnel_count(value: usize) -> Result<u64, String> {
    u64::try_from(value).map_err(|_| "Tier 1.5 funnel count overflowed".to_string())
}

fn receipt(
    state: &autospec_core::autonomous::waterfall::WaterfallState,
    status: TierStatus,
    funnel: FunnelCounts,
    evidence: SealedEvidence,
) -> Result<TierReceipt, String> {
    let now = now_secs();
    TierReceipt::new(
        state.repo(),
        state.next_pass_id(),
        NoWorkTier::Tier1_5,
        PRODUCER_VERSION,
        now,
        now,
        status,
        funnel,
        vec![evidence],
    )
}

fn read_failure_document(reason: &str) -> String {
    format!(
        "{{\"schema\":1,\"kind\":\"read_failure\",\"reason\":\"{}\"}}\n",
        json_escape(reason)
    )
}

fn json_escape(value: &str) -> String {
    value
        .chars()
        .flat_map(|character| match character {
            '\\' => "\\\\".chars().collect::<Vec<_>>(),
            '"' => "\\\"".chars().collect(),
            '\n' => "\\n".chars().collect(),
            '\r' => "\\r".chars().collect(),
            '\t' => "\\t".chars().collect(),
            control if control.is_control() => {
                format!("\\u{:04x}", control as u32).chars().collect()
            }
            other => vec![other],
        })
        .collect()
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn store_error(error: WaterfallStoreError) -> String {
    match error {
        WaterfallStoreError::Diagnostic(reason)
        | WaterfallStoreError::InvalidReceipt(reason)
        | WaterfallStoreError::InvalidState(reason) => reason,
    }
}

#[cfg(test)]
pub(super) fn record_tier15(
    state_root: &Path,
    repo: &str,
    scan: Tier15Scan,
) -> Result<Tier15Progress, String> {
    record_tier15_fenced(state_root, repo, scan)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    use autospec_core::autonomous::no_work::{DryReason, NoWorkTier};
    use autospec_core::autonomous::waterfall::{
        FunnelCounts, TierReceipt, TierStatus, WaterfallState,
    };
    use autospec_core::coordination::{RemoteIssue, RemoteIssuePage};

    use super::{record_tier15, Tier15Progress};
    use crate::commands::autonomous::tier15::{scan_with, IssueState, Tier15Scan};
    use crate::commands::autonomous::waterfall::{
        StoreAcquisition, Tier1EvidenceArtifact, WaterfallStore,
    };

    const REPO: &str = "owner/repo";
    static ROOT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

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
            let path =
                std::env::temp_dir().join(format!("autospec-tier15-receipts-{nanos}-{sequence}",));
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

    fn store(root: &TempRoot) -> WaterfallStore {
        match WaterfallStore::acquire_for_receipts(root.path().join("waterfall"), REPO, None)
            .expect("store acquisition")
        {
            StoreAcquisition::Acquired(store) => store,
            StoreAcquisition::Held => panic!("temporary root has one owner"),
        }
    }

    fn tier_one_receipt(
        evidence: autospec_core::autonomous::waterfall::SealedEvidence,
    ) -> TierReceipt {
        TierReceipt::new(
            REPO,
            1,
            NoWorkTier::Tier1,
            "rust-foreground-tier1-v1",
            1,
            1,
            TierStatus::Exhausted {
                reason: DryReason::NoProposalsGenerated,
            },
            FunnelCounts::new(0, 0, 0, 0, 0).expect("counts"),
            vec![evidence],
        )
        .expect("Tier 1 receipt")
    }

    fn seed_tier_one_point_five_cursor(root: &TempRoot) {
        let store = store(root);
        let evidence = store
            .persist_tier1_evidence(
                1,
                Tier1EvidenceArtifact::ReadyPage,
                "{\"schema\":1,\"kind\":\"ready_page\",\"gate_counts\":{\"open\":0,\"candidate\":0,\"reviewed\":0,\"blocked\":0,\"ready\":0,\"claimed\":0,\"selected\":0},\"worker_cap\":{\"active_count\":0,\"remaining\":1,\"reached\":false}}\n",
            )
            .expect("Tier 1 evidence");
        let receipt = tier_one_receipt(evidence);
        store.persist_receipt(&receipt).expect("Tier 1 receipt");
        let state = WaterfallState::new(REPO, 1, NoWorkTier::Tier1)
            .expect("Tier 1 state")
            .record_receipt(&receipt)
            .expect("advance to Tier 1.5");
        store.persist_state(&state).expect("Tier 1.5 cursor");
    }

    fn page(raw_count: usize, issues: Vec<RemoteIssue>) -> RemoteIssuePage {
        RemoteIssuePage { raw_count, issues }
    }

    fn produced_scan() -> Tier15Scan {
        scan_with(1, |state, page_number| match (state, page_number) {
            (IssueState::Open, 1) => Ok(page(
                1,
                vec![RemoteIssue::open(
                    7,
                    "Feature",
                    "fix: persist the sealed Tier 1.5 observation.",
                    Vec::new(),
                    "autospec",
                )],
            )),
            (IssueState::Closed, 1) => Ok(page(0, Vec::new())),
            _ => panic!("unexpected page"),
        })
    }

    fn exhausted_scan() -> Tier15Scan {
        scan_with(1, |_, page_number| {
            assert_eq!(page_number, 1);
            Ok(page(0, Vec::new()))
        })
    }

    #[test]
    fn produced_receipt_retains_cursor_and_replays_after_evidence_verification() {
        let root = TempRoot::new();
        seed_tier_one_point_five_cursor(&root);

        assert_eq!(
            record_tier15(root.path(), REPO, produced_scan()).expect("record produced scan"),
            Tier15Progress::Produced(1)
        );
        assert_eq!(
            record_tier15(
                root.path(),
                REPO,
                Tier15Scan::Failed("must not replace sealed receipt".to_string())
            )
            .expect("replay produced receipt"),
            Tier15Progress::Produced(1)
        );
        let store = store(&root);
        assert_eq!(
            store
                .load_state()
                .expect("state")
                .expect("cursor")
                .current_tier(),
            NoWorkTier::Tier1_5
        );
        let receipt = store
            .load_receipt(1, NoWorkTier::Tier1_5)
            .expect("receipt")
            .expect("sealed receipt");
        assert!(matches!(
            receipt.status(),
            TierStatus::Produced { count: 1 }
        ));
        store
            .verify_tier15_evidence(1, &receipt)
            .expect("receipt evidence");
        let evidence = fs::read_to_string(
            root.path()
                .join("waterfall")
                .join(&receipt.evidence()[0].reference),
        )
        .expect("observation evidence");
        assert!(evidence.contains("\"readiness\":{\"7\":\"candidate\"}"));
    }

    #[test]
    fn readiness_tampering_is_rejected_during_tier15_replay() {
        let root = TempRoot::new();
        seed_tier_one_point_five_cursor(&root);
        assert_eq!(
            record_tier15(root.path(), REPO, produced_scan()).expect("record produced scan"),
            Tier15Progress::Produced(1)
        );
        let store = store(&root);
        let receipt = store
            .load_receipt(1, NoWorkTier::Tier1_5)
            .expect("receipt")
            .expect("sealed receipt");
        let path = root
            .path()
            .join("waterfall")
            .join(&receipt.evidence()[0].reference);
        let evidence = fs::read_to_string(&path).expect("observation evidence");
        fs::write(&path, evidence.replace("\"candidate\"", "\"forged\""))
            .expect("tamper readiness state");
        assert!(store.verify_tier15_evidence(1, &receipt).is_err());
    }

    #[test]
    fn exhausted_receipt_advances_to_tier_two_and_recovers_before_cursor_write() {
        let root = TempRoot::new();
        seed_tier_one_point_five_cursor(&root);

        assert_eq!(
            record_tier15(root.path(), REPO, exhausted_scan()).expect("record exhausted scan"),
            Tier15Progress::Advanced
        );
        let store = store(&root);
        assert_eq!(
            store
                .load_state()
                .expect("state")
                .expect("cursor")
                .current_tier(),
            NoWorkTier::Tier2
        );
        let receipt = store
            .load_receipt(1, NoWorkTier::Tier1_5)
            .expect("receipt")
            .expect("sealed receipt");
        assert!(matches!(receipt.status(), TierStatus::Exhausted { .. }));
        assert_eq!(receipt.reference(), "waterfall/1/tier1_5.json");
    }

    #[test]
    fn sealed_exhausted_receipt_replays_after_a_pre_cursor_restart() {
        let root = TempRoot::new();
        seed_tier_one_point_five_cursor(&root);
        let Tier15Scan::Complete(observation) = exhausted_scan() else {
            panic!("empty snapshots are complete observations");
        };
        let waterfall_store = store(&root);
        let state = waterfall_store
            .load_state()
            .expect("state")
            .expect("Tier 1.5 cursor");
        let (status, funnel, artifact, contents) =
            super::observation_receipt_parts(&observation).expect("receipt parts");
        let evidence = waterfall_store
            .persist_tier15_evidence(state.next_pass_id(), artifact, &contents)
            .expect("sealed evidence before cursor");
        let receipt = super::receipt(&state, status, funnel, evidence).expect("sealed receipt");
        waterfall_store
            .persist_receipt(&receipt)
            .expect("receipt before cursor");
        drop(waterfall_store);

        assert_eq!(
            record_tier15(
                root.path(),
                REPO,
                Tier15Scan::Failed("must replay existing evidence".to_string())
            )
            .expect("recover sealed receipt"),
            Tier15Progress::Advanced
        );
        assert_eq!(
            store(&root)
                .load_state()
                .expect("state")
                .expect("advanced cursor")
                .current_tier(),
            NoWorkTier::Tier2
        );
    }

    #[test]
    fn failed_receipt_replays_its_sealed_reason_and_rejects_tampered_evidence() {
        let root = TempRoot::new();
        seed_tier_one_point_five_cursor(&root);

        assert_eq!(
            record_tier15(
                root.path(),
                REPO,
                Tier15Scan::Failed("later closed page failed".to_string())
            )
            .expect("record failed scan"),
            Tier15Progress::Failed("later closed page failed".to_string())
        );
        assert_eq!(
            record_tier15(root.path(), REPO, produced_scan()).expect("replay failed receipt"),
            Tier15Progress::Failed("later closed page failed".to_string())
        );
        let store = store(&root);
        assert_eq!(
            store
                .load_state()
                .expect("state")
                .expect("cursor")
                .current_tier(),
            NoWorkTier::Tier1_5
        );
        let receipt = store
            .load_receipt(1, NoWorkTier::Tier1_5)
            .expect("receipt")
            .expect("sealed receipt");
        assert!(matches!(receipt.status(), TierStatus::Failed { .. }));
        store
            .verify_tier15_evidence(1, &receipt)
            .expect("failure evidence");
        drop(store);

        fs::write(
            root.path()
                .join("waterfall/waterfall/1/tier1_5/read-failure.json"),
            "{\"schema\":1,\"kind\":\"tampered\"}\n",
        )
        .expect("tamper failure evidence");
        assert!(
            record_tier15(root.path(), REPO, produced_scan()).is_err(),
            "failed replay must reject tampered read-failure evidence"
        );
    }

    #[test]
    fn receipt_coordinator_has_no_promotion_or_write_authority() {
        let source = include_str!("tier15_receipts.rs")
            .split("\n#[cfg(test)]")
            .next()
            .expect("production source before module tests");
        assert!(
            source.contains("fn replay_tier15_fenced"),
            "authority scan must include the replay preflight"
        );
        let forbidden = [
            "queue",
            "claim",
            "legacy",
            "promote-eligibility",
            "promoter",
            "classifier",
            "std::process",
            "Command",
            "run_foreground",
            "why-no-work",
            "gh issue",
            "issue edit",
            "issue comment",
            "label",
            "body=",
            "comments",
            "POST",
            "PATCH",
            "PUT",
            "DELETE",
            "graphql",
        ];
        for authority in forbidden {
            assert!(
                !source.contains(authority),
                "Tier 1.5 receipt coordinator must not own {authority} authority"
            );
        }
    }
}
