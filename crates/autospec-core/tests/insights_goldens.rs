//! Golden session fixtures + snapshot regression harness
//! (`docs/specs/2026-09-08-continuous-improvement-engine.md` §48).
//!
//! `tests/fixtures/insights/` holds 10 synthetic raw-session JSONL fixtures,
//! one per §48 golden scenario. Each fixture pairs with a `.snapshot.json`
//! of its normalized-event stream. The harness fails on any drift between
//! re-normalizing a fixture and its stored snapshot — an analyzer change
//! MUST NOT silently alter historical classification without an intentional
//! fixture update.
//!
//! **Refresh switch.** `INSIGHTS_GOLDEN_UPDATE=1` rewrites drifted (or
//! missing) snapshots in place instead of failing, so a snapshot refresh is
//! one command away from a hand edit:
//!
//! ```sh
//! INSIGHTS_GOLDEN_UPDATE=1 cargo test -p autospec-core insights_goldens
//! ```
//!
//! **Synthetic values only (§39).** Every fixture carries invented
//! identifiers (`synth-*` sessions/models, `acme/synthetic-repo`-style
//! paths); no real session data, no credential-shaped strings.
//!
//! **Interim normalization.** Issue #3826 owns the canonical
//! `NormalizedEvent` / `EXTRACTOR_VERSION` and has not landed on `main`
//! yet, so this harness carries a test-local definition with the same
//! field names and `event_type` wire values as §7 (and the interim
//! definition that landed with #3837). Replacing this module's type with a
//! re-export of #3826's is a drop-in change once it lands; the snapshots
//! are pinned to `EXTRACTOR_VERSION` so any extractor change must pass
//! through the refresh switch on purpose.

use serde::Serialize;
use serde_json::{Map, Value};
use std::fs;
use std::path::{Path, PathBuf};

/// Version of the normalization this harness pins (spec §52:
/// `extractor_version`). Bump + refresh when the normalizer intentionally
/// changes.
const EXTRACTOR_VERSION: &str = "0.1.0";

/// Explicit refresh switch: `INSIGHTS_GOLDEN_UPDATE=1` rewrites drifted or
/// missing snapshots instead of failing the test (spec §48).
const UPDATE_SWITCH: &str = "INSIGHTS_GOLDEN_UPDATE";

const FIXTURE_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/insights");

/// The ten §48 golden scenarios, in fixture order.
const SCENARIOS: &[&str] = &[
    "01-autonomous-success",
    "02-repeated-corrections",
    "03-excessive-exploration",
    "04-context-exhaustion",
    "05-broken-shell-commands",
    "06-over-abstraction",
    "07-unused-skill",
    "08-model-fallback",
    "09-reviewer-rejection",
    "10-ci-failure",
];

/// Core `event_type` wire values (spec §7 list, plus `user_intervention`
/// and `pull_request_merged` from the interim #3837 taxonomy).
const CORE_EVENT_TYPES: &[&str] = &[
    "session_started",
    "session_finished",
    "model_selected",
    "model_fallback",
    "user_message",
    "assistant_message",
    "user_intervention",
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
    "pull_request_merged",
];

/// Per-event token accounting (§7 `tokens`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
struct TokenUsage {
    input: u64,
    output: u64,
}

/// One normalized session event (§7 field names/wire values; interim local
/// definition until #3826 lands — see module docs).
#[derive(Debug, Clone, PartialEq, Serialize)]
struct NormalizedEvent {
    event_id: String,
    session_id: String,
    /// Unix epoch seconds. Raw fixtures carry ISO-8601 UTC capture
    /// timestamps; the normalizer converts on ingest, per §6/§7.
    timestamp: i64,
    event_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(default)]
    tokens: TokenUsage,
    #[serde(default)]
    payload: Value,
}

/// The stored snapshot for one fixture.
#[derive(Debug, PartialEq, Serialize)]
struct Snapshot {
    extractor_version: &'static str,
    fixture: String,
    events: Vec<NormalizedEvent>,
}

/// One golden scenario: a raw JSONL fixture plus its normalized-event
/// snapshot.
#[derive(Debug, Clone)]
pub struct GoldenCase {
    /// Fixture stem, e.g. `01-autonomous-success`.
    pub name: String,
    pub fixture_path: PathBuf,
    pub snapshot_path: PathBuf,
}

/// Walk `tests/fixtures/insights/` and pair every `.jsonl` fixture with its
/// `.snapshot.json`. Sorted by filename so drift reports are stable.
fn golden_cases() -> Vec<GoldenCase> {
    let dir = Path::new(FIXTURE_DIR);
    let mut entries: Vec<String> = fs::read_dir(dir)
        .expect("fixtures/insights directory must exist")
        .map(|entry| {
            entry
                .expect("fixture entry readable")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .filter(|name| name.ends_with(".jsonl"))
        .collect();
    entries.sort();

    entries
        .into_iter()
        .map(|name| {
            let stem = name.trim_end_matches(".jsonl");
            GoldenCase {
                name: stem.to_string(),
                fixture_path: dir.join(format!("{name}")),
                snapshot_path: dir.join(format!("{stem}.snapshot.json")),
            }
        })
        .collect()
}

/// Convert a fixed-shape ISO-8601 UTC timestamp (`YYYY-MM-DDTHH:MM:SSZ`)
/// to Unix epoch seconds. No date library: the fixtures pin the shape.
fn iso8601_utc_to_epoch(raw: &str) -> Result<i64, String> {
    let b = raw.as_bytes();
    if b.len() != 20
        || b[4] != b'-'
        || b[7] != b'-'
        || b[10] != b'T'
        || b[13] != b':'
        || b[16] != b':'
        || b[19] != b'Z'
    {
        return Err(format!(
            "timestamp {raw:?} is not fixed-shape ISO-8601 UTC (YYYY-MM-DDTHH:MM:SSZ)"
        ));
    }
    let num = |lo: usize, hi: usize| -> Result<i64, String> {
        let mut v: i64 = 0;
        for c in &b[lo..hi] {
            if !c.is_ascii_digit() {
                return Err(format!("timestamp {raw:?} has a non-digit field"));
            }
            v = v * 10 + i64::from(c - b'0');
        }
        Ok(v)
    };
    let (year, month, day) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    let (hour, minute, second) = (num(11, 13)?, num(14, 16)?, num(17, 19)?);
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || !(0..=23).contains(&hour)
        || !(0..=59).contains(&minute)
        || !(0..=59).contains(&second)
    {
        return Err(format!("timestamp {raw:?} has an out-of-range field"));
    }
    Ok(days_from_civil(year, month, day) * 86_400 + hour * 3_600 + minute * 60 + second)
}

/// Days between 1970-01-01 and `year-month-day` (Howard Hinnant's
/// `days_from_civil`).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let yoe = year - era * 400; // [0, 399]
    let mp = if month > 2 { month - 3 } else { month + 9 }; // [0, 11]
    let doy = (153 * mp + 2) / 5 + day - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146_097 + doe - 719_468
}

/// Normalize one raw JSONL line into a normalized event.
fn normalize_line(
    seq: usize,
    raw: &Value,
    session_id: &str,
    header_model: Option<&str>,
) -> Result<NormalizedEvent, String> {
    let obj = raw
        .as_object()
        .ok_or_else(|| "raw event must be a JSON object".to_string())?;

    let event_type = obj
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| "raw event is missing `type`".to_string())?;
    if !CORE_EVENT_TYPES.contains(&event_type) {
        return Err(format!(
            "`type` {event_type:?} is not one of the §7 core event types"
        ));
    }

    let timestamp_raw = obj
        .get("ts")
        .and_then(Value::as_str)
        .ok_or_else(|| "raw event is missing `ts`".to_string())?;
    let timestamp = iso8601_utc_to_epoch(timestamp_raw)?;

    if let Some(line_session) = obj.get("session_id").and_then(Value::as_str) {
        if line_session != session_id {
            return Err(format!(
                "event {seq} has session_id {line_session:?}, header has {session_id:?}"
            ));
        }
    }

    let model = obj
        .get("model")
        .and_then(Value::as_str)
        .or(header_model)
        .map(str::to_string);

    let tokens = match obj.get("tokens") {
        None => TokenUsage::default(),
        Some(Value::Object(m)) => TokenUsage {
            input: m.get("input").and_then(Value::as_u64).unwrap_or(0),
            output: m.get("output").and_then(Value::as_u64).unwrap_or(0),
        },
        Some(_) => {
            return Err("tokens must be an object {input, output}".to_string());
        }
    };

    // Everything that is not structural metadata becomes the event payload,
    // in sorted-key order (serde_json's default map is a BTreeMap) so the
    // snapshot is byte-stable.
    let mut payload = Map::new();
    for (key, value) in obj {
        if !["ts", "type", "session_id", "model", "tokens"].contains(&key.as_str()) {
            payload.insert(key.clone(), value.clone());
        }
    }

    Ok(NormalizedEvent {
        event_id: format!("evt-{session_id}-{seq:04}"),
        session_id: session_id.to_string(),
        timestamp,
        event_type: event_type.to_string(),
        model,
        tokens,
        payload: Value::Object(payload),
    })
}

/// Normalize a full raw JSONL fixture into its normalized-event stream.
fn normalize_fixture(raw: &str) -> Result<Vec<NormalizedEvent>, String> {
    let mut lines: Vec<Value> = Vec::new();
    for (idx, line) in raw.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let parsed: Value = serde_json::from_str(line)
            .map_err(|err| format!("line {}: invalid JSON: {err}", idx + 1))?;
        lines.push(parsed);
    }
    if lines.is_empty() {
        return Err("fixture has no events".to_string());
    }

    let header = lines.first().expect("checked non-empty");
    let header_type = header
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| "first raw event is missing `type`".to_string())?;
    if header_type != "session_started" {
        return Err(format!(
            "first raw event must be session_started, got {header_type:?}"
        ));
    }
    let session_id = header
        .get("session_id")
        .and_then(Value::as_str)
        .ok_or_else(|| "session_started is missing `session_id`".to_string())?;
    let header_model = header.get("model").and_then(|v| v.as_str());

    lines
        .iter()
        .enumerate()
        .map(|(idx, line)| normalize_line(idx + 1, line, session_id, header_model))
        .collect()
}

/// Render the canonical snapshot bytes for a fixture.
fn render_snapshot(fixture: &str, events: Vec<NormalizedEvent>) -> String {
    let snapshot = Snapshot {
        extractor_version: EXTRACTOR_VERSION,
        fixture: fixture.to_string(),
        events,
    };
    format!(
        "{}\n",
        serde_json::to_string_pretty(&snapshot).expect("snapshot serializes")
    )
}

/// Check one golden case. On drift (or a missing snapshot) the case fails
/// unless `update` is true, in which case the snapshot is rewritten in
/// place — the §48 "intentional fixture update" path.
fn check_case(case: &GoldenCase, update: bool) -> Result<(), String> {
    let raw = fs::read_to_string(&case.fixture_path)
        .map_err(|err| format!("cannot read {}: {err}", case.fixture_path.display()))?;
    let events =
        normalize_fixture(&raw).map_err(|err| format!("{}: {err}", case.fixture_path.display()))?;
    let expected = render_snapshot(&case.name, events);

    match fs::read_to_string(&case.snapshot_path) {
        Ok(actual) if actual == expected => Ok(()),
        Ok(actual) => {
            if update {
                fs::write(&case.snapshot_path, expected).map_err(|err| {
                    format!("cannot refresh {}: {err}", case.snapshot_path.display())
                })?;
                Ok(())
            } else {
                Err(format!(
                    "snapshot drift for {}: stored snapshot differs from re-normalized events.\n\
                     Stored:\n{}\nRe-normalized:\n{}\n\
                     Refresh intentionally with INSIGHTS_GOLDEN_UPDATE=1.",
                    case.name, actual, expected
                ))
            }
        }
        Err(_) => {
            if update {
                fs::write(&case.snapshot_path, expected).map_err(|err| {
                    format!("cannot write {}: {err}", case.snapshot_path.display())
                })?;
                Ok(())
            } else {
                Err(format!(
                    "missing snapshot {} (create it with INSIGHTS_GOLDEN_UPDATE=1)",
                    case.snapshot_path.display()
                ))
            }
        }
    }
}

/// The test-facing wrapper: drift fails the test unless the operator set
/// the explicit refresh switch.
fn check_case_from_env(case: &GoldenCase) {
    let update = std::env::var_os(UPDATE_SWITCH).as_deref() == Some(std::ffi::OsStr::new("1"));
    check_case(case, update).unwrap_or_else(|err| panic!("{err}"));
}

macro_rules! golden_test {
    ($name:ident, $stem:expr) => {
        #[test]
        fn $name() {
            let case = golden_cases()
                .into_iter()
                .find(|c| c.name == $stem)
                .unwrap_or_else(|| panic!("golden fixture {} not found", $stem));
            check_case_from_env(&case);
        }
    };
}

golden_test!(golden_01_autonomous_success, "01-autonomous-success");
golden_test!(golden_02_repeated_corrections, "02-repeated-corrections");
golden_test!(golden_03_excessive_exploration, "03-excessive-exploration");
golden_test!(golden_04_context_exhaustion, "04-context-exhaustion");
golden_test!(golden_05_broken_shell_commands, "05-broken-shell-commands");
golden_test!(golden_06_over_abstraction, "06-over-abstraction");
golden_test!(golden_07_unused_skill, "07-unused-skill");
golden_test!(golden_08_model_fallback, "08-model-fallback");
golden_test!(golden_09_reviewer_rejection, "09-reviewer-rejection");
golden_test!(golden_10_ci_failure, "10-ci-failure");

#[test]
fn golden_cases_walks_exactly_the_ten_scenarios() {
    let cases = golden_cases();
    let names: Vec<&str> = cases.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(
        names, SCENARIOS,
        "fixtures/insights must hold exactly the 10 §48 scenario fixtures, in order"
    );
    for case in &cases {
        assert!(
            case.fixture_path.is_file(),
            "missing fixture {}",
            case.fixture_path.display()
        );
    }
}

// ── normalizer unit tests ────────────────────────────────────────────────────

#[test]
fn normalize_derives_event_ids_and_inherits_session_and_model() {
    let raw = "\
{\"ts\":\"2026-09-08T18:00:00Z\",\"type\":\"session_started\",\"session_id\":\"synth-u\",\"model\":\"synth-32b\"}
{\"ts\":\"2026-09-08T18:00:05Z\",\"type\":\"user_message\",\"text\":\"do the thing\"}
{\"ts\":\"2026-09-08T18:00:09Z\",\"type\":\"tool_call\",\"tool\":\"shell\",\"input\":\"cargo test\",\"tokens\":{\"input\":120,\"output\":12}}
";
    let events = normalize_fixture(raw).expect("fixture normalizes");
    assert_eq!(events.len(), 3);
    assert_eq!(events[0].event_id, "evt-synth-u-0001");
    assert_eq!(events[0].event_type, "session_started");
    assert_eq!(events[0].timestamp, 1788890400); // 2026-09-08T18:00:00Z
    assert_eq!(events[0].model.as_deref(), Some("synth-32b"));

    assert_eq!(events[1].event_id, "evt-synth-u-0002");
    assert_eq!(
        events[1].model.as_deref(),
        Some("synth-32b"),
        "model inherits from the header"
    );
    assert_eq!(events[1].tokens, TokenUsage::default());
    assert_eq!(
        events[1].payload,
        serde_json::json!({"text": "do the thing"})
    );

    assert_eq!(events[2].event_type, "tool_call");
    assert_eq!(
        events[2].tokens,
        TokenUsage {
            input: 120,
            output: 12
        }
    );
    let payload = events[2].payload.as_object().expect("payload object");
    assert_eq!(payload.get("tool").and_then(Value::as_str), Some("shell"));
    assert_eq!(
        payload.get("input").and_then(Value::as_str),
        Some("cargo test")
    );
    assert!(
        !payload.contains_key("tokens"),
        "structural keys never leak into the payload"
    );
}

#[test]
fn normalize_overrides_model_on_the_line_that_carries_one() {
    let raw = "\
{\"ts\":\"2026-09-08T18:00:00Z\",\"type\":\"session_started\",\"session_id\":\"synth-u\",\"model\":\"synth-32b\"}
{\"ts\":\"2026-09-08T18:02:00Z\",\"type\":\"model_fallback\",\"model\":\"synth-70b\",\"reason\":\"quota_exhausted\"}
";
    let events = normalize_fixture(raw).expect("fixture normalizes");
    assert_eq!(events[1].model.as_deref(), Some("synth-70b"));
    assert_eq!(
        events[1].payload,
        serde_json::json!({"reason": "quota_exhausted"})
    );
}

#[test]
fn normalize_rejects_unknown_event_types_and_bad_timestamps() {
    let unknown_type = "\
{\"ts\":\"2026-09-08T18:00:00Z\",\"type\":\"session_started\",\"session_id\":\"synth-u\"}
{\"ts\":\"2026-09-08T18:00:05Z\",\"type\":\"teleport\"}
";
    let err = normalize_fixture(unknown_type).expect_err("unknown type rejects");
    assert!(err.contains("teleport"), "error names the bad type: {err}");

    let bad_ts = "\
{\"ts\":\"2026-09-08T18:00:00Z\",\"type\":\"session_started\",\"session_id\":\"synth-u\"}
{\"ts\":\"yesterday\",\"type\":\"user_message\",\"text\":\"hi\"}
";
    let err = normalize_fixture(bad_ts).expect_err("bad timestamp rejects");
    assert!(
        err.contains("ISO-8601"),
        "error names the timestamp problem: {err}"
    );
}

#[test]
fn normalize_requires_a_session_started_header() {
    let no_header = "{\"ts\":\"2026-09-08T18:00:00Z\",\"type\":\"user_message\",\"session_id\":\"synth-u\",\"text\":\"hi\"}\n";
    let err = normalize_fixture(no_header).expect_err("missing header rejects");
    assert!(
        err.contains("session_started"),
        "error names the header requirement: {err}"
    );

    let empty = "\n  \n";
    assert!(normalize_fixture(empty).is_err(), "empty fixture rejects");
}

#[test]
fn normalize_rejects_session_mismatch_and_malformed_shapes() {
    let mismatch = "\
{\"ts\":\"2026-09-08T18:00:00Z\",\"type\":\"session_started\",\"session_id\":\"synth-u\"}
{\"ts\":\"2026-09-08T18:00:05Z\",\"type\":\"user_message\",\"session_id\":\"synth-other\",\"text\":\"hi\"}
";
    let err = normalize_fixture(mismatch).expect_err("session mismatch rejects");
    assert!(
        err.contains("synth-other"),
        "error names the mismatched session: {err}"
    );

    let bad_tokens = "\
{\"ts\":\"2026-09-08T18:00:00Z\",\"type\":\"session_started\",\"session_id\":\"synth-u\"}
{\"ts\":\"2026-09-08T18:00:05Z\",\"type\":\"user_message\",\"tokens\":\"1234\",\"text\":\"hi\"}
";
    let err = normalize_fixture(bad_tokens).expect_err("non-object tokens rejects");
    assert!(
        err.contains("tokens must be an object"),
        "error names the tokens problem: {err}"
    );

    let non_object = "\
{\"ts\":\"2026-09-08T18:00:00Z\",\"type\":\"session_started\",\"session_id\":\"synth-u\"}
[1, 2, 3]
";
    let err = normalize_fixture(non_object).expect_err("non-object event rejects");
    assert!(
        err.contains("JSON object"),
        "error names the object requirement: {err}"
    );
}

#[test]
fn missing_snapshot_fails_without_the_update_switch() {
    let dir = scratch_dir("missing-noswitch");
    let case = GoldenCase {
        name: "missing-noswitch".to_string(),
        fixture_path: dir.join("missing-noswitch.jsonl"),
        snapshot_path: dir.join("missing-noswitch.snapshot.json"),
    };
    let raw = "{\"ts\":\"2026-09-08T18:00:00Z\",\"type\":\"session_started\",\"session_id\":\"synth-m\",\"model\":\"synth-32b\"}\n";
    fs::write(&case.fixture_path, raw).expect("fixture seed");
    let err = check_case(&case, false).expect_err("missing snapshot must fail");
    assert!(
        err.contains("missing snapshot"),
        "failure names the missing snapshot: {err}"
    );
    assert!(
        err.contains(UPDATE_SWITCH),
        "failure points at the refresh switch: {err}"
    );
    assert!(
        !case.snapshot_path.exists(),
        "no snapshot is written without the switch"
    );
}

#[test]
fn epoch_conversion_pins_known_instants() {
    assert_eq!(iso8601_utc_to_epoch("1970-01-01T00:00:00Z").unwrap(), 0);
    assert_eq!(iso8601_utc_to_epoch("1970-01-01T00:00:01Z").unwrap(), 1);
    assert_eq!(
        iso8601_utc_to_epoch("2026-09-08T18:00:00Z").unwrap(),
        1788890400
    );
    assert!(
        iso8601_utc_to_epoch("2026-13-08T18:00:00Z").is_err(),
        "month 13 rejects"
    );
    assert!(
        iso8601_utc_to_epoch("2026-09-08T18:00:00").is_err(),
        "missing Z rejects"
    );
}

// ── drift + refresh-switch tests (isolated temp dirs) ────────────────────────

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("insights-goldens-{tag}-{}", std::process::id()));
    fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn seed_case(tag: &str, mutate_snapshot: bool) -> GoldenCase {
    let dir = scratch_dir(tag);
    let fixture = format!("{tag}.jsonl");
    let snapshot = format!("{tag}.snapshot.json");
    let lines = [
        serde_json::json!({
            "ts": "2026-09-08T18:00:00Z",
            "type": "session_started",
            "session_id": format!("synth-{tag}"),
            "model": "synth-32b",
        }),
        serde_json::json!({"ts": "2026-09-08T18:00:05Z", "type": "user_message", "text": "do the thing"}),
        serde_json::json!({"ts": "2026-09-08T18:00:10Z", "type": "session_finished", "outcome": "success"}),
    ];
    let raw = lines
        .iter()
        .map(serde_json::Value::to_string)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    let case = GoldenCase {
        name: tag.to_string(),
        fixture_path: dir.join(&fixture),
        snapshot_path: dir.join(&snapshot),
    };
    fs::write(&case.fixture_path, &raw).expect("fixture seed");
    let events = normalize_fixture(&raw).expect("seed normalizes");
    let rendered = render_snapshot(tag, events);
    if mutate_snapshot {
        fs::write(
            &case.snapshot_path,
            rendered.replace("synth-32b", "some-other-model"),
        )
        .expect("mutated snapshot seed");
    } else {
        fs::write(&case.snapshot_path, rendered).expect("snapshot seed");
    }
    case
}

#[test]
fn untouched_snapshot_passes_and_mutated_snapshot_fails() {
    let ok = seed_case("drift-ok", false);
    check_case(&ok, false).expect("matching snapshot passes");

    let mutated = seed_case("drift-bad", true);
    let err = check_case(&mutated, false).expect_err("mutated snapshot must fail");
    assert!(
        err.contains("snapshot drift"),
        "failure names the drift: {err}"
    );
    assert!(
        err.contains(UPDATE_SWITCH),
        "failure points at the refresh switch: {err}"
    );
}

#[test]
fn update_switch_rewrites_drifted_and_missing_snapshots() {
    let drifted = seed_case("update-drift", true);
    check_case(&drifted, true).expect("update switch refreshes drift");
    let refreshed = fs::read_to_string(&drifted.snapshot_path).expect("refreshed");
    let raw = fs::read_to_string(&drifted.fixture_path).expect("fixture");
    let events = normalize_fixture(&raw).expect("normalizes");
    assert_eq!(refreshed, render_snapshot(&drifted.name, events));

    let missing = seed_case("update-missing", false);
    fs::remove_file(&missing.snapshot_path).expect("remove snapshot");
    check_case(&missing, true).expect("update switch writes a missing snapshot");
    assert!(
        missing.snapshot_path.is_file(),
        "refreshed snapshot exists on disk"
    );
}
