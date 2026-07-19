mod candidate;
mod evaluate;
mod evidence;
mod failure;
mod model;

pub use evaluate::evaluate_tier4;
pub use evidence::Tier4EvidenceDocuments;
pub use failure::Tier4FailureCode;
pub use failure::{Tier4Failure, Tier4PartialEvidence};
pub use model::{
    Tier4Candidate, Tier4CandidateReference, Tier4Deduplication, Tier4DeduplicationGroup,
    Tier4Evaluation, Tier4GeneratedCandidates, Tier4Input, Tier4NotRun, Tier4Observation,
    Tier4RankedCandidate, Tier4RoiDecision, Tier4RoiPolicy, Tier4SourceEnvelope, Tier4SourceFact,
    Tier4SourcePolicy, Tier4Stage, Tier4StageResult, Tier4Terminal, Tier4Verification,
    Tier4VerifierVerdicts,
};

pub const DISABLED_REASON: &str = "tier4_external_discovery_disabled_by_checked_in_policy";
pub const TIER4_SCHEMA: u64 = 1;
pub const TIER4_RANK_LIMIT: u64 = 10;
