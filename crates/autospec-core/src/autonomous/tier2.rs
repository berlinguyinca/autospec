mod evidence;
mod funnel;
mod funnel_validation;
mod model;

pub use evidence::{
    render_tier2_collector_json, render_tier2_deduplication_json, render_tier2_evaluation_json,
    render_tier2_failure_json, render_tier2_generated_json, render_tier2_roi_rank_json,
    render_tier2_verification_json,
};
pub use funnel::evaluate_tier2;
pub use model::{
    Tier2CandidateScore, Tier2Complexity, Tier2Deduplication, Tier2DeduplicationGroup,
    Tier2Evaluation, Tier2Failure, Tier2FailureCode, Tier2GeneratedProposals, Tier2Input,
    Tier2NotRun, Tier2Observation, Tier2PartialEvidence, Tier2Proposal, Tier2RankedProposal,
    Tier2RoiDecision, Tier2RoiPolicy, Tier2Severity, Tier2Source, Tier2Stage, Tier2StageResult,
    Tier2Verification, Tier2VerifierVerdicts, TIER2_RANK_LIMIT, TIER2_SCHEMA,
};

pub const DISABLED_REASON: &str = "tier2_local_discovery_disabled_by_policy";
