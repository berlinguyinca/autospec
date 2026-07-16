mod evaluate;
mod evidence;
mod model;

pub use evaluate::evaluate_tier3;
pub use evidence::Tier3EvidenceDocuments;
pub use model::{
    Tier3AdapterEvidence, Tier3Evaluation, Tier3Failure, Tier3FailureCode, Tier3Finding,
    Tier3FindingKind, Tier3Input, Tier3NotRun, Tier3Observation, Tier3PartialEvidence,
    Tier3Severity, Tier3Stage, Tier3StageResult, TIER3_RANK_LIMIT, TIER3_SCHEMA,
};

pub const DISABLED_REASON: &str = "tier3_metadata_disabled_by_checked_in_policy";
