//! Issue-DAG and spec-order graph models.
//!
//! Not the canonical task graph. The canonical model is the Initiative DAG,
//! `crate::initiative::dag::TaskGraph`. `IssueGraph` and
//! `execution_order` stay as they are; `crate::initiative::compat_dag` builds
//! the canonical graph from either, so new code should consume `TaskGraph`.
//! There is deliberately no deprecation attribute, so existing callers do not
//! gain warnings. See `docs/contracts/initiative-dag-model-mapping.md`.

pub mod issue_dag;
pub mod metadata;
pub mod metrics;
pub mod optimization;
pub mod order;

pub use issue_dag::{DependencyEdge, IssueGraph, PlannedIssue};
pub use metadata::{
    normalize_path, overlaps, ConcurrencyMetadata, DependencyReason, OwnedSurface, Ownership,
};
pub use metrics::{metrics, GraphMetrics};
pub use optimization::{
    compare, should_retry, OptimizationSummary, RetryPolicy, DEFAULT_MAX_PASSES, DEFAULT_THRESHOLD,
};
pub use order::{execution_order, GraphError, GraphErrorKind};
