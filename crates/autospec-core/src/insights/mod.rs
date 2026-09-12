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
pub mod context;
pub mod correlate;
pub mod enrich;
pub mod evaluation;
pub mod events;
pub mod ingest;
pub mod metrics;
pub mod models;
pub mod patterns;
pub mod pr;
pub mod proposals;
pub mod quality;
pub mod report;
pub mod summarize;
pub mod tools;

pub use models::{
    model_performance, recommend, ModelPerformance, RecommendationConfidence,
    RoutingRecommendation, Window, TASK_DIMENSIONS,
};
pub use patterns::{detect, transition, DetectConfig, Finding, FindingStatus, Severity};
pub use tools::{
    retirement_candidates, tool_roi, wrapper_candidates, InvocationRow, RetirementCandidate,
    RetirementConfig, RetirementEvidence, ToolRoi, WrapperCandidate,
};
