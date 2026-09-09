//! Normalized session event model (`docs/specs/2026-09-08-continuous-improvement-engine.md` §7).
//!
//! Interim local definition: issue #3826 owns the canonical `NormalizedEvent`
//! shared by every insights consumer, and has not landed yet. The shape here
//! is the exact subset §7 defines that [`super::summarize`] consumes — same
//! field names, same `event_type` wire values — so replacing this module with
//! a re-export of #3826's type is a drop-in change with no reducer edits.

use serde::{Deserialize, Serialize};

/// The event type discriminator (§7 `event_type`), snake_case on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    SessionStarted,
    SessionFinished,
    /// A model change mid-session for a non-fallback reason (configuration).
    ModelSelected,
    /// A model change mid-session for a fallback reason (quota, failure).
    ModelFallback,
    UserMessage,
    AssistantMessage,
    /// User re-steering during autonomous work. Present in §7's taxonomy via
    /// §3's "user interventions" fact class and required deterministically by
    /// §8's `user_interventions`.
    UserIntervention,
    ToolCall,
    ToolResult,
    ToolError,
    FileRead,
    FileWrite,
    FilePatch,
    CommandRun,
    CommandFailed,
    TestRun,
    TestResult,
    LintResult,
    ReviewResult,
    ContextCompaction,
    ContextLimitWarning,
    SubagentSpawned,
    SubagentFinished,
    GitCommit,
    PullRequestOpened,
    PullRequestReviewed,
    PullRequestMerged,
}

/// Per-event token accounting (§7 `tokens`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input: u64,
    pub output: u64,
}

/// One normalized session event (§7). Events are append-only and never
/// rewritten (§61); everything the reducer needs is inlined per event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NormalizedEvent {
    pub event_id: String,
    pub session_id: String,
    pub event_type: EventType,
    /// Unix epoch seconds. Kept as an integer so the reducer stays free of a
    /// date library; adapters convert ISO-8601 capture timestamps on ingest.
    pub timestamp: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default)]
    pub tokens: TokenUsage,
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub payload: serde_json::Value,
}
