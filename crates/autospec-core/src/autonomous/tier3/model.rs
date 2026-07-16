use crate::autonomous::waterfall::FunnelCounts;

use super::{evidence::Tier3EvidenceDocuments, DISABLED_REASON};

pub const TIER3_SCHEMA: u64 = 1;
pub const TIER3_RANK_LIMIT: u64 = 10;
pub(super) const FIELD_LIMIT: usize = 200;
pub(super) const DETAIL_LIMIT: usize = 240;

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::large_enum_variant)]
pub enum Tier3Input {
    DisabledByCheckedInPolicy,
    Enabled {
        architecture: Tier3StageResult<Tier3AdapterEvidence>,
        coverage: Tier3StageResult<Tier3AdapterEvidence>,
        debt: Tier3StageResult<Tier3AdapterEvidence>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tier3StageResult<T> {
    Complete(T),
    Failed(Tier3Failure),
    Missing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tier3AdapterEvidence {
    pub schema_version: u64,
    pub adapter_version: String,
    pub rule_version: String,
    pub findings: Vec<Tier3Finding>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tier3Finding {
    pub kind: Tier3FindingKind,
    pub severity: Tier3Severity,
    pub rule_id: String,
    pub path: String,
    pub line: u64,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier3FindingKind {
    Architecture,
    Coverage,
    Debt,
}

impl Tier3FindingKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Architecture => "architecture",
            Self::Coverage => "coverage",
            Self::Debt => "debt",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier3Severity {
    Critical,
    High,
    Medium,
    Low,
}

impl Tier3Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Critical => "critical",
            Self::High => "high",
            Self::Medium => "medium",
            Self::Low => "low",
        }
    }

    pub(super) fn rank(self) -> u64 {
        match self {
            Self::Critical => 0,
            Self::High => 1,
            Self::Medium => 2,
            Self::Low => 3,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier3Stage {
    Architecture,
    Coverage,
    Debt,
    Ranking,
}

impl Tier3Stage {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Architecture => "architecture",
            Self::Coverage => "coverage",
            Self::Debt => "debt",
            Self::Ranking => "ranking",
        }
    }

    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "architecture" => Ok(Self::Architecture),
            "coverage" => Ok(Self::Coverage),
            "debt" => Ok(Self::Debt),
            "ranking" => Ok(Self::Ranking),
            _ => Err(format!("unknown Tier 3 stage: {value}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier3FailureCode {
    MissingStageResult,
    InvalidAdapterEvidence,
    InvalidFinding,
    WrongFindingKind,
    NonCanonicalOrder,
    DuplicateConflict,
    InvalidRanking,
    CountOverflow,
}

impl Tier3FailureCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MissingStageResult => "missing_stage_result",
            Self::InvalidAdapterEvidence => "invalid_adapter_evidence",
            Self::InvalidFinding => "invalid_finding",
            Self::WrongFindingKind => "wrong_finding_kind",
            Self::NonCanonicalOrder => "noncanonical_order",
            Self::DuplicateConflict => "duplicate_conflict",
            Self::InvalidRanking => "invalid_ranking",
            Self::CountOverflow => "count_overflow",
        }
    }

    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "missing_stage_result" => Ok(Self::MissingStageResult),
            "invalid_adapter_evidence" => Ok(Self::InvalidAdapterEvidence),
            "invalid_finding" => Ok(Self::InvalidFinding),
            "wrong_finding_kind" => Ok(Self::WrongFindingKind),
            "noncanonical_order" => Ok(Self::NonCanonicalOrder),
            "duplicate_conflict" => Ok(Self::DuplicateConflict),
            "invalid_ranking" => Ok(Self::InvalidRanking),
            "count_overflow" => Ok(Self::CountOverflow),
            _ => Err(format!("unknown Tier 3 failure code: {value}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tier3Failure {
    stage: Tier3Stage,
    code: Tier3FailureCode,
    detail: String,
    partial: Box<Tier3PartialEvidence>,
    sealed: bool,
}

impl Tier3Failure {
    pub fn new(
        stage: Tier3Stage,
        code: Tier3FailureCode,
        detail: impl Into<String>,
    ) -> Result<Self, String> {
        let detail = detail.into();
        if !bounded_text(&detail, DETAIL_LIMIT) {
            return Err("Tier 3 failure detail must be nonempty and bounded".to_string());
        }
        Ok(Self::initial(stage, code, detail))
    }

    pub fn stage(&self) -> Tier3Stage {
        self.stage
    }

    pub fn code(&self) -> Tier3FailureCode {
        self.code
    }

    pub fn detail(&self) -> &str {
        &self.detail
    }

    pub fn status_reason(&self) -> String {
        format!("tier3_{}_{}", self.stage.as_str(), self.code.as_str())
    }

    pub fn parse_status_reason(value: &str) -> Result<(Tier3Stage, Tier3FailureCode), String> {
        let value = value
            .strip_prefix("tier3_")
            .ok_or_else(|| "Tier 3 status reason must start with tier3_".to_string())?;
        let (stage, code) = value
            .split_once('_')
            .ok_or_else(|| "Tier 3 status reason must include stage and code".to_string())?;
        let stage = Tier3Stage::parse(stage)?;
        let code = Tier3FailureCode::parse(code)?;
        if format!("tier3_{}_{}", stage.as_str(), code.as_str()) == format!("tier3_{value}") {
            Ok((stage, code))
        } else {
            Err("Tier 3 status reason is not canonical".to_string())
        }
    }

    pub fn partial_evidence(&self) -> &Tier3PartialEvidence {
        &self.partial
    }

    pub fn documents(&self) -> Option<Tier3EvidenceDocuments<'_>> {
        self.sealed.then_some(Tier3EvidenceDocuments::failure(self))
    }

    pub(super) fn initial(stage: Tier3Stage, code: Tier3FailureCode, detail: String) -> Self {
        Self {
            stage,
            code,
            detail,
            partial: Box::new(Tier3PartialEvidence::none()),
            sealed: false,
        }
    }

    pub(super) fn with_partial(mut self, partial: Tier3PartialEvidence) -> Self {
        self.partial = Box::new(partial);
        self
    }

    pub(super) fn rebind(mut self, stage: Tier3Stage) -> Self {
        self.stage = stage;
        self
    }

    pub(super) fn seal(mut self) -> Self {
        self.sealed = true;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tier3PartialEvidence {
    architecture: Option<Tier3AdapterEvidence>,
    coverage: Option<Tier3AdapterEvidence>,
    debt: Option<Tier3AdapterEvidence>,
    funnel: FunnelCounts,
}

impl Tier3PartialEvidence {
    pub fn has_architecture(&self) -> bool {
        self.architecture.is_some()
    }

    pub fn has_coverage(&self) -> bool {
        self.coverage.is_some()
    }

    pub fn has_debt(&self) -> bool {
        self.debt.is_some()
    }

    pub fn funnel(&self) -> &FunnelCounts {
        &self.funnel
    }

    pub(super) fn architecture(&self) -> Option<&Tier3AdapterEvidence> {
        self.architecture.as_ref()
    }

    pub(super) fn coverage(&self) -> Option<&Tier3AdapterEvidence> {
        self.coverage.as_ref()
    }

    pub(super) fn debt(&self) -> Option<&Tier3AdapterEvidence> {
        self.debt.as_ref()
    }

    pub(super) fn none() -> Self {
        Self {
            architecture: None,
            coverage: None,
            debt: None,
            funnel: zero_funnel(),
        }
    }

    pub(super) fn architecture_complete(architecture: Tier3AdapterEvidence) -> Self {
        Self {
            architecture: Some(architecture),
            coverage: None,
            debt: None,
            funnel: zero_funnel(),
        }
    }

    pub(super) fn coverage_complete(
        architecture: Tier3AdapterEvidence,
        coverage: Tier3AdapterEvidence,
    ) -> Self {
        Self {
            architecture: Some(architecture),
            coverage: Some(coverage),
            debt: None,
            funnel: zero_funnel(),
        }
    }

    pub(super) fn complete(
        architecture: Tier3AdapterEvidence,
        coverage: Tier3AdapterEvidence,
        debt: Tier3AdapterEvidence,
        funnel: FunnelCounts,
    ) -> Self {
        Self {
            architecture: Some(architecture),
            coverage: Some(coverage),
            debt: Some(debt),
            funnel,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::large_enum_variant)]
pub enum Tier3Evaluation {
    NotRun(Tier3NotRun),
    Complete(Tier3Observation),
}

impl Tier3Evaluation {
    pub fn observation(&self) -> Option<&Tier3Observation> {
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
pub struct Tier3NotRun {
    reason: String,
}

impl Tier3NotRun {
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
pub struct Tier3Observation {
    pub(super) architecture: Tier3AdapterEvidence,
    pub(super) coverage: Tier3AdapterEvidence,
    pub(super) debt: Tier3AdapterEvidence,
    pub(super) deduplicated: Vec<Tier3Finding>,
    pub(super) ranked: Vec<Tier3Finding>,
    pub(super) funnel: FunnelCounts,
}

impl Tier3Observation {
    pub fn funnel(&self) -> &FunnelCounts {
        &self.funnel
    }

    pub fn ranked(&self) -> &[Tier3Finding] {
        &self.ranked
    }

    pub fn documents(&self) -> Tier3EvidenceDocuments<'_> {
        Tier3EvidenceDocuments::observation(self)
    }
}

pub(super) fn bounded_text(value: &str, limit: usize) -> bool {
    !value.trim().is_empty() && value.chars().count() <= limit
}

fn zero_funnel() -> FunnelCounts {
    FunnelCounts::new(0, 0, 0, 0, 0).expect("zero funnel counts are valid")
}
