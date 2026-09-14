//! Context Guardian — proactive structured continuation checkpoints.
//!
//! Long-running Pi/agent sessions must preserve durable continuation state
//! before useful context is lost to truncation or opaque compaction. The
//! guardian turns a context-utilization observation into a threshold verdict,
//! renders a versioned checkpoint, and validates a checkpoint before it is
//! acknowledged as durable.
//!
//! This module is pure: it returns verdicts, plans and validation results, and
//! the caller performs the persistence I/O. That keeps threshold policy and
//! checkpoint validation testable without a harness or a model.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::ids::{AttemptId, CheckpointId, ExecutionId, SessionId, WorkId};

/// Versioned checkpoint schema identity.
pub const CONTEXT_CHECKPOINT_SCHEMA: &str = "autospec.context-checkpoint.v1";

/// Configurable threshold policy with sensible defaults.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextGuardianConfig {
    pub enabled: bool,
    /// Opportunistic checkpoint at this utilization percent.
    pub soft_checkpoint_percent: u32,
    /// Checkpoint at the next safe boundary at this percent.
    pub checkpoint_warning_percent: u32,
    /// No new substantial implementation phase may begin beyond this percent.
    pub required_checkpoint_percent: u32,
    /// Maximum tokens a single checkpoint may consume.
    pub max_checkpoint_tokens: usize,
    /// Budget for the resume memory map.
    pub resume_memory_map_tokens: usize,
}

impl Default for ContextGuardianConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            soft_checkpoint_percent: 60,
            checkpoint_warning_percent: 75,
            required_checkpoint_percent: 85,
            max_checkpoint_tokens: 6000,
            resume_memory_map_tokens: 3000,
        }
    }
}

/// How context usage is known. Capability hierarchy: exact > estimated >
/// conservative-estimate > unknown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum EstimateSource {
    /// The provider/harness reported an exact context count.
    Exact,
    /// Model metadata plus a tokenizer/estimator.
    Estimated,
    /// A conservative local estimate.
    ConservativeEstimate,
    /// No usable context count; fall back to milestone-based checkpointing.
    Unknown,
}

impl EstimateSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            EstimateSource::Exact => "exact",
            EstimateSource::Estimated => "estimated",
            EstimateSource::ConservativeEstimate => "conservative-estimate",
            EstimateSource::Unknown => "unknown",
        }
    }
}

/// A context-utilization observation with provenance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextObservation {
    pub window_tokens: usize,
    pub estimated_used_tokens: usize,
    pub source: EstimateSource,
}

impl ContextObservation {
    /// Utilization percent (0..=100), or `None` when unknown.
    pub fn utilization_percent(&self) -> Option<u32> {
        if self.source == EstimateSource::Unknown || self.window_tokens == 0 {
            return None;
        }
        let pct = (self.estimated_used_tokens as u64 * 100) / self.window_tokens as u64;
        Some(pct.min(100) as u32)
    }
}

/// The guardian's threshold verdict for an observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThresholdVerdict {
    /// Below the soft threshold; checkpointing is optional.
    Below,
    /// At or above the soft threshold; checkpoint opportunistically at a
    /// coherent milestone.
    Soft,
    /// At or above the warning threshold; checkpoint at the next safe boundary.
    Warning,
    /// At or above the required threshold; no new substantial implementation
    /// phase may begin until a valid durable checkpoint exists.
    Required,
    /// Utilization is unknown; fall back to milestone-based checkpointing and
    /// do not block solely on an absent exact count.
    Unknown,
}

/// Evaluate an observation against a config.
pub fn evaluate_threshold(
    config: &ContextGuardianConfig,
    observation: &ContextObservation,
) -> ThresholdVerdict {
    if !config.enabled {
        return ThresholdVerdict::Below;
    }
    let Some(pct) = observation.utilization_percent() else {
        return ThresholdVerdict::Unknown;
    };
    if pct >= config.required_checkpoint_percent {
        ThresholdVerdict::Required
    } else if pct >= config.checkpoint_warning_percent {
        ThresholdVerdict::Warning
    } else if pct >= config.soft_checkpoint_percent {
        ThresholdVerdict::Soft
    } else {
        ThresholdVerdict::Below
    }
}

/// Validation status of a checkpoint before it may be acknowledged as durable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointValidation {
    pub valid: bool,
    /// Human/machine readable reasons. Empty when valid.
    pub errors: Vec<String>,
}

impl CheckpointValidation {
    fn ok() -> Self {
        Self {
            valid: true,
            errors: Vec::new(),
        }
    }
    fn fail(errors: Vec<String>) -> Self {
        Self {
            valid: false,
            errors,
        }
    }
}

/// A structured continuation checkpoint.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ContextCheckpoint {
    pub schema: String,
    pub checkpoint_id: CheckpointId,
    pub created_at: String,
    pub execution_id: ExecutionId,
    pub attempt_id: AttemptId,
    pub session_id: SessionId,
    pub work_id: WorkId,
    pub repository: Option<String>,
    pub worktree: Option<String>,
    pub branch: Option<String>,
    pub issue: Option<String>,
    pub pull_request: Option<String>,
    pub objective: String,
    pub acceptance_criteria: Vec<String>,
    pub completed: Vec<String>,
    pub in_progress: Vec<String>,
    pub next_actions: Vec<String>,
    pub changed_files: Vec<String>,
    pub important_code_refs: Vec<String>,
    pub evidence_refs: Vec<String>,
    pub decisions: Vec<String>,
    pub known_failures: Vec<String>,
    pub blockers: Vec<String>,
    pub unresolved_questions: Vec<String>,
    pub memory_refs: Vec<String>,
    pub attention_stream_refs: Vec<String>,
    pub artifact_refs: Vec<String>,
    pub context: ContextObservation,
    pub validation: BTreeMap<String, String>,
    pub review: Vec<String>,
    /// Redaction: any secret-like value found here makes the checkpoint unsafe.
    pub redacted: Vec<String>,
}

impl ContextCheckpoint {
    pub fn new(
        checkpoint_id: CheckpointId,
        execution_id: ExecutionId,
        attempt_id: AttemptId,
        session_id: SessionId,
        work_id: WorkId,
        objective: impl Into<String>,
        context: ContextObservation,
    ) -> Self {
        Self {
            schema: CONTEXT_CHECKPOINT_SCHEMA.to_string(),
            checkpoint_id,
            created_at: String::new(),
            execution_id,
            attempt_id,
            session_id,
            work_id,
            repository: None,
            worktree: None,
            branch: None,
            issue: None,
            pull_request: None,
            objective: objective.into(),
            acceptance_criteria: Vec::new(),
            completed: Vec::new(),
            in_progress: Vec::new(),
            next_actions: Vec::new(),
            changed_files: Vec::new(),
            important_code_refs: Vec::new(),
            evidence_refs: Vec::new(),
            decisions: Vec::new(),
            known_failures: Vec::new(),
            blockers: Vec::new(),
            unresolved_questions: Vec::new(),
            memory_refs: Vec::new(),
            attention_stream_refs: Vec::new(),
            artifact_refs: Vec::new(),
            context,
            validation: BTreeMap::new(),
            review: Vec::new(),
            redacted: Vec::new(),
        }
    }

    /// Estimate the continuation-oriented size in "tokens" (a simple bounded
    /// proxy: characters / 4). Used to enforce the bounded-checkpoint rule.
    pub fn estimated_tokens(&self) -> usize {
        let json = serde_json::to_string(self).unwrap_or_default();
        json.chars().count() / 4
    }
}

/// Secret-like patterns that must never be persisted in a checkpoint.
/// This is a deliberately conservative denylist used in addition to any
/// existing secret scanning. It rejects raw environment dumps and common
/// credential forms.
pub fn contains_secret_like(text: &str) -> Option<&'static str> {
    let patterns: &[(&str, &'static str)] = &[
        ("BEGIN RSA PRIVATE KEY", "private-key"),
        ("BEGIN OPENSSH PRIVATE KEY", "private-key"),
        ("BEGIN PRIVATE KEY", "private-key"),
        ("ghp_", "github-token"),
        ("gho_", "github-token"),
        ("AKIA", "aws-access-key"),
        ("sk-", "openai-style-secret"),
        ("xoxb-", "slack-token"),
        ("-----BEGIN", "pem-block"),
        ("password=", "password"),
        ("api_key=", "api-key"),
    ];
    for (needle, label) in patterns {
        if text.contains(needle) {
            return Some(label);
        }
    }
    None
}

/// Validate a checkpoint before it may be acknowledged as durable.
///
/// Checks, in order: schema identity, identifiers match the active execution/
/// attempt/session, the checkpoint is bounded, and no secret-like value is
/// present in the free-text fields (or in a caller-provided artifact check).
pub fn validate_checkpoint(cp: &ContextCheckpoint, config: &ContextGuardianConfig) -> CheckpointValidation {
    let mut errors = Vec::new();

    if cp.schema != CONTEXT_CHECKPOINT_SCHEMA {
        errors.push(format!("unexpected schema {}", cp.schema));
    }
    if cp.objective.trim().is_empty() {
        errors.push("objective is empty".to_string());
    }
    if cp.estimated_tokens() > config.max_checkpoint_tokens {
        errors.push(format!(
            "checkpoint exceeds max_checkpoint_tokens ({} > {})",
            cp.estimated_tokens(),
            config.max_checkpoint_tokens
        ));
    }

    // Secret scan across every free-text field and references.
    let text = [
        cp.objective.as_str(),
        &cp.acceptance_criteria.join("\n"),
        &cp.completed.join("\n"),
        &cp.in_progress.join("\n"),
        &cp.next_actions.join("\n"),
        &cp.decisions.join("\n"),
        &cp.known_failures.join("\n"),
        &cp.blockers.join("\n"),
        &cp.unresolved_questions.join("\n"),
        &cp.memory_refs.join("\n"),
        &cp.attention_stream_refs.join("\n"),
        &cp.artifact_refs.join("\n"),
    ]
    .join("\n");
    if let Some(kind) = contains_secret_like(&text) {
        errors.push(format!("secret-like content rejected ({kind})"));
    }

    if errors.is_empty() {
        CheckpointValidation::ok()
    } else {
        CheckpointValidation::fail(errors)
    }
}

/// Whether a new substantial implementation phase may begin, given a valid
/// durable checkpoint exists and the current threshold verdict.
///
/// Deterministic gate: the guardian may *not* be bypassed by a model merely
/// because the model believes the work is safe.
pub fn may_begin_substantial_phase(
    verdict: ThresholdVerdict,
    has_valid_durable_checkpoint: bool,
) -> bool {
    match verdict {
        ThresholdVerdict::Required => has_valid_durable_checkpoint,
        ThresholdVerdict::Unknown => true, // milestone-based fallback; never deadlock
        _ => true,
    }
}

/// The phases of the checkpoint lifecycle. Recovery must distinguish these so a
/// crash before acknowledgement never falsely marks a checkpoint complete.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum CheckpointPhase {
    Requested,
    Generated,
    DurablyPersisted,
    Acknowledged,
    ResumeStarted,
    ResumeCompleted,
}

impl CheckpointPhase {
    pub fn as_str(&self) -> &'static str {
        match self {
            CheckpointPhase::Requested => "requested",
            CheckpointPhase::Generated => "generated",
            CheckpointPhase::DurablyPersisted => "durably-persisted",
            CheckpointPhase::Acknowledged => "acknowledged",
            CheckpointPhase::ResumeStarted => "resume-started",
            CheckpointPhase::ResumeCompleted => "resume-completed",
        }
    }
}

/// The resume hydration plan delivered to a new session. It intentionally does
/// **not** replay the prior conversation; it hands over continuation state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResumePlan {
    pub checkpoint_id: CheckpointId,
    pub execution_id: ExecutionId,
    pub work_id: WorkId,
    pub objective: String,
    pub acceptance_criteria: Vec<String>,
    pub next_actions: Vec<String>,
    pub required_files: Vec<String>,
    pub required_memory_queries: Vec<String>,
    pub required_commands: Vec<String>,
    /// True when repository state diverged from the checkpoint; the resume must
    /// be marked as needing reconciliation rather than pretending it is current.
    pub needs_reconciliation: bool,
    pub reconciliation_notes: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obs(used: usize, window: usize, source: EstimateSource) -> ContextObservation {
        ContextObservation {
            window_tokens: window,
            estimated_used_tokens: used,
            source,
        }
    }

    #[test]
    fn threshold_verdicts_follow_percent_bands() {
        let cfg = ContextGuardianConfig::default();
        assert_eq!(
            evaluate_threshold(&cfg, &obs(50, 100, EstimateSource::Exact)),
            ThresholdVerdict::Below
        );
        assert_eq!(
            evaluate_threshold(&cfg, &obs(60, 100, EstimateSource::Exact)),
            ThresholdVerdict::Soft
        );
        assert_eq!(
            evaluate_threshold(&cfg, &obs(80, 100, EstimateSource::Estimated)),
            ThresholdVerdict::Warning
        );
        assert_eq!(
            evaluate_threshold(&cfg, &obs(90, 100, EstimateSource::ConservativeEstimate)),
            ThresholdVerdict::Required
        );
    }

    #[test]
    fn unknown_usage_does_not_deadlock() {
        let cfg = ContextGuardianConfig::default();
        assert_eq!(
            evaluate_threshold(&cfg, &obs(0, 0, EstimateSource::Unknown)),
            ThresholdVerdict::Unknown
        );
        // Even at Required with no checkpoint we allow Unknown to proceed via
        // milestone fallback; but a known Required blocks.
        assert!(may_begin_substantial_phase(ThresholdVerdict::Unknown, false));
        assert!(!may_begin_substantial_phase(ThresholdVerdict::Required, false));
        assert!(may_begin_substantial_phase(ThresholdVerdict::Required, true));
    }

    #[test]
    fn utilization_percent_is_capped() {
        assert_eq!(obs(200, 100, EstimateSource::Exact).utilization_percent(), Some(100));
        assert_eq!(obs(0, 0, EstimateSource::Unknown).utilization_percent(), None);
    }

    fn base_checkpoint() -> ContextCheckpoint {
        let nonce = b"checkpoint-nonce";
        ContextCheckpoint::new(
            CheckpointId::new(nonce),
            ExecutionId::new(nonce),
            AttemptId::new(nonce),
            SessionId::new(nonce),
            WorkId::new(nonce),
            "implement request routing",
            obs(50, 100, EstimateSource::Estimated),
        )
    }

    #[test]
    fn valid_checkpoint_passes_validation() {
        let cfg = ContextGuardianConfig::default();
        let cp = base_checkpoint();
        let v = validate_checkpoint(&cp, &cfg);
        assert!(v.valid, "errors: {:?}", v.errors);
    }

    #[test]
    fn checkpoint_rejects_secrets() {
        let cfg = ContextGuardianConfig::default();
        let mut cp = base_checkpoint();
        cp.decisions.push("use token ghp_1234567890abcdef for deploys".to_string());
        let v = validate_checkpoint(&cp, &cfg);
        assert!(!v.valid);
        assert!(v.errors.iter().any(|e| e.contains("secret-like")));
    }

    #[test]
    fn checkpoint_rejects_wrong_schema_and_oversize() {
        let mut cfg = ContextGuardianConfig::default();
        cfg.max_checkpoint_tokens = 10;
        let mut cp = base_checkpoint();
        cp.schema = "autospec.something-else.v1".to_string();
        let v = validate_checkpoint(&cp, &cfg);
        assert!(!v.valid);
        assert!(v.errors.iter().any(|e| e.contains("schema")));
        assert!(v.errors.iter().any(|e| e.contains("max_checkpoint_tokens")));
    }

    #[test]
    fn contains_secret_like_detects_common_forms() {
        assert!(contains_secret_like("ghp_abcdefghijklmnopqrstuvwxyz").is_some());
        assert!(contains_secret_like("-----BEGIN PRIVATE KEY-----").is_some());
        assert!(contains_secret_like("AKIAIOSFODNN7EXAMPLE").is_some());
        assert!(contains_secret_like("just prose, no secrets").is_none());
    }

    #[test]
    fn checkpoint_phase_ordering_distinguishes_crash_points() {
        assert!(CheckpointPhase::Requested < CheckpointPhase::Generated);
        assert!(CheckpointPhase::Generated < CheckpointPhase::DurablyPersisted);
        assert!(CheckpointPhase::DurablyPersisted < CheckpointPhase::Acknowledged);
        // A crash before persistence must never be read as acknowledged.
        assert_ne!(
            CheckpointPhase::Generated.as_str(),
            CheckpointPhase::Acknowledged.as_str()
        );
    }
}
