//! Context waste metrics (§16) and context graph prototype (§15) for the
//! continuous-improvement engine
//! (`docs/specs/2026-09-08-continuous-improvement-engine.md`).
//!
//! [`context_metrics`] is a pure, deterministic reducer over one session's
//! [`NormalizedEvent`] slice (spec §4.1 deterministic-first): no I/O, no
//! clock reads, and the same slice always yields the same value (spec §51
//! reproducibility). All 10 §16 counters are comparative signals — "use
//! this comparatively, not as an absolute truth" (§16) — never an absolute
//! quality judgment.
//!
//! The one useful/unnecessary distinction is spelled out by the issue
//! contract: **a read is useful when the path later appears in a write or
//! patch.** Everything else is a small, documented deterministic rule:
//!
//! | §16 counter | deterministic rule |
//! |---|---|
//! | files read but never referenced again | a `file_read` whose path appears in no later `file_write` / `file_patch` |
//! | directories enumerated without downstream use | a `command_run` whose payload carries a `directory` and no later `file_write` / `file_patch` touches a path under it |
//! | git history searches without impact | a `command_run` whose command is `git log` / `git blame` / `git rev-list` and no later `file_write` / `file_patch` touches the search's payload `path` |
//! | repeated file reads | per path, reads beyond the first (`count - 1`); a path read repeatedly counts its repeats once each |
//! | redundant tool invocations | a `tool_call` whose `(tool, payload)` pair was already seen earlier in the session |
//! | context added before first useful edit | input tokens of every event before the first useful edit; all input tokens when there is none |
//! | context compactions | count of `context_compaction` events |
//! | context exhaustion | count of `context_limit_warning` events |
//! | token count at first correct implementation | cumulative input tokens up to and including the first passing `test_result` (payload `passed` or `success` is true); 0 when none |
//! | proportion of context associated with changed or referenced code | [`ContextMetrics::context_efficiency`] = `useful_context_tokens / total_context_tokens`, where useful context is the input tokens of useful reads plus of every `file_write` / `file_patch`, and total context is every event's input tokens |
//!
//! §15 graph: [`build_graph`] re-aggregates the stored sessions of one
//! repository (issue #3827 tables: `sessions`, `session_events`,
//! `session_summaries`) into a per-task-class map of useful and
//! unnecessary paths. It is read-only and idempotent — ingestion appends
//! events incrementally, and the graph is rebuilt by re-aggregation over
//! the stored rows, so a rebuild is always safe and cheap. Stored file
//! paths are repo-relative strings already present in the event payloads;
//! this module never logs them and adds no access control of its own
//! (§39 privacy lives in the storage layer).
//!
//! The graph is a prototype only: nothing here feeds a Scout agent, tool
//! or model analytics, or any rendering (all out of scope for this issue).

use std::collections::{BTreeMap, BTreeSet};

use sqlx::{AnyPool, Row};

use crate::error::AutospecError;
use crate::insights::events::{EventType, NormalizedEvent, Tokens};

/// The 10 §16 context waste counters for one session, plus the components
/// of [`Self::context_efficiency`] so the ratio stays reproducible for one
/// input.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContextMetrics {
    /// 1. files read but never referenced again.
    pub files_read_unused: u64,
    /// 2. directories enumerated without downstream use.
    pub directories_enumerated_unused: u64,
    /// 3. git history searches without impact.
    pub git_history_searches_unused: u64,
    /// 4. repeated file reads (per path, reads beyond the first).
    pub repeated_file_reads: u64,
    /// 5. redundant tool invocations (exact `(tool, payload)` repeats).
    pub redundant_tool_invocations: u64,
    /// 6. context (input tokens) added before the first useful edit.
    pub context_before_first_useful_edit_tokens: u64,
    /// 7. context compactions.
    pub context_compactions: u64,
    /// 8. context exhaustion.
    pub context_exhaustions: u64,
    /// 9. token count (input tokens) at the first correct implementation.
    pub tokens_at_first_correct_implementation: u64,
    /// 10. proportion of context associated with changed or referenced
    ///     code: `useful_context_tokens / total_context_tokens`, always
    ///     within 0.0 and 1.0 (0.0 when the session carried no context
    ///     tokens).
    pub context_efficiency: f64,
    /// Input tokens of useful reads plus of every write/patch.
    pub useful_context_tokens: u64,
    /// Input tokens of every event (the context fed to the model).
    pub total_context_tokens: u64,
}

impl Default for ContextMetrics {
    fn default() -> Self {
        Self {
            files_read_unused: 0,
            directories_enumerated_unused: 0,
            git_history_searches_unused: 0,
            repeated_file_reads: 0,
            redundant_tool_invocations: 0,
            context_before_first_useful_edit_tokens: 0,
            context_compactions: 0,
            context_exhaustions: 0,
            tokens_at_first_correct_implementation: 0,
            context_efficiency: 0.0,
            useful_context_tokens: 0,
            total_context_tokens: 0,
        }
    }
}

/// One task class's share of the §15 context graph: the paths that proved
/// useful (later written or patched after being read) and the paths that
/// were read and never used, each with the number of read events that
/// contributed. `BTree*` keeps the ordering — and therefore the whole
/// graph — deterministic for one stored input.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TaskClassContext {
    /// Sessions of this task class aggregated into the graph.
    pub sessions: u64,
    /// Path -> useful read events across the aggregated sessions.
    pub useful_paths: BTreeMap<String, u64>,
    /// Path -> read events that were never used across the aggregated
    /// sessions.
    pub unnecessary_paths: BTreeMap<String, u64>,
}

/// The §15 context graph prototype: task class -> useful/unnecessary paths.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContextGraph {
    pub by_task_class: BTreeMap<String, TaskClassContext>,
}

fn is_read(event: &NormalizedEvent) -> bool {
    matches!(event.event_type, EventType::FileRead)
}

fn is_edit(event: &NormalizedEvent) -> bool {
    matches!(
        event.event_type,
        EventType::FileWrite | EventType::FilePatch
    )
}

/// The `path` payload field of a file read/write/patch event.
fn payload_path(event: &NormalizedEvent) -> Option<&str> {
    event
        .payload
        .get("path")
        .and_then(serde_json::Value::as_str)
}

/// A directory enumeration: a `command_run` whose payload names the
/// enumerated `directory`.
fn enumerated_directory(event: &NormalizedEvent) -> Option<&str> {
    (event.event_type == EventType::CommandRun)
        .then(|| {
            event
                .payload
                .get("directory")
                .and_then(serde_json::Value::as_str)
        })
        .flatten()
}

/// A git history search: a `command_run` of `git log`, `git blame`, or
/// `git rev-list`. Returns the search's `path` payload when present.
fn git_history_search_path(event: &NormalizedEvent) -> Option<Option<&str>> {
    if event.event_type != EventType::CommandRun {
        return None;
    }
    let command = event
        .payload
        .get("command")
        .and_then(serde_json::Value::as_str)?;
    let mut tokens = command.split_whitespace();
    if tokens.next() != Some("git") {
        return None;
    }
    match tokens.next() {
        Some("log") | Some("blame") | Some("rev-list") => {}
        _ => return None,
    }
    Some(
        event
            .payload
            .get("path")
            .and_then(serde_json::Value::as_str),
    )
}

/// True when `path` is the directory itself or lives underneath it.
fn path_under_directory(path: &str, directory: &str) -> bool {
    if path == directory {
        return true;
    }
    let prefix = if directory.ends_with('/') {
        directory.to_string()
    } else {
        format!("{directory}/")
    };
    path.starts_with(&prefix)
}

/// A passing `test_result` marks the first correct implementation.
fn is_passing_test_result(event: &NormalizedEvent) -> bool {
    if event.event_type != EventType::TestResult {
        return false;
    }
    let payload = &event.payload;
    payload
        .get("passed")
        .or_else(|| payload.get("success"))
        .and_then(serde_json::Value::as_bool)
        == Some(true)
}

/// Compute the 10 §16 context waste counters for one session's events, in
/// the given (chronological) order. Pure: the same slice produces an
/// identical [`ContextMetrics`] value.
pub fn context_metrics(events: &[NormalizedEvent]) -> ContextMetrics {
    // Usefulness pass: a read is useful when the path later appears in a
    // write or patch; a write/patch is a useful edit when the path was
    // read earlier.
    let mut useful_read = vec![false; events.len()];
    let mut useful_edit = vec![false; events.len()];
    for i in 0..events.len() {
        if is_read(&events[i]) {
            if let Some(path) = payload_path(&events[i]) {
                useful_read[i] = events[i + 1..]
                    .iter()
                    .any(|later| is_edit(later) && payload_path(later) == Some(path));
            }
        }
        if is_edit(&events[i]) {
            if let Some(path) = payload_path(&events[i]) {
                useful_edit[i] = events[..i]
                    .iter()
                    .any(|earlier| is_read(earlier) && payload_path(earlier) == Some(path));
            }
        }
    }

    let mut metrics = ContextMetrics::default();
    let mut read_counts: BTreeMap<&str, u64> = BTreeMap::new();
    let mut seen_tool_calls: BTreeSet<String> = BTreeSet::new();
    let mut cumulative_input: u64 = 0;
    let mut first_useful_edit_index: Option<usize> = None;
    let mut first_correct_implementation_tokens: Option<u64> = None;

    for (i, event) in events.iter().enumerate() {
        cumulative_input = cumulative_input.saturating_add(event.tokens.input);
        metrics.total_context_tokens = metrics
            .total_context_tokens
            .saturating_add(event.tokens.input);

        match event.event_type {
            EventType::FileRead => {
                if let Some(path) = payload_path(event) {
                    *read_counts.entry(path).or_insert(0) += 1;
                    if useful_read[i] {
                        metrics.useful_context_tokens = metrics
                            .useful_context_tokens
                            .saturating_add(event.tokens.input);
                    } else {
                        metrics.files_read_unused += 1;
                    }
                }
            }
            EventType::FileWrite | EventType::FilePatch => {
                metrics.useful_context_tokens = metrics
                    .useful_context_tokens
                    .saturating_add(event.tokens.input);
                if useful_edit[i] && first_useful_edit_index.is_none() {
                    first_useful_edit_index = Some(i);
                }
            }
            EventType::ContextCompaction => metrics.context_compactions += 1,
            EventType::ContextLimitWarning => metrics.context_exhaustions += 1,
            EventType::ToolCall => {
                let key = format!(
                    "{}\u{0}{}",
                    event.tool.as_deref().unwrap_or(""),
                    serde_json::to_string(&event.payload).unwrap_or_default()
                );
                if !seen_tool_calls.insert(key) {
                    metrics.redundant_tool_invocations += 1;
                }
            }
            _ => {}
        }

        if let Some(directory) = enumerated_directory(event) {
            let used = events[i + 1..].iter().any(|later| {
                is_edit(later)
                    && payload_path(later).is_some_and(|path| path_under_directory(path, directory))
            });
            if !used {
                metrics.directories_enumerated_unused += 1;
            }
        }

        if let Some(search_path) = git_history_search_path(event) {
            let impacted = search_path.is_some_and(|path| {
                events[i + 1..]
                    .iter()
                    .any(|later| is_edit(later) && payload_path(later) == Some(path))
            });
            if !impacted {
                metrics.git_history_searches_unused += 1;
            }
        }

        if is_passing_test_result(event) && first_correct_implementation_tokens.is_none() {
            first_correct_implementation_tokens = Some(cumulative_input);
        }
    }

    metrics.repeated_file_reads = read_counts
        .values()
        .map(|&count| count.saturating_sub(1))
        .sum();
    metrics.context_before_first_useful_edit_tokens = match first_useful_edit_index {
        Some(index) => events[..index].iter().map(|event| event.tokens.input).sum(),
        None => metrics.total_context_tokens,
    };
    metrics.tokens_at_first_correct_implementation =
        first_correct_implementation_tokens.unwrap_or(0);
    metrics.context_efficiency = if metrics.total_context_tokens == 0 {
        0.0
    } else {
        (metrics.useful_context_tokens as f64 / metrics.total_context_tokens as f64).clamp(0.0, 1.0)
    };
    metrics
}

/// Rebuild the §15 context graph over the stored sessions of `repo`:
/// `sessions` (issue #3827) selects the sessions, `session_events` supplies
/// the events (payload JSON per row), and `session_summaries.task_type`
/// supplies the task class (a session with no stored summary is classed
/// `unknown`). Read-only and idempotent — the graph is a re-aggregation, so
/// rebuilding over the same stored rows always yields the same graph.
pub async fn build_graph(pool: &AnyPool, repo: &str) -> Result<ContextGraph, AutospecError> {
    if repo.trim().is_empty() {
        return Err(AutospecError::validation("repo must be a non-empty string"));
    }

    let session_ids: Vec<String> =
        sqlx::query("SELECT id FROM sessions WHERE repo = $1 ORDER BY id")
            .bind(repo)
            .fetch_all(pool)
            .await
            .map_err(|error| AutospecError::state("insights.sessions", error.to_string()))?
            .into_iter()
            .map(|row| row.try_get::<String, _>(0).unwrap_or_default())
            .collect();

    let mut graph = ContextGraph::default();
    for session_id in session_ids {
        let events = read_session_events(pool, &session_id)
            .await
            .map_err(|error| AutospecError::state("insights.session_events", error.to_string()))?;
        // A missing summary degrades the class to `unknown`; it never
        // drops the session from the graph.
        let task_class = read_task_class(pool, &session_id).await;
        let class = graph
            .by_task_class
            .entry(task_class)
            .or_insert_with(TaskClassContext::default);
        class.sessions += 1;
        // The per-session projection of the §16 useful/unused read rule,
        // aggregated into the graph.
        for (path, useful) in useful_and_unused_paths(&events) {
            if useful {
                *class.useful_paths.entry(path).or_insert(0) += 1;
            } else {
                *class.unnecessary_paths.entry(path).or_insert(0) += 1;
            }
        }
    }
    Ok(graph)
}

/// Every read path of the session, with whether it later appears in a
/// write or patch. Each read event contributes one count.
fn useful_and_unused_paths(events: &[NormalizedEvent]) -> Vec<(String, bool)> {
    events
        .iter()
        .enumerate()
        .filter(|(_, event)| is_read(event))
        .filter_map(|(i, event)| {
            payload_path(event).map(|path| {
                (
                    path.to_string(),
                    events[i + 1..]
                        .iter()
                        .any(|later| is_edit(later) && payload_path(later) == Some(path)),
                )
            })
        })
        .collect()
}

/// Stored events of one session, in `seq` order, reconstructed as
/// [`NormalizedEvent`]s. A row whose `event_type` or payload does not parse
/// degrades to a skipped row / empty payload — it never drops the session.
/// Tokens, when the ingestion stored them in the payload, come from the
/// `tokens` object; otherwise they are 0.
async fn read_session_events(
    pool: &AnyPool,
    session_id: &str,
) -> Result<Vec<NormalizedEvent>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT event_type, payload FROM session_events \
         WHERE session_id = $1 ORDER BY seq",
    )
    .bind(session_id)
    .fetch_all(pool)
    .await?;

    let mut events = Vec::with_capacity(rows.len());
    for row in rows {
        let event_type = row.try_get::<String, _>(0)?;
        let payload_text = row
            .try_get::<Option<String>, _>(1)?
            .unwrap_or_else(|| "{}".into());
        let event_type: EventType = match serde_json::from_str(&format!("\"{event_type}\"")) {
            Ok(event_type) => event_type,
            Err(_) => continue,
        };
        let payload = serde_json::from_str(&payload_text)
            .unwrap_or_else(|_| serde_json::Value::Object(Default::default()));
        let tokens = payload
            .get("tokens")
            .map(|tokens| Tokens {
                input: tokens
                    .get("input")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0),
                output: tokens
                    .get("output")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0),
            })
            .unwrap_or_default();
        events.push(NormalizedEvent {
            event_id: format!("{session_id}:{}", events.len()),
            session_id: session_id.to_string(),
            parent_session_id: None,
            timestamp: 0.0,
            repo: None,
            branch: None,
            work_item_id: None,
            agent_role: None,
            provider: None,
            model: None,
            event_type,
            tool: None,
            payload,
            tokens,
        });
    }
    Ok(events)
}

/// The stored `session_summaries.task_type`, or `unknown` when the session
/// has no summary (or the lookup degrades).
async fn read_task_class(pool: &AnyPool, session_id: &str) -> String {
    sqlx::query("SELECT task_type FROM session_summaries WHERE session_id = $1")
        .bind(session_id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .and_then(|row| row.try_get::<Option<String>, _>(0).ok().flatten())
        .unwrap_or_else(|| "unknown".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::insights::events::EventType;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Mutex;

    fn event(
        index: u32,
        session_id: &str,
        event_type: EventType,
        tool: Option<&str>,
        tokens: Tokens,
        payload: serde_json::Value,
    ) -> NormalizedEvent {
        NormalizedEvent {
            event_id: format!("evt-{index}"),
            session_id: session_id.to_string(),
            parent_session_id: None,
            timestamp: index as f64,
            repo: Some("fixture/repo".into()),
            branch: None,
            work_item_id: None,
            agent_role: None,
            provider: None,
            model: None,
            event_type,
            tool: tool.map(str::to_string),
            payload,
            tokens,
        }
    }

    fn tokens(input: u64) -> Tokens {
        Tokens { input, output: 0 }
    }

    /// The required local fixture: 12 reads and 3 writes. The 3 writes hit
    /// 3 of the read paths (after the reads), so 9 reads are unused.
    fn fixture_12_reads_3_writes() -> Vec<NormalizedEvent> {
        let s = "session-waste";
        let mut events = Vec::with_capacity(15);
        for i in 0..12u32 {
            events.push(event(
                i,
                s,
                EventType::FileRead,
                None,
                tokens(100),
                serde_json::json!({"path": format!("src/read-{i}.rs")}),
            ));
        }
        for (i, name) in ["read-0", "read-1", "read-2"].into_iter().enumerate() {
            events.push(event(
                12 + i as u32,
                s,
                EventType::FileWrite,
                None,
                tokens(50),
                serde_json::json!({"path": format!("src/{name}.rs")}),
            ));
        }
        assert_eq!(events.len(), 15);
        events
    }

    #[test]
    fn fixture_with_12_reads_and_3_writes_yields_9_unused_reads() {
        let metrics = context_metrics(&fixture_12_reads_3_writes());
        assert_eq!(metrics.files_read_unused, 9);
        assert_eq!(metrics.repeated_file_reads, 0);
        assert!(
            (0.0..=1.0).contains(&metrics.context_efficiency),
            "context_efficiency must stay within 0.0 and 1.0"
        );
    }

    #[test]
    fn repeated_reads_of_one_path_count_once_as_repeated() {
        let s = "session-repeat";
        let events = vec![
            event(
                0,
                s,
                EventType::FileRead,
                None,
                tokens(0),
                serde_json::json!({"path": "src/a.rs"}),
            ),
            event(
                1,
                s,
                EventType::FileRead,
                None,
                tokens(0),
                serde_json::json!({"path": "src/a.rs"}),
            ),
            event(
                2,
                s,
                EventType::FileWrite,
                None,
                tokens(0),
                serde_json::json!({"path": "src/a.rs"}),
            ),
        ];
        let metrics = context_metrics(&events);
        assert_eq!(metrics.repeated_file_reads, 1);
        // Both reads precede the write: neither is unused.
        assert_eq!(metrics.files_read_unused, 0);
    }

    #[test]
    fn same_event_slice_produces_identical_metrics_twice() {
        let events = fixture_12_reads_3_writes();
        let first = context_metrics(&events);
        let second = context_metrics(&events);
        assert_eq!(first, second);
    }

    #[test]
    fn context_efficiency_is_useful_over_total_tokens() {
        let events = fixture_12_reads_3_writes();
        let metrics = context_metrics(&events);
        // Total: 12 reads x 100 + 3 writes x 50 = 1350 input tokens.
        assert_eq!(metrics.total_context_tokens, 1_350);
        // Useful: 3 useful reads x 100 + 3 writes x 50 = 450.
        assert_eq!(metrics.useful_context_tokens, 450);
        assert!((metrics.context_efficiency - 1.0 / 3.0).abs() < 1e-9);
    }

    #[test]
    fn read_after_the_last_write_is_not_useful() {
        let s = "session-order";
        let events = vec![
            event(
                0,
                s,
                EventType::FileWrite,
                None,
                tokens(0),
                serde_json::json!({"path": "src/a.rs"}),
            ),
            event(
                1,
                s,
                EventType::FileRead,
                None,
                tokens(0),
                serde_json::json!({"path": "src/a.rs"}),
            ),
        ];
        let metrics = context_metrics(&events);
        assert_eq!(metrics.files_read_unused, 1);
    }

    #[test]
    fn empty_session_is_all_zero_with_zero_efficiency() {
        let metrics = context_metrics(&[]);
        assert_eq!(metrics, ContextMetrics::default());
        assert_eq!(metrics.context_efficiency, 0.0);
    }

    #[test]
    fn compaction_exhaustion_and_correct_implementation_counters() {
        let s = "session-tokens";
        let events = vec![
            event(
                0,
                s,
                EventType::ContextCompaction,
                None,
                tokens(10),
                serde_json::json!({}),
            ),
            event(
                1,
                s,
                EventType::ContextLimitWarning,
                None,
                tokens(20),
                serde_json::json!({}),
            ),
            event(
                2,
                s,
                EventType::FileRead,
                None,
                tokens(30),
                serde_json::json!({"path": "src/a.rs"}),
            ),
            event(
                3,
                s,
                EventType::FileWrite,
                None,
                tokens(40),
                serde_json::json!({"path": "src/a.rs"}),
            ),
            event(
                4,
                s,
                EventType::TestResult,
                None,
                tokens(50),
                serde_json::json!({"passed": false}),
            ),
            event(
                5,
                s,
                EventType::TestResult,
                None,
                tokens(60),
                serde_json::json!({"passed": true}),
            ),
        ];
        let metrics = context_metrics(&events);
        assert_eq!(metrics.context_compactions, 1);
        assert_eq!(metrics.context_exhaustions, 1);
        // Cumulative input tokens through the first passing test result:
        // 10 + 20 + 30 + 40 + 50 + 60 = 210.
        assert_eq!(metrics.tokens_at_first_correct_implementation, 210);
        // Context before the first useful edit (events 0..3): 10 + 20 + 30.
        assert_eq!(metrics.context_before_first_useful_edit_tokens, 60);
        assert!(
            (0.0..=1.0).contains(&metrics.context_efficiency),
            "context_efficiency must stay within 0.0 and 1.0"
        );
    }

    #[test]
    fn no_passing_test_result_yields_zero_tokens_at_correct_implementation() {
        let s = "session-nopass";
        let events = vec![event(
            0,
            s,
            EventType::TestResult,
            None,
            tokens(50),
            serde_json::json!({"passed": false}),
        )];
        let metrics = context_metrics(&events);
        assert_eq!(metrics.tokens_at_first_correct_implementation, 0);
        // No useful edit: the whole context counts as added before one.
        assert_eq!(metrics.context_before_first_useful_edit_tokens, 50);
    }

    #[test]
    fn redundant_tool_invocations_count_exact_repeats() {
        let s = "session-tools";
        let events = vec![
            event(
                0,
                s,
                EventType::ToolCall,
                Some("shell"),
                tokens(0),
                serde_json::json!({"command": "cargo test"}),
            ),
            event(
                1,
                s,
                EventType::ToolCall,
                Some("shell"),
                tokens(0),
                serde_json::json!({"command": "cargo test"}),
            ),
            event(
                2,
                s,
                EventType::ToolCall,
                Some("shell"),
                tokens(0),
                serde_json::json!({"command": "cargo build"}),
            ),
        ];
        let metrics = context_metrics(&events);
        assert_eq!(metrics.redundant_tool_invocations, 1);
    }

    #[test]
    fn directory_enumeration_is_used_when_a_later_write_lands_under_it() {
        let s = "session-dirs";
        let events = vec![
            event(
                0,
                s,
                EventType::CommandRun,
                Some("shell"),
                tokens(0),
                serde_json::json!({"command": "ls src", "directory": "src"}),
            ),
            event(
                1,
                s,
                EventType::CommandRun,
                Some("shell"),
                tokens(0),
                serde_json::json!({"command": "ls docs", "directory": "docs"}),
            ),
            event(
                2,
                s,
                EventType::FileWrite,
                None,
                tokens(0),
                serde_json::json!({"path": "src/foo.rs"}),
            ),
        ];
        let metrics = context_metrics(&events);
        // `src` is used by the later write; `docs` is not.
        assert_eq!(metrics.directories_enumerated_unused, 1);
    }

    #[test]
    fn git_history_search_has_impact_only_for_the_path_it_searched() {
        let s = "session-git";
        let events = vec![
            event(
                0,
                s,
                EventType::CommandRun,
                Some("shell"),
                tokens(0),
                serde_json::json!({"command": "git log -- src/a.rs", "path": "src/a.rs"}),
            ),
            event(
                1,
                s,
                EventType::CommandRun,
                Some("shell"),
                tokens(0),
                serde_json::json!({"command": "git blame src/b.rs", "path": "src/b.rs"}),
            ),
            event(
                2,
                s,
                EventType::FilePatch,
                None,
                tokens(0),
                serde_json::json!({"path": "src/a.rs"}),
            ),
        ];
        let metrics = context_metrics(&events);
        // The search for a.rs paid off (patched later); b.rs did not.
        assert_eq!(metrics.git_history_searches_unused, 1);
    }

    // ── build_graph over stored sessions (real database, no mocks) ──
    // Disposable PostgreSQL 16 under Apptainer via AUTOSPEC_TEST_DB_URL in
    // the operator full run; a disposable SQLite database otherwise.

    static ENV_LOCK: Mutex<()> = Mutex::new(());
    static FILE_COUNTER: AtomicU32 = AtomicU32::new(0);

    /// The stored schema (issue #3827 tables): `sessions` and
    /// `session_events` from the insights migration DDL, `session_summaries`
    /// in its full classification shape (the write path in
    /// `insights::summarize::store`).
    const SCHEMA: [&str; 3] = [
        "CREATE TABLE IF NOT EXISTS sessions (
            id TEXT PRIMARY KEY,
            repo TEXT NOT NULL,
            work_item_id TEXT,
            harness TEXT,
            model TEXT,
            status TEXT NOT NULL,
            started_at TEXT NOT NULL DEFAULT '2026-09-08T18:00:00Z',
            ended_at TEXT,
            created_at TEXT NOT NULL DEFAULT '2026-09-08T18:00:00Z'
        )",
        "CREATE TABLE IF NOT EXISTS session_events (
            session_id TEXT NOT NULL REFERENCES sessions (id),
            seq INTEGER NOT NULL,
            event_type TEXT NOT NULL,
            occurred_at TEXT NOT NULL DEFAULT '2026-09-08T18:00:00Z',
            payload TEXT,
            PRIMARY KEY (session_id, seq)
        )",
        "CREATE TABLE IF NOT EXISTS session_summaries (
            session_id TEXT NOT NULL REFERENCES sessions (id),
            task_type TEXT,
            PRIMARY KEY (session_id)
        )",
    ];

    fn test_db_url() -> String {
        match std::env::var("AUTOSPEC_TEST_DB_URL") {
            Ok(url) if !url.trim().is_empty() => url,
            _ => {
                let counter = FILE_COUNTER.fetch_add(1, Ordering::SeqCst);
                format!(
                    "sqlite://{}/autospec-insights-context-{}-{counter}.db",
                    std::env::temp_dir().display(),
                    std::process::id()
                )
            }
        }
    }

    async fn open_test_pool() -> AnyPool {
        crate::resources::db::open_shared_db(&test_db_url())
            .await
            .expect("test database must open")
    }

    fn unique_id(kind: &str) -> String {
        let counter = FILE_COUNTER.fetch_add(1, Ordering::SeqCst);
        format!("{kind}-{}-{counter}", std::process::id())
    }

    async fn insert_session(pool: &AnyPool, session_id: &str, repo: &str, task_type: Option<&str>) {
        sqlx::query("INSERT INTO sessions (id, repo, status) VALUES ($1, $2, 'completed')")
            .bind(session_id)
            .bind(repo)
            .execute(pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO session_summaries (session_id, task_type) VALUES ($1, $2)")
            .bind(session_id)
            .bind(task_type)
            .execute(pool)
            .await
            .unwrap();
    }

    async fn insert_event(
        pool: &AnyPool,
        session_id: &str,
        seq: i64,
        event_type: &str,
        payload: &str,
    ) {
        sqlx::query(
            "INSERT INTO session_events (session_id, seq, event_type, payload) \
             VALUES ($1, $2, $3, $4)",
        )
        .bind(session_id)
        .bind(seq)
        .bind(event_type)
        .bind(payload)
        .execute(pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn build_graph_returns_useful_and_unnecessary_paths_per_task_class() {
        let _guard = ENV_LOCK.lock().unwrap();
        let pool = open_test_pool().await;
        for statement in SCHEMA {
            sqlx::query(statement).execute(&pool).await.unwrap();
        }
        let repo = unique_id("repo");
        let session_a = unique_id("session-a");
        let session_b = unique_id("session-b");
        let other = unique_id("other-repo");

        // implementation session: reads controller + dto; only the
        // controller is written later.
        insert_session(&pool, &session_a, &repo, Some("implementation")).await;
        insert_event(
            &pool,
            &session_a,
            1,
            "file_read",
            r#"{"path": "src/controller.rs", "tokens": {"input": 100, "output": 0}}"#,
        )
        .await;
        insert_event(
            &pool,
            &session_a,
            2,
            "file_read",
            r#"{"path": "src/dto.rs", "tokens": {"input": 80, "output": 0}}"#,
        )
        .await;
        insert_event(
            &pool,
            &session_a,
            3,
            "file_write",
            r#"{"path": "src/controller.rs", "tokens": {"input": 40, "output": 0}}"#,
        )
        .await;

        // debugging session with no stored summary row: class `unknown`;
        // its single read is never written.
        sqlx::query("INSERT INTO sessions (id, repo, status) VALUES ($1, $2, 'completed')")
            .bind(&session_b)
            .bind(&repo)
            .execute(&pool)
            .await
            .unwrap();
        insert_event(
            &pool,
            &session_b,
            1,
            "file_read",
            r#"{"path": "src/logs.rs", "tokens": {"input": 60, "output": 0}}"#,
        )
        .await;

        // A session of another repository must not leak into the graph.
        let other_session = unique_id("session-other");
        insert_session(&pool, &other_session, &other, Some("implementation")).await;
        insert_event(
            &pool,
            &other_session,
            1,
            "file_read",
            r#"{"path": "src/other.rs"}"#,
        )
        .await;

        let graph = build_graph(&pool, &repo).await.unwrap();
        assert_eq!(graph.by_task_class.len(), 2);

        let implementation = &graph.by_task_class["implementation"];
        assert_eq!(implementation.sessions, 1);
        assert_eq!(
            implementation.useful_paths.get("src/controller.rs"),
            Some(&1)
        );
        assert_eq!(implementation.useful_paths.len(), 1);
        assert_eq!(implementation.unnecessary_paths.get("src/dto.rs"), Some(&1));
        assert_eq!(implementation.unnecessary_paths.len(), 1);

        let unknown = &graph.by_task_class["unknown"];
        assert_eq!(unknown.sessions, 1);
        assert!(unknown.useful_paths.is_empty());
        assert_eq!(unknown.unnecessary_paths.get("src/logs.rs"), Some(&1));
    }

    #[tokio::test]
    async fn build_graph_is_reproducible_for_one_stored_input() {
        let _guard = ENV_LOCK.lock().unwrap();
        let pool = open_test_pool().await;
        for statement in SCHEMA {
            sqlx::query(statement).execute(&pool).await.unwrap();
        }
        let repo = unique_id("repo");
        let session_id = unique_id("session");
        insert_session(&pool, &session_id, &repo, Some("plan")).await;
        insert_event(
            &pool,
            &session_id,
            1,
            "file_read",
            r#"{"path": "docs/arch.md"}"#,
        )
        .await;
        insert_event(
            &pool,
            &session_id,
            2,
            "file_write",
            r#"{"path": "docs/arch.md"}"#,
        )
        .await;

        let first = build_graph(&pool, &repo).await.unwrap();
        let second = build_graph(&pool, &repo).await.unwrap();
        assert_eq!(first, second);
        assert_eq!(
            first.by_task_class["plan"].useful_paths.get("docs/arch.md"),
            Some(&1)
        );
    }

    #[tokio::test]
    async fn build_graph_rejects_an_empty_repo() {
        let _guard = ENV_LOCK.lock().unwrap();
        let pool = open_test_pool().await;
        assert!(build_graph(&pool, "").await.is_err());
        assert!(build_graph(&pool, "   ").await.is_err());
    }
}
