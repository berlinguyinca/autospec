//! Resource lifecycle model and observers (spec
//! `docs/specs/2026-08-16-resource-lifecycle-cleanup-design.md`).
//!
//! Phase 1 of the migration/rollout strategy (spec §47) is "Resource model,
//! SQLite ledger, resource inspection" — this module owns the model plus the
//! **read-only** observers that turn Docker objects, AutoSpec child processes,
//! and local git branches/worktrees into `ObservedResource` values (`docker`,
//! `process`, `git`). It still owns none of the rest: no persistence, no
//! SQLite ledger, no CLI, no deletion, and no `ResourceHandler` (§38) live
//! here.
//!
//! Observers never mutate: the Docker observer issues only read-only
//! list/inspect subcommands (spec §47), the git observer issues only
//! read-only argument-vector subcommands (spec §36), and anything an observer
//! cannot attribute to exactly one AutoSpec run is
//! `OwnershipClass::External` and must never be deleted (spec §36).

pub mod db;
pub mod docker;
pub mod dry_run;
pub mod git;
pub mod ledger;
pub mod model;
pub mod process;

pub use docker::observe_docker;
pub use dry_run::{DryRunEntry, DryRunPlan, ProposedAction, ResourceTypeTotals};
pub use git::{observe_branches, observe_worktrees};
pub use ledger::ResourceLedger;
pub use model::{ManagedResource, ObservedResource, OwnershipClass, ResourceState, ResourceType};
pub use process::observe_processes;
