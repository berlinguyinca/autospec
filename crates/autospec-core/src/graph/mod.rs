pub mod metadata;
pub mod optimization;
pub mod order;

pub use metadata::{
    normalize_path, overlaps, ConcurrencyMetadata, DependencyReason, OwnedSurface, Ownership,
};
pub use optimization::{
    compare, should_retry, GraphMetrics, OptimizationSummary, RetryPolicy,
    DEFAULT_MAX_OPTIMIZATION_PASSES, DEFAULT_RETRY_THRESHOLD,
};
pub use order::{execution_order, GraphError, GraphErrorKind};
