//! Git, pull-request and CI correlation for insight sessions.
//!
//! Spec: `docs/specs/2026-09-08-continuous-improvement-engine.md` —
//! §34 (storage), §47 Phase 1 (git + CI correlation), §50 (failure
//! handling: "git correlation unavailable -> retain unmatched session").
//!
//! `correlate_session` links ONE session to the commits, pull requests
//! and CI runs it produced. It is read-only with respect to git and
//! GitHub: it reads the normalized `sessions` / `session_events` rows
//! written by ingestion (#3826 / #3827) and upserts into the
//! `git_events` / `ci_events` tables. Unmatched sessions are retained
//! with `match_state = 'unmatched'`, never dropped, and an unavailable
//! source yields a PARTIAL [`Correlation`] with `degraded = true` —
//! never an `Err`.

use std::collections::BTreeMap;

use sqlx::{AnyPool, Executor, Row};

use crate::error::AutospecError;

/// How a link was resolved; `matched_on` values stored in
/// `git_events` / `ci_events`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchBasis {
    /// The event payload carried the session's `work_item_id`.
    WorkItem,
    /// The event payload's branch equals the session's branch.
    Branch,
    /// The commit message carries an `Autospec-Session: <session_id>`
    /// trailer.
    CommitTrailer,
}

impl MatchBasis {
    pub fn as_str(self) -> &'static str {
        match self {
            MatchBasis::WorkItem => "work_item",
            MatchBasis::Branch => "branch",
            MatchBasis::CommitTrailer => "commit_trailer",
        }
    }
}

/// One resolved commit belonging to the session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitLink {
    pub sha: String,
    pub branch: Option<String>,
    pub matched_on: MatchBasis,
}

/// One resolved pull request belonging to the session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequestLink {
    pub number: i64,
    pub branch: Option<String>,
    pub matched_on: MatchBasis,
}

/// One CI run belonging to the session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CiRunLink {
    pub run_id: String,
    pub status: Option<String>,
    pub duration_seconds: Option<i64>,
}

/// The git / PR / CI correlation for one session.
///
/// `degraded` is true when a source (`sessions`, `session_events`, or one
/// of the sink tables) was unavailable and the result is PARTIAL; the
/// session is still retained per spec §50.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Correlation {
    pub session_id: String,
    pub commits: Vec<CommitLink>,
    pub pull_requests: Vec<PullRequestLink>,
    pub ci_runs: Vec<CiRunLink>,
    pub degraded: bool,
}

/// Correlation-relevant attributes of the session row.
#[derive(Debug, Default)]
struct SessionRef {
    work_item_id: Option<String>,
    branch: Option<String>,
}

/// One normalized session event (shape from #3826).
struct SessionEvent {
    event_type: String,
    timestamp: String,
    payload: serde_json::Value,
}

/// One CI run accumulated from its `ci_started` / `ci_finished` events.
#[derive(Default)]
struct CiRun {
    status: Option<String>,
    started_at: Option<String>,
    finished_at: Option<String>,
    matched_on: Option<MatchBasis>,
}

/// Link one session to its commits, pull requests and CI runs, upserting
/// into `git_events` and `ci_events` as it goes.
///
/// Match order (most to least specific): `work_item_id`, branch name,
/// commit trailer. Every resolved and unresolved event is persisted:
/// unresolved events and event-free sessions are stored with
/// `match_state = 'unmatched'` and are never dropped (§50: git
/// correlation unavailable -> retain unmatched session). A source error
/// (missing table, unreadable row) degrades the result — `degraded`
/// becomes true and the call still returns `Ok` with what was resolved.
///
/// The only `Err` is a structurally invalid request: an empty
/// `session_id`.
#[allow(clippy::too_many_arguments)]
pub async fn correlate_session(
    pool: &AnyPool,
    session_id: &str,
) -> Result<Correlation, AutospecError> {
    if session_id.trim().is_empty() {
        return Err(AutospecError::validation(
            "session_id must be a non-empty string",
        ));
    }

    let mut correlation = Correlation {
        session_id: session_id.to_string(),
        ..Default::default()
    };

    let session = match read_session(pool, session_id).await {
        Ok(session) => session,
        Err(_) => {
            correlation.degraded = true;
            SessionRef::default()
        }
    };
    let events = match read_events(pool, session_id).await {
        Ok(events) => events,
        Err(_) => {
            correlation.degraded = true;
            Vec::new()
        }
    };

    correlate_commits(&mut correlation, pool, session_id, &session, &events).await;
    correlate_pull_requests(&mut correlation, &session, &events);
    correlate_ci(&mut correlation, pool, session_id, &session, &events).await;

    Ok(correlation)
}

async fn read_session(pool: &AnyPool, session_id: &str) -> Result<SessionRef, AutospecError> {
    let mut conn = pool
        .acquire()
        .await
        .map_err(|error| AutospecError::state("insights.sessions", error.to_string()))?;
    let row = sqlx::query("SELECT work_item_id, branch FROM sessions WHERE session_id = ?")
        .bind(session_id)
        .fetch_optional(&mut *conn)
        .await
        .map_err(|error| AutospecError::state("insights.sessions", error.to_string()))?;

    let Some(row) = row else {
        return Ok(SessionRef::default());
    };
    Ok(SessionRef {
        work_item_id: row.try_get(0).unwrap_or(None),
        branch: row.try_get(1).unwrap_or(None),
    })
}

async fn read_events(pool: &AnyPool, session_id: &str) -> Result<Vec<SessionEvent>, AutospecError> {
    let mut conn = pool
        .acquire()
        .await
        .map_err(|error| AutospecError::state("insights.session_events", error.to_string()))?;
    let rows = sqlx::query(
        "SELECT event_type, timestamp, payload FROM session_events \
         WHERE session_id = ? ORDER BY timestamp ASC, event_id ASC",
    )
    .bind(session_id)
    .fetch_all(&mut *conn)
    .await
    .map_err(|error| AutospecError::state("insights.session_events", error.to_string()))?;

    let mut events = Vec::with_capacity(rows.len());
    for row in rows {
        let (event_type, timestamp, payload_text) = match (
            row.try_get::<String, _>(0),
            row.try_get::<String, _>(1),
            row.try_get::<String, _>(2),
        ) {
            (Ok(event_type), Ok(timestamp), Ok(payload_text)) => {
                (event_type, timestamp, payload_text)
            }
            (Err(error), _, _) | (_, Err(error), _) | (_, _, Err(error)) => {
                return Err(AutospecError::state(
                    "insights.session_events",
                    error.to_string(),
                ))
            }
        };
        // A malformed payload degrades THAT event, not the call: it is
        // treated as an empty object and therefore matches no key.
        let payload = serde_json::from_str(&payload_text)
            .unwrap_or_else(|_| serde_json::Value::Object(Default::default()));
        events.push(SessionEvent {
            event_type,
            timestamp,
            payload,
        });
    }
    Ok(events)
}

async fn correlate_commits(
    correlation: &mut Correlation,
    pool: &AnyPool,
    session_id: &str,
    session: &SessionRef,
    events: &[SessionEvent],
) {
    let mut saw_git_event = false;
    for event in events {
        if event.event_type != "git_commit" {
            continue;
        }
        saw_git_event = true;
        let payload = &event.payload;
        let sha = payload
            .get("sha")
            .or_else(|| payload.get("commit_sha"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_string();
        let branch = payload
            .get("branch")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string);
        let basis = match_basis(session, payload, session_id);
        let (match_state, basis) = match basis {
            Some(basis) => ("matched", Some(basis)),
            None => ("unmatched", None),
        };
        if let Some(basis) = basis {
            correlation.commits.push(CommitLink {
                sha: sha.clone(),
                branch: branch.clone(),
                matched_on: basis,
            });
        }
        // Retention: even an unresolvable commit is stored, so evidence
        // is never dropped.
        if upsert_git_event(
            pool,
            session_id,
            &sha,
            branch.as_deref(),
            match_state,
            basis,
            &event.timestamp,
        )
        .await
        .is_err()
        {
            correlation.degraded = true;
        }
    }

    // Retention: a session with no git evidence at all (or whose event
    // source was unavailable) is stored as one unmatched row.
    if !saw_git_event {
        if upsert_git_event(pool, session_id, "", None, "unmatched", None, "")
            .await
            .is_err()
        {
            correlation.degraded = true;
        }
    }
}

fn correlate_pull_requests(
    correlation: &mut Correlation,
    session: &SessionRef,
    events: &[SessionEvent],
) {
    for event in events {
        if event.event_type != "pull_request_opened" {
            continue;
        }
        let payload = &event.payload;
        let Some(number) = payload.get("number").and_then(|value| {
            value
                .as_i64()
                .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
        }) else {
            continue;
        };
        // A PR payload has no message, so only the work-item and branch
        // keys can resolve it (pass a sentinel session id for the trailer
        // check, which can never match).
        let Some(basis) = match_basis(session, payload, "\0") else {
            continue;
        };
        correlation.pull_requests.push(PullRequestLink {
            number,
            branch: payload
                .get("head_branch")
                .or_else(|| payload.get("branch"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_string),
            matched_on: basis,
        });
    }
}

async fn correlate_ci(
    correlation: &mut Correlation,
    pool: &AnyPool,
    session_id: &str,
    session: &SessionRef,
    events: &[SessionEvent],
) {
    let mut runs: BTreeMap<String, CiRun> = BTreeMap::new();
    let mut saw_ci_event = false;

    for event in events {
        let is_start = event.event_type == "ci_started";
        let is_finish = event.event_type == "ci_finished";
        if !is_start && !is_finish {
            continue;
        }
        saw_ci_event = true;
        let Some(run_id) = ci_run_id(&event.payload) else {
            continue;
        };
        let run = runs.entry(run_id).or_default();
        if run.matched_on.is_none() {
            run.matched_on = match_basis(session, &event.payload, session_id);
        }
        if is_start {
            run.started_at = Some(event.timestamp.clone());
        } else {
            run.finished_at = Some(event.timestamp.clone());
            run.status = event
                .payload
                .get("status")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string);
        }
    }

    for (run_id, run) in runs {
        let duration = match (run.started_at.as_deref(), run.finished_at.as_deref()) {
            (Some(started), Some(finished)) => rfc3339_epoch_seconds(started)
                .zip(rfc3339_epoch_seconds(finished))
                .and_then(|(start, end)| end.checked_sub(start).filter(|delta| *delta >= 0)),
            _ => None,
        };
        let (match_state, basis) = match run.matched_on {
            Some(basis) => ("matched", Some(basis)),
            None => ("unmatched", None),
        };
        let linked_at = run
            .finished_at
            .clone()
            .or_else(|| run.started_at.clone())
            .unwrap_or_default();
        if upsert_ci_event(
            pool,
            session_id,
            &run_id,
            run.status.as_deref(),
            duration,
            match_state,
            basis,
            &linked_at,
        )
        .await
        .is_err()
        {
            correlation.degraded = true;
        }
        correlation.ci_runs.push(CiRunLink {
            run_id,
            status: run.status,
            duration_seconds: duration,
        });
    }

    // Retention: a session with no CI evidence at all is stored as one
    // unmatched row.
    if !saw_ci_event {
        if upsert_ci_event(pool, session_id, "", None, None, "unmatched", None, "")
            .await
            .is_err()
        {
            correlation.degraded = true;
        }
    }
}

/// Resolve the match basis for one event payload, trying the keys in
/// order: `work_item_id`, branch name, commit trailer.
fn match_basis(
    session: &SessionRef,
    payload: &serde_json::Value,
    session_id: &str,
) -> Option<MatchBasis> {
    if let Some(work_item) = session.work_item_id.as_deref() {
        if payload
            .get("work_item_id")
            .and_then(serde_json::Value::as_str)
            == Some(work_item)
        {
            return Some(MatchBasis::WorkItem);
        }
    }
    if let Some(branch) = session.branch.as_deref() {
        let event_branch = payload
            .get("branch")
            .or_else(|| payload.get("head_branch"))
            .and_then(serde_json::Value::as_str);
        if event_branch == Some(branch) {
            return Some(MatchBasis::Branch);
        }
    }
    let message = payload
        .get("message")
        .or_else(|| payload.get("trailers"))
        .and_then(serde_json::Value::as_str);
    if message
        .map(|text| commit_trailer_session(text) == Some(session_id))
        .unwrap_or(false)
    {
        return Some(MatchBasis::CommitTrailer);
    }
    None
}

/// Value of the `Autospec-Session:` trailer in a commit message, if any.
fn commit_trailer_session(message: &str) -> Option<&str> {
    message.lines().find_map(|line| {
        let line = line.trim();
        let (key, value) = line.split_once(':')?;
        if key.trim().eq_ignore_ascii_case("autospec-session") {
            Some(value.trim())
        } else {
            None
        }
    })
}

fn ci_run_id(payload: &serde_json::Value) -> Option<String> {
    payload
        .get("run_id")
        .or_else(|| payload.get("id"))
        .and_then(|value| value.as_str().map(str::to_string))
}

#[allow(clippy::too_many_arguments)]
async fn upsert_git_event(
    pool: &AnyPool,
    session_id: &str,
    commit_sha: &str,
    branch: Option<&str>,
    match_state: &str,
    matched_on: Option<MatchBasis>,
    linked_at: &str,
) -> Result<(), AutospecError> {
    let query = sqlx::query(
        "INSERT INTO git_events \
         (session_id, commit_sha, branch, match_state, matched_on, source, linked_at) \
         VALUES (?, ?, ?, ?, ?, 'session_events', ?) \
         ON CONFLICT (session_id, commit_sha) DO UPDATE SET \
         branch = excluded.branch, \
         match_state = excluded.match_state, \
         matched_on = excluded.matched_on, \
         linked_at = excluded.linked_at",
    )
    .bind(session_id)
    .bind(commit_sha)
    .bind(branch)
    .bind(match_state)
    .bind(matched_on.map(MatchBasis::as_str))
    .bind(linked_at);
    let mut conn = pool
        .acquire()
        .await
        .map_err(|error| AutospecError::state("insights.git_events", error.to_string()))?;
    (&mut *conn)
        .execute(query)
        .await
        .map_err(|error| AutospecError::state("insights.git_events", error.to_string()))?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn upsert_ci_event(
    pool: &AnyPool,
    session_id: &str,
    run_id: &str,
    status: Option<&str>,
    duration_seconds: Option<i64>,
    match_state: &str,
    matched_on: Option<MatchBasis>,
    linked_at: &str,
) -> Result<(), AutospecError> {
    let query = sqlx::query(
        "INSERT INTO ci_events \
         (session_id, run_id, status, duration_seconds, match_state, matched_on, source, linked_at) \
         VALUES (?, ?, ?, ?, ?, ?, 'session_events', ?) \
         ON CONFLICT (session_id, run_id) DO UPDATE SET \
         status = excluded.status, \
         duration_seconds = excluded.duration_seconds, \
         match_state = excluded.match_state, \
         matched_on = excluded.matched_on, \
         linked_at = excluded.linked_at",
    )
    .bind(session_id)
    .bind(run_id)
    .bind(status)
    .bind(duration_seconds)
    .bind(match_state)
    .bind(matched_on.map(MatchBasis::as_str))
    .bind(linked_at);
    let mut conn = pool
        .acquire()
        .await
        .map_err(|error| AutospecError::state("insights.ci_events", error.to_string()))?;
    (&mut *conn)
        .execute(query)
        .await
        .map_err(|error| AutospecError::state("insights.ci_events", error.to_string()))?;
    Ok(())
}

/// Epoch seconds for an RFC3339 timestamp with `Z` or a numeric offset.
/// Returns `None` for anything else (fractions are floored to seconds).
fn rfc3339_epoch_seconds(timestamp: &str) -> Option<i64> {
    let bytes = timestamp.as_bytes();
    let year = parse_u32(bytes, 0, 4)? as i64;
    if bytes.get(4) != Some(&b'-') {
        return None;
    }
    let month = parse_u32(bytes, 5, 2)?;
    if bytes.get(7) != Some(&b'-') {
        return None;
    }
    let day = parse_u32(bytes, 8, 2)?;
    if bytes.get(10) != Some(&b'T') {
        return None;
    }
    let hour = parse_u32(bytes, 11, 2)? as i64;
    if bytes.get(13) != Some(&b':') {
        return None;
    }
    let minute = parse_u32(bytes, 14, 2)? as i64;
    if bytes.get(16) != Some(&b':') {
        return None;
    }
    let second = parse_u32(bytes, 17, 2)? as i64;
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return None;
    }

    let mut index = 19usize;
    if matches!(bytes.get(index), Some(b'.') | Some(b',')) {
        index += 1;
        while matches!(bytes.get(index), Some(byte) if byte.is_ascii_digit()) {
            index += 1;
        }
    }
    let offset_seconds = match bytes.get(index) {
        Some(b'Z') | Some(b'z') => 0,
        Some(sign @ b'+') | Some(sign @ b'-') => {
            let sign = if *sign == b'+' { 1 } else { -1 };
            let offset_hours = parse_u32(bytes, index + 1, 2)? as i64;
            if bytes.get(index + 3) != Some(&b':') {
                return None;
            }
            let offset_minutes = parse_u32(bytes, index + 4, 2)? as i64;
            sign * (offset_hours * 3600 + offset_minutes * 60)
        }
        _ => return None,
    };

    let days = days_from_civil(year, month, day)?;
    days.checked_mul(86400)
        .and_then(|base| base.checked_add(hour * 3600 + minute * 60 + second - offset_seconds))
}

fn parse_u32(bytes: &[u8], start: usize, len: usize) -> Option<u32> {
    let slice = bytes.get(start..start + len)?;
    let mut value = 0u32;
    for byte in slice {
        let digit = (*byte as char).to_digit(10)?;
        value = value.checked_mul(10)?.checked_add(digit as u32)?;
    }
    Some(value)
}

/// Days between 1970-01-01 and `year-month-day` (Howard Hinnant's
/// `days_from_civil`).
fn days_from_civil(year: i64, month: u32, day: u32) -> Option<i64> {
    let month = month as i64;
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let day_of_year =
        (153 * (if month > 2 { month - 3 } else { month + 9 }) + 3) / 5 + day as i64 - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    (era * 146097 + day_of_era - 719468).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::process::Command;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());
    static DB_LOCK: Mutex<()> = Mutex::new(());
    static ID_COUNTER: Mutex<u32> = Mutex::new(0);
    static FILE_COUNTER: Mutex<u32> = Mutex::new(0);

    /// Disposable schema used by the integration tests below. The
    /// production schema is owned by the ingestion epic (#3826 / #3827);
    /// this mirror exists only so correlation can be exercised against a
    /// throwaway database.
    const SCHEMA: &str = "\
CREATE TABLE IF NOT EXISTS sessions (
    session_id TEXT PRIMARY KEY,
    work_item_id TEXT,
    branch TEXT,
    repo TEXT
);
CREATE TABLE IF NOT EXISTS session_events (
    event_id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    event_type TEXT NOT NULL,
    timestamp TEXT NOT NULL,
    payload TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS git_events (
    session_id TEXT NOT NULL,
    commit_sha TEXT NOT NULL,
    branch TEXT,
    match_state TEXT NOT NULL,
    matched_on TEXT,
    source TEXT NOT NULL,
    linked_at TEXT NOT NULL,
    PRIMARY KEY (session_id, commit_sha)
);
CREATE TABLE IF NOT EXISTS ci_events (
    session_id TEXT NOT NULL,
    run_id TEXT NOT NULL,
    status TEXT,
    duration_seconds INTEGER,
    match_state TEXT NOT NULL,
    matched_on TEXT,
    source TEXT NOT NULL,
    linked_at TEXT NOT NULL,
    PRIMARY KEY (session_id, run_id)
);";

    fn unique_id(kind: &str) -> String {
        let mut counter = ID_COUNTER.lock().unwrap();
        *counter += 1;
        format!("{kind}-{}-{}", std::process::id(), *counter)
    }

    /// `AUTOSPEC_TEST_DB_URL` (disposable PostgreSQL 16, e.g. under
    /// Apptainer) when set; otherwise a fresh per-test SQLite file. Both
    /// are real databases; nothing here fakes a query.
    async fn test_pool() -> AnyPool {
        let _db = DB_LOCK.lock().unwrap();
        let _env = ENV_LOCK.lock().unwrap();
        let url = match std::env::var("AUTOSPEC_TEST_DB_URL") {
            Ok(value) if !value.trim().is_empty() => value,
            _ => {
                let mut counter = FILE_COUNTER.lock().unwrap();
                *counter += 1;
                let dir =
                    std::env::temp_dir().join(format!("autospec-correlate-{}", std::process::id()));
                std::fs::create_dir_all(&dir).unwrap();
                format!("sqlite://{}/correlate-{}.db", dir.display(), *counter)
            }
        };
        crate::resources::db::open_shared_db(&url)
            .await
            .expect("test database must open")
    }

    async fn apply_schema(pool: &AnyPool) {
        for statement in SCHEMA.split(';') {
            let statement = statement.trim();
            if !statement.is_empty() {
                sqlx::query(statement).execute(pool).await.unwrap();
            }
        }
    }

    async fn insert_session(
        pool: &AnyPool,
        session_id: &str,
        work_item_id: Option<&str>,
        branch: Option<&str>,
    ) {
        sqlx::query(
            "INSERT INTO sessions (session_id, work_item_id, branch, repo) \
             VALUES (?, ?, ?, 'fixture/repo')",
        )
        .bind(session_id)
        .bind(work_item_id)
        .bind(branch)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn insert_event(
        pool: &AnyPool,
        event_id: &str,
        session_id: &str,
        event_type: &str,
        timestamp: &str,
        payload: &str,
    ) {
        sqlx::query(
            "INSERT INTO session_events \
             (event_id, session_id, event_type, timestamp, payload) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(event_id)
        .bind(session_id)
        .bind(event_type)
        .bind(timestamp)
        .bind(payload)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn git_event_rows(
        pool: &AnyPool,
        session_id: &str,
    ) -> Vec<(String, String, Option<String>)> {
        let rows = sqlx::query(
            "SELECT commit_sha, match_state, matched_on FROM git_events \
             WHERE session_id = ? ORDER BY commit_sha",
        )
        .bind(session_id)
        .fetch_all(pool)
        .await
        .unwrap();
        rows.iter()
            .map(|row| {
                (
                    row.try_get::<String, _>(0).unwrap(),
                    row.try_get::<String, _>(1).unwrap(),
                    row.try_get::<Option<String>, _>(2).unwrap(),
                )
            })
            .collect()
    }

    async fn ci_event_rows(
        pool: &AnyPool,
        session_id: &str,
    ) -> Vec<(String, Option<String>, Option<i64>, String)> {
        let rows = sqlx::query(
            "SELECT run_id, status, duration_seconds, match_state \
             FROM ci_events WHERE session_id = ? ORDER BY run_id",
        )
        .bind(session_id)
        .fetch_all(pool)
        .await
        .unwrap();
        rows.iter()
            .map(|row| {
                (
                    row.try_get::<String, _>(0).unwrap(),
                    row.try_get::<Option<String>, _>(1).unwrap(),
                    row.try_get::<Option<i64>, _>(2).unwrap(),
                    row.try_get::<String, _>(3).unwrap(),
                )
            })
            .collect()
    }

    fn run_git(dir: &std::path::Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .output()
            .expect("git must be available for the fixture repo");
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).to_string()
    }

    /// Build a throwaway local git repository, make one commit on
    /// `branch`, and return (dir, sha, commit message, HEAD branch).
    fn make_fixture_commit(
        branch: &str,
        trailer_session: Option<&str>,
    ) -> (PathBuf, String, String, String) {
        let mut counter = ID_COUNTER.lock().unwrap();
        *counter += 1;
        let dir = std::env::temp_dir().join(format!(
            "autospec-correlate-git-{}-{}",
            std::process::id(),
            *counter
        ));
        std::fs::create_dir_all(&dir).unwrap();
        run_git(&dir, &["init", "-q", "-b", branch]);
        run_git(&dir, &["config", "user.email", "fixture@example.invalid"]);
        run_git(&dir, &["config", "user.name", "Fixture"]);
        std::fs::write(dir.join("hello.txt"), "fixture\n").unwrap();
        run_git(&dir, &["add", "hello.txt"]);
        let mut commit_args: Vec<String> = vec![
            "commit".into(),
            "-q".into(),
            "-m".into(),
            "Add hello".into(),
        ];
        if let Some(session_id) = trailer_session {
            commit_args.push("--trailer".into());
            commit_args.push(format!("Autospec-Session: {session_id}"));
        }
        let args: Vec<&str> = commit_args.iter().map(String::as_str).collect();
        run_git(&dir, &args);
        let sha = run_git(&dir, &["show", "-s", "--format=%H"])
            .trim()
            .to_string();
        let message = run_git(&dir, &["show", "-s", "--format=%B"]);
        let head = run_git(&dir, &["rev-parse", "--abbrev-ref", "HEAD"])
            .trim()
            .to_string();
        (dir, sha, message, head)
    }

    // ── TDD red test: unmatched retention lands first ──────────────────

    #[tokio::test]
    async fn session_matching_nothing_is_retained_with_unmatched_state() {
        let pool = test_pool().await;
        apply_schema(&pool).await;
        let session_id = unique_id("sess-unmatched");
        insert_session(&pool, &session_id, None, None).await;

        let correlation = correlate_session(&pool, &session_id)
            .await
            .expect("an event-free session must correlate without failing");

        assert!(correlation.commits.is_empty());
        assert!(correlation.pull_requests.is_empty());
        assert!(correlation.ci_runs.is_empty());

        let git = git_event_rows(&pool, &session_id).await;
        assert_eq!(
            git.len(),
            1,
            "unmatched session must be retained, not dropped"
        );
        assert_eq!(git[0].1, "unmatched");

        let ci = ci_event_rows(&pool, &session_id).await;
        assert_eq!(
            ci.len(),
            1,
            "unmatched session must be retained in ci_events too"
        );
        assert_eq!(ci[0].3, "unmatched");
    }

    // ── matching ───────────────────────────────────────────────────────

    #[tokio::test]
    async fn work_item_id_links_commits_and_pull_requests() {
        let pool = test_pool().await;
        apply_schema(&pool).await;
        let session_id = unique_id("sess-workitem");
        insert_session(&pool, &session_id, Some("issue-721"), None).await;
        insert_event(
            &pool,
            &unique_id("evt"),
            &session_id,
            "git_commit",
            "2026-09-08T18:00:00Z",
            r#"{"sha":"abc123","work_item_id":"issue-721"}"#,
        )
        .await;
        insert_event(
            &pool,
            &unique_id("evt"),
            &session_id,
            "pull_request_opened",
            "2026-09-08T18:05:00Z",
            r#"{"number":721,"work_item_id":"issue-721"}"#,
        )
        .await;

        let correlation = correlate_session(&pool, &session_id).await.unwrap();

        assert_eq!(correlation.commits.len(), 1);
        assert_eq!(correlation.commits[0].sha, "abc123");
        assert_eq!(correlation.commits[0].matched_on, MatchBasis::WorkItem);
        assert_eq!(correlation.pull_requests.len(), 1);
        assert_eq!(correlation.pull_requests[0].number, 721);

        let git = git_event_rows(&pool, &session_id).await;
        assert_eq!(git.len(), 1);
        assert_eq!(git[0].0, "abc123");
        assert_eq!(git[0].1, "matched");
        assert_eq!(git[0].2.as_deref(), Some("work_item"));
    }

    #[tokio::test]
    async fn branch_matching_uses_the_fixture_repo_head_branch() {
        let pool = test_pool().await;
        apply_schema(&pool).await;
        let (_dir, sha, _message, head) = make_fixture_commit("feature/fixture-branch", None);
        let session_id = unique_id("sess-branch");
        insert_session(&pool, &session_id, None, Some(&head)).await;
        let payload = serde_json::json!({ "sha": sha, "branch": head }).to_string();
        insert_event(
            &pool,
            &unique_id("evt"),
            &session_id,
            "git_commit",
            "2026-09-08T18:10:00Z",
            &payload,
        )
        .await;

        let correlation = correlate_session(&pool, &session_id).await.unwrap();

        assert_eq!(correlation.commits.len(), 1);
        assert_eq!(correlation.commits[0].sha, sha);
        assert_eq!(correlation.commits[0].matched_on, MatchBasis::Branch);
        let git = git_event_rows(&pool, &session_id).await;
        assert_eq!(git[0].1, "matched");
        assert_eq!(git[0].2.as_deref(), Some("branch"));
    }

    #[tokio::test]
    async fn commit_trailer_matching_uses_a_local_fixture_repo() {
        let pool = test_pool().await;
        apply_schema(&pool).await;
        let session_id = unique_id("sess-trailer");
        // The fixture commit carries `Autospec-Session: <session_id>` in
        // its trailer block; the session has NO work item and NO branch,
        // so only the trailer can resolve the link.
        let (_dir, sha, message, _head) =
            make_fixture_commit("feature/fixture-trailer", Some(&session_id));
        insert_session(&pool, &session_id, None, None).await;
        let payload = serde_json::json!({ "sha": sha, "message": message }).to_string();
        insert_event(
            &pool,
            &unique_id("evt"),
            &session_id,
            "git_commit",
            "2026-09-08T18:15:00Z",
            &payload,
        )
        .await;

        let correlation = correlate_session(&pool, &session_id).await.unwrap();

        assert_eq!(correlation.commits.len(), 1);
        assert_eq!(correlation.commits[0].matched_on, MatchBasis::CommitTrailer);
        let git = git_event_rows(&pool, &session_id).await;
        assert_eq!(git[0].0, sha);
        assert_eq!(git[0].1, "matched");
        assert_eq!(git[0].2.as_deref(), Some("commit_trailer"));
    }

    #[tokio::test]
    async fn commit_event_matching_no_key_is_stored_unmatched() {
        let pool = test_pool().await;
        apply_schema(&pool).await;
        let session_id = unique_id("sess-keyless");
        insert_session(&pool, &session_id, None, None).await;
        insert_event(
            &pool,
            &unique_id("evt"),
            &session_id,
            "git_commit",
            "2026-09-08T18:20:00Z",
            r#"{"sha":"abc789"}"#,
        )
        .await;

        let correlation = correlate_session(&pool, &session_id).await.unwrap();
        assert!(correlation.commits.is_empty(), "no key -> no resolved link");

        let git = git_event_rows(&pool, &session_id).await;
        assert_eq!(
            git.len(),
            1,
            "an unresolvable commit must still be retained"
        );
        assert_eq!(git[0].0, "abc789");
        assert_eq!(git[0].1, "unmatched");
    }

    // ── CI linkage ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn ci_runs_record_status_and_duration() {
        let pool = test_pool().await;
        apply_schema(&pool).await;
        let session_id = unique_id("sess-ci");
        insert_session(&pool, &session_id, Some("issue-721"), None).await;
        insert_event(
            &pool,
            &unique_id("evt"),
            &session_id,
            "ci_started",
            "2026-09-08T18:00:00Z",
            r#"{"run_id":"run-1","work_item_id":"issue-721"}"#,
        )
        .await;
        insert_event(
            &pool,
            &unique_id("evt"),
            &session_id,
            "ci_finished",
            "2026-09-08T18:01:35Z",
            r#"{"run_id":"run-1","status":"success"}"#,
        )
        .await;

        let correlation = correlate_session(&pool, &session_id).await.unwrap();

        assert_eq!(correlation.ci_runs.len(), 1);
        assert_eq!(correlation.ci_runs[0].run_id, "run-1");
        assert_eq!(correlation.ci_runs[0].status.as_deref(), Some("success"));
        assert_eq!(correlation.ci_runs[0].duration_seconds, Some(95));

        let ci = ci_event_rows(&pool, &session_id).await;
        assert_eq!(ci.len(), 1);
        assert_eq!(ci[0].0, "run-1");
        assert_eq!(ci[0].1.as_deref(), Some("success"));
        assert_eq!(ci[0].2, Some(95));
        assert_eq!(ci[0].3, "matched");
    }

    // ── degradation (§50) ──────────────────────────────────────────────

    #[tokio::test]
    async fn unavailable_event_source_yields_partial_result_and_ok() {
        let pool = test_pool().await;
        apply_schema(&pool).await;
        let session_id = unique_id("sess-degraded");
        insert_session(&pool, &session_id, Some("issue-721"), None).await;
        // The event source is unavailable: the table is gone, so no
        // events can be read at all.
        sqlx::query("DROP TABLE session_events")
            .execute(&pool)
            .await
            .unwrap();

        let correlation = correlate_session(&pool, &session_id)
            .await
            .expect("an unavailable source must return a partial Correlation, not a failure");

        assert!(correlation.degraded, "the partial result must be flagged");
        assert!(correlation.commits.is_empty());
        assert!(correlation.ci_runs.is_empty());

        let git = git_event_rows(&pool, &session_id).await;
        assert_eq!(git.len(), 1, "the session must still be retained");
        assert_eq!(git[0].1, "unmatched");
    }

    #[tokio::test]
    async fn correlate_is_idempotent_on_recorrelation() {
        let pool = test_pool().await;
        apply_schema(&pool).await;
        let session_id = unique_id("sess-idem");
        insert_session(&pool, &session_id, Some("issue-721"), None).await;
        insert_event(
            &pool,
            &unique_id("evt"),
            &session_id,
            "git_commit",
            "2026-09-08T18:30:00Z",
            r#"{"sha":"abc123","work_item_id":"issue-721"}"#,
        )
        .await;

        let first = correlate_session(&pool, &session_id).await.unwrap();
        let second = correlate_session(&pool, &session_id).await.unwrap();

        assert_eq!(first, second);
        let git = git_event_rows(&pool, &session_id).await;
        assert_eq!(git.len(), 1, "recorrelation must upsert, not append");
    }

    #[tokio::test]
    async fn empty_session_id_is_rejected() {
        let pool = test_pool().await;
        let error = correlate_session(&pool, "  ").await.unwrap_err();
        assert!(
            matches!(error, AutospecError::Validation { .. }),
            "got: {error:?}"
        );
    }

    // ── RFC3339 helper ─────────────────────────────────────────────────

    #[test]
    fn rfc3339_parses_zulu_offsets_and_fractional_seconds() {
        let zulu = rfc3339_epoch_seconds("2026-09-08T18:00:00Z").unwrap();
        assert_eq!(zulu, 1788890400);
        assert_eq!(
            rfc3339_epoch_seconds("2026-09-08T20:00:00+02:00").unwrap(),
            zulu
        );
        assert_eq!(
            rfc3339_epoch_seconds("2026-09-08T15:00:00-03:00").unwrap(),
            zulu
        );
        assert_eq!(
            rfc3339_epoch_seconds("2026-09-08T18:00:00.25Z").unwrap(),
            zulu
        );
        assert_eq!(
            rfc3339_epoch_seconds("2026-09-08T17:59:59Z").unwrap(),
            zulu - 1
        );
        assert!(rfc3339_epoch_seconds("not-a-timestamp").is_none());
        assert!(rfc3339_epoch_seconds("2026-13-08T18:00:00Z").is_none());
        assert!(rfc3339_epoch_seconds("2026-09-08T18:00:00").is_none());
    }
}
