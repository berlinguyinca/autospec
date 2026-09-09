//! Persistence for the §8 session summary record.
//!
//! Storage rides the one shared autospec database behind the backend-neutral
//! [`AnyPool`] (`crate::resources::db`, ADR 0001 D5/D10). `session_summaries`
//! is created here with `CREATE TABLE IF NOT EXISTS` rather than a namespaced
//! migration so this module stays self-contained until the #3827 storage
//! epic adds the table to the migration set; the statement is a no-op once
//! that migration exists.
//!
//! Every stored row carries [`super::EXTRACTOR_VERSION`]. Raw events are only
//! reprocessed when that version differs from the stored one (§51: "raw
//! history MUST NOT be reprocessed unless analyzer version changes"; §52
//! versioning for reproducibility).

use std::time::{SystemTime, UNIX_EPOCH};

use sqlx::{AnyPool, Executor, Row};

use super::{SessionSummary, EXTRACTOR_VERSION};
use crate::error::AutospecError;

/// DDL for `session_summaries`, valid on both pooled backends (SQLite and
/// Postgres): integer columns BIGINT, floats DOUBLE PRECISION, list/JSON
/// fields TEXT. Keyed by `session_id` (one summary row per session).
pub const CREATE_TABLE_SQL: &str = "CREATE TABLE IF NOT EXISTS session_summaries (
    session_id TEXT PRIMARY KEY,
    task_type TEXT,
    task_domain TEXT,
    outcome TEXT,
    autonomy_score DOUBLE PRECISION NOT NULL,
    user_interventions BIGINT NOT NULL,
    review_rework_count BIGINT NOT NULL,
    tool_calls BIGINT NOT NULL,
    tool_errors BIGINT NOT NULL,
    files_read BIGINT NOT NULL,
    files_changed BIGINT NOT NULL,
    tests_run BIGINT NOT NULL,
    context_peak_tokens BIGINT NOT NULL,
    input_tokens BIGINT NOT NULL,
    output_tokens BIGINT NOT NULL,
    estimated_cost DOUBLE PRECISION NOT NULL,
    duration_seconds BIGINT NOT NULL,
    models TEXT NOT NULL,
    commits TEXT NOT NULL,
    pull_requests TEXT NOT NULL,
    extractor_version TEXT NOT NULL,
    updated_at BIGINT NOT NULL
)";

/// Upsert keyed by `session_id`. `ON CONFLICT ... DO UPDATE` is supported by
/// both pooled backends; `updated_at` refreshes on every write.
const UPSERT_SQL: &str = "INSERT INTO session_summaries (
    session_id, task_type, task_domain, outcome, autonomy_score,
    user_interventions, review_rework_count, tool_calls, tool_errors,
    files_read, files_changed, tests_run, context_peak_tokens,
    input_tokens, output_tokens, estimated_cost, duration_seconds,
    models, commits, pull_requests, extractor_version, updated_at
) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
ON CONFLICT (session_id) DO UPDATE SET
    task_type = excluded.task_type,
    task_domain = excluded.task_domain,
    outcome = excluded.outcome,
    autonomy_score = excluded.autonomy_score,
    user_interventions = excluded.user_interventions,
    review_rework_count = excluded.review_rework_count,
    tool_calls = excluded.tool_calls,
    tool_errors = excluded.tool_errors,
    files_read = excluded.files_read,
    files_changed = excluded.files_changed,
    tests_run = excluded.tests_run,
    context_peak_tokens = excluded.context_peak_tokens,
    input_tokens = excluded.input_tokens,
    output_tokens = excluded.output_tokens,
    estimated_cost = excluded.estimated_cost,
    duration_seconds = excluded.duration_seconds,
    models = excluded.models,
    commits = excluded.commits,
    pull_requests = excluded.pull_requests,
    extractor_version = excluded.extractor_version,
    updated_at = excluded.updated_at";

fn map_db_error(operation: &str, error: sqlx::Error) -> AutospecError {
    AutospecError::Io {
        operation: operation.to_string(),
        path: "session_summaries".to_string(),
        source: error.to_string(),
    }
}

fn json_text(value: &impl serde::Serialize) -> Result<String, AutospecError> {
    serde_json::to_string(value).map_err(|error| {
        AutospecError::validation(format!("cannot serialize session summary field: {error}"))
    })
}

fn now_epoch_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0)
}

/// Create the `session_summaries` table if it does not exist yet. Idempotent.
pub async fn ensure_summary_table(pool: &AnyPool) -> Result<(), AutospecError> {
    pool.execute(sqlx::raw_sql(CREATE_TABLE_SQL))
        .await
        .map_err(|error| map_db_error("create session_summaries table", error))?;
    Ok(())
}

/// Upsert `summary` into `session_summaries`, stamped with the current
/// [`super::EXTRACTOR_VERSION`]. A second write for the same `session_id`
/// updates the existing row (primary key), never duplicates it.
pub async fn store_summary(pool: &AnyPool, summary: &SessionSummary) -> Result<(), AutospecError> {
    let task_domain = json_text(&summary.task_domain)?;
    let models = json_text(&summary.models)?;
    let commits = json_text(&summary.commits)?;
    let pull_requests = json_text(&summary.pull_requests)?;
    sqlx::query(UPSERT_SQL)
        .bind(&summary.session_id)
        .bind(&summary.task_type)
        .bind(task_domain)
        .bind(&summary.outcome)
        .bind(summary.autonomy_score)
        .bind(i64::from(summary.user_interventions))
        .bind(i64::from(summary.review_rework_count))
        .bind(i64::from(summary.tool_calls))
        .bind(i64::from(summary.tool_errors))
        .bind(i64::from(summary.files_read))
        .bind(i64::from(summary.files_changed))
        .bind(i64::from(summary.tests_run))
        .bind(summary.context_peak_tokens as i64)
        .bind(summary.input_tokens as i64)
        .bind(summary.output_tokens as i64)
        .bind(summary.estimated_cost)
        .bind(summary.duration_seconds)
        .bind(models)
        .bind(commits)
        .bind(pull_requests)
        .bind(EXTRACTOR_VERSION)
        .bind(now_epoch_seconds())
        .execute(pool)
        .await
        .map_err(|error| map_db_error("upsert session summary", error))?;
    Ok(())
}

/// The `extractor_version` stored for `session_id`, or `None` when no summary
/// has been stored for that session yet.
pub async fn stored_extractor_version(
    pool: &AnyPool,
    session_id: &str,
) -> Result<Option<String>, AutospecError> {
    let row = sqlx::query("SELECT extractor_version FROM session_summaries WHERE session_id = ?")
        .bind(session_id)
        .fetch_optional(pool)
        .await
        .map_err(|error| map_db_error("read stored extractor version", error))?;
    row.map(|row| {
        row.try_get::<String, _>("extractor_version")
            .map_err(|error| map_db_error("decode stored extractor version", error))
    })
    .transpose()
}

/// Pure recompute gate: recompute only when nothing is stored yet or the
/// stored extractor version differs from the running one (§51/§52). Never a
/// full-history rescan on its own.
pub fn should_recompute(stored_extractor_version: Option<&str>) -> bool {
    stored_extractor_version != Some(EXTRACTOR_VERSION)
}

/// Whether the stored summary for `session_id` must be recomputed from raw
/// events: `true` when absent or written by a different extractor version.
pub async fn needs_recompute(pool: &AnyPool, session_id: &str) -> Result<bool, AutospecError> {
    let stored = stored_extractor_version(pool, session_id).await?;
    Ok(should_recompute(stored.as_deref()))
}
