use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use autospec_core::autonomous::no_work::{DryReason, NoWorkTier};
use autospec_core::autonomous::tier4::{
    Tier4EvidenceDocuments, Tier4Failure, Tier4Observation, DISABLED_REASON,
};
use autospec_core::autonomous::waterfall::{
    FunnelCounts, SealedEvidence, TierReceipt, TierStatus, WaterfallState,
};

use super::tier4::Tier4Scan;
use super::waterfall::{
    StoreAcquisition, Tier4EvidenceArtifact, WaterfallStore, WaterfallStoreError,
};

const DISABLED_PRODUCER_VERSION: &str = "rust-tier4-disabled-policy-v1";
const PRODUCER_VERSION: &str = "rust-tier4-external-discovery-receipts-v1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Tier4Progress {
    Pending,
    Advanced,
    Produced(u64),
    Failed(String),
    NotRun(String),
}

pub(super) fn record_tier4(
    state_root: &Path,
    repo: &str,
    scan: Tier4Scan,
) -> Result<Tier4Progress, String> {
    let store =
        match WaterfallStore::acquire(state_root.join("waterfall"), repo).map_err(store_error)? {
            StoreAcquisition::Acquired(store) => store,
            StoreAcquisition::Held => return Ok(Tier4Progress::Pending),
        };
    let Some(state) = store.load_state().map_err(store_error)? else {
        return Ok(Tier4Progress::Pending);
    };
    if state.current_tier() != NoWorkTier::Tier4 {
        return Ok(Tier4Progress::Pending);
    }
    if let Some(receipt) = existing_receipt(&store, &state)? {
        return settle_receipt(&store, &state, receipt);
    }
    let receipt = match scan {
        Tier4Scan::NotRun => disabled_receipt(&store, &state)?,
        Tier4Scan::Complete(observation) => observation_receipt(&store, &state, &observation)?,
        Tier4Scan::Failed(failure) => failure_receipt(&store, &state, &failure)?,
    };
    store
        .verify_tier4_evidence(state.next_pass_id(), &receipt)
        .map_err(store_error)?;
    store.persist_receipt(&receipt).map_err(store_error)?;
    settle_receipt(&store, &state, receipt)
}

fn existing_receipt(
    store: &WaterfallStore,
    state: &WaterfallState,
) -> Result<Option<TierReceipt>, String> {
    let Some(receipt) = store
        .load_receipt(state.next_pass_id(), NoWorkTier::Tier4)
        .map_err(store_error)?
    else {
        return Ok(None);
    };
    store
        .verify_tier4_evidence(state.next_pass_id(), &receipt)
        .map_err(store_error)?;
    Ok(Some(receipt))
}

fn settle_receipt(
    store: &WaterfallStore,
    state: &WaterfallState,
    receipt: TierReceipt,
) -> Result<Tier4Progress, String> {
    match receipt.status() {
        TierStatus::Produced { count } => Ok(Tier4Progress::Produced(*count)),
        TierStatus::Failed { reason } => Ok(Tier4Progress::Failed(reason.clone())),
        TierStatus::NotRun { reason } => Ok(Tier4Progress::NotRun(reason.clone())),
        TierStatus::Exhausted {
            reason:
                DryReason::NoProposalsGenerated
                | DryReason::VerificationRejected
                | DryReason::RoiFiltered,
        } => {
            let advanced = state.clone().record_receipt(&receipt)?;
            store.persist_state(&advanced).map_err(store_error)?;
            Ok(Tier4Progress::Advanced)
        }
        TierStatus::Exhausted { reason } => Err(format!(
            "Tier 4 receipt has non-advancing exhausted reason: {}",
            reason.as_str()
        )),
        TierStatus::Blocked { reason } => Err(format!("Tier 4 receipt is blocked: {reason}")),
    }
}

fn observation_receipt(
    store: &WaterfallStore,
    state: &WaterfallState,
    observation: &Tier4Observation,
) -> Result<TierReceipt, String> {
    let status = match observation.terminal_dry_reason() {
        Some(reason) => TierStatus::Exhausted { reason },
        None => TierStatus::Produced {
            count: observation.funnel().ranked,
        },
    };
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
    failure: &Tier4Failure,
) -> Result<TierReceipt, String> {
    let documents = failure
        .documents()
        .ok_or_else(|| "Tier 4 failure was not sealed by evaluation".to_string())?;
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
    documents: Tier4EvidenceDocuments<'_>,
) -> Result<Vec<SealedEvidence>, String> {
    let mut evidence = Vec::new();
    if let Some(document) = documents.source_policy_json() {
        evidence.push(persist_document(
            store,
            pass_id,
            Tier4EvidenceArtifact::SourcePolicy,
            document,
        )?);
    }
    persist_sources(store, pass_id, &documents, &mut evidence)?;
    persist_generated(store, pass_id, &documents, &mut evidence)?;
    persist_dedup(store, pass_id, &documents, &mut evidence)?;
    persist_verification(store, pass_id, &documents, &mut evidence)?;
    persist_roi_rank(store, pass_id, &documents, &mut evidence)?;
    match documents.failure_json(evidence.last().map(|item| item.digest.as_str())) {
        Ok(document) => evidence.push(persist_document(
            store,
            pass_id,
            Tier4EvidenceArtifact::Failure,
            document,
        )?),
        Err(error) if error == "complete Tier 4 evidence does not render a failure document" => {}
        Err(error) => return Err(format!("cannot render Tier 4 failure evidence: {error}")),
    }
    Ok(evidence)
}

fn persist_sources(
    store: &WaterfallStore,
    pass_id: u64,
    documents: &Tier4EvidenceDocuments<'_>,
    evidence: &mut Vec<SealedEvidence>,
) -> Result<(), String> {
    persist_chained(
        store,
        pass_id,
        documents.sources_json(
            evidence
                .last()
                .map(|item| item.digest.as_str())
                .unwrap_or_default(),
        ),
        Tier4EvidenceArtifact::Sources,
        evidence,
    )
}

fn persist_generated(
    store: &WaterfallStore,
    pass_id: u64,
    documents: &Tier4EvidenceDocuments<'_>,
    evidence: &mut Vec<SealedEvidence>,
) -> Result<(), String> {
    persist_chained(
        store,
        pass_id,
        documents.generated_json(
            evidence
                .last()
                .map(|item| item.digest.as_str())
                .unwrap_or_default(),
        ),
        Tier4EvidenceArtifact::Generated,
        evidence,
    )
}

fn persist_dedup(
    store: &WaterfallStore,
    pass_id: u64,
    documents: &Tier4EvidenceDocuments<'_>,
    evidence: &mut Vec<SealedEvidence>,
) -> Result<(), String> {
    persist_chained(
        store,
        pass_id,
        documents.dedup_json(
            evidence
                .last()
                .map(|item| item.digest.as_str())
                .unwrap_or_default(),
        ),
        Tier4EvidenceArtifact::Dedup,
        evidence,
    )
}

fn persist_verification(
    store: &WaterfallStore,
    pass_id: u64,
    documents: &Tier4EvidenceDocuments<'_>,
    evidence: &mut Vec<SealedEvidence>,
) -> Result<(), String> {
    persist_chained(
        store,
        pass_id,
        documents.verification_json(
            evidence
                .last()
                .map(|item| item.digest.as_str())
                .unwrap_or_default(),
        ),
        Tier4EvidenceArtifact::Verification,
        evidence,
    )
}

fn persist_roi_rank(
    store: &WaterfallStore,
    pass_id: u64,
    documents: &Tier4EvidenceDocuments<'_>,
    evidence: &mut Vec<SealedEvidence>,
) -> Result<(), String> {
    persist_chained(
        store,
        pass_id,
        documents.roi_rank_json(
            evidence
                .last()
                .map(|item| item.digest.as_str())
                .unwrap_or_default(),
        ),
        Tier4EvidenceArtifact::RoiRank,
        evidence,
    )
}

fn persist_chained(
    store: &WaterfallStore,
    pass_id: u64,
    document: Result<Option<String>, String>,
    artifact: Tier4EvidenceArtifact,
    evidence: &mut Vec<SealedEvidence>,
) -> Result<(), String> {
    if let Some(document) =
        document.map_err(|error| format!("cannot render Tier 4 evidence: {error}"))?
    {
        evidence.push(persist_document(store, pass_id, artifact, document)?);
    }
    Ok(())
}

fn persist_document(
    store: &WaterfallStore,
    pass_id: u64,
    artifact: Tier4EvidenceArtifact,
    document: String,
) -> Result<SealedEvidence, String> {
    store
        .persist_tier4_evidence(pass_id, artifact, &document)
        .map_err(store_error)
}

fn disabled_receipt(store: &WaterfallStore, state: &WaterfallState) -> Result<TierReceipt, String> {
    let evidence = store
        .persist_tier4_evidence(
            state.next_pass_id(),
            Tier4EvidenceArtifact::Policy,
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

fn receipt(
    state: &WaterfallState,
    producer: &str,
    status: TierStatus,
    funnel: FunnelCounts,
    evidence: Vec<autospec_core::autonomous::waterfall::SealedEvidence>,
) -> Result<TierReceipt, String> {
    let now = now_secs();
    TierReceipt::new(
        state.repo(),
        state.next_pass_id(),
        NoWorkTier::Tier4,
        producer,
        now,
        now,
        status,
        funnel,
        evidence,
    )
}

fn disabled_policy_document() -> String {
    format!(
        "{{\"schema\":1,\"kind\":\"tier4_policy\",\"mode\":\"disabled\",\"reason\":\"{DISABLED_REASON}\",\"policy_source\":\"checked_in\"}}\n"
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
mod tests {
    #[test]
    fn receipt_coordinator_keeps_only_local_receipt_authority() {
        let source = include_str!("tier4_receipts.rs");
        let production = source
            .split("\n#[cfg(test)]")
            .next()
            .expect("production source");
        assert!(production.contains("WaterfallStore"));
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
            "run_foreground",
            "scan_foreground",
            "dispatch",
            "ExecutorRequest",
            "Tier4Input",
        ] {
            assert!(
                !production.contains(forbidden),
                "Tier 4 receipt coordinator retains prohibited authority: {forbidden}"
            );
        }
    }
}
