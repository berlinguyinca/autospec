//! Model performance analytics (§17) and recommendation-only routing
//! advice (§18) for the continuous-improvement engine
//! (`docs/specs/2026-09-08-continuous-improvement-engine.md`).
//!
//! [`model_performance`] aggregates the `model_invocations` table (issue
//! #3827) per model and task dimension. Expected columns (all NOT NULL;
//! booleans stored as 0/1 integers so the group-by SQL stays portable
//! across the shared SQLite and Postgres backends):
//!
//! ```text
//! model TEXT, task_class TEXT, started_at TEXT (ISO-8601 UTC),
//! succeeded, first_pass, rejected, re_steers, tokens_in, tokens_out,
//! elapsed_ms, tool_errors, reworked, ci_failed, cost_cents (INTEGER)
//! ```
//!
//! §18 is recommendation-only: [`recommend`] output is recorded evidence,
//! never applied to live routing. No provider pricing or keys are read or
//! emitted; cost is the plain sum of the stored `cost_cents` rows.

use crate::error::AutospecError;
use sqlx::AnyPool;
use sqlx::Row;

/// The 9 task dimensions of §17.
pub const TASK_DIMENSIONS: [&str; 9] = [
    "plan",
    "implement",
    "review",
    "test_generation",
    "documentation",
    "ui_ux",
    "architecture",
    "debugging",
    "repository_exploration",
];

/// Confidence in a routing recommendation. A task class below the minimum
/// sample threshold still emits advice, but at [`RecommendationConfidence::Low`]
/// so consumers know it must not be trusted outright (spec §18).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecommendationConfidence {
    Low,
    High,
}

/// One (model, task_class) aggregate over a window: the 10 §17 metrics.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelPerformance {
    pub model: String,
    pub task_class: String,
    pub samples: i64,
    pub success_rate: f64,
    pub first_pass_success: f64,
    pub rejection_rate: f64,
    pub re_steers: i64,
    pub tokens: i64,
    pub avg_elapsed_ms: f64,
    pub tool_error_rate: f64,
    pub rework_rate: f64,
    pub ci_failure_rate: f64,
    pub cost_cents: i64,
}

/// Window over `started_at`: `from` (inclusive) .. `to` (exclusive), given
/// as ISO-8601 UTC strings (uniform-width ISO-8601 sorts chronologically).
#[derive(Debug, Clone, Copy)]
pub struct Window<'a> {
    pub from: &'a str,
    pub to: &'a str,
}

/// §18 routing advice for one task class. `evidence` carries the per-model
/// rows the decision was made from, so any recommendation is traceable back
/// to stored invocations.
#[derive(Debug, Clone, PartialEq)]
pub struct RoutingRecommendation {
    pub task_class: String,
    pub preferred_model: String,
    pub evidence: Vec<ModelPerformance>,
    pub samples: i64,
    pub confidence: RecommendationConfidence,
}

const MODEL_PERFORMANCE_SQL: &str = "\
SELECT model, task_class,
  COUNT(*) AS samples,
  SUM(CASE WHEN succeeded THEN 1 ELSE 0 END) AS successes,
  SUM(CASE WHEN first_pass THEN 1 ELSE 0 END) AS first_passes,
  SUM(CASE WHEN rejected THEN 1 ELSE 0 END) AS rejections,
  SUM(re_steers) AS re_steers,
  SUM(tokens_in + tokens_out) AS tokens,
  SUM(elapsed_ms) AS elapsed_ms,
  SUM(CASE WHEN tool_errors > 0 THEN 1 ELSE 0 END) AS tool_error_invocations,
  SUM(CASE WHEN reworked THEN 1 ELSE 0 END) AS reworks,
  SUM(CASE WHEN ci_failed THEN 1 ELSE 0 END) AS ci_failures,
  SUM(cost_cents) AS cost_cents
FROM model_invocations
WHERE started_at >= $1 AND started_at < $2
GROUP BY model, task_class
ORDER BY model, task_class";

/// Aggregate `model_invocations` over `window`, one [`ModelPerformance`]
/// per (model, task_class). Every metric is traceable to stored rows:
/// rates are the boolean-flag counts in the window; `tokens`, `re_steers`
/// and `cost_cents` are plain sums of per-row columns.
pub async fn model_performance(
    pool: &AnyPool,
    window: Window<'_>,
) -> Result<Vec<ModelPerformance>, AutospecError> {
    let rows = sqlx::query(MODEL_PERFORMANCE_SQL)
        .bind(window.from)
        .bind(window.to)
        .fetch_all(pool)
        .await
        .map_err(|error| AutospecError::state("model_invocations", error.to_string()))?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(row_to_performance(row)?);
    }
    Ok(out)
}

fn row_to_performance(row: sqlx::any::AnyRow) -> Result<ModelPerformance, AutospecError> {
    let get = |index: usize| -> Result<i64, AutospecError> {
        row.try_get::<i64, _>(index)
            .map_err(|error| AutospecError::state("model_invocations", error.to_string()))
    };
    let samples = get(2)?;
    let rate = |count: i64| count as f64 / samples as f64;
    Ok(ModelPerformance {
        model: row
            .try_get::<String, _>(0)
            .map_err(|error| AutospecError::state("model_invocations", error.to_string()))?,
        task_class: row
            .try_get::<String, _>(1)
            .map_err(|error| AutospecError::state("model_invocations", error.to_string()))?,
        samples,
        success_rate: rate(get(3)?),
        first_pass_success: rate(get(4)?),
        rejection_rate: rate(get(5)?),
        re_steers: get(6)?,
        tokens: get(7)?,
        avg_elapsed_ms: get(8)? as f64 / samples as f64,
        tool_error_rate: rate(get(9)?),
        rework_rate: rate(get(10)?),
        ci_failure_rate: rate(get(11)?),
        cost_cents: get(12)?,
    })
}

/// Pick the preferred model for every task class present in `rows`.
/// Deterministic: highest success rate, then highest first-pass success,
/// then lowest cost, then model name. A task class whose total samples
/// are below `min_samples` still yields a recommendation, but with
/// [`RecommendationConfidence::Low`] — it is never recommended outright.
pub fn recommend(rows: &[ModelPerformance], min_samples: i64) -> Vec<RoutingRecommendation> {
    let mut classes: Vec<&str> = rows.iter().map(|r| r.task_class.as_str()).collect();
    classes.sort_unstable();
    classes.dedup();
    let mut out = Vec::with_capacity(classes.len());
    for task_class in classes {
        let evidence: Vec<ModelPerformance> = rows
            .iter()
            .filter(|r| r.task_class == task_class)
            .cloned()
            .collect();
        let samples: i64 = evidence.iter().map(|r| r.samples).sum();
        let best = pick_best(&evidence);
        out.push(RoutingRecommendation {
            task_class: task_class.to_string(),
            preferred_model: best.model.clone(),
            evidence,
            samples,
            confidence: if samples >= min_samples {
                RecommendationConfidence::High
            } else {
                RecommendationConfidence::Low
            },
        });
    }
    out
}

/// Deterministic winner: highest success rate, then first-pass success, then
/// lowest cost, then model name (byte order) so ties never depend on input
/// order.
fn pick_best(rows: &[ModelPerformance]) -> &ModelPerformance {
    rows.iter()
        .max_by(|a, b| {
            a.success_rate
                .total_cmp(&b.success_rate)
                .then_with(|| a.first_pass_success.total_cmp(&b.first_pass_success))
                .then_with(|| b.cost_cents.cmp(&a.cost_cents))
                .then_with(|| b.model.cmp(&a.model))
        })
        .expect("recommendation evidence groups are never empty")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resources::db::open_shared_db;
    use sqlx::pool::PoolConnection;
    use std::sync::Mutex;

    static DIR_COUNTER: Mutex<u32> = Mutex::new(0);

    fn perf(
        model: &str,
        task_class: &str,
        samples: i64,
        success_rate: f64,
        cost_cents: i64,
    ) -> ModelPerformance {
        ModelPerformance {
            model: model.into(),
            task_class: task_class.into(),
            samples,
            success_rate,
            first_pass_success: success_rate,
            rejection_rate: 0.0,
            re_steers: 0,
            tokens: 0,
            avg_elapsed_ms: 0.0,
            tool_error_rate: 0.0,
            rework_rate: 0.0,
            ci_failure_rate: 0.0,
            cost_cents,
        }
    }

    #[test]
    fn recommend_prefers_higher_success_rate_and_carries_evidence_rows() {
        let rows = vec![
            perf("qwen-38b", "implement", 60, 0.93, 100),
            perf("codex", "implement", 40, 0.97, 500),
        ];
        let recs = recommend(&rows, 5);
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].task_class, "implement");
        assert_eq!(recs[0].preferred_model, "codex");
        assert_eq!(recs[0].samples, 100);
        assert_eq!(recs[0].confidence, RecommendationConfidence::High);
        // Evidence rows are the per-model aggregates that fed the call.
        assert_eq!(recs[0].evidence.len(), 2);
        assert!(recs[0].evidence.iter().any(|r| r.model == "qwen-38b"));
        assert!(recs[0].evidence.iter().any(|r| r.model == "codex"));
    }

    #[test]
    fn recommend_does_not_recommend_outright_below_min_samples() {
        // A 2-sample task class is not recommended outright: it still gets
        // advice, but at Low confidence.
        let rows = vec![perf("qwen-38b", "plan", 2, 0.5, 10)];
        let recs = recommend(&rows, 5);
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].preferred_model, "qwen-38b");
        assert_eq!(recs[0].samples, 2);
        assert_eq!(recs[0].confidence, RecommendationConfidence::Low);
    }

    #[test]
    fn recommend_breaks_success_ties_on_lower_cost() {
        let rows = vec![
            perf("expensive", "review", 10, 0.9, 900),
            perf("cheap", "review", 10, 0.9, 200),
        ];
        let recs = recommend(&rows, 5);
        assert_eq!(recs[0].preferred_model, "cheap");
    }

    // ── DB-backed per-dimension aggregation (real database, no mocks) ──
    // Postgres via AUTOSPEC_TEST_DB_URL when set; a real throwaway SQLite
    // database otherwise.

    fn sqlite_test_url() -> String {
        let mut counter = DIR_COUNTER.lock().unwrap();
        *counter += 1;
        let dir = std::env::temp_dir().join(format!(
            "autospec-insights-test-{}-{}",
            std::process::id(),
            *counter
        ));
        std::fs::create_dir_all(&dir).unwrap();
        format!("sqlite://{}", dir.join("insights.db").display())
    }

    async fn test_pool() -> AnyPool {
        let url = match std::env::var("AUTOSPEC_TEST_DB_URL") {
            Ok(url) if !url.trim().is_empty() => url,
            _ => sqlite_test_url(),
        };
        open_shared_db(&url).await.unwrap()
    }

    async fn seed_combo(
        conn: &mut PoolConnection<sqlx::Any>,
        model: &str,
        task_class: &str,
        n_success: i64,
        n_rework: i64,
    ) {
        for i in 0..100i64 {
            let succeeded = i < n_success;
            let reworked = i >= 100 - n_rework;
            sqlx::query(
                "INSERT INTO model_invocations (model, task_class, started_at, succeeded, \
                 first_pass, rejected, re_steers, tokens_in, tokens_out, elapsed_ms, \
                 tool_errors, reworked, ci_failed, cost_cents) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)",
            )
            .bind(model)
            .bind(task_class)
            .bind("2026-09-08T10:00:00Z")
            .bind(succeeded as i64)
            .bind(succeeded as i64)
            .bind(0i64)
            .bind((i % 5 == 0) as i64)
            .bind(1000 + i)
            .bind(500 + i)
            .bind(60_000i64)
            .bind((i % 10 == 0) as i64)
            .bind(reworked as i64)
            .bind((i % 20 == 0) as i64)
            .bind(10i64)
            .execute(&mut **conn)
            .await
            .unwrap();
        }
    }

    #[tokio::test]
    async fn model_performance_reproduces_section17_example_table_per_dimension() {
        let pool = test_pool().await;
        let mut conn = pool.acquire().await.unwrap();
        sqlx::raw_sql("DROP TABLE IF EXISTS model_invocations")
            .execute(&mut *conn)
            .await
            .unwrap();
        sqlx::raw_sql(
            "CREATE TABLE model_invocations (model TEXT NOT NULL, task_class TEXT NOT NULL, \
             started_at TEXT NOT NULL, succeeded INTEGER NOT NULL, first_pass INTEGER NOT NULL, \
             rejected INTEGER NOT NULL, re_steers INTEGER NOT NULL, tokens_in INTEGER NOT NULL, \
             tokens_out INTEGER NOT NULL, elapsed_ms INTEGER NOT NULL, tool_errors INTEGER NOT NULL, \
             reworked INTEGER NOT NULL, ci_failed INTEGER NOT NULL, cost_cents INTEGER NOT NULL)",
        )
        .execute(&mut *conn)
        .await
        .unwrap();

        // The §17 example table, 100 invocations each so the rates land on
        // the exact one-percentage-point values:
        //   Qwen implementation   93%   7%
        //   Codex implementation  97%   3%
        //   Qwen planning         76%   31%
        //   Codex planning        96%   5%
        seed_combo(&mut conn, "qwen", "implement", 93, 7).await;
        seed_combo(&mut conn, "codex", "implement", 97, 3).await;
        seed_combo(&mut conn, "qwen", "plan", 76, 31).await;
        seed_combo(&mut conn, "codex", "plan", 96, 5).await;
        // Two more dimensions, one row each: grouping must stay per
        // (model, task_class).
        sqlx::raw_sql(
            "INSERT INTO model_invocations VALUES ('qwen', 'review', \
             '2026-09-08T11:00:00Z', 1, 1, 0, 0, 100, 50, 1000, 0, 0, 0, 5)",
        )
        .execute(&mut *conn)
        .await
        .unwrap();
        sqlx::raw_sql(
            "INSERT INTO model_invocations VALUES ('qwen', 'debugging', \
             '2026-09-08T12:00:00Z', 0, 0, 1, 1, 200, 100, 2000, 2, 1, 1, 15)",
        )
        .execute(&mut *conn)
        .await
        .unwrap();
        // Outside the window: must not count toward any aggregate.
        sqlx::raw_sql(
            "INSERT INTO model_invocations VALUES ('qwen', 'implement', \
             '2020-01-01T00:00:00Z', 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0)",
        )
        .execute(&mut *conn)
        .await
        .unwrap();

        let window = Window {
            from: "2026-09-01T00:00:00Z",
            to: "2026-10-01T00:00:00Z",
        };
        let perf = model_performance(&pool, window).await.unwrap();
        assert_eq!(perf.len(), 6);

        let find = |model: &str, class: &str| -> &ModelPerformance {
            perf.iter()
                .find(|p| p.model == model && p.task_class == class)
                .unwrap()
        };

        let qwen_impl = find("qwen", "implement");
        assert_eq!(qwen_impl.samples, 100);
        assert!((qwen_impl.success_rate - 0.93).abs() < 1e-9);
        assert!((qwen_impl.rework_rate - 0.07).abs() < 1e-9);
        assert_eq!(qwen_impl.tokens, 159_900);
        assert_eq!(qwen_impl.cost_cents, 1_000);
        assert!((qwen_impl.avg_elapsed_ms - 60_000.0).abs() < 1e-9);
        assert!((qwen_impl.tool_error_rate - 0.10).abs() < 1e-9);
        assert_eq!(qwen_impl.re_steers, 20);

        let codex_impl = find("codex", "implement");
        assert!((codex_impl.success_rate - 0.97).abs() < 1e-9);
        assert!((codex_impl.rework_rate - 0.03).abs() < 1e-9);

        let qwen_plan = find("qwen", "plan");
        assert!((qwen_plan.success_rate - 0.76).abs() < 1e-9);
        assert!((qwen_plan.rework_rate - 0.31).abs() < 1e-9);

        let codex_plan = find("codex", "plan");
        assert!((codex_plan.success_rate - 0.96).abs() < 1e-9);
        assert!((codex_plan.rework_rate - 0.05).abs() < 1e-9);

        let qwen_review = find("qwen", "review");
        assert_eq!(qwen_review.samples, 1);
        assert!((qwen_review.success_rate - 1.0).abs() < 1e-9);

        let qwen_debug = find("qwen", "debugging");
        assert_eq!(qwen_debug.samples, 1);
        assert!((qwen_debug.rejection_rate - 1.0).abs() < 1e-9);
        assert_eq!(qwen_debug.re_steers, 1);
    }
}
