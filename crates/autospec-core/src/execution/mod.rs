pub mod backlog;
pub mod closure;
pub mod endpoint;
pub mod executor;
pub mod gate;
pub mod gate_scope;
pub mod patch_apply;
pub mod patch_conflicts;
pub mod patch_pipeline;
pub mod publication;
pub mod queue;
mod queue_parser;
mod queue_runtime;
mod queue_storage;
pub mod relocation;
pub mod report;
pub mod result;
pub mod sort_key;
pub mod stage_throughput;
pub mod test_diff;
pub mod verification;
pub mod work;
pub mod yaml_config;

pub use endpoint::{
    classify_status, plan_agent_budget, startup_limits_line, AttemptOutcome, EndpointPool, Grant,
    PoolError, ReResolution, StatusReason, Step, Worker, WorkerState,
};
pub use executor::{
    ExecutionStatus, Executor, ExecutorError, ExecutorRegistry, ExecutorRequest, ExecutorResult,
    FailureClass, Role,
};
pub use patch_apply::{captured_error, classify_apply, ApplyOutcome, ConflictObservation};
pub use patch_conflicts::{
    hold_shape, resolve, HoldShape, Refusal, RefusalKind, ResolveOutcome, ResolvedFile,
    UnverifiedResolution,
};
pub use queue::{
    ExecutionQueue, FailureKind, OneShotIssueSelector, QueueEntry, QueueResultApplication,
    QueueStatus, QueueValidationResult, QueueValidationStatus, SpecDigest,
};
pub use queue_runtime::StagedSpecCheck;
pub use result::{AgentOutcome, IngestedAgentResult};
pub use test_diff::{
    diff_test_failures, resolve_reverification, DiffVerdict, FlakyQuarantine,
    ReverificationOutcome, TestFailureDiff, DEFAULT_CHURN_THRESHOLD,
};
pub use work::ProducedWork;
