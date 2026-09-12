//! Local InferWeave semantic enrichment pipeline (issue #3848).
//!
//! Spec §36 describes the 8-stage semantic pipeline and §37 the
//! InferWeave service family. This module ships the first stage of the
//! pipeline: a **local** enrichment pipeline over session payload
//! evidence.
//!
//! Design (deterministic-first, §4.1):
//!
//! - [`Enricher`] is the backend-neutral seam; [`inferweave::InferWeaveEnricher`]
//!   is the local InferWeave implementation — a low-priority client that
//!   honors the `semantic_analysis` config, and `remote_allowed: false`
//!   confines it to a local endpoint.
//! - [`queue::run_queue`] drives a resumable [`EnrichmentJob`] queue:
//!   crash-safe status transitions, a per-job `cursor`, and retry with
//!   `attempts` accounting (a stopped queue resumes where it stopped).
//! - [`classify::classify`] runs §36 stage 3 on the same backend-neutral
//!   seam: §9 user re-steering is classified into the 16 §9 categories
//!   and persisted to `user_interventions` with a message reference
//!   (issue #3851, spec §35).
//! - Dispatch gates: every batch passes [`redact::Redactor`] (secrets are
//!   never dispatched) and every session passes [`redact::repo_allowed`]
//!   (a session in a repo outside the allowlist stays unenriched, with
//!   the reason logged).
//! - Strong model diagnosis (spec §36 stages 4-8) is deferred to the
//!   classifier epic (#3830).

pub mod classify;
pub mod inferweave;
pub mod queue;
pub mod redact;

use crate::error::AutospecError;

/// One enrichment record produced by an [`Enricher`] for one payload item.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Enrichment {
    pub session_id: String,
    /// Pipeline stage this enrichment belongs to (spec §36).
    pub stage: String,
    /// Index of the payload item within the job (cursor-relative callers
    /// record the absolute index).
    pub index: u64,
    /// Kind of enrichment (e.g. `"embedding"`, `"classification"`).
    pub kind: String,
    /// The enrichment value (serialized vector, label, …).
    pub value: String,
    /// Model that produced it (from `semantic_analysis.model`).
    pub model: String,
}

/// A redacted chunk of one session's payload evidence, ready for
/// dispatch. The queue guarantees `items` passed
/// [`redact::Redactor::redact`] and the session passed
/// [`redact::repo_allowed`] before this struct reaches an [`Enricher`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnrichmentBatch {
    pub session_id: String,
    pub stage: String,
    pub repo: Option<String>,
    /// Index into the job payload where `items` starts.
    pub cursor: u64,
    pub items: Vec<String>,
}

/// Status of an [`EnrichmentJob`] row (crash-safe: `running` jobs are
/// recovered to `pending` at the start of every [`queue::run_queue`]
/// pass).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobStatus {
    Pending,
    Running,
    Done,
    Failed,
    Skipped,
}

impl JobStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
        }
    }

    fn from_str(value: &str) -> Self {
        match value {
            "running" => Self::Running,
            "done" => Self::Done,
            "failed" => Self::Failed,
            "skipped" => Self::Skipped,
            _ => Self::Pending,
        }
    }
}

/// A unit of enrichment work: one session's payload evidence for one
/// pipeline stage. `cursor` and `attempts` make the job resumable and
/// retryable; `payload` holds the redacted source evidence (kept in the
/// row so it stays queryable after completion).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnrichmentJob {
    pub id: String,
    pub session_id: String,
    pub stage: String,
    /// Owning repository (`owner/name`); gates the session through
    /// [`redact::repo_allowed`].
    pub repo: Option<String>,
    /// Redacted payload evidence items.
    pub payload: Vec<String>,
    /// Number of payload items (must equal `payload.len()`).
    pub total: u64,
    /// Index into `payload` where the next dispatch starts.
    pub cursor: u64,
    /// Enricher attempts so far.
    pub attempts: u64,
    pub status: JobStatus,
}

/// Backend-neutral enrichment seam. Implementations receive only
/// [`EnrichmentBatch`]es that already passed the redaction gate and the
/// repo allowlist.
pub trait Enricher: Send + Sync {
    fn enrich(&self, batch: &EnrichmentBatch) -> Result<Vec<Enrichment>, AutospecError>;
}
