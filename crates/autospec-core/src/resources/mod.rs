//! Resource lifecycle model and observers (spec
//! `docs/specs/2026-08-16-resource-lifecycle-cleanup-design.md`).
//!
//! Phase 1 of the migration/rollout strategy (spec §47) is "Resource model,
//! SQLite ledger, resource inspection" — this module owns the model plus the
//! **read-only** observers that turn Docker objects and AutoSpec child
//! processes into `ObservedResource` values (`docker`, `process`). It still
//! owns none of the rest: no persistence, no SQLite ledger, no CLI, no
//! deletion, and no `ResourceHandler` (§38) live here.
//!
//! Observers never mutate: the Docker observer issues only read-only
//! list/inspect subcommands (spec §47), and anything it (or the process
//! observer) cannot attribute to exactly one AutoSpec run is
//! `OwnershipClass::External` and must never be deleted (spec §36).

pub mod docker;
pub mod model;
pub mod process;

pub use docker::observe_docker;
pub use model::{ManagedResource, ObservedResource, OwnershipClass, ResourceState, ResourceType};
pub use process::observe_processes;
