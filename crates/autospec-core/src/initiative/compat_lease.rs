//! One read-only view over the three lease types.
//!
//! `initiative::task::Lease`, `resilience::work_protocol::Lease` and
//! `claim::ClaimLease` all say who holds work and until when, with different
//! fields and different expiry rules. [`LeaseView`] reads any of them the same
//! way and never drops fencing data: a source that carries a
//! `fencing_generation` reports it, and one that does not reports `None` rather
//! than an invented value. See `docs/contracts/initiative-dag-model-mapping.md`
//! section "Lease types".
//!
//! `claim/mod.rs` is over the LOC ratchet, so the impls live here.

use super::task::Lease as TaskLease;
use crate::claim::ClaimLease;
use crate::resilience::work_protocol::Lease as WorkLease;

/// A read-only view of a lease.
pub trait LeaseView {
    /// Who holds the lease.
    fn holder(&self) -> &str;
    /// Unix seconds at which the lease expires. Informational: use
    /// [`LeaseView::is_live`] to decide whether the lease still holds.
    fn expires_at(&self) -> u64;
    /// The monotonic fencing generation, when the lease carries one.
    fn fencing_generation(&self) -> Option<u64>;
    /// Whether the lease is still held at `now`, by the source type's own rule.
    fn is_live(&self, now: u64) -> bool;
}

impl LeaseView for TaskLease {
    fn holder(&self) -> &str {
        &self.session_name
    }

    fn expires_at(&self) -> u64 {
        self.expires_at
    }

    fn fencing_generation(&self) -> Option<u64> {
        None
    }

    fn is_live(&self, now: u64) -> bool {
        TaskLease::is_live(self, now)
    }
}

impl LeaseView for WorkLease {
    fn holder(&self) -> &str {
        self.session_id.as_str()
    }

    fn expires_at(&self) -> u64 {
        self.expires_at
    }

    fn fencing_generation(&self) -> Option<u64> {
        Some(self.fencing_generation)
    }

    fn is_live(&self, now: u64) -> bool {
        !self.is_expired(now)
    }
}

impl LeaseView for ClaimLease {
    fn holder(&self) -> &str {
        &self.worker_id
    }

    /// `updated_at + ttl_seconds`. A merged claim is terminal and never
    /// expires: its `expires_at` looks long past, but [`LeaseView::is_live`]
    /// stays true. Callers must test liveness with `is_live`, not by comparing
    /// `expires_at` to the clock.
    fn expires_at(&self) -> u64 {
        self.updated_at.saturating_add(self.ttl_seconds)
    }

    fn fencing_generation(&self) -> Option<u64> {
        None
    }

    fn is_live(&self, now: u64) -> bool {
        !self.is_stale_at(now)
    }
}

/// Whether a new holder with `next_generation` may take over from `current`.
///
/// `false` while `current` is live. After expiry, `true` only for a strictly
/// higher fencing generation. A lease that carries no generation, or a request
/// that supplies none, returns `false`: an expired lease alone never
/// authorizes a second mutator.
pub fn may_supersede(current: &dyn LeaseView, next_generation: Option<u64>, now: u64) -> bool {
    if current.is_live(now) {
        return false;
    }
    match (current.fencing_generation(), next_generation) {
        (Some(held), Some(next)) => next > held,
        (Some(_), None) | (None, Some(_)) | (None, None) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resilience::ids::{AttemptId, ClaimId, SessionId, WorkId};

    fn work_lease(expires_at: u64, generation: u64) -> WorkLease {
        WorkLease {
            claim_id: ClaimId("claim-1".to_string()),
            work_id: WorkId("work-1".to_string()),
            attempt_id: AttemptId("attempt-1".to_string()),
            session_id: SessionId("session-1".to_string()),
            acquired_at: 0,
            expires_at,
            heartbeat_at: 0,
            fencing_generation: generation,
            lease_seconds: expires_at,
        }
    }

    #[test]
    fn a_work_protocol_lease_reports_its_fencing_generation() {
        let lease = work_lease(100, 3);
        assert_eq!(LeaseView::fencing_generation(&lease), Some(3));
        assert_eq!(LeaseView::holder(&lease), "session-1");
        assert_eq!(LeaseView::expires_at(&lease), 100);
    }

    #[test]
    fn a_claim_lease_expires_at_its_last_update_plus_ttl() {
        let lease = ClaimLease::new(7, "worker", "feat/x", 10, 40, 60);
        assert_eq!(LeaseView::expires_at(&lease), 100);
        assert_eq!(LeaseView::holder(&lease), "worker");
        assert_eq!(LeaseView::fencing_generation(&lease), None);
        assert!(LeaseView::is_live(&lease, 100));
        assert!(!LeaseView::is_live(&lease, 101));
    }

    #[test]
    fn a_merged_claim_is_always_live_so_it_is_never_superseded() {
        let lease = ClaimLease::terminal_merged(7, "worker", "feat/x", 40);
        assert!(LeaseView::is_live(&lease, u64::MAX));
        assert!(!may_supersede(&lease, Some(9), u64::MAX));
    }

    #[test]
    fn a_task_lease_has_no_generation_and_is_live_until_it_expires() {
        let lease = TaskLease {
            session_name: "pi-1".to_string(),
            expires_at: 50,
        };
        assert_eq!(LeaseView::holder(&lease), "pi-1");
        assert_eq!(LeaseView::fencing_generation(&lease), None);
        assert!(LeaseView::is_live(&lease, 49));
        assert!(!LeaseView::is_live(&lease, 50));
    }

    #[test]
    fn a_live_lease_cannot_be_superseded() {
        let lease = work_lease(100, 3);
        assert!(!may_supersede(&lease, Some(4), 99));
    }

    #[test]
    fn an_expired_lease_yields_only_to_a_strictly_higher_generation() {
        let lease = work_lease(100, 3);
        assert!(may_supersede(&lease, Some(4), 100));
        assert!(!may_supersede(&lease, Some(3), 100));
        assert!(!may_supersede(&lease, Some(2), 100));
        assert!(!may_supersede(&lease, None, 100));
    }

    #[test]
    fn an_expired_lease_without_a_generation_is_never_superseded() {
        let task = TaskLease {
            session_name: "pi-1".to_string(),
            expires_at: 50,
        };
        assert!(!may_supersede(&task, Some(1), 50));
        let claim = ClaimLease::new(7, "worker", "feat/x", 0, 0, 10);
        assert!(!may_supersede(&claim, Some(1), 1000));
    }
}
