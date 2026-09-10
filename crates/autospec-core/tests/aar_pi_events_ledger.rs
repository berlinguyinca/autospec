//! Issue #3319: Pi execution events normalize into routing-ledger rows.
//!
//! The fixture at `tests/fixtures/pi-events/session.jsonl` is a replayed Pi
//! session. The tests below replay it and assert the three properties the
//! issue asks for: mandatory identity, `unknown` instead of a fabricated
//! zero, and the performance metrics recorded verbatim.

use autospec_core::aar::{
    audit_ledger, normalize_event, normalize_jsonl, to_ledger_lines, EventKind, Metric,
    PiEventRecord, SessionIdentity, DISPATCH_RECORD_TYPE, EVENT_ALIASES, EVENT_RECORD_TYPE,
    EVENT_SCHEMA_VERSION, PI_HARNESS, UNKNOWN,
};
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;

const FIXTURE: &str = include_str!("fixtures/pi-events/session.jsonl");

fn identity() -> SessionIdentity {
    SessionIdentity::new("pi-7f3a", "3319", "implementer")
}

fn fixture_records() -> Vec<PiEventRecord> {
    normalize_jsonl(FIXTURE, &identity()).expect("fixture normalizes")
}

fn kinds(records: &[PiEventRecord]) -> Vec<&'static str> {
    records.iter().map(|r| r.event.as_str()).collect()
}

/// AC: every event carries timestamp, session_id, work_item_id, agent_role.
#[test]
fn every_replayed_event_carries_the_mandatory_identity() {
    let records = fixture_records();
    assert_eq!(records.len(), 10);
    for record in &records {
        assert!(
            !record.timestamp.trim().is_empty(),
            "seq {} has no timestamp",
            record.seq
        );
        assert_eq!(record.session_id, "pi-7f3a");
        assert_eq!(record.work_item_id, "3319");
        assert_eq!(record.agent_role, "implementer");
        assert_eq!(record.harness, PI_HARNESS);
        assert_eq!(record.record_type, EVENT_RECORD_TYPE);
        assert_eq!(record.schema_version, EVENT_SCHEMA_VERSION);
        assert_eq!(record.missing_identity_field(), None);
    }
}

/// Only the session start repeats the identity; the rest inherit it.
#[test]
fn events_inherit_the_session_identity_they_do_not_repeat() {
    let raw = json!({"type": "test_run", "timestamp": "2026-08-21T09:18:02.770Z"});
    let record = normalize_event(&raw, &identity(), 3).expect("inherits identity");
    assert_eq!(record.session_id, "pi-7f3a");
    assert_eq!(record.work_item_id, "3319");
    assert_eq!(record.agent_role, "implementer");

    let own = json!({
        "event": "model_request",
        "timestamp": "2026-08-21T09:14:05.884Z",
        "session_id": "pi-9999",
        "work_item_id": "4242",
        "role": "reviewer",
    });
    let record = normalize_event(&own, &identity(), 1).expect("event wins over session");
    assert_eq!(record.session_id, "pi-9999");
    assert_eq!(record.work_item_id, "4242");
    assert_eq!(record.agent_role, "reviewer");
}

/// AC: a missing metric is `unknown`, never a fabricated zero.
#[test]
fn an_unmeasured_metric_serializes_as_unknown_not_zero() {
    let record = PiEventRecord::new(
        EventKind::Finish,
        1,
        "2026-08-21T09:25:07.225Z",
        &identity(),
    );
    let line = record.to_json_line().expect("serializes");

    for key in [
        "input_tokens",
        "output_tokens",
        "reasoning_tokens",
        "cache_hit_tokens",
        "cache_miss_tokens",
        "ttft_ms",
        "decode_tok_s",
        "queue_ms",
        "wall_ms",
        "context_used_tokens",
        "tool",
        "tests_total",
        "tests_failed",
        "repair_count",
        "success",
    ] {
        assert!(
            line.contains(&format!("\"{}\":\"{}\"", key, UNKNOWN)),
            "{} must serialize as unknown, got: {}",
            key,
            line
        );
        assert!(
            !line.contains(&format!("\"{}\":0", key)),
            "{} must never serialize as 0 when unmeasured",
            key
        );
    }
}

/// A reported zero stays the number zero: measured-zero and unmeasured are
/// different facts.
#[test]
fn a_reported_zero_survives_as_a_number() {
    let raw = json!({
        "type": "test_run",
        "timestamp": "2026-08-21T09:18:02.770Z",
        "tests_total": 18,
        "tests_failed": 0,
        "tool_ms": "417",
    });
    let record = normalize_event(&raw, &identity(), 1).expect("normalizes");
    assert_eq!(record.tests_failed, Metric::measured(0));
    assert_eq!(record.tool_ms, Metric::measured(417));

    let line = record.to_json_line().expect("serializes");
    assert!(line.contains("\"tests_failed\":0"), "got: {}", line);
    assert!(line.contains("\"tool_ms\":417"), "got: {}", line);
    assert!(
        line.contains(&format!("\"queue_ms\":\"{}\"", UNKNOWN)),
        "got: {}",
        line
    );
}

/// AC: the fixture records ttft_ms, decode_tok_s and cache_hit_tokens.
#[test]
fn the_fixture_records_the_performance_metrics() {
    let records = fixture_records();
    let request = records
        .iter()
        .find(|r| r.event == EventKind::ModelRequest)
        .expect("fixture contains a model_request");

    assert_eq!(request.ttft_ms, Metric::measured(412));
    assert_eq!(request.decode_tok_s, Metric::measured(61.5));
    assert_eq!(request.cache_hit_tokens, Metric::measured(18_944));
    assert_eq!(request.cache_miss_tokens, Metric::measured(1_536));
    assert_eq!(request.input_tokens, Metric::measured(20_480));
    assert_eq!(request.prefill_ms, Metric::measured(2_360));
    assert_eq!(request.turn_ms, Metric::measured(8_734));
    assert_eq!(request.model, Metric::measured("qwen3.8-27b".to_string()));

    let line = request.to_json_line().expect("serializes");
    assert!(line.contains("\"ttft_ms\":412"), "got: {}", line);
    assert!(line.contains("\"decode_tok_s\":61.5"), "got: {}", line);
    assert!(line.contains("\"cache_hit_tokens\":18944"), "got: {}", line);
}

/// The whole fixture vocabulary maps onto the canonical kinds, in order.
#[test]
fn the_fixture_covers_the_lifecycle_vocabulary() {
    assert_eq!(
        kinds(&fixture_records()),
        vec![
            "session_start",
            "model_request",
            "tool_call",
            "tool_call",
            "file_edit",
            "test_run",
            "model_request",
            "compaction",
            "failure",
            "finish",
        ]
    );
}

#[test]
fn raw_pi_event_names_map_onto_the_canonical_vocabulary() {
    for (wire, kind) in EVENT_ALIASES {
        assert_eq!(EventKind::from_wire(wire), Some(*kind));
    }
    assert_eq!(EventKind::from_wire("model_turn"), None);
}

/// A row whose identity cannot be completed is rejected, not written blank.
#[test]
fn an_event_without_identity_is_rejected() {
    let blank = SessionIdentity::new("", "3319", "implementer");
    let raw = json!({"type": "finish", "timestamp": "2026-08-21T09:25:07.225Z"});
    let error = normalize_event(&raw, &blank, 1).expect_err("blank session id rejected");
    assert!(error.contains("session_id"), "got: {}", error);

    let no_ts = json!({"type": "finish", "session_id": "pi-7f3a"});
    let error = normalize_event(&no_ts, &identity(), 1).expect_err("missing ts rejected");
    assert!(error.contains("timestamp"), "got: {}", error);
}

#[test]
fn an_unrecognized_event_name_is_rejected_rather_than_dropped() {
    let raw = json!({"type": "mic_drop", "timestamp": "2026-08-21T09:25:07.225Z"});
    let error = normalize_event(&raw, &identity(), 1).expect_err("unknown name rejected");
    assert_eq!(error, "unknown pi event name: mic_drop");
}

#[test]
fn a_bare_metric_contradiction_is_rejected() {
    let raw = json!({
        "type": "model_request",
        "timestamp": "2026-08-21T09:14:05.884Z",
        "input_tokens": 100,
        "cache_hit_tokens": 90,
        "cache_miss_tokens": 90,
    });
    let error = normalize_event(&raw, &identity(), 1).expect_err("cache > input rejected");
    assert!(error.contains("cache tokens"), "got: {}", error);

    let raw = json!({
        "type": "test_run",
        "timestamp": "2026-08-21T09:18:02.770Z",
        "tests_total": 3,
        "tests_failed": 4,
    });
    assert!(normalize_event(&raw, &identity(), 1).is_err());
}

/// A replay names the offending line rather than half-normalizing the session.
#[test]
fn a_replay_reports_the_line_that_failed() {
    let broken = format!("{}\n{}", FIXTURE.trim_end(), "{\"type\": \"oops\"}");
    let error = normalize_jsonl(&broken, &identity()).expect_err("line 11 rejected");
    assert!(error.starts_with("line 11:"), "got: {}", error);

    assert!(
        normalize_jsonl("\n\n", &identity()).is_err(),
        "an empty transcript is an error, not an empty ledger"
    );
}

/// Rows round-trip through the ledger wire format, unknowns included.
#[test]
fn rows_round_trip_through_the_ledger_wire_format() {
    for record in fixture_records() {
        let line = record.to_json_line().expect("serializes");
        let parsed = PiEventRecord::from_json_line(&line).expect("parses back");
        assert_eq!(parsed, record);
        assert_eq!(parsed.decode_tok_s, record.decode_tok_s);
    }

    let sparse = PiEventRecord::new(EventKind::Compaction, 1, "t", &identity())
        .to_json_line()
        .unwrap();
    let parsed = PiEventRecord::from_json_line(&sparse).expect("unknowns parse back");
    assert_eq!(parsed.ttft_ms, Metric::<u64>::unknown());
    assert!(parsed.decode_tok_s.is_unknown());
    assert!(parsed.failure_category.is_unknown());
}

/// AC: replaying the fixture yields rows that validate against the ledger,
/// side by side with the dispatch rows already in it.
#[test]
fn replayed_rows_validate_alongside_dispatch_rows() {
    let dispatch = json!({
        "dispatch_id": "d-2261",
        "ts": "2026-08-21T09:13:58Z",
        "profile": "tier-b",
        "model": "qwen3.8-27b",
        "harness": "pi",
        "issue": "3319",
        "outcome": "success",
    })
    .to_string();
    let ledger = format!(
        "{}\n{}",
        dispatch,
        to_ledger_lines(&fixture_records()).unwrap()
    );

    let audit = audit_ledger(&ledger);
    assert!(audit.ok(), "findings: {:?}", audit.findings);
    assert_eq!(audit.dispatches, 1);
    assert_eq!(audit.events, 10);

    // Corrupt the identity of the first event row (ledger line 2) and the
    // audit must name that line.
    let mut corrupted: Vec<String> = ledger.lines().map(str::to_string).collect();
    corrupted[1] = corrupted[1].replace("\"work_item_id\":\"3319\"", "\"work_item_id\":\"\"");
    let audit = audit_ledger(&corrupted.join("\n"));
    assert!(!audit.ok(), "a blank identity must be a finding");
    assert!(
        audit.findings[0].starts_with("2:"),
        "got: {:?}",
        audit.findings
    );
    assert_eq!(audit.events, 9);
}

#[test]
fn a_row_that_is_neither_an_event_nor_a_dispatch_is_a_finding() {
    let audit = audit_ledger("{\"note\":\"hello\"}\n\n{\"record_type\":\"event\",\n");
    assert_eq!(audit.events, 0);
    assert_eq!(audit.dispatches, 0);
    assert_eq!(audit.findings.len(), 2);
    assert!(audit.findings[0].contains("neither an event nor a dispatch"));
    assert!(audit.findings[1].contains("malformed json"));
    assert!(!audit.ok());
}

/// The ledger is append-only: appending grows the file and leaves earlier
/// bytes untouched.
#[test]
fn appending_to_the_ledger_never_rewrites_an_earlier_row() {
    let dir = std::env::temp_dir().join(format!("autospec-3319-ledger-{}", std::process::id()));
    fs::create_dir_all(&dir).expect("tempdir");
    let path = PathBuf::from(&dir).join("ledger.jsonl");

    let first = to_ledger_lines(&fixture_records()[..3]).unwrap();
    fs::write(&path, &first).expect("seed ledger");

    let appended = to_ledger_lines(&fixture_records()[3..]).unwrap();
    let mut handle = fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .expect("open for append");
    std::io::Write::write_all(&mut handle, appended.as_bytes()).expect("append rows");
    drop(handle);

    let text = fs::read_to_string(&path).expect("read ledger");
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 10, "one row per event");
    assert!(
        text.starts_with(&first),
        "the first rows are byte-identical"
    );

    let audit = audit_ledger(&text);
    assert!(audit.ok(), "findings: {:?}", audit.findings);
    assert_eq!(audit.events, 10);

    // Replaying the same session appends again rather than deduplicating: the
    // ledger records what happened, per run.
    fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .and_then(|mut f| std::io::Write::write_all(&mut f, first.as_bytes()))
        .expect("re-append");
    let text = fs::read_to_string(&path).expect("read ledger");
    assert_eq!(audit_ledger(&text).events, 13);

    fs::remove_dir_all(&dir).ok();
}

/// The dispatch vocabulary in the ledger is untouched by the event contract.
#[test]
fn dispatch_rows_are_read_as_dispatches() {
    let value: Value = json!({"dispatch_id": "d-1", "outcome": "success"});
    let audit = audit_ledger(&value.to_string());
    assert_eq!(audit.dispatches, 1);
    assert!(audit.ok());

    let record = PiEventRecord::new(EventKind::Finish, 1, "t", &identity());
    assert_ne!(record.record_type, DISPATCH_RECORD_TYPE);
}
