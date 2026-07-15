use autospec_core::autonomous_lifecycle::{
    decide, Budget, Health, LifecycleDecision, LifecycleInput, LifecycleReject, LifecycleTier,
    ParkReason, StopMode, ABANDONED_LEASE_SECS, ISSUE_FAILURE_CAP, STALE_LEASE_SECS,
};

#[test]
fn stop_precedes_ready_tier_one() {
    let input = LifecycleInput::ready("owner/repo").with_stop(StopMode::Graceful);

    assert_eq!(
        decide(&input),
        LifecycleDecision::Stop {
            mode: StopMode::Graceful
        }
    );
}

#[test]
fn ready_tier_one_runs_when_no_gate_applies() {
    assert_eq!(
        decide(&LifecycleInput::ready("owner/repo")),
        LifecycleDecision::Run {
            tier: LifecycleTier::Tier1
        }
    );
}

#[test]
fn stale_and_cross_scope_claims_are_non_executable() {
    assert_eq!(
        decide(&LifecycleInput::ready("owner/repo").with_lease_age_secs(STALE_LEASE_SECS + 1)),
        LifecycleDecision::Reject(LifecycleReject::StaleLease)
    );
    assert_eq!(
        decide(&LifecycleInput::ready("owner/repo").with_lease_age_secs(ABANDONED_LEASE_SECS + 1),),
        LifecycleDecision::Reject(LifecycleReject::StaleLease)
    );
    assert_eq!(
        decide(&LifecycleInput::ready("owner/repo").with_claim_repo("other/repo")),
        LifecycleDecision::Reject(LifecycleReject::CrossScopeClaim)
    );
}

#[test]
fn every_non_idle_tier_runs_and_idle_rescans() {
    for tier in [
        LifecycleTier::Tier15,
        LifecycleTier::Tier2,
        LifecycleTier::Tier3,
        LifecycleTier::Tier4,
        LifecycleTier::Tier5,
        LifecycleTier::Tier6,
        LifecycleTier::Tier7,
    ] {
        assert_eq!(
            decide(&LifecycleInput::ready("owner/repo").with_tier(tier)),
            LifecycleDecision::Run { tier }
        );
    }
    assert_eq!(
        decide(&LifecycleInput::ready("owner/repo").with_tier(LifecycleTier::Idle)),
        LifecycleDecision::Park {
            reason: ParkReason::IdleRescan
        }
    );
}

#[test]
fn human_health_and_budget_gates_precede_work() {
    assert_eq!(
        decide(&LifecycleInput::ready("owner/repo").with_human_gate()),
        LifecycleDecision::Park {
            reason: ParkReason::HumanGate
        }
    );
    assert_eq!(
        decide(&LifecycleInput::ready("owner/repo").with_health(Health::Wait)),
        LifecycleDecision::Park {
            reason: ParkReason::HealthWait
        }
    );
    assert_eq!(
        decide(&LifecycleInput::ready("owner/repo").with_health(Health::Halt)),
        LifecycleDecision::Park {
            reason: ParkReason::HealthHalt
        }
    );
    assert_eq!(
        decide(&LifecycleInput::ready("owner/repo").with_budget(Budget::SoftCap)),
        LifecycleDecision::Park {
            reason: ParkReason::BudgetSoftCap
        }
    );
    assert_eq!(
        decide(&LifecycleInput::ready("owner/repo").with_budget(Budget::HardCap)),
        LifecycleDecision::Park {
            reason: ParkReason::BudgetHardCap
        }
    );
}

#[test]
fn terminal_ownership_and_failure_cap_claims_are_rejected() {
    assert_eq!(
        decide(&LifecycleInput::ready("owner/repo").with_terminal_claim()),
        LifecycleDecision::Reject(LifecycleReject::TerminalClaim)
    );
    assert_eq!(
        decide(&LifecycleInput::ready("owner/repo").with_ownership_mismatch()),
        LifecycleDecision::Reject(LifecycleReject::OwnershipMismatch)
    );
    assert_eq!(
        decide(&LifecycleInput::ready("owner/repo").with_failure_count(ISSUE_FAILURE_CAP)),
        LifecycleDecision::Reject(LifecycleReject::FailureCap)
    );
}

#[test]
fn malformed_scope_is_rejected_after_stop_precedence() {
    assert_eq!(
        decide(&LifecycleInput::ready("owner").with_stop(StopMode::Immediate)),
        LifecycleDecision::Stop {
            mode: StopMode::Immediate
        }
    );
    assert_eq!(
        decide(&LifecycleInput::ready("owner")),
        LifecycleDecision::Reject(LifecycleReject::InvalidScope)
    );
}
