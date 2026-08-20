use autospec_core::autonomous::no_work::{
    DryReason, NoWorkObservation, NoWorkState, NoWorkTier, TierOutcome,
};
use autospec_core::autonomous::waterfall::{FunnelCounts, SealedEvidence, TierReceipt, TierStatus};
use autospec_core::coordination::{
    ConductorEvent, ConductorOutcome, ConductorScope, ConductorState,
};

const BLOCKED_BACKLOG_THRESHOLD: u32 = 5;

#[test]
fn no_progress_diagnostic_persists_counts_and_resets_deterministically() {
    let state = conductor(ConductorScope::Repository)
        .record_no_progress_cycle("waterfall_pending")
        .expect("first no-progress cycle")
        .record_no_progress_cycle("waterfall_pending")
        .expect("repeated no-progress cycle");
    assert_eq!(state.no_progress_reason(), Some("waterfall_pending"));
    assert_eq!(state.no_progress_cycles(), 2);

    let restored = ConductorState::parse_json(&state.to_json()).expect("diagnostic persists");
    assert_eq!(restored.no_progress_reason(), Some("waterfall_pending"));
    assert_eq!(restored.no_progress_cycles(), 2);

    let changed = restored
        .record_no_progress_cycle("repository_empty")
        .expect("changed reason resets count");
    assert_eq!(changed.no_progress_reason(), Some("repository_empty"));
    assert_eq!(changed.no_progress_cycles(), 1);

    let cleared = changed
        .clear_no_progress_diagnostic()
        .expect("work advancement clears diagnostic");
    assert_eq!(cleared.no_progress_reason(), None);
    assert_eq!(cleared.no_progress_cycles(), 0);
}

#[test]
fn scan_no_progress_diagnostic_overrides_a_stale_blocked_outcome() {
    let state = selected_state()
        .transition(ConductorEvent::Claimed)
        .expect("claim is recorded")
        .transition(ConductorEvent::DispatchRecorded {
            outcome: ConductorOutcome::Blocked("executor_receipt_failed".to_string()),
        })
        .expect("blocked outcome is recorded")
        .transition(ConductorEvent::RetireObsoleteSelection)
        .expect("obsolete selection is retired")
        .record_no_progress_cycle("integration_base_dirty")
        .expect("dirty integration cycle is recorded");

    let normalized = state.normalized_state_for_cycle(13);

    assert!(normalized.starts_with("cycle 13: scan;"), "{normalized}");
    assert!(normalized.contains("action=scan"), "{normalized}");
    assert!(
        normalized.contains("reason=integration_base_dirty"),
        "{normalized}"
    );
    assert!(normalized.contains("affected issues=none"), "{normalized}");
    assert!(
        !normalized.contains("executor_receipt_failed"),
        "{normalized}"
    );
}

#[test]
fn legacy_blocked_scan_state_migrates_to_current_no_progress_status() {
    let state = selected_state()
        .transition(ConductorEvent::Claimed)
        .expect("claim is recorded")
        .transition(ConductorEvent::DispatchRecorded {
            outcome: ConductorOutcome::Blocked("executor_receipt_failed".to_string()),
        })
        .expect("blocked outcome is recorded")
        .transition(ConductorEvent::RetireObsoleteSelection)
        .expect("obsolete selection is retired")
        .record_no_progress_cycle("integration_base_dirty")
        .expect("dirty integration cycle is recorded");
    let legacy = state
        .to_json()
        .replace("\"state\":\"scan\"", "\"state\":\"blocked\"");

    let restored = ConductorState::parse_json(&legacy).expect("legacy scan state migrates");

    assert_eq!(restored, state);
    assert!(restored.to_json().contains("\"state\":\"scan\""));
}

#[test]
fn no_progress_diagnostic_is_bounded_and_legacy_state_defaults_empty() {
    let legacy = conductor(ConductorScope::Repository)
        .to_json()
        .replace("\"no_progress_cycles\":0,", "")
        .replace("\"no_progress_reason\":null,", "");
    let restored = ConductorState::parse_json(&legacy).expect("legacy state parses");
    assert_eq!(restored.no_progress_reason(), None);
    assert_eq!(restored.no_progress_cycles(), 0);

    assert!(conductor(ConductorScope::Repository)
        .record_no_progress_cycle("x".repeat(257))
        .expect_err("oversized diagnostic is rejected")
        .contains("256"));
}

#[test]
fn blocked_backlog_governor_seals_after_repeated_identical_cycles() {
    let mut state = conductor(ConductorScope::Repository);
    for cycle in 1..=BLOCKED_BACKLOG_THRESHOLD {
        state = state
            .record_blocked_backlog_cycle("missing_safety_reviewed", vec![43, 42])
            .expect("record blocked cycle");
        assert_eq!(state.blocked_backlog_cycles(), cycle);
        if cycle < BLOCKED_BACKLOG_THRESHOLD {
            assert_eq!(
                state.phase(),
                autospec_core::coordination::ConductorPhase::Scan
            );
        }
    }
    assert_eq!(
        state.phase(),
        autospec_core::coordination::ConductorPhase::AllBlocked
    );
    assert_eq!(
        state.blocked_backlog_reason(),
        Some("missing_safety_reviewed")
    );
    assert_eq!(state.blocked_backlog_issues(), &[42, 43]);
    let restored = ConductorState::parse_json(&state.to_json()).expect("persisted governor state");
    assert_eq!(restored.blocked_backlog_cycles(), BLOCKED_BACKLOG_THRESHOLD);
    assert_eq!(
        restored.blocked_backlog_reason(),
        Some("missing_safety_reviewed")
    );
    assert_eq!(
        restored.phase(),
        autospec_core::coordination::ConductorPhase::AllBlocked
    );
}

#[test]
fn persisted_blocked_backlog_reason_defaults_to_none_when_missing() {
    let encoded = conductor(ConductorScope::Repository)
        .to_json()
        .replace("\"blocked_backlog_reason\":null,", "");

    let restored = ConductorState::parse_json(&encoded).expect("legacy state parses");

    assert_eq!(restored.blocked_backlog_reason(), None);
}

#[test]
fn persisted_blocked_backlog_reason_accepts_explicit_null() {
    let encoded = conductor(ConductorScope::Repository).to_json();

    let restored = ConductorState::parse_json(&encoded).expect("null reason parses");

    assert_eq!(restored.blocked_backlog_reason(), None);
}

#[test]
fn blocked_backlog_governor_resets_when_reason_changes() {
    let state = conductor(ConductorScope::Repository)
        .record_blocked_backlog_cycle("missing_safety_reviewed", vec![42])
        .expect("first cycle")
        .record_blocked_backlog_cycle("missing_qa", vec![42])
        .expect("changed cycle");
    assert_eq!(state.blocked_backlog_cycles(), 1);
    assert_eq!(state.blocked_backlog_reason(), Some("missing_qa"));
}

#[test]
fn blocked_backlog_governor_can_resume_after_issue_set_changes() {
    let mut state = conductor(ConductorScope::Repository);
    for _ in 0..BLOCKED_BACKLOG_THRESHOLD {
        state = state
            .record_blocked_backlog_cycle("missing_safety_reviewed", vec![42])
            .expect("block cycle");
    }
    assert_eq!(
        state.phase(),
        autospec_core::coordination::ConductorPhase::AllBlocked
    );
    let resumed = state
        .record_blocked_backlog_cycle("missing_safety_reviewed", vec![99])
        .expect("changed issue set resumes scan");
    assert_eq!(
        resumed.phase(),
        autospec_core::coordination::ConductorPhase::Scan
    );
    assert_eq!(resumed.blocked_backlog_cycles(), 1);
}

#[test]
fn persisted_state_rejects_a_forged_normalized_state() {
    let encoded = selected_state()
        .to_json()
        .replace("\"state\":\"claim\"", "\"state\":\"idle-rescan\"");

    assert!(ConductorState::parse_json(&encoded).is_err());
}

#[test]
fn persisted_boundary_state_rejects_contradictory_phase_and_outcome() {
    let forged = concat!(
        r#"{"schema":1,"repo":"berlinguyinca/autospec","scope":"repository","#,
        r#""phase":"all_blocked","state":"succeeded","selected_issue":null,"#,
        r#""serialization_reasons":[],"retry_count":0,"retry_limit":2,"#,
        r#""last_outcome":{"kind":"succeeded","reason":null},"#,
        r#""pause_reason":null,"terminal_reason":null,"resume_phase":null}"#,
    );

    assert!(ConductorState::parse_json(forged)
        .expect_err("contradictory boundary state is rejected")
        .contains("incompatible phase/outcome"));
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
