//! The §7 normalized session event model and the §6 `SessionAdapter` trait.
//!
//! Spec: `docs/specs/2026-09-08-continuous-improvement-engine.md` §6, §7.
//!
//! Every stage of the Continuous Improvement Engine reads this one record: the
//! summary builder (§8), the pattern detectors (§10), the analytics dashboards
//! (§28-§33). It is therefore a wire/storage contract, not a convenience
//! struct — the field set is exactly §7 plus the token counts §16-§17 need.
//!
//! Privacy (§39): raw prompt and tool output text MUST NOT be promoted into a
//! typed column. Anything provider-specific stays in the `payload` field of
//! [`NormalizedEvent`], which is opaque to this module and is the single
//! surface the redaction pass (§39) rewrites. Concrete adapters, storage writes,
//! enrichment, retention and the CLI surfaces are deliberately out of scope
//! here.

use crate::error::AutospecError;
use serde::{Deserialize, Serialize};

/// §52 `extractor_version`, stamped onto every batch an adapter emits so a
/// historical aggregate can be reproduced from the code that produced it.
///
/// Bump the **major** version when a §7 field changes name, type or meaning;
/// the **minor** version when a new event type or optional field is added;
/// the **patch** version when extraction is corrected without a schema change.
pub const EXTRACTOR_VERSION: &str = "1.0.0";

/// §7/§16/§17 token accounting for a single event. `u64` because the §34
/// columns are `BIGINT` and a cumulative session total can exceed `u32`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TokenCounts {
    /// Tokens billed/prompted on the input side.
    pub input: u64,
    /// Tokens generated on the output side.
    pub output: u64,
}

impl TokenCounts {
    /// Input plus output — the figure §16 context-waste and §17 model
    /// performance report per event.
    pub fn total(&self) -> u64 {
        self.input.saturating_add(self.output)
    }
}

/// §7 `event_type`. The names are the contract: they are what the §34
/// `session_events.event_type` column stores and what the detectors match on,
/// so an unrecognised name is an ingestion error rather than a silent variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    SessionStarted,
    SessionFinished,
    ModelSelected,
    ModelFallback,
    UserMessage,
    AssistantMessage,
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
    CiStarted,
    CiFinished,
    IssueLinked,
    UserIntervention,
}

impl EventType {
    /// Every §7 event-type name, in §7 order. `ALL.len()` is the size of the
    /// catalogue; `event_type_catalogue_matches_section_7` pins it against the
    /// spec list.
    pub const ALL: [&'static str; 29] = [
        "session_started",
        "session_finished",
        "model_selected",
        "model_fallback",
        "user_message",
        "assistant_message",
        "tool_call",
        "tool_result",
        "tool_error",
        "file_read",
        "file_write",
        "file_patch",
        "command_run",
        "command_failed",
        "test_run",
        "test_result",
        "lint_result",
        "review_result",
        "context_compaction",
        "context_limit_warning",
        "subagent_spawned",
        "subagent_finished",
        "git_commit",
        "pull_request_opened",
        "pull_request_reviewed",
        "ci_started",
        "ci_finished",
        "issue_linked",
        "user_intervention",
    ];

    /// The §7 wire name of this event type — the value stored in §34.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SessionStarted => "session_started",
            Self::SessionFinished => "session_finished",
            Self::ModelSelected => "model_selected",
            Self::ModelFallback => "model_fallback",
            Self::UserMessage => "user_message",
            Self::AssistantMessage => "assistant_message",
            Self::ToolCall => "tool_call",
            Self::ToolResult => "tool_result",
            Self::ToolError => "tool_error",
            Self::FileRead => "file_read",
            Self::FileWrite => "file_write",
            Self::FilePatch => "file_patch",
            Self::CommandRun => "command_run",
            Self::CommandFailed => "command_failed",
            Self::TestRun => "test_run",
            Self::TestResult => "test_result",
            Self::LintResult => "lint_result",
            Self::ReviewResult => "review_result",
            Self::ContextCompaction => "context_compaction",
            Self::ContextLimitWarning => "context_limit_warning",
            Self::SubagentSpawned => "subagent_spawned",
            Self::SubagentFinished => "subagent_finished",
            Self::GitCommit => "git_commit",
            Self::PullRequestOpened => "pull_request_opened",
            Self::PullRequestReviewed => "pull_request_reviewed",
            Self::CiStarted => "ci_started",
            Self::CiFinished => "ci_finished",
            Self::IssueLinked => "issue_linked",
            Self::UserIntervention => "user_intervention",
        }
    }

    /// Parse a §7 event-type name, rejecting anything else with
    /// [`AutospecError::Parse`] so an adapter typo surfaces at ingestion.
    pub fn parse(name: &str) -> Result<Self, AutospecError> {
        Self::try_from(name)
    }
}

impl TryFrom<&str> for EventType {
    type Error = AutospecError;

    fn try_from(name: &str) -> Result<Self, Self::Error> {
        let parsed = match name {
            "session_started" => Self::SessionStarted,
            "session_finished" => Self::SessionFinished,
            "model_selected" => Self::ModelSelected,
            "model_fallback" => Self::ModelFallback,
            "user_message" => Self::UserMessage,
            "assistant_message" => Self::AssistantMessage,
            "tool_call" => Self::ToolCall,
            "tool_result" => Self::ToolResult,
            "tool_error" => Self::ToolError,
            "file_read" => Self::FileRead,
            "file_write" => Self::FileWrite,
            "file_patch" => Self::FilePatch,
            "command_run" => Self::CommandRun,
            "command_failed" => Self::CommandFailed,
            "test_run" => Self::TestRun,
            "test_result" => Self::TestResult,
            "lint_result" => Self::LintResult,
            "review_result" => Self::ReviewResult,
            "context_compaction" => Self::ContextCompaction,
            "context_limit_warning" => Self::ContextLimitWarning,
            "subagent_spawned" => Self::SubagentSpawned,
            "subagent_finished" => Self::SubagentFinished,
            "git_commit" => Self::GitCommit,
            "pull_request_opened" => Self::PullRequestOpened,
            "pull_request_reviewed" => Self::PullRequestReviewed,
            "ci_started" => Self::CiStarted,
            "ci_finished" => Self::CiFinished,
            "issue_linked" => Self::IssueLinked,
            "user_intervention" => Self::UserIntervention,
            other => return Err(unknown_event_type(other)),
        };
        Ok(parsed)
    }
}

/// The one error every unrecognized §7 name produces.
fn unknown_event_type(name: &str) -> AutospecError {
    AutospecError::parse(
        "insights::events::EventType",
        format!("unknown §7 event_type: {name}"),
    )
}

/// The 13 §7 fields plus [`TokenCounts`]: one normalized thing that happened in
/// a session, independent of which harness produced it.
///
/// Field types are chosen to survive the §34 `session_events` columns without a
/// lossy cast: `event_id`/`session_id` are `TEXT` keys, `timestamp` stays the
/// RFC 3339 UTC string the adapter reported (parsed into `timestamptz` at the
/// storage boundary, so the core model carries no date library), `payload` is
/// the `jsonb` column, `tokens` is the `input`/`output` pair.
///
/// Unknown JSON fields are ignored on read — a newer extractor (§52) must not
/// break an older reader.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NormalizedEvent {
    /// Adapter-assigned unique id (§35 evidence anchor).
    pub event_id: String,
    /// Session this event belongs to.
    pub session_id: String,
    /// §9/§15 subagent lineage; `null` for a top-level session.
    pub parent_session_id: Option<String>,
    /// RFC 3339 UTC instant as reported by the source.
    pub timestamp: String,
    /// `owner/repo` the session ran in, as the adapter resolved it.
    pub repo: String,
    /// Branch checked out for the session.
    pub branch: String,
    /// Issue/PR/task the work item is tracked under (§35).
    pub work_item_id: String,
    /// Harness role, e.g. `implementer` (see `execution::ExecutorRole`).
    pub agent_role: String,
    /// Provider class, e.g. `local`, `anthropic`, `openai`.
    pub provider: String,
    /// Concrete model, e.g. `qwen3`.
    pub model: String,
    pub event_type: EventType,
    /// Tool name for `tool_*`/`file_*`/`command_*` events; `null` otherwise.
    pub tool: Option<String>,
    /// Opaque, provider-specific detail (§39 redaction target). Never promote a
    /// field from here into a typed column without a §52 major bump.
    pub payload: serde_json::Value,
    pub tokens: TokenCounts,
}

impl NormalizedEvent {
    /// Deserialize a §7 record, mapping any serde failure to
    /// [`AutospecError::Parse`] so callers see one error type across ingestion.
    pub fn from_json_str(raw: &str) -> Result<Self, AutospecError> {
        serde_json::from_str(raw).map_err(|error| {
            AutospecError::parse("insights::events::NormalizedEvent", error.to_string())
        })
    }

    /// [`NormalizedEvent::from_json_str`] for a value already parsed off the wire.
    pub fn from_json_value(raw: serde_json::Value) -> Result<Self, AutospecError> {
        serde_json::from_value(raw).map_err(|error| {
            AutospecError::parse("insights::events::NormalizedEvent", error.to_string())
        })
    }
}

/// A discoverable session source, returned by [`SessionAdapter::discover`] and
/// consumed by [`SessionAdapter::read`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionRef {
    /// Adapter id that produced this reference, e.g. `pi-jsonl`.
    pub source: String,
    /// Where the raw session lives (file path, URL, or provider locator).
    pub uri: String,
    /// Incremental-ingestion cursor (§6 "support incremental ingestion").
    /// `None` means "read from the beginning"; an adapter defines the encoding
    /// (byte offset, record sequence, timestamp) and stores it verbatim so the
    /// core model stays source-agnostic.
    pub resume_cursor: Option<String>,
}

/// One raw record as the source emitted it, before normalization. Opaque JSON:
/// the §6 adapters each have a different native shape, and the only guarantee
/// the core model needs is that it is JSON that can be archived (§35) and
/// redacted (§39).
pub type RawEvent = serde_json::Value;

/// §6 session adapter contract: discover sources, read raw records, normalize
/// them into [`NormalizedEvent`]s.
///
/// The trait is synchronous and object-safe. §6 sketches it with a `context`
/// and a channel; the Rust port drops both on purpose — cancellation and
/// timeouts belong to the ingestion service that owns the task, streaming is a
/// property of a concrete adapter's `read` (a batch return keeps the trait
/// object-safe for a `Vec<Box<dyn SessionAdapter>>` registry), and async stays
/// confined to the storage boundary (ADR 0001 D5/D10).
///
/// Implementations MUST be idempotent for a given [`SessionRef`] plus resume
/// cursor, which is what prevents duplicate ingestion (§6).
pub trait SessionAdapter {
    /// Enumerate session sources this adapter can read.
    fn discover(&self) -> Result<Vec<SessionRef>, AutospecError>;

    /// Read the raw records of one session, honouring
    /// [`SessionRef::resume_cursor`] for incremental ingestion.
    fn read(&self, reference: &SessionRef) -> Result<Vec<RawEvent>, AutospecError>;

    /// Map one raw record onto zero or more normalized events. Zero is legal
    /// (a record the adapter recognizes but does not model); an unrepresentable
    /// record is an [`AutospecError::Parse`], never a silently wrong event.
    fn normalize(&self, event: &RawEvent) -> Result<Vec<NormalizedEvent>, AutospecError>;
}

#[cfg(test)]
mod tests {
    use crate::error::AutospecError;
    use crate::insights::events::{
        EventType, NormalizedEvent, SessionAdapter, SessionRef, EXTRACTOR_VERSION,
    };
    use serde_json::{json, Value};

    /// The verbatim §7 example.
    const SECTION_7_EXAMPLE: &str = r#"{
      "event_id": "evt_123",
      "session_id": "session_456",
      "parent_session_id": null,
      "timestamp": "2026-09-08T18:00:00Z",
      "repo": "inferweave/autospec",
      "branch": "feature/foo",
      "work_item_id": "issue-721",
      "agent_role": "implementer",
      "provider": "local",
      "model": "qwen3",
      "event_type": "tool_call",
      "tool": "shell",
      "payload": {},
      "tokens": { "input": 1234, "output": 212 }
    }"#;

    #[test]
    fn section_7_example_deserializes_with_every_field_populated() {
        let event = NormalizedEvent::from_json_str(SECTION_7_EXAMPLE).expect("§7 example parses");
        assert_eq!(event.event_id, "evt_123");
        assert_eq!(event.session_id, "session_456");
        assert_eq!(event.parent_session_id, None);
        assert_eq!(event.timestamp, "2026-09-08T18:00:00Z");
        assert_eq!(event.repo, "inferweave/autospec");
        assert_eq!(event.branch, "feature/foo");
        assert_eq!(event.work_item_id, "issue-721");
        assert_eq!(event.agent_role, "implementer");
        assert_eq!(event.provider, "local");
        assert_eq!(event.model, "qwen3");
        assert_eq!(event.event_type, EventType::ToolCall);
        assert_eq!(event.tool.as_deref(), Some("shell"));
        assert_eq!(event.payload, json!({}));
        assert_eq!(event.tokens.input, 1234);
        assert_eq!(event.tokens.output, 212);
    }

    #[test]
    fn section_7_example_round_trips_to_an_equal_json_value() {
        let original: Value = serde_json::from_str(SECTION_7_EXAMPLE).expect("example is JSON");
        let event = NormalizedEvent::from_json_str(SECTION_7_EXAMPLE).expect("§7 example parses");
        let reserialized = serde_json::to_value(&event).expect("event serializes");
        assert_eq!(reserialized, original);
    }

    #[test]
    fn every_event_type_name_round_trips() {
        for name in EventType::ALL {
            let parsed = EventType::parse(name).expect("§7 name parses");
            let encoded = serde_json::to_value(parsed).expect("event type serializes");
            assert_eq!(encoded, json!(name));
            assert_eq!(parsed.as_str(), name);
        }
    }

    #[test]
    fn token_counts_total_is_input_plus_output() {
        let event = NormalizedEvent::from_json_str(SECTION_7_EXAMPLE).expect("§7 example parses");
        assert_eq!(event.tokens.total(), 1446);
    }

    /// The issue body quotes "26" §7 names; §7 itself enumerates 29 (the body
    /// miscounts). The spec list is the contract, so it is pinned verbatim here
    /// and every one of its names must round-trip.
    #[test]
    fn event_type_catalogue_matches_section_7() {
        assert_eq!(
            EventType::ALL,
            [
                "session_started",
                "session_finished",
                "model_selected",
                "model_fallback",
                "user_message",
                "assistant_message",
                "tool_call",
                "tool_result",
                "tool_error",
                "file_read",
                "file_write",
                "file_patch",
                "command_run",
                "command_failed",
                "test_run",
                "test_result",
                "lint_result",
                "review_result",
                "context_compaction",
                "context_limit_warning",
                "subagent_spawned",
                "subagent_finished",
                "git_commit",
                "pull_request_opened",
                "pull_request_reviewed",
                "ci_started",
                "ci_finished",
                "issue_linked",
                "user_intervention",
            ]
        );
        assert_eq!(EventType::ALL.len(), 29);
    }

    #[test]
    fn from_json_value_accepts_the_example_and_rejects_garbage() {
        let value: Value = serde_json::from_str(SECTION_7_EXAMPLE).expect("example is JSON");
        let event = NormalizedEvent::from_json_value(value).expect("§7 value parses");
        assert_eq!(event.event_type, EventType::ToolCall);

        let err = NormalizedEvent::from_json_value(json!({ "event_id": "evt_1" }))
            .expect_err("partial record must fail");
        assert!(matches!(err, AutospecError::Parse { .. }));
        assert!(err.to_string().contains("session_id"), "{err}");
    }

    #[test]
    fn unrecognised_event_type_is_an_autospec_error() {
        let err = EventType::parse("quantum_leap").expect_err("unknown name must fail");
        assert!(matches!(err, AutospecError::Parse { .. }));
        assert!(err.to_string().contains("quantum_leap"));

        let body = SECTION_7_EXAMPLE.replace("tool_call", "quantum_leap");
        let err = NormalizedEvent::from_json_str(&body).expect_err("unknown event_type must fail");
        assert!(matches!(err, AutospecError::Parse { .. }));
    }

    #[test]
    fn session_ref_carries_source_uri_and_resume_cursor() {
        let reference = SessionRef {
            source: "pi-jsonl".to_string(),
            uri: "file:///sessions/session_456.jsonl".to_string(),
            resume_cursor: Some("byte:4096".to_string()),
        };
        let encoded = serde_json::to_value(&reference).expect("session ref serializes");
        assert_eq!(
            encoded,
            json!({
                "source": "pi-jsonl",
                "uri": "file:///sessions/session_456.jsonl",
                "resume_cursor": "byte:4096",
            })
        );
        let decoded: SessionRef =
            serde_json::from_value(encoded).expect("session ref deserializes");
        assert_eq!(decoded, reference);

        let fresh = SessionRef {
            resume_cursor: None,
            ..reference
        };
        let encoded = serde_json::to_value(&fresh).expect("session ref serializes");
        assert_eq!(encoded["resume_cursor"], Value::Null);
    }

    #[test]
    fn extractor_version_is_a_release_string() {
        // §52 prints `extractor_version: 1.2.0`, so the constant must stay a
        // three-part numeric release string.
        let parts: Vec<&str> = EXTRACTOR_VERSION.split('.').collect();
        assert_eq!(parts.len(), 3, "§52 version is major.minor.patch");
        assert!(
            parts
                .iter()
                .all(|part| !part.is_empty() && part.chars().all(|digit| digit.is_ascii_digit())),
            "each component must be numeric: {EXTRACTOR_VERSION}"
        );
    }

    #[test]
    fn session_adapter_is_object_safe() {
        // Compile-time proof: `&dyn SessionAdapter` only type-checks while the
        // trait stays object-safe. No adapter mock is needed — a concrete
        // adapter is out of scope for this issue.
        fn count_sessions(adapter: &dyn SessionAdapter) -> Result<usize, AutospecError> {
            Ok(adapter.discover()?.len())
        }
        let _ = count_sessions;
    }
}
