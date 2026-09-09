//! Golden session fixtures and snapshot regression harness (spec §48 Testing
//! Strategy, spec §52 Versioning).
//!
//! Each `tests/fixtures/insights/<name>.jsonl` fixture is a synthetic raw
//! session covering one §48 scenario. Per spec §39 every value in the fixtures
//! is synthetic (fake repos, sessions, models, and token counts) — no live
//! session capture.
//!
//! The harness normalizes a fixture with the normalized event model introduced
//! by issue #3826 and compares the rendered snapshot byte for byte with the
//! sibling `<name>.snapshot.json`. Drift fails the suite so an analyzer change
//! cannot silently alter historical classification; set
//! `INSIGHTS_GOLDEN_UPDATE=1` to refresh every snapshot intentionally.
//!
//! Expected findings are out of scope here; they arrive with the pattern
//! engine.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Schema version of the normalized event model (issue #3826 / spec §52).
const EXTRACTOR_VERSION: &str = "1.0.0";

const DEFAULT_TIMESTAMP: &str = "1970-01-01T00:00:00Z";
const DEFAULT_REPO: &str = "synthetic/repo";
const DEFAULT_BRANCH: &str = "main";
const DEFAULT_AGENT_ROLE: &str = "orchestrator";
const DEFAULT_PROVIDER: &str = "local";
const UPDATE_SWITCH: &str = "INSIGHTS_GOLDEN_UPDATE";

/// Token accounting for a single normalized event (spec §7).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
struct Tokens {
    input: u64,
    output: u64,
}

/// Normalized session event (spec §7 Normalized Session Event Model).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct NormalizedEvent {
    event_id: String,
    session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    parent_session_id: Option<String>,
    timestamp: String,
    repo: String,
    branch: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    work_item_id: Option<String>,
    agent_role: String,
    provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    event_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tool: Option<String>,
    payload: Map<String, Value>,
    tokens: Tokens,
}

/// Snapshot of a normalized fixture, versioned for reproducibility (spec §52).
#[derive(Debug, Serialize, Deserialize)]
struct Snapshot {
    extractor_version: String,
    session_id: String,
    events: Vec<NormalizedEvent>,
}

/// One raw JSONL line of a synthetic session fixture.
#[derive(Debug, Deserialize)]
struct RawRecord {
    #[serde(rename = "type")]
    event_type: String,
    timestamp: Option<String>,
    session_id: Option<String>,
    parent_session_id: Option<String>,
    repo: Option<String>,
    branch: Option<String>,
    work_item_id: Option<String>,
    agent_role: Option<String>,
    provider: Option<String>,
    model: Option<String>,
    tool: Option<String>,
    text: Option<String>,
    payload: Option<Map<String, Value>>,
    tokens: Option<Tokens>,
}

/// One fixture plus the snapshot it must match.
struct GoldenCase {
    name: String,
    fixture: PathBuf,
    snapshot: PathBuf,
}

fn default_fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/insights")
}

/// Walk the fixtures directory and return one case per `.jsonl` fixture,
/// sorted by file name. Each fixture pairs with `<name>.snapshot.json`.
fn golden_cases() -> Vec<GoldenCase> {
    golden_cases_in(&default_fixtures_dir())
}

fn golden_cases_in(dir: &Path) -> Vec<GoldenCase> {
    let mut entries: Vec<PathBuf> = match fs::read_dir(dir) {
        Ok(entries) => entries
            .filter_map(|entry| entry.ok().map(|e| e.path()))
            .filter(|path| path.extension().is_some_and(|ext| ext == "jsonl"))
            .collect(),
        Err(err) => panic!("cannot read fixtures directory {dir:?}: {err}"),
    };
    entries.sort();
    entries
        .into_iter()
        .map(|fixture| {
            let snapshot = fixture.with_extension("snapshot.json");
            let name = fixture
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            GoldenCase {
                name,
                fixture,
                snapshot,
            }
        })
        .collect()
}

fn normalize_record(index: usize, rec: &RawRecord, session_id: &str) -> NormalizedEvent {
    let mut payload = rec.payload.clone().unwrap_or_default();
    if let Some(text) = &rec.text {
        payload
            .entry("text")
            .or_insert_with(|| Value::String(text.clone()));
    }
    NormalizedEvent {
        event_id: format!("evt_{:04}", index + 1),
        session_id: session_id.to_string(),
        parent_session_id: rec.parent_session_id.clone(),
        timestamp: rec
            .timestamp
            .clone()
            .unwrap_or_else(|| DEFAULT_TIMESTAMP.to_string()),
        repo: rec.repo.clone().unwrap_or_else(|| DEFAULT_REPO.to_string()),
        branch: rec
            .branch
            .clone()
            .unwrap_or_else(|| DEFAULT_BRANCH.to_string()),
        work_item_id: rec.work_item_id.clone(),
        agent_role: rec
            .agent_role
            .clone()
            .unwrap_or_else(|| DEFAULT_AGENT_ROLE.to_string()),
        provider: rec
            .provider
            .clone()
            .unwrap_or_else(|| DEFAULT_PROVIDER.to_string()),
        model: rec.model.clone(),
        event_type: rec.event_type.clone(),
        tool: rec.tool.clone(),
        payload,
        tokens: rec.tokens.clone().unwrap_or_default(),
    }
}

/// Normalize a raw JSONL fixture into a versioned snapshot of events.
fn normalize_fixture(raw: &str) -> Result<Snapshot, String> {
    let mut records: Vec<RawRecord> = Vec::new();
    for (i, line) in raw.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let rec: RawRecord =
            serde_json::from_str(line).map_err(|err| format!("line {}: {err}", i + 1))?;
        records.push(rec);
    }
    let session_id = records
        .iter()
        .find_map(|rec| rec.session_id.clone())
        .unwrap_or_else(|| "unknown-session".to_string());
    let events = records
        .iter()
        .enumerate()
        .map(|(i, rec)| normalize_record(i, rec, &session_id))
        .collect();
    Ok(Snapshot {
        extractor_version: EXTRACTOR_VERSION.to_string(),
        session_id,
        events,
    })
}

fn render(snap: &Snapshot) -> String {
    let mut out = serde_json::to_string_pretty(snap).expect("snapshot serialization cannot fail");
    out.push('\n');
    out
}

/// Compare every fixture in `dir` against its snapshot. Returns the names of
/// drifted or missing fixtures; with `update = true` it rewrites every drifted
/// snapshot instead (the `INSIGHTS_GOLDEN_UPDATE=1` refresh switch).
fn evaluate(dir: &Path, update: bool) -> Vec<String> {
    let mut drifted = Vec::new();
    for case in golden_cases_in(dir) {
        let raw = match fs::read_to_string(&case.fixture) {
            Ok(raw) => raw,
            Err(err) => {
                drifted.push(format!("{}: unreadable fixture: {err}", case.name));
                continue;
            }
        };
        let rendered = match normalize_fixture(&raw) {
            Ok(snap) => render(&snap),
            Err(err) => {
                drifted.push(format!("{}: {err}", case.name));
                continue;
            }
        };
        let current = fs::read_to_string(&case.snapshot).ok();
        if current.as_deref() == Some(rendered.as_str()) {
            continue;
        }
        if update {
            write_atomic(&case.snapshot, rendered.as_bytes());
        } else {
            drifted.push(case.name);
        }
    }
    drifted
}

static FILE_COUNTER: AtomicUsize = AtomicUsize::new(0);

/// Refresh writes go through a unique temp file + rename so a concurrent
/// reader (or a parallel test refreshing the same snapshot) never observes a
/// torn or half-published file.
fn write_atomic(path: &Path, content: &[u8]) {
    let file_name = path.file_name().unwrap_or_default().to_string_lossy();
    let tmp_name = format!(
        "{file_name}.{process_id}.{counter}.tmp",
        process_id = std::process::id(),
        counter = FILE_COUNTER.fetch_add(1, Ordering::SeqCst)
    );
    let tmp = path
        .parent()
        .map(|parent| parent.join(&tmp_name))
        .unwrap_or_else(|| PathBuf::from(tmp_name));
    fs::write(&tmp, content).expect("write snapshot temp file");
    fs::rename(&tmp, path).expect("publish refreshed snapshot");
}

static TEMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

fn fresh_temp_dir() -> PathBuf {
    let dir = env::temp_dir().join(format!(
        "insights_goldens_{}_{}",
        std::process::id(),
        TEMP_COUNTER.fetch_add(1, Ordering::SeqCst)
    ));
    fs::create_dir_all(&dir).expect("create temp fixtures dir");
    dir
}

#[test]
fn insights_goldens_ten_fixtures_each_with_a_matching_snapshot() {
    if env::var(UPDATE_SWITCH).is_ok_and(|value| value == "1") {
        // Refresh mode: the refresh may not have run in a parallel test yet,
        // so guarantee the snapshot siblings exist before asserting on them.
        let _ = evaluate(&default_fixtures_dir(), true);
    }
    let cases = golden_cases();
    assert_eq!(
        cases.len(),
        10,
        "expected exactly the ten §48 scenario fixtures, found {}",
        cases.len()
    );
    for case in &cases {
        assert!(
            case.snapshot.is_file(),
            "fixture {} has no .snapshot.json sibling",
            case.name
        );
    }
}

#[test]
fn insights_goldens_every_fixture_matches_its_snapshot_byte_for_byte() {
    let update = env::var(UPDATE_SWITCH).is_ok_and(|value| value == "1");
    let drifted = evaluate(&default_fixtures_dir(), update);
    assert!(
        drifted.is_empty(),
        "snapshot drift on: {} (set INSIGHTS_GOLDEN_UPDATE=1 to refresh snapshots intentionally)",
        drifted.join(", ")
    );
}

#[test]
fn insights_goldens_a_mutated_snapshot_makes_the_harness_fail() {
    let dir = fresh_temp_dir();
    let source = golden_cases().remove(0).fixture;
    fs::copy(&source, dir.join("case.jsonl")).expect("seed temp fixture");
    evaluate(&dir, true);
    let snapshot_path = dir.join("case.snapshot.json");
    let bytes = fs::read(&snapshot_path).expect("read seeded snapshot");
    // Deliberately truncate the snapshot so it no longer matches the fixture.
    fs::write(&snapshot_path, &bytes[..bytes.len() - 3]).expect("write mutated snapshot");
    let drifted = evaluate(&dir, false);
    assert_eq!(drifted, vec!["case".to_string()]);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn insights_goldens_the_update_switch_rewrites_and_is_idempotent() {
    let dir = fresh_temp_dir();
    let source = golden_cases().remove(0).fixture;
    fs::copy(&source, dir.join("case.jsonl")).expect("seed temp fixture");
    // A missing snapshot is drift until the refresh switch writes it.
    assert_eq!(evaluate(&dir, false), vec!["case".to_string()]);
    assert!(evaluate(&dir, true).is_empty());
    let snapshot_path = dir.join("case.snapshot.json");
    let first_pass = fs::read(&snapshot_path).expect("read refreshed snapshot");
    assert!(first_pass.starts_with(b"{"));
    assert!(first_pass.ends_with(b"}\n"));
    assert!(evaluate(&dir, false).is_empty());
    // Re-running the refresh switch is byte-stable: no hidden churn.
    assert!(evaluate(&dir, true).is_empty());
    assert_eq!(
        fs::read(&snapshot_path).expect("re-read refreshed snapshot"),
        first_pass
    );
    let _ = fs::remove_dir_all(&dir);
}
