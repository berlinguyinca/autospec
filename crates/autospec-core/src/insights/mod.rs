//! Continuous Improvement Engine subsystem.
//!
//! Spec: `docs/specs/2026-09-08-continuous-improvement-engine.md`.
//!
//! The engine is a loop over session telemetry: ingest raw harness sessions,
//! normalize them, detect patterns, find findings, propose improvements,
//! evaluate them, and verify them after deployment. This module owns the
//! stages that have landed so far; ingestion, storage, enrichment,
//! redaction and the CLI surfaces live in their own issues.
//!
//! The analytics here are recommendation-only (spec §18): nothing in this
//! module is applied to live dispatch or written to any routing policy file.

pub mod config;
pub mod correlate;
pub mod events;
pub mod models;
pub mod proposals;
pub mod summarize;

pub use models::{
    model_performance, recommend, ModelPerformance, RecommendationConfidence,
    RoutingRecommendation, Window, TASK_DIMENSIONS,
};
