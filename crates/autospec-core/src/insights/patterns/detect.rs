//! §10 deterministic pattern detection: threshold qualification, recency
//! weighting, §34 storage and §35 evidence writes.
//!
//! Spec: [`docs/specs/2026-09-08-continuous-improvement-engine.md`](../../../../docs/specs/2026-09-08-continuous-improvement-engine.md)
//! §10, §11, §12, §34, §35, §45.
//!
//! [`detect`] scans the normalized telemetry written by ingestion
//! (#3827): `user_interventions` rows (§9 user re-steering) joined to
//! their `sessions` row. Each (repo, model, task, intervention class)
//! group that reaches BOTH §45 thresholds (`recurring_pattern_min_sessions`
//! distinct sessions AND `recurring_pattern_min_occurrences` rows) yields
//! one §11 finding:
//!
//! * a `patterns` row (the pattern identity; re-detect refreshes the
//!   aggregates but never the lifecycle status — see the module docs),
//! * one `finding_evidence` row per supporting intervention (§35),
//! * recency-weighted confidence: each occurrence weighs
//!   `0.5 ** (age_days / recency_half_life_days)`, so recent evidence
//!   dominates (§12: "Recent observations SHOULD normally weigh more
//!   heavily than old observations").
//!
//! Security (§39): only row ids and labels cross this boundary — the
//! intervention `excerpt` column is never read, and no finding field can
//! carry session text.
//!
//! Portability (D10): the store's timestamp columns are `TIMESTAMP`,
//! which the sqlx Any driver cannot map on Postgres, so every timestamp
//! is read through `CAST(... AS TEXT)` and written through
//! `CAST(? AS TEXT)`. The stored shape is the uniform-width ISO-8601 UTC
//! string (`YYYY-MM-DDTHH:MM:SSZ`) the rest of the insights module family
//! uses; PostgreSQL's `CAST(timestamp AS TEXT)` renders the same instant
//! as `YYYY-MM-DD HH:MM:SS` (UTC by store convention) and the parser
//! accepts both. The shapes sort chronologically, so MIN/MAX in SQL is
//! chronological MIN/MAX.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{SystemTime, UNIX_EPOCH};

use sqlx::{AnyPool, Row};

use super::{EstimatedCost, Finding, FindingEvidence, FindingStatus, Severity};
use crate::error::AutospecError;
use crate::insights::config::ThresholdsConfig;

/// §11 `type` value detect assigns to a user-intervention pattern.
pub const PATTERN_KIND: &str = "recurring_user_intervention";
/// §11 `evidence_kind` stored in `finding_evidence` rows.
pub const EVIDENCE_KIND: &str = "user_intervention";
/// Per-occurrence rework estimate behind §11 `estimated_cost.extra_tokens`.
pub const EXTRA_TOKENS_PER_OCCURRENCE: u64 = 12_000;
/// Per-occurrence rework estimate behind §11 `estimated_cost.extra_minutes`.
pub const EXTRA_MINUTES_PER_OCCURRENCE: u64 = 5;
/// Label for a NULL model / work_item in group keys and finding ids.
const UNLABELED: &str = "-";

/// §45 thresholds plus the recency reference time and decay half-life.
#[derive(Debug, Clone, PartialEq)]
pub struct DetectConfig {
    /// §45 `recurring_pattern_min_sessions`.
    pub recurring_pattern_min_sessions: u64,
    /// §45 `recurring_pattern_min_occurrences`.
    pub recurring_pattern_min_occurrences: u64,
    /// Reference time (Unix seconds) that recency is measured against.
    pub now: i64,
    /// Exponential-decay half-life (days) for recency weighting.
    pub recency_half_life_days: f64,
}

impl DetectConfig {
    /// §12 recency weighting with a deterministic 30-day half-life.
    pub const DEFAULT_RECENCY_HALF_LIFE_DAYS: f64 = 30.0;

    /// Build a detect config from the §45 `thresholds` block, using the
    /// current wall clock as the recency reference.
    pub fn from_thresholds(thresholds: &ThresholdsConfig) -> Self {
        Self {
            recurring_pattern_min_sessions: thresholds.recurring_pattern_min_sessions,
            recurring_pattern_min_occurrences: thresholds.recurring_pattern_min_occurrences,
            now: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0),
            recency_half_life_days: Self::DEFAULT_RECENCY_HALF_LIFE_DAYS,
        }
    }

    fn validate(&self) -> Result<(), AutospecError> {
        if self.recency_half_life_days <= 0.0 || !self.recency_half_life_days.is_finite() {
            return Err(AutospecError::validation(
                "detect: recency_half_life_days must be a positive, finite number of days",
            ));
        }
        if self.recurring_pattern_min_sessions == 0 {
            return Err(AutospecError::validation(
                "detect: recurring_pattern_min_sessions must be >= 1",
            ));
        }
        if self.recurring_pattern_min_occurrences == 0 {
            return Err(AutospecError::validation(
                "detect: recurring_pattern_min_occurrences must be >= 1",
            ));
        }
        Ok(())
    }
}

/// Recency weight of one occurrence: 1.0 at `now`, halving every
/// `half_life_days`; future-dated rows are clamped to 1.0.
pub fn recency_weight(age_days: f64, half_life_days: f64) -> f64 {
    if age_days <= 0.0 {
        1.0
    } else {
        0.5_f64.powf(age_days / half_life_days)
    }
}

/// Parse a stored timestamp into Unix seconds. Accepted shapes:
///
/// * `YYYY-MM-DDTHH:MM:SSZ` — the family's canonical ISO-8601 UTC string;
/// * `YYYY-MM-DD HH:MM:SS` — PostgreSQL's `CAST(timestamp AS TEXT)`
///   rendering of the same instant (UTC by store convention).
///
/// Anything else is an error — detect fails closed instead of silently
/// mis-weighting a row.
pub fn parse_iso8601_utc(value: &str) -> Result<i64, AutospecError> {
    let bytes = value.as_bytes();
    let separator_ok = match bytes.len() {
        20 => bytes[10] == b'T' && bytes[19] == b'Z',
        19 => bytes[10] == b' ',
        _ => false,
    };
    if !separator_ok
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[13] != b':'
        || bytes[16] != b':'
    {
        return Err(AutospecError::parse(
            "occurred_at",
            format!("expected YYYY-MM-DDTHH:MM:SSZ (or YYYY-MM-DD HH:MM:SS), got {value:?}"),
        ));
    }
    let year = parse_digits(bytes, 0, 4, value)?;
    let month = parse_digits(bytes, 5, 2, value)?;
    let day = parse_digits(bytes, 8, 2, value)?;
    let hour = parse_digits(bytes, 11, 2, value)?;
    let minute = parse_digits(bytes, 14, 2, value)?;
    let second = parse_digits(bytes, 17, 2, value)?;
    if !(1..=12).contains(&month)
        || !(1..=days_in_month(year, month)).contains(&day)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return Err(AutospecError::parse(
            "occurred_at",
            format!("out-of-range calendar fields in {value:?}"),
        ));
    }
    Ok(days_from_civil(year, month, day) * 86_400 + hour * 3_600 + minute * 60 + second)
}

fn parse_digits(bytes: &[u8], lo: usize, len: usize, value: &str) -> Result<i64, AutospecError> {
    let mut out = 0i64;
    for byte in &bytes[lo..lo + len] {
        if !byte.is_ascii_digit() {
            return Err(AutospecError::parse(
                "occurred_at",
                format!("expected digits in {value:?}"),
            ));
        }
        out = out * 10 + i64::from(byte - b'0');
    }
    Ok(out)
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
        2 => 28,
        _ => 0,
    }
}

/// Days since 1970-01-01 for a proleptic Gregorian civil date
/// (Howard Hinnant's `days_from_civil`).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = if month > 2 { month - 3 } else { month + 9 };
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// One stored user intervention joined to its session row.
struct Occurrence {
    session_id: String,
    seq: i64,
    /// Raw stored timestamp (ISO-8601 UTC).
    occurred_at: String,
    /// Parsed Unix seconds of `occurred_at`.
    at: i64,
}

/// Deterministic §10 group key: (repo, model, task, intervention class).
/// NULL model / work_item are bucketed under [`UNLABELED`].
#[derive(Clone, Eq, Ord, PartialEq, PartialOrd)]
struct Key {
    repo: String,
    model: String,
    task: String,
    signal: String,
}

/// All occurrences of one (repo, model, task, signal) group.
struct Group {
    key: Key,
    /// Distinct sessions the group spans.
    sessions: BTreeSet<String>,
    occurrences: Vec<Occurrence>,
}

const OCCURRENCE_SQL: &str = "\
SELECT s.repo, s.model, s.work_item_id, ui.session_id, ui.seq, \
       ui.intervention_type, CAST(ui.occurred_at AS TEXT) AS occurred_at
FROM user_interventions ui
JOIN sessions s ON s.id = ui.session_id
ORDER BY ui.session_id, ui.seq";

/// Detect recurring §10 patterns in the stored telemetry, qualify them
/// against the §45 thresholds and persist each finding (a `patterns` row
/// plus one `finding_evidence` row per supporting intervention).
///
/// Findings are sorted by confidence (newest activity first), tie-broken
/// by `finding_id`, so the ordering is deterministic for a given store
/// state and `cfg.now`.
pub async fn detect(pool: &AnyPool, cfg: &DetectConfig) -> Result<Vec<Finding>, AutospecError> {
    cfg.validate()?;
    let groups = load_groups(pool).await?;
    let mut findings = Vec::new();
    for group in groups.values() {
        if !qualifies(group, cfg) {
            continue;
        }
        let mut finding = build_finding(group, cfg);
        persist_finding(pool, &mut finding).await?;
        findings.push(finding);
    }
    findings.sort_by(|left, right| {
        right
            .confidence
            .partial_cmp(&left.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.finding_id.cmp(&right.finding_id))
    });
    Ok(findings)
}

/// Read every user intervention joined to its session row and group it by
/// (repo, model, task, intervention class). A row whose `occurred_at` is
/// not the store's ISO-8601 UTC shape is an error (fail closed).
async fn load_groups(pool: &AnyPool) -> Result<BTreeMap<Key, Group>, AutospecError> {
    let rows = sqlx::query(OCCURRENCE_SQL)
        .fetch_all(pool)
        .await
        .map_err(|error| AutospecError::state("user_interventions", error.to_string()))?;
    let mut groups: BTreeMap<Key, Group> = BTreeMap::new();
    for row in rows {
        let (repo, model, work_item, session_id, seq, signal, occurred_at) =
            read_occurrence_row(&row)?;
        let at = parse_iso8601_utc(&occurred_at).map_err(|_| {
            AutospecError::parse(
                "user_interventions.occurred_at",
                format!(
                    "unparseable occurred_at {occurred_at:?} in session {session_id:?}; \
                     expected YYYY-MM-DDTHH:MM:SSZ or YYYY-MM-DD HH:MM:SS"
                ),
            )
        })?;
        let key = Key {
            repo,
            model: model.as_deref().unwrap_or(UNLABELED).to_string(),
            task: work_item.as_deref().unwrap_or(UNLABELED).to_string(),
            signal,
        };
        let group = groups.entry(key.clone()).or_insert_with(|| Group {
            key: key.clone(),
            sessions: BTreeSet::new(),
            occurrences: Vec::new(),
        });
        group.sessions.insert(session_id.clone());
        group.occurrences.push(Occurrence {
            session_id,
            seq,
            occurred_at,
            at,
        });
    }
    Ok(groups)
}

fn read_occurrence_row(
    row: &sqlx::any::AnyRow,
) -> Result<
    (
        String,
        Option<String>,
        Option<String>,
        String,
        i64,
        String,
        String,
    ),
    AutospecError,
> {
    let map = |error: sqlx::Error| AutospecError::state("user_interventions", error.to_string());
    Ok((
        row.try_get(0).map_err(map)?,
        row.try_get(1).map_err(map)?,
        row.try_get(2).map_err(map)?,
        row.try_get(3).map_err(map)?,
        row.try_get(4).map_err(map)?,
        row.try_get(5).map_err(map)?,
        row.try_get(6).map_err(map)?,
    ))
}

/// §45 qualification: BOTH thresholds must be reached.
fn qualifies(group: &Group, cfg: &DetectConfig) -> bool {
    group.sessions.len() as u64 >= cfg.recurring_pattern_min_sessions
        && group.occurrences.len() as u64 >= cfg.recurring_pattern_min_occurrences
}

/// Assemble the §11 finding for one qualifying group. Confidence is the
/// mean recency weight of the group's occurrences, in `0..=1`: 1.0 means
/// every supporting row is from `now`, and the same group decays toward
/// 0 as its evidence ages (§12: recent observations weigh more heavily).
/// The §45 thresholds are the qualification gate; confidence measures how
/// fresh the qualified group's evidence is.
fn build_finding(group: &Group, cfg: &DetectConfig) -> Finding {
    let key = &group.key;
    let occurrences = group.occurrences.len() as u64;
    let mut first_seen = String::new();
    let mut last_seen = String::new();
    let mut weighted = 0.0f64;
    for occurrence in &group.occurrences {
        let seen = &occurrence.occurred_at;
        if first_seen.is_empty() || seen < &first_seen {
            first_seen = seen.clone();
        }
        if seen > &last_seen {
            last_seen = seen.clone();
        }
        let age_days = (cfg.now - occurrence.at) as f64 / 86_400.0;
        weighted += recency_weight(age_days, cfg.recency_half_life_days);
    }
    let confidence = weighted / occurrences as f64;
    Finding {
        finding_id: format!(
            "finding|{}|{}|{}|{}",
            key.repo, key.model, key.task, key.signal
        ),
        kind: PATTERN_KIND.to_string(),
        title: format!("Recurring user intervention: {}", key.signal),
        status: FindingStatus::Candidate,
        first_seen,
        last_seen,
        occurrences,
        sessions: group.sessions.iter().cloned().collect(),
        repositories: vec![key.repo.clone()],
        models: BTreeMap::from([(key.model.clone(), occurrences)]),
        confidence,
        severity: Severity::from_occurrences(occurrences),
        estimated_cost: EstimatedCost {
            extra_tokens: occurrences * EXTRA_TOKENS_PER_OCCURRENCE,
            extra_minutes: occurrences * EXTRA_MINUTES_PER_OCCURRENCE,
        },
        evidence: group
            .occurrences
            .iter()
            .map(|occurrence| FindingEvidence {
                session_id: occurrence.session_id.clone(),
                event_ref: Some(format!("user_intervention#{}", occurrence.seq)),
            })
            .collect(),
        candidate_remediations: vec![],
    }
}

const PATTERN_UPSERT_SQL: &str = "\
INSERT INTO patterns (id, kind, title, description, session_count, \
                        first_seen_at, last_seen_at, status) \
VALUES (?, ?, ?, ?, ?, CAST(? AS TEXT), CAST(? AS TEXT), 'candidate') \
ON CONFLICT (id) DO UPDATE SET \
    kind = excluded.kind, \
    title = excluded.title, \
    description = excluded.description, \
    session_count = excluded.session_count, \
    first_seen_at = MIN(patterns.first_seen_at, excluded.first_seen_at), \
    last_seen_at = MAX(patterns.last_seen_at, excluded.last_seen_at)";

const EVIDENCE_UPSERT_SQL: &str = "\
INSERT INTO finding_evidence (id, finding_id, evidence_kind, session_id, event_ref) \
VALUES (?, ?, ?, ?, ?) \
ON CONFLICT (id) DO UPDATE SET \
    finding_id = excluded.finding_id, \
    evidence_kind = excluded.evidence_kind, \
    session_id = excluded.session_id, \
    event_ref = excluded.event_ref";

/// Persist one finding: upsert the `patterns` row (never touching
/// `status`), read the lifecycle status back, then write one
/// `finding_evidence` row per supporting intervention.
async fn persist_finding(pool: &AnyPool, finding: &mut Finding) -> Result<(), AutospecError> {
    upsert_pattern(pool, finding).await?;
    finding.status = read_pattern_status(pool, &finding.finding_id).await?;
    upsert_evidence(pool, finding).await?;
    Ok(())
}

async fn upsert_pattern(pool: &AnyPool, finding: &Finding) -> Result<(), AutospecError> {
    let description = format!(
        "{} occurrences across {} session(s)",
        finding.occurrences,
        finding.sessions.len()
    );
    sqlx::query(PATTERN_UPSERT_SQL)
        .bind(&finding.finding_id)
        .bind(&finding.kind)
        .bind(&finding.title)
        .bind(&description)
        .bind(i64::try_from(finding.sessions.len()).unwrap_or(i64::MAX))
        .bind(&finding.first_seen)
        .bind(&finding.last_seen)
        .execute(pool)
        .await
        .map_err(|error| AutospecError::io("upsert pattern", "patterns", error))?;
    Ok(())
}

/// Read the lifecycle status the store already holds for this pattern. A
/// re-detect must observe the advanced state, not reset it (§12 guard).
async fn read_pattern_status(
    pool: &AnyPool,
    finding_id: &str,
) -> Result<FindingStatus, AutospecError> {
    let row = sqlx::query("SELECT status FROM patterns WHERE id = ?")
        .bind(finding_id)
        .fetch_optional(pool)
        .await
        .map_err(|error| AutospecError::state("patterns", error.to_string()))?
        .ok_or_else(|| {
            AutospecError::state(
                "patterns",
                format!("no pattern row for finding {finding_id}"),
            )
        })?;
    let status: String = row
        .try_get(0)
        .map_err(|error| AutospecError::state("patterns", error.to_string()))?;
    status.parse()
}

async fn upsert_evidence(pool: &AnyPool, finding: &Finding) -> Result<(), AutospecError> {
    for (index, evidence) in finding.evidence.iter().enumerate() {
        let id = format!(
            "finding-evidence|{}|{}|{}",
            finding.finding_id,
            evidence.session_id,
            evidence
                .event_ref
                .clone()
                .unwrap_or_else(|| index.to_string())
        );
        sqlx::query(EVIDENCE_UPSERT_SQL)
            .bind(&id)
            .bind(&finding.finding_id)
            .bind(EVIDENCE_KIND)
            .bind(&evidence.session_id)
            .bind(&evidence.event_ref)
            .execute(pool)
            .await
            .map_err(|error| {
                AutospecError::io("upsert finding evidence", "finding_evidence", error)
            })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::insights::patterns::transition;
    use crate::resources::db;
    use std::path::PathBuf;
    use std::sync::{Mutex, MutexGuard};

    /// Serializes the DB-touching tests: `AUTOSPEC_TEST_DB_URL` names a
    /// shared disposable database whose per-test counts the assertions
    /// below depend on.
    static TEST_LOCK: Mutex<()> = Mutex::new(());
    static DIR_COUNTER: Mutex<u32> = Mutex::new(0);

    fn fresh_tmp_dir() -> PathBuf {
        let mut counter = DIR_COUNTER.lock().unwrap();
        *counter += 1;
        let dir = std::env::temp_dir().join(format!(
            "autospec-patterns-test-{}-{}",
            std::process::id(),
            *counter
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Mirror of `migrations/insights/3000001_init.sql` (sessions +
    /// user_interventions + patterns + finding_evidence); the FK to
    /// sessions is omitted, like the `correlate::tests` mirror does.
    const TEST_SCHEMA: &str = "
        CREATE TABLE IF NOT EXISTS sessions (
            id TEXT PRIMARY KEY,
            repo TEXT NOT NULL,
            work_item_id TEXT,
            harness TEXT,
            model TEXT,
            status TEXT NOT NULL,
            started_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
            ended_at TIMESTAMP,
            created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
        );
        CREATE TABLE IF NOT EXISTS user_interventions (
            session_id TEXT NOT NULL,
            seq INTEGER NOT NULL,
            intervention_type TEXT NOT NULL,
            occurred_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
            excerpt TEXT,
            PRIMARY KEY (session_id, seq)
        );
        CREATE TABLE IF NOT EXISTS patterns (
            id TEXT PRIMARY KEY,
            kind TEXT NOT NULL,
            title TEXT NOT NULL,
            description TEXT,
            session_count INTEGER NOT NULL DEFAULT 0,
            first_seen_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
            last_seen_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
            status TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS finding_evidence (
            id TEXT PRIMARY KEY,
            finding_id TEXT NOT NULL,
            evidence_kind TEXT NOT NULL,
            session_id TEXT NOT NULL,
            event_ref TEXT,
            message_ref TEXT,
            commit_sha TEXT,
            pr_ref TEXT,
            ci_run_ref TEXT,
            created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
        )
    ";

    /// Holds the test lock for the lifetime of the returned guard.
    fn lock_tests() -> MutexGuard<'static, ()> {
        TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// `AUTOSPEC_TEST_DB_URL` (a shared disposable database) when set; a
    /// fresh per-test SQLite file otherwise. Both are real databases; the
    /// tables are truncated after schema creation so counts start clean.
    async fn test_pool() -> AnyPool {
        let _lock = lock_tests();
        let url = match std::env::var("AUTOSPEC_TEST_DB_URL") {
            Ok(value) if !value.trim().is_empty() => value,
            _ => {
                let dir = fresh_tmp_dir();
                format!("sqlite://{}/insights-patterns.db", dir.display())
            }
        };
        let pool = db::open_shared_db(&url).await.unwrap();
        for statement in TEST_SCHEMA.split(';') {
            let statement = statement.trim();
            if !statement.is_empty() {
                sqlx::query(statement).execute(&pool).await.unwrap();
            }
        }
        for table in [
            "finding_evidence",
            "patterns",
            "user_interventions",
            "sessions",
        ] {
            sqlx::query(&format!("DELETE FROM {table}"))
                .execute(&pool)
                .await
                .unwrap();
        }
        pool
    }

    /// Recency reference: 2026-07-20T00:00:00Z — exactly 200 days after
    /// 2026-01-01T00:00:00Z.
    const NOW: &str = "2026-07-20T00:00:00Z";
    const SEVEN_DAYS_OLD: &str = "2026-07-13T00:00:00Z";
    const TWO_HUNDRED_DAYS_OLD: &str = "2026-01-01T00:00:00Z";

    fn cfg_at(now: &str) -> DetectConfig {
        DetectConfig {
            recurring_pattern_min_sessions: 3,
            recurring_pattern_min_occurrences: 5,
            now: parse_iso8601_utc(now).unwrap(),
            recency_half_life_days: DetectConfig::DEFAULT_RECENCY_HALF_LIFE_DAYS,
        }
    }

    async fn seed_session(
        pool: &AnyPool,
        id: &str,
        repo: &str,
        model: &str,
        work_item: Option<&str>,
    ) {
        sqlx::query(
            "INSERT INTO sessions (id, repo, work_item_id, model, status) \
             VALUES (?, ?, ?, ?, 'completed')",
        )
        .bind(id)
        .bind(repo)
        .bind(work_item)
        .bind(model)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn seed_intervention(
        pool: &AnyPool,
        session_id: &str,
        seq: i64,
        intervention_type: &str,
        occurred_at: &str,
    ) {
        sqlx::query(
            "INSERT INTO user_interventions (session_id, seq, intervention_type, occurred_at) \
             VALUES (?, ?, ?, CAST(? AS TEXT))",
        )
        .bind(session_id)
        .bind(seq)
        .bind(intervention_type)
        .bind(occurred_at)
        .execute(pool)
        .await
        .unwrap();
    }

    /// Seeds `sessions_count` sessions, each carrying `per_session`
    /// interventions of `intervention_type` at `occurred_at`.
    async fn seed_cluster(
        pool: &AnyPool,
        prefix: &str,
        repo: &str,
        model: &str,
        work_item: &str,
        intervention_type: &str,
        occurred_at: &str,
        sessions_count: usize,
        per_session: usize,
    ) {
        for i in 1..=sessions_count {
            let session_id = format!("{prefix}-{i}");
            seed_session(pool, &session_id, repo, model, Some(work_item)).await;
            for seq in 0..per_session {
                seed_intervention(
                    pool,
                    &session_id,
                    seq as i64,
                    intervention_type,
                    occurred_at,
                )
                .await;
            }
        }
    }

    async fn count_rows(pool: &AnyPool, table: &str, column: &str, value: &str) -> i64 {
        let sql = format!("SELECT COUNT(*) AS n FROM {table} WHERE {column} = ?");
        let row = sqlx::query(&sql).bind(value).fetch_one(pool).await.unwrap();
        row.get::<i64, _>("n")
    }

    async fn evidence_rows(
        pool: &AnyPool,
        finding_id: &str,
    ) -> Vec<(String, String, String, String)> {
        let rows = sqlx::query(
            "SELECT finding_id, evidence_kind, session_id, event_ref \
             FROM finding_evidence WHERE finding_id = ? ORDER BY id",
        )
        .bind(finding_id)
        .fetch_all(pool)
        .await
        .unwrap();
        rows.iter()
            .map(|row| {
                (
                    row.get::<String, _>(0),
                    row.get::<String, _>(1),
                    row.get::<String, _>(2),
                    row.get::<Option<String>, _>(3).unwrap_or_default(),
                )
            })
            .collect()
    }

    /// Seed the canonical above-threshold cluster: 3 sessions x 2
    /// `replan` interventions = 6 occurrences.
    async fn seed_above_threshold(pool: &AnyPool) {
        seed_cluster(
            pool,
            "pat-above",
            "fixture/repo",
            "qwen3",
            "wi-1",
            "replan",
            SEVEN_DAYS_OLD,
            3,
            2,
        )
        .await;
    }

    // ── parse_iso8601_utc ────────────────────────────────────────────────

    #[test]
    fn parse_iso8601_utc_maps_known_instants() {
        assert_eq!(parse_iso8601_utc("1970-01-01T00:00:00Z").unwrap(), 0);
        assert_eq!(
            parse_iso8601_utc("2026-01-01T00:00:00Z").unwrap(),
            1_767_225_600
        );
        // PostgreSQL's CAST(timestamp AS TEXT) rendering of the same instant.
        assert_eq!(
            parse_iso8601_utc("2026-01-01 00:00:00").unwrap(),
            1_767_225_600
        );
        assert_eq!(
            parse_iso8601_utc("2024-02-29T23:59:59Z").unwrap(),
            1_709_251_199
        );
        assert_eq!(parse_iso8601_utc(NOW).unwrap(), 1_784_505_600);
        assert_eq!(parse_iso8601_utc(SEVEN_DAYS_OLD).unwrap(), 1_783_900_800);
    }

    #[test]
    fn parse_iso8601_utc_rejects_malformed_shapes() {
        for value in [
            "not a timestamp",
            "2026-01-01 00:00:00Z",
            "2026-01-01T00:00:00",
            "2026-01-01T00:00:00z",
            "2026-1-01T00:00:00Z",
            "2026-13-01T00:00:00Z",
            "2026-00-10T00:00:00Z",
            "2026-02-30T00:00:00Z",
            "2026-02-29T00:00:00Z", // 2026 is not a leap year
            "2026-01-01T24:00:00Z",
            "2026-01-01T00:60:00Z",
            "2026-01-01T00:00:60Z",
        ] {
            assert!(
                matches!(parse_iso8601_utc(value), Err(AutospecError::Parse { .. })),
                "{value:?} must be rejected"
            );
        }
        assert_eq!(
            parse_iso8601_utc("2024-02-29T00:00:00Z").unwrap(),
            1_709_164_800
        );
    }

    // ── recency_weight ───────────────────────────────────────────────────

    #[test]
    fn recency_weight_halves_every_half_life() {
        assert_eq!(recency_weight(0.0, 30.0), 1.0);
        assert!((recency_weight(-5.0, 30.0) - 1.0).abs() < 1e-12);
        assert!((recency_weight(30.0, 30.0) - 0.5).abs() < 1e-12);
        assert!((recency_weight(60.0, 30.0) - 0.25).abs() < 1e-12);
        assert!(recency_weight(1000.0, 30.0) < 0.001);
    }

    // ── DetectConfig ─────────────────────────────────────────────────────

    #[test]
    fn from_thresholds_copies_the_section_45_thresholds() {
        let thresholds = ThresholdsConfig {
            recurring_pattern_min_sessions: 4,
            recurring_pattern_min_occurrences: 9,
            proposal_confidence_min: 0.8,
        };
        let cfg = DetectConfig::from_thresholds(&thresholds);
        assert_eq!(cfg.recurring_pattern_min_sessions, 4);
        assert_eq!(cfg.recurring_pattern_min_occurrences, 9);
        assert_eq!(cfg.recency_half_life_days, 30.0);
        assert!(cfg.now > 0);
    }

    #[tokio::test]
    async fn detect_rejects_invalid_config() {
        let pool = test_pool().await;
        let mut cfg = cfg_at(NOW);
        cfg.recency_half_life_days = 0.0;
        assert!(detect(&pool, &cfg).await.is_err());
        let mut cfg = cfg_at(NOW);
        cfg.recency_half_life_days = f64::NAN;
        assert!(detect(&pool, &cfg).await.is_err());
        let mut cfg = cfg_at(NOW);
        cfg.recurring_pattern_min_sessions = 0;
        assert!(detect(&pool, &cfg).await.is_err());
        let mut cfg = cfg_at(NOW);
        cfg.recurring_pattern_min_occurrences = 0;
        assert!(detect(&pool, &cfg).await.is_err());
    }

    // ── threshold qualification ──────────────────────────────────────────

    #[tokio::test]
    async fn seeded_rows_below_threshold_yield_no_finding() {
        let pool = test_pool().await;
        // Group A: 6 occurrences but only 2 sessions (< min_sessions 3).
        seed_cluster(
            &pool,
            "pat-below-a",
            "fixture/repo",
            "qwen3",
            "wi-1",
            "replan",
            SEVEN_DAYS_OLD,
            2,
            3,
        )
        .await;
        // Group B: 3 sessions but only 4 occurrences (< min_occurrences 5).
        seed_cluster(
            &pool,
            "pat-below-b",
            "fixture/repo",
            "qwen3",
            "wi-2",
            "replan",
            SEVEN_DAYS_OLD,
            3,
            1,
        )
        .await;
        let findings = detect(&pool, &cfg_at(NOW)).await.unwrap();
        assert_eq!(findings.len(), 0);
        assert_eq!(
            count_rows(&pool, "patterns", "kind", PATTERN_KIND).await,
            0,
            "below-threshold groups must persist nothing"
        );
        assert_eq!(
            count_rows(&pool, "finding_evidence", "evidence_kind", EVIDENCE_KIND).await,
            0
        );
    }

    #[tokio::test]
    async fn seeded_rows_above_threshold_yield_one_finding() {
        let pool = test_pool().await;
        seed_above_threshold(&pool).await;
        let findings = detect(&pool, &cfg_at(NOW)).await.unwrap();
        assert_eq!(findings.len(), 1);
        let finding = &findings[0];
        assert_eq!(finding.finding_id, "finding|fixture/repo|qwen3|wi-1|replan");
        assert_eq!(finding.kind, PATTERN_KIND);
        assert_eq!(finding.status, FindingStatus::Candidate);
        assert_eq!(finding.occurrences, 6);
        assert_eq!(
            finding.sessions,
            vec![
                "pat-above-1".to_string(),
                "pat-above-2".to_string(),
                "pat-above-3".to_string(),
            ]
        );
        assert_eq!(finding.repositories, vec!["fixture/repo".to_string()]);
        assert_eq!(finding.models.get("qwen3"), Some(&6));
        assert_eq!(finding.first_seen, SEVEN_DAYS_OLD);
        assert_eq!(finding.last_seen, SEVEN_DAYS_OLD);
        assert_eq!(finding.severity, Severity::Medium);
        assert_eq!(
            finding.estimated_cost,
            EstimatedCost {
                extra_tokens: 6 * EXTRA_TOKENS_PER_OCCURRENCE,
                extra_minutes: 6 * EXTRA_MINUTES_PER_OCCURRENCE,
            }
        );
        assert!(finding.confidence > 0.0 && finding.confidence <= 1.0);
        assert!(finding.candidate_remediations.is_empty());
        assert_eq!(finding.evidence.len(), 6);
        // All-fresh cluster: confidence is the mean recency weight,
        // 0.5^(7/30).
        let expected = 0.5_f64.powf(7.0 / 30.0);
        assert!((finding.confidence - expected).abs() < 1e-9);
    }

    // ── §35 evidence preservation ────────────────────────────────────────

    #[tokio::test]
    async fn each_finding_writes_its_supporting_rows_to_finding_evidence() {
        let pool = test_pool().await;
        seed_above_threshold(&pool).await;
        let findings = detect(&pool, &cfg_at(NOW)).await.unwrap();
        let finding = &findings[0];
        let rows = evidence_rows(&pool, &finding.finding_id).await;
        assert!(
            !rows.is_empty(),
            "every finding must write at least one finding_evidence row"
        );
        assert_eq!(rows.len(), 6);
        for (_, kind, session, event_ref) in &rows {
            assert_eq!(kind, EVIDENCE_KIND);
            assert!(
                finding.sessions.iter().any(|s| s == session),
                "evidence session {session} must be listed in finding.sessions"
            );
            assert!(
                event_ref.starts_with("user_intervention#"),
                "event_ref {event_ref} must be a row id, not payload text"
            );
        }
        // Data integrity: the evidence sessions cover exactly the
        // finding's sessions — no supporting session is unnamed.
        let evidence_sessions: BTreeSet<&String> = rows.iter().map(|r| &r.2).collect();
        let finding_sessions: BTreeSet<&String> = finding.sessions.iter().collect();
        assert_eq!(evidence_sessions, finding_sessions);
    }

    // ── recency weighting (ranking) ──────────────────────────────────────

    #[tokio::test]
    async fn recency_weighting_ranks_a_7_day_old_cluster_above_a_200_day_old_one() {
        let pool = test_pool().await;
        // Same (repo, model, task); the intervention class separates the
        // clusters. Both qualify with 3 sessions and 5 occurrences.
        seed_cluster(
            &pool,
            "pat-recent",
            "fixture/repo",
            "qwen3",
            "wi-1",
            "replan",
            SEVEN_DAYS_OLD,
            3,
            2,
        )
        .await;
        // 3 sessions x 2 = 6 >= 5 occurrences.
        seed_cluster(
            &pool,
            "pat-stale",
            "fixture/repo",
            "qwen3",
            "wi-1",
            "interrupt",
            TWO_HUNDRED_DAYS_OLD,
            3,
            2,
        )
        .await;
        let findings = detect(&pool, &cfg_at(NOW)).await.unwrap();
        assert_eq!(findings.len(), 2);
        let recent = findings
            .iter()
            .find(|f| f.finding_id.ends_with("|replan"))
            .unwrap();
        let stale = findings
            .iter()
            .find(|f| f.finding_id.ends_with("|interrupt"))
            .unwrap();
        assert!(
            recent.confidence > stale.confidence,
            "the 7-day-old cluster ({}), not the 200-day-old one ({}), must rank first",
            recent.confidence,
            stale.confidence
        );
        assert_eq!(findings[0].finding_id, recent.finding_id);
        assert!((recent.confidence - 0.5_f64.powf(7.0 / 30.0)).abs() < 1e-9);
        assert!((stale.confidence - 0.5_f64.powf(200.0 / 30.0)).abs() < 1e-9);
    }

    // ── §12 lifecycle guard on re-detect ─────────────────────────────────

    #[tokio::test]
    async fn re_detect_preserves_an_advanced_lifecycle_status() {
        let pool = test_pool().await;
        seed_above_threshold(&pool).await;
        let cfg = cfg_at(NOW);
        let first = detect(&pool, &cfg).await.unwrap();
        assert_eq!(first[0].status, FindingStatus::Candidate);
        // Advance the lifecycle through the state machine, then persist
        // the new status the way a reviewer would.
        let advanced = transition(first[0].status, FindingStatus::Active).unwrap();
        assert_eq!(advanced, FindingStatus::Active);
        sqlx::query("UPDATE patterns SET status = ? WHERE id = ?")
            .bind(advanced.as_str())
            .bind(&first[0].finding_id)
            .execute(&pool)
            .await
            .unwrap();
        let second = detect(&pool, &cfg).await.unwrap();
        assert_eq!(
            second[0].status,
            FindingStatus::Active,
            "a re-detect must not reset an advanced lifecycle status"
        );
        // A resolved finding must not be resurrected either: walk the
        // whole §12 chain to resolved, then try to go back.
        let mut status = FindingStatus::Active;
        for next in [
            FindingStatus::Acknowledged,
            FindingStatus::ProposalCreated,
            FindingStatus::FixInProgress,
            FindingStatus::Monitoring,
            FindingStatus::Resolved,
        ] {
            status = transition(status, next).unwrap();
        }
        assert_eq!(status, FindingStatus::Resolved);
        assert!(
            transition(status, FindingStatus::Candidate).is_err(),
            "resolved -> candidate must stay illegal even after a re-detect"
        );
        sqlx::query("UPDATE patterns SET status = ? WHERE id = ?")
            .bind(FindingStatus::Resolved.as_str())
            .bind(&first[0].finding_id)
            .execute(&pool)
            .await
            .unwrap();
        let third = detect(&pool, &cfg).await.unwrap();
        assert_eq!(third[0].status, FindingStatus::Resolved);
    }

    #[tokio::test]
    async fn re_detect_does_not_duplicate_patterns_or_evidence_rows() {
        let pool = test_pool().await;
        seed_above_threshold(&pool).await;
        let cfg = cfg_at(NOW);
        let first = detect(&pool, &cfg).await.unwrap();
        detect(&pool, &cfg).await.unwrap();
        let finding_id = &first[0].finding_id;
        assert_eq!(
            count_rows(&pool, "patterns", "id", finding_id).await,
            1,
            "re-detect must upsert, not duplicate, the pattern row"
        );
        assert_eq!(
            evidence_rows(&pool, finding_id).await.len(),
            6,
            "re-detect must not duplicate evidence rows"
        );
        // The pattern row carries the aggregate the §10 example shows.
        let row = sqlx::query("SELECT session_count FROM patterns WHERE id = ?")
            .bind(finding_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(row.get::<i64, _>(0), 3);
    }

    // ── PostgreSQL 16 (migrated schema, real TIMESTAMP columns) ─────────

    #[tokio::test]
    async fn detect_runs_on_postgres_16_migrated_schema() {
        let Ok(url) = std::env::var("AUTOSPEC_TEST_DB_URL") else {
            eprintln!(
                // autospec:allow-output — test SKIP notice
                "SKIP detect_runs_on_postgres_16_migrated_schema: \
                 AUTOSPEC_TEST_DB_URL is not set"
            );
            return;
        };
        let pool = db::open_shared_db(&url)
            .await
            .expect("AUTOSPEC_TEST_DB_URL must point at a reachable PostgreSQL 16");
        db::apply_subsystem_migrations(&pool, db::SUBSYSTEM_INSIGHTS)
            .await
            .expect("insights migrations must apply on PostgreSQL 16");
        for table in [
            "finding_evidence",
            "patterns",
            "user_interventions",
            "sessions",
        ] {
            sqlx::query(&format!("DELETE FROM {table}"))
                .execute(&pool)
                .await
                .unwrap();
        }
        seed_above_threshold(&pool).await;
        let findings = detect(&pool, &cfg_at(NOW)).await.unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].occurrences, 6);
        assert!(
            !evidence_rows(&pool, &findings[0].finding_id)
                .await
                .is_empty(),
            "evidence rows must be written on PostgreSQL 16"
        );
    }
}
