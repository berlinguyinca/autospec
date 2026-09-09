//! §20/§21 improvement proposal engine: schema, validation, deterministic
//! drafting, §34 storage, §41 conflict detection and the §40 growth-control
//! priority.
//!
//! Spec: `docs/specs/2026-09-08-continuous-improvement-engine.md` §20-§22,
//! §34-§35, §40-§41.
//!
//! Security invariant (§4.5, "No Silent Self-Modification"): a proposal's
//! `patch` is always **text on a draft record** — nothing in this module
//! applies a patch to the working tree, edits AGENTS.md, installs a skill or
//! changes a routing policy. A fresh draft is always `status: draft`, and a
//! model-created proposal is never its own sole reviewer (§25: `review_model`
//! is a separate, later stage).

pub mod conflicts;
pub mod draft;
pub mod schema;

pub use conflicts::{detect_conflicts, Conflict, ConflictKind, EnforcementLevel, Rule, RuleSource};
pub use draft::{draft, ensure_schema, load, store};
pub use schema::{
    EvaluationPlan, Evidence, ExpectedEffect, Proposal, ProposalStatus, ProposalType,
};
