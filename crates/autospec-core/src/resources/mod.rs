//! Resource lifecycle model (spec
//! `docs/specs/2026-08-16-resource-lifecycle-cleanup-design.md`).
//!
//! Phase 1 of the migration/rollout strategy (spec §47) is "Resource model,
//! SQLite ledger, resource inspection" — this module owns only the first of
//! those. No persistence, no CLI, no git/Docker inspection, no deletion, and
//! no `ResourceHandler` (§38) live here.

pub mod model;

pub use model::{ManagedResource, ObservedResource, OwnershipClass, ResourceState, ResourceType};
