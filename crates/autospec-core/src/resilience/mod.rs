//! Resilient Agent Runtime, Durable Work, Shared Memory, and Verified Learning.
//!
//! This module is the control-plane home for the five capabilities in
//! `docs/specs/2026-09-13-autospec-artificium-resilience-memory-learning-spec.md`:
//!
//! 1. **Context Guardian** ([`context_guardian`]) — proactive structured
//!    continuation checkpoints before context exhaustion.
//! 2. **Dynamic Memory Map** ([`memory_map`]) — bounded, task-specific shared
//!    memory retrieval over a narrow provider contract.
//! 3. **Repository Attention Streams** ([`attention_stream`]) — resumable
//!    analysis of corpora larger than a model context.
//! 4. **Durable Agent Work Protocol** ([`work_protocol`]) — the canonical
//!    work/attempt/claim/lease/receipt lifecycle with recovery and fencing.
//! 5. **Verified Engineering Learning** ([`learning`]) — evidence-backed lesson
//!    promotion that can never poison shared memory or weaken policy.
//!
//! Every submodule is pure: it returns verdicts, plans, records and validation
//! results, and the caller performs the persistence I/O. That keeps the
//! deterministic gates authoritative and testable without a harness or a model.
//!
//! Architectural boundaries are preserved: this module defines the *contracts*
//! and *policy*. `autospec-dispatcher` owns dispatch judgement, the orchestrator
//! owns execution/session mechanics, `autospec-db` mirrors telemetry (never a
//! correctness source), and `autospec-gui` reads projections.

pub mod attention_stream;
pub mod context_guardian;
pub mod ids;
pub mod learning;
pub mod memory_map;
pub mod work_protocol;

pub use attention_stream::{
    AttentionStream, ChunkOutput, Finding, MutationVerdict, SourceRef, StreamStatus, resume_verdict,
};
pub use context_guardian::{
    contains_secret_like, evaluate_threshold, may_begin_substantial_phase, validate_checkpoint,
    CheckpointPhase, ContextCheckpoint, ContextGuardianConfig, ContextObservation, EstimateSource,
    ResumePlan, ThresholdVerdict, CONTEXT_CHECKPOINT_SCHEMA,
};
pub use ids::{
    AttemptId, CandidateId, CheckpointId, ClaimId, ExecutionId, IdempotencyKey, ReceiptId,
    SessionId, StreamId, WorkId,
};
pub use learning::{
    is_authoritative, is_unsafe_lesson, promote_verdict, set_status, supersede,
    touches_immutable_policy, Evidence, LessonCandidate, LessonKind, LessonStatus, PromotionVerdict,
    LESSON_CANDIDATE_SCHEMA,
};
pub use memory_map::{
    generate_map, resolve_conflict, MemoryDomainRef, MemoryEntryRef, MemoryKind, MemoryMap,
    MemoryProvider, Suggestion, StaticMemoryProvider, MEMORY_MAP_SCHEMA,
};
pub use work_protocol::{
    acquire_lease, can_transition, heartbeat, reclaim_expired, reconcile, record_delivery,
    transition, try_finalize, AcquireResult, Clock, InMemoryWorkStore, Lease, ReceiptStage,
    RecoveryAction, WorkItem, WorkReceipt, WorkState, WorkStateNonHappy, WorkStore,
    WORK_RECEIPT_SCHEMA,
};

/// Stable additive event names for lifecycle changes. Telemetry mirrors these;
/// correctness never depends on them being delivered.
pub const EVENTS: &[&str] = &[
    "context.threshold_reached",
    "checkpoint.requested",
    "checkpoint.persisted",
    "checkpoint.acknowledged",
    "execution.resumed",
    "work.assigned",
    "work.delivered",
    "claim.acquired",
    "claim.renewed",
    "claim.expired",
    "attempt.started",
    "attempt.completed",
    "validation.completed",
    "review.completed",
    "attention.started",
    "attention.progressed",
    "attention.completed",
    "memory.map_generated",
    "memory.retrieved",
    "lesson.candidate_created",
    "lesson.validated",
    "lesson.promoted",
    "lesson.rejected",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_names_are_stable_and_unique() {
        let mut seen = std::collections::HashSet::new();
        for e in EVENTS {
            assert!(seen.insert(*e), "duplicate event: {}", e);
        }
        assert!(EVENTS.contains(&"checkpoint.persisted"));
        assert!(EVENTS.contains(&"claim.acquired"));
        assert!(EVENTS.contains(&"lesson.promoted"));
    }
}
