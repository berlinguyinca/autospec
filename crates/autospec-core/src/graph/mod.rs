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
