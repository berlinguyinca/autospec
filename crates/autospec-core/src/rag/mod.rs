//! Agentic RAG: AutoSpec's evidence and retrieval layer.
//!
//! Implements the Agentic RAG subsystem specification. The subsystem replaces
//! "retrieve top-K chunks, paste into a prompt" with an iterative loop that
//! plans retrieval, chooses among sources, evaluates what came back, reformulates
//! when the evidence falls short, and stops for a stated reason — returning a
//! structured context package with provenance for every item rather than a text
//! blob.
//!
//! # Layout
//!
//! | Module | Specification sections |
//! |---|---|
//! | [`score`] | 10 (integer scores in place of the spec's decimals) |
//! | [`authority`] | 9 — source authority and precedence |
//! | [`scope`] | 15, 46, 47 — revision and worktree awareness |
//! | [`evidence`] | 10, 11, 53 — the Evidence object and provenance |
//! | [`source`] | 8, 50 — source adapters and the registry |
//! | [`budget`] | 39, 40, 41.1 — budgets and stopping rules |
//! | [`policy`] | 7, 20, 22 — role policies and token budgets |
//! | [`query`] | 13, 41.2 — query planning and reformulation |
//! | [`graph`] | 14, 15 — the revision-aware knowledge graph |
//! | [`contradiction`] | 9, 30 — contradiction records |
//! | [`freshness`] | 31 — staleness policy |
//! | [`evaluator`] | 12 — evidence evaluation and sufficiency |
//! | [`injection`] | 29 — trust bands and injection defense |
//! | [`compression`] | 19 — hierarchical compression |
//! | [`context_package`] | 18, 19, 20 — the returned package |
//! | [`cache`] | 25, 41.4 — revision-aware caching |
//! | [`trace`] | 35, 54 — retrieval traces |
//! | [`metrics`] | 37, 38 — counters and context efficiency |
//! | [`routing`] | 23, 24 — InferWeave capability routing |
//! | [`memory`] | 16, 17 — tiered memory and the write policy |
//! | [`config`] | 51 — configuration |
//! | [`coordinator`] | 6, 32, 33, 40 — the agentic loop |
//! | [`baseline`] | 56, 57.15 — fixed top-K, for comparison |
//!
//! # Two decisions worth knowing before reading further
//!
//! **Scores are integers.** The specification writes relevance and confidence
//! as decimals. This workspace bans binary floating point in Rust crates, and a
//! loop that branches on a threshold needs comparisons that are reproducible
//! across hosts, so [`score::Score`] holds permille and renders decimals at the
//! serialization boundary.
//!
//! **The core performs no I/O.** Source adapters retrieve; the caller supplies
//! the clock and the current revision. That keeps the budget, stopping-rule,
//! cache-invalidation and worktree behaviors testable as pure functions, which
//! is what the specification's section 55 evaluation suites need in order to
//! assert anything stable.

pub mod authority;
pub mod baseline;
pub mod budget;
pub mod cache;
pub mod compression;
pub mod config;
pub mod context_package;
pub mod contradiction;
pub mod coordinator;
pub mod evaluator;
pub mod evidence;
pub mod freshness;
pub mod graph;
pub mod injection;
pub mod memory;
pub mod metrics;
pub mod policy;
pub mod query;
pub mod routing;
pub mod score;
pub mod scope;
pub mod source;
pub mod trace;

pub use authority::{AuthorityLadder, SourceAuthority};
pub use baseline::{BaselineOutcome, retrieve_top_k};
pub use budget::{BudgetLedger, BudgetLimit, RetrievalBudget, StopReason};
pub use cache::{CacheClass, CacheEntry, CacheKey, CacheStats, RetrievalCache};
pub use compression::{CompressionLevel, estimate_tokens};
pub use config::RagConfig;
pub use context_package::{ContextPackage, ContextPackageBuilder};
pub use contradiction::{Contradiction, ContradictionSeverity, ContradictionSet};
pub use coordinator::{RetrievalCoordinator, RetrievalOutcome, RetrievalRequest};
pub use evaluator::{
    EvaluationRequest, EvidenceAssessment, EvidenceEvaluator, SufficiencyDecision,
};
pub use evidence::{
    ContentForm, Evidence, EvidenceBuilder, EvidenceCapture, Privacy, QueryProvenance,
    SourceLocation,
};
pub use freshness::{Freshness, FreshnessPolicy, StalenessRule};
pub use graph::{GraphNode, KnowledgeGraph, NodeKind, Relation};
pub use injection::{InjectionRisk, TrustBand};
pub use memory::{MemoryCandidate, MemoryTier, MemoryWritePolicy};
pub use metrics::RetrievalMetrics;
pub use policy::{AgentRole, PolicySet, RetrievalPolicy};
pub use query::{PlannedQuery, QueryPlanner};
pub use routing::{ModelCapabilities, NodeCandidate, RagModelTask, select_node};
pub use score::Score;
pub use scope::{PathState, RetrievalScope};
pub use source::{KnowledgeSource, SearchMode, SearchRequest, SearchResult, SourceKind, SourceRegistry};
pub use trace::{RetrievalTrace, TraceEvent};

/// Schema version of the evidence, context-package and trace wire formats.
///
/// Bumped when a field changes meaning. A stored trace records the version it
/// was written under so a later reader can tell whether it can be interpreted.
pub const RAG_SCHEMA_VERSION: u32 = 1;
