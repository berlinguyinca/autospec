use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use autospec_core::autonomous::no_work::{DryReason, NoWorkTier};
use autospec_core::autonomous::waterfall::{
    FunnelCounts, SealedEvidence, TierReceipt, TierStatus, WaterfallState,
};
use autospec_core::coordination::ReadyQueuePlan;

use super::resilience::ConductorLease;
use super::waterfall::{
    StoreAcquisition, Tier1EvidenceArtifact, WaterfallStore, WaterfallStoreError,
};

const PRODUCER_VERSION: &str = "rust-foreground-tier1-v1";

pub(super) enum Tier1QueueEvidence<'a> {
    EmptyPage(&'a ReadyQueuePlan),
    Failed(&'a str),
}

pub(super) enum Tier1Progress {
    Pending,
    Advanced,
    Failed(String),
}

pub(super) fn should_start_tier_one(plan: &ReadyQueuePlan) -> bool {
    plan.gate_counts.candidate == 0
        && plan.gate_counts.claimed == 0
        && plan.claimed.is_empty()
        && !plan.worker_cap.reached
        && plan.batch.is_empty()
}

pub(super) fn record_tier_one(
    state_root: &Path,
    repo: &str,
    _lease: &ConductorLease,
    evidence: Tier1QueueEvidence<'_>,
) -> Result<Tier1Progress, String> {
    let root = state_root.join("waterfall");
    let store = match WaterfallStore::acquire(&root, repo).map_err(store_error)? {
        StoreAcquisition::Acquired(store) => store,
        StoreAcquisition::Held => return Ok(Tier1Progress::Pending),
    };
    let state = store
        .load_state()
        .map_err(store_error)?
        .unwrap_or(WaterfallState::new(repo, 1, NoWorkTier::Tier1)?);
    let existing = existing_tier1_receipt(&store, &state)?;
    if state.current_tier() != NoWorkTier::Tier1 {
        if existing.is_none() {
            return Err("waterfall cursor is missing its sealed Tier 1 receipt".to_string());
        }
        return Ok(Tier1Progress::Pending);
    }
    if let Some(receipt) = existing {
        return settle_tier1_receipt(&store, &state, receipt);
    }

    match evidence {
        Tier1QueueEvidence::EmptyPage(plan) => {
            if !should_start_tier_one(plan) {
                return Ok(Tier1Progress::Pending);
            }
            let evidence = store
                .persist_tier1_evidence(
                    state.next_pass_id(),
                    Tier1EvidenceArtifact::ReadyPage,
                    &ready_page_document(plan),
                )
                .map_err(store_error)?;
            let receipt = receipt_for_empty_page(&state, plan, evidence)?;
            store.persist_receipt(&receipt).map_err(store_error)?;
            settle_tier1_receipt(&store, &state, receipt)
        }
        Tier1QueueEvidence::Failed(reason) => {
            let reason = format!("ready_queue_read_failed: {reason}");
            let evidence = store
                .persist_tier1_evidence(
                    state.next_pass_id(),
                    Tier1EvidenceArtifact::ReadFailure,
                    &queue_failure_document(&reason),
                )
                .map_err(store_error)?;
            let receipt = receipt_for_failed_queue(&state, reason, evidence)?;
            store.persist_receipt(&receipt).map_err(store_error)?;
            settle_tier1_receipt(&store, &state, receipt)
        }
    }
}

fn existing_tier1_receipt(
    store: &WaterfallStore,
    state: &WaterfallState,
) -> Result<Option<TierReceipt>, String> {
    let Some(receipt) = store
        .load_receipt(state.next_pass_id(), NoWorkTier::Tier1)
        .map_err(store_error)?
    else {
        return Ok(None);
    };
    let artifact = match receipt.status() {
        TierStatus::Exhausted { .. } => Tier1EvidenceArtifact::ReadyPage,
        TierStatus::Failed { .. } => Tier1EvidenceArtifact::ReadFailure,
        status => {
            return Err(format!(
                "Tier 1 receipt has unexpected {} status during recovery",
                status.as_str()
            ))
        }
    };
    store
        .verify_tier1_evidence(state.next_pass_id(), artifact, &receipt)
        .map_err(store_error)?;
    Ok(Some(receipt))
}

fn settle_tier1_receipt(
    store: &WaterfallStore,
    state: &WaterfallState,
    receipt: TierReceipt,
) -> Result<Tier1Progress, String> {
    match receipt.status() {
        TierStatus::Exhausted { .. } => {
            let advanced = state.clone().record_receipt(&receipt)?;
            store.persist_state(&advanced).map_err(store_error)?;
            Ok(Tier1Progress::Advanced)
        }
        TierStatus::Failed { reason } => Ok(Tier1Progress::Failed(reason.clone())),
        status => Err(format!(
            "Tier 1 receipt has unexpected {} status",
            status.as_str()
        )),
    }
}

fn receipt_for_empty_page(
    state: &WaterfallState,
    plan: &ReadyQueuePlan,
    evidence: SealedEvidence,
) -> Result<TierReceipt, String> {
    let counts = FunnelCounts::new(
        plan.gate_counts.open as u64,
        plan.gate_counts.candidate as u64,
        plan.gate_counts.reviewed as u64,
        plan.gate_counts.ready as u64,
        plan.gate_counts.selected as u64,
    )?;
    receipt(
        state,
        TierStatus::Exhausted {
            reason: DryReason::NoProposalsGenerated,
        },
        counts,
        evidence,
    )
}

fn receipt_for_failed_queue(
    state: &WaterfallState,
    reason: String,
    evidence: SealedEvidence,
) -> Result<TierReceipt, String> {
    receipt(
        state,
        TierStatus::Failed { reason },
        FunnelCounts::new(0, 0, 0, 0, 0)?,
        evidence,
    )
}

fn receipt(
    state: &WaterfallState,
    status: TierStatus,
    counts: FunnelCounts,
    evidence: SealedEvidence,
) -> Result<TierReceipt, String> {
    let now = now_secs();
    TierReceipt::new(
        state.repo(),
        state.next_pass_id(),
        NoWorkTier::Tier1,
        PRODUCER_VERSION,
        now,
        now,
        status,
        counts,
        vec![evidence],
    )
}

fn ready_page_document(plan: &ReadyQueuePlan) -> String {
    let gate_counts = format!(
        "\"open\":{},\"candidate\":{},\"reviewed\":{},\"blocked\":{},\"ready\":{},\"claimed\":{},\"selected\":{}",
        plan.gate_counts.open,
        plan.gate_counts.candidate,
        plan.gate_counts.reviewed,
        plan.gate_counts.blocked,
        plan.gate_counts.ready,
        plan.gate_counts.claimed,
        plan.gate_counts.selected,
    );
    let worker_cap = format!(
        "\"active_count\":{},\"remaining\":{},\"reached\":{}",
        plan.worker_cap.active_count, plan.worker_cap.remaining, plan.worker_cap.reached,
    );
    format!(
        "{{\"schema\":1,\"kind\":\"ready_page\",\"gate_counts\":{{{gate_counts}}},\"worker_cap\":{{{worker_cap}}}}}\n"
    )
}

fn queue_failure_document(reason: &str) -> String {
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
mod tests {
    use autospec_core::coordination::{QueueGateCounts, RemoteIssue, WorkerCap};

    use super::*;

    fn empty_plan() -> ReadyQueuePlan {
        ReadyQueuePlan {
            ready: Vec::new(),
            blocked: Vec::new(),
            claimed: Vec::new(),
            conflicts: Vec::new(),
            worker_cap: WorkerCap {
                max_repo_workers: 1,
                active_count: 0,
                remaining: 1,
                reached: false,
            },
            batch: Vec::new(),
            gate_counts: QueueGateCounts::default(),
        }
    }

    #[test]
    fn only_an_unclaimed_repository_empty_page_starts_tier_one() {
        let empty = empty_plan();
        assert!(should_start_tier_one(&empty));

        let mut active_claim = empty.clone();
        active_claim.gate_counts.claimed = 1;
        assert!(!should_start_tier_one(&active_claim));

        let mut listed_active_claim = empty.clone();
        listed_active_claim.claimed.push(RemoteIssue::open(
            42,
            "Active issue",
            "",
            vec!["auto-implement".to_string()],
            "autospec",
        ));
        assert!(!should_start_tier_one(&listed_active_claim));

        let mut open_but_ineligible = empty.clone();
        open_but_ineligible.gate_counts.open = 1;
        assert!(should_start_tier_one(&open_but_ineligible));

        let mut worker_capped = empty;
        worker_capped.worker_cap.reached = true;
        assert!(!should_start_tier_one(&worker_capped));
    }
}
