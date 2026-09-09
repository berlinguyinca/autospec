//! Tests for the deterministic §8 summary reducer and its storage boundary.

use serde_json::Value;
use sqlx::{AnyPool, Row};

use super::*;

/// Fixture session all events belong to.
const SESSION: &str = "sess_fixture_01";
/// Monotonic fixture start: 2026-09-08T08:00:00Z, 90s between events.
const T0: i64 = 1_789_833_600;
const STEP: i64 = 90;
/// The two models the fixture session runs on (fallback mid-session).
const MODEL_A: &str = "claude-opus-5";
const MODEL_B: &str = "claude-sonnet-5";
const COMMIT_SHA: &str = "8b3f4d2a91c4e5f6071829a3b4c5d6e7f8091a2b";
const PR_NUMBER: u64 = 1234;

/// One fixture row: event type, optional model, token usage, payload.
type Spec = (
    EventType,
    Option<&'static str>,
    (u64, u64),
    serde_json::Value,
);

/// The 40-event fixture: one session, two models, a rework review, one
/// commit and one PR. Derived expectations are hand-computed below and
/// asserted literally, so a reducer change cannot silently move them.
fn fixture() -> Vec<NormalizedEvent> {
    use serde_json::json;
    let assistant = |i: usize| {
        (
            EventType::AssistantMessage,
            Some(if i < 4 { MODEL_A } else { MODEL_B }),
            (100, 50),
            Value::Null,
        )
    };
    let specs: Vec<Spec> = vec![
        (EventType::SessionStarted, None, (0, 0), Value::Null),
        (EventType::ModelSelected, Some(MODEL_A), (0, 0), Value::Null),
        (EventType::UserMessage, None, (0, 0), Value::Null),
        assistant(0),
        (EventType::ToolCall, None, (0, 0), Value::Null),
        (EventType::ToolResult, None, (0, 0), Value::Null),
        (EventType::FileRead, None, (0, 0), Value::Null),
        (EventType::FileRead, None, (0, 0), Value::Null),
        assistant(1),
        (EventType::ToolCall, None, (0, 0), Value::Null),
        (EventType::ToolError, None, (0, 0), Value::Null),
        (EventType::UserIntervention, None, (0, 0), Value::Null),
        assistant(2),
        (EventType::ToolCall, None, (0, 0), Value::Null),
        (EventType::FileWrite, None, (0, 0), Value::Null),
        (EventType::ToolCall, None, (0, 0), Value::Null),
        (EventType::ToolResult, None, (0, 0), Value::Null),
        (EventType::CommandRun, None, (0, 0), Value::Null),
        (EventType::CommandFailed, None, (0, 0), Value::Null),
        (EventType::UserIntervention, None, (0, 0), Value::Null),
        assistant(3),
        (EventType::ToolCall, None, (0, 0), Value::Null),
        (EventType::ToolError, None, (0, 0), Value::Null),
        (EventType::FilePatch, None, (0, 0), Value::Null),
        (EventType::ModelSelected, Some(MODEL_B), (0, 0), Value::Null),
        assistant(4),
        (EventType::ToolCall, None, (0, 0), Value::Null),
        (EventType::ToolResult, None, (0, 0), Value::Null),
        (EventType::FileRead, None, (0, 0), Value::Null),
        (EventType::FileWrite, None, (0, 0), Value::Null),
        (EventType::ContextLimitWarning, None, (900, 0), Value::Null),
        assistant(5),
        (EventType::ToolCall, None, (0, 0), Value::Null),
        (EventType::TestRun, None, (0, 0), Value::Null),
        (
            EventType::ReviewResult,
            None,
            (0, 0),
            json!({"rework": true}),
        ),
        assistant(6),
        (EventType::ContextCompaction, None, (0, 0), Value::Null),
        (
            EventType::GitCommit,
            None,
            (0, 0),
            json!({"sha": COMMIT_SHA}),
        ),
        (
            EventType::PullRequestOpened,
            None,
            (0, 0),
            json!({"number": PR_NUMBER}),
        ),
        (EventType::SessionFinished, None, (0, 0), Value::Null),
    ]; // exactly 40 events
    specs
        .into_iter()
        .enumerate()
        .map(
            |(index, (event_type, model, tokens, payload))| NormalizedEvent {
                event_id: format!("evt-{index:04}"),
                session_id: SESSION.to_string(),
                event_type,
                timestamp: T0 + i64::try_from(index).expect("fixture fits i64") * STEP,
                model: model.map(str::to_string),
                tokens: TokenUsage {
                    input: tokens.0,
                    output: tokens.1,
                },
                payload,
            },
        )
        .collect()
}

#[test]
fn fixture_has_exactly_forty_events() {
    assert_eq!(fixture().len(), 40);
}

#[test]
fn summarize_derives_known_counts_from_fixture() {
    let summary = summarize(&fixture());
    assert_eq!(summary.session_id, SESSION);
    assert_eq!(summary.tool_calls, 7);
    assert_eq!(summary.tool_errors, 2);
    assert_eq!(summary.files_read, 3);
    assert_eq!(summary.files_changed, 3); // 2 file_write + 1 file_patch
    assert_eq!(summary.tests_run, 1);
    assert_eq!(summary.user_interventions, 2);
    assert_eq!(summary.review_rework_count, 1);
}

#[test]
fn summarize_derives_known_tokens_duration_and_autonomy() {
    let summary = summarize(&fixture());
    // 7 assistant turns at 100 in / 50 out plus one 900-token limit warning.
    assert_eq!(summary.input_tokens, 1_600);
    assert_eq!(summary.output_tokens, 350);
    assert_eq!(summary.context_peak_tokens, 900);
    assert_eq!(summary.duration_seconds, 39 * STEP);
    // 1 - 2/7, the exact f64 the clamp expression produces.
    let expected = 1.0 - (2.0_f64 / 7.0_f64);
    assert!((summary.autonomy_score - expected).abs() < 1e-12);
    assert_eq!(summary.estimated_cost, 0.0);
}

#[test]
fn summarize_keeps_semantic_fields_null() {
    let summary = summarize(&fixture());
    assert_eq!(summary.task_type, None);
    assert_eq!(summary.task_domain, None);
    assert_eq!(summary.outcome, None);
}

#[test]
fn summarize_collects_models_commits_and_pull_requests() {
    let summary = summarize(&fixture());
    assert_eq!(
        summary.models,
        vec![MODEL_A.to_string(), MODEL_B.to_string()]
    );
    assert_eq!(summary.commits, vec![COMMIT_SHA.to_string()]);
    assert_eq!(summary.pull_requests, vec![PR_NUMBER]);
}

#[test]
fn summarize_is_deterministic_across_runs_and_order() {
    let events = fixture();
    assert_eq!(summarize(&events), summarize(&events));
    let mut reversed = events.clone();
    reversed.reverse();
    // Every field is order-independent: session_id is per-session-constant.
    assert_eq!(summarize(&events), summarize(&reversed));
}

#[test]
fn summarize_of_empty_input_is_all_zero() {
    let summary = summarize(&[]);
    #[allow(clippy::redundant_field_names)]
    let expected = SessionSummary {
        session_id: String::new(),
        task_type: None,
        task_domain: None,
        outcome: None,
        autonomy_score: 1.0,
        user_interventions: 0,
        review_rework_count: 0,
        tool_calls: 0,
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
    };
    assert_eq!(summary, expected);
}

#[test]
fn should_recompute_gates_on_extractor_version() {
    assert!(should_recompute(None));
    assert!(should_recompute(Some("0.9.0")));
    assert!(!should_recompute(Some(EXTRACTOR_VERSION)));
}

// ── storage tests ───────────────────────────────────────────────────────────

/// Read-back assertions shared by both backend tests: one row, second-write
/// values, version stamp, and the recompute gate.
async fn assert_upserted(pool: &AnyPool, summary: &SessionSummary) {
    let row = sqlx::query(
        "SELECT tool_calls, extractor_version FROM session_summaries WHERE session_id = ?",
    )
    .bind(&summary.session_id)
    .fetch_one(pool)
    .await
    .expect("read back summary");
    let tool_calls: i64 = row.try_get("tool_calls").expect("tool_calls column");
    assert_eq!(
        u32::try_from(tool_calls).expect("fits u32"),
        summary.tool_calls
    );
    let version: String = row.try_get("extractor_version").expect("version column");
    assert_eq!(version, EXTRACTOR_VERSION);

    let rows = sqlx::query("SELECT COUNT(*) FROM session_summaries WHERE session_id = ?")
        .bind(&summary.session_id)
        .fetch_one(pool)
        .await
        .expect("count rows");
    let count: i64 = rows.try_get(0).expect("count column");
    assert_eq!(count, 1, "second write must update, not duplicate");
}

async fn store_twice_and_verify(pool: &AnyPool, session: &str) {
    ensure_summary_table(pool).await.expect("create table");
    let mut base = summarize(&fixture());
    base.session_id = session.to_string();
    store_summary(pool, &base).await.expect("first store");

    let mut updated = base.clone();
    updated.tool_calls = 11;
    updated.input_tokens = 4_242;
    store_summary(pool, &updated)
        .await
        .expect("second store (upsert)");
    assert_upserted(pool, &updated).await;

    // Recompute gate: current version stays, stale version recomputes.
    assert!(!needs_recompute(pool, session).await.expect("fresh gate"));
    sqlx::query("UPDATE session_summaries SET extractor_version = '0.0.1' WHERE session_id = ?")
        .bind(session)
        .execute(pool)
        .await
        .expect("age the stored version");
    assert!(needs_recompute(pool, session).await.expect("stale gate"));
    let absent = format!("{session}-absent");
    assert!(needs_recompute(pool, &absent).await.expect("absent gate"));
}

#[tokio::test]
async fn store_summary_upserts_one_row_on_sqlite() {
    let dir = std::env::temp_dir().join(format!("autospec-summarize-test-{}", std::process::id()));
    let url = format!(
        "sqlite://{}/summaries-{}.db",
        dir.display(),
        // unique per test in case of parallel runs in the same process id space
        line!()
    );
    let pool = crate::resources::db::open_shared_db(&url)
        .await
        .expect("open sqlite test db");
    store_twice_and_verify(&pool, SESSION).await;
    drop(pool);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Real PostgreSQL 16, disposable, provided via `AUTOSPEC_TEST_DB_URL`
/// (e.g. `postgres://autospec:autospec@127.0.0.1:55432/autospec_test`).
/// Skipped when the variable is unset; never mocked.
#[tokio::test]
async fn store_summary_upserts_one_row_on_postgres() {
    let Ok(url) = std::env::var("AUTOSPEC_TEST_DB_URL") else {
        eprintln!("SKIP: AUTOSPEC_TEST_DB_URL unset; PostgreSQL upsert test skipped");
        return;
    };
    let pool = crate::resources::db::open_shared_db(&url)
        .await
        .expect("open AUTOSPEC_TEST_DB_URL");
    let session = format!("{}-pg-{}", SESSION, std::process::id());
    store_twice_and_verify(&pool, &session).await;
    // The test owns its rows only; the table itself stays for other users.
    sqlx::query("DELETE FROM session_summaries WHERE session_id = ?")
        .bind(&session)
        .execute(&pool)
        .await
        .expect("cleanup test rows");
}
