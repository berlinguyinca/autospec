//! Adaptive Agent Runtime (AAR).
//!
//! AAR turns a work item into an execution policy:
//!
//! ```text
//! task -> classify -> retrieve minimum context -> choose topology
//!      -> choose model -> choose reasoning budget -> execute
//!      -> measure -> evaluate -> learn
//! ```
//!
//! The division of responsibility is deliberate and load-bearing. AutoSpec
//! decides what intelligence is required; a harness (Pi first) executes the
//! agent loop; InferWeave finds eligible inference capacity; the engine runs
//! the model; AAR measures the outcome and improves the next decision. Nothing
//! in this module is Pi- or Qwen-specific: adapters live at the edges.
//!
//! Every module here is pure. Callers perform I/O with the plans these
//! functions return, which keeps policy testable without a model, a node, or a
//! worktree.

pub mod capsule;
pub mod classify;
pub mod context;
pub mod dashboard;
pub mod dispatch_fit;
pub mod escalation;
pub mod guards;
pub mod inferweave;
pub mod memory;
pub mod outcome;
pub mod pi;
pub mod policy;
pub mod profile;
pub mod reasoning;
pub mod result_target;
pub mod telemetry;
pub mod topology;

pub use capsule::{RolePolicy, TaskCapsule};
pub use classify::{
    classify, Capability, ClassificationInput, Complexity, Risk, TaskClass, TaskClassification,
};
pub use context::{
    check_context_fit, context_policy_for, CacheFriendlyPrompt, ContextPolicy, ContextSegment,
    PromptBlock, RetrievalStrategy,
};
pub use dashboard::{
    advise, elapsed_ratio, percentile_nearest_rank, summarize_history, AdviceConfig, AdviceSource,
    HistorySummary, IssueSample, LiveWorkItem, Liveness, ProfileSample, RoutingAdvice,
    DEFAULT_LIVENESS_THRESHOLD_MS, DEFAULT_LOCKED_MODEL_FAMILY, DEFAULT_MIN_SAMPLES,
    LIVE_CARD_FIELDS, MIN_SUCCESS_RATE,
};
pub use dispatch_fit::{
    dispatch, parse_context_class, redispatch, status_json_with_grant, ContextClass, ContextGrant,
    DispatchGrant, DispatchVerdict, Endpoint, NO_ENDPOINT_LARGE_ENOUGH,
};
pub use escalation::{
    next_attempt, Attempt, EscalationContext, EscalationOutcome, EscalationPolicy, EscalationStep,
    QuotaState,
};
pub use guards::{
    evaluate_stop, EditAction, EditGuard, EditGuardViolation, EditPolicy, ExecutionProgress,
    StepEvent, StopPolicy, StopReason, ThrashDetector, ThrashFinding, ThrashResponse, ThrashSignal,
};
pub use inferweave::{
    route, CapabilityRequest, LatencyPriority, NodeOffer, RoutingDecision, SessionSeat,
};
pub use memory::{MemoryEntry, MemoryFile, WorktreeMemory};
pub use outcome::{
    apply_policy_override, recommend, score_outcome, ExecutionOutcome, HardPolicy, OutcomeScore,
    ProfileStats, QualityThreshold, Recommendation,
};
pub use pi::{
    build_pi_argv, fold_events, parse_pi_event, pi_tools_for, PiEvent, PiSessionSpec, WORKING_RULES,
};
pub use policy::{
    decide, decide_for_classification, DecisionRecord, ExecutionPolicy, PolicyConfig,
    PolicyDecision,
};
pub use profile::{
    CapabilityScores, ModelProfile, ModelProfileRegistry, ModelRequirements, ProfileObservations,
};
pub use reasoning::{
    select_reasoning, ReasoningBudget, ReasoningHistory, ReasoningLimits, ReasoningSelection,
    SamplingProfile, SamplingRegistry,
};
pub use result_target::{
    evaluate_result_target, median_wall_ms, GateStatus, GateVerdict, ResultTarget,
};
pub use telemetry::{ExecutionTelemetry, FailureCategory, ReviewOutcome};
pub use topology::{
    enforce_separation, select_topology, AgentRole, AgentTopology, Handoff, RoleAssignment,
    SeparationPolicy, SeparationVerdict,
};
