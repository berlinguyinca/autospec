//! End-to-end proof of the resilient runtime across all five features.
//!
//! This integration test walks the canonical lifecycle the implementation
//! spec requires (§15 Phase 8 / §16 acceptance):
//!
//! ```text
//! dispatch -> execution -> context threshold -> durable checkpoint
//! -> resume -> implementation completion -> deterministic validation
//! -> independent review -> lesson candidate -> validated promotion
//! -> later memory-map retrieval
//! ```
//!
//! It also exercises the resilience scenarios: secret rejection, unavailable
//! memory provider, lease fencing, duplicate delivery, crash-state recovery,
//! unknown context usage, and source mutation during an attention stream. It
//! uses only pure modules and an in-memory store, so it runs with no harness,
//! no model, and no telemetry.

use autospec_core::resilience::attention_stream::{
    AttentionStream, ChunkOutput, Finding,
};
use autospec_core::resilience::context_guardian::{
    evaluate_threshold, may_begin_substantial_phase, validate_checkpoint, ContextGuardianConfig,
    ContextObservation, EstimateSource, ThresholdVerdict,
};
use autospec_core::resilience::ids::{
    AttemptId, CandidateId, CheckpointId, ExecutionId, IdempotencyKey, ReceiptId, SessionId,
    StreamId, WorkId,
};
use autospec_core::resilience::learning::{
    promote_verdict, set_status, LessonCandidate, LessonKind, LessonStatus, PromotionVerdict,
};
use autospec_core::resilience::memory_map::{
    generate_map, MemoryEntryRef, MemoryKind, StaticMemoryProvider,
};
use autospec_core::resilience::work_protocol::{
    acquire_lease, record_delivery, reconcile, transition, try_finalize, AcquireResult,
    InMemoryWorkStore, ReceiptStage, RecoveryAction, WorkItem, WorkReceipt, WorkState, WorkStore,
    DEFAULT_LEASE_SECONDS,
};
use autospec_core::resilience::Clock;

struct FixedClock(u64);
impl Clock for FixedClock {
    fn now_secs(&self) -> u64 {
        self.0
    }
}

fn make_work(work_id: WorkId) -> WorkItem {
    WorkItem {
        work_id,
        state: WorkState::Created,
        non_happy: None,
        current_attempt: None,
        created_at: 0,
        updated_at: 0,
        attempts: Vec::new(),
    }
}

#[test]
fn end_to_end_dispatch_to_promotion_to_retrieval() {
    let nonce = b"e2e";
    let work_id = WorkId::new(nonce);
    let attempt_id = AttemptId::new(nonce);
    let execution_id = ExecutionId::new(nonce);
    let session_id = SessionId::new(nonce);

    let mut store = InMemoryWorkStore::default();
    store.insert_work(make_work(work_id.clone()));
    let clock = FixedClock(1000);

    // --- dispatch / execution ---
    assert!(
        transition(&mut store.work.get_mut(&work_id).unwrap(), WorkState::Assigned, 1000).is_ok()
    );

    // --- execution: acquire a lease ---
    let acquired = acquire_lease(
        &mut store,
        &work_id,
        &attempt_id,
        &session_id,
        1000,
        DEFAULT_LEASE_SECONDS,
        &clock,
    );
    let lease = match acquired {
        AcquireResult::Acquired(l) => l,
        other => panic!("expected acquired, got {other:?}"),
    };

    // --- context threshold reached; checkpoint generated ---
    let cfg = ContextGuardianConfig::default();
    let observation = ContextObservation {
        window_tokens: 10_000,
        estimated_used_tokens: 9_000,
        source: EstimateSource::Estimated,
    };
    assert_eq!(
        evaluate_threshold(&cfg, &observation),
        ThresholdVerdict::Required
    );
    // Without a durable checkpoint, a new substantial phase must not begin.
    assert!(!may_begin_substantial_phase(ThresholdVerdict::Required, false));

    let mut checkpoint = autospec_core::resilience::ContextCheckpoint::new(
        CheckpointId::new(nonce),
        execution_id.clone(),
        attempt_id.clone(),
        session_id.clone(),
        work_id.clone(),
        "implement request routing",
        observation,
    );
    checkpoint.next_actions = vec!["add router tests".to_string()];
    assert!(validate_checkpoint(&checkpoint, &cfg).valid);

    // --- resume ---
    assert!(may_begin_substantial_phase(ThresholdVerdict::Required, true));

    // --- implementation completion (walk the canonical chain) ---
    assert!(
        transition(&mut store.work.get_mut(&work_id).unwrap(), WorkState::Delivered, 1001).is_ok()
    );
    assert!(
        transition(&mut store.work.get_mut(&work_id).unwrap(), WorkState::Claimed, 1001).is_ok()
    );
    assert!(
        transition(&mut store.work.get_mut(&work_id).unwrap(), WorkState::Running, 1001).is_ok()
    );
    assert!(
        try_finalize(&mut store, &work_id, &attempt_id, &session_id, lease.fencing_generation)
            .is_ok()
    );
    assert!(
        transition(&mut store.work.get_mut(&work_id).unwrap(), WorkState::Completed, 1002).is_ok()
    );

    // --- deterministic validation + independent review (forward chain) ---
    assert!(
        transition(&mut store.work.get_mut(&work_id).unwrap(), WorkState::Validated, 1003).is_ok()
    );
    assert!(
        transition(&mut store.work.get_mut(&work_id).unwrap(), WorkState::Reviewed, 1004).is_ok()
    );

    // --- lesson candidate from the successful work ---
    let mut candidate = LessonCandidate::new(
        CandidateId::new(b"lesson"),
        work_id.clone(),
        attempt_id.clone(),
        LessonKind::Procedure,
        "request routing dispatch must live in the router module",
        "repo/autospec-orchestrator",
    );
    assert_eq!(promote_verdict(&candidate), PromotionVerdict::KeepCandidate);
    candidate
        .evidence
        .validation_results
        .push("validation passed".to_string());
    candidate
        .evidence
        .review_results
        .push("independent review OK".to_string());
    candidate.confidence = 0.92;
    assert_eq!(promote_verdict(&candidate), PromotionVerdict::Promote);
    set_status(&mut candidate, LessonStatus::Promoted);

    // --- later memory-map retrieval surfaces the promoted lesson ---
    let provider = StaticMemoryProvider {
        available: true,
        entries: vec![MemoryEntryRef {
            summary: candidate.statement.clone(),
            kind: MemoryKind::Procedure,
            scope: Some(candidate.scope.clone()),
            confidence: candidate.confidence,
            status: "validated".to_string(),
            provenance: candidate.source_work_id.as_str().to_string(),
            supersedes: None,
        }],
    };
    let map = generate_map(&provider, "implement request routing", 3000).unwrap();
    assert!(!map.degraded);
    assert!(
        map.validated_procedures
            .iter()
            .any(|e| e.summary.contains("request routing"))
    );
}

#[test]
fn secret_in_checkpoint_is_rejected_end_to_end() {
    let cfg = ContextGuardianConfig::default();
    let mut checkpoint = autospec_core::resilience::ContextCheckpoint::new(
        CheckpointId::new(b"s"),
        ExecutionId::new(b"s"),
        AttemptId::new(b"s"),
        SessionId::new(b"s"),
        WorkId::new(b"s"),
        "objective",
        ContextObservation {
            window_tokens: 100,
            estimated_used_tokens: 50,
            source: EstimateSource::Exact,
        },
    );
    checkpoint.decisions.push("use ghp_abcdefghijklmnopqrstuvwxyz".to_string());
    let validation = validate_checkpoint(&checkpoint, &cfg);
    assert!(!validation.valid);
    assert!(validation.errors.iter().any(|e| e.contains("secret-like")));
}

#[test]
fn unavailable_memory_degrades_without_fabrication() {
    let provider = StaticMemoryProvider {
        available: false,
        entries: vec![],
    };
    let map = generate_map(&provider, "task", 3000).unwrap();
    assert!(map.degraded);
    assert!(map.validated_procedures.is_empty());
    assert!(map.recent_decisions.is_empty());
}

#[test]
fn stale_worker_cannot_finalize_after_reassignment() {
    let mut store = InMemoryWorkStore::default();
    let work_id = WorkId::new(b"fence");
    store.insert_work(make_work(work_id.clone()));
    let clock = FixedClock(1000);

    let attempt_a = AttemptId::new(b"a");
    let session_a = SessionId::new(b"a");
    let lease_a = match acquire_lease(
        &mut store,
        &work_id,
        &attempt_a,
        &session_a,
        1000,
        DEFAULT_LEASE_SECONDS,
        &clock,
    ) {
        AcquireResult::Acquired(l) => l,
        other => panic!("unexpected {other:?}"),
    };

    // Lease expires; the system reclaims it (generation bumps). This fences out
    // the stale worker.
    let now = 1000 + DEFAULT_LEASE_SECONDS + 1;
    assert!(
        autospec_core::resilience::reclaim_expired(&mut store, &work_id, now)
    );
    let reclaimed = store.load_lease(&work_id).unwrap();
    assert_eq!(reclaimed.fencing_generation, lease_a.fencing_generation + 1);

    // Stale worker A (old generation) must fail to finalize after ownership moved.
    assert!(
        try_finalize(&mut store, &work_id, &attempt_a, &session_a, lease_a.fencing_generation)
            .is_err()
    );
    // The current owner (reclaimed lease, current generation) can finalize.
    assert!(
        try_finalize(
            &mut store,
            &work_id,
            &attempt_a,
            &session_a,
            reclaimed.fencing_generation
        )
        .is_ok()
    );
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
    assert!(!record_delivery(&mut store, &receipt));
    assert!(record_delivery(&mut store, &receipt));
}

#[test]
fn crash_recovery_distinguishes_resume_block_and_retry() {
    let mut store = InMemoryWorkStore::default();
    let work_id = WorkId::new(b"crash");
    store.insert_work(make_work(work_id.clone()));
    let clock = FixedClock(1000);

    // No lease yet: block (work may not be running).
    assert_eq!(reconcile(&store, &work_id, 1000), RecoveryAction::Block);

    // Acquire, then simulate a crashed worker by letting the lease expire.
    let attempt = AttemptId::new(b"a");
    let session = SessionId::new(b"s");
    acquire_lease(
        &mut store,
        &work_id,
        &attempt,
        &session,
        1000,
        DEFAULT_LEASE_SECONDS,
        &clock,
    );
    assert_eq!(reconcile(&store, &work_id, 1100), RecoveryAction::Resume);
    assert_eq!(
        reconcile(&store, &work_id, 1000 + DEFAULT_LEASE_SECONDS + 1),
        RecoveryAction::Retry
    );
}

#[test]
fn unknown_context_usage_never_deadlocks() {
    let cfg = ContextGuardianConfig::default();
    let observation = ContextObservation {
        window_tokens: 0,
        estimated_used_tokens: 0,
        source: EstimateSource::Unknown,
    };
    assert_eq!(
        evaluate_threshold(&cfg, &observation),
        ThresholdVerdict::Unknown
    );
    // Unknown usage must not block a new phase purely for lack of an exact count.
    assert!(may_begin_substantial_phase(ThresholdVerdict::Unknown, false));
}

#[test]
fn source_mutation_during_attention_stream_requires_reconciliation() {
    use std::collections::BTreeMap;
    let mut stream = AttentionStream::new(StreamId::new(b"att"), "find routing impls");
    stream.source_set.push(autospec_core::resilience::SourceRef {
        path: "src/router.rs".to_string(),
        digest: "digest-v1".to_string(),
    });
    stream.apply_chunk(&ChunkOutput {
        chunk_index: 1,
        new_findings: vec![Finding {
            summary: "router dispatch at 42".to_string(),
            evidence_refs: vec!["src/router.rs:42".to_string()],
            contradictions: Vec::new(),
            hypothesis: None,
        }],
        contradictions: Vec::new(),
        unresolved_questions: Vec::new(),
        changed_hypotheses: Vec::new(),
        follow_up_queries: Vec::new(),
        next_cursor: 1,
    });

    // Source unchanged: safe to resume from the durable cursor.
    let mut unchanged = BTreeMap::new();
    unchanged.insert("src/router.rs".to_string(), "digest-v1".to_string());
    let (status, _) = autospec_core::resilience::resume_verdict(&stream, &unchanged);
    assert_eq!(status, autospec_core::resilience::StreamStatus::Active);

    // Source mutated: must reconcile, never silently resume.
    let mut mutated = BTreeMap::new();
    mutated.insert("src/router.rs".to_string(), "digest-v2".to_string());
    let (status, _) = autospec_core::resilience::resume_verdict(&stream, &mutated);
    assert_eq!(
        status,
        autospec_core::resilience::StreamStatus::NeedsReconciliation
    );
}
