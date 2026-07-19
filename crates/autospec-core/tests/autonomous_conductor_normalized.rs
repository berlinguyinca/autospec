use autospec_core::autonomous::no_work::{
    DryReason, NoWorkObservation, NoWorkState, NoWorkTier, TierOutcome,
};
use autospec_core::autonomous::waterfall::{FunnelCounts, SealedEvidence, TierReceipt, TierStatus};
use autospec_core::coordination::{
    ConductorEvent, ConductorOutcome, ConductorScope, ConductorState,
};

#[test]
fn persisted_state_rejects_a_forged_normalized_state() {
    let encoded = selected_state()
        .to_json()
        .replace("\"state\":\"claim\"", "\"state\":\"idle-rescan\"");

    assert!(ConductorState::parse_json(&encoded).is_err());
}

#[test]
fn normalized_state_names_a_dry_tier_without_confusing_launch_dry_run() {
    let receipt = tier_receipt(
        7,
        NoWorkTier::Tier2,
        TierStatus::Exhausted {
            reason: DryReason::VerificationRejected,
        },
    );
    let normalized =
        conductor(ConductorScope::Repository).normalized_state_for_tier_receipt(7, &receipt);

    assert!(normalized.starts_with("cycle 7: tier-dry;"));
    assert!(normalized.contains("tier=tier2"));
    assert!(normalized.contains("reason=verification_rejected"));
    assert!(normalized.contains("affected issues=none"));
    assert!(normalized.contains("mutation_allowed=false"));
    assert!(normalized.contains("next=advance waterfall to tier3"));
    assert!(!normalized.contains("launch --dry-run"));
}

#[test]
fn normalized_state_names_an_all_blocked_cycle_with_affected_issues() {
    let state = selected_state()
        .transition(ConductorEvent::Claimed)
        .expect("claim is recorded")
        .transition(ConductorEvent::DispatchRecorded {
            outcome: ConductorOutcome::AllBlocked {
                reason: "tier1_all_blocked".to_string(),
                issues: vec![42, 43].into_boxed_slice(),
            },
        })
        .expect("all-blocked outcome is recorded");
    let encoded = state.to_json();
    let restored = ConductorState::parse_json(&encoded).expect("all-blocked state parses");
    let normalized = restored.normalized_state_for_cycle(8);

    assert!(encoded.contains("\"state\":\"all-blocked\""));
    assert!(normalized.starts_with("cycle 8: all-blocked;"));
    assert!(normalized.contains("reason=tier1_all_blocked"));
    assert!(normalized.contains("affected issues=#42,#43"));
    assert!(normalized.contains("mutation_allowed=false"));
    assert!(normalized.contains("next=promote or unblock affected issues"));
}

#[test]
fn normalized_state_names_verifier_unavailable_without_log_parsing() {
    let state = blocked_state(ConductorOutcome::VerifierUnavailable {
        reason: "verifier_unavailable".to_string(),
    });

    assert!(state
        .normalized_state_for_cycle(9)
        .contains("cycle 9: verifier-unavailable;"));
    assert!(state
        .normalized_state_for_cycle(9)
        .contains("next=retry verifier or park without mutation"));
}

#[test]
fn normalized_state_names_idle_rescan_from_no_work_state() {
    let state = NoWorkState::record(None, complete_dry_observation(10)).expect("dry pass");
    let normalized = conductor(ConductorScope::Repository).normalized_state_for_no_work(10, &state);

    assert!(normalized.starts_with("cycle 10: idle-rescan;"));
    assert!(normalized.contains("tier=all"));
    assert!(normalized.contains("action=rescan"));
    assert!(normalized.contains("reason=idle_rescan"));
    assert!(normalized.contains("mutation_allowed=false"));
    assert!(normalized.contains("next=sleep until the next rescan interval"));
}

#[test]
fn normalized_state_names_resource_park() {
    let state = blocked_state(ConductorOutcome::ResourcePark {
        reason: "token_budget_exhausted".to_string(),
    });
    let normalized = state.normalized_state_for_cycle(11);

    assert!(normalized.starts_with("cycle 11: resource-park;"));
    assert!(normalized.contains("reason=token_budget_exhausted"));
    assert!(normalized.contains("next=wait for resource budget or operator resume"));
}

#[test]
fn normalized_state_names_operator_stop() {
    let state = blocked_state(ConductorOutcome::OperatorStop {
        reason: "operator_requested_stop".to_string(),
    });
    let normalized = state.normalized_state_for_cycle(12);

    assert!(normalized.starts_with("cycle 12: operator-stop;"));
    assert!(normalized.contains("reason=operator_requested_stop"));
    assert!(normalized.contains("next=wait for operator resume"));
}

fn blocked_state(outcome: ConductorOutcome) -> ConductorState {
    selected_state()
        .transition(ConductorEvent::Claimed)
        .expect("claim is recorded")
        .transition(ConductorEvent::DispatchRecorded { outcome })
        .expect("blocked outcome is recorded")
}

fn selected_state() -> ConductorState {
    conductor(ConductorScope::Repository)
        .transition(ConductorEvent::ScanFoundWork)
        .expect("scan finds work")
        .transition(ConductorEvent::SafetyReviewed)
        .expect("review passes")
        .transition(ConductorEvent::Selected {
            issue: 42,
            serialization_reasons: Vec::new(),
        })
        .expect("selection is valid")
}

fn conductor(scope: ConductorScope) -> ConductorState {
    ConductorState::new("berlinguyinca/autospec", scope, 2).expect("repository is valid")
}

fn tier_receipt(pass_id: u64, tier: NoWorkTier, status: TierStatus) -> TierReceipt {
    TierReceipt::new(
        "berlinguyinca/autospec",
        pass_id,
        tier,
        "test-producer",
        1,
        2,
        status,
        FunnelCounts::new(3, 2, 1, 0, 0).expect("funnel is valid"),
        vec![SealedEvidence::new(
            format!("waterfall/{pass_id}/{}.json", tier.as_str()),
            "a".repeat(64),
        )
        .expect("evidence is sealed")],
    )
    .expect("receipt is valid")
}

fn complete_dry_observation(pass_id: u64) -> NoWorkObservation {
    NoWorkObservation {
        repo: "berlinguyinca/autospec".to_string(),
        pass_id,
        evidence_digest: "b".repeat(64),
        tiers: NoWorkTier::ALL.into_iter().map(complete_dry_tier).collect(),
    }
}

fn complete_dry_tier(tier: NoWorkTier) -> (NoWorkTier, TierOutcome) {
    (
        tier,
        TierOutcome::Dry {
            reason: DryReason::NoProposalsGenerated,
        },
    )
}
