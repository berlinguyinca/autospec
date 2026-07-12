use autospec_core::claim::{ClaimLease, ClaimRunState};

#[test]
fn claim_lease_stores_distributed_coordination_fields() {
    let lease = ClaimLease::new(
        1867,
        "worker-a",
        "feat/1867-rust-core-claim-leases",
        1_000,
        1_025,
        300,
    );

    assert_eq!(lease.issue_number, 1867);
    assert_eq!(lease.worker_id, "worker-a");
    assert_eq!(lease.branch, "feat/1867-rust-core-claim-leases");
    assert_eq!(lease.claimed_at, 1_000);
    assert_eq!(lease.updated_at, 1_025);
    assert_eq!(lease.ttl_seconds, 300);
}

#[test]
fn claim_lease_is_fresh_at_injected_server_ttl_boundary() {
    let lease = ClaimLease::new(1867, "worker-a", "feat/1867", 1_000, 1_000, 300);

    assert!(!lease.is_stale_at(1_300));
}

#[test]
fn claim_lease_is_stale_after_injected_server_ttl_boundary() {
    let lease = ClaimLease::new(1867, "worker-a", "feat/1867", 1_000, 1_000, 300);

    assert!(lease.is_stale_at(1_301));
}

#[test]
fn claim_lease_terminal_merged_state_prevents_new_claim() {
    let run_state = ClaimRunState::terminal_merged(1867, "worker-a", "main", 2_000);
    let incoming = ClaimLease::new(1867, "worker-b", "feat/1867", 2_100, 2_100, 300);

    assert!(!run_state.accepts_claim(&incoming, 2_100));
}
