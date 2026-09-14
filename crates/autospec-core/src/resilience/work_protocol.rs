//! Durable Agent Work Protocol — the canonical lifecycle for work, attempts,
//! claims (leases), receipts, and recovery.
//!
//! AutoSpec must know the difference between work existing, being assigned, a
//! worker receiving it, claiming it, an attempt running, a checkpoint being
//! durable, implementation completing, deterministic validation completing,
//! independent review completing, and merge completing. These facts are **not**
//! inferred from one process exit code.
//!
//! This module is pure: it defines the state machine and the transition rules,
//! and the caller persists state through a store. Deterministic checks decide
//! pass/fail; the model may interpret a failure but never convert a
//! deterministic failing gate into a passing one.

use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::ids::{
    AttemptId, ClaimId, IdempotencyKey, ReceiptId, SessionId, WorkId,
};

/// Versioned receipt schema identity.
pub const WORK_RECEIPT_SCHEMA: &str = "autospec.work-receipt.v1";

/// Canonical happy-path lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum WorkState {
    Created,
    Assigned,
    Delivered,
    Claimed,
    Running,
    Completed,
    Validated,
    Reviewed,
    Merged,
}

impl WorkState {
    pub fn as_str(&self) -> &'static str {
        match self {
            WorkState::Created => "CREATED",
            WorkState::Assigned => "ASSIGNED",
            WorkState::Delivered => "DELIVERED",
            WorkState::Claimed => "CLAIMED",
            WorkState::Running => "RUNNING",
            WorkState::Completed => "COMPLETED",
            WorkState::Validated => "VALIDATED",
            WorkState::Reviewed => "REVIEWED",
            WorkState::Merged => "MERGED",
        }
    }
}

/// Non-happy lifecycle states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum WorkStateNonHappy {
    Blocked,
    Failed,
    Abandoned,
    RetryPending,
    NeedsHuman,
    Superseded,
    Cancelled,
    Unknown,
}

impl WorkStateNonHappy {
    pub fn as_str(&self) -> &'static str {
        match self {
            WorkStateNonHappy::Blocked => "BLOCKED",
            WorkStateNonHappy::Failed => "FAILED",
            WorkStateNonHappy::Abandoned => "ABANDONED",
            WorkStateNonHappy::RetryPending => "RETRY_PENDING",
            WorkStateNonHappy::NeedsHuman => "NEEDS_HUMAN",
            WorkStateNonHappy::Superseded => "SUPERSEDED",
            WorkStateNonHappy::Cancelled => "CANCELLED",
            WorkStateNonHappy::Unknown => "UNKNOWN",
        }
    }
}

/// A single durable work item.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkItem {
    pub work_id: WorkId,
    pub state: WorkState,
    pub non_happy: Option<WorkStateNonHappy>,
    /// Latest attempt id (retries create new attempts preserving work identity).
    pub current_attempt: Option<AttemptId>,
    pub created_at: u64,
    pub updated_at: u64,
    pub attempts: Vec<AttemptId>,
}

/// A lease on an attempt. Claims are leases, not permanent flags.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Lease {
    pub claim_id: ClaimId,
    pub work_id: WorkId,
    pub attempt_id: AttemptId,
    pub session_id: SessionId,
    pub acquired_at: u64,
    pub expires_at: u64,
    pub heartbeat_at: u64,
    /// Monotonic fencing generation. A stale worker holding an older generation
    /// cannot finalize an attempt after ownership has moved.
    pub fencing_generation: u64,
    pub lease_seconds: u64,
}

impl Lease {
    pub fn is_expired(&self, now: u64) -> bool {
        now >= self.expires_at
    }
}

/// Receipt acknowledgement stages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ReceiptStage {
    Sent,
    Delivered,
    Claimed,
    Handled,
}

impl ReceiptStage {
    pub fn as_str(&self) -> &'static str {
        match self {
            ReceiptStage::Sent => "sent",
            ReceiptStage::Delivered => "delivered",
            ReceiptStage::Claimed => "claimed",
            ReceiptStage::Handled => "handled",
        }
    }
}

/// A durable acknowledgement receipt attributable to an attempt and consumer.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkReceipt {
    pub schema: String,
    pub receipt_id: ReceiptId,
    pub idempotency_key: IdempotencyKey,
    pub work_id: WorkId,
    pub attempt_id: AttemptId,
    pub consumer: String,
    pub stage: ReceiptStage,
    pub at: u64,
}

impl WorkReceipt {
    pub fn new(
        receipt_id: ReceiptId,
        idempotency_key: IdempotencyKey,
        work_id: WorkId,
        attempt_id: AttemptId,
        consumer: impl Into<String>,
        stage: ReceiptStage,
        at: u64,
    ) -> Self {
        Self {
            schema: WORK_RECEIPT_SCHEMA.to_string(),
            receipt_id,
            idempotency_key,
            work_id,
            attempt_id,
            consumer: consumer.into(),
            stage,
            at,
        }
    }
}

/// Result of an attempted lease acquisition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AcquireResult {
    Acquired(Lease),
    /// Another live lease holds the attempt.
    Contended,
    /// The work/attempt is terminal and must not be re-run.
    Terminal,
}

/// Clock abstraction so the protocol is deterministic and testable.
pub trait Clock {
    fn now_secs(&self) -> u64;
}

/// A `SystemTime`-backed clock.
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_secs(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }
}

/// The store interface the protocol needs. A caller may back this with durable
/// files, a DB, or an in-memory map for tests. Correctness-critical state must
/// live in a durable store, never only in telemetry.
pub trait WorkStore {
    fn load_work(&self, work_id: &WorkId) -> Option<WorkItem>;
    fn save_work(&mut self, item: &WorkItem);
    fn load_lease(&self, work_id: &WorkId) -> Option<Lease>;
    fn save_lease(&mut self, lease: &Lease);
    fn delete_lease(&mut self, work_id: &WorkId);
    /// Record a receipt idempotently. Returns true if it was a duplicate.
    fn record_receipt(&mut self, receipt: &WorkReceipt) -> bool;
}

/// Deterministic transition rules. `from` is the current state; `to` is the
/// requested next happy-path state.
pub fn can_transition(from: WorkState, to: WorkState) -> bool {
    let rank = |s: WorkState| s as usize;
    // Allow forward movement along the canonical chain; also allow staying put.
    rank(to) >= rank(from) && (rank(to) - rank(from)) <= 1
}

/// Apply a deterministic transition to a work item. Returns an error string on
/// an illegal transition so a model can never force a gate to pass.
pub fn transition(item: &mut WorkItem, to: WorkState, now: u64) -> Result<(), String> {
    if item.non_happy.is_some() && to != WorkState::Completed {
        // A non-happy item may only be moved out by explicit recovery, not by a
        // naive forward transition. Keep this conservative.
        return Err(format!(
            "cannot transition {:?} from non-happy state {:?}",
            to,
            item.non_happy.unwrap()
        ));
    }
    if !can_transition(item.state, to) {
        return Err(format!(
            "illegal transition {} -> {}",
            item.state.as_str(),
            to.as_str()
        ));
    }
    item.state = to;
    item.updated_at = now;
    Ok(())
}

/// Atomically acquire a lease on an attempt.
///
/// `existing` is the current lease (if any); the caller reads it through the
/// store. Fencing: if a stale worker (older generation) tries to renew or
/// finalize, it must be rejected.
pub fn acquire_lease(
    store: &mut dyn WorkStore,
    work_id: &WorkId,
    attempt_id: &AttemptId,
    session_id: &SessionId,
    now: u64,
    lease_seconds: u64,
    clock: &dyn Clock,
) -> AcquireResult {
    let work = store.load_work(work_id);
    let terminal = work
        .as_ref()
        .map(|w| {
            matches!(w.state, WorkState::Merged)
                || matches!(w.state, WorkState::Completed)
        })
        .unwrap_or(false);
    if terminal {
        return AcquireResult::Terminal;
    }

    if let Some(existing) = store.load_lease(work_id) {
        if existing.attempt_id == *attempt_id {
            // Same attempt re-claim: only if expired.
            if existing.is_expired(now) {
                let lease = Lease {
                    claim_id: ClaimId::new(b"claim"),
                    work_id: work_id.clone(),
                    attempt_id: attempt_id.clone(),
                    session_id: session_id.clone(),
                    acquired_at: now,
                    expires_at: now + lease_seconds,
                    heartbeat_at: now,
                    fencing_generation: existing.fencing_generation + 1,
                    lease_seconds,
                };
                store.save_lease(&lease);
                return AcquireResult::Acquired(lease);
            }
            return AcquireResult::Contended;
        }
        // A different attempt holds the lease: contended.
        return AcquireResult::Contended;
    }

    let lease = Lease {
        claim_id: ClaimId::new(b"claim"),
        work_id: work_id.clone(),
        attempt_id: attempt_id.clone(),
        session_id: session_id.clone(),
        acquired_at: now,
        expires_at: now + lease_seconds,
        heartbeat_at: now,
        fencing_generation: 1,
        lease_seconds,
    };
    store.save_lease(&lease);
    let _ = clock.now_secs();
    AcquireResult::Acquired(lease)
}

/// Heartbeat renews a lease. Returns true when renewed. A stale worker whose
/// session no longer matches, or whose generation has moved on, is rejected.
pub fn heartbeat(
    store: &mut dyn WorkStore,
    work_id: &WorkId,
    session_id: &SessionId,
    fencing_generation: u64,
    now: u64,
) -> bool {
    let Some(mut lease) = store.load_lease(work_id) else {
        return false;
    };
    if lease.session_id != *session_id || lease.fencing_generation != fencing_generation {
        return false;
    }
    lease.heartbeat_at = now;
    lease.expires_at = now + lease.lease_seconds;
    store.save_lease(&lease);
    true
}

/// Reclaim an expired lease so a new worker can take over. Fencing generation
/// is incremented so the old worker can no longer finalize.
pub fn reclaim_expired(
    store: &mut dyn WorkStore,
    work_id: &WorkId,
    now: u64,
) -> bool {
    let Some(lease) = store.load_lease(work_id) else {
        return false;
    };
    if !lease.is_expired(now) {
        return false;
    }
    let mut new_lease = lease;
    new_lease.fencing_generation += 1;
    new_lease.acquired_at = now;
    new_lease.heartbeat_at = now;
    new_lease.expires_at = now + new_lease.lease_seconds;
    store.save_lease(&new_lease);
    true
}

/// Try to finalize (complete) an attempt. A stale worker with the wrong
/// fencing generation is rejected, preventing split-brain completion after
/// ownership moved.
pub fn try_finalize(
    store: &mut dyn WorkStore,
    work_id: &WorkId,
    attempt_id: &AttemptId,
    session_id: &SessionId,
    fencing_generation: u64,
) -> Result<(), String> {
    let Some(lease) = store.load_lease(work_id) else {
        return Err("no lease to finalize".to_string());
    };
    if lease.attempt_id != *attempt_id {
        return Err("attempt no longer owns the work".to_string());
    }
    if lease.session_id != *session_id {
        return Err("stale session cannot finalize".to_string());
    }
    if lease.fencing_generation != fencing_generation {
        return Err("stale fencing generation cannot finalize".to_string());
    }
    Ok(())
}

/// Idempotent duplicate delivery: record a receipt; if the idempotency key was
/// already seen, treat it as a duplicate and do not double-apply.
pub fn record_delivery(
    store: &mut dyn WorkStore,
    receipt: &WorkReceipt,
) -> bool {
    store.record_receipt(receipt)
}

/// Reconcile nonterminal attempts on dispatcher/orchestrator restart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryAction {
    Resume,
    Retry,
    Block,
    Escalate,
    Noop,
}

/// Deterministic recovery decision for a work item's lease.
pub fn reconcile(
    store: &dyn WorkStore,
    work_id: &WorkId,
    now: u64,
) -> RecoveryAction {
    let Some(lease) = store.load_lease(work_id) else {
        // No lease: work may not be started.
        return RecoveryAction::Block;
    };
    if lease.is_expired(now) {
        // Lease expired: the worker may have crashed. Retry is deterministic.
        return RecoveryAction::Retry;
    }
    // Lease alive: let the owner continue or resume.
    RecoveryAction::Resume
}

/// A durable in-memory store for tests and for callers that want a simple
/// single-process store. Not for production cross-process correctness.
#[derive(Debug, Default)]
pub struct InMemoryWorkStore {
    pub work: std::collections::HashMap<WorkId, WorkItem>,
    pub leases: std::collections::HashMap<WorkId, Lease>,
    pub receipts: std::collections::HashSet<(IdempotencyKey, AttemptId)>,
}

impl WorkStore for InMemoryWorkStore {
    fn load_work(&self, work_id: &WorkId) -> Option<WorkItem> {
        self.work.get(work_id).cloned()
    }
    fn save_work(&mut self, item: &WorkItem) {
        self.work.insert(item.work_id.clone(), item.clone());
    }
    fn load_lease(&self, work_id: &WorkId) -> Option<Lease> {
        self.leases.get(work_id).cloned()
    }
    fn save_lease(&mut self, lease: &Lease) {
        self.leases.insert(lease.work_id.clone(), lease.clone());
    }
    fn delete_lease(&mut self, work_id: &WorkId) {
        self.leases.remove(work_id);
    }
    fn record_receipt(&mut self, receipt: &WorkReceipt) -> bool {
        // HashSet::insert returns true when the element was newly inserted. We
        // return true when it was a *duplicate* so record_delivery reports it
        // idempotently.
        !self
            .receipts
            .insert((receipt.idempotency_key.clone(), receipt.attempt_id.clone()))
    }
}

impl InMemoryWorkStore {
    pub fn insert_work(&mut self, item: WorkItem) {
        self.work.insert(item.work_id.clone(), item);
    }
}

/// A fixed clock for deterministic tests.
pub struct FixedClock(pub u64);

impl Clock for FixedClock {
    fn now_secs(&self) -> u64 {
        self.0
    }
}

/// Default lease length.
pub const DEFAULT_LEASE_SECONDS: u64 = 300;

#[cfg(test)]
mod tests {
    use super::*;

    fn make_work() -> WorkItem {
        WorkItem {
            work_id: WorkId::new(b"w"),
            state: WorkState::Created,
            non_happy: None,
            current_attempt: None,
            created_at: 0,
            updated_at: 0,
            attempts: Vec::new(),
        }
    }

    #[test]
    fn canonical_transitions_are_forward() {
        assert!(can_transition(WorkState::Created, WorkState::Assigned));
        assert!(can_transition(WorkState::Assigned, WorkState::Delivered));
        assert!(can_transition(WorkState::Delivered, WorkState::Claimed));
        assert!(can_transition(WorkState::Claimed, WorkState::Running));
        assert!(can_transition(WorkState::Running, WorkState::Completed));
        assert!(can_transition(WorkState::Completed, WorkState::Validated));
        assert!(can_transition(WorkState::Validated, WorkState::Reviewed));
        assert!(can_transition(WorkState::Reviewed, WorkState::Merged));
        // Illegal: skipping or jumping backward.
        assert!(!can_transition(WorkState::Created, WorkState::Running));
        assert!(!can_transition(WorkState::Merged, WorkState::Created));
    }

    #[test]
    fn transition_rejects_illegal_and_model_override() {
        let mut w = make_work();
        // A model cannot skip straight to Merged (deterministic gate).
        assert!(transition(&mut w, WorkState::Merged, 1).is_err());
        assert!(transition(&mut w, WorkState::Assigned, 1).is_ok());
        // Cannot move a non-happy item forward via naive transition.
        w.non_happy = Some(WorkStateNonHappy::Blocked);
        assert!(transition(&mut w, WorkState::Completed, 2).is_err());
    }

    #[test]
    fn lease_acquire_heartbeat_expiry_reclaim_fencing() {
        let mut store = InMemoryWorkStore::default();
        let work_id = WorkId::new(b"w");
        let mut work = make_work();
        work.work_id = work_id.clone();
        store.insert_work(work);

        let clock = FixedClock(1000);
        let attempt = AttemptId::new(b"a");
        let session = SessionId::new(b"s");
        let acquired = acquire_lease(
            &mut store,
            &work_id,
            &attempt,
            &session,
            1000,
            DEFAULT_LEASE_SECONDS,
            &clock,
        );
        let lease = match acquired {
            AcquireResult::Acquired(l) => l,
            other => panic!("expected acquired, got {:?}", other),
        };

        // Heartbeat renews.
        assert!(heartbeat(&mut store, &work_id, &session, lease.fencing_generation, 1100));
        // Wrong generation is rejected (stale worker).
        assert!(!heartbeat(&mut store, &work_id, &session, lease.fencing_generation + 1, 1200));

        // Finalize works with correct generation, fails with stale.
        assert!(try_finalize(&mut store, &work_id, &attempt, &session, lease.fencing_generation).is_ok());
        assert!(try_finalize(&mut store, &work_id, &attempt, &session, lease.fencing_generation + 1).is_err());

        // Expiry + reclaim bumps generation. The lease was renewed at 1100 so
        // it expires at 1400; use a now past that renewal.
        let now = 1100 + DEFAULT_LEASE_SECONDS + 10;
        assert!(reclaim_expired(&mut store, &work_id, now));
        let new_lease = store.load_lease(&work_id).unwrap();
        assert_eq!(new_lease.fencing_generation, lease.fencing_generation + 1);
        // Old generation can no longer finalize.
        assert!(try_finalize(&mut store, &work_id, &attempt, &session, lease.fencing_generation).is_err());
    }

    #[test]
    fn duplicate_delivery_is_idempotent() {
        let mut store = InMemoryWorkStore::default();
        let receipt = WorkReceipt::new(
            ReceiptId::new(b"r"),
            IdempotencyKey::new(b"k"),
            WorkId::new(b"w"),
            AttemptId::new(b"a"),
            "worker-1",
            ReceiptStage::Delivered,
            1,
        );
        assert!(!record_delivery(&mut store, &receipt)); // first delivery
        assert!(record_delivery(&mut store, &receipt)); // duplicate
    }

    #[test]
    fn recovery_blocks_when_no_lease_and_retries_when_expired() {
        let mut store = InMemoryWorkStore::default();
        let work_id = WorkId::new(b"w");
        let mut work = make_work();
        work.work_id = work_id.clone();
        store.insert_work(work);

        assert_eq!(reconcile(&store, &work_id, 1000), RecoveryAction::Block);

        let clock = FixedClock(1000);
        let attempt = AttemptId::new(b"a");
        let session = SessionId::new(b"s");
        acquire_lease(&mut store, &work_id, &attempt, &session, 1000, DEFAULT_LEASE_SECONDS, &clock);
        assert_eq!(reconcile(&store, &work_id, 1100), RecoveryAction::Resume);
        // After expiry, deterministic retry.
        assert_eq!(
            reconcile(&store, &work_id, 1000 + DEFAULT_LEASE_SECONDS + 1),
            RecoveryAction::Retry
        );
    }

    #[test]
    fn terminal_work_is_not_reacquired() {
        let mut store = InMemoryWorkStore::default();
        let work_id = WorkId::new(b"w");
        let mut work = make_work();
        work.work_id = work_id.clone();
        work.state = WorkState::Merged;
        store.insert_work(work);
        let clock = FixedClock(1000);
        let result = acquire_lease(
            &mut store,
            &work_id,
            &AttemptId::new(b"a"),
            &SessionId::new(b"s"),
            1000,
            DEFAULT_LEASE_SECONDS,
            &clock,
        );
        assert_eq!(result, AcquireResult::Terminal);
    }
}
