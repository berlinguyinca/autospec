use std::collections::BTreeSet;

use crate::autonomous::no_work::IDEATION_CANDIDATE_LIMIT;
use crate::autonomous::waterfall::FunnelCounts;

use super::{evidence, partial::Tier2PartialEvidence, DISABLED_REASON};

pub use super::evidence::{Tier2ExclusionReport, Tier2PollutionCode, Tier2PollutionFinding};

pub use crate::explore::specialists::StrictCollectorEvidence;

pub const TIER2_SCHEMA: u64 = 1;
pub const TIER2_RANK_LIMIT: u64 = IDEATION_CANDIDATE_LIMIT;
pub const TIER2_NORMALIZATION_VERSION: u64 = 1;
pub(super) const FIELD_SCALAR_LIMIT: usize = 200;
pub(super) const REASON_SCALAR_LIMIT: usize = 240;
const DEFAULT_EXCLUDED_COMPONENTS: &[&str] = &[
    ".cache",
    ".git",
    ".next",
    "build",
    "coverage",
    "dist",
    "node_modules",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tier2ExclusionPolicy {
    excluded_components: BTreeSet<String>,
    digest: String,
}

impl Tier2ExclusionPolicy {
    pub fn with_repository_additions<I, S>(additions: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut components = DEFAULT_EXCLUDED_COMPONENTS
            .iter()
            .map(|value| value.to_string())
            .collect::<Vec<_>>();
        components.extend(
            additions
                .into_iter()
                .map(|value| value.as_ref().to_string()),
        );
        Self::from_components(components)
    }

    pub fn digest(&self) -> &str {
        &self.digest
    }

    pub fn excludes_component(&self, component: &str) -> bool {
        self.excluded_components.contains(component)
    }

    pub(super) fn matching_component<'a>(&self, path: &'a str) -> Option<&'a str> {
        path.split('/')
            .find(|component| self.excludes_component(component))
    }

    fn from_components<I, S>(components: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut excluded_components = BTreeSet::new();
        for component in components {
            let component = component.as_ref();
            if !evidence::valid_exclusion_component(component) {
                return Err("Tier 2 exclusion must be one bounded directory component".into());
            }
            excluded_components.insert(component.to_string());
        }
        let digest = evidence::exclusion_policy_digest(&excluded_components);
        Ok(Self {
            excluded_components,
            digest,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::large_enum_variant)] // Keeps the plan's explicit typed stage-input constructor.
pub enum Tier2Input {
    DisabledByCheckedInPolicy,
    Enabled {
        collector: Tier2StageResult<StrictCollectorEvidence>,
        generator: Tier2StageResult<Tier2GeneratedProposals>,
        verifier: Tier2StageResult<Tier2VerifierVerdicts>,
        roi_policy: Tier2RoiPolicy,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tier2StageResult<T> {
    Complete(T),
    Failed(Tier2Failure),
    Missing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tier2RoiPolicy {
    pub permitted_sources: BTreeSet<Tier2Source>,
}

impl Tier2RoiPolicy {
    pub fn new(permitted_sources: BTreeSet<Tier2Source>) -> Self {
        Self { permitted_sources }
    }

    pub fn v1() -> Self {
        Self::new(BTreeSet::from([Tier2Source::StrictLocalSpecialist]))
    }

    pub fn permits(&self, source: Tier2Source) -> bool {
        self.permitted_sources.contains(&source)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tier2GeneratedProposals {
    pub generator_identity: String,
    pub generator_protocol_version: String,
    pub proposals: Vec<Tier2Proposal>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tier2VerifierVerdicts {
    pub verifier_identity: String,
    pub verifier_protocol_version: String,
    pub verdicts: Vec<Tier2Verification>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tier2Proposal {
    pub stable_key: String,
    pub title: String,
    pub source: Tier2Source,
    pub evidence: Vec<crate::explore::specialists::FileLineEvidence>,
    pub severity: Tier2Severity,
    pub confidence_millis: u16,
    pub complexity: Tier2Complexity,
    pub named_consumer: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier2Source {
    StrictLocalSpecialist,
}

impl Tier2Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::StrictLocalSpecialist => "strict_local_specialist",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier2Severity {
    Critical,
    High,
    Medium,
    Low,
}

impl Tier2Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Critical => "critical",
            Self::High => "high",
            Self::Medium => "medium",
            Self::Low => "low",
        }
    }

    pub fn rank(self) -> u64 {
        match self {
            Self::Critical => 0,
            Self::High => 1,
            Self::Medium => 2,
            Self::Low => 3,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier2Complexity {
    Small,
    Medium,
    Large,
}

impl Tier2Complexity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Small => "small",
            Self::Medium => "medium",
            Self::Large => "large",
        }
    }

    pub fn units(self) -> u64 {
        match self {
            Self::Small => 1,
            Self::Medium => 2,
            Self::Large => 4,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tier2Verification {
    Survived { stable_key: String, reason: String },
    Refuted { stable_key: String, reason: String },
}

impl Tier2Verification {
    pub fn stable_key(&self) -> &str {
        match self {
            Self::Survived { stable_key, .. } | Self::Refuted { stable_key, .. } => stable_key,
        }
    }

    pub fn reason(&self) -> &str {
        match self {
            Self::Survived { reason, .. } | Self::Refuted { reason, .. } => reason,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Survived { .. } => "survived",
            Self::Refuted { .. } => "refuted",
        }
    }

    pub(super) fn survived(&self) -> bool {
        matches!(self, Self::Survived { .. })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier2Stage {
    Collector,
    Generator,
    Deduplicator,
    Verifier,
    RoiRank,
}

impl Tier2Stage {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Collector => "collector",
            Self::Generator => "generator",
            Self::Deduplicator => "deduplicator",
            Self::Verifier => "verifier",
            Self::RoiRank => "roi_rank",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier2FailureCode {
    InvalidRoot,
    PathEscapesRoot,
    ReadDirectory,
    ReadFile,
    InvalidUtf8,
    InvalidCollectorSchema,
    MissingStageResult,
    InvalidProposal,
    DuplicateConflict,
    InvalidVerdictCoverage,
    InvalidRanking,
    CountOverflow,
}

impl Tier2FailureCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InvalidRoot => "invalid_root",
            Self::PathEscapesRoot => "path_escapes_root",
            Self::ReadDirectory => "read_directory",
            Self::ReadFile => "read_file",
            Self::InvalidUtf8 => "invalid_utf8",
            Self::InvalidCollectorSchema => "invalid_collector_schema",
            Self::MissingStageResult => "missing_stage_result",
            Self::InvalidProposal => "invalid_proposal",
            Self::DuplicateConflict => "duplicate_conflict",
            Self::InvalidVerdictCoverage => "invalid_verdict_coverage",
            Self::InvalidRanking => "invalid_ranking",
            Self::CountOverflow => "count_overflow",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tier2Failure {
    pub(super) stage: Tier2Stage,
    pub(super) code: Tier2FailureCode,
    pub(super) detail: String,
    partial: Box<Tier2PartialEvidence>,
    sealed: bool,
    pub(super) exclusion_report: Option<Tier2ExclusionReport>,
}

impl Tier2Failure {
    pub fn new(
        stage: Tier2Stage,
        code: Tier2FailureCode,
        detail: impl Into<String>,
    ) -> Result<Self, String> {
        let detail = detail.into();
        if !bounded_text(&detail, REASON_SCALAR_LIMIT) {
            return Err("Tier 2 failure detail must be nonempty and bounded".to_string());
        }
        Ok(Self::initial(stage, code, detail))
    }

    pub fn status_reason(&self) -> String {
        format!("tier2_{}_{}", self.stage.as_str(), self.code.as_str())
    }

    pub fn stage(&self) -> Tier2Stage {
        self.stage
    }

    pub fn code(&self) -> Tier2FailureCode {
        self.code
    }

    pub fn detail(&self) -> &str {
        &self.detail
    }

    pub fn partial_evidence(&self) -> &Tier2PartialEvidence {
        &self.partial
    }

    pub(super) fn initial(
        stage: Tier2Stage,
        code: Tier2FailureCode,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            stage,
            code,
            detail: detail.into(),
            partial: Box::new(Tier2PartialEvidence::none()),
            sealed: false,
            exclusion_report: None,
        }
    }

    pub(super) fn with_partial(mut self, partial: Tier2PartialEvidence) -> Self {
        self.partial = Box::new(partial);
        self
    }

    pub(super) fn seal(mut self) -> Self {
        self.sealed = true;
        self
    }

    pub(super) fn with_exclusion_report(mut self, report: Tier2ExclusionReport) -> Self {
        self.exclusion_report = Some(report);
        self
    }

    pub(super) fn is_sealed(&self) -> bool {
        self.sealed
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::large_enum_variant)] // Keeps `Complete(Tier2Observation)` direct for callers.
pub enum Tier2Evaluation {
    NotRun(Tier2NotRun),
    Complete(Tier2Observation),
}

impl Tier2Evaluation {
    pub fn observation(&self) -> Option<&Tier2Observation> {
        match self {
            Self::NotRun(_) => None,
            Self::Complete(observation) => Some(observation),
        }
    }

    pub fn not_run_reason(&self) -> Option<&str> {
        match self {
            Self::NotRun(not_run) => Some(&not_run.reason),
            Self::Complete(_) => None,
        }
    }

    pub fn evidence_json(&self) -> String {
        evidence::render_tier2_evaluation_json(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tier2NotRun {
    pub(super) reason: String,
}

impl Tier2NotRun {
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
pub struct Tier2Observation {
    pub(super) collector: StrictCollectorEvidence,
    pub(super) generated: Tier2GeneratedProposals,
    pub(super) deduplication: Tier2Deduplication,
    pub(super) verification: Tier2VerifierVerdicts,
    pub(super) roi: Vec<Tier2RoiDecision>,
    pub(super) ranked: Vec<Tier2RankedProposal>,
    pub(super) funnel: FunnelCounts,
    pub(super) exclusion_report: Tier2ExclusionReport,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tier2RoiDecision {
    pub stable_key: String,
    pub source: Tier2Source,
    pub permitted: bool,
    pub proposal: Tier2Proposal,
    pub score_numerator: u64,
    pub complexity_units: u64,
    pub score_quotient: u64,
    pub severity_rank: u64,
}

impl Tier2Observation {
    pub fn funnel(&self) -> &FunnelCounts {
        &self.funnel
    }

    pub fn evidence_json(&self) -> String {
        Tier2Evaluation::Complete(self.clone()).evidence_json()
    }

    pub fn exclusion_report(&self) -> &Tier2ExclusionReport {
        &self.exclusion_report
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tier2Deduplication {
    pub groups: Vec<Tier2DeduplicationGroup>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tier2DeduplicationGroup {
    pub key: String,
    pub candidate_keys: Vec<String>,
    pub winner_key: String,
    pub suppressed_keys: Vec<String>,
    pub score_quotients: Vec<Tier2CandidateScore>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tier2CandidateScore {
    pub stable_key: String,
    pub score_quotient: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tier2RankedProposal {
    pub proposal: Tier2Proposal,
    pub score_numerator: u64,
    pub complexity_units: u64,
    pub score_quotient: u64,
    pub severity_rank: u64,
    pub stable_key: String,
    pub named_consumer: String,
    pub rank: u64,
}

pub(super) fn bounded_text(value: &str, limit: usize) -> bool {
    !value.trim().is_empty() && value.chars().count() <= limit
}
