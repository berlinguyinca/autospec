//! Additive learning-integration contracts (delta design "Shared contracts").
//!
//! This module freezes versioned records for the observational memory, native
//! sessions, and readiness evidence delta
//! (`docs/specs/2026-09-01-observational-memory-native-sessions-readiness-integration-delta-design.md`).
//!
//! The records REUSE the existing AutoSpec authorities instead of introducing
//! parallel ones:
//!
//! - work item / stage / role / worktree / branch / PR / model / provider
//!   identity: the native session lineage identity
//!   (`crate::agent::session::SessionLineage`);
//! - evidence correlation id: the evidence bundle run id
//!   (`crate::evidence::EvidenceBundle`);
//! - outcome correlation: the append-only routing ledger dispatch id and
//!   outcome vocabulary (`scripts/routing-ledger.sh`).
//!
//! It deliberately introduces no second store, ledger, role set, scheduler,
//! executor, project, benchmark, or database authority.
//! `tests/learning_contracts.rs` proves the absence of parallel authorities
//! against the module source.

pub mod contracts;
