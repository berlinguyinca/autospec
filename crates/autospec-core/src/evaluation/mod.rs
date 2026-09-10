//! Versioned learned evaluators, frozen per-slot epochs, protected anchor
//! qualification, and controlled promotion.
//! Design: docs/specs/2026-09-05-evaluator-coevolution-design.md, ADR 0002.
//!
//! Two properties hold across everything in this module and its successors:
//! persisted documents are integer fixed-point (no binary floating point) and
//! everything worth keeping is content-addressed via [`digest`].
pub mod digest;
pub mod epoch;
pub mod error;
pub mod ids;
pub mod qualification;
pub mod record;
pub mod statistics;
pub mod store;

/// Bumped when any persisted `.autospec/evaluation/**` document changes shape.
pub const EVALUATION_SCHEMA_VERSION: u64 = 1;

pub use digest::Digest;
pub use epoch::EvaluatorEpoch;
pub use error::{EvaluationError, EvaluationErrorKind};
pub use ids::{
    AnchorCaseId, AnchorSuiteId, ChallengerTrialId, EpochId, EvaluationId, EvaluatorSlot,
    EvaluatorVersionRef, PromotionId,
};
pub use qualification::Verdict;
pub use record::{
    stale_candidates, ActiveRankingStatus, EvaluationRecord, Independence, RankingTransition,
    RuntimeProvenance,
};
pub use statistics::Ppm;
