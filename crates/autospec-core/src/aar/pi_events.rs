//! Normalized Pi execution events for the routing ledger (issue #3319).
//!
//! Pi emits one JSON object per line while a session runs: session start,
//! model requests, tool calls, edits, test runs, compaction, failures and the
//! final result. Raw Pi lines are shaped by whatever the harness happened to
//! be doing -- they carry different metric keys per event, and a provider that
//! never measured something reports nothing at all. Learning needs one stable
//! row shape instead, so every raw line is normalized into exactly one
//! [`PiEventRecord`] and appended to the same append-only routing ledger the
//! dispatch rows live in (`scripts/routing-ledger.sh`). One ledger, two record
//! types, discriminated by `record_type`; nothing rewrites an existing line.
//!
//! Two invariants make the rows comparable across sessions and runs:
//!
//! 1. **Identity is mandatory.** Every record carries `timestamp`,
//!    `session_id`, `work_item_id` and `agent_role`. A raw event that omits
//!    the session-scoped identity inherits it from the [`SessionIdentity`]
//!    the caller supplies; if the field is still empty the event is rejected
//!    rather than written with a blank, because a row without identity cannot
//!    be correlated back to a work item and silently poisons every aggregate.
//! 2. **Missing metrics are `unknown`, never `0`.** An unobserved metric
//!    serializes as the string `"unknown"`. A `0` is a measurement, and a
//!    average over rows that mix in zeros reads as "the model was fast" or
//!    "the cache never hit" -- the exact conclusion the telemetry exists to
//!    draw.
//!
//! Like the rest of AAR this module is pure: it maps text in, returns records
//! and text out. The caller performs the append. The row contract, the wire
//! round-trip and the audit that reads a ledger back are in
//! [`crate::aar::event_ledger`].

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::aar::metric::{Metric, MetricValue};

/// Schema version stamped on every event row; bumped when a field changes
/// meaning, not when one is added.
pub const EVENT_SCHEMA_VERSION: u32 = 1;

/// The `record_type` value marking a row as an execution event. Dispatch rows
/// written by `scripts/routing-ledger.sh` carry no `record_type` at all and
/// are read as dispatches.
pub const EVENT_RECORD_TYPE: &str = "event";

/// The routing ledger every event row is appended to, relative to the repo
/// root (the path `scripts/routing-ledger.sh` defaults to, overridable there
/// with `--ledger` or `$AUTOSPEC_ROUTING_LEDGER`). One ledger for dispatches
/// and events alike, discriminated by `record_type`.
pub const ROUTING_LEDGER: &str = ".autospec/routing-ledger.jsonl";

/// The harness this adapter normalizes events from.
pub const PI_HARNESS: &str = "pi";

/// The normalized lifecycle vocabulary. Raw Pi names are aliases onto these
/// variants; an unrecognized event name is rejected instead of filed under
/// `Finish`, because a silently dropped event class is a hole in the history.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    /// The session opened.
    SessionStart,
    /// One inference request against the model, with token and timing metrics.
    ModelRequest,
    /// One tool invocation.
    ToolCall,
    /// One file edit.
    FileEdit,
    /// One test command and its counts.
    TestRun,
    /// Context compaction.
    Compaction,
    /// A guardrail, tool or test failure.
    Failure,
    /// The session finished.
    Finish,
}

/// Raw Pi event names mapped onto the canonical vocabulary.
pub const EVENT_ALIASES: &[(&str, EventKind)] = &[
    ("session_start", EventKind::SessionStart),
    ("session.start", EventKind::SessionStart),
    ("session", EventKind::SessionStart),
    ("model_request", EventKind::ModelRequest),
    ("model.request", EventKind::ModelRequest),
    ("model_request_end", EventKind::ModelRequest),
    ("inference", EventKind::ModelRequest),
    ("tool_call", EventKind::ToolCall),
    ("tool_execution_start", EventKind::ToolCall),
    ("tool_execution_end", EventKind::ToolCall),
    ("file_edit", EventKind::FileEdit),
    ("edit", EventKind::FileEdit),
    ("test_run", EventKind::TestRun),
    ("test", EventKind::TestRun),
    ("compaction", EventKind::Compaction),
    ("compact", EventKind::Compaction),
    ("context_compaction", EventKind::Compaction),
    ("failure", EventKind::Failure),
    ("error", EventKind::Failure),
    ("finish", EventKind::Finish),
    ("session_end", EventKind::Finish),
    ("agent_end", EventKind::Finish),
];

impl EventKind {
    /// Every canonical kind. The shell validator keeps a mirror of this list in
    /// `ALLOWED_EVENT_KINDS`; `tests/aar_pi_events_ledger.rs` pins them equal.
    pub const ALL: [EventKind; 8] = [
        EventKind::SessionStart,
        EventKind::ModelRequest,
        EventKind::ToolCall,
        EventKind::FileEdit,
        EventKind::TestRun,
        EventKind::Compaction,
        EventKind::Failure,
        EventKind::Finish,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            EventKind::SessionStart => "session_start",
            EventKind::ModelRequest => "model_request",
            EventKind::ToolCall => "tool_call",
            EventKind::FileEdit => "file_edit",
            EventKind::TestRun => "test_run",
            EventKind::Compaction => "compaction",
            EventKind::Failure => "failure",
            EventKind::Finish => "finish",
        }
    }

    /// Resolve a raw Pi event name through [`EVENT_ALIASES`].
    pub fn from_wire(name: &str) -> Option<EventKind> {
        EVENT_ALIASES
            .iter()
            .find(|(wire, _)| *wire == name)
            .map(|(_, kind)| *kind)
    }
}

/// The session-scoped identity a raw event inherits when it does not repeat
/// it. Pi stamps the identity on the session start and sparsely afterwards.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionIdentity {
    pub session_id: String,
    pub work_item_id: String,
    pub agent_role: String,
    pub harness: String,
}

impl SessionIdentity {
    pub fn new(session_id: &str, work_item_id: &str, agent_role: &str) -> Self {
        SessionIdentity {
            session_id: session_id.to_string(),
            work_item_id: work_item_id.to_string(),
            agent_role: agent_role.to_string(),
            harness: PI_HARNESS.to_string(),
        }
    }
}

/// One normalized routing-ledger row for one Pi event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PiEventRecord {
    pub schema_version: u32,
    pub record_type: String,
    pub event: EventKind,
    /// Position of the event within the replayed session, 1-based.
    pub seq: u64,
    pub timestamp: String,
    pub session_id: String,
    pub work_item_id: String,
    pub agent_role: String,
    pub harness: String,
    /// The routing-ledger dispatch this session belongs to, when known.
    pub dispatch_id: Metric<String>,
    pub model: Metric<String>,
    pub input_tokens: Metric<u64>,
    pub output_tokens: Metric<u64>,
    pub reasoning_tokens: Metric<u64>,
    pub cache_hit_tokens: Metric<u64>,
    pub cache_miss_tokens: Metric<u64>,
    pub ttft_ms: Metric<u64>,
    pub prefill_ms: Metric<u64>,
    pub prefill_tok_s: Metric<f64>,
    pub decode_tok_s: Metric<f64>,
    pub queue_ms: Metric<u64>,
    pub turn_ms: Metric<u64>,
    pub tool_ms: Metric<u64>,
    pub wall_ms: Metric<u64>,
    pub context_used_tokens: Metric<u64>,
    pub context_window_tokens: Metric<u64>,
    pub tool: Metric<String>,
    pub tool_ok: Metric<bool>,
    pub tests_total: Metric<u64>,
    pub tests_failed: Metric<u64>,
    pub repair_count: Metric<u64>,
    pub success: Metric<bool>,
    pub failure_category: Metric<String>,
}

impl PiEventRecord {
    /// A record carrying only the mandatory identity, every metric unknown.
    pub fn new(kind: EventKind, seq: u64, timestamp: &str, identity: &SessionIdentity) -> Self {
        PiEventRecord {
            schema_version: EVENT_SCHEMA_VERSION,
            record_type: EVENT_RECORD_TYPE.to_string(),
            event: kind,
            seq,
            timestamp: timestamp.to_string(),
            session_id: identity.session_id.clone(),
            work_item_id: identity.work_item_id.clone(),
            agent_role: identity.agent_role.clone(),
            harness: identity.harness.clone(),
            dispatch_id: Metric::unknown(),
            model: Metric::unknown(),
            input_tokens: Metric::unknown(),
            output_tokens: Metric::unknown(),
            reasoning_tokens: Metric::unknown(),
            cache_hit_tokens: Metric::unknown(),
            cache_miss_tokens: Metric::unknown(),
            ttft_ms: Metric::unknown(),
            prefill_ms: Metric::unknown(),
            prefill_tok_s: Metric::unknown(),
            decode_tok_s: Metric::unknown(),
            queue_ms: Metric::unknown(),
            turn_ms: Metric::unknown(),
            tool_ms: Metric::unknown(),
            wall_ms: Metric::unknown(),
            context_used_tokens: Metric::unknown(),
            context_window_tokens: Metric::unknown(),
            tool: Metric::unknown(),
            tool_ok: Metric::unknown(),
            tests_total: Metric::unknown(),
            tests_failed: Metric::unknown(),
            repair_count: Metric::unknown(),
            success: Metric::unknown(),
            failure_category: Metric::unknown(),
        }
    }
}

/// Normalize one raw Pi event into a ledger row.
///
/// Unknown *keys* are ignored -- Pi adds fields and a new field must never
/// break the ledger -- but an unknown *event name* is rejected, because an
/// unrecognized event class silently disappearing is a hole in the history.
pub fn normalize_event(
    raw: &Value,
    identity: &SessionIdentity,
    seq: u64,
) -> Result<PiEventRecord, String> {
    let name = first_text(raw, &["event", "type", "kind"])
        .ok_or_else(|| "pi event carries no event name".to_string())?;
    let kind =
        EventKind::from_wire(&name).ok_or_else(|| format!("unknown pi event name: {}", name))?;

    let timestamp = first_text(raw, &["timestamp", "ts"])
        .ok_or_else(|| format!("{} event carries no timestamp", kind.as_str()))?;

    let mut record = PiEventRecord::new(kind, seq, &timestamp, &inherit(raw, identity));
    record.dispatch_id = first_metric(raw, &["dispatch_id"]);
    record.model = first_metric(raw, &["model", "model_id"]);
    record.input_tokens = first_metric(raw, &["input_tokens", "prompt_tokens"]);
    record.output_tokens = first_metric(raw, &["output_tokens", "completion_tokens"]);
    record.reasoning_tokens = first_metric(raw, &["reasoning_tokens", "thinking_tokens"]);
    record.cache_hit_tokens = first_metric(
        raw,
        &[
            "cache_hit_tokens",
            "cached_tokens",
            "cache_read_input_tokens",
        ],
    );
    record.cache_miss_tokens =
        first_metric(raw, &["cache_miss_tokens", "cache_creation_input_tokens"]);
    record.ttft_ms = first_metric(raw, &["ttft_ms", "time_to_first_token_ms"]);
    record.prefill_ms = first_metric(raw, &["prefill_ms"]);
    record.prefill_tok_s = first_metric(raw, &["prefill_tok_s", "prefill_tokens_per_second"]);
    record.decode_tok_s = first_metric(raw, &["decode_tok_s", "decode_tokens_per_second"]);
    record.queue_ms = first_metric(raw, &["queue_ms"]);
    record.turn_ms = first_metric(raw, &["turn_ms"]);
    record.tool_ms = first_metric(raw, &["tool_ms", "tool_duration_ms"]);
    record.wall_ms = first_metric(raw, &["wall_ms"]);
    record.context_used_tokens = first_metric(raw, &["context_used_tokens", "context_used"]);
    record.context_window_tokens = first_metric(raw, &["context_window_tokens", "context_window"]);
    record.tool = first_metric(raw, &["tool", "tool_name"]);
    record.tool_ok = first_metric(raw, &["tool_ok"]);
    record.tests_total = first_metric(raw, &["tests_total", "test_total"]);
    record.tests_failed = first_metric(raw, &["tests_failed", "test_failed"]);
    record.repair_count = first_metric(raw, &["repair_count"]);
    record.success = first_metric(raw, &["success"]);
    record.failure_category = first_metric(raw, &["failure_category", "category"]);

    record.validate().map(|_| record)
}

/// Fill any session-scoped identity field the raw event left out.
fn inherit(raw: &Value, identity: &SessionIdentity) -> SessionIdentity {
    SessionIdentity {
        session_id: text_or(raw, &["session_id"], &identity.session_id),
        work_item_id: text_or(raw, &["work_item_id", "issue"], &identity.work_item_id),
        agent_role: text_or(raw, &["agent_role", "role"], &identity.agent_role),
        harness: text_or(raw, &["harness"], &identity.harness),
    }
}

/// Normalize a whole Pi JSONL transcript. Blank lines are skipped; a bad line
/// names its line number, because a half-normalized session is worse than a
/// rejected replay.
pub fn normalize_jsonl(
    text: &str,
    identity: &SessionIdentity,
) -> Result<Vec<PiEventRecord>, String> {
    let mut records = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let raw: Value = serde_json::from_str(line)
            .map_err(|e| format!("line {}: malformed pi event json: {}", index + 1, e))?;
        records.push(
            normalize_event(&raw, identity, (records.len() + 1) as u64)
                .map_err(|e| format!("line {}: {}", index + 1, e))?,
        );
    }
    if records.is_empty() {
        return Err("pi transcript contains no events".to_string());
    }
    Ok(records)
}

/// First present key, as a trimmed string (numbers and bools are stringified
/// so a raw epoch or id survives verbatim).
pub(crate) fn first_text(raw: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| match raw.get(*key) {
        Some(Value::String(text)) if !text.trim().is_empty() => Some(text.trim().to_string()),
        Some(Value::Number(number)) => Some(number.to_string()),
        Some(Value::Bool(flag)) => Some(flag.to_string()),
        _ => None,
    })
}

/// A session-scoped identity field: the event's own value if it repeated it,
/// otherwise the session's.
fn text_or(raw: &Value, keys: &[&str], fallback: &str) -> String {
    first_text(raw, keys).unwrap_or_else(|| fallback.to_string())
}

/// The first key the event reports, read as a metric of `T`.
fn first_metric<T: MetricValue>(raw: &Value, keys: &[&str]) -> Metric<T> {
    keys.iter()
        .find_map(|key| raw.get(*key))
        .and_then(MetricValue::from_json)
        .into()
}
