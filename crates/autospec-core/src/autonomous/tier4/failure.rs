use crate::autonomous::waterfall::FunnelCounts;

use super::model::{
    bounded_text, zero_funnel, Tier4Deduplication, Tier4GeneratedCandidates, Tier4SourceEnvelope,
    Tier4SourcePolicy, Tier4Stage, Tier4VerifierVerdicts, DETAIL_LIMIT,
};
use super::Tier4EvidenceDocuments;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier4FailureCode {
    MissingStageResult,
    InvalidSourcePolicy,
    InvalidSourceCoverage,
    InvalidSourceEnvelope,
    InvalidSourceFact,
    InvalidGeneratedCandidates,
    InvalidCandidate,
    DuplicateConflict,
    InvalidVerdictCoverage,
    InvalidRoiPolicy,
    InvalidRanking,
    CountOverflow,
}

impl Tier4FailureCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MissingStageResult => "missing_stage_result",
            Self::InvalidSourcePolicy => "invalid_source_policy",
            Self::InvalidSourceCoverage => "invalid_source_coverage",
            Self::InvalidSourceEnvelope => "invalid_source_envelope",
            Self::InvalidSourceFact => "invalid_source_fact",
            Self::InvalidGeneratedCandidates => "invalid_generated_candidates",
            Self::InvalidCandidate => "invalid_candidate",
            Self::DuplicateConflict => "duplicate_conflict",
            Self::InvalidVerdictCoverage => "invalid_verdict_coverage",
            Self::InvalidRoiPolicy => "invalid_roi_policy",
            Self::InvalidRanking => "invalid_ranking",
            Self::CountOverflow => "count_overflow",
        }
    }

    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "missing_stage_result" => Ok(Self::MissingStageResult),
            "invalid_source_policy" => Ok(Self::InvalidSourcePolicy),
            "invalid_source_coverage" => Ok(Self::InvalidSourceCoverage),
            "invalid_source_envelope" => Ok(Self::InvalidSourceEnvelope),
            "invalid_source_fact" => Ok(Self::InvalidSourceFact),
            "invalid_generated_candidates" => Ok(Self::InvalidGeneratedCandidates),
            "invalid_candidate" => Ok(Self::InvalidCandidate),
            "duplicate_conflict" => Ok(Self::DuplicateConflict),
            "invalid_verdict_coverage" => Ok(Self::InvalidVerdictCoverage),
            "invalid_roi_policy" => Ok(Self::InvalidRoiPolicy),
            "invalid_ranking" => Ok(Self::InvalidRanking),
            "count_overflow" => Ok(Self::CountOverflow),
            _ => Err(format!("unknown Tier 4 failure code: {value}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tier4Failure {
    stage: Tier4Stage,
    code: Tier4FailureCode,
    detail: String,
    partial: Box<Tier4PartialEvidence>,
    sealed: bool,
}

impl Tier4Failure {
    pub fn new(
        stage: Tier4Stage,
        code: Tier4FailureCode,
        detail: impl Into<String>,
    ) -> Result<Self, String> {
        let detail = detail.into();
        if !bounded_text(&detail, DETAIL_LIMIT) {
            return Err("Tier 4 failure detail must be nonempty and bounded".to_string());
        }
        Ok(Self::initial(stage, code, detail))
    }

    pub fn stage(&self) -> Tier4Stage {
        self.stage
    }

    pub fn code(&self) -> Tier4FailureCode {
        self.code
    }

    pub fn detail(&self) -> &str {
        &self.detail
    }

    pub fn status_reason(&self) -> String {
        format!("tier4_{}_{}", self.stage.as_str(), self.code.as_str())
    }

    pub fn partial_evidence(&self) -> &Tier4PartialEvidence {
        &self.partial
    }

    pub fn documents(&self) -> Option<Tier4EvidenceDocuments<'_>> {
        self.sealed.then_some(Tier4EvidenceDocuments::failure(self))
    }

    pub(super) fn initial(stage: Tier4Stage, code: Tier4FailureCode, detail: String) -> Self {
        Self {
            stage,
            code,
            detail,
            partial: Box::new(Tier4PartialEvidence::none()),
            sealed: false,
        }
    }

    pub(super) fn with_partial(mut self, partial: Tier4PartialEvidence) -> Self {
        self.partial = Box::new(partial);
        self
    }

    pub(super) fn rebind(mut self, stage: Tier4Stage) -> Self {
        self.stage = stage;
        self
    }

    pub(super) fn seal(mut self) -> Self {
        self.sealed = true;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tier4PartialEvidence {
    source_policy: Option<Tier4SourcePolicy>,
    sources: Option<Vec<Tier4SourceEnvelope>>,
    generated: Option<Tier4GeneratedCandidates>,
    deduplication: Option<Tier4Deduplication>,
    verification: Option<Tier4VerifierVerdicts>,
    funnel: FunnelCounts,
}

impl Tier4PartialEvidence {
    pub fn has_source_policy(&self) -> bool {
        self.source_policy.is_some()
    }

    pub fn has_sources(&self) -> bool {
        self.sources.is_some()
    }

    pub fn has_generated(&self) -> bool {
        self.generated.is_some()
    }

    pub fn has_deduplication(&self) -> bool {
        self.deduplication.is_some()
    }

    pub fn has_verification(&self) -> bool {
        self.verification.is_some()
    }

    pub fn funnel(&self) -> &FunnelCounts {
        &self.funnel
    }

    pub(super) fn source_policy(&self) -> Option<&Tier4SourcePolicy> {
        self.source_policy.as_ref()
    }

    pub(super) fn sources(&self) -> Option<&[Tier4SourceEnvelope]> {
        self.sources.as_deref()
    }

    pub(super) fn generated(&self) -> Option<&Tier4GeneratedCandidates> {
        self.generated.as_ref()
    }

    pub(super) fn deduplication(&self) -> Option<&Tier4Deduplication> {
        self.deduplication.as_ref()
    }

    pub(super) fn verification(&self) -> Option<&Tier4VerifierVerdicts> {
        self.verification.as_ref()
    }

    pub(super) fn none() -> Self {
        Self {
            source_policy: None,
            sources: None,
            generated: None,
            deduplication: None,
            verification: None,
            funnel: zero_funnel(),
        }
    }

    pub(super) fn after_source_policy(
        source_policy: Tier4SourcePolicy,
        funnel: FunnelCounts,
    ) -> Self {
        Self {
            source_policy: Some(source_policy),
            sources: None,
            generated: None,
            deduplication: None,
            verification: None,
            funnel,
        }
    }

    pub(super) fn after_sources(
        source_policy: Tier4SourcePolicy,
        sources: Vec<Tier4SourceEnvelope>,
        funnel: FunnelCounts,
    ) -> Self {
        Self {
            source_policy: Some(source_policy),
            sources: Some(sources),
            generated: None,
            deduplication: None,
            verification: None,
            funnel,
        }
    }

    pub(super) fn after_generated(
        source_policy: Tier4SourcePolicy,
        sources: Vec<Tier4SourceEnvelope>,
        generated: Tier4GeneratedCandidates,
        funnel: FunnelCounts,
    ) -> Self {
        Self {
            source_policy: Some(source_policy),
            sources: Some(sources),
            generated: Some(generated),
            deduplication: None,
            verification: None,
            funnel,
        }
    }

    pub(super) fn after_deduplication(
        source_policy: Tier4SourcePolicy,
        sources: Vec<Tier4SourceEnvelope>,
        generated: Tier4GeneratedCandidates,
        deduplication: Tier4Deduplication,
        funnel: FunnelCounts,
    ) -> Self {
        Self {
            source_policy: Some(source_policy),
            sources: Some(sources),
            generated: Some(generated),
            deduplication: Some(deduplication),
            verification: None,
            funnel,
        }
    }

    pub(super) fn after_verification(
        source_policy: Tier4SourcePolicy,
        sources: Vec<Tier4SourceEnvelope>,
        generated: Tier4GeneratedCandidates,
        deduplication: Tier4Deduplication,
        verification: Tier4VerifierVerdicts,
        funnel: FunnelCounts,
    ) -> Self {
        Self {
            source_policy: Some(source_policy),
            sources: Some(sources),
            generated: Some(generated),
            deduplication: Some(deduplication),
            verification: Some(verification),
            funnel,
        }
    }
}
