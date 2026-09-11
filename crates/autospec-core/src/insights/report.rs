//! Agent Intelligence dashboard aggregate payloads (issue #3846, parent
//! #3825).
//!
//! Spec: `docs/specs/2026-09-08-continuous-improvement-engine.md` —
//! §28-§33 (the six panels), §43 (API surface), §39 (privacy), §51
//! (performance).
//!
//! The six payload structs map one-to-one onto the §43 read paths under
//! `/api/intelligence`: `ModelsPanel` → `/models`, `ToolsPanel` → `/tools`,
//! `ContextPanel` → `/context`, `QualityPanel` → `/quality`,
//! `ImprovementPanel` → `/findings` + `/proposals`, and
//! `AgentPerformancePanel` → `/sessions` (the overview). Every payload
//! carries drill-down row ids (§33) and nothing else: ids and counts only —
//! no `payload`, `summary`, `excerpt`, `description`, `rationale`, or
//! `title` column is ever selected into a payload (§39).
//!
//! [`PanelFilter`] values are always bound as query parameters, never
//! formatted into SQL. `repo`, `model`, and the ISO-8601 time window
//! (`since` inclusive, `until` exclusive; uniform-width ISO-8601 sorts
//! chronologically, the convention shared with `insights::models` and
//! `insights::tools`) narrow every panel. `branch` and `role` are carried
//! for the §28 filter set, but no issue-#3827 table has a branch or role
//! column yet, so they narrow nothing until ingest populates one.
//!
//! §51: one aggregate query per panel (scalar subqueries), predicates on
//! indexed structural columns, drill-down ids capped.

use crate::error::AutospecError;
use serde::{Deserialize, Serialize};
use sqlx::{AnyPool, Row};

/// Shared dashboard filter for the §28-§33 panels. `None` = match all.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PanelFilter {
    pub repo: Option<String>,
    /// No issue-#3827 table has a branch column yet; see module docs.
    pub branch: Option<String>,
    /// ISO-8601 UTC lower bound on `sessions.started_at` (inclusive).
    pub since: Option<String>,
    /// ISO-8601 UTC upper bound on `sessions.started_at` (exclusive).
    pub until: Option<String>,
    pub model: Option<String>,
    /// No issue-#3827 table has a role column yet; see module docs.
    pub role: Option<String>,
}

/// §28 Agent Performance panel: the 10 §28 measures plus the session count
/// they are divided by, and the session ids that back them (§33 drill-down).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentPerformancePanel {
    pub sessions: i64,
    /// `sessions.status = 'success'` / sessions.
    pub task_success_rate: f64,
    /// successful sessions with no user intervention and no review rework
    /// (per `session_summaries`) / sessions.
    pub first_pass_success: f64,
    /// sessions whose summary records review rework / sessions.
    pub rework_rate: f64,
    /// total `session_summaries.user_interventions`.
    pub user_interventions: i64,
    /// recorded `session_events` per session.
    pub turns_per_task: f64,
    /// `input_tokens + output_tokens` per session.
    pub tokens_per_task: f64,
    /// `duration_seconds` per session, in milliseconds.
    pub duration_per_task_ms: f64,
    /// total `session_summaries.estimated_cost`.
    pub model_cost: f64,
    /// `ci_events` rows with `status = 'failed'`.
    pub ci_failures: i64,
    /// `review_findings` rows correlated to the filtered sessions.
    pub review_failures: i64,
    /// §33 drill-down: session ids backing the measures (capped at 100).
    pub session_ids: Vec<String>,
}

/// One §29 per-model row over `model_invocations`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelRow {
    pub model: String,
    pub invocations: i64,
    /// invocations with `status = 'success'` / invocations.
    pub success_rate: f64,
    /// `tokens_in + tokens_out`.
    pub tokens: i64,
    pub avg_latency_ms: f64,
}

/// §29 Models panel: per-model rows plus the panel-wide §29 measures.
/// Routing recommendations are served by `/api/intelligence/proposals`
/// (`RoutingPolicy` proposals), not restated here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelsPanel {
    pub models: Vec<ModelRow>,
    /// `model_fallback` session events / sessions.
    pub fallback_rate: f64,
    /// total `session_summaries.user_interventions`.
    pub user_corrections: i64,
    /// total `session_summaries.review_rework_count`.
    pub reviewer_corrections: i64,
    /// total `session_summaries.estimated_cost`.
    pub cost: f64,
    /// §33 drill-down: session ids (capped at 100).
    pub session_ids: Vec<String>,
}

/// One §30 per-tool row over `tool_invocations`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolRow {
    pub tool_name: String,
    pub invocations: i64,
    /// invocations with `status = 'success'` / invocations.
    pub success_rate: f64,
    /// invocations without `status = 'success'` / invocations.
    pub error_rate: f64,
    pub avg_duration_ms: f64,
}

/// §30 Tools & Skills panel over `tool_invocations`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolsPanel {
    pub invocations: i64,
    pub success_rate: f64,
    pub error_rate: f64,
    /// per-tool rows (§30 "tool-specific failure patterns" = rows with
    /// `error_rate > 0`; "unused capability candidates" = rows with
    /// `success_rate = 0` — invoked, never once successful).
    pub tools: Vec<ToolRow>,
    /// tool names with at least one non-success invocation.
    pub failure_patterns: Vec<String>,
    pub unused_candidates: i64,
    /// §33 drill-down: session ids (capped at 100).
    pub session_ids: Vec<String>,
}

/// §31 Context panel over `session_summaries` and `session_events`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextPanel {
    pub sessions: i64,
    /// sessions with a stored `session_summaries` row.
    pub summarized_sessions: i64,
    pub avg_peak_context_tokens: f64,
    pub files_read: i64,
    pub files_changed: i64,
    /// sessions whose summary read more files than it changed.
    pub unused_read_sessions: i64,
    /// sessions with more than one `file_read` event.
    pub repeated_read_sessions: i64,
    /// `context_compaction` session events.
    pub context_compactions: i64,
    /// `context_limit_warning` session events.
    pub context_exhaustions: i64,
    /// files changed / files read (0.0 when nothing was read).
    pub context_efficiency: f64,
    /// §33 drill-down: session ids (capped at 100).
    pub session_ids: Vec<String>,
}

/// One §32 row per finding taxonomy (`review_findings.category` /
/// `quality_findings.gate`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FindingRow {
    /// `review` or `quality` — the source table.
    pub source: String,
    pub taxonomy: String,
    pub total: i64,
    /// rows with `status = 'active'`.
    pub active: i64,
}

/// §32 Quality panel over `review_findings` and `quality_findings`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QualityPanel {
    pub findings: Vec<FindingRow>,
    /// active review findings whose title occurs in more than one session.
    pub recurring_findings: i64,
    /// total `session_summaries.user_interventions`.
    pub user_corrections: i64,
    /// §33 drill-down: finding ids (capped at 50 per table).
    pub finding_ids: Vec<String>,
}

/// §33 Improvement panel over `improvement_proposals`,
/// `proposal_evaluations`, and `post_change_measurements`, scoped to
/// proposals whose finding is correlated to a filtered session.
/// Status mapping: proposed = `draft`, under evaluation = `evaluated`,
/// open PRs = `approved`, rejected = `rejected`; monitoring = proposals
/// with at least one `post_change_measurements` row; validated = proposals
/// with a `pass` evaluation; regressions = proposals with a `fail`
/// evaluation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImprovementPanel {
    /// `active` rows in both finding tables.
    pub active_findings: i64,
    pub proposed_improvements: i64,
    pub under_evaluation: i64,
    pub open_prs: i64,
    pub monitoring: i64,
    pub validated_improvements: i64,
    pub rejected_improvements: i64,
    pub regressions: i64,
    /// §33 drill-down: active finding ids (capped at 50 per table).
    pub finding_ids: Vec<String>,
    /// §33 drill-down: in-scope proposal ids (capped at 100).
    pub proposal_ids: Vec<String>,
}

/// The six dashboard panels; the serde names are the §43 resource names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PanelKind {
    AgentPerformance,
    Models,
    Tools,
    Context,
    Quality,
    Improvement,
}

/// A serde-serialisable dashboard payload: one variant per panel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PanelPayload {
    AgentPerformance(AgentPerformancePanel),
    Models(ModelsPanel),
    Tools(ToolsPanel),
    Context(ContextPanel),
    Quality(QualityPanel),
    Improvement(ImprovementPanel),
}

/// Fetch one panel aggregate for `filter`. Filter values are bound as
/// parameters ($1-$4), never formatted into the SQL text.
pub async fn panel(
    pool: &AnyPool,
    kind: PanelKind,
    filter: &PanelFilter,
) -> Result<PanelPayload, AutospecError> {
    match kind {
        PanelKind::AgentPerformance => Ok(PanelPayload::AgentPerformance(
            agent_performance(pool, filter).await?,
        )),
        PanelKind::Models => Ok(PanelPayload::Models(models_panel(pool, filter).await?)),
        PanelKind::Tools => Ok(PanelPayload::Tools(tools_panel(pool, filter).await?)),
        PanelKind::Context => Ok(PanelPayload::Context(context_panel(pool, filter).await?)),
        PanelKind::Quality => Ok(PanelPayload::Quality(quality_panel(pool, filter).await?)),
        PanelKind::Improvement => Ok(PanelPayload::Improvement(
            improvement_panel(pool, filter).await?,
        )),
    }
}

// ── implementation ────────────────────────────────────────────────────────
// One aggregate query per panel (§51): every predicate is a scalar
// subquery over the issue-#3827 tables, joined to `sessions` so one shared
// filter applies to all of them.

/// Shared sessions filter: repo ($1), model ($2), since ($3), until ($4).
/// `None` values match everything. Only these constant fragments are ever
/// spliced into the SQL; filter *values* are always bound as parameters.
macro_rules! filter_sql {
    () => {
        "($1 IS NULL OR s.repo = $1) AND ($2 IS NULL OR s.model = $2) \
         AND ($3 IS NULL OR s.started_at >= $3) AND ($4 IS NULL OR s.started_at < $4)"
    };
}

/// Proposal scope: proposals whose finding is correlated to a filtered session.
const FINDING_SCOPE: &str = concat!(
    "p.finding_id IN (SELECT rf.id FROM review_findings rf \
     JOIN sessions s ON s.id = rf.session_id WHERE ",
    filter_sql!(),
    " UNION SELECT qf.id FROM quality_findings qf \
     JOIN sessions s ON s.id = qf.session_id WHERE ",
    filter_sql!(),
    ")"
);

fn improvement_sql() -> String {
    format!(
        "SELECT (SELECT COUNT(*) FROM review_findings rf JOIN sessions s ON s.id = rf.session_id \
         WHERE {} AND rf.status = 'active') \
         + (SELECT COUNT(*) FROM quality_findings qf JOIN sessions s ON s.id = qf.session_id \
         WHERE {} AND qf.status = 'active') AS active_findings, \
         (SELECT COUNT(*) FROM improvement_proposals p WHERE p.status = 'draft' AND {}) AS proposed, \
         (SELECT COUNT(*) FROM improvement_proposals p WHERE p.status = 'evaluated' AND {}) AS under_evaluation, \
         (SELECT COUNT(*) FROM improvement_proposals p WHERE p.status = 'approved' AND {}) AS open_prs, \
         (SELECT COUNT(*) FROM improvement_proposals p WHERE p.status = 'rejected' AND {}) AS rejected, \
         (SELECT COUNT(*) FROM (SELECT p.id FROM improvement_proposals p \
         JOIN post_change_measurements m ON m.proposal_id = p.id WHERE {} GROUP BY p.id) t) AS monitoring, \
         (SELECT COUNT(*) FROM (SELECT p.id FROM improvement_proposals p \
         JOIN proposal_evaluations ev ON ev.proposal_id = p.id AND ev.verdict = 'pass' \
         WHERE {} GROUP BY p.id) t) AS validated, \
         (SELECT COUNT(*) FROM (SELECT p.id FROM improvement_proposals p \
         JOIN proposal_evaluations ev ON ev.proposal_id = p.id AND ev.verdict = 'fail' \
         WHERE {} GROUP BY p.id) t) AS regressions",
        filter_sql!(),
        filter_sql!(),
        FINDING_SCOPE,
        FINDING_SCOPE,
        FINDING_SCOPE,
        FINDING_SCOPE,
        FINDING_SCOPE,
        FINDING_SCOPE,
        FINDING_SCOPE
    )
}

fn proposal_ids_sql() -> String {
    format!(
        "SELECT p.id FROM improvement_proposals p WHERE {FINDING_SCOPE} ORDER BY p.id LIMIT 100"
    )
}

const AGENT_PERFORMANCE_SQL: &str = concat!(
    "SELECT (SELECT COUNT(*) FROM sessions s WHERE ",
    filter_sql!(),
    ") AS sessions, \
     (SELECT COUNT(*) FROM sessions s WHERE ",
    filter_sql!(),
    " AND s.status = 'success') AS successes, \
     (SELECT COUNT(*) FROM sessions s WHERE ",
    filter_sql!(),
    " AND s.status = 'success' AND NOT EXISTS \
        (SELECT 1 FROM session_summaries ss WHERE ss.session_id = s.id \
         AND (ss.user_interventions > 0 OR ss.review_rework_count > 0))) AS first_pass, \
     (SELECT COUNT(*) FROM sessions s WHERE ",
    filter_sql!(),
    " AND EXISTS \
        (SELECT 1 FROM session_summaries ss WHERE ss.session_id = s.id \
         AND ss.review_rework_count > 0)) AS reworked, \
     (SELECT COALESCE(SUM(ss.user_interventions), 0) FROM session_summaries ss \
        JOIN sessions s ON s.id = ss.session_id WHERE ",
    filter_sql!(),
    ") AS interventions, \
     (SELECT COUNT(*) FROM session_events e JOIN sessions s ON s.id = e.session_id \
        WHERE ",
    filter_sql!(),
    ") AS turns, \
     (SELECT COALESCE(SUM(ss.input_tokens + ss.output_tokens), 0) FROM session_summaries ss \
        JOIN sessions s ON s.id = ss.session_id WHERE ",
    filter_sql!(),
    ") AS tokens, \
     (SELECT COALESCE(SUM(ss.duration_seconds), 0) FROM session_summaries ss \
        JOIN sessions s ON s.id = ss.session_id WHERE ",
    filter_sql!(),
    ") AS duration_s, \
     (SELECT COALESCE(SUM(ss.estimated_cost), 0.0) FROM session_summaries ss \
        JOIN sessions s ON s.id = ss.session_id WHERE ",
    filter_sql!(),
    ") AS cost, \
     (SELECT COUNT(*) FROM ci_events c JOIN sessions s ON s.id = c.session_id \
        WHERE ",
    filter_sql!(),
    " AND c.status = 'failed') AS ci_failures, \
     (SELECT COUNT(*) FROM review_findings r JOIN sessions s ON s.id = r.session_id \
        WHERE ",
    filter_sql!(),
    ") AS review_failures"
);

const MODELS_SQL: &str = concat!(
    "SELECT mi.model, COUNT(*) AS invocations, \
     COALESCE(SUM(CASE WHEN mi.status = 'success' THEN 1 ELSE 0 END), 0) AS successes, \
     COALESCE(SUM(mi.tokens_in + mi.tokens_out), 0) AS tokens, \
     COALESCE(SUM(mi.latency_ms), 0) AS latency_ms \
     FROM model_invocations mi JOIN sessions s ON s.id = mi.session_id \
     WHERE ",
    filter_sql!(),
    " GROUP BY mi.model ORDER BY mi.model"
);

const MODELS_AGG_SQL: &str = concat!(
    "SELECT (SELECT COUNT(*) FROM sessions s WHERE ",
    filter_sql!(),
    ") AS sessions, \
     (SELECT COUNT(*) FROM session_events e JOIN sessions s ON s.id = e.session_id \
        WHERE ",
    filter_sql!(),
    " AND e.event_type = 'model_fallback') AS fallbacks, \
     (SELECT COALESCE(SUM(ss.user_interventions), 0) FROM session_summaries ss \
        JOIN sessions s ON s.id = ss.session_id WHERE ",
    filter_sql!(),
    ") AS user_corrections, \
     (SELECT COALESCE(SUM(ss.review_rework_count), 0) FROM session_summaries ss \
        JOIN sessions s ON s.id = ss.session_id WHERE ",
    filter_sql!(),
    ") AS reviewer_corrections, \
     (SELECT COALESCE(SUM(ss.estimated_cost), 0.0) FROM session_summaries ss \
        JOIN sessions s ON s.id = ss.session_id WHERE ",
    filter_sql!(),
    ") AS cost"
);

const TOOLS_SQL: &str = concat!(
    "SELECT ti.tool_name, COUNT(*) AS invocations, \
     COALESCE(SUM(CASE WHEN ti.status = 'success' THEN 1 ELSE 0 END), 0) AS successes, \
     COALESCE(SUM(ti.duration_ms), 0) AS duration_ms \
     FROM tool_invocations ti JOIN sessions s ON s.id = ti.session_id \
     WHERE ",
    filter_sql!(),
    " GROUP BY ti.tool_name ORDER BY ti.tool_name"
);

const CONTEXT_SQL: &str = concat!(
    "SELECT (SELECT COUNT(*) FROM sessions s WHERE ",
    filter_sql!(),
    ") AS sessions, \
     (SELECT COUNT(*) FROM session_summaries ss JOIN sessions s ON s.id = ss.session_id \
        WHERE ",
    filter_sql!(),
    ") AS summarized, \
     (SELECT COALESCE(AVG(ss.context_peak_tokens), 0.0) FROM session_summaries ss \
        JOIN sessions s ON s.id = ss.session_id WHERE ",
    filter_sql!(),
    ") AS avg_peak, \
     (SELECT COALESCE(SUM(ss.files_read), 0) FROM session_summaries ss \
        JOIN sessions s ON s.id = ss.session_id WHERE ",
    filter_sql!(),
    ") AS files_read, \
     (SELECT COALESCE(SUM(ss.files_changed), 0) FROM session_summaries ss \
        JOIN sessions s ON s.id = ss.session_id WHERE ",
    filter_sql!(),
    ") AS files_changed, \
     (SELECT COUNT(*) FROM (SELECT ss.session_id FROM session_summaries ss \
        JOIN sessions s ON s.id = ss.session_id WHERE ",
    filter_sql!(),
    " AND ss.files_read > ss.files_changed) t) \
        AS unused_read_sessions, \
     (SELECT COUNT(*) FROM (SELECT e.session_id FROM session_events e \
        JOIN sessions s ON s.id = e.session_id WHERE ",
    filter_sql!(),
    " AND e.event_type = 'file_read' \
        GROUP BY e.session_id HAVING COUNT(*) > 1) t) AS repeated_read_sessions, \
     (SELECT COUNT(*) FROM session_events e JOIN sessions s ON s.id = e.session_id \
        WHERE ",
    filter_sql!(),
    " AND e.event_type = 'context_compaction') AS context_compactions, \
     (SELECT COUNT(*) FROM session_events e JOIN sessions s ON s.id = e.session_id \
        WHERE ",
    filter_sql!(),
    " AND e.event_type = 'context_limit_warning') AS context_exhaustions"
);

const QUALITY_ROWS_SQL: &str = concat!(
    "SELECT 'review' AS source, rf.category AS taxonomy, COUNT(*) AS total, \
     COALESCE(SUM(CASE WHEN rf.status = 'active' THEN 1 ELSE 0 END), 0) AS active \
     FROM review_findings rf JOIN sessions s ON s.id = rf.session_id WHERE ",
    filter_sql!(),
    " GROUP BY rf.category \
     UNION ALL \
     SELECT 'quality', qf.gate, COUNT(*), \
     COALESCE(SUM(CASE WHEN qf.status = 'active' THEN 1 ELSE 0 END), 0) \
     FROM quality_findings qf JOIN sessions s ON s.id = qf.session_id WHERE ",
    filter_sql!(),
    " GROUP BY qf.gate ORDER BY 1, 2"
);

const QUALITY_AGG_SQL: &str = concat!(
    "SELECT (SELECT COUNT(*) FROM (SELECT rf.title FROM review_findings rf \
        JOIN sessions s ON s.id = rf.session_id WHERE ",
    filter_sql!(),
    " AND rf.status = 'active' \
        GROUP BY rf.title HAVING COUNT(*) > 1) t) AS recurring, \
     (SELECT COALESCE(SUM(ss.user_interventions), 0) FROM session_summaries ss \
        JOIN sessions s ON s.id = ss.session_id WHERE ",
    filter_sql!(),
    ") AS user_corrections"
);

const SESSION_IDS_SQL: &str = concat!(
    "SELECT s.id FROM sessions s WHERE ",
    filter_sql!(),
    " ORDER BY s.id LIMIT 100"
);

/// Finding-id drill-down query for one table; SQLite cannot use a
/// parenthesized compound SELECT in a derived table, so the two finding
/// tables are fetched separately and concatenated in Rust.
fn finding_ids_sql(table: &str, alias: &str, active_only: bool) -> String {
    let extra = if active_only {
        format!(" AND {alias}.status = 'active'")
    } else {
        String::new()
    };
    format!("SELECT {alias}.id FROM {table} {alias} JOIN sessions s ON s.id = {alias}.session_id WHERE {f} {extra} ORDER BY {alias}.id LIMIT 50", f = filter_sql!())
}

async fn finding_ids(
    pool: &AnyPool,
    f: &PanelFilter,
    table: &str,
    alias: &str,
    active_only: bool,
) -> Result<Vec<String>, AutospecError> {
    let rows = bind(sqlx::query(&finding_ids_sql(table, alias, active_only)), f)
        .fetch_all(pool)
        .await
        .map_err(|e| state(table, e))?;
    rows.into_iter()
        .map(|r| r.try_get::<String, _>(0).map_err(|e| state(table, e)))
        .collect()
}

fn state(entity: &str, err: sqlx::Error) -> AutospecError {
    AutospecError::State {
        entity: entity.to_string(),
        message: err.to_string(),
    }
}

fn bind<'q>(
    q: sqlx::query::Query<'q, sqlx::Any, sqlx::any::AnyArguments<'q>>,
    f: &'q PanelFilter,
) -> sqlx::query::Query<'q, sqlx::Any, sqlx::any::AnyArguments<'q>> {
    q.bind(f.repo.as_deref())
        .bind(f.model.as_deref())
        .bind(f.since.as_deref())
        .bind(f.until.as_deref())
}

async fn session_ids(pool: &AnyPool, f: &PanelFilter) -> Result<Vec<String>, AutospecError> {
    let rows = bind(sqlx::query(SESSION_IDS_SQL), f)
        .fetch_all(pool)
        .await
        .map_err(|e| state("sessions", e))?;
    rows.into_iter()
        .map(|r| {
            r.try_get::<String, _>("id")
                .map_err(|e| state("sessions", e))
        })
        .collect()
}

async fn agent_performance(
    pool: &AnyPool,
    f: &PanelFilter,
) -> Result<AgentPerformancePanel, AutospecError> {
    let row = bind(sqlx::query(AGENT_PERFORMANCE_SQL), f)
        .fetch_one(pool)
        .await
        .map_err(|e| state("agent_performance", e))?;
    let sessions = row.get::<i64, _>("sessions");
    let per_session = |n: i64| {
        if sessions > 0 {
            n as f64 / sessions as f64
        } else {
            0.0
        }
    };
    Ok(AgentPerformancePanel {
        sessions,
        task_success_rate: per_session(row.get::<i64, _>("successes")),
        first_pass_success: per_session(row.get::<i64, _>("first_pass")),
        rework_rate: per_session(row.get::<i64, _>("reworked")),
        user_interventions: row.get::<i64, _>("interventions"),
        turns_per_task: per_session(row.get::<i64, _>("turns")),
        tokens_per_task: per_session(row.get::<i64, _>("tokens")),
        duration_per_task_ms: per_session(row.get::<i64, _>("duration_s")) * 1000.0,
        model_cost: row.get::<f64, _>("cost"),
        ci_failures: row.get::<i64, _>("ci_failures"),
        review_failures: row.get::<i64, _>("review_failures"),
        session_ids: session_ids(pool, f).await?,
    })
}

async fn models_panel(pool: &AnyPool, f: &PanelFilter) -> Result<ModelsPanel, AutospecError> {
    let rows = bind(sqlx::query(MODELS_SQL), f)
        .fetch_all(pool)
        .await
        .map_err(|e| state("model_invocations", e))?;
    let models = rows
        .into_iter()
        .map(|r| {
            let invocations = r.get::<i64, _>("invocations");
            let successes = r.get::<i64, _>("successes");
            let latency = r.get::<i64, _>("latency_ms");
            Ok(ModelRow {
                model: r.get("model"),
                invocations,
                success_rate: if invocations > 0 {
                    successes as f64 / invocations as f64
                } else {
                    0.0
                },
                tokens: r.get::<i64, _>("tokens"),
                avg_latency_ms: if invocations > 0 {
                    latency as f64 / invocations as f64
                } else {
                    0.0
                },
            })
        })
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| state("model_invocations", e))?;
    let agg = bind(sqlx::query(MODELS_AGG_SQL), f)
        .fetch_one(pool)
        .await
        .map_err(|e| state("models_panel", e))?;
    let sessions = agg.get::<i64, _>("sessions");
    Ok(ModelsPanel {
        models,
        fallback_rate: if sessions > 0 {
            agg.get::<i64, _>("fallbacks") as f64 / sessions as f64
        } else {
            0.0
        },
        user_corrections: agg.get::<i64, _>("user_corrections"),
        reviewer_corrections: agg.get::<i64, _>("reviewer_corrections"),
        cost: agg.get::<f64, _>("cost"),
        session_ids: session_ids(pool, f).await?,
    })
}

async fn tools_panel(pool: &AnyPool, f: &PanelFilter) -> Result<ToolsPanel, AutospecError> {
    let rows = bind(sqlx::query(TOOLS_SQL), f)
        .fetch_all(pool)
        .await
        .map_err(|e| state("tool_invocations", e))?;
    let mut tools = Vec::with_capacity(rows.len());
    let mut invocations = 0i64;
    let mut successes = 0i64;
    for r in rows {
        let n = r.get::<i64, _>("invocations");
        let ok = r.get::<i64, _>("successes");
        invocations += n;
        successes += ok;
        tools.push(ToolRow {
            tool_name: r.get("tool_name"),
            invocations: n,
            success_rate: if n > 0 { ok as f64 / n as f64 } else { 0.0 },
            error_rate: if n > 0 {
                (n - ok) as f64 / n as f64
            } else {
                0.0
            },
            avg_duration_ms: {
                let d = r.get::<i64, _>("duration_ms");
                if n > 0 {
                    d as f64 / n as f64
                } else {
                    0.0
                }
            },
        });
    }
    let failure_patterns: Vec<String> = tools
        .iter()
        .filter(|t| t.error_rate > 0.0)
        .map(|t| t.tool_name.clone())
        .collect();
    let unused = tools
        .iter()
        .filter(|t| t.invocations > 0 && t.success_rate == 0.0)
        .count();
    Ok(ToolsPanel {
        success_rate: if invocations > 0 {
            successes as f64 / invocations as f64
        } else {
            0.0
        },
        error_rate: if invocations > 0 {
            (invocations - successes) as f64 / invocations as f64
        } else {
            0.0
        },
        invocations,
        tools,
        failure_patterns,
        unused_candidates: unused as i64,
        session_ids: session_ids(pool, f).await?,
    })
}

async fn context_panel(pool: &AnyPool, f: &PanelFilter) -> Result<ContextPanel, AutospecError> {
    let row = bind(sqlx::query(CONTEXT_SQL), f)
        .fetch_one(pool)
        .await
        .map_err(|e| state("context_panel", e))?;
    let files_read = row.get::<i64, _>("files_read");
    let files_changed = row.get::<i64, _>("files_changed");
    Ok(ContextPanel {
        sessions: row.get("sessions"),
        summarized_sessions: row.get("summarized"),
        avg_peak_context_tokens: row.get("avg_peak"),
        files_read,
        files_changed,
        unused_read_sessions: row.get("unused_read_sessions"),
        repeated_read_sessions: row.get("repeated_read_sessions"),
        context_compactions: row.get("context_compactions"),
        context_exhaustions: row.get("context_exhaustions"),
        context_efficiency: if files_read > 0 {
            files_changed as f64 / files_read as f64
        } else {
            0.0
        },
        session_ids: session_ids(pool, f).await?,
    })
}

async fn quality_panel(pool: &AnyPool, f: &PanelFilter) -> Result<QualityPanel, AutospecError> {
    let rows = bind(sqlx::query(QUALITY_ROWS_SQL), f)
        .fetch_all(pool)
        .await
        .map_err(|e| state("quality_panel", e))?;
    let findings = rows
        .into_iter()
        .map(|r| {
            Ok(FindingRow {
                source: r.get("source"),
                taxonomy: r.get("taxonomy"),
                total: r.get("total"),
                active: r.get("active"),
            })
        })
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| state("quality_panel", e))?;
    let agg = bind(sqlx::query(QUALITY_AGG_SQL), f)
        .fetch_one(pool)
        .await
        .map_err(|e| state("quality_panel", e))?;
    let mut ids = finding_ids(pool, f, "review_findings", "rf", false).await?;
    ids.extend(finding_ids(pool, f, "quality_findings", "qf", false).await?);
    Ok(QualityPanel {
        findings,
        recurring_findings: agg.get("recurring"),
        user_corrections: agg.get("user_corrections"),
        finding_ids: ids,
    })
}

async fn improvement_panel(
    pool: &AnyPool,
    f: &PanelFilter,
) -> Result<ImprovementPanel, AutospecError> {
    let row = bind(sqlx::query(&improvement_sql()), f)
        .fetch_one(pool)
        .await
        .map_err(|e| state("improvement_panel", e))?;
    let p_rows = bind(sqlx::query(&proposal_ids_sql()), f)
        .fetch_all(pool)
        .await
        .map_err(|e| state("improvement_panel", e))?;
    let proposal_ids = p_rows
        .into_iter()
        .map(|r| {
            r.try_get::<String, _>(0)
                .map_err(|e| state("improvement_panel", e))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut ids = finding_ids(pool, f, "review_findings", "rf", true).await?;
    ids.extend(finding_ids(pool, f, "quality_findings", "qf", true).await?);
    Ok(ImprovementPanel {
        active_findings: row.get("active_findings"),
        proposed_improvements: row.get("proposed"),
        under_evaluation: row.get("under_evaluation"),
        open_prs: row.get("open_prs"),
        monitoring: row.get("monitoring"),
        validated_improvements: row.get("validated"),
        rejected_improvements: row.get("rejected"),
        regressions: row.get("regressions"),
        finding_ids: ids,
        proposal_ids,
    })
}

// ── tests ─────────────────────────────────────────────────────────────────
// Real databases only, no mocks: a dedicated `report_*` database on the
// disposable `AUTOSPEC_TEST_DB_URL` PostgreSQL 16 server (dedicated so the
// sibling test files' same-named tables cannot collide), or a throwaway
// SQLite file otherwise.

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::Executor;
    use std::sync::atomic::{AtomicU32, Ordering};

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    /// The issue-#3827 tables the panels read; time columns as TEXT so the
    /// ISO-8601 window compares lexicographically on both backends (the
    /// `insights::models` / `insights::tools` test convention).
    const SCHEMA: &str = "
        DROP TABLE IF EXISTS post_change_measurements;
        DROP TABLE IF EXISTS proposal_evaluations;
        DROP TABLE IF EXISTS improvement_proposals;
        DROP TABLE IF EXISTS quality_findings;
        DROP TABLE IF EXISTS review_findings;
        DROP TABLE IF EXISTS ci_events;
        DROP TABLE IF EXISTS model_invocations;
        DROP TABLE IF EXISTS tool_invocations;
        DROP TABLE IF EXISTS session_summaries;
        DROP TABLE IF EXISTS session_events;
        DROP TABLE IF EXISTS sessions;
        CREATE TABLE sessions (id TEXT PRIMARY KEY, repo TEXT NOT NULL, work_item_id TEXT,
            harness TEXT, model TEXT, status TEXT NOT NULL,
            started_at TEXT NOT NULL DEFAULT '2026-09-08T18:00:00Z', ended_at TEXT,
            created_at TEXT NOT NULL DEFAULT '2026-09-08T18:00:00Z');
        CREATE TABLE session_events (session_id TEXT NOT NULL, seq INTEGER NOT NULL,
            event_type TEXT NOT NULL, occurred_at TEXT NOT NULL DEFAULT '2026-09-08T18:00:00Z',
            payload TEXT, PRIMARY KEY (session_id, seq));
        CREATE TABLE session_summaries (session_id TEXT PRIMARY KEY, task_type TEXT,
            task_domain TEXT NOT NULL, outcome TEXT, autonomy_score REAL NOT NULL,
            user_interventions INTEGER NOT NULL, review_rework_count INTEGER NOT NULL,
            tool_calls INTEGER NOT NULL, tool_errors INTEGER NOT NULL, files_read INTEGER NOT NULL,
            files_changed INTEGER NOT NULL, tests_run INTEGER NOT NULL,
            context_peak_tokens INTEGER NOT NULL, input_tokens INTEGER NOT NULL,
            output_tokens INTEGER NOT NULL, estimated_cost REAL NOT NULL,
            duration_seconds INTEGER NOT NULL, models TEXT NOT NULL, commits TEXT NOT NULL,
            pull_requests TEXT NOT NULL, extractor_version TEXT NOT NULL);
        CREATE TABLE tool_invocations (session_id TEXT NOT NULL, seq INTEGER NOT NULL,
            tool_name TEXT NOT NULL, args_summary TEXT, status TEXT NOT NULL,
            duration_ms INTEGER, occurred_at TEXT NOT NULL DEFAULT '2026-09-08T18:00:00Z',
            PRIMARY KEY (session_id, seq));
        CREATE TABLE model_invocations (session_id TEXT NOT NULL, seq INTEGER NOT NULL,
            model TEXT NOT NULL, tokens_in INTEGER, tokens_out INTEGER, latency_ms INTEGER,
            status TEXT NOT NULL, occurred_at TEXT NOT NULL DEFAULT '2026-09-08T18:00:00Z',
            PRIMARY KEY (session_id, seq));
        CREATE TABLE ci_events (session_id TEXT NOT NULL, seq INTEGER NOT NULL, repo TEXT NOT NULL,
            run_id TEXT NOT NULL, status TEXT NOT NULL,
            occurred_at TEXT NOT NULL DEFAULT '2026-09-08T18:00:00Z', payload TEXT,
            PRIMARY KEY (session_id, seq));
        CREATE TABLE review_findings (id TEXT PRIMARY KEY, session_id TEXT NOT NULL,
            work_item_id TEXT, repo TEXT, category TEXT NOT NULL, severity TEXT,
            title TEXT NOT NULL, description TEXT, status TEXT NOT NULL,
            created_at TEXT NOT NULL DEFAULT '2026-09-08T18:00:00Z');
        CREATE TABLE quality_findings (id TEXT PRIMARY KEY, session_id TEXT NOT NULL,
            work_item_id TEXT, repo TEXT, gate TEXT NOT NULL, severity TEXT, title TEXT NOT NULL,
            description TEXT, status TEXT NOT NULL,
            created_at TEXT NOT NULL DEFAULT '2026-09-08T18:00:00Z');
        CREATE TABLE improvement_proposals (id TEXT PRIMARY KEY, finding_id TEXT NOT NULL,
            title TEXT NOT NULL, body TEXT, target TEXT, status TEXT NOT NULL,
            created_at TEXT NOT NULL DEFAULT '2026-09-08T18:00:00Z',
            updated_at TEXT NOT NULL DEFAULT '2026-09-08T18:00:00Z');
        CREATE TABLE proposal_evaluations (proposal_id TEXT NOT NULL, seq INTEGER NOT NULL,
            evaluator TEXT NOT NULL, verdict TEXT NOT NULL, rationale TEXT,
            created_at TEXT NOT NULL DEFAULT '2026-09-08T18:00:00Z',
            PRIMARY KEY (proposal_id, seq));
        CREATE TABLE post_change_measurements (id TEXT PRIMARY KEY, proposal_id TEXT, repo TEXT,
            work_item_id TEXT, metric TEXT NOT NULL, before_value REAL, after_value REAL,
            measured_at TEXT NOT NULL DEFAULT '2026-09-08T18:00:00Z');";

    /// Open the panel test pool: `(pool, admin_url, dedicated_db)`;
    /// `dedicated_db` is empty for the SQLite fallback.
    async fn open_pool() -> (AnyPool, String, String) {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let pid = std::process::id();
        match std::env::var("AUTOSPEC_TEST_DB_URL") {
            Ok(url) if url.starts_with("postgres") && !url.trim().is_empty() => {
                let db = format!("report_{pid}_{n}");
                let admin = crate::resources::db::open_shared_db(&url).await.unwrap();
                let ddl = format!("CREATE DATABASE {db}");
                sqlx::query(&ddl).execute(&admin).await.unwrap();
                let (base, _) = url.rsplit_once('/').unwrap();
                let pool = crate::resources::db::open_shared_db(&format!("{base}/{db}"))
                    .await
                    .unwrap();
                (pool, url, db)
            }
            _ => (
                crate::resources::db::open_shared_db(&format!(
                    "sqlite://{}/autospec-insights-report-{pid}-{n}.db",
                    std::env::temp_dir().display()
                ))
                .await
                .unwrap(),
                String::new(),
                String::new(),
            ),
        }
    }

    async fn close_pool(pool: AnyPool, admin_url: &str, db: &str) {
        drop(pool);
        if db.is_empty() || admin_url.is_empty() {
            return;
        }
        if let Ok(admin) = crate::resources::db::open_shared_db(admin_url).await {
            let ddl = format!("DROP DATABASE IF EXISTS {db} WITH (FORCE)");
            let _ = sqlx::query(&ddl).execute(&admin).await;
        }
    }

    async fn seed(pool: &AnyPool) {
        // The repo name carries a quote: the quote-bearing filter test and
        // the totals test share one seed.
        for (id, repo, model, status, at) in [
            (
                "s1",
                "acme'widgets",
                "qwen3",
                "success",
                "2026-09-08T10:00:00Z",
            ),
            (
                "s2",
                "acme'widgets",
                "codex5",
                "failed",
                "2026-09-08T11:00:00Z",
            ),
            (
                "s3",
                "acme'widgets",
                "qwen3",
                "success",
                "2026-09-09T10:00:00Z",
            ),
            ("s4", "other", "qwen3", "success", "2026-09-08T10:00:00Z"),
        ] {
            sqlx::query(
                "INSERT INTO sessions (id, repo, model, status, started_at) \
                         VALUES ($1, $2, $3, $4, $5)",
            )
            .bind(id)
            .bind(repo)
            .bind(model)
            .bind(status)
            .bind(at)
            .execute(pool)
            .await
            .unwrap();
        }
        // s1: clean first pass; s2: one intervention, two review reworks.
        let sum = "INSERT INTO session_summaries (session_id, task_domain, autonomy_score, \
                   user_interventions, review_rework_count, tool_calls, tool_errors, files_read, \
                   files_changed, tests_run, context_peak_tokens, input_tokens, output_tokens, \
                   estimated_cost, duration_seconds, models, commits, pull_requests, \
                   extractor_version) VALUES ($1, '[]', 1.0, $2, $3, 0, 0, $4, $5, 0, $6, $7, \
                   $8, $9, $10, '[]', '[]', '[]', '1')";
        for (id, iv, rw, fr, fc, peak, tin, tout, cost, dur) in [
            (
                "s1", 0i64, 0i64, 10i64, 4i64, 1000i64, 100i64, 50i64, 0.5f64, 10i64,
            ),
            (
                "s2", 1i64, 2i64, 5i64, 5i64, 2000i64, 200i64, 100i64, 1.5f64, 20i64,
            ),
        ] {
            sqlx::query(sum)
                .bind(id)
                .bind(iv)
                .bind(rw)
                .bind(fr)
                .bind(fc)
                .bind(peak)
                .bind(tin)
                .bind(tout)
                .bind(cost)
                .bind(dur)
                .execute(pool)
                .await
                .unwrap();
        }
        for (sid, seq, kind) in [
            ("s1", 1i64, "file_read"),
            ("s1", 2i64, "file_read"),
            ("s1", 3i64, "context_compaction"),
            ("s2", 1i64, "file_read"),
            ("s2", 2i64, "model_fallback"),
            ("s2", 3i64, "context_limit_warning"),
        ] {
            sqlx::query(
                "INSERT INTO session_events (session_id, seq, event_type) \
                         VALUES ($1, $2, $3)",
            )
            .bind(sid)
            .bind(seq)
            .bind(kind)
            .execute(pool)
            .await
            .unwrap();
        }
        for (sid, seq, model, tin, tout, lat, status) in [
            ("s1", 1i64, "qwen3", 100i64, 50i64, 1000i64, "success"),
            ("s1", 2i64, "qwen3", 10i64, 5i64, 500i64, "success"),
            ("s2", 1i64, "codex5", 200i64, 100i64, 2000i64, "failed"),
        ] {
            sqlx::query(
                "INSERT INTO model_invocations (session_id, seq, model, tokens_in, \
                         tokens_out, latency_ms, status) VALUES ($1, $2, $3, $4, $5, $6, $7)",
            )
            .bind(sid)
            .bind(seq)
            .bind(model)
            .bind(tin)
            .bind(tout)
            .bind(lat)
            .bind(status)
            .execute(pool)
            .await
            .unwrap();
        }
        for (sid, seq, tool, status, dur) in [
            ("s1", 1i64, "read", "success", 10i64),
            ("s1", 2i64, "bash", "success", 20i64),
            ("s1", 3i64, "bash", "failed", 30i64),
            ("s2", 1i64, "grep", "failed", 40i64),
        ] {
            sqlx::query(
                "INSERT INTO tool_invocations (session_id, seq, tool_name, status, \
                         duration_ms) VALUES ($1, $2, $3, $4, $5)",
            )
            .bind(sid)
            .bind(seq)
            .bind(tool)
            .bind(status)
            .bind(dur)
            .execute(pool)
            .await
            .unwrap();
        }
        for (sid, seq, run, status) in [
            ("s1", 1i64, "run-1", "success"),
            ("s1", 2i64, "run-2", "failed"),
            ("s2", 1i64, "run-3", "failed"),
        ] {
            sqlx::query(
                "INSERT INTO ci_events (session_id, seq, repo, run_id, status) \
                         VALUES ($1, $2, 'r', $3, $4)",
            )
            .bind(sid)
            .bind(seq)
            .bind(run)
            .bind(status)
            .execute(pool)
            .await
            .unwrap();
        }
        for (id, sid, cat, title, status) in [
            ("rf-1", "s1", "linter", "clippy::x", "active"),
            ("rf-2", "s2", "linter", "clippy::x", "active"),
            ("rf-3", "s2", "reviewer_feedback", "t", "quarantined"),
        ] {
            sqlx::query(
                "INSERT INTO review_findings (id, session_id, category, title, status) \
                         VALUES ($1, $2, $3, $4, $5)",
            )
            .bind(id)
            .bind(sid)
            .bind(cat)
            .bind(title)
            .bind(status)
            .execute(pool)
            .await
            .unwrap();
        }
        sqlx::query(
            "INSERT INTO quality_findings (id, session_id, gate, title, status) \
                     VALUES ('qf-1', 's1', 'complexity_scanner', 'c', 'active')",
        )
        .execute(pool)
        .await
        .unwrap();
        for (id, fid, status) in [
            ("p-1", "rf-1", "draft"),
            ("p-2", "rf-2", "evaluated"),
            ("p-3", "qf-1", "approved"),
            ("p-4", "rf-3", "rejected"),
        ] {
            sqlx::query(
                "INSERT INTO improvement_proposals (id, finding_id, title, status) \
                         VALUES ($1, $2, 't', $3)",
            )
            .bind(id)
            .bind(fid)
            .bind(status)
            .execute(pool)
            .await
            .unwrap();
        }
        for (pid, seq, verdict) in [("p-2", 1i64, "pass"), ("p-3", 1i64, "fail")] {
            sqlx::query(
                "INSERT INTO proposal_evaluations (proposal_id, seq, evaluator, verdict) \
                         VALUES ($1, $2, 'e', $3)",
            )
            .bind(pid)
            .bind(seq)
            .bind(verdict)
            .execute(pool)
            .await
            .unwrap();
        }
        sqlx::query(
            "INSERT INTO post_change_measurements (id, proposal_id, metric, before_value, \
                     after_value) VALUES ('m-1', 'p-3', 'latency', 1.0, 2.0)",
        )
        .execute(pool)
        .await
        .unwrap();
    }

    fn day1(repo: &str) -> PanelFilter {
        PanelFilter {
            repo: Some(repo.to_string()),
            since: Some("2026-09-08T00:00:00Z".to_string()),
            until: Some("2026-09-09T00:00:00Z".to_string()),
            ..Default::default()
        }
    }

    async fn seeded_pool() -> (AnyPool, String, String) {
        let (pool, url, db) = open_pool().await;
        pool.execute(SCHEMA).await.unwrap();
        seed(&pool).await;
        (pool, url, db)
    }

    fn agent(p: PanelPayload) -> AgentPerformancePanel {
        match p {
            PanelPayload::AgentPerformance(p) => p,
            other => panic!("expected agent_performance panel, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn panel_shapes_are_serde_payloads() {
        let payloads = [
            PanelPayload::AgentPerformance(AgentPerformancePanel {
                sessions: 2,
                task_success_rate: 0.5,
                first_pass_success: 0.5,
                rework_rate: 0.5,
                user_interventions: 1,
                turns_per_task: 3.0,
                tokens_per_task: 225.0,
                duration_per_task_ms: 15000.0,
                model_cost: 2.0,
                ci_failures: 2,
                review_failures: 3,
                session_ids: vec!["s1".into()],
            }),
            PanelPayload::Models(ModelsPanel {
                models: vec![ModelRow {
                    model: "qwen3".into(),
                    invocations: 2,
                    success_rate: 1.0,
                    tokens: 165,
                    avg_latency_ms: 750.0,
                }],
                fallback_rate: 0.5,
                user_corrections: 1,
                reviewer_corrections: 2,
                cost: 2.0,
                session_ids: vec!["s1".into()],
            }),
            PanelPayload::Tools(ToolsPanel {
                invocations: 4,
                success_rate: 0.5,
                error_rate: 0.5,
                tools: vec![ToolRow {
                    tool_name: "bash".into(),
                    invocations: 2,
                    success_rate: 0.5,
                    error_rate: 0.5,
                    avg_duration_ms: 25.0,
                }],
                failure_patterns: vec!["bash".into()],
                unused_candidates: 1,
                session_ids: vec!["s1".into()],
            }),
            PanelPayload::Context(ContextPanel {
                sessions: 2,
                summarized_sessions: 2,
                avg_peak_context_tokens: 1500.0,
                files_read: 15,
                files_changed: 9,
                unused_read_sessions: 1,
                repeated_read_sessions: 1,
                context_compactions: 1,
                context_exhaustions: 1,
                context_efficiency: 0.6,
                session_ids: vec!["s1".into()],
            }),
            PanelPayload::Quality(QualityPanel {
                findings: vec![FindingRow {
                    source: "review".into(),
                    taxonomy: "linter".into(),
                    total: 2,
                    active: 2,
                }],
                recurring_findings: 1,
                user_corrections: 1,
                finding_ids: vec!["rf-1".into()],
            }),
            PanelPayload::Improvement(ImprovementPanel {
                active_findings: 3,
                proposed_improvements: 1,
                under_evaluation: 1,
                open_prs: 1,
                monitoring: 1,
                validated_improvements: 1,
                rejected_improvements: 1,
                regressions: 1,
                finding_ids: vec!["rf-1".into()],
                proposal_ids: vec!["p-1".into()],
            }),
        ];
        for p in &payloads {
            let value = serde_json::to_value(p).unwrap();
            let back: PanelPayload = serde_json::from_value(value).unwrap();
            assert_eq!(&back, p);
        }
        // §43 resource names on the wire.
        let value = serde_json::to_value(&payloads).unwrap();
        let names: Vec<String> = value
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p.as_object().unwrap().keys().next().unwrap().clone())
            .collect();
        assert_eq!(
            names,
            vec![
                "agent_performance",
                "models",
                "tools",
                "context",
                "quality",
                "improvement"
            ]
        );
    }

    #[tokio::test]
    async fn panels_reconcile_with_seeded_rows() {
        let (pool, url, db) = seeded_pool().await;
        let f = day1("acme'widgets");

        let p = agent(panel(&pool, PanelKind::AgentPerformance, &f).await.unwrap());
        assert_eq!(p.sessions, 2);
        assert_eq!(p.task_success_rate, 0.5);
        assert_eq!(p.first_pass_success, 0.5);
        assert_eq!(p.rework_rate, 0.5);
        assert_eq!(p.user_interventions, 1);
        assert_eq!(p.turns_per_task, 3.0);
        assert_eq!(p.tokens_per_task, 225.0);
        assert_eq!(p.duration_per_task_ms, 15000.0);
        assert_eq!(p.model_cost, 2.0);
        assert_eq!(p.ci_failures, 2);
        assert_eq!(p.review_failures, 3);
        assert_eq!(p.session_ids, vec!["s1".to_string(), "s2".to_string()]);

        match panel(&pool, PanelKind::Models, &f).await.unwrap() {
            PanelPayload::Models(p) => {
                assert_eq!(
                    p.models,
                    vec![
                        ModelRow {
                            model: "codex5".into(),
                            invocations: 1,
                            success_rate: 0.0,
                            tokens: 300,
                            avg_latency_ms: 2000.0
                        },
                        ModelRow {
                            model: "qwen3".into(),
                            invocations: 2,
                            success_rate: 1.0,
                            tokens: 165,
                            avg_latency_ms: 750.0
                        },
                    ]
                );
                assert_eq!(p.fallback_rate, 0.5);
                assert_eq!(p.user_corrections, 1);
                assert_eq!(p.reviewer_corrections, 2);
                assert_eq!(p.cost, 2.0);
                assert_eq!(p.session_ids.len(), 2);
            }
            other => panic!("expected models panel, got {other:?}"),
        }

        match panel(&pool, PanelKind::Tools, &f).await.unwrap() {
            PanelPayload::Tools(p) => {
                assert_eq!(p.invocations, 4);
                assert_eq!(p.success_rate, 0.5);
                assert_eq!(p.error_rate, 0.5);
                assert_eq!(p.tools.len(), 3);
                assert_eq!(
                    p.failure_patterns,
                    vec!["bash".to_string(), "grep".to_string()]
                );
                assert_eq!(p.unused_candidates, 1);
                assert_eq!(p.session_ids.len(), 2);
            }
            other => panic!("expected tools panel, got {other:?}"),
        }

        match panel(&pool, PanelKind::Context, &f).await.unwrap() {
            PanelPayload::Context(p) => {
                assert_eq!(p.sessions, 2);
                assert_eq!(p.summarized_sessions, 2);
                assert_eq!(p.avg_peak_context_tokens, 1500.0);
                assert_eq!(p.files_read, 15);
                assert_eq!(p.files_changed, 9);
                assert_eq!(p.unused_read_sessions, 1);
                assert_eq!(p.repeated_read_sessions, 1);
                assert_eq!(p.context_compactions, 1);
                assert_eq!(p.context_exhaustions, 1);
                assert!((p.context_efficiency - 9.0 / 15.0).abs() < f64::EPSILON);
                assert_eq!(p.session_ids.len(), 2);
            }
            other => panic!("expected context panel, got {other:?}"),
        }

        match panel(&pool, PanelKind::Quality, &f).await.unwrap() {
            PanelPayload::Quality(p) => {
                assert_eq!(
                    p.findings,
                    vec![
                        FindingRow {
                            source: "quality".into(),
                            taxonomy: "complexity_scanner".into(),
                            total: 1,
                            active: 1
                        },
                        FindingRow {
                            source: "review".into(),
                            taxonomy: "linter".into(),
                            total: 2,
                            active: 2
                        },
                        FindingRow {
                            source: "review".into(),
                            taxonomy: "reviewer_feedback".into(),
                            total: 1,
                            active: 0
                        },
                    ]
                );
                assert_eq!(p.recurring_findings, 1);
                assert_eq!(p.user_corrections, 1);
                assert_eq!(p.finding_ids.len(), 4);
            }
            other => panic!("expected quality panel, got {other:?}"),
        }

        match panel(&pool, PanelKind::Improvement, &f).await.unwrap() {
            PanelPayload::Improvement(p) => {
                assert_eq!(p.active_findings, 3);
                assert_eq!(p.proposed_improvements, 1);
                assert_eq!(p.under_evaluation, 1);
                assert_eq!(p.open_prs, 1);
                assert_eq!(p.monitoring, 1);
                assert_eq!(p.validated_improvements, 1);
                assert_eq!(p.rejected_improvements, 1);
                assert_eq!(p.regressions, 1);
                assert_eq!(p.finding_ids.len(), 3);
                assert_eq!(p.proposal_ids.len(), 4);
            }
            other => panic!("expected improvement panel, got {other:?}"),
        }

        // repo, model, and time filters each narrow the aggregate.
        let only_model = PanelFilter {
            model: Some("codex5".into()),
            ..f.clone()
        };
        assert_eq!(
            agent(
                panel(&pool, PanelKind::AgentPerformance, &only_model)
                    .await
                    .unwrap()
            )
            .sessions,
            1
        );
        let other_repo = day1("other");
        assert_eq!(
            agent(
                panel(&pool, PanelKind::AgentPerformance, &other_repo)
                    .await
                    .unwrap()
            )
            .sessions,
            1
        );
        let day2 = PanelFilter {
            repo: Some("acme'widgets".into()),
            since: Some("2026-09-09T00:00:00Z".into()),
            ..Default::default()
        };
        assert_eq!(
            agent(
                panel(&pool, PanelKind::AgentPerformance, &day2)
                    .await
                    .unwrap()
            )
            .sessions,
            1
        );
        let morning = PanelFilter {
            repo: Some("acme'widgets".into()),
            until: Some("2026-09-08T11:00:00Z".into()),
            ..Default::default()
        };
        assert_eq!(
            agent(
                panel(&pool, PanelKind::AgentPerformance, &morning)
                    .await
                    .unwrap()
            )
            .sessions,
            1
        );

        close_pool(pool, &url, &db).await;
    }

    #[tokio::test]
    async fn quote_bearing_repo_filter_binds_safely() {
        let (pool, url, db) = seeded_pool().await;
        // A quote in the value still selects its own rows as a bound
        // parameter — it never reaches the SQL text.
        let quoted = PanelFilter {
            repo: Some("acme'widgets".into()),
            ..Default::default()
        };
        let p = agent(
            panel(&pool, PanelKind::AgentPerformance, &quoted)
                .await
                .unwrap(),
        );
        assert_eq!(p.sessions, 3);
        assert_eq!(
            p.session_ids,
            vec!["s1".to_string(), "s2".to_string(), "s3".to_string()]
        );
        // An injection-shaped value matches nothing instead of rewriting
        // the predicate.
        let injection = PanelFilter {
            repo: Some("x' OR 1=1 --".into()),
            ..Default::default()
        };
        let p = agent(
            panel(&pool, PanelKind::AgentPerformance, &injection)
                .await
                .unwrap(),
        );
        assert_eq!(p.sessions, 0);
        assert!(p.session_ids.is_empty());
        close_pool(pool, &url, &db).await;
    }
}
