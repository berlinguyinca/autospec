//! Deterministic §8 session summary record
//! (`docs/specs/2026-09-08-continuous-improvement-engine.md` §8).
//!
//! [`summarize`] is a pure reducer over the §7 [`NormalizedEvent`] stream:
//! no I/O, no clock, no LLM (§4.1 "Deterministic First, Semantic Second").
//! The semantic fields `task_type`, `task_domain` and `outcome` stay `None`
//! here; they are filled by a separate pass (#3832) and never re-derived.
//!
//! [`store_summary`] persists the record to the shared `session_summaries`
//! table (upsert keyed by `session_id`). Each row is stamped with
//! [`EXTRACTOR_VERSION`]; a stored summary is only recomputed from raw events
//! when that version changes (§51 "raw history MUST NOT be reprocessed unless
//! analyzer version changes", §52 versioning).

use serde::{Deserialize, Serialize};

mod events;
mod store;

#[cfg(test)]
mod tests;

pub use events::{EventType, NormalizedEvent, TokenUsage};
pub use store::{
    ensure_summary_table, needs_recompute, should_recompute, store_summary,
    stored_extractor_version, CREATE_TABLE_SQL,
};

/// Version of this deterministic extractor. Bump on ANY change to the reducer
/// semantics so stored summaries are marked stale and recomputed (§51/§52).
pub const EXTRACTOR_VERSION: &str = "1.0.0";

/// The §8 session summary record: the analysis unit consumed by pattern
/// mining, metrics, and dashboards. Field names mirror §8 exactly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionSummary {
    pub session_id: String,
    /// Semantic (#3832): null until the enrichment pass fills it.
    pub task_type: Option<String>,
    /// Semantic (#3832): null until the enrichment pass fills it.
    pub task_domain: Option<Vec<String>>,
    /// Semantic (#3832): null until the enrichment pass fills it.
    pub outcome: Option<String>,
    /// `1 - user_interventions / assistant_turns`, clamped to `[0.0, 1.0]`.
    /// `1.0` when the session produced no assistant turns.
    pub autonomy_score: f64,
    pub user_interventions: u32,
    /// `review_result` events whose payload declares `"rework": true`.
    pub review_rework_count: u32,
    pub tool_calls: u32,
    pub tool_errors: u32,
    /// Count of `file_read` events (one event per read).
    pub files_read: u32,
    /// Count of `file_write` plus `file_patch` events.
    pub files_changed: u32,
    /// Count of `test_run` events.
    pub tests_run: u32,
    /// Peak observed per-event input-token count — the best mechanical proxy
    /// for peak context occupancy available in the §7 event stream.
    pub context_peak_tokens: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// 0.0 until a price table enters deterministic scope; §4.1 excludes
    /// provider billing lookups from mechanical extraction.
    pub estimated_cost: f64,
    /// Last event timestamp minus first, in seconds; 0 for empty input.
    pub duration_seconds: i64,
    /// Distinct models seen in the session, sorted for determinism.
    pub models: Vec<String>,
    /// Commit SHAs from `git_commit` event payloads, sorted.
    pub commits: Vec<String>,
    /// PR numbers from `pull_request_*` event payloads, sorted.
    pub pull_requests: Vec<u64>,
}

/// Pure deterministic reduction of a §7 event stream to one §8 summary.
///
/// Order-independent for every field except `session_id` (taken from the
/// first event; every event of a session carries the same id anyway), so two
/// runs over the same events always produce identical output.
pub fn summarize(events: &[NormalizedEvent]) -> SessionSummary {
    let mut acc = Aggregator::default();
    for event in events {
        acc.absorb(event);
    }
    acc.finish()
}

/// Running totals for [`summarize`]; private to keep the public surface at
/// the pure reducer plus persistence.
#[derive(Debug, Default)]
struct Aggregator {
    session_id: Option<String>,
    user_interventions: u32,
    review_rework_count: u32,
    tool_calls: u32,
    tool_errors: u32,
    files_read: u32,
    files_changed: u32,
    tests_run: u32,
    assistant_turns: u32,
    input_tokens: u64,
    output_tokens: u64,
    context_peak_tokens: u64,
    first_ts: Option<i64>,
    last_ts: Option<i64>,
    models: Vec<String>,
    commits: Vec<String>,
    pull_requests: Vec<u64>,
}

impl Aggregator {
    fn absorb(&mut self, event: &NormalizedEvent) {
        self.session_id
            .get_or_insert_with(|| event.session_id.clone());
        self.input_tokens += event.tokens.input;
        self.output_tokens += event.tokens.output;
        self.context_peak_tokens = self.context_peak_tokens.max(event.tokens.input);
        self.first_ts = Some(
            self.first_ts
                .map_or(event.timestamp, |t| t.min(event.timestamp)),
        );
        self.last_ts = Some(
            self.last_ts
                .map_or(event.timestamp, |t| t.max(event.timestamp)),
        );
        if let Some(model) = &event.model {
            if !self.models.contains(model) {
                self.models.push(model.clone());
            }
        }
        match event.event_type {
            EventType::ToolCall => self.tool_calls += 1,
            EventType::ToolError => self.tool_errors += 1,
            EventType::FileRead => self.files_read += 1,
            EventType::FileWrite | EventType::FilePatch => self.files_changed += 1,
            EventType::TestRun => self.tests_run += 1,
            EventType::UserIntervention => self.user_interventions += 1,
            EventType::AssistantMessage => self.assistant_turns += 1,
            EventType::ReviewResult if rework_requested(&event.payload) => {
                self.review_rework_count += 1;
            }
            EventType::GitCommit => {
                if let Some(sha) = payload_str(&event.payload, "sha") {
                    push_unique(&mut self.commits, sha);
                }
            }
            EventType::PullRequestOpened
            | EventType::PullRequestReviewed
            | EventType::PullRequestMerged => {
                if let Some(number) = payload_u64(&event.payload, "number") {
                    push_unique(&mut self.pull_requests, number);
                }
            }
            _ => {}
        }
    }

    fn finish(mut self) -> SessionSummary {
        sort_dedup(&mut self.models);
        sort_dedup(&mut self.commits);
        sort_dedup(&mut self.pull_requests);
        let autonomy_score = if self.assistant_turns == 0 {
            1.0
        } else {
            1.0 - (f64::from(self.user_interventions) / f64::from(self.assistant_turns))
                .clamp(0.0, 1.0)
        };
        SessionSummary {
            session_id: self.session_id.unwrap_or_default(),
            task_type: None,
            task_domain: None,
            outcome: None,
            autonomy_score,
            user_interventions: self.user_interventions,
            review_rework_count: self.review_rework_count,
            tool_calls: self.tool_calls,
            tool_errors: self.tool_errors,
            files_read: self.files_read,
            files_changed: self.files_changed,
            tests_run: self.tests_run,
            context_peak_tokens: self.context_peak_tokens,
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
            estimated_cost: 0.0,
            duration_seconds: match (self.first_ts, self.last_ts) {
                (Some(first), Some(last)) => last - first,
                _ => 0,
            },
            models: self.models,
            commits: self.commits,
            pull_requests: self.pull_requests,
        }
    }
}

/// `review_result` rework signal: payload boolean field `rework`.
fn rework_requested(payload: &serde_json::Value) -> bool {
    payload.get("rework").and_then(serde_json::Value::as_bool) == Some(true)
}

/// String field of a payload object, empty string treated as absent.
fn payload_str(payload: &serde_json::Value, key: &str) -> Option<String> {
    payload
        .get(key)
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// Integer field of a payload object.
fn payload_u64(payload: &serde_json::Value, key: &str) -> Option<u64> {
    payload.get(key).and_then(serde_json::Value::as_u64)
}

/// Append `value` unless already present, preserving first-seen order before
/// the sort-dedup in [`Aggregator::finish`].
fn push_unique<T: PartialEq>(values: &mut Vec<T>, value: T) {
    if !values.contains(&value) {
        values.push(value);
    }
}

fn sort_dedup<T: Ord>(values: &mut Vec<T>) {
    values.sort();
    values.dedup();
}
