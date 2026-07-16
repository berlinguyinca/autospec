use crate::autonomous::config::Tier4SourceDescriptor;
use crate::autonomous::no_work::DryReason;
use crate::autonomous::waterfall::FunnelCounts;

use super::{Tier4EvidenceDocuments, Tier4Failure, DISABLED_REASON};

pub(super) const FIELD_LIMIT: usize = 200;
pub(super) const DETAIL_LIMIT: usize = 240;
pub(super) const MAX_SOURCES: usize = 4;
pub(super) const MAX_SOURCE_FACTS: usize = 128;
pub(super) const MAX_GENERATED_REFERENCES: usize = 512;
pub(super) const ROI_THRESHOLD_MILLIS: u16 = 500;
pub(super) const ROI_SCALE_MILLIS: u16 = 1_000;

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::large_enum_variant)]
pub enum Tier4Input {
    DisabledByCheckedInPolicy,
    Enabled {
        source_policy: Tier4SourcePolicy,
        sources: Vec<Tier4StageResult<Tier4SourceEnvelope>>,
        generated: Tier4StageResult<Tier4GeneratedCandidates>,
        verifier: Tier4StageResult<Tier4VerifierVerdicts>,
        roi_policy: Tier4RoiPolicy,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tier4StageResult<T> {
    Complete(T),
    Failed(Tier4Failure),
    Missing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tier4SourcePolicy {
    pub schema_version: u64,
    pub policy_identity: String,
    pub descriptors: Vec<Tier4SourceDescriptor>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tier4SourceEnvelope {
    pub schema_version: u64,
    pub producer_identity: String,
    pub producer_protocol_version: String,
    pub source_id: String,
    pub byte_length: u32,
    pub body_sha256: String,
    pub facts: Vec<Tier4SourceFact>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tier4SourceFact {
    pub fact_key: String,
    pub fact_type: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tier4GeneratedCandidates {
    pub schema_version: u64,
    pub generator_identity: String,
    pub generator_protocol_version: String,
    pub candidates: Vec<Tier4Candidate>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tier4Candidate {
    pub stable_key: String,
    pub source_id: String,
    pub fact_key: String,
    pub title: String,
    pub rationale: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Tier4CandidateReference {
    pub source_id: String,
    pub fact_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tier4VerifierVerdicts {
    pub schema_version: u64,
    pub verifier_identity: String,
    pub verifier_protocol_version: String,
    pub verdicts: Vec<Tier4Verification>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tier4Verification {
    Accepted {
        stable_key: String,
        roi_millis: u16,
        reason: String,
    },
    Rejected {
        stable_key: String,
        reason: String,
    },
}

impl Tier4Verification {
    pub fn stable_key(&self) -> &str {
        match self {
            Self::Accepted { stable_key, .. } | Self::Rejected { stable_key, .. } => stable_key,
        }
    }

    pub fn reason(&self) -> &str {
        match self {
            Self::Accepted { reason, .. } | Self::Rejected { reason, .. } => reason,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Accepted { .. } => "accepted",
            Self::Rejected { .. } => "rejected",
        }
    }

    pub(super) fn accepted(&self) -> bool {
        matches!(self, Self::Accepted { .. })
    }

    pub(super) fn roi_millis(&self) -> Option<u16> {
        match self {
            Self::Accepted { roi_millis, .. } => Some(*roi_millis),
            Self::Rejected { .. } => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tier4RoiPolicy {
    pub threshold_millis: u16,
    pub scale_millis: u16,
}

impl Tier4RoiPolicy {
    pub fn v1() -> Self {
        Self {
            threshold_millis: ROI_THRESHOLD_MILLIS,
            scale_millis: ROI_SCALE_MILLIS,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier4Stage {
    SourcePolicy,
    Sources,
    Generator,
    Deduplicator,
    Verifier,
    RoiRank,
}

impl Tier4Stage {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SourcePolicy => "source_policy",
            Self::Sources => "sources",
            Self::Generator => "generator",
            Self::Deduplicator => "deduplicator",
            Self::Verifier => "verifier",
            Self::RoiRank => "roi_rank",
        }
    }

    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "source_policy" => Ok(Self::SourcePolicy),
            "sources" => Ok(Self::Sources),
            "generator" => Ok(Self::Generator),
            "deduplicator" => Ok(Self::Deduplicator),
            "verifier" => Ok(Self::Verifier),
            "roi_rank" => Ok(Self::RoiRank),
            _ => Err(format!("unknown Tier 4 stage: {value}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tier4Deduplication {
    pub groups: Vec<Tier4DeduplicationGroup>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tier4DeduplicationGroup {
    pub stable_key: String,
    pub title: String,
    pub rationale: String,
    pub references: Vec<Tier4CandidateReference>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tier4RoiDecision {
    pub stable_key: String,
    pub verified: bool,
    pub roi_millis: Option<u16>,
    pub permitted: bool,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tier4RankedCandidate {
    pub rank: u64,
    pub stable_key: String,
    pub roi_millis: u16,
    pub title: String,
    pub rationale: String,
    pub references: Vec<Tier4CandidateReference>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tier4Terminal {
    Exhausted { reason: DryReason },
    Produced { count: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::large_enum_variant)] // Keeps `Complete(Tier4Observation)` direct for callers.
pub enum Tier4Evaluation {
    NotRun(Tier4NotRun),
    Complete(Tier4Observation),
}

impl Tier4Evaluation {
    pub fn observation(&self) -> Option<&Tier4Observation> {
        match self {
            Self::NotRun(_) => None,
            Self::Complete(observation) => Some(observation),
        }
    }

    pub fn not_run_reason(&self) -> Option<&str> {
        match self {
            Self::NotRun(not_run) => Some(not_run.reason()),
            Self::Complete(_) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tier4NotRun {
    reason: String,
}

impl Tier4NotRun {
    pub fn reason(&self) -> &str {
        &self.reason
    }

    pub(super) fn disabled() -> Self {
        Self {
            reason: DISABLED_REASON.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tier4Observation {
    pub(super) source_policy: Tier4SourcePolicy,
    pub(super) sources: Vec<Tier4SourceEnvelope>,
    pub(super) generated: Tier4GeneratedCandidates,
    pub(super) deduplication: Tier4Deduplication,
    pub(super) verification: Tier4VerifierVerdicts,
    pub(super) roi: Vec<Tier4RoiDecision>,
    pub(super) ranked: Vec<Tier4RankedCandidate>,
    pub(super) funnel: FunnelCounts,
    pub(super) terminal: Tier4Terminal,
}

impl Tier4Observation {
    pub fn source_policy(&self) -> &Tier4SourcePolicy {
        &self.source_policy
    }

    pub fn sources(&self) -> &[Tier4SourceEnvelope] {
        &self.sources
    }

    pub fn generated(&self) -> &Tier4GeneratedCandidates {
        &self.generated
    }

    pub fn deduplication(&self) -> &Tier4Deduplication {
        &self.deduplication
    }

    pub fn verification(&self) -> &Tier4VerifierVerdicts {
        &self.verification
    }

    pub fn roi(&self) -> &[Tier4RoiDecision] {
        &self.roi
    }

    pub fn ranked(&self) -> &[Tier4RankedCandidate] {
        &self.ranked
    }

    pub fn funnel(&self) -> &FunnelCounts {
        &self.funnel
    }

    pub fn terminal(&self) -> &Tier4Terminal {
        &self.terminal
    }

    pub fn terminal_dry_reason(&self) -> Option<DryReason> {
        match self.terminal {
            Tier4Terminal::Exhausted { reason } => Some(reason),
            Tier4Terminal::Produced { .. } => None,
        }
    }

    pub fn documents(&self) -> Tier4EvidenceDocuments<'_> {
        Tier4EvidenceDocuments::observation(self)
    }
}

pub(super) fn bounded_text(value: &str, limit: usize) -> bool {
    !value.trim().is_empty() && value.chars().count() <= limit
}

pub(super) fn zero_funnel() -> FunnelCounts {
    FunnelCounts::new(0, 0, 0, 0, 0).expect("zero funnel counts are valid")
}
