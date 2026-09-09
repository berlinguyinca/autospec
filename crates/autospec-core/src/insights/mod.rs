//! Continuous Improvement Engine subsystem.
//!
//! Spec: `docs/specs/2026-09-08-continuous-improvement-engine.md`.
//!
//! The engine is a loop over session telemetry: ingest raw harness sessions,
//! normalize them, detect patterns, find findings, propose improvements,
//! evaluate them, and verify them after deployment. This module owns the
//! stages that have landed so far; ingestion, storage, enrichment,
//! redaction and the CLI surfaces live in their own issues.

pub mod config;
pub mod proposals;
