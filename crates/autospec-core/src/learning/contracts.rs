//! Versioned learning-integration records.
//!
//! Implements the "Shared contracts" section of
//! `docs/specs/2026-09-01-observational-memory-native-sessions-readiness-integration-delta-design.md`:
//! frozen, additive, versioned records for scoped native session identity and
//! lineage, canonical artifact/revision/target-role/target-model identity,
//! observation candidates and promotion results, structured memory, quality
//! provider result/report/policy fingerprint/decision, context/freshness
//! fingerprint, requirement-preservation result, and execution/outcome
//! correlation with a calibration band.
//!
//! Every identifier field REUSES an existing AutoSpec authority instead of
//! inventing a new id space:
//!
//! | field                                                    | reused authority |
//! |----------------------------------------------------------|------------------|
//! | `SessionLineageRef` work/stage/role/worktree/branch/PR/model/provider | native session lineage identity (`src/agent/session.rs` `SessionLineage`) |
//! | `EvidenceRef::run_id`, `OutcomeCorrelation::evidence_run_id` | evidence bundle run id (`src/evidence/mod.rs` `EvidenceBundle`) |
//! | `OutcomeCorrelation::dispatch_id`, `RoutingOutcome`      | append-only routing ledger dispatch id + outcome vocabulary (`scripts/routing-ledger.sh`) |
//!
//! This module deliberately defines no scheduler, executor, project, ledger,
//! benchmark, role, or database type of its own;
//! `tests/learning_contracts.rs` proves that against the module source.
//!
//! Deserialization is strict: every struct uses `deny_unknown_fields` and
//! every enum rejects unknown variant names, so an authority-changing value
//! added later fails closed instead of being silently accepted.

use serde::{Deserialize, Serialize};

/// Schema version of every record in this module. Records are frozen and
/// additive: changing an existing field is a new schema version, never a
/// silent redefinition. A record carrying any other `schema_version` fails
/// [`LearningContractV1::validate`] instead of silently mis-parsing.
pub const LEARNING_CONTRACT_SCHEMA: u32 = 1;

/// Trust hierarchy for memory content (delta design "Security and trust"):
/// user-approved specifications and authenticated directives outrank
/// inferred memory and semantic scores; observations are untrusted data and
/// candidate evidence, never policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryAuthority {
    UserApprovedSpec,
    AuthenticatedDirective,
    RuntimeObservation,
    InferredMemory,
}

impl MemoryAuthority {
    /// Untrusted data / candidate evidence — the only authorities an
    /// observation candidate may ever carry.
    pub fn is_untrusted(self) -> bool {
        matches!(self, Self::RuntimeObservation | Self::InferredMemory)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::UserApprovedSpec => "user_approved_spec",
            Self::AuthenticatedDirective => "authenticated_directive",
            Self::RuntimeObservation => "runtime_observation",
            Self::InferredMemory => "inferred_memory",
        }
    }
}

/// Durable memory lifecycle status (delta design "Durable structured memory
/// delta": challenges, validation, supersession, staleness, archiving).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryStatus {
    Active,
    Challenged,
    Stale,
    Superseded,
    Archived,
    Quarantined,
}

impl MemoryStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Challenged => "challenged",
            Self::Stale => "stale",
            Self::Superseded => "superseded",
            Self::Archived => "archived",
            Self::Quarantined => "quarantined",
        }
    }
}

/// Memory record type. Failed approaches are first-class, per the delta
/// design.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryType {
    Statement,
    FailedApproach,
    Constraint,
    Preference,
}

impl MemoryType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Statement => "statement",
            Self::FailedApproach => "failed_approach",
            Self::Constraint => "constraint",
            Self::Preference => "preference",
        }
    }
}

/// Explicit memory scope. Cross-repository and branch-local memory stay
/// scope-isolated: the scope is a named, serialized field, never inferred
/// and never silently merged across scopes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryScope {
    Repository,
    Branch,
    Worktree,
    CrossRepository,
}

impl MemoryScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Repository => "repository",
            Self::Branch => "branch",
            Self::Worktree => "worktree",
            Self::CrossRepository => "cross_repository",
        }
    }
}

/// Result of AutoSpec-owned normalization/promotion of an observation
/// candidate. Observations never self-promote.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromotionOutcome {
    Promoted,
    Rejected,
    Quarantined,
}

/// Readiness policy fingerprinting modes (delta design "Readiness shadow
/// foundation"): the shadow foundation is disabled/shadow until promotion
/// criteria pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QualityPolicyMode {
    Disabled,
    Shadow,
    Warn,
    Enforce,
}

/// Readiness decision vocabulary, matching the RealWork benchmark corpus
/// artifact classes (ready / repairable / blocked / clarification).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QualityDecision {
    Ready,
    Repairable,
    Blocked,
    Clarification,
}

/// Outcome vocabulary of the single append-only routing ledger
/// (`ALLOWED_OUTCOMES` in `scripts/routing-ledger.sh`). Mirrored here so
/// learning evidence correlates with the existing ledger — no second ledger
/// exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoutingOutcome {
    Pending,
    MergedClean,
    LgtmFirstPass,
    RetriedOk,
    Escalated,
    QaFailed,
    Reverted,
    Abandoned,
}

impl RoutingOutcome {
    /// The exact outcome string the routing ledger persists.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::MergedClean => "merged_clean",
            Self::LgtmFirstPass => "lgtm_first_pass",
            Self::RetriedOk => "retried_ok",
            Self::Escalated => "escalated",
            Self::QaFailed => "qa_failed",
            Self::Reverted => "reverted",
            Self::Abandoned => "abandoned",
        }
    }

    /// Parse the routing ledger's outcome column. Unknown outcomes are an
    /// error, not a silent fallback: this vocabulary belongs to the ledger.
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "pending" => Ok(Self::Pending),
            "merged_clean" => Ok(Self::MergedClean),
            "lgtm_first_pass" => Ok(Self::LgtmFirstPass),
            "retried_ok" => Ok(Self::RetriedOk),
            "escalated" => Ok(Self::Escalated),
            "qa_failed" => Ok(Self::QaFailed),
            "reverted" => Ok(Self::Reverted),
            "abandoned" => Ok(Self::Abandoned),
            other => Err(format!("unknown routing ledger outcome {other:?}")),
        }
    }
}

/// Calibration state of a quality/outcome correlation. The V1 shadow
/// foundation is advisory: prediction is uncalibrated until samples and
/// thresholds are approved, so [`CalibrationBand::Uncalibrated`] is the
/// honest default and never a silent guess.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CalibrationBand {
    Uncalibrated,
    Calibrated,
}

/// Why a preservation check failed. Each variant is a forbidden repair
/// behavior from the delta design "Controlled repair and preservation
/// boundary".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreservationFailure {
    RequirementInvented,
    ArchitectureChanged,
    ConstraintWeakened,
    ScopeChanged,
    AcceptanceCriteriaInvented,
}

/// Typed memory audit events (delta design "Durable structured memory
/// delta": challenges, validation, supersession, staleness, archiving,
/// audit).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryEvent {
    Challenged,
    Validated,
    Superseded,
    MarkedStale,
    Archived,
    Audited,
}

/// Typed relation kind between memory records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryRelationKind {
    Supports,
    Contradicts,
    Supersedes,
    DerivedFrom,
}

/// Scoped native session identity and lineage. Reuses the field-for-field
/// lineage identity of the native session authority (`SessionLineage` in
/// `src/agent/session.rs`) so a learning record always correlates to one
/// existing session/work identity: work item, stage, role, worktree,
/// branch, PR, model, provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionLineageRef {
    pub work_item: String,
    pub stage: String,
    pub role: String,
    pub worktree: String,
    pub branch: String,
    #[serde(default)]
    pub pull_request: Option<String>,
    pub model: String,
    pub provider: String,
}

impl SessionLineageRef {
    /// Mirrors `SessionLineage::validate`: the lineage identity is
    /// non-optional; only the PR may be absent pre-PR.
    pub fn validate(&self) -> Result<(), String> {
        for (field, value) in [
            ("work_item", self.work_item.as_str()),
            ("stage", self.stage.as_str()),
            ("role", self.role.as_str()),
            ("worktree", self.worktree.as_str()),
            ("branch", self.branch.as_str()),
            ("model", self.model.as_str()),
            ("provider", self.provider.as_str()),
        ] {
            if value.is_empty() {
                return Err(format!("lineage {field} must be non-empty"));
            }
        }
        if let Some(pr) = self.pull_request.as_deref() {
            if pr.is_empty() {
                return Err("lineage pull_request must be non-empty when set".to_string());
            }
        }
        Ok(())
    }
}

/// Canonical artifact / revision / target-role / target-model identity.
/// Revisions are 1-based; a controlled repair MAY advance the revision but
/// never redefines the artifact id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactIdentity {
    pub artifact_id: String,
    pub revision: u64,
    pub target_role: String,
    pub target_model: String,
    pub content_hash: String,
}

impl ArtifactIdentity {
    pub fn validate(&self) -> Result<(), String> {
        for (field, value) in [
            ("artifact_id", self.artifact_id.as_str()),
            ("target_role", self.target_role.as_str()),
            ("target_model", self.target_model.as_str()),
        ] {
            if value.is_empty() {
                return Err(format!("artifact {field} must be non-empty"));
            }
        }
        if self.revision < 1 {
            return Err("artifact revision is 1-based".to_string());
        }
        if !is_sha256_hex_digest(&self.content_hash) {
            return Err("artifact content_hash must be a sha256 hex digest".to_string());
        }
        Ok(())
    }
}

/// An observation candidate ingested through the optional, pinned observer
/// bridge. Observations are untrusted data and candidate evidence, never
/// policy, and must be credential-redacted before storage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationCandidate {
    pub candidate_id: String,
    pub statement: String,
    pub authority: MemoryAuthority,
    /// Canonical harness name of the source (see
    /// `SessionHarness` in `src/agent/session.rs`).
    pub source_harness: String,
    pub provenance_commit: String,
    /// Unix seconds.
    pub observed_at: u64,
    pub redacted: bool,
}

impl ObservationCandidate {
    pub fn validate(&self) -> Result<(), String> {
        for (field, value) in [
            ("candidate_id", self.candidate_id.as_str()),
            ("statement", self.statement.as_str()),
            ("source_harness", self.source_harness.as_str()),
            ("provenance_commit", self.provenance_commit.as_str()),
        ] {
            if value.is_empty() {
                return Err(format!("observation {field} must be non-empty"));
            }
        }
        if !self.authority.is_untrusted() {
            return Err(
                "observation candidates are untrusted data and candidate evidence, never policy"
                    .to_string(),
            );
        }
        if !self.redacted {
            return Err("observations must be credential-redacted before storage".to_string());
        }
        Ok(())
    }
}

/// Result of AutoSpec-owned normalization/promotion of an observation
/// candidate. The candidate id reuses the observation candidate's id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PromotionResult {
    pub candidate_id: String,
    pub outcome: PromotionOutcome,
    /// Unix seconds.
    pub decided_at: u64,
    pub reason: String,
}

impl PromotionResult {
    pub fn validate(&self) -> Result<(), String> {
        if self.candidate_id.is_empty() {
            return Err("promotion candidate_id must be non-empty".to_string());
        }
        if self.reason.is_empty() {
            return Err("promotion reason must be non-empty".to_string());
        }
        Ok(())
    }
}

/// Evidence reference. The run id reuses the evidence bundle run-id
/// authority (`EvidenceBundle` in `src/evidence/mod.rs`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceRef {
    pub run_id: String,
    /// Optional repo-relative artifact path inside the evidence bundle.
    pub artifact: Option<String>,
}

impl EvidenceRef {
    pub fn validate(&self) -> Result<(), String> {
        if !is_valid_evidence_run_id(&self.run_id) {
            return Err(format!(
                "evidence run_id {:?} is not a valid run id",
                self.run_id
            ));
        }
        if let Some(artifact) = self.artifact.as_deref() {
            if artifact.is_empty() {
                return Err("evidence artifact path must be non-empty when set".to_string());
            }
        }
        Ok(())
    }
}

/// A typed relation between memory records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryRelation {
    pub kind: MemoryRelationKind,
    pub target_id: String,
}

/// Failed-approach detail — first-class in the delta design.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FailedApproachDetail {
    pub approach: String,
    pub failure_conditions: Vec<String>,
    pub replacement: Option<String>,
    pub retry_conditions: Option<String>,
    pub do_not_retry: bool,
}

/// A typed memory audit event. Separation of duties: the actor role must
/// not equal the record's origin role (no self-approval).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryAuditEvent {
    pub event: MemoryEvent,
    pub actor_role: String,
    /// Unix seconds.
    pub at: u64,
}

/// Structured durable memory record (delta design "Durable structured memory
/// delta"): type, statement, reason, scope, authority, confidence, status,
/// origin, evidence, revisions, content hash, tags, typed relations, and
/// challenge/validation audit events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryRecord {
    pub record_id: String,
    pub mem_type: MemoryType,
    pub statement: String,
    pub reason: String,
    pub scope: MemoryScope,
    pub authority: MemoryAuthority,
    /// Evidence-backed confidence in percent (0-100); never a policy.
    pub confidence: u8,
    pub status: MemoryStatus,
    pub origin_role: String,
    pub origin_model: String,
    pub origin_provider: String,
    /// Unix seconds.
    pub created_at: u64,
    /// Unix seconds.
    pub updated_at: u64,
    pub evidence: Vec<EvidenceRef>,
    pub source_commit: Option<String>,
    pub revision: u64,
    pub content_hash: String,
    pub tags: Vec<String>,
    pub relations: Vec<MemoryRelation>,
    /// Required exactly when `mem_type` is [`MemoryType::FailedApproach`].
    pub failed_approach: Option<FailedApproachDetail>,
    pub events: Vec<MemoryAuditEvent>,
    /// Separation of duties: the role that validated this record; must not
    /// equal `origin_role` when set.
    pub validated_by: Option<String>,
}

impl MemoryRecord {
    pub fn validate(&self) -> Result<(), String> {
        for (field, value) in [
            ("record_id", self.record_id.as_str()),
            ("statement", self.statement.as_str()),
            ("reason", self.reason.as_str()),
            ("origin_role", self.origin_role.as_str()),
            ("origin_model", self.origin_model.as_str()),
            ("origin_provider", self.origin_provider.as_str()),
        ] {
            if value.is_empty() {
                return Err(format!("memory {field} must be non-empty"));
            }
        }
        if self.updated_at < self.created_at {
            return Err("memory updated_at must not precede created_at".to_string());
        }
        if self.revision < 1 {
            return Err("memory revision is 1-based".to_string());
        }
        if !is_sha256_hex_digest(&self.content_hash) {
            return Err("memory content_hash must be a sha256 hex digest".to_string());
        }
        for evidence in &self.evidence {
            evidence.validate()?;
        }
        let has_detail = self.failed_approach.is_some();
        let is_failed_approach = self.mem_type == MemoryType::FailedApproach;
        if is_failed_approach && !has_detail {
            return Err("a failed-approach memory requires its detail".to_string());
        }
        if has_detail && !is_failed_approach {
            return Err("failed-approach detail requires the failed-approach type".to_string());
        }
        if let Some(validated_by) = self.validated_by.as_deref() {
            if validated_by == self.origin_role {
                return Err(
                    "separation of duties: the originating role cannot validate its own memory"
                        .to_string(),
                );
            }
        }
        for event in &self.events {
            if event.actor_role.is_empty() {
                return Err("memory audit event actor_role must be non-empty".to_string());
            }
            if event.actor_role == self.origin_role {
                return Err(
                    "separation of duties: the originating role cannot audit its own memory"
                        .to_string(),
                );
            }
        }
        Ok(())
    }

    /// Export this record back to reviewable Markdown (delta design:
    /// important memory changes are exportable back to reviewable Markdown
    /// when policy demands).
    pub fn to_markdown(&self) -> String {
        let evidence = self
            .evidence
            .iter()
            .map(|e| e.run_id.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "## Memory {}\n\n- **type:** {}\n- **statement:** {}\n- **reason:** {}\n\
             - **scope:** {}\n- **authority:** {}\n- **confidence:** {}/100\n\
             - **status:** {}\n- **origin:** {}/{}/{}\n- **revision:** {}\n\
             - **content hash:** `{}`\n- **evidence:** {}\n",
            self.record_id,
            self.mem_type.as_str(),
            self.statement,
            self.reason,
            self.scope.as_str(),
            self.authority.as_str(),
            self.confidence,
            self.status.as_str(),
            self.origin_role,
            self.origin_model,
            self.origin_provider,
            self.revision,
            self.content_hash,
            evidence,
        )
    }
}

/// Quality provider result (delta design "Readiness shadow foundation").
/// `available: false` is the truthful degraded state: a missing or
/// unavailable provider is never scored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QualityProviderResult {
    pub provider: String,
    pub dimension: String,
    pub available: bool,
    /// 0-100; present exactly when `available` is true.
    pub score: Option<u8>,
}

impl QualityProviderResult {
    pub fn validate(&self) -> Result<(), String> {
        if self.provider.is_empty() {
            return Err("quality provider must be non-empty".to_string());
        }
        if self.dimension.is_empty() {
            return Err("quality dimension must be non-empty".to_string());
        }
        match (self.available, self.score) {
            (true, Some(_)) | (false, None) => Ok(()),
            (true, None) => Err("an available quality provider must carry a score".to_string()),
            (false, Some(score)) => Err(format!(
                "an unavailable quality provider cannot carry score {score}"
            )),
        }
    }
}

/// Deterministic FNV-1a 64-bit fingerprint of the readiness policy (mode +
/// provider/dimension/score set). The same policy always produces the same
/// `fp1-` hex digest; the digest is a correlation aid, not a security
/// control.
pub fn policy_fingerprint(mode: QualityPolicyMode, providers: &[QualityProviderResult]) -> String {
    let canonical =
        serde_json::to_string(&(mode, providers)).expect("serde_json cannot fail on these types");
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in canonical.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x1000_0000_01b3);
    }
    format!("fp1-{hash:016x}")
}

/// Quality report with policy fingerprint and decision. A higher quality
/// score cannot override a preservation failure (enforced by
/// [`LearningContractV1::validate`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QualityReport {
    pub report_id: String,
    pub artifact: ArtifactIdentity,
    pub policy_mode: QualityPolicyMode,
    pub providers: Vec<QualityProviderResult>,
    /// None when a hard failure or an unavailable required provider prevents
    /// aggregation.
    pub aggregate: Option<u8>,
    pub hard_failure: bool,
    pub decision: QualityDecision,
    /// Deterministic fingerprint of `(policy_mode, providers)`.
    pub policy_fingerprint: String,
}

impl QualityReport {
    pub fn validate(&self) -> Result<(), String> {
        if self.report_id.is_empty() {
            return Err("quality report_id must be non-empty".to_string());
        }
        self.artifact.validate()?;
        for provider in &self.providers {
            provider.validate()?;
        }
        let mut seen = std::collections::BTreeSet::new();
        for provider in &self.providers {
            if !seen.insert((provider.provider.clone(), provider.dimension.clone())) {
                return Err(format!(
                    "duplicate quality provider/dimension {}/{}",
                    provider.provider, provider.dimension
                ));
            }
        }
        if self.hard_failure && self.aggregate.is_some() {
            return Err("a hard failure must not aggregate a score".to_string());
        }
        if self.hard_failure && self.decision != QualityDecision::Blocked {
            return Err(
                "a hard failure must produce a Blocked decision, not a higher score".to_string(),
            );
        }
        if self.policy_fingerprint != policy_fingerprint(self.policy_mode, &self.providers) {
            return Err("quality report policy fingerprint mismatch".to_string());
        }
        Ok(())
    }
}

/// Context/freshness fingerprint (delta design "context/freshness
/// fingerprint").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextFingerprint {
    pub fingerprint: String,
    pub source_commit: String,
    pub fresh: bool,
    /// Unix seconds.
    pub compiled_at: u64,
}

impl ContextFingerprint {
    pub fn validate(&self) -> Result<(), String> {
        if !is_sha256_hex_digest(&self.fingerprint) {
            return Err("context fingerprint must be a sha256 hex digest".to_string());
        }
        if self.source_commit.is_empty() {
            return Err("context source_commit must be non-empty".to_string());
        }
        Ok(())
    }
}

/// Requirement-preservation result for a controlled repair. Repair attempts
/// are bounded at two; a higher quality score cannot override a preservation
/// failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreservationResult {
    pub before: ArtifactIdentity,
    pub after: ArtifactIdentity,
    pub preserved: bool,
    pub failures: Vec<PreservationFailure>,
    /// Bounded at two (delta design "Controlled repair and preservation
    /// boundary").
    pub attempts: u8,
}

impl PreservationResult {
    pub fn validate(&self) -> Result<(), String> {
        self.before.validate()?;
        self.after.validate()?;
        if self.preserved != self.failures.is_empty() {
            return Err("preservation preserved flag must equal failures.is_empty()".to_string());
        }
        if self.attempts > 2 {
            return Err("controlled repair attempts are bounded at two".to_string());
        }
        Ok(())
    }
}

/// Execution/outcome correlation: binds learning evidence to the single
/// existing outcome ledger via its dispatch id and outcome vocabulary, and
/// to RealWork benchmark ids when present. No second outcome or benchmark
/// store exists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutcomeCorrelation {
    /// The routing ledger's dispatch id (correlation identifier).
    pub dispatch_id: String,
    /// The evidence bundle run id this correlation is backed by.
    pub evidence_run_id: String,
    pub routing_outcome: RoutingOutcome,
    /// RealWork benchmark id when the run is benchmark-backed.
    pub benchmark_id: Option<String>,
    pub calibration: CalibrationBand,
}

impl OutcomeCorrelation {
    pub fn validate(&self) -> Result<(), String> {
        if self.dispatch_id.is_empty() {
            return Err("correlation dispatch_id must be non-empty".to_string());
        }
        if !is_valid_evidence_run_id(&self.evidence_run_id) {
            return Err(format!(
                "correlation evidence_run_id {:?} is not a valid run id",
                self.evidence_run_id
            ));
        }
        if let Some(benchmark_id) = self.benchmark_id.as_deref() {
            if benchmark_id.is_empty() {
                return Err("correlation benchmark_id must be non-empty when set".to_string());
            }
        }
        Ok(())
    }
}

/// The frozen additive learning-integration contract: one versioned record
/// tying scoped session lineage, artifact identity, observation/promotion,
/// memory, quality, context freshness, preservation, and outcome
/// correlation together.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LearningContractV1 {
    pub schema_version: u32,
    pub session: SessionLineageRef,
    pub artifact: ArtifactIdentity,
    #[serde(default)]
    pub observation: Option<ObservationCandidate>,
    #[serde(default)]
    pub promotion: Option<PromotionResult>,
    #[serde(default)]
    pub memory: Option<MemoryRecord>,
    #[serde(default)]
    pub quality: Option<QualityReport>,
    #[serde(default)]
    pub fingerprint: Option<ContextFingerprint>,
    #[serde(default)]
    pub preservation: Option<PreservationResult>,
    #[serde(default)]
    pub correlation: Option<OutcomeCorrelation>,
}

impl LearningContractV1 {
    /// Validate the record against its own invariants, including the
    /// cross-record trust and separation-of-duties rules.
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != LEARNING_CONTRACT_SCHEMA {
            return Err(format!(
                "unsupported learning contract schema version {} (expected {LEARNING_CONTRACT_SCHEMA})",
                self.schema_version
            ));
        }
        self.session.validate()?;
        self.artifact.validate()?;
        if let Some(observation) = &self.observation {
            observation.validate()?;
        }
        if let Some(promotion) = &self.promotion {
            promotion.validate()?;
        }
        if let Some(memory) = &self.memory {
            memory.validate()?;
        }
        if let Some(quality) = &self.quality {
            quality.validate()?;
        }
        if let Some(fingerprint) = &self.fingerprint {
            fingerprint.validate()?;
        }
        if let Some(preservation) = &self.preservation {
            preservation.validate()?;
        }
        if let Some(correlation) = &self.correlation {
            correlation.validate()?;
        }
        if let (Some(observation), Some(promotion)) = (&self.observation, &self.promotion) {
            if observation.candidate_id != promotion.candidate_id {
                return Err("promotion must reference the observed candidate id".to_string());
            }
        }
        if let (Some(preservation), Some(quality)) = (&self.preservation, &self.quality) {
            if !preservation.preserved && quality.decision == QualityDecision::Ready {
                return Err(
                    "a preservation failure cannot be overridden by a quality score".to_string(),
                );
            }
        }
        Ok(())
    }

    /// The existing work-item identity this contract correlates to.
    pub fn work_item(&self) -> &str {
        &self.session.work_item
    }

    /// The existing role identity this contract correlates to.
    pub fn role(&self) -> &str {
        &self.session.role
    }

    /// The existing evidence bundle run id this contract correlates to, if
    /// any.
    pub fn evidence_run_id(&self) -> Option<&str> {
        self.correlation
            .as_ref()
            .map(|c| c.evidence_run_id.as_str())
    }

    /// The existing routing-ledger dispatch id this contract correlates to,
    /// if any.
    pub fn ledger_dispatch_id(&self) -> Option<&str> {
        self.correlation.as_ref().map(|c| c.dispatch_id.as_str())
    }

    /// The existing routing-ledger outcome this correlation records, if any.
    pub fn routing_outcome(&self) -> Option<RoutingOutcome> {
        self.correlation.as_ref().map(|c| c.routing_outcome)
    }
}

/// Evidence bundle run-id grammar, mirroring the evidence authority
/// (`valid_run_id` in `src/evidence/mod.rs`): non-empty, first byte
/// alphanumeric, remainder alphanumeric or `.`/`_`/`-`.
pub fn is_valid_evidence_run_id(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.first().is_some_and(u8::is_ascii_alphanumeric)
        && bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// sha256 hex digest grammar: exactly 64 lowercase/uppercase hex characters.
pub fn is_sha256_hex_digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}
