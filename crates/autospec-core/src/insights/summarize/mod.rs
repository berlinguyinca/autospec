//! Deterministic §8 session summary (issue #3837).
//!
//! [`summarize`] is a pure reducer over [`super::events::NormalizedEvent`]
//! slices: no LLM calls, no I/O, no clock reads — it runs without an async
//! runtime (spec §4.1 deterministic-first, §8). Semantic fields
//! (`task_type`, `task_domain`, `outcome`) are left `None`/empty for a
//! purely deterministic run; they are filled by the local classification
//! stage, not this reducer.
//!
//! Persistence lives in [`store`]: `store_summary` upserts by
//! `session_id`, and `recompute_summary` rewrites a row only when the
//! stored `extractor_version` differs from [`EXTRACTOR_VERSION`]
//! (spec §51 incremental recompute, §52 versioning).

pub mod store;

pub use store::{recompute_summary, store_summary};

use super::events::{EventType, NormalizedEvent};

/// Analyzer version stamped on every written summary row (spec §52:
/// `extractor_version: 1.2.0`).
pub const EXTRACTOR_VERSION: &str = "1.2.0";

/// The §8 session summary record.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionSummary {
    pub session_id: String,
    pub task_type: Option<String>,
    pub task_domain: Vec<String>,
    pub outcome: Option<String>,
    pub autonomy_score: f64,
    pub user_interventions: u64,
    pub review_rework_count: u64,
    pub tool_calls: u64,
    pub tool_errors: u64,
    pub files_read: u64,
    pub files_changed: u64,
    pub tests_run: u64,
    pub context_peak_tokens: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub estimated_cost: f64,
    pub duration_seconds: u64,
    pub models: Vec<String>,
    pub commits: Vec<String>,
    pub pull_requests: Vec<u64>,
}

/// Reduce a slice of normalized events (one session) to its §8 summary.
///
/// Pure and synchronous: counts derive from [`EventType`], token fields
/// from per-event usage, and durations from event timestamps.
pub fn summarize(events: &[NormalizedEvent]) -> SessionSummary {
    let mut summary = SessionSummary {
        session_id: events
            .first()
            .map(|event| event.session_id.clone())
            .unwrap_or_default(),
        task_type: None,
        task_domain: Vec::new(),
        outcome: None,
        autonomy_score: 1.0,
        user_interventions: 0,
        review_rework_count: 0,
        tool_calls: 0,
        tool_errors: 0,
        files_read: 0,
        files_changed: 0,
        tests_run: 0,
        context_peak_tokens: 0,
        input_tokens: 0,
        output_tokens: 0,
        estimated_cost: 0.0,
        duration_seconds: 0,
        models: Vec::new(),
        commits: Vec::new(),
        pull_requests: Vec::new(),
    };

    let mut assistant_turns: u64 = 0;
    let mut min_timestamp = f64::INFINITY;
    let mut max_timestamp = f64::NEG_INFINITY;
    for event in events {
        match event.event_type {
            EventType::AssistantMessage => assistant_turns += 1,
            EventType::ToolCall => summary.tool_calls += 1,
            EventType::ToolError => summary.tool_errors += 1,
            EventType::FileRead => summary.files_read += 1,
            EventType::FileWrite | EventType::FilePatch => summary.files_changed += 1,
            EventType::TestRun => summary.tests_run += 1,
            EventType::UserIntervention => summary.user_interventions += 1,
            EventType::ReviewResult => summary.review_rework_count += 1,
            EventType::GitCommit => push_commit(&mut summary.commits, event),
            EventType::PullRequestOpened => push_pull_request(&mut summary.pull_requests, event),
            _ => {}
        }
        summary.input_tokens = summary.input_tokens.saturating_add(event.tokens.input);
        summary.output_tokens = summary.output_tokens.saturating_add(event.tokens.output);
        summary.context_peak_tokens = summary.context_peak_tokens.max(event.tokens.input);
        if let Some(model) = &event.model {
            if !summary.models.iter().any(|existing| existing == model) {
                summary.models.push(model.clone());
            }
        }
        min_timestamp = min_timestamp.min(event.timestamp);
        max_timestamp = max_timestamp.max(event.timestamp);
    }

    summary.autonomy_score = autonomy_score(assistant_turns, summary.user_interventions);
    if !events.is_empty() {
        summary.duration_seconds = (max_timestamp - min_timestamp).round().max(0.0) as u64;
    }
    summary
}

/// 1 − (interventions / assistant turns), clamped to [0, 1]. A session with
/// no assistant turns is fully autonomous by definition (1.0).
fn autonomy_score(assistant_turns: u64, user_interventions: u64) -> f64 {
    if assistant_turns == 0 {
        return 1.0;
    }
    (1.0 - user_interventions as f64 / assistant_turns as f64).clamp(0.0, 1.0)
}

fn push_commit(commits: &mut Vec<String>, event: &NormalizedEvent) {
    let sha = event
        .payload
        .get("sha")
        .or_else(|| event.payload.get("commit"))
        .and_then(|value| value.as_str());
    if let Some(sha) = sha {
        if !commits.iter().any(|existing| existing == sha) {
            commits.push(sha.to_string());
        }
    }
}

fn push_pull_request(pull_requests: &mut Vec<u64>, event: &NormalizedEvent) {
    if let Some(number) = event.payload.get("number").and_then(|value| value.as_u64()) {
        if !pull_requests.contains(&number) {
            pull_requests.push(number);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::insights::events::Tokens;

    fn event(
        index: u32,
        session_id: &str,
        timestamp: f64,
        event_type: EventType,
        model: Option<&str>,
        tokens: Tokens,
        payload: serde_json::Value,
    ) -> NormalizedEvent {
        NormalizedEvent {
            event_id: format!("evt-{index}"),
            session_id: session_id.to_string(),
            parent_session_id: None,
            timestamp,
            repo: Some("inferweave/autospec".into()),
            branch: Some("main".into()),
            work_item_id: None,
            agent_role: Some("implementer".into()),
            provider: Some("local".into()),
            model: model.map(str::to_string),
            event_type,
            tool: None,
            payload,
            tokens,
        }
    }

    fn no_tokens() -> Tokens {
        Tokens {
            input: 0,
            output: 0,
        }
    }

    /// The 40-event local fixture (issue acceptance criterion: a 40-event
    /// fixture produces the expected summary).
    fn fixture_40_events() -> Vec<NormalizedEvent> {
        let s = "session-40";
        let mut events = Vec::with_capacity(40);
        // 1 session_started
        events.push(event(
            1,
            s,
            0.0,
            EventType::SessionStarted,
            Some("qwen3"),
            no_tokens(),
            serde_json::json!({}),
        ));
        // 5 assistant_message (turns), 1000 input / 200 output each
        for i in 0..5 {
            events.push(event(
                2 + i,
                s,
                (i * 10) as f64,
                EventType::AssistantMessage,
                Some("qwen3"),
                Tokens {
                    input: 1000,
                    output: 200,
                },
                serde_json::json!({}),
            ));
        }
        // 1 user_message
        events.push(event(
            7,
            s,
            52.0,
            EventType::UserMessage,
            Some("qwen3"),
            no_tokens(),
            serde_json::json!({}),
        ));
        // 2 user_intervention
        events.push(event(
            8,
            s,
            53.0,
            EventType::UserIntervention,
            Some("qwen3"),
            no_tokens(),
            serde_json::json!({}),
        ));
        events.push(event(
            9,
            s,
            54.0,
            EventType::UserIntervention,
            Some("qwen3"),
            no_tokens(),
            serde_json::json!({}),
        ));
        // 8 tool_call, 100 input each
        for i in 0..8 {
            events.push(event(
                10 + i,
                s,
                (55.0 + i as f64),
                EventType::ToolCall,
                Some("qwen3"),
                Tokens {
                    input: 100,
                    output: 0,
                },
                serde_json::json!({"tool": "shell"}),
            ));
        }
        // 4 tool_error
        for i in 0..4 {
            events.push(event(
                18 + i,
                s,
                (100.0 + i as f64),
                EventType::ToolError,
                Some("qwen3"),
                no_tokens(),
                serde_json::json!({}),
            ));
        }
        // 6 file_read
        for i in 0..6 {
            events.push(event(
                22 + i,
                s,
                (110.0 + i as f64),
                EventType::FileRead,
                Some("qwen3"),
                no_tokens(),
                serde_json::json!({}),
            ));
        }
        // 3 file_write + 2 file_patch = 5 files_changed
        for i in 0..3 {
            events.push(event(
                28 + i,
                s,
                (120.0 + i as f64),
                EventType::FileWrite,
                Some("qwen3"),
                no_tokens(),
                serde_json::json!({}),
            ));
        }
        for i in 0..2 {
            events.push(event(
                31 + i,
                s,
                (130.0 + i as f64),
                EventType::FilePatch,
                Some("qwen3"),
                no_tokens(),
                serde_json::json!({}),
            ));
        }
        // 3 test_run
        for i in 0..3 {
            events.push(event(
                33 + i,
                s,
                (140.0 + i as f64),
                EventType::TestRun,
                Some("qwen3"),
                no_tokens(),
                serde_json::json!({}),
            ));
        }
        // 2 review_result = 2 review reworks
        events.push(event(
            36,
            s,
            150.0,
            EventType::ReviewResult,
            Some("qwen3"),
            no_tokens(),
            serde_json::json!({}),
        ));
        events.push(event(
            37,
            s,
            151.0,
            EventType::ReviewResult,
            Some("qwen3"),
            no_tokens(),
            serde_json::json!({}),
        ));
        // 1 git_commit + 1 pull_request_opened
        events.push(event(
            38,
            s,
            160.0,
            EventType::GitCommit,
            Some("qwen3"),
            no_tokens(),
            serde_json::json!({"sha": "abc123"}),
        ));
        events.push(event(
            39,
            s,
            170.0,
            EventType::PullRequestOpened,
            Some("qwen3"),
            no_tokens(),
            serde_json::json!({"number": 721}),
        ));
        // 1 session_finished at t=551
        events.push(event(
            40,
            s,
            551.0,
            EventType::SessionFinished,
            Some("qwen3"),
            no_tokens(),
            serde_json::json!({}),
        ));
        assert_eq!(events.len(), 40);
        events
    }

    #[test]
    fn summarize_40_event_fixture_yields_expected_counts() {
        let summary = summarize(&fixture_40_events());
        assert_eq!(summary.session_id, "session-40");
        assert_eq!(summary.tool_calls, 8);
        assert_eq!(summary.tool_errors, 4);
        assert_eq!(summary.files_read, 6);
        assert_eq!(summary.files_changed, 5);
        assert_eq!(summary.tests_run, 3);
        assert_eq!(summary.user_interventions, 2);
        assert_eq!(summary.review_rework_count, 2);
        assert_eq!(summary.input_tokens, 5_800);
        assert_eq!(summary.output_tokens, 1_000);
        assert_eq!(summary.context_peak_tokens, 1_000);
        assert_eq!(summary.duration_seconds, 551);
        assert_eq!(summary.models, vec!["qwen3".to_string()]);
        assert_eq!(summary.commits, vec!["abc123".to_string()]);
        assert_eq!(summary.pull_requests, vec![721]);
        assert_eq!(summary.autonomy_score, 0.6);
        assert_eq!(summary.estimated_cost, 0.0);
    }

    #[test]
    fn summarize_leaves_semantic_fields_null_or_empty() {
        let summary = summarize(&fixture_40_events());
        assert!(summary.task_type.is_none());
        assert!(summary.outcome.is_none());
        assert!(summary.task_domain.is_empty());
    }

    #[test]
    fn summarize_empty_slice_is_all_zero() {
        let summary = summarize(&[]);
        assert_eq!(summary.session_id, "");
        assert_eq!(summary.autonomy_score, 1.0);
        assert_eq!(summary.duration_seconds, 0);
        assert_eq!(summary.tool_calls, 0);
        assert!(summary.models.is_empty());
    }

    #[test]
    fn autonomy_score_clamps_to_zero_when_interventions_exceed_turns() {
        let events = vec![
            event(
                1,
                "s",
                0.0,
                EventType::AssistantMessage,
                None,
                no_tokens(),
                serde_json::json!({}),
            ),
            event(
                2,
                "s",
                1.0,
                EventType::UserIntervention,
                None,
                no_tokens(),
                serde_json::json!({}),
            ),
            event(
                3,
                "s",
                2.0,
                EventType::UserIntervention,
                None,
                no_tokens(),
                serde_json::json!({}),
            ),
        ];
        let summary = summarize(&events);
        assert_eq!(summary.autonomy_score, 0.0);
    }

    #[test]
    fn models_keep_first_seen_order_without_duplicates() {
        let events = vec![
            event(
                1,
                "s",
                0.0,
                EventType::SessionStarted,
                Some("qwen3"),
                no_tokens(),
                serde_json::json!({}),
            ),
            event(
                2,
                "s",
                1.0,
                EventType::ModelFallback,
                Some("glm4"),
                no_tokens(),
                serde_json::json!({}),
            ),
            event(
                3,
                "s",
                2.0,
                EventType::AssistantMessage,
                Some("qwen3"),
                no_tokens(),
                serde_json::json!({}),
            ),
            event(
                4,
                "s",
                3.0,
                EventType::AssistantMessage,
                Some("glm4"),
                no_tokens(),
                serde_json::json!({}),
            ),
        ];
        let summary = summarize(&events);
        assert_eq!(
            summary.models,
            vec!["qwen3".to_string(), "glm4".to_string()]
        );
    }
}
