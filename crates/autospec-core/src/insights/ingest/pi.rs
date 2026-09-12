//! Pi JSONL session adapter and incremental ingest (issue #3836, spec §6).
//!
//! A Pi session is a JSONL file: one JSON record per line. [`PiAdapter`]
//! discovers every `*.jsonl` file under its root, reads only the lines
//! after the stored resume cursor, and normalizes each line into §7
//! [`NormalizedEvent`]s. [`ingest`] writes them into the §34 `sessions`
//! and `session_events` tables.
//!
//! Idempotency (§6 "prevent duplicate ingestion"): every event carries a
//! stable `event_id` derived from the file, line and part position, and
//! inserts use `ON CONFLICT (event_id) DO NOTHING` — a second ingest, or a
//! full re-read after cursor loss, changes nothing.
//!
//! Quarantine (§50 "corrupted session -> quarantine session and continue"):
//! a line that is not parseable JSON — or that fails to normalize — is
//! written to `insights_quarantine` (one row per line, idempotent) and the
//! run continues. One bad record never stops a run, and one bad line never
//! loses the rest of the session.
//!
//! Incremental (§6 / §51 "incremental processing rather than repeated
//! full-history scans"): the resume cursor is the 0-based index of the
//! last line processed, stored per source URI in the
//! [`IngestCursor`] table as the stored half of the `SessionRef`
//! contract. It advances only after the batched inserts succeed, so a run
//! interrupted mid-file re-reads its tail on the next pass and dedups by
//! `event_id` instead of losing or duplicating events.
//!
//! Privacy (§39): raw record text is written only to the local autospec
//! database (`session_events.payload`, `insights_quarantine.raw`); it is
//! never logged, indexed, or transmitted.

use std::path::{Path, PathBuf};

use sqlx::{AnyPool, Row};

use crate::error::AutospecError;
use crate::insights::events::{EventType, NormalizedEvent, SessionAdapter, SessionRef};

use super::{IngestCursor, IngestReport};

/// Source kind recorded on every [`SessionRef`] this adapter yields.
pub const SOURCE: &str = "pi_jsonl";
/// Harness name recorded on the `sessions` rows this adapter creates.
pub const HARNESS: &str = "pi";
/// Rows per `INSERT ... ON CONFLICT (event_id) DO NOTHING` batch (§51
/// throughput: bulk writes, not one round-trip per event).
const INSERT_BATCH: usize = 500;

const SESSIONS_DDL: &str = "CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY,
    repo TEXT NOT NULL,
    work_item_id TEXT,
    harness TEXT,
    model TEXT,
    status TEXT NOT NULL,
    started_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    ended_at TIMESTAMP,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
)";

/// Portable (PostgreSQL 16 and SQLite, D10). `event_id` is the #3836
/// dedup key; `occurred_at` is Unix seconds as REAL so fractional
/// timestamps stay portable without a date library.
const SESSION_EVENTS_DDL: &str = "CREATE TABLE IF NOT EXISTS session_events (
    session_id TEXT NOT NULL REFERENCES sessions (id),
    event_id TEXT NOT NULL UNIQUE,
    seq INTEGER NOT NULL,
    event_type TEXT NOT NULL,
    occurred_at REAL,
    payload TEXT,
    PRIMARY KEY (session_id, seq)
)";

/// Quarantined corrupt records (§50). `raw` holds the verbatim line;
/// deliberately not indexed (§39).
const QUARANTINE_DDL: &str = "CREATE TABLE IF NOT EXISTS insights_quarantine (
    id TEXT PRIMARY KEY,
    source TEXT NOT NULL,
    uri TEXT NOT NULL,
    line INTEGER,
    reason TEXT NOT NULL,
    raw TEXT,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
)";

/// Pi JSONL session adapter (spec §6 `SessionAdapter`).
#[derive(Debug, Clone)]
pub struct PiAdapter {
    /// Root directory walked by [`SessionAdapter::discover`].
    pub root: PathBuf,
}

impl PiAdapter {
    /// An adapter that discovers `*.jsonl` sessions under `root`.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
}

impl SessionAdapter for PiAdapter {
    /// Every `*.jsonl` file under `root` (recursively) is one session.
    /// A missing root yields no sessions, not an error.
    fn discover(&self) -> Result<Vec<SessionRef>, AutospecError> {
        let mut refs = Vec::new();
        if self.root.is_dir() {
            walk_jsonl(&self.root, &mut refs).map_err(|err| {
                AutospecError::io("discover pi sessions", self.root.display().to_string(), err)
            })?;
        }
        refs.sort_by(|a, b| a.uri.cmp(&b.uri));
        Ok(refs)
    }

    /// The raw lines of one session file, as
    /// `{"line": <0-based index>, "session_id": <file stem>, "uri": …,
    /// "raw": <verbatim line>}` values, starting after
    /// [`SessionRef::resume_cursor`] (the last processed line index).
    /// Blank lines are not records. A corrupt cursor is an error, never
    /// silently treated as "start over".
    fn read(&self, source: &SessionRef) -> Result<Vec<serde_json::Value>, AutospecError> {
        let path = Path::new(&source.uri);
        let text = std::fs::read_to_string(path)
            .map_err(|err| AutospecError::io("read pi session", path.display().to_string(), err))?;
        let start = match &source.resume_cursor {
            None => 0u64,
            Some(cursor) => cursor
                .parse::<u64>()
                .map_err(|_| {
                    AutospecError::state(
                        IngestCursor::DEFAULT_TABLE,
                        format!(
                            "unparseable resume cursor {cursor:?} for {}",
                            path.display()
                        ),
                    )
                })?
                .saturating_add(1),
        };
        let start = usize::try_from(start).unwrap_or(usize::MAX);
        let session_id = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("unknown")
            .to_string();
        Ok(text
            .lines()
            .enumerate()
            .skip(start)
            .filter(|(_, line)| !line.trim().is_empty())
            .map(|(index, line)| {
                serde_json::json!({
                    "line": index,
                    "session_id": session_id,
                    "uri": source.uri,
                    "raw": line,
                })
            })
            .collect())
    }

    /// Map one raw line to zero or more §7 [`NormalizedEvent`]s.
    ///
    /// Record shape: `{"type":"message","message":{"role":"user" |
    /// "assistant","content": string | [parts],"usage":{"input","output"},
    /// "timestamp": …}}` with parts of type `text`, `toolCall` (name) or
    /// `toolResult` (name, `isError`). A line that is not parseable JSON,
    /// or a message record with no role, is an [`AutospecError::Parse`] —
    /// the quarantine trigger in [`ingest`]. A record that parses but is
    /// not a `message` record yields no events.
    fn normalize(&self, event: &serde_json::Value) -> Result<Vec<NormalizedEvent>, AutospecError> {
        let line = event
            .get("line")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        let session_id = event
            .get("session_id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown");
        let raw = event
            .get("raw")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| AutospecError::parse("pi jsonl line", "raw line text missing"))?;

        let record: serde_json::Value = serde_json::from_str(raw).map_err(|err| {
            AutospecError::parse(format!("pi jsonl line {line}"), err.to_string())
        })?;

        let is_message = record.get("type").and_then(serde_json::Value::as_str) == Some("message")
            && record
                .get("message")
                .is_some_and(serde_json::Value::is_object);
        if !is_message {
            return Ok(Vec::new());
        }
        let message = record["message"].clone();
        let role = message
            .get("role")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                AutospecError::parse(
                    format!("pi jsonl line {line}"),
                    "message record has no role",
                )
            })?;

        let ctx = EventContext {
            session_id,
            line,
            record: &record,
            timestamp: message
                .get("timestamp")
                .cloned()
                .unwrap_or(serde_json::json!(0)),
            input: message
                .pointer("/usage/input")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0),
            output: message
                .pointer("/usage/output")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0),
            model: record
                .get("model")
                .or_else(|| message.get("model"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_string),
        };

        // (event type, tool name, part index); `None` means the whole
        // record (string content, or no recognizable parts).
        let mut kinds: Vec<(EventType, Option<String>, Option<usize>)> = Vec::new();
        match &message["content"] {
            serde_json::Value::Array(parts) => {
                for (index, part) in parts.iter().enumerate() {
                    let tool = part
                        .get("name")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_string);
                    match part.get("type").and_then(serde_json::Value::as_str) {
                        Some("toolCall") => kinds.push((EventType::ToolCall, tool, Some(index))),
                        Some("toolResult") => {
                            let failed = part
                                .get("isError")
                                .and_then(serde_json::Value::as_bool)
                                .unwrap_or(false);
                            let event_type = if failed {
                                EventType::ToolError
                            } else {
                                EventType::ToolResult
                            };
                            kinds.push((event_type, tool, Some(index)));
                        }
                        Some("text") => kinds.push((message_type(role), None, Some(index))),
                        _ => {}
                    }
                }
                if kinds.is_empty() {
                    kinds.push((message_type(role), None, None));
                }
            }
            _ => kinds.push((message_type(role), None, None)),
        }

        kinds
            .into_iter()
            .map(|(event_type, tool, part)| build_event(&ctx, event_type, tool, part))
            .collect()
    }
}

/// §7 event type of a whole-record message event, by role.
fn message_type(role: &str) -> EventType {
    if role == "assistant" {
        EventType::AssistantMessage
    } else {
        EventType::UserMessage
    }
}

/// Context shared by every event built from one Pi record.
struct EventContext<'a> {
    session_id: &'a str,
    line: u64,
    record: &'a serde_json::Value,
    timestamp: serde_json::Value,
    input: u64,
    output: u64,
    model: Option<String>,
}

/// Build one §7 [`NormalizedEvent`].
///
/// `event_id` is stable across runs: `{session}:{line}` for a whole-record
/// event, `{session}:{line}:{part}` for one content part. That stability is
/// what makes `ON CONFLICT (event_id) DO NOTHING` a real dedup.
fn build_event(
    ctx: &EventContext,
    event_type: EventType,
    tool: Option<String>,
    part: Option<usize>,
) -> Result<NormalizedEvent, AutospecError> {
    let event_id = match part {
        Some(index) => format!("{}:{}:{}", ctx.session_id, ctx.line, index),
        None => format!("{}:{}", ctx.session_id, ctx.line),
    };
    let value = serde_json::json!({
        "event_id": event_id,
        "session_id": ctx.session_id,
        "parent_session_id": null,
        "timestamp": ctx.timestamp,
        "repo": null,
        "branch": null,
        "work_item_id": null,
        "agent_role": null,
        "provider": "pi",
        "model": ctx.model,
        "event_type": event_type,
        "tool": tool,
        "payload": ctx.record,
        "tokens": { "input": ctx.input, "output": ctx.output },
    });
    serde_json::from_value(value)
        .map_err(|err| AutospecError::parse("normalized event", err.to_string()))
}

/// Recursively collect `*.jsonl` files under `dir`.
fn walk_jsonl(dir: &Path, refs: &mut Vec<SessionRef>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            walk_jsonl(&path, refs)?;
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
            refs.push(SessionRef {
                source: SOURCE.to_string(),
                uri: path.to_string_lossy().into_owned(),
                resume_cursor: None,
            });
        }
    }
    Ok(())
}

/// Ensure every table the ingest path needs exists, and that the
/// `session_events` dedup key is present. Fails closed (naming the missing
/// `event_id` column) when a pre-existing `session_events` from the #3827
/// migration lacks it, rather than silently deduping on nothing.
pub async fn ensure_ingest_schema(
    pool: &AnyPool,
    cursor: &IngestCursor,
) -> Result<(), AutospecError> {
    for ddl in [SESSIONS_DDL, SESSION_EVENTS_DDL, QUARANTINE_DDL] {
        sqlx::query(ddl)
            .execute(pool)
            .await
            .map_err(|err| AutospecError::state("insights ingest schema", err.to_string()))?;
    }
    let cursor_ddl = format!(
        "CREATE TABLE IF NOT EXISTS {table} (
            uri TEXT PRIMARY KEY,
            resume_cursor TEXT,
            updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
        )",
        table = cursor.table
    );
    sqlx::query(&cursor_ddl)
        .execute(pool)
        .await
        .map_err(|err| AutospecError::state("insights ingest schema", err.to_string()))?;
    sqlx::query("SELECT event_id FROM session_events LIMIT 1")
        .fetch_optional(pool)
        .await
        .map_err(|err| {
            AutospecError::state(
                "session_events",
                format!("event_id dedup key missing or unreadable: {err}"),
            )
        })?;
    Ok(())
}

/// Ingest every session the adapter discovers (spec §6).
///
/// Per source: resume from the stored cursor, read the new lines,
/// normalize (quarantining bad lines and continuing), upsert the
/// `sessions` row, insert the events batched with
/// `ON CONFLICT (event_id) DO NOTHING`, and only then advance the stored
/// cursor past every line read. A partial run therefore re-reads its tail
/// on the next pass and dedups by `event_id` — the cursor and the rows
/// stay consistent.
pub async fn ingest(
    pool: &AnyPool,
    adapter: &PiAdapter,
    cursor: &IngestCursor,
) -> Result<IngestReport, AutospecError> {
    ensure_ingest_schema(pool, cursor).await?;
    let mut report = IngestReport::default();

    for source in adapter.discover()? {
        report.sessions_discovered += 1;

        let mut source = source;
        source.resume_cursor = load_cursor(pool, cursor.table, &source.uri).await?;

        let raws = match adapter.read(&source) {
            Ok(raws) => raws,
            Err(err) => {
                // An unreadable file is a corrupted session: quarantine
                // the source and continue (§50).
                quarantine(pool, &source, None, "", &err).await?;
                report.quarantined += 1;
                continue;
            }
        };
        if raws.is_empty() {
            // Nothing new: a healthy idle run leaves the cursor untouched.
            continue;
        }

        let session_id = raws[0]
            .get("session_id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown");
        let mut next_seq = next_seq(pool, session_id).await?;

        let mut rows: Vec<(NormalizedEvent, i32)> = Vec::new();
        for raw in &raws {
            report.events_read += 1;
            match adapter.normalize(raw) {
                Ok(events) => {
                    for event in events {
                        rows.push((event, next_seq));
                        next_seq = next_seq.checked_add(1).ok_or_else(|| {
                            AutospecError::state("session_events", "seq overflow")
                        })?;
                    }
                }
                Err(err) => {
                    let line = raw
                        .get("line")
                        .and_then(|v| v.as_u64())
                        .and_then(|v| v.try_into().ok());
                    let text = raw
                        .get("raw")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("");
                    quarantine(pool, &source, line, text, &err).await?;
                    report.quarantined += 1;
                }
            }
        }

        if !rows.is_empty() {
            upsert_session(pool, session_id, &source.uri).await?;
            let inserted = insert_events_batched(pool, &rows).await?;
            report.events_inserted += inserted;
            report.events_deduplicated += rows.len() as u64 - inserted;
        }

        if let Some(last_line) = raws
            .last()
            .and_then(|raw| raw.get("line").and_then(serde_json::Value::as_u64))
        {
            store_cursor(pool, cursor.table, &source.uri, last_line.to_string()).await?;
            report.cursors_updated += 1;
        }
    }

    Ok(report)
}

/// The next `seq` for a session: one past the stored maximum, so a resume
/// run appends without colliding with the primary key.
async fn next_seq(pool: &AnyPool, session_id: &str) -> Result<i32, AutospecError> {
    let row = sqlx::query(
        "SELECT CAST(COALESCE(MAX(seq), -1) AS TEXT) FROM session_events WHERE session_id = ?",
    )
    .bind(session_id)
    .fetch_one(pool)
    .await
    .map_err(|err| AutospecError::state("session_events", err.to_string()))?;
    let text: String = row
        .try_get(0)
        .map_err(|err| AutospecError::state("session_events", err.to_string()))?;
    let max: i64 = text.trim().parse().map_err(|_| {
        AutospecError::state("session_events", format!("unparseable max seq {text:?}"))
    })?;
    i32::try_from(max)
        .ok()
        .and_then(|max| max.checked_add(1))
        .ok_or_else(|| AutospecError::state("session_events", "seq overflow"))
}

/// Upsert the `sessions` row for one ingested session. `repo` is the
/// session file's parent directory name (the best local stand-in for a
/// repository identity a Pi JSONL session does not carry); `status` stays
/// `active` because a JSONL file may still be appended to.
async fn upsert_session(pool: &AnyPool, session_id: &str, uri: &str) -> Result<(), AutospecError> {
    let repo = Path::new(uri)
        .parent()
        .and_then(|parent| parent.file_name())
        .and_then(|name| name.to_str())
        .unwrap_or("unknown")
        .to_string();
    sqlx::query(
        "INSERT INTO sessions (id, repo, harness, status) VALUES (?, ?, ?, 'active') \
         ON CONFLICT (id) DO NOTHING",
    )
    .bind(session_id)
    .bind(&repo)
    .bind(HARNESS)
    .execute(pool)
    .await
    .map_err(|err| AutospecError::state("sessions", err.to_string()))?;
    Ok(())
}

/// Insert events in batches of [`INSERT_BATCH`] with
/// `ON CONFLICT (event_id) DO NOTHING`; returns the number of rows
/// actually inserted (conflicts count as skipped).
async fn insert_events_batched(
    pool: &AnyPool,
    rows: &[(NormalizedEvent, i32)],
) -> Result<u64, AutospecError> {
    let mut inserted = 0u64;
    for chunk in rows.chunks(INSERT_BATCH) {
        let mut sql = String::from(
            "INSERT INTO session_events (session_id, event_id, seq, event_type, occurred_at, payload) VALUES",
        );
        for (i, _) in chunk.iter().enumerate() {
            sql.push_str(if i == 0 {
                " (?, ?, ?, ?, ?, ?)"
            } else {
                ", (?, ?, ?, ?, ?, ?)"
            });
        }
        sql.push_str(" ON CONFLICT (event_id) DO NOTHING");
        let payloads: Vec<String> = chunk
            .iter()
            .map(|(event, _)| {
                serde_json::to_string(&event.payload)
                    .map_err(|err| AutospecError::parse("session_events payload", err.to_string()))
            })
            .collect::<Result<Vec<String>, AutospecError>>()?;
        let mut query = sqlx::query(&sql);
        for ((event, seq), payload) in chunk.iter().zip(&payloads) {
            query = query
                .bind(&event.session_id)
                .bind(&event.event_id)
                .bind(*seq)
                .bind(event_type_name(event))
                .bind(event.timestamp)
                .bind(payload);
        }
        inserted += query
            .execute(pool)
            .await
            .map_err(|err| AutospecError::state("session_events", err.to_string()))?
            .rows_affected();
    }
    Ok(inserted)
}

/// The §7 snake_case name of an event type.
fn event_type_name(event: &NormalizedEvent) -> String {
    serde_json::to_value(event.event_type)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_else(|| "unknown".to_string())
}

/// Write one quarantine row (idempotent: re-quarantining the same line or
/// source keeps a single row).
async fn quarantine(
    pool: &AnyPool,
    source: &SessionRef,
    line: Option<i32>,
    raw: &str,
    err: &AutospecError,
) -> Result<(), AutospecError> {
    let id = match line {
        Some(n) => format!("{}:L{n}", source.uri),
        None => format!("{}:read", source.uri),
    };
    sqlx::query(
        "INSERT INTO insights_quarantine (id, source, uri, line, reason, raw) \
         VALUES (?, ?, ?, ?, ?, ?) ON CONFLICT (id) DO NOTHING",
    )
    .bind(&id)
    .bind(&source.source)
    .bind(&source.uri)
    .bind(line)
    .bind(err.to_string())
    .bind(raw)
    .execute(pool)
    .await
    .map_err(|err| AutospecError::state("insights_quarantine", err.to_string()))?;
    Ok(())
}

async fn load_cursor(
    pool: &AnyPool,
    table: &str,
    uri: &str,
) -> Result<Option<String>, AutospecError> {
    let sql = format!("SELECT resume_cursor FROM {table} WHERE uri = ?");
    Ok(sqlx::query(&sql)
        .bind(uri)
        .fetch_optional(pool)
        .await
        .map_err(|err| AutospecError::state(table, err.to_string()))?
        .map(|row| row.try_get::<String, _>(0).unwrap_or_default()))
}

async fn store_cursor(
    pool: &AnyPool,
    table: &str,
    uri: &str,
    value: String,
) -> Result<(), AutospecError> {
    let sql = format!(
        "INSERT INTO {table} (uri, resume_cursor, updated_at) VALUES (?, ?, CURRENT_TIMESTAMP) \
         ON CONFLICT (uri) DO UPDATE SET \
         resume_cursor = excluded.resume_cursor, updated_at = CURRENT_TIMESTAMP"
    );
    sqlx::query(&sql)
        .bind(uri)
        .bind(value)
        .execute(pool)
        .await
        .map_err(|err| AutospecError::state(table, err.to_string()))?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::insights::events::{EventType, Tokens};
    use sqlx::Row;
    use std::io::Write;
    use std::sync::atomic::{AtomicU32, Ordering};

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    fn next_counter() -> u32 {
        COUNTER.fetch_add(1, Ordering::SeqCst)
    }

    const FIXTURE_LINES: usize = 200;
    const MALFORMED_LINE: usize = 100;

    /// Disposable PostgreSQL 16 (under Apptainer in the operator full
    /// run) via `AUTOSPEC_TEST_DB_URL`; otherwise a disposable per-test
    /// SQLite file. A Postgres URL gets a dedicated per-test database
    /// (dropped again by [`close_test_pool`]) so parallel test binaries
    /// never share insights tables. No database mocks anywhere.
    async fn open_test_pool() -> (sqlx::AnyPool, String, String) {
        let n = next_counter();
        let pid = std::process::id();
        match std::env::var("AUTOSPEC_TEST_DB_URL") {
            Ok(url) if url.starts_with("postgres") && !url.trim().is_empty() => {
                let db = format!("ingest_{pid}_{n}");
                let admin = crate::resources::db::open_shared_db(&url)
                    .await
                    .expect("admin pool must open");
                sqlx::query(&format!("CREATE DATABASE {db}"))
                    .execute(&admin)
                    .await
                    .expect("dedicated database must be created");
                let (base, _) = url
                    .rsplit_once('/')
                    .expect("postgres url carries a database name");
                let pool = crate::resources::db::open_shared_db(&format!("{base}/{db}"))
                    .await
                    .expect("test pool must open");
                (pool, url, db)
            }
            _ => (
                crate::resources::db::open_shared_db(&format!(
                    "sqlite://{}/autospec-insights-ingest-{pid}-{n}.db",
                    std::env::temp_dir().display()
                ))
                .await
                .expect("test pool must open"),
                String::new(),
                String::new(),
            ),
        }
    }

    async fn close_test_pool(pool: sqlx::AnyPool, admin_url: &str, db: &str) {
        drop(pool);
        if db.is_empty() || admin_url.is_empty() {
            return;
        }
        if let Ok(admin) = crate::resources::db::open_shared_db(admin_url).await {
            let ddl = format!("DROP DATABASE IF EXISTS {db} WITH (FORCE)");
            let _ = sqlx::query(&ddl).execute(&admin).await;
        }
    }

    async fn count(sql: &str, pool: &sqlx::AnyPool) -> i64 {
        let row = sqlx::raw_sql(sql).fetch_one(pool).await.unwrap();
        row.try_get::<i64, _>(0).unwrap()
    }

    /// One fixture line: line 0 is a plain user message; the rest cycle
    /// assistant `toolCall`, user `toolResult`, assistant text.
    fn fixture_line(i: usize) -> String {
        let ts = format!("2026-09-08T18:00:{:02}Z", i % 60);
        if i == 0 {
            return serde_json::json!({
                "type": "message",
                "id": format!("r{i}"),
                "message": {
                    "role": "user",
                    "content": "hello pi",
                    "timestamp": ts
                }
            })
            .to_string();
        }
        match i % 3 {
            1 => serde_json::json!({
                "type": "message",
                "id": format!("r{i}"),
                "message": {
                    "role": "assistant",
                    "model": "qwen3",
                    "content": [{
                        "type": "toolCall",
                        "name": "bash",
                        "arguments": { "command": "ls" }
                    }],
                    "usage": { "input": 100 + i, "output": 10 },
                    "timestamp": ts
                }
            })
            .to_string(),
            2 => serde_json::json!({
                "type": "message",
                "id": format!("r{i}"),
                "message": {
                    "role": "user",
                    "content": [{ "type": "toolResult", "name": "bash", "content": "ok" }],
                    "timestamp": ts
                }
            })
            .to_string(),
            _ => serde_json::json!({
                "type": "message",
                "id": format!("r{i}"),
                "message": {
                    "role": "assistant",
                    "model": "qwen3",
                    "content": [{ "type": "text", "text": "thinking" }],
                    "usage": { "input": 50 + i, "output": 5 },
                    "timestamp": ts
                }
            })
            .to_string(),
        }
    }

    /// A truncated line: valid JSON up to the cut, then gone.
    fn malformed_line(i: usize) -> String {
        format!(
            "{{\"type\":\"message\",\"id\":\"r{i}\",\"message\":{{\"role\":\"assistant\",\"content\":[{{\"type\":\"toolCall\",\"name\":\"bash\""
        )
    }

    /// Write the 200-line fixture into a fresh unique root;
    /// `malformed_at` (when `Some`) replaces that line with a truncated
    /// record. Returns `(root, file path)`.
    fn write_fixture(malformed_at: Option<usize>) -> (PathBuf, PathBuf) {
        let n = next_counter();
        let pid = std::process::id();
        let root = std::env::temp_dir().join(format!("autospec-pi-fixture-{pid}-{n}"));
        std::fs::create_dir_all(&root).expect("fixture dir");
        let path = root.join("sess-abc123.jsonl");
        let mut text = String::new();
        for i in 0..FIXTURE_LINES {
            let line = match malformed_at {
                Some(m) if m == i => malformed_line(i),
                _ => fixture_line(i),
            };
            text.push_str(&line);
            text.push('\n');
        }
        std::fs::write(&path, text).expect("fixture file");
        (root, path)
    }

    // -- TDD: the failing idempotence test lands first -----------------

    #[tokio::test]
    async fn second_ingest_is_idempotent() {
        let (root, _path) = write_fixture(Some(MALFORMED_LINE));
        let (pool, admin, db) = open_test_pool().await;
        let adapter = PiAdapter::new(&root);

        let first = ingest(&pool, &adapter, &IngestCursor::default())
            .await
            .expect("first ingest must succeed");
        assert_eq!(
            first.events_inserted, 199,
            "200 fixture lines minus 1 quarantined must land"
        );
        assert_eq!(first.quarantined, 1);

        let before = count("SELECT COUNT(*) FROM session_events", &pool).await;
        assert_eq!(before, 199);

        let second = ingest(&pool, &adapter, &IngestCursor::default())
            .await
            .expect("second ingest must return Ok");
        assert_eq!(
            second.events_read, 0,
            "stored cursor must skip already-processed lines"
        );
        assert_eq!(second.events_inserted, 0);

        let after = count("SELECT COUNT(*) FROM session_events", &pool).await;
        assert_eq!(after, before, "a second ingest must leave the count equal");
        assert_eq!(
            count("SELECT COUNT(*) FROM insights_quarantine", &pool).await,
            1
        );

        close_test_pool(pool, &admin, &db).await;
    }

    #[tokio::test]
    async fn reingest_after_cursor_loss_still_dedups_on_event_id() {
        let (root, _path) = write_fixture(Some(MALFORMED_LINE));
        let (pool, admin, db) = open_test_pool().await;
        let adapter = PiAdapter::new(&root);

        let first = ingest(&pool, &adapter, &IngestCursor::default())
            .await
            .unwrap();
        assert_eq!(first.events_inserted, 199);

        // The cursor is lost (e.g. a partial run's cursor write was
        // rolled back): the full re-read must dedup on event_id alone.
        sqlx::query("DELETE FROM insights_ingest_cursors")
            .execute(&pool)
            .await
            .unwrap();

        let second = ingest(&pool, &adapter, &IngestCursor::default())
            .await
            .expect("re-read must succeed");
        assert_eq!(second.events_read, 200);
        assert_eq!(second.events_inserted, 0);
        assert_eq!(second.events_deduplicated, 199);
        assert_eq!(second.quarantined, 1);

        assert_eq!(
            count("SELECT COUNT(*) FROM session_events", &pool).await,
            199
        );
        // Re-quarantining the same line is idempotent: still one row.
        assert_eq!(
            count("SELECT COUNT(*) FROM insights_quarantine", &pool).await,
            1
        );

        close_test_pool(pool, &admin, &db).await;
    }

    #[tokio::test]
    async fn at_least_ninety_five_percent_of_fixture_events_land() {
        let (root, _path) = write_fixture(Some(MALFORMED_LINE));
        let (pool, admin, db) = open_test_pool().await;
        let adapter = PiAdapter::new(&root);

        ingest(&pool, &adapter, &IngestCursor::default())
            .await
            .unwrap();

        let landed = count("SELECT COUNT(*) FROM session_events", &pool).await as f64;
        assert!(
            landed >= 0.95 * FIXTURE_LINES as f64,
            "only {landed}/{} fixture events reached session_events",
            FIXTURE_LINES
        );

        close_test_pool(pool, &admin, &db).await;
    }

    #[tokio::test]
    async fn malformed_line_is_quarantined_and_the_run_continues() {
        let (root, _path) = write_fixture(Some(MALFORMED_LINE));
        let (pool, admin, db) = open_test_pool().await;
        let adapter = PiAdapter::new(&root);

        let report = ingest(&pool, &adapter, &IngestCursor::default())
            .await
            .expect("one bad line must not stop the run");
        assert_eq!(report.quarantined, 1);

        let row = sqlx::raw_sql("SELECT CAST(line AS TEXT), reason FROM insights_quarantine")
            .fetch_one(&pool)
            .await
            .expect("exactly one quarantine row");
        assert_eq!(
            row.try_get::<String, _>(0).unwrap(),
            MALFORMED_LINE.to_string()
        );
        assert!(
            row.try_get::<String, _>(1).unwrap().contains("parse"),
            "the reason must say the line did not parse"
        );

        // One corrupt line must not lose the session: the events on the
        // lines directly before and after the bad line both landed.
        let neighbours = count(
            "SELECT COUNT(*) FROM session_events WHERE event_id IN \
             ('sess-abc123:99:0', 'sess-abc123:101:0')",
            &pool,
        )
        .await;
        assert_eq!(neighbours, 2);

        close_test_pool(pool, &admin, &db).await;
    }

    #[tokio::test]
    async fn ingest_resumes_from_stored_cursor_and_appends_newer_events() {
        let (root, path) = write_fixture(Some(MALFORMED_LINE));
        let (pool, admin, db) = open_test_pool().await;
        let adapter = PiAdapter::new(&root);

        let first = ingest(&pool, &adapter, &IngestCursor::default())
            .await
            .unwrap();
        assert_eq!(first.events_inserted, 199);
        assert_eq!(first.cursors_updated, 1);

        // The live session file grows: three newer events are appended.
        let mut appended = String::new();
        for i in 200..203 {
            appended.push_str(&fixture_line(i));
            appended.push('\n');
        }
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        file.write_all(appended.as_bytes()).unwrap();

        let second = ingest(&pool, &adapter, &IngestCursor::default())
            .await
            .expect("resume run must succeed");
        assert_eq!(second.events_read, 3, "only the newer lines are read");
        assert_eq!(second.events_inserted, 3);
        assert_eq!(second.events_deduplicated, 0);

        assert_eq!(
            count("SELECT COUNT(*) FROM session_events", &pool).await,
            202
        );
        // No duplicate of the first line after the resume.
        assert_eq!(
            count(
                "SELECT COUNT(*) FROM session_events WHERE event_id = 'sess-abc123:0'",
                &pool
            )
            .await,
            1
        );
        // The stored cursor advanced past the new lines.
        let cursor = sqlx::query("SELECT resume_cursor FROM insights_ingest_cursors WHERE uri = ?")
            .bind(path.to_string_lossy().as_ref())
            .fetch_one(&pool)
            .await
            .expect("cursor row must exist");
        assert_eq!(cursor.try_get::<String, _>(0).unwrap(), "202");

        close_test_pool(pool, &admin, &db).await;
    }

    // -- adapter unit tests --------------------------------------------

    #[test]
    fn discover_yields_one_ref_per_jsonl_file_and_ignores_others() {
        let n = next_counter();
        let pid = std::process::id();
        let root = std::env::temp_dir().join(format!("autospec-pi-discover-{pid}-{n}"));
        std::fs::create_dir_all(root.join("nested")).unwrap();
        std::fs::write(root.join("a.jsonl"), "{}\n").unwrap();
        std::fs::write(root.join("nested").join("b.jsonl"), "{}\n").unwrap();
        std::fs::write(root.join("notes.txt"), "not a session").unwrap();

        let adapter = PiAdapter::new(&root);
        let refs = adapter.discover().unwrap();
        assert_eq!(refs.len(), 2, "only .jsonl files are session sources");
        for ref_ in &refs {
            assert_eq!(ref_.source, "pi_jsonl");
            assert!(ref_.resume_cursor.is_none());
            assert!(ref_.uri.ends_with(".jsonl"));
        }
        let uris: Vec<&str> = refs.iter().map(|r| r.uri.as_str()).collect();
        assert!(uris[0] < uris[1], "refs must be sorted by uri");

        // A missing root yields no sessions, not an error.
        let missing = PiAdapter::new(root.join("no-such-dir"));
        assert!(missing.discover().unwrap().is_empty());
    }

    #[test]
    fn read_honours_the_resume_cursor() {
        let (root, _path) = write_fixture(None);
        let adapter = PiAdapter::new(&root);
        let refs = adapter.discover().unwrap();
        let source = &refs[0];

        let all = adapter.read(source).unwrap();
        assert_eq!(all.len(), FIXTURE_LINES);

        let mut done = source.clone();
        done.resume_cursor = Some((FIXTURE_LINES - 1).to_string());
        assert!(
            adapter.read(&done).unwrap().is_empty(),
            "a fully-processed file reads nothing new"
        );

        let mut tail = source.clone();
        tail.resume_cursor = Some((FIXTURE_LINES - 2).to_string());
        let read_tail = adapter.read(&tail).unwrap();
        assert_eq!(read_tail.len(), 1);
        assert_eq!(
            read_tail[0]["line"].as_u64(),
            Some(FIXTURE_LINES as u64 - 1)
        );

        // A corrupt cursor fails closed: start over is someone else's
        // decision, never the reader's.
        let mut corrupt = source.clone();
        corrupt.resume_cursor = Some("not-a-number".into());
        assert!(adapter.read(&corrupt).is_err());
    }

    fn raw(line: usize, text: &str) -> serde_json::Value {
        serde_json::json!({
            "line": line,
            "session_id": "s",
            "uri": "u",
            "raw": text
        })
    }

    #[test]
    fn normalize_maps_pi_record_shapes_to_spec_seven_event_types() {
        let adapter = PiAdapter::new("/unused");

        let ev = adapter
            .normalize(&raw(
                0,
                r#"{"type":"message","message":{"role":"user","content":"hi","timestamp":"2026-09-08T18:00:00Z"}}"#,
            ))
            .unwrap();
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].event_type, EventType::UserMessage);
        assert_eq!(ev[0].event_id, "s:0");
        assert_eq!(ev[0].timestamp, 1_788_890_400.0);

        let ev = adapter
            .normalize(&raw(
                1,
                r#"{"type":"message","message":{"role":"assistant","model":"qwen3","content":[{"type":"toolCall","name":"bash","arguments":{}}],"usage":{"input":7,"output":2},"timestamp":1757316000}}"#,
            ))
            .unwrap();
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].event_type, EventType::ToolCall);
        assert_eq!(ev[0].tool.as_deref(), Some("bash"));
        assert_eq!(
            ev[0].tokens,
            Tokens {
                input: 7,
                output: 2
            }
        );
        assert_eq!(ev[0].model.as_deref(), Some("qwen3"));
        assert_eq!(ev[0].event_id, "s:1:0");

        let ev = adapter
            .normalize(&raw(
                2,
                r#"{"type":"message","message":{"role":"user","content":[{"type":"toolResult","name":"bash","content":"ok"}]}}"#,
            ))
            .unwrap();
        assert_eq!(ev[0].event_type, EventType::ToolResult);
        assert_eq!(ev[0].tool.as_deref(), Some("bash"));

        let ev = adapter
            .normalize(&raw(
                3,
                r#"{"type":"message","message":{"role":"user","content":[{"type":"toolResult","name":"bash","content":"boom","isError":true}]}}"#,
            ))
            .unwrap();
        assert_eq!(ev[0].event_type, EventType::ToolError);

        let ev = adapter
            .normalize(&raw(
                4,
                r#"{"type":"message","message":{"role":"assistant","content":[{"type":"text","text":"done"}]}}"#,
            ))
            .unwrap();
        assert_eq!(ev[0].event_type, EventType::AssistantMessage);

        // A record that is valid JSON but not a message record yields no
        // events and no error (not corruption, just out of scope).
        assert!(adapter
            .normalize(&raw(5, r#"{"type":"state","x":1}"#))
            .unwrap()
            .is_empty());

        // Malformed JSON is the quarantine trigger: a parse error, so
        // ingest writes a quarantine row and continues.
        let err = adapter
            .normalize(&raw(6, r#"{"type":"message","message":{"role""#))
            .unwrap_err();
        assert!(matches!(err, AutospecError::Parse { .. }));
    }

    #[tokio::test]
    async fn raw_payload_text_is_written_locally_not_just_counted() {
        let (root, _path) = write_fixture(Some(MALFORMED_LINE));
        let (pool, admin, db) = open_test_pool().await;
        let adapter = PiAdapter::new(&root);

        ingest(&pool, &adapter, &IngestCursor::default())
            .await
            .unwrap();

        // The raw record text lives in the local database only: the user
        // message payload and the quarantined raw line are both stored.
        let row =
            sqlx::raw_sql("SELECT payload FROM session_events WHERE event_id = 'sess-abc123:0'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(
            row.try_get::<String, _>(0).unwrap().contains("hello pi"),
            "the raw payload text must be stored locally"
        );
        let row = sqlx::raw_sql("SELECT raw FROM insights_quarantine")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(row.try_get::<String, _>(0).unwrap().contains("toolCall"));

        close_test_pool(pool, &admin, &db).await;
    }

    #[tokio::test]
    async fn empty_root_ingests_a_clean_zero_report() {
        let n = next_counter();
        let pid = std::process::id();
        let root = std::env::temp_dir().join(format!("autospec-pi-empty-{pid}-{n}"));
        std::fs::create_dir_all(&root).unwrap();
        let (pool, admin, db) = open_test_pool().await;

        let report = ingest(&pool, &PiAdapter::new(&root), &IngestCursor::default())
            .await
            .unwrap();
        assert_eq!(report, IngestReport::default());

        close_test_pool(pool, &admin, &db).await;
    }
}
