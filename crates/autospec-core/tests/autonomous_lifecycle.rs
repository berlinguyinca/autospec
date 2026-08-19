use autospec_core::autonomous_lifecycle::{
    decide, decide_capacity, decide_conductor_lease, Budget, CapacityDecision, CapacityInput,
    ClaimBranch, ClaimContext, ClaimEvidence, ConductorLeaseDecision, ConductorLeaseInput,
    ConductorLeaseReclaim, Health, IssueNumber, LeaseFreshness, LifecycleDecision, LifecycleInput,
    LifecycleRecord, LifecycleReject, LifecycleTier, ParkReason, RepositoryScope, StopMode,
    WorkerId, ABANDONED_LEASE_SECS, ISSUE_FAILURE_CAP, STALE_LEASE_SECS,
};

#[test]
fn conductor_lease_reclaims_at_boundaries_and_for_dead_local_pid() {
    // A dead owner on this host is reclaimable inside the claim window too. Asserting Held
    // here parked the repository for STALE_LEASE_SECS after any conductor crash, and
    // contradicted release_terminated_owner, which already releases a claimed lease whose
    // lock_pid is dead.
    assert_eq!(
        decide_conductor_lease(ConductorLeaseInput::claimed(1, true)),
        ConductorLeaseDecision::Reclaim(ConductorLeaseReclaim::DeadSameHostPid),
    );
    // Still Held when the owner is not known-dead: only proven absence reclaims.
    assert_eq!(
        decide_conductor_lease(ConductorLeaseInput::claimed(1, false)),
        ConductorLeaseDecision::Held,
    );
    // Still reclaimed at the stale boundary, now attributed to the dead owner rather than the
    // elapsed clock. The reason is diagnostic only -- nothing branches on it, it is rendered to
    // a string for reporting -- and a proven-dead owner is the more precise explanation.
    assert_eq!(
        decide_conductor_lease(ConductorLeaseInput::claimed(300, true)),
        ConductorLeaseDecision::Reclaim(ConductorLeaseReclaim::DeadSameHostPid),
    );
    assert_eq!(
        decide_conductor_lease(ConductorLeaseInput::claimed(300, false)),
        ConductorLeaseDecision::Reclaim(ConductorLeaseReclaim::ClaimedExpired),
    );
    assert_eq!(
        decide_conductor_lease(ConductorLeaseInput::running(10_800, false)),
        ConductorLeaseDecision::Reclaim(ConductorLeaseReclaim::Abandoned),
    );
    assert_eq!(
        decide_conductor_lease(ConductorLeaseInput::running(300, false)),
        ConductorLeaseDecision::Held,
    );
    assert_eq!(
        decide_conductor_lease(ConductorLeaseInput::running(1, true)),
        ConductorLeaseDecision::Reclaim(ConductorLeaseReclaim::DeadSameHostPid),
    );
}

#[test]
fn capacity_checks_usage_before_issue_and_zero_disables_a_cap() {
    assert_eq!(
        decide_capacity(CapacityInput::new(10, 10, 4, 4)),
        CapacityDecision::UsageCap
    );
    assert_eq!(
        decide_capacity(CapacityInput::new(10, 0, 4, 4)),
        CapacityDecision::IssueCap
    );
    assert_eq!(
        decide_capacity(CapacityInput::new(0, 10, 10_000, 0)),
        CapacityDecision::WithinCap
    );
}

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
        LifecycleDecision::Reject(LifecycleReject::AbandonedLease)
    );
    let claim = ClaimContext::active(
        RepositoryScope::try_from("other/repo").expect("scope"),
        IssueNumber::new(41).expect("issue"),
        WorkerId::try_from("worker-a").expect("worker"),
        ClaimBranch::try_from("autonomous/issue-41").expect("branch"),
        LeaseFreshness::Fresh,
    );
    assert_eq!(
        decide(&LifecycleInput::ready("owner/repo").with_claim(claim)),
        LifecycleDecision::Reject(LifecycleReject::CrossScopeClaim)
    );
}

#[test]
fn typed_claim_identity_distinguishes_abandoned_leases_and_owner_mismatches() {
    let scope = RepositoryScope::try_from("owner/repo").expect("scope");
    let worker = WorkerId::try_from("worker-a").expect("worker");
    let branch = ClaimBranch::try_from("autonomous/issue-41").expect("branch");
    let claim = ClaimContext::active(
        scope.clone(),
        IssueNumber::new(41).expect("issue"),
        worker.clone(),
        branch.clone(),
        LeaseFreshness::from_age_secs(ABANDONED_LEASE_SECS + 1),
    );

    assert_eq!(claim.lease_freshness(), LeaseFreshness::Abandoned);
    assert_eq!(
        decide(&LifecycleInput::from_scope(scope.clone()).with_claim(claim)),
        LifecycleDecision::Reject(LifecycleReject::AbandonedLease)
    );

    let fresh_claim = ClaimContext::active(
        scope.clone(),
        IssueNumber::new(41).expect("issue"),
        worker,
        branch.clone(),
        LeaseFreshness::Fresh,
    );
    assert_eq!(
        decide(
            &LifecycleInput::from_scope(scope)
                .with_claim(fresh_claim)
                .with_expected_claim(WorkerId::try_from("worker-b").expect("worker"), branch,),
        ),
        LifecycleDecision::Reject(LifecycleReject::OwnershipMismatch)
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
    let scope = RepositoryScope::try_from("owner/repo").expect("scope");
    let worker = WorkerId::try_from("worker-a").expect("worker");
    let branch = ClaimBranch::try_from("autonomous/issue-41").expect("branch");
    assert_eq!(
        decide(
            &LifecycleInput::from_scope(scope.clone()).with_claim(ClaimContext::terminal(
                scope.clone(),
                IssueNumber::new(41).expect("issue"),
                worker.clone(),
                branch.clone(),
            ))
        ),
        LifecycleDecision::Reject(LifecycleReject::TerminalClaim)
    );
    assert_eq!(
        decide(
            &LifecycleInput::from_scope(scope.clone())
                .with_claim(ClaimContext::active(
                    scope.clone(),
                    IssueNumber::new(41).expect("issue"),
                    worker,
                    branch.clone(),
                    LeaseFreshness::Fresh,
                ))
                .with_expected_claim(WorkerId::try_from("worker-b").expect("worker"), branch,),
        ),
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

#[test]
fn malformed_claim_evidence_is_non_executable() {
    assert_eq!(
        decide(&LifecycleInput::ready("owner/repo").with_claim_evidence(ClaimEvidence::Malformed)),
        LifecycleDecision::Reject(LifecycleReject::MalformedClaim)
    );
}

#[test]
fn lifecycle_record_parses_no_steering_park_reason() {
    let record = LifecycleRecord::parse_json(
        r#"{"version":1,"repo":"test/repo","result":{"decision":"park","reason":"no-steering"}}"#,
    )
    .expect("parse no-steering park record");

    assert_eq!(
        record.decision,
        LifecycleDecision::Park {
            reason: ParkReason::NoSteering,
        }
    );
}
