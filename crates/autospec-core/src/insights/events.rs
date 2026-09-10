//! Normalized session event model (spec §7; dependency issue #3826).
//!
//! One row per normalized event, produced by the deterministic Stage 1
//! extraction. No semantic classification happens here (spec §4.1).

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::AutospecError;

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

impl EventType {
    /// Parse one of the 29 spec §7 event type names (snake_case).
    ///
    /// Unrecognised names are an [`AutospecError`], so adapters fail closed
    /// instead of silently dropping or mis-bucketing events.
    pub fn parse(name: &str) -> Result<Self, AutospecError> {
        serde_json::from_value(serde_json::Value::String(name.to_string())).map_err(|err| {
            AutospecError::parse(
                "event_type",
                format!("unrecognised event type {name:?}: {err}"),
            )
        })
    }
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
    #[serde(deserialize_with = "deserialize_timestamp")]
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

/// A reference to a session source for incremental ingestion (spec §6):
/// where to read, and where the previous ingestion run left off.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionRef {
    /// Source kind (e.g. `pi_jsonl`, `autospec_orchestration`).
    pub source: String,
    /// Location of the raw session records.
    pub uri: String,
    /// Resume cursor from the last ingestion run; `None` means start at the
    /// beginning. Adapters define cursor semantics.
    pub resume_cursor: Option<String>,
}

/// A session source reader (spec §6 `SessionAdapter`).
///
/// Concrete adapters (Pi JSONL, AutoSpec orchestration/CI events, Git/PR
/// metadata) live in their own issues; this trait fixes the contract.
/// Raw events are opaque JSON values — the adapter, not the model, knows
/// their shape.
pub trait SessionAdapter {
    /// Discover the session sources this adapter can read (spec §6 `Discover`).
    fn discover(&self) -> Result<Vec<SessionRef>, AutospecError>;
    /// Read the raw session events for one source reference (spec §6 `Read`).
    fn read(&self, source: &SessionRef) -> Result<Vec<serde_json::Value>, AutospecError>;
    /// Normalize one raw event into zero or more [`NormalizedEvent`]s
    /// (spec §6 `Normalize`).
    fn normalize(&self, event: &serde_json::Value) -> Result<Vec<NormalizedEvent>, AutospecError>;
}

/// Extractor schema version for reproducibility (spec §52).
///
/// Bump when any [`NormalizedEvent`] field or [`EventType`] name changes.
pub const EXTRACTOR_VERSION: &str = "1.2.0";

/// Deserialize `timestamp` from either a Unix-seconds number or an RFC 3339
/// string (as in the spec §7 example, `"2026-09-08T18:00:00Z"`),
/// normalizing to Unix seconds so reducers can compute durations
/// deterministically without a date library.
fn deserialize_timestamp<'de, D>(deserializer: D) -> Result<f64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct TimestampVisitor;

    impl serde::de::Visitor<'_> for TimestampVisitor {
        type Value = f64;

        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("a Unix-seconds number or an RFC 3339 timestamp string")
        }

        fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            Ok(v as f64)
        }

        fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            Ok(v as f64)
        }

        fn visit_f64<E>(self, v: f64) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            Ok(v)
        }

        fn visit_str<E>(self, s: &str) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            rfc3339_to_unix_seconds(s).map_err(serde::de::Error::custom)
        }
    }

    deserializer.deserialize_any(TimestampVisitor)
}

/// Parse an RFC 3339 timestamp (`YYYY-MM-DDTHH:MM:SS[.frac][Z|±HH:MM]`) to
/// Unix seconds. No date library in the dependency set (issue #3826 is
/// std+serde-only), so the calendar math is inline and unit-tested.
fn rfc3339_to_unix_seconds(s: &str) -> Result<f64, String> {
    let s = s.trim();
    let (date, time) = s
        .split_once(['T', 't', ' '])
        .ok_or_else(|| format!("missing date/time separator in {s:?}"))?;
    let date_b = date.as_bytes();
    if date.len() != 10 || date_b[4] != b'-' || date_b[7] != b'-' {
        return Err(format!("malformed date in {s:?}"));
    }
    let year: i64 = date[0..4]
        .parse()
        .map_err(|_| format!("malformed date in {s:?}"))?;
    let month: i64 = date[5..7]
        .parse()
        .map_err(|_| format!("malformed date in {s:?}"))?;
    let day: i64 = date[8..10]
        .parse()
        .map_err(|_| format!("malformed date in {s:?}"))?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return Err(format!("date out of range in {s:?}"));
    }

    let (time_part, offset) = parse_utc_designator(time, s)?;

    let mut t = time_part;
    let mut fraction = 0.0f64;
    if let Some((whole, frac)) = t.split_once('.') {
        t = whole;
        if frac.is_empty() || !frac.bytes().all(|b| b.is_ascii_digit()) {
            return Err(format!("malformed fractional seconds in {s:?}"));
        }
        fraction = format!("0.{frac}")
            .parse()
            .map_err(|_| format!("malformed fractional seconds in {s:?}"))?;
    }
    let mut parts: Vec<i64> = Vec::new();
    for chunk in t.split(':') {
        if chunk.len() != 2 || !chunk.bytes().all(|b| b.is_ascii_digit()) {
            return Err(format!("malformed time of day in {s:?}"));
        }
        parts.push(
            chunk
                .parse()
                .map_err(|_| format!("malformed time of day in {s:?}"))?,
        );
    }
    if parts.len() != 3 || parts[0] > 23 || parts[1] > 59 || parts[2] > 60 {
        return Err(format!("time of day out of range in {s:?}"));
    }

    let days = days_from_civil(year, month, day);
    let seconds = days * 86_400 + parts[0] * 3_600 + parts[1] * 60 + parts[2] - offset;
    Ok(seconds as f64 + fraction)
}

/// Split an RFC 3339 time-of-day tail into the wall-clock part and the UTC
/// offset in signed seconds (`Z`, `±HH:MM`, `±HHMM`).
fn parse_utc_designator<'a>(time: &'a str, s: &str) -> Result<(&'a str, i64), String> {
    if let Some((t, _)) = time.rsplit_once(['Z', 'z']) {
        return Ok((t, 0));
    }
    let (t, o) = time
        .rsplit_once(['+', '-'])
        .ok_or_else(|| format!("missing UTC designator in {s:?}"))?;
    let mut off = [0i64; 4];
    let mut k = 0usize;
    for c in o.chars() {
        if c == ':' {
            continue;
        }
        let d = c
            .to_digit(10)
            .ok_or_else(|| format!("malformed offset in {s:?}"))? as i64;
        off[k] = off[k] * 10 + d;
        k += 1;
    }
    if k != 2 && k != 4 {
        return Err(format!("malformed offset in {s:?}"));
    }
    let hours = off[0] * 10 + off[1];
    let minutes = if k == 4 { off[2] * 10 + off[3] } else { 0 };
    if hours > 23 || minutes > 59 {
        return Err(format!("offset out of range in {s:?}"));
    }
    let sign = if time.as_bytes().get(time.len() - o.len() - 1) == Some(&b'+') {
        1
    } else {
        -1
    };
    Ok((t, sign * (hours * 3_600 + minutes * 60)))
}

/// Days since 1970-01-01 for a civil calendar date (Howard Hinnant's
/// `days_from_civil`); handles pre-1970 dates via the era trick.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // [0, 399]
    let mp = if month > 2 { month - 3 } else { month + 9 }; // [0, 11]
    let doy = (153 * mp + 2) / 5 + day - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146_097 + doe - 719_468
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

    const SPEC_SEVEN_EXAMPLE: &str = r#"{
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
    fn spec_seven_example_deserializes_with_every_field_populated() {
        let event: NormalizedEvent = serde_json::from_str(SPEC_SEVEN_EXAMPLE).unwrap();
        assert_eq!(event.event_id, "evt_123");
        assert_eq!(event.session_id, "session_456");
        assert_eq!(event.parent_session_id, None);
        assert_eq!(event.timestamp, 1_788_890_400.0);
        assert_eq!(event.repo.as_deref(), Some("inferweave/autospec"));
        assert_eq!(event.branch.as_deref(), Some("feature/foo"));
        assert_eq!(event.work_item_id.as_deref(), Some("issue-721"));
        assert_eq!(event.agent_role.as_deref(), Some("implementer"));
        assert_eq!(event.provider.as_deref(), Some("local"));
        assert_eq!(event.model.as_deref(), Some("qwen3"));
        assert_eq!(event.event_type, EventType::ToolCall);
        assert_eq!(event.tool.as_deref(), Some("shell"));
        assert_eq!(event.payload, serde_json::json!({}));
        assert_eq!(
            event.tokens,
            Tokens {
                input: 1234,
                output: 212
            }
        );
    }

    #[test]
    fn spec_seven_example_round_trips_to_an_equal_value() {
        let event: NormalizedEvent = serde_json::from_str(SPEC_SEVEN_EXAMPLE).unwrap();
        let serialized = serde_json::to_string(&event).unwrap();
        let reparsed: NormalizedEvent = serde_json::from_str(&serialized).unwrap();
        assert_eq!(event, reparsed);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&serialized).unwrap(),
            serde_json::to_value(&event).unwrap()
        );
    }

    #[test]
    fn numeric_timestamps_still_deserialize() {
        let json = r#"{"event_id": "e", "session_id": "s", "parent_session_id": null,
            "timestamp": 1757316000.5, "repo": null, "branch": null,
            "work_item_id": null, "agent_role": null, "provider": null,
            "model": null, "event_type": "session_started", "tool": null,
            "payload": {}, "tokens": {"input": 0, "output": 0}}"#;
        let event: NormalizedEvent = serde_json::from_str(json).unwrap();
        assert_eq!(event.timestamp, 1757316000.5);
    }

    #[test]
    fn integer_and_negative_integer_timestamps_deserialize() {
        let base = r#"{"event_id": "e", "session_id": "s", "parent_session_id": null,
            "timestamp": T, "repo": null, "branch": null,
            "work_item_id": null, "agent_role": null, "provider": null,
            "model": null, "event_type": "session_started", "tool": null,
            "payload": {}, "tokens": {"input": 0, "output": 0}}"#;
        let positive: NormalizedEvent =
            serde_json::from_str(&base.replace("T", "1757316000")).unwrap();
        assert_eq!(positive.timestamp, 1757316000.0);
        let negative: NormalizedEvent = serde_json::from_str(&base.replace("T", "-1")).unwrap();
        assert_eq!(negative.timestamp, -1.0);
    }

    #[test]
    fn non_numeric_timestamp_reports_what_is_expected() {
        let base = r#"{"event_id": "e", "session_id": "s", "parent_session_id": null,
            "timestamp": T, "repo": null, "branch": null,
            "work_item_id": null, "agent_role": null, "provider": null,
            "model": null, "event_type": "session_started", "tool": null,
            "payload": {}, "tokens": {"input": 0, "output": 0}}"#;
        let err = serde_json::from_str::<NormalizedEvent>(&base.replace("T", "true")).unwrap_err();
        assert!(err.to_string().contains("RFC 3339"), "got: {err}");
    }

    #[test]
    fn unrecognised_event_type_is_an_autospec_error() {
        let err = EventType::parse("vibes_detected").unwrap_err();
        assert!(matches!(err, AutospecError::Parse { .. }));
        let json = SPEC_SEVEN_EXAMPLE.replace("\"tool_call\"", "\"vibes_detected\"");
        assert!(serde_json::from_str::<NormalizedEvent>(&json).is_err());
    }

    #[test]
    fn all_twenty_nine_event_types_parse_from_their_spec_seven_names() {
        const NAMES: [&str; 29] = [
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
        for name in NAMES {
            let parsed = EventType::parse(name).unwrap();
            let reserialized = serde_json::to_value(&parsed).unwrap();
            assert_eq!(
                reserialized,
                serde_json::json!(name),
                "round-trip failed for {name}"
            );
        }
    }

    #[test]
    fn rfc3339_timestamps_normalize_to_unix_seconds() {
        assert_eq!(
            rfc3339_to_unix_seconds("2026-09-08T18:00:00Z").unwrap(),
            1_788_890_400.0
        );
        assert_eq!(
            rfc3339_to_unix_seconds("2026-09-08T18:00:00.25Z").unwrap(),
            1_788_890_400.25
        );
        // 18:00Z == 23:30+05:30
        assert_eq!(
            rfc3339_to_unix_seconds("2026-09-08T23:30:00+05:30").unwrap(),
            1_788_890_400.0
        );
        assert_eq!(
            rfc3339_to_unix_seconds("1969-12-31T23:59:59-00:00").unwrap(),
            -1.0
        );
        // HHMM (colonless) offsets and pre-epoch / year-0 eras.
        assert_eq!(
            rfc3339_to_unix_seconds("2026-09-08T23:30:00+0530").unwrap(),
            1_788_890_400.0
        );
        assert_eq!(
            rfc3339_to_unix_seconds("0000-01-01T00:00:00Z").unwrap(),
            -62_167_219_200.0
        );
        for bad in [
            "18:00:00Z",
            "2026-13-08T18:00:00Z",
            "2026-09-08T18:00:00",
            "not-a-time",
            "abcd-09-08T18:00:00Z",
            "2026-09-08T18:00:00.Z",
            "2026-09-08T18:00:00.5xZ",
            "2026-09-08T18:00Z",
            "2026-09-08T1:00:00Z",
            "2026-09-08T24:00:00Z",
            "2026-09-08T18:00:00+XX:00",
            "2026-09-08T18:00:00+05:3",
            "2026-09-08T18:00:00+24:00",
        ] {
            assert!(
                rfc3339_to_unix_seconds(bad).is_err(),
                "expected error for {bad:?}"
            );
        }
    }

    #[test]
    fn session_ref_carries_source_uri_and_resume_cursor() {
        let json = r#"{"source": "pi_jsonl", "uri": "/var/sessions/a.jsonl",
            "resume_cursor": null}"#;
        let ref_ = serde_json::from_str::<SessionRef>(json).unwrap();
        assert_eq!(ref_.source, "pi_jsonl");
        assert_eq!(ref_.uri, "/var/sessions/a.jsonl");
        assert_eq!(ref_.resume_cursor, None);
        assert_eq!(
            serde_json::to_value(&ref_).unwrap(),
            serde_json::from_str::<serde_json::Value>(json).unwrap()
        );
    }

    struct FixtureAdapter;

    impl SessionAdapter for FixtureAdapter {
        fn discover(&self) -> Result<Vec<SessionRef>, AutospecError> {
            Ok(vec![SessionRef {
                source: "fixture".into(),
                uri: "fixture://one".into(),
                resume_cursor: None,
            }])
        }

        fn read(&self, source: &SessionRef) -> Result<Vec<serde_json::Value>, AutospecError> {
            Ok(vec![
                serde_json::json!({"kind": "tool_call", "uri": source.uri}),
            ])
        }

        fn normalize(
            &self,
            event: &serde_json::Value,
        ) -> Result<Vec<NormalizedEvent>, AutospecError> {
            let event_type = EventType::parse(event["kind"].as_str().unwrap())?;
            Ok(vec![NormalizedEvent {
                event_id: "evt_fixture".into(),
                session_id: "session_fixture".into(),
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
                payload: event.clone(),
                tokens: Tokens::default(),
            }])
        }
    }

    #[test]
    fn session_adapter_contract_round_trips() {
        let adapter = FixtureAdapter;
        let refs = adapter.discover().unwrap();
        assert_eq!(refs.len(), 1);
        let raw = adapter.read(&refs[0]).unwrap();
        let events = adapter.normalize(&raw[0]).unwrap();
        assert_eq!(events[0].event_type, EventType::ToolCall);
        assert_eq!(events[0].payload["uri"], "fixture://one");
    }

    #[test]
    fn extractor_version_is_the_spec_fifty_two_extractor_shape() {
        assert_eq!(EXTRACTOR_VERSION, "1.2.0");
        assert!(EXTRACTOR_VERSION.contains('.'));
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
