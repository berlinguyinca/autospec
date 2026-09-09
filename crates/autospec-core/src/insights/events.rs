//! Normalized session event model (spec §7; dependency issue #3826).
//!
//! One row per normalized event, produced by the deterministic Stage 1
//! extraction. No semantic classification happens here (spec §4.1).

use serde::{Deserialize, Serialize};

/// Core event types per spec §7.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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

/// Token usage attached to an event (spec §7 `tokens`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tokens {
    pub input: u64,
    pub output: u64,
}

/// One normalized session event (spec §7).
///
/// `timestamp` is Unix seconds (fractional allowed) so reducers can compute
/// durations deterministically without a date library.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NormalizedEvent {
    pub event_id: String,
    pub session_id: String,
    pub parent_session_id: Option<String>,
    pub timestamp: f64,
    pub repo: Option<String>,
    pub branch: Option<String>,
    pub work_item_id: Option<String>,
    pub agent_role: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub event_type: EventType,
    pub tool: Option<String>,
    pub payload: serde_json::Value,
    pub tokens: Tokens,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_type_round_trips_through_snake_case_serde() {
        let json = serde_json::to_string(&EventType::UserIntervention).unwrap();
        assert_eq!(json, "\"user_intervention\"");
        assert_eq!(
            serde_json::from_str::<EventType>("\"pull_request_opened\"").unwrap(),
            EventType::PullRequestOpened
        );
    }

    #[test]
    fn normalized_event_matches_the_spec_seven_shape() {
        let event = NormalizedEvent {
            event_id: "evt_123".into(),
            session_id: "session_456".into(),
            parent_session_id: None,
            timestamp: 1757316000.0,
            repo: Some("inferweave/autospec".into()),
            branch: Some("feature/foo".into()),
            work_item_id: Some("issue-721".into()),
            agent_role: Some("implementer".into()),
            provider: Some("local".into()),
            model: Some("qwen3".into()),
            event_type: EventType::ToolCall,
            tool: Some("shell".into()),
            payload: serde_json::json!({}),
            tokens: Tokens {
                input: 1234,
                output: 212,
            },
        };
        let value: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&event).unwrap()).unwrap();
        assert_eq!(value["event_type"], "tool_call");
        assert_eq!(value["tokens"]["input"], 1234);
    }
}
