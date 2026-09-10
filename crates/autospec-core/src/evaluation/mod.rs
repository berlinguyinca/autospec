//! Versioned learned evaluators, frozen per-slot epochs, protected anchor
//! qualification, and controlled promotion.
//! Design: docs/specs/2026-09-05-evaluator-coevolution-design.md, ADR 0002.
pub mod anchor;
pub mod digest;
pub mod epoch;
pub mod error;
pub mod evaluator;
pub mod ids;
pub mod policy;
pub mod promotion;
pub mod qualification;
pub mod record;
pub mod statistics;
pub mod store;

/// Bumped when any persisted `.autospec/evaluation/**` document changes shape.
pub const EVALUATION_SCHEMA_VERSION: u64 = 1;

pub use error::{EvaluationError, EvaluationErrorKind};
