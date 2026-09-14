//! Repository Attention Streams — resumable analysis of corpora larger than a
//! model context window.
//!
//! An attention stream is a durable, resumable analysis job, not a giant
//! prompt. It processes a source set in chunks, persisting structured
//! incremental output and advancing a durable cursor atomically. If a source
//! changes while the stream is suspended, the change is detected and the
//! stream either continues safely, invalidates affected findings, or requires
//! reconciliation — it never silently resumes against materially different
//! content.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::ids::StreamId;

/// Versioned stream schema identity.
pub const ATTENTION_STREAM_SCHEMA: &str = "autospec.attention-stream.v1";

/// Stream lifecycle status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StreamStatus {
    Created,
    Active,
    Suspended,
    Completed,
    Cancelled,
    NeedsReconciliation,
}

impl StreamStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            StreamStatus::Created => "created",
            StreamStatus::Active => "active",
            StreamStatus::Suspended => "suspended",
            StreamStatus::Completed => "completed",
            StreamStatus::Cancelled => "cancelled",
            StreamStatus::NeedsReconciliation => "needs-reconciliation",
        }
    }
}

/// A source file with a digest used to detect mutation between runs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SourceRef {
    pub path: String,
    /// Content digest (e.g. sha256) captured when the stream last saw it.
    pub digest: String,
}

/// A structured finding with its supporting evidence.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Finding {
    pub summary: String,
    pub evidence_refs: Vec<String>,
    pub contradictions: Vec<String>,
    pub hypothesis: Option<String>,
}

/// A durable attention stream.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AttentionStream {
    pub schema: String,
    pub stream_id: StreamId,
    pub objective: String,
    pub scope: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub status: StreamStatus,
    pub source_set: Vec<SourceRef>,
    /// The last processed chunk cursor (inclusive). Persist atomically before
    /// advancing the durable cursor.
    pub cursor: u64,
    pub chunks_completed: u64,
    pub chunks_total_if_known: Option<u64>,
    pub accumulated_findings: Vec<Finding>,
    pub unresolved_questions: Vec<String>,
    pub follow_up_queries: Vec<String>,
    pub checkpoint_refs: Vec<String>,
    pub memory_refs: Vec<String>,
    pub artifact_refs: Vec<String>,
}

impl AttentionStream {
    pub fn new(stream_id: StreamId, objective: impl Into<String>) -> Self {
        Self {
            schema: ATTENTION_STREAM_SCHEMA.to_string(),
            stream_id,
            objective: objective.into(),
            scope: None,
            created_at: String::new(),
            updated_at: String::new(),
            status: StreamStatus::Created,
            source_set: Vec::new(),
            cursor: 0,
            chunks_completed: 0,
            chunks_total_if_known: None,
            accumulated_findings: Vec::new(),
            unresolved_questions: Vec::new(),
            follow_up_queries: Vec::new(),
            checkpoint_refs: Vec::new(),
            memory_refs: Vec::new(),
            artifact_refs: Vec::new(),
        }
    }

    /// Apply a chunk's structured incremental output, advancing the cursor.
    ///
    /// The caller must persist `stream` **after** this returns so the durable
    /// cursor never advances without the findings that justify it.
    pub fn apply_chunk(&mut self, output: &ChunkOutput) {
        self.accumulated_findings.extend(output.new_findings.clone());
        self.unresolved_questions
            .extend(output.unresolved_questions.clone());
        self.follow_up_queries.extend(output.follow_up_queries.clone());
        self.cursor = output.chunk_index;
        self.chunks_completed += 1;
        self.status = StreamStatus::Active;
        self.updated_at.clear();
    }
}

/// Structured incremental output for one processed chunk.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChunkOutput {
    pub chunk_index: u64,
    pub new_findings: Vec<Finding>,
    pub contradictions: Vec<String>,
    pub unresolved_questions: Vec<String>,
    pub changed_hypotheses: Vec<String>,
    pub follow_up_queries: Vec<String>,
    pub next_cursor: u64,
}

/// The result of comparing a previously-seen source set against the current
/// on-disk state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MutationVerdict {
    /// No source changed; safe to continue.
    Unchanged,
    /// A source changed but the change is additive/compatible; continue but
    /// record that affected findings may need revalidation.
    ChangedCompatible,
    /// A source changed materially; invalidate affected findings or require
    /// reconciliation before resuming.
    ChangedMaterial,
    /// A previously-seen source is missing.
    SourceMissing,
}

/// Classify a source mutation against the digests recorded in a stream.
pub fn classify_mutation(stream: &AttentionStream, current: &BTreeMap<String, String>) -> MutationVerdict {
    let mut any_change = false;
    let mut any_material = false;
    for src in &stream.source_set {
        match current.get(&src.path) {
            None => return MutationVerdict::SourceMissing,
            Some(digest) => {
                if digest != &src.digest {
                    any_change = true;
                    // Conservative: any digest change on a source the stream
                    // already produced findings for is treated as material so we
                    // never silently resume against different content.
                    any_material = true;
                }
            }
        }
    }
    if any_material {
        MutationVerdict::ChangedMaterial
    } else if any_change {
        MutationVerdict::ChangedCompatible
    } else {
        MutationVerdict::Unchanged
    }
}

/// Resume decision: a stream may only resume from its durable cursor when its
/// sources are unchanged. Otherwise it must be reconciled first.
pub fn resume_verdict(stream: &AttentionStream, current: &BTreeMap<String, String>) -> (StreamStatus, MutationVerdict) {
    let m = classify_mutation(stream, current);
    match m {
        MutationVerdict::Unchanged => (StreamStatus::Active, m),
        MutationVerdict::ChangedCompatible => (StreamStatus::Active, m),
        MutationVerdict::ChangedMaterial | MutationVerdict::SourceMissing => {
            (StreamStatus::NeedsReconciliation, m)
        }
    }
}

/// Cancel a stream durably.
pub fn cancel(stream: &mut AttentionStream) {
    stream.status = StreamStatus::Cancelled;
    stream.updated_at.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream_with_sources() -> AttentionStream {
        let nonce = b"stream-nonce";
        let mut s = AttentionStream::new(StreamId::new(nonce), "find request routing impls");
        s.source_set = vec![
            SourceRef {
                path: "src/router.rs".to_string(),
                digest: "digest-a".to_string(),
            },
            SourceRef {
                path: "src/handler.rs".to_string(),
                digest: "digest-b".to_string(),
            },
        ];
        s
    }

    #[test]
    fn chunk_advances_cursor_and_accumulates_findings() {
        let mut s = stream_with_sources();
        let output = ChunkOutput {
            chunk_index: 1,
            new_findings: vec![Finding {
                summary: "router dispatch at src/router.rs:42".to_string(),
                evidence_refs: vec!["src/router.rs:42".to_string()],
                contradictions: Vec::new(),
                hypothesis: None,
            }],
            contradictions: Vec::new(),
            unresolved_questions: vec!["who owns the fallback?".to_string()],
            changed_hypotheses: Vec::new(),
            follow_up_queries: vec!["search fallback".to_string()],
            next_cursor: 1,
        };
        s.apply_chunk(&output);
        assert_eq!(s.cursor, 1);
        assert_eq!(s.chunks_completed, 1);
        assert_eq!(s.accumulated_findings.len(), 1);
        assert_eq!(s.unresolved_questions.len(), 1);
        assert_eq!(s.status, StreamStatus::Active);
    }

    #[test]
    fn unchanged_sources_resume() {
        let s = stream_with_sources();
        let mut current = BTreeMap::new();
        current.insert("src/router.rs".to_string(), "digest-a".to_string());
        current.insert("src/handler.rs".to_string(), "digest-b".to_string());
        assert_eq!(
            resume_verdict(&s, &current).0,
            StreamStatus::Active
        );
    }

    #[test]
    fn material_mutation_requires_reconciliation() {
        let s = stream_with_sources();
        let mut current = BTreeMap::new();
        current.insert("src/router.rs".to_string(), "digest-a".to_string());
        // handler.rs changed
        current.insert("src/handler.rs".to_string(), "digest-changed".to_string());
        let (status, _) = resume_verdict(&s, &current);
        assert_eq!(status, StreamStatus::NeedsReconciliation);
    }

    #[test]
    fn missing_source_requires_reconciliation() {
        let s = stream_with_sources();
        let mut current = BTreeMap::new();
        current.insert("src/router.rs".to_string(), "digest-a".to_string());
        let (status, verdict) = resume_verdict(&s, &current);
        assert_eq!(status, StreamStatus::NeedsReconciliation);
        assert_eq!(verdict, MutationVerdict::SourceMissing);
    }

    #[test]
    fn cancel_is_durable_and_observable() {
        let mut s = stream_with_sources();
        cancel(&mut s);
        assert_eq!(s.status, StreamStatus::Cancelled);
    }
}
