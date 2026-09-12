pub mod acceptance_gate;
pub mod backlog;
pub mod closure;
pub mod conversion_claim;
pub mod dispatch_base;
pub mod endpoint;
pub mod executor;
pub mod fast_lane;
pub mod gate;
pub mod gate_reuse;
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
pub mod status_triage;
pub mod test_diff;
pub mod verification;
pub mod work;
pub mod yaml_config;

pub use conversion_claim::{in_flight, ConversionClaim, ConversionClaimError};
pub use endpoint::{
    classify_status, classify_watchdog_termination, plan_agent_budget, startup_limits_line,
    AttemptOutcome, EndpointPool, Grant, PoolError, QueueState, ReResolution, StatusReason, Step,
    WatchdogTermination, Worker, WorkerOccupancy, WorkerState,
};
pub use executor::{
    ExecutionStatus, Executor, ExecutorError, ExecutorRegistry, ExecutorRequest, ExecutorResult,
    FailureClass, Role,
};
pub use fast_lane::{
    FastLanePolicy, FastLanePolicyError, GateSet, ImprovementError, ImprovementLedger,
    ImprovementRate, Schedule, Work, WorkClass, FAST_LANE_CONFIG_PATH,
};
pub use patch_apply::{
    captured_error, classify_apply, rejected_binaries, stage_guard, ApplyOutcome,
    ConflictObservation, StageVerdict,
};
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
    diff_test_failures, isolation_reverify_args, resolve_reverification, DiffVerdict,
    FlakyQuarantine, MissingReobservation, QuarantineMark, Reobservation, ReverificationOutcome,
    ReverificationResolution, TestFailureDiff, DEFAULT_CHURN_THRESHOLD, ISOLATION_RUNS,
};
pub use work::ProducedWork;
