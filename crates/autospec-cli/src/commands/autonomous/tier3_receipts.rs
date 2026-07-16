use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use autospec_core::autonomous::no_work::{DryReason, NoWorkTier};
use autospec_core::autonomous::tier3::{
    Tier3EvidenceDocuments, Tier3Failure, Tier3Observation, DISABLED_REASON,
};
use autospec_core::autonomous::waterfall::{
    FunnelCounts, SealedEvidence, TierReceipt, TierStatus, WaterfallState,
};

use super::tier3::Tier3Scan;
use super::waterfall::{
    StoreAcquisition, Tier3EvidenceArtifact, WaterfallStore, WaterfallStoreError,
};

const DISABLED_PRODUCER_VERSION: &str = "rust-tier3-disabled-policy-v1";
const PRODUCER_VERSION: &str = "rust-tier3-metadata-receipts-v1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Tier3Progress {
    Pending,
    Advanced,
    Produced(u64),
    Failed(String),
    NotRun(String),
}

pub(super) fn record_tier3(
    state_root: &Path,
    repo: &str,
    scan: Tier3Scan,
) -> Result<Tier3Progress, String> {
    let store =
        match WaterfallStore::acquire(state_root.join("waterfall"), repo).map_err(store_error)? {
            StoreAcquisition::Acquired(store) => store,
            StoreAcquisition::Held => return Ok(Tier3Progress::Pending),
        };
    let Some(state) = store.load_state().map_err(store_error)? else {
        return Ok(Tier3Progress::Pending);
    };
    if state.current_tier() != NoWorkTier::Tier3 {
        return Ok(Tier3Progress::Pending);
    }
    if let Some(receipt) = existing_receipt(&store, &state)? {
        return settle_receipt(&store, &state, receipt);
    }
    let receipt = match scan {
        Tier3Scan::NotRun => disabled_receipt(&store, &state)?,
        Tier3Scan::Complete(observation) => observation_receipt(&store, &state, &observation)?,
        Tier3Scan::Failed(failure) => failure_receipt(&store, &state, &failure)?,
    };
    store
        .verify_tier3_evidence(state.next_pass_id(), &receipt)
        .map_err(store_error)?;
    store.persist_receipt(&receipt).map_err(store_error)?;
    settle_receipt(&store, &state, receipt)
}

fn existing_receipt(
    store: &WaterfallStore,
    state: &WaterfallState,
) -> Result<Option<TierReceipt>, String> {
    let Some(receipt) = store
        .load_receipt(state.next_pass_id(), NoWorkTier::Tier3)
        .map_err(store_error)?
    else {
        return Ok(None);
    };
    store
        .verify_tier3_evidence(state.next_pass_id(), &receipt)
        .map_err(store_error)?;
    Ok(Some(receipt))
}

fn settle_receipt(
    store: &WaterfallStore,
    state: &WaterfallState,
    receipt: TierReceipt,
) -> Result<Tier3Progress, String> {
    match receipt.status() {
        TierStatus::Exhausted {
            reason: DryReason::NoMetadataFindings,
        } => {
            let advanced = state.clone().record_receipt(&receipt)?;
            store.persist_state(&advanced).map_err(store_error)?;
            Ok(Tier3Progress::Advanced)
        }
        TierStatus::Produced { count } => Ok(Tier3Progress::Produced(*count)),
        TierStatus::Failed { reason } => Ok(Tier3Progress::Failed(reason.clone())),
        TierStatus::NotRun { reason } => Ok(Tier3Progress::NotRun(reason.clone())),
        status => Err(format!(
            "Tier 3 receipt has unexpected {} status",
            status.as_str()
        )),
    }
}

fn disabled_receipt(store: &WaterfallStore, state: &WaterfallState) -> Result<TierReceipt, String> {
    let evidence = persist_document(
        store,
        state.next_pass_id(),
        Tier3EvidenceArtifact::Policy,
        &disabled_policy_document(),
    )?;
    receipt(
        state,
        DISABLED_PRODUCER_VERSION,
        TierStatus::NotRun {
            reason: DISABLED_REASON.to_string(),
        },
        zero_funnel(),
        vec![evidence],
    )
}

fn observation_receipt(
    store: &WaterfallStore,
    state: &WaterfallState,
    observation: &Tier3Observation,
) -> Result<TierReceipt, String> {
    let evidence = persist_documents(store, state.next_pass_id(), observation.documents())?;
    let status = if observation.funnel().ranked == 0 {
        TierStatus::Exhausted {
            reason: DryReason::NoMetadataFindings,
        }
    } else {
        TierStatus::Produced {
            count: observation.funnel().ranked,
        }
    };
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
    failure: &Tier3Failure,
) -> Result<TierReceipt, String> {
    let documents = failure
        .documents()
        .ok_or_else(|| "Tier 3 failure was not sealed by evaluation".to_string())?;
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
    documents: Tier3EvidenceDocuments<'_>,
) -> Result<Vec<SealedEvidence>, String> {
    let mut evidence = Vec::new();
    if let Some(document) = documents.architecture_json() {
        evidence.push(persist_document(
            store,
            pass_id,
            Tier3EvidenceArtifact::Architecture,
            &document,
        )?);
    }
    if let Some(predecessor) = evidence.last().map(|item| item.digest.as_str()) {
        if let Some(document) = documents.coverage_json(predecessor).map_err(render_error)? {
            evidence.push(persist_document(
                store,
                pass_id,
                Tier3EvidenceArtifact::Coverage,
                &document,
            )?);
        }
    }
    if let Some(predecessor) = evidence.last().map(|item| item.digest.as_str()) {
        if let Some(document) = documents.debt_json(predecessor).map_err(render_error)? {
            evidence.push(persist_document(
                store,
                pass_id,
                Tier3EvidenceArtifact::Debt,
                &document,
            )?);
        }
    }
    if let Some(predecessor) = evidence.last().map(|item| item.digest.as_str()) {
        if let Some(document) = documents.findings_json(predecessor).map_err(render_error)? {
            evidence.push(persist_document(
                store,
                pass_id,
                Tier3EvidenceArtifact::Findings,
                &document,
            )?);
        }
    }
    let predecessor = evidence.last().map(|item| item.digest.as_str());
    if let Some(document) = documents.failure_json(predecessor).map_err(render_error)? {
        evidence.push(persist_document(
            store,
            pass_id,
            Tier3EvidenceArtifact::Failure,
            &document,
        )?);
    }
    Ok(evidence)
}

fn persist_document(
    store: &WaterfallStore,
    pass_id: u64,
    artifact: Tier3EvidenceArtifact,
    document: &str,
) -> Result<SealedEvidence, String> {
    store
        .persist_tier3_evidence(pass_id, artifact, document)
        .map_err(store_error)
}

fn receipt(
    state: &WaterfallState,
    producer: &str,
    status: TierStatus,
    funnel: FunnelCounts,
    evidence: Vec<SealedEvidence>,
) -> Result<TierReceipt, String> {
    let now = now_secs();
    TierReceipt::new(
        state.repo(),
        state.next_pass_id(),
        NoWorkTier::Tier3,
        producer,
        now,
        now,
        status,
        funnel,
        evidence,
    )
}

fn disabled_policy_document() -> String {
    format!("{{\"schema\":1,\"kind\":\"tier3_policy\",\"mode\":\"disabled\",\"reason\":\"{DISABLED_REASON}\",\"policy_source\":\"checked_in\"}}\n")
}
fn zero_funnel() -> FunnelCounts {
    FunnelCounts::new(0, 0, 0, 0, 0).expect("zero Tier 3 funnel is valid")
}
fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn render_error(error: String) -> String {
    format!("cannot render Tier 3 evidence: {error}")
}
fn store_error(error: WaterfallStoreError) -> String {
    match error {
        WaterfallStoreError::Diagnostic(reason)
        | WaterfallStoreError::InvalidReceipt(reason)
        | WaterfallStoreError::InvalidState(reason) => reason,
    }
}
