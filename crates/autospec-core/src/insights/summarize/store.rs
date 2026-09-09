//! `session_summaries` write path (issue #3837; table from dependency
//! issue #3827).
//!
//! Backend-neutral: the same code runs on the SQLite and Postgres
//! `AnyPool` backends (ADR 0001 D10). List-valued columns are stored as
//! JSON text so one DDL binds on both backends.

use sqlx::{AnyPool, Row};

use crate::error::AutospecError;
use crate::insights::events::NormalizedEvent;

use super::{summarize, SessionSummary, EXTRACTOR_VERSION};

/// Create the `session_summaries` table if absent (idempotent DDL shared
/// by both backends).
const SESSION_SUMMARIES_DDL: &str = "CREATE TABLE IF NOT EXISTS session_summaries (\
    session_id TEXT PRIMARY KEY, \
    task_type TEXT, \
    task_domain TEXT NOT NULL, \
    outcome TEXT, \
    autonomy_score REAL NOT NULL, \
    user_interventions INTEGER NOT NULL, \
    review_rework_count INTEGER NOT NULL, \
    tool_calls INTEGER NOT NULL, \
    tool_errors INTEGER NOT NULL, \
    files_read INTEGER NOT NULL, \
    files_changed INTEGER NOT NULL, \
    tests_run INTEGER NOT NULL, \
    context_peak_tokens INTEGER NOT NULL, \
    input_tokens INTEGER NOT NULL, \
    output_tokens INTEGER NOT NULL, \
    estimated_cost REAL NOT NULL, \
    duration_seconds INTEGER NOT NULL, \
    models TEXT NOT NULL, \
    commits TEXT NOT NULL, \
    pull_requests TEXT NOT NULL, \
    extractor_version TEXT NOT NULL \
)";

fn map_error(operation: &str, error: sqlx::Error) -> AutospecError {
    AutospecError::State {
        entity: "session_summaries".to_string(),
        message: format!("{operation}: {error}"),
    }
}

fn json_str<T: serde::Serialize>(value: &T, what: &str) -> Result<String, AutospecError> {
    serde_json::to_string(value).map_err(|error| AutospecError::parse(what, error.to_string()))
}

/// Create the `session_summaries` table if it does not exist.
pub async fn ensure_session_summaries(pool: &AnyPool) -> Result<(), AutospecError> {
    sqlx::query(SESSION_SUMMARIES_DDL)
        .execute(pool)
        .await
        .map_err(|error| map_error("ensure schema", error))
        .map(|_| ())
}

/// Upsert `summary` into `session_summaries` by `session_id`.
///
/// Running twice for the same `session_id` leaves exactly one row (the
/// second run overwrites the first).
pub async fn store_summary(pool: &AnyPool, summary: &SessionSummary) -> Result<(), AutospecError> {
    ensure_session_summaries(pool).await?;
    sqlx::query(
        "INSERT INTO session_summaries (\
         session_id, task_type, task_domain, outcome, autonomy_score, \
         user_interventions, review_rework_count, tool_calls, tool_errors, \
         files_read, files_changed, tests_run, context_peak_tokens, \
         input_tokens, output_tokens, estimated_cost, duration_seconds, \
         models, commits, pull_requests, extractor_version) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21) \
         ON CONFLICT (session_id) DO UPDATE SET \
         task_type = excluded.task_type, \
         task_domain = excluded.task_domain, \
         outcome = excluded.outcome, \
         autonomy_score = excluded.autonomy_score, \
         user_interventions = excluded.user_interventions, \
         review_rework_count = excluded.review_rework_count, \
         tool_calls = excluded.tool_calls, \
         tool_errors = excluded.tool_errors, \
         files_read = excluded.files_read, \
         files_changed = excluded.files_changed, \
         tests_run = excluded.tests_run, \
         context_peak_tokens = excluded.context_peak_tokens, \
         input_tokens = excluded.input_tokens, \
         output_tokens = excluded.output_tokens, \
         estimated_cost = excluded.estimated_cost, \
         duration_seconds = excluded.duration_seconds, \
         models = excluded.models, \
         commits = excluded.commits, \
         pull_requests = excluded.pull_requests, \
         extractor_version = excluded.extractor_version",
    )
    .bind(&summary.session_id)
    .bind(&summary.task_type)
    .bind(json_str(&summary.task_domain, "task_domain")?)
    .bind(&summary.outcome)
    .bind(summary.autonomy_score)
    .bind(i64::try_from(summary.user_interventions).unwrap_or(i64::MAX))
    .bind(i64::try_from(summary.review_rework_count).unwrap_or(i64::MAX))
    .bind(i64::try_from(summary.tool_calls).unwrap_or(i64::MAX))
    .bind(i64::try_from(summary.tool_errors).unwrap_or(i64::MAX))
    .bind(i64::try_from(summary.files_read).unwrap_or(i64::MAX))
    .bind(i64::try_from(summary.files_changed).unwrap_or(i64::MAX))
    .bind(i64::try_from(summary.tests_run).unwrap_or(i64::MAX))
    .bind(i64::try_from(summary.context_peak_tokens).unwrap_or(i64::MAX))
    .bind(i64::try_from(summary.input_tokens).unwrap_or(i64::MAX))
    .bind(i64::try_from(summary.output_tokens).unwrap_or(i64::MAX))
    .bind(summary.estimated_cost)
    .bind(i64::try_from(summary.duration_seconds).unwrap_or(i64::MAX))
    .bind(json_str(&summary.models, "models")?)
    .bind(json_str(&summary.commits, "commits")?)
    .bind(json_str(&summary.pull_requests, "pull_requests")?)
    .bind(EXTRACTOR_VERSION)
    .execute(pool)
    .await
    .map_err(|error| map_error("upsert", error))?;
    Ok(())
}

/// Recompute the §8 summary for `events` and persist it, but only when the
/// stored row's `extractor_version` differs from [`EXTRACTOR_VERSION`]
/// (spec §51: raw history is not reprocessed unless the analyzer version
/// changes). A missing row counts as a version change.
///
/// Returns `true` when a row was written, `false` when the stored version
/// already matched and nothing was touched.
pub async fn recompute_summary(
    pool: &AnyPool,
    events: &[NormalizedEvent],
) -> Result<bool, AutospecError> {
    ensure_session_summaries(pool).await?;
    let session_id = events
        .first()
        .map(|event| event.session_id.clone())
        .unwrap_or_default();
    let stored: Option<String> =
        sqlx::query("SELECT extractor_version FROM session_summaries WHERE session_id = $1")
            .bind(&session_id)
            .fetch_optional(pool)
            .await
            .map_err(|error| map_error("read extractor_version", error))?
            .map(|row| row.try_get::<String, _>(0).unwrap_or_default());
    if stored.as_deref() == Some(EXTRACTOR_VERSION) {
        return Ok(false);
    }
    store_summary(pool, &summarize(events)).await?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::insights::events::{EventType, Tokens};
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());
    static FILE_COUNTER: AtomicU32 = AtomicU32::new(0);

    /// Test database URL: `AUTOSPEC_TEST_DB_URL` (disposable PostgreSQL 16
    /// under Apptainer in the operator full run) when set; otherwise a
    /// disposable per-test SQLite file. Always a real database — no
    /// database mocks.
    fn test_db_url() -> String {
        match std::env::var("AUTOSPEC_TEST_DB_URL") {
            Ok(url) if !url.trim().is_empty() => url,
            _ => {
                let counter = FILE_COUNTER.fetch_add(1, Ordering::SeqCst);
                format!(
                    "sqlite://{}/autospec-session-summaries-{}-{}.db",
                    std::env::temp_dir().display(),
                    std::process::id(),
                    counter
                )
            }
        }
    }

    async fn open_test_pool() -> AnyPool {
        crate::resources::db::open_shared_db(&test_db_url())
            .await
            .expect("test database must open")
    }

    fn small_summary(id: &str, tool_calls: u64) -> SessionSummary {
        SessionSummary {
            session_id: id.to_string(),
            task_type: None,
            task_domain: Vec::new(),
            outcome: None,
            autonomy_score: 1.0,
            user_interventions: 0,
            review_rework_count: 0,
            tool_calls,
            tool_errors: 0,
            files_read: 0,
            files_changed: 0,
            tests_run: 0,
            context_peak_tokens: 0,
            input_tokens: 0,
            output_tokens: 0,
            estimated_cost: 0.0,
            duration_seconds: 0,
            models: Vec::new(),
            commits: Vec::new(),
            pull_requests: Vec::new(),
        }
    }

    /// A unique session id per test run: the shared `AUTOSPEC_TEST_DB_URL`
    /// (Postgres) keeps rows between runs, so a fixed id would leak state.
    fn unique_session_id(prefix: &str) -> String {
        let counter = FILE_COUNTER.fetch_add(1, Ordering::SeqCst);
        format!("{prefix}-{}-{}", std::process::id(), counter)
    }

    fn fixture_events(session_id: &str) -> Vec<NormalizedEvent> {
        (0..3)
            .map(|i| NormalizedEvent {
                event_id: format!("evt-{i}"),
                session_id: session_id.into(),
                parent_session_id: None,
                timestamp: i as f64,
                repo: None,
                branch: None,
                work_item_id: None,
                agent_role: None,
                provider: None,
                model: Some("qwen3".into()),
                event_type: EventType::ToolCall,
                tool: None,
                payload: serde_json::json!({}),
                tokens: Tokens {
                    input: 10,
                    output: 1,
                },
            })
            .collect()
    }

    #[tokio::test]
    async fn store_summary_twice_leaves_exactly_one_row() {
        let _guard = ENV_LOCK.lock().unwrap();
        let pool = open_test_pool().await;
        let session_id = unique_session_id("session-recompute");
        let summary = summarize(&fixture_events(&session_id));
        store_summary(&pool, &summary).await.unwrap();
        store_summary(&pool, &summary).await.unwrap();

        let row = sqlx::raw_sql(
            format!("SELECT COUNT(*) FROM session_summaries WHERE session_id = '{session_id}'")
                .as_str(),
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(row.try_get::<i64, _>(0).unwrap(), 1);
    }

    #[tokio::test]
    async fn store_summary_round_trips_counts_and_null_semantic_fields() {
        let _guard = ENV_LOCK.lock().unwrap();
        let pool = open_test_pool().await;
        let session_id = unique_session_id("session-roundtrip");
        let summary = small_summary(&session_id, 7);
        store_summary(&pool, &summary).await.unwrap();

        let row = sqlx::raw_sql(
            format!(
                "SELECT tool_calls, task_type, outcome, extractor_version \
                 FROM session_summaries WHERE session_id = '{session_id}'"
            )
            .as_str(),
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(row.try_get::<i64, _>(0).unwrap(), 7);
        assert!(
            row.try_get::<Option<String>, _>(1).unwrap().is_none(),
            "task_type must stay null"
        );
        assert!(
            row.try_get::<Option<String>, _>(2).unwrap().is_none(),
            "outcome must stay null"
        );
        assert_eq!(row.try_get::<String, _>(3).unwrap(), EXTRACTOR_VERSION);
    }

    #[tokio::test]
    async fn store_summary_upsert_overwrites_previous_values() {
        let _guard = ENV_LOCK.lock().unwrap();
        let pool = open_test_pool().await;
        let session_id = unique_session_id("session-upsert");
        store_summary(&pool, &small_summary(&session_id, 1))
            .await
            .unwrap();
        store_summary(&pool, &small_summary(&session_id, 9))
            .await
            .unwrap();

        let row = sqlx::raw_sql(
            format!("SELECT tool_calls FROM session_summaries WHERE session_id = '{session_id}'")
                .as_str(),
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(row.try_get::<i64, _>(0).unwrap(), 9);
    }

    #[tokio::test]
    async fn recompute_writes_when_stored_version_differs() {
        let _guard = ENV_LOCK.lock().unwrap();
        let pool = open_test_pool().await;
        let session_id = unique_session_id("session-recompute");
        ensure_session_summaries(&pool).await.unwrap();
        sqlx::query(format!(
            "INSERT INTO session_summaries (session_id, task_domain, autonomy_score, \
             user_interventions, review_rework_count, tool_calls, tool_errors, files_read, \
             files_changed, tests_run, context_peak_tokens, input_tokens, output_tokens, \
             estimated_cost, duration_seconds, models, commits, pull_requests, extractor_version) \
             VALUES ('{session_id}', '[]', 1.0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0.0, 0, '[]', '[]', '[]', '0.9.0')"
        )
        .as_str())
        .execute(&pool)
        .await
        .unwrap();

        let events = fixture_events(&session_id);
        let written = recompute_summary(&pool, &events).await.unwrap();
        assert!(
            written,
            "a stale extractor_version must trigger a recompute"
        );

        let row = sqlx::raw_sql(
            format!(
                "SELECT tool_calls, extractor_version FROM session_summaries \
                 WHERE session_id = '{session_id}'"
            )
            .as_str(),
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(row.try_get::<i64, _>(0).unwrap(), 3);
        assert_eq!(row.try_get::<String, _>(1).unwrap(), EXTRACTOR_VERSION);
    }

    #[tokio::test]
    async fn recompute_skips_when_stored_version_matches() {
        let _guard = ENV_LOCK.lock().unwrap();
        let pool = open_test_pool().await;
        let session_id = unique_session_id("session-recompute");
        let written = recompute_summary(&pool, &fixture_events(&session_id))
            .await
            .unwrap();
        assert!(written, "a missing row must be written");

        // Same version: a recompute is a no-op even if the events changed.
        let mut changed = fixture_events(&session_id);
        changed.push(changed[0].clone());
        let skipped = recompute_summary(&pool, &changed).await.unwrap();
        assert!(
            !skipped,
            "matching extractor_version must skip the recompute"
        );

        let row = sqlx::raw_sql(
            format!("SELECT tool_calls FROM session_summaries WHERE session_id = '{session_id}'")
                .as_str(),
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            row.try_get::<i64, _>(0).unwrap(),
            3,
            "row must be untouched"
        );
    }
}
