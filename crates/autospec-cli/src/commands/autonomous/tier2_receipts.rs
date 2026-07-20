use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use autospec_core::autonomous::no_work::{DryReason, NoWorkTier};
use autospec_core::autonomous::tier2::{
    Tier2EvidenceDocuments, Tier2Failure, Tier2Observation, DISABLED_REASON,
};
use autospec_core::autonomous::waterfall::{
    FunnelCounts, SealedEvidence, TierReceipt, TierStatus, WaterfallState,
};

use super::resilience::{with_current_lifecycle_lease, ConductorLease};
use super::tier15_receipts::ReceiptPreflight;
use super::tier2::Tier2Scan;
use super::waterfall::{
    StoreAcquisition, Tier2EvidenceArtifact, WaterfallStore, WaterfallStoreError,
};

const DISABLED_PRODUCER_VERSION: &str = "rust-tier2-disabled-policy-v1";
const PRODUCER_VERSION: &str = "rust-tier2-local-receipts-v1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Tier2Progress {
    Pending,
    Advanced,
    Produced(u64),
    Failed(String),
    NotRun(String),
}

pub(super) fn replay_tier2_with_lease(
    state_root: &Path,
    repo: &str,
    lease: &ConductorLease,
) -> Result<ReceiptPreflight<Tier2Progress>, String> {
    with_current_lifecycle_lease(lease, || replay_tier2_fenced(state_root, repo))
}

pub(super) fn record_tier2_with_lease(
    state_root: &Path,
    repo: &str,
    lease: &ConductorLease,
    scan: Tier2Scan,
) -> Result<Tier2Progress, String> {
    with_current_lifecycle_lease(lease, || record_tier2_fenced(state_root, repo, scan))
}

fn record_tier2_fenced(
    state_root: &Path,
    repo: &str,
    scan: Tier2Scan,
) -> Result<Tier2Progress, String> {
    let store = match WaterfallStore::acquire_for_receipts(state_root.join("waterfall"), repo, None)
        .map_err(store_error)?
    {
        StoreAcquisition::Acquired(store) => store,
        StoreAcquisition::Held => return Ok(Tier2Progress::Pending),
    };
    let Some(state) = store.load_state().map_err(store_error)? else {
        return Ok(Tier2Progress::Pending);
    };
    if state.current_tier() != NoWorkTier::Tier2 {
        return Ok(Tier2Progress::Pending);
    }
    if let Some(receipt) = existing_receipt(&store, &state)? {
        return settle_receipt(&store, &state, receipt);
    }

    let receipt = match scan {
        Tier2Scan::NotRun => disabled_receipt(&store, &state)?,
        Tier2Scan::Complete(observation) => observation_receipt(&store, &state, &observation)?,
        Tier2Scan::Failed(failure) => failure_receipt(&store, &state, &failure)?,
    };
    store
        .verify_tier2_evidence(state.next_pass_id(), &receipt)
        .map_err(store_error)?;
    store.persist_receipt(&receipt).map_err(store_error)?;
    settle_receipt(&store, &state, receipt)
}

fn replay_tier2_fenced(
    state_root: &Path,
    repo: &str,
) -> Result<ReceiptPreflight<Tier2Progress>, String> {
    let store = match WaterfallStore::acquire(state_root.join("waterfall"), repo)
        .map_err(store_error)?
    {
        StoreAcquisition::Acquired(store) => store,
        StoreAcquisition::Held => return Ok(ReceiptPreflight::Replayed(Tier2Progress::Pending)),
    };
    let Some(state) = store.load_state().map_err(store_error)? else {
        return Ok(ReceiptPreflight::Replayed(Tier2Progress::Pending));
    };
    if state.current_tier() != NoWorkTier::Tier2 {
        return Ok(ReceiptPreflight::Replayed(Tier2Progress::Pending));
    }
    match existing_receipt(&store, &state)? {
        Some(receipt) => settle_receipt(&store, &state, receipt).map(ReceiptPreflight::Replayed),
        None => Ok(ReceiptPreflight::NeedsCollection),
    }
}

fn existing_receipt(
    store: &WaterfallStore,
    state: &WaterfallState,
) -> Result<Option<TierReceipt>, String> {
    let Some(receipt) = store
        .load_receipt(state.next_pass_id(), NoWorkTier::Tier2)
        .map_err(store_error)?
    else {
        return Ok(None);
    };
    store
        .verify_tier2_evidence(state.next_pass_id(), &receipt)
        .map_err(store_error)?;
    Ok(Some(receipt))
}

fn settle_receipt(
    store: &WaterfallStore,
    state: &WaterfallState,
    receipt: TierReceipt,
) -> Result<Tier2Progress, String> {
    match receipt.status() {
        TierStatus::Exhausted { .. } => {
            let advanced = state.clone().record_receipt(&receipt)?;
            store.persist_state(&advanced).map_err(store_error)?;
            Ok(Tier2Progress::Advanced)
        }
        TierStatus::Produced { count } => Ok(Tier2Progress::Produced(*count)),
        TierStatus::Failed { reason } => Ok(Tier2Progress::Failed(reason.clone())),
        TierStatus::NotRun { reason } => Ok(Tier2Progress::NotRun(reason.clone())),
        status => Err(format!(
            "Tier 2 receipt has unexpected {} status",
            status.as_str()
        )),
    }
}

fn disabled_receipt(store: &WaterfallStore, state: &WaterfallState) -> Result<TierReceipt, String> {
    let evidence = store
        .persist_tier2_evidence(
            state.next_pass_id(),
            Tier2EvidenceArtifact::Policy,
            &disabled_policy_document(),
        )
        .map_err(store_error)?;
    receipt(
        state,
        DISABLED_PRODUCER_VERSION,
        TierStatus::NotRun {
            reason: DISABLED_REASON.to_string(),
        },
        FunnelCounts::new(0, 0, 0, 0, 0)?,
        vec![evidence],
    )
}

fn observation_receipt(
    store: &WaterfallStore,
    state: &WaterfallState,
    observation: &Tier2Observation,
) -> Result<TierReceipt, String> {
    let status = observation_status(observation);
    let evidence = persist_documents(store, state.next_pass_id(), observation.documents())?;
    receipt(
        state,
        PRODUCER_VERSION,
        status,
        observation.funnel().clone(),
        evidence,
    )
}

fn failure_receipt(
    store: &WaterfallStore,
    state: &WaterfallState,
    failure: &Tier2Failure,
) -> Result<TierReceipt, String> {
    let documents = failure
        .documents()
        .ok_or_else(|| "Tier 2 failure was not sealed by evaluation".to_string())?;
    let evidence = persist_documents(store, state.next_pass_id(), documents)?;
    receipt(
        state,
        PRODUCER_VERSION,
        TierStatus::Failed {
            reason: failure.status_reason(),
        },
        failure.partial_evidence().funnel().clone(),
        evidence,
    )
}

fn persist_documents(
    store: &WaterfallStore,
    pass_id: u64,
    documents: Tier2EvidenceDocuments<'_>,
) -> Result<Vec<SealedEvidence>, String> {
    let mut evidence = Vec::new();
    if let Some(document) = documents.collector_json() {
        evidence.push(persist_document(
            store,
            pass_id,
            Tier2EvidenceArtifact::Collector,
            document,
        )?);
    }
    persist_generated(store, pass_id, &documents, &mut evidence)?;
    persist_deduplication(store, pass_id, &documents, &mut evidence)?;
    persist_verification(store, pass_id, &documents, &mut evidence)?;
    persist_roi_rank(store, pass_id, &documents, &mut evidence)?;
    if let Some(document) = documents
        .failure_json(evidence.last().map(|item| item.digest.as_str()))
        .map_err(|error| format!("cannot render Tier 2 failure evidence: {error}"))?
    {
        evidence.push(persist_document(
            store,
            pass_id,
            Tier2EvidenceArtifact::Failure,
            document,
        )?);
    }
    Ok(evidence)
}

fn persist_generated(
    store: &WaterfallStore,
    pass_id: u64,
    documents: &Tier2EvidenceDocuments<'_>,
    evidence: &mut Vec<SealedEvidence>,
) -> Result<(), String> {
    let Some(predecessor) = evidence.last().map(|item| item.digest.as_str()) else {
        return Ok(());
    };
    if let Some(document) = documents
        .generated_json(predecessor)
        .map_err(|error| format!("cannot render Tier 2 evidence: {error}"))?
    {
        evidence.push(persist_document(
            store,
            pass_id,
            Tier2EvidenceArtifact::Generated,
            document,
        )?);
    }
    Ok(())
}

fn persist_deduplication(
    store: &WaterfallStore,
    pass_id: u64,
    documents: &Tier2EvidenceDocuments<'_>,
    evidence: &mut Vec<SealedEvidence>,
) -> Result<(), String> {
    let Some(predecessor) = evidence.last().map(|item| item.digest.as_str()) else {
        return Ok(());
    };
    if let Some(document) = documents
        .deduplication_json(predecessor)
        .map_err(|error| format!("cannot render Tier 2 evidence: {error}"))?
    {
        evidence.push(persist_document(
            store,
            pass_id,
            Tier2EvidenceArtifact::Dedup,
            document,
        )?);
    }
    Ok(())
}

fn persist_verification(
    store: &WaterfallStore,
    pass_id: u64,
    documents: &Tier2EvidenceDocuments<'_>,
    evidence: &mut Vec<SealedEvidence>,
) -> Result<(), String> {
    let Some(predecessor) = evidence.last().map(|item| item.digest.as_str()) else {
        return Ok(());
    };
    if let Some(document) = documents
        .verification_json(predecessor)
        .map_err(|error| format!("cannot render Tier 2 evidence: {error}"))?
    {
        evidence.push(persist_document(
            store,
            pass_id,
            Tier2EvidenceArtifact::Verification,
            document,
        )?);
    }
    Ok(())
}

fn persist_roi_rank(
    store: &WaterfallStore,
    pass_id: u64,
    documents: &Tier2EvidenceDocuments<'_>,
    evidence: &mut Vec<SealedEvidence>,
) -> Result<(), String> {
    let Some(predecessor) = evidence.last().map(|item| item.digest.as_str()) else {
        return Ok(());
    };
    if let Some(document) = documents
        .roi_rank_json(predecessor)
        .map_err(|error| format!("cannot render Tier 2 evidence: {error}"))?
    {
        evidence.push(persist_document(
            store,
            pass_id,
            Tier2EvidenceArtifact::RoiRank,
            document,
        )?);
    }
    Ok(())
}

fn persist_document(
    store: &WaterfallStore,
    pass_id: u64,
    artifact: Tier2EvidenceArtifact,
    document: String,
) -> Result<SealedEvidence, String> {
    store
        .persist_tier2_evidence(pass_id, artifact, &document)
        .map_err(store_error)
}

fn observation_status(observation: &Tier2Observation) -> TierStatus {
    let funnel = observation.funnel();
    if funnel.ranked > 0 {
        TierStatus::Produced {
            count: funnel.ranked,
        }
    } else if funnel.observed == 0 {
        TierStatus::Exhausted {
            reason: DryReason::NoProposalsGenerated,
        }
    } else if funnel.verified == 0 {
        TierStatus::Exhausted {
            reason: DryReason::VerificationRejected,
        }
    } else {
        TierStatus::Exhausted {
            reason: DryReason::RoiFiltered,
        }
    }
}

fn receipt(
    state: &WaterfallState,
    producer_version: &str,
    status: TierStatus,
    funnel: FunnelCounts,
    evidence: Vec<SealedEvidence>,
) -> Result<TierReceipt, String> {
    let now = now_secs();
    TierReceipt::new(
        state.repo(),
        state.next_pass_id(),
        NoWorkTier::Tier2,
        producer_version,
        now,
        now,
        status,
        funnel,
        evidence,
    )
}

fn disabled_policy_document() -> String {
    format!(
        "{{\"schema\":1,\"kind\":\"tier2_policy\",\"mode\":\"disabled\",\"reason\":\"{DISABLED_REASON}\",\"policy_source\":\"checked_in\"}}\n"
    )
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
pub(super) fn record_tier2(
    state_root: &Path,
    repo: &str,
    scan: Tier2Scan,
) -> Result<Tier2Progress, String> {
    record_tier2_fenced(state_root, repo, scan)
}

#[cfg(test)]
mod tests {
    #[test]
    fn receipt_coordinator_keeps_only_local_persistence_authority() {
        let source = include_str!("tier2_receipts.rs");
        let production = source
            .split("\n#[cfg(test)]")
            .next()
            .expect("production source before module tests");
        assert!(
            production.contains("WaterfallStore"),
            "Tier 2 coordinator owns the local waterfall persistence boundary"
        );
        assert!(
            production.contains("fn record_tier2_fenced"),
            "authority scan must include the Tier 2 fenced recorder"
        );
        let gh_cli = ["\"", "g", "h "].concat();
        for forbidden in [
            "std::env",
            "std::fs",
            "fs::",
            "std::io",
            "io::",
            "OpenOptions",
            "File::",
            "std::process",
            "std::net",
            "Command",
            "AUTOSPEC_SPECIALIST_LLM_STUB_OUTPUT",
            "scan_specialists",
            "load_or_derive",
            "autospec-explore",
            "bash",
            "zsh",
            "sh -c",
            "omx",
            "curl",
            gh_cli.as_str(),
            "github",
            "queue",
            "claim",
            "label",
            "branch",
            "worktree",
            "pull_request",
            "pull request",
            "pr create",
            "issue create",
            "issue edit",
            "issue comment",
            "auto-implement",
            "run_foreground",
            "scan_foreground",
            "ConductorEvent",
            "legacy",
            "dispatch",
            "ExecutorRequest",
            "\"POST\"",
            "\"PATCH\"",
            "\"PUT\"",
            "\"DELETE\"",
            "graphql",
            "pr edit",
            "pr comment",
            "pr merge",
        ] {
            assert!(
                !production.contains(forbidden),
                "Tier 2 receipt coordinator retains prohibited authority: {forbidden}"
            );
        }
    }
}
