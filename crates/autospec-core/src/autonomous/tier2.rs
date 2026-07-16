mod evidence;
mod funnel;
mod funnel_validation;
mod model;
mod partial;

pub use evidence::Tier2EvidenceDocuments;
pub use funnel::evaluate_tier2;
pub use model::{
    StrictCollectorEvidence, Tier2CandidateScore, Tier2Complexity, Tier2Deduplication,
    Tier2DeduplicationGroup, Tier2Evaluation, Tier2Failure, Tier2FailureCode,
    Tier2GeneratedProposals, Tier2Input, Tier2NotRun, Tier2Observation, Tier2Proposal,
    Tier2RankedProposal, Tier2RoiDecision, Tier2RoiPolicy, Tier2Severity, Tier2Source, Tier2Stage,
    Tier2StageResult, Tier2Verification, Tier2VerifierVerdicts, TIER2_NORMALIZATION_VERSION,
    TIER2_RANK_LIMIT, TIER2_SCHEMA,
};
pub use partial::Tier2PartialEvidence;

pub const DISABLED_REASON: &str = "tier2_local_discovery_disabled_by_policy";
