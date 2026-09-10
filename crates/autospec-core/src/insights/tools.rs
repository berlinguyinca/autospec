//! Tool and skill ROI (§13) and removal/retirement candidates (§14) for the
//! continuous-improvement engine
//! (`docs/specs/2026-09-08-continuous-improvement-engine.md`).
//!
//! [`tool_roi`] aggregates the `tool_invocations` table (issue #3827) per tool.
//! Expected columns (all NOT NULL; booleans stored as 0/1 integers so the
//! group-by SQL stays portable across the shared SQLite and Postgres
//! backends):
//!
//! ```text
//! session_id TEXT, seq INTEGER, tool_name TEXT, model TEXT, task_class TEXT,
//! occurred_at TEXT (ISO-8601 UTC), succeeded, errored, downstream_succeeded,
//! added_tokens, saved_turns, reworked, user_corrected (INTEGER)
//! ```
//!
//! Everything here is advisory (spec §4.2, §14): candidates are review
//! records, nothing is uninstalled, and every number is reproducible from
//! stored rows. Candidate names come from rows that already exist in
//! telemetry — no private skill name ever leaves the stored data set.
//!
//! Performance (§51): the group-by in [`tool_roi`] and the totals query in
//! [`retirement_candidates`] run against `idx_tool_invocations_tool_name`;
//! the per-candidate evidence fetch uses the same index, and a retirement
//! run only fetches evidence for the (few) candidate names.

use crate::error::AutospecError;
use crate::insights::models::Window;
use sqlx::AnyPool;
use sqlx::Row;
use std::collections::BTreeMap;

/// A tool with at most this many invocations over its whole lifetime is a
/// removal candidate (§14: "Invoked: 1").
pub const MAX_INVOCATIONS: i64 = 1;

/// A tool must be installed for at least this many days before it is a
/// removal candidate (§14: "Installed: 73 days").
pub const MIN_AGE_DAYS: i64 = 60;

/// §42: a tool whose pre-execution failures repeat at or above this count
/// becomes a wrapper candidate ("failures repeatedly occur before
/// execution").
pub const WRAPPER_FAILURE_THRESHOLD: i64 = 3;

/// One tool's §13 ROI aggregate over a window. The 10 §13 metrics:
/// invocation count ([`ToolRoi::calls`]); success rate;
/// downstream task success; tool error rate; average added tokens;
/// average saved turns; correlation with rework; frequency of user
/// correction after use; model compatibility; task compatibility.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolRoi {
    pub tool: String,
    /// Metric 1 of 10: invocation count.
    pub calls: i64,
    /// Metric 2 of 10: success rate (`succeeded / calls`).
    pub success_rate: f64,
    /// Metric 3 of 10: downstream task success. Fraction of uses after
    /// which the downstream task still succeeded.
    pub downstream_task_success: f64,
    /// Metric 4 of 10: tool error rate (`errored / calls`).
    ///
    /// `succeeded` and `errored` are independent flags, so the two rates do
    /// not sum to 1 (the §13 table shows 94% success alongside 2% errors).
    pub tool_error_rate: f64,
    /// Metric 5 of 10: average added tokens per invocation.
    pub avg_added_tokens: f64,
    /// Metric 6 of 10: average saved turns per invocation.
    pub avg_saved_turns: f64,
    /// Metric 7 of 10: correlation with rework. Fraction of uses followed
    /// by rework.
    pub rework_correlation: f64,
    /// Metric 8 of 10: frequency of user correction after use.
    pub user_correction_frequency: f64,
    /// Metric 9 of 10: model compatibility. Fraction of the distinct models
    /// this tool was used with that saw at least one successful call.
    pub model_compatibility: f64,
    /// Metric 10 of 10: task compatibility. Fraction of the distinct task
    /// classes this tool was used in that saw at least one successful call.
    pub task_compatibility: f64,
}

const TOOL_ROI_SQL: &str = "\
SELECT tool_name,
  COUNT(*) AS calls,
  SUM(CASE WHEN succeeded = 1 THEN 1 ELSE 0 END) AS successes,
  SUM(CASE WHEN errored = 1 THEN 1 ELSE 0 END) AS errors,
  SUM(CASE WHEN downstream_succeeded = 1 THEN 1 ELSE 0 END) AS downstream,
  SUM(added_tokens) AS added_tokens,
  SUM(saved_turns) AS saved_turns,
  SUM(CASE WHEN reworked = 1 THEN 1 ELSE 0 END) AS reworks,
  SUM(CASE WHEN user_corrected = 1 THEN 1 ELSE 0 END) AS corrections,
  COUNT(DISTINCT model) AS models_seen,
  COUNT(DISTINCT CASE WHEN succeeded = 1 THEN model END) AS models_ok,
  COUNT(DISTINCT task_class) AS tasks_seen,
  COUNT(DISTINCT CASE WHEN succeeded = 1 THEN task_class END) AS tasks_ok
FROM tool_invocations
WHERE occurred_at >= $1 AND occurred_at < $2
GROUP BY tool_name
ORDER BY tool_name";

/// Aggregate `tool_invocations` over `window`, one [`ToolRoi`] per tool.
/// Every metric is traceable to stored rows: rates are stored-flag counts
/// in the window, averages are plain sums, and the compatibility rates are
/// distinct-model / distinct-task-class counts.
pub async fn tool_roi(pool: &AnyPool, window: Window<'_>) -> Result<Vec<ToolRoi>, AutospecError> {
    let rows = sqlx::query(TOOL_ROI_SQL)
        .bind(window.from)
        .bind(window.to)
        .fetch_all(pool)
        .await
        .map_err(|error| AutospecError::state("tool_invocations", error.to_string()))?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(row_to_roi(row)?);
    }
    Ok(out)
}

fn row_to_roi(row: sqlx::any::AnyRow) -> Result<ToolRoi, AutospecError> {
    let get = |index: usize| -> Result<i64, AutospecError> {
        row.try_get::<i64, _>(index)
            .map_err(|error| AutospecError::state("tool_invocations", error.to_string()))
    };
    let calls = get(1)?;
    let rate = |count: i64| count as f64 / calls as f64;
    Ok(ToolRoi {
        tool: row
            .try_get::<String, _>(0)
            .map_err(|error| AutospecError::state("tool_invocations", error.to_string()))?,
        calls,
        success_rate: rate(get(2)?),
        tool_error_rate: rate(get(3)?),
        downstream_task_success: rate(get(4)?),
        avg_added_tokens: get(5)? as f64 / calls as f64,
        avg_saved_turns: get(6)? as f64 / calls as f64,
        rework_correlation: rate(get(7)?),
        user_correction_frequency: rate(get(8)?),
        model_compatibility: get(10)? as f64 / get(9)? as f64,
        task_compatibility: get(12)? as f64 / get(11)? as f64,
    })
}

/// Installed facts for retirement analysis. The installed-at stamp is a
/// registry fact the caller supplies — it is not derivable from invocation
/// rows, so `retirement_candidates` never guesses an age.
#[derive(Debug, Clone, Default)]
pub struct RetirementConfig {
    /// "Now" for age computation (ISO-8601 UTC).
    pub now: String,
    /// Tool/skill name -> installed-at stamp (ISO-8601 UTC). Names absent
    /// from this map cannot be age-checked and are not candidates.
    pub installed_at: BTreeMap<String, String>,
    /// Estimated context overhead in tokens per session, as carried in
    /// every candidate record (§14: "Estimated context overhead").
    pub context_overhead_tokens: i64,
}

/// One stored `tool_invocations` row behind a candidate: the evidence a
/// reviewer re-checks before any removal decision (§14: removal MUST still
/// require review).
#[derive(Debug, Clone, PartialEq)]
pub struct RetirementEvidence {
    pub session_id: String,
    pub seq: i64,
    pub occurred_at: String,
    pub succeeded: bool,
}

/// §14 removal/retirement candidate. Advisory only: this record is
/// evidence for a human review, not an uninstall instruction.
#[derive(Debug, Clone, PartialEq)]
pub struct RetirementCandidate {
    pub name: String,
    pub installed_days: i64,
    pub invocations: i64,
    pub useful_outcomes: i64,
    pub context_overhead_tokens: i64,
    /// Always true — §14 mandates review before any removal.
    pub requires_review: bool,
    pub evidence: Vec<RetirementEvidence>,
}

const RETIREMENT_TOTALS_SQL: &str = "\
SELECT tool_name,
  COUNT(*) AS invocations,
  SUM(CASE WHEN succeeded = 1 THEN 1 ELSE 0 END) AS useful_outcomes
FROM tool_invocations
GROUP BY tool_name
ORDER BY tool_name";

const RETIREMENT_EVIDENCE_SQL: &str = "\
SELECT session_id, seq, occurred_at, succeeded
FROM tool_invocations
WHERE tool_name = $1
ORDER BY occurred_at, seq";

/// §14 removal/retirement candidates over the full invocation history (no
/// window: retirement is about lifetime age, not a recent window).
/// A tool is a candidate iff its installed age is at least
/// [`MIN_AGE_DAYS`] and its lifetime invocations are at most
/// [`MAX_INVOCATIONS`]. Every candidate carries
/// [`RetirementCandidate::requires_review`] and its complete evidence
/// rows, so the decision is reproducible from stored data.
pub async fn retirement_candidates(
    pool: &AnyPool,
    cfg: &RetirementConfig,
) -> Result<Vec<RetirementCandidate>, AutospecError> {
    let totals = sqlx::query(RETIREMENT_TOTALS_SQL)
        .fetch_all(pool)
        .await
        .map_err(|error| AutospecError::state("tool_invocations", error.to_string()))?;
    let mut candidates = Vec::new();
    for row in totals {
        let name = row
            .try_get::<String, _>(0)
            .map_err(|error| AutospecError::state("tool_invocations", error.to_string()))?;
        let invocations = row
            .try_get::<i64, _>(1)
            .map_err(|error| AutospecError::state("tool_invocations", error.to_string()))?;
        let useful_outcomes = row
            .try_get::<i64, _>(2)
            .map_err(|error| AutospecError::state("tool_invocations", error.to_string()))?;
        let Some(installed_at) = cfg.installed_at.get(&name) else {
            continue;
        };
        let installed_days = days_between(installed_at, &cfg.now)?;
        if invocations <= MAX_INVOCATIONS && installed_days >= MIN_AGE_DAYS {
            let evidence_rows = sqlx::query(RETIREMENT_EVIDENCE_SQL)
                .bind(&name)
                .fetch_all(pool)
                .await
                .map_err(|error| AutospecError::state("tool_invocations", error.to_string()))?;
            let mut evidence = Vec::with_capacity(evidence_rows.len());
            for evidence_row in evidence_rows {
                evidence.push(RetirementEvidence {
                    session_id: evidence_row.try_get::<String, _>(0).map_err(|error| {
                        AutospecError::state("tool_invocations", error.to_string())
                    })?,
                    seq: evidence_row.try_get::<i64, _>(1).map_err(|error| {
                        AutospecError::state("tool_invocations", error.to_string())
                    })?,
                    occurred_at: evidence_row.try_get::<String, _>(2).map_err(|error| {
                        AutospecError::state("tool_invocations", error.to_string())
                    })?,
                    succeeded: {
                        let flag: i64 = evidence_row.try_get(3).map_err(|error| {
                            AutospecError::state("tool_invocations", error.to_string())
                        })?;
                        flag != 0
                    },
                });
            }
            candidates.push(RetirementCandidate {
                name,
                installed_days,
                invocations,
                useful_outcomes,
                context_overhead_tokens: cfg.context_overhead_tokens,
                requires_review: true,
                evidence,
            });
        }
    }
    Ok(candidates)
}

/// One `tool_invocations` row as seen by [`wrapper_candidates`]: pure data,
/// no pool access, so the §42 recommendation is testable without a
/// database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvocationRow {
    pub tool_name: String,
    /// Status recorded on the invocation; anything other than `"success"`
    /// counts as a pre-execution failure.
    pub status: String,
}

/// §42 wrapper candidate: a tool whose failures repeat before execution
/// ("the improvement engine SHOULD recommend wrappers when failures
/// repeatedly occur before execution"). Candidates come out in
/// tool-name order and are advisory records only — no wrapper is applied
/// here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WrapperCandidate {
    pub tool_name: String,
    pub invocation_count: i64,
    pub failure_count: i64,
}

/// Recommend wrappers for tools with at least [`WRAPPER_FAILURE_THRESHOLD`]
/// non-success rows in `rows`. Deterministic: input order never affects
/// the output (grouped and emitted in tool-name order).
pub fn wrapper_candidates(rows: &[InvocationRow]) -> Vec<WrapperCandidate> {
    let mut counts: BTreeMap<&str, (i64, i64)> = BTreeMap::new();
    for row in rows {
        let entry = counts.entry(row.tool_name.as_str()).or_insert((0, 0));
        entry.0 += 1;
        if row.status != "success" {
            entry.1 += 1;
        }
    }
    counts
        .into_iter()
        .filter(|&(_, (_, failures))| failures >= WRAPPER_FAILURE_THRESHOLD)
        .map(
            |(tool_name, (invocation_count, failure_count))| WrapperCandidate {
                tool_name: tool_name.to_string(),
                invocation_count,
                failure_count,
            },
        )
        .collect()
}

/// Whole calendar days between two ISO-8601 UTC stamps (only the date part
/// is used). Deterministic and dependency-free: civil-date arithmetic.
fn days_between(from: &str, to: &str) -> Result<i64, AutospecError> {
    let (fy, fm, fd) = parse_civil(from, "installed_at")?;
    let (ty, tm, td) = parse_civil(to, "now")?;
    Ok(days_from_civil(ty, tm, td) - days_from_civil(fy, fm, fd))
}

fn parse_civil(stamp: &str, context: &str) -> Result<(i64, i64, i64), AutospecError> {
    let bytes = stamp.as_bytes();
    let digits = |start: usize, len: usize| -> Result<i64, AutospecError> {
        bytes[start..start + len]
            .iter()
            .try_fold(0i64, |acc, b| {
                b.is_ascii_digit().then_some(acc * 10 + (b - b'0') as i64)
            })
            .ok_or_else(|| AutospecError::parse(context, format!("not a digit: {stamp}")))
    };
    if bytes.len() < 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return Err(AutospecError::parse(
            context,
            format!("expected ISO-8601 UTC date prefix YYYY-MM-DD, got: {stamp}"),
        ));
    }
    let year = digits(0, 4)?;
    let month = digits(5, 2)?;
    let day = digits(8, 2)?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return Err(AutospecError::parse(
            context,
            format!("month/day out of range: {stamp}"),
        ));
    }
    Ok((year, month, day))
}

/// Days from 1970-01-01 for a civil (year, month, day), proleptic Gregorian
/// calendar (Howard Hinnant's `days_from_civil`).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400; // [0, 399]
    let day_of_year = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resources::db::open_shared_db;
    use sqlx::pool::PoolConnection;
    use std::sync::Mutex;

    static DIR_COUNTER: Mutex<u32> = Mutex::new(0);

    fn sqlite_test_url() -> String {
        let mut counter = DIR_COUNTER.lock().unwrap();
        *counter += 1;
        let dir = std::env::temp_dir().join(format!(
            "autospec-insights-tools-test-{}-{}",
            std::process::id(),
            *counter
        ));
        std::fs::create_dir_all(&dir).unwrap();
        format!("sqlite://{}", dir.join("tools.db").display())
    }

    async fn test_pool() -> AnyPool {
        let url = match std::env::var("AUTOSPEC_TEST_DB_URL") {
            Ok(url) if !url.trim().is_empty() => url,
            _ => sqlite_test_url(),
        };
        open_shared_db(&url).await.unwrap()
    }

    async fn reset_tool_invocations(conn: &mut PoolConnection<sqlx::Any>) {
        sqlx::raw_sql("DROP TABLE IF EXISTS tool_invocations")
            .execute(&mut **conn)
            .await
            .unwrap();
        sqlx::raw_sql(
            "CREATE TABLE tool_invocations (session_id TEXT NOT NULL, seq INTEGER NOT NULL, \
             tool_name TEXT NOT NULL, model TEXT NOT NULL, task_class TEXT NOT NULL, \
             occurred_at TEXT NOT NULL, succeeded INTEGER NOT NULL, errored INTEGER NOT NULL, \
             downstream_succeeded INTEGER NOT NULL, added_tokens INTEGER NOT NULL, \
             saved_turns INTEGER NOT NULL, reworked INTEGER NOT NULL, \
             user_corrected INTEGER NOT NULL)",
        )
        .execute(&mut **conn)
        .await
        .unwrap();
    }

    #[allow(clippy::too_many_arguments)]
    async fn seed_invocation(
        conn: &mut PoolConnection<sqlx::Any>,
        session: &str,
        seq: i64,
        tool: &str,
        model: &str,
        task_class: &str,
        occurred_at: &str,
        succeeded: i64,
        errored: i64,
        downstream_succeeded: i64,
        added_tokens: i64,
        saved_turns: i64,
        reworked: i64,
        user_corrected: i64,
    ) {
        sqlx::query(
            "INSERT INTO tool_invocations VALUES \
             ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)",
        )
        .bind(session)
        .bind(seq)
        .bind(tool)
        .bind(model)
        .bind(task_class)
        .bind(occurred_at)
        .bind(succeeded)
        .bind(errored)
        .bind(downstream_succeeded)
        .bind(added_tokens)
        .bind(saved_turns)
        .bind(reworked)
        .bind(user_corrected)
        .execute(&mut **conn)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn tool_roi_reproduces_section13_example_table() {
        // AC: a tool with 391 calls and 2% errors reports a 94% success
        // rate — the §13 example row for LSP.
        let pool = test_pool().await;
        let mut conn = pool.acquire().await.unwrap();
        reset_tool_invocations(&mut conn).await;

        // LSP: 391 calls — 368 successful (94.1%), 8 errored (2.0%), 15
        // neither (e.g. cancelled before running).
        for i in 0..391i64 {
            let succeeded = i < 368;
            let errored = (368..376).contains(&i);
            seed_invocation(
                &mut conn,
                "s-lsp",
                i,
                "lsp",
                "sonnet",
                "implement",
                "2026-09-08T10:00:00Z",
                succeeded as i64,
                errored as i64,
                succeeded as i64,
                50,
                1,
                0,
                0,
            )
            .await;
        }
        // legacy-java-helper: 2 calls, 1 success, 1 error — the §13 "---"
        // row must stay distinguishable from the LSP row.
        seed_invocation(
            &mut conn,
            "s-legacy",
            0,
            "legacy-java-helper",
            "sonnet",
            "implement",
            "2026-09-08T11:00:00Z",
            1,
            0,
            1,
            10,
            0,
            1,
            1,
        )
        .await;
        seed_invocation(
            &mut conn,
            "s-legacy",
            1,
            "legacy-java-helper",
            "sonnet",
            "implement",
            "2026-09-08T11:30:00Z",
            0,
            1,
            0,
            20,
            0,
            0,
            0,
        )
        .await;
        // Outside the window: must not count toward any aggregate.
        seed_invocation(
            &mut conn,
            "s-stale",
            0,
            "lsp",
            "sonnet",
            "implement",
            "2020-01-01T00:00:00Z",
            0,
            1,
            0,
            0,
            0,
            0,
            0,
        )
        .await;

        let window = Window {
            from: "2026-09-01T00:00:00Z",
            to: "2026-10-01T00:00:00Z",
        };
        let roi = tool_roi(&pool, window).await.unwrap();
        assert_eq!(roi.len(), 2);

        // Byte order: "legacy-java-helper" < "lsp".
        let lsp = &roi[1];
        assert_eq!(lsp.tool, "lsp");
        assert_eq!(lsp.calls, 391);
        assert!((lsp.success_rate - 0.9411764706).abs() < 5e-3);
        assert!((lsp.tool_error_rate - 0.0204603580).abs() < 5e-3);
        assert_eq!(lsp.avg_added_tokens, 50.0);
        assert_eq!(lsp.avg_saved_turns, 1.0);

        let legacy = &roi[0];
        assert_eq!(legacy.tool, "legacy-java-helper");
        assert_eq!(legacy.calls, 2);
        assert!((legacy.success_rate - 0.5).abs() < 1e-9);
        assert!((legacy.tool_error_rate - 0.5).abs() < 1e-9);
    }

    #[tokio::test]
    async fn tool_roi_reports_all_ten_section13_metrics_from_seeded_rows() {
        let pool = test_pool().await;
        let mut conn = pool.acquire().await.unwrap();
        reset_tool_invocations(&mut conn).await;

        // browser-test: 4 invocations spanning two models and two task
        // classes; every model and task class has at least one successful
        // call, so both compatibility rates are 100%.
        seed_invocation(
            &mut conn,
            "s-b",
            0,
            "browser-test",
            "sonnet",
            "implement",
            "2026-09-08T10:00:00Z",
            1,
            0,
            1,
            100,
            2,
            0,
            0,
        )
        .await;
        seed_invocation(
            &mut conn,
            "s-b",
            1,
            "browser-test",
            "sonnet",
            "debugging",
            "2026-09-08T10:10:00Z",
            0,
            0,
            0,
            200,
            0,
            1,
            0,
        )
        .await;
        seed_invocation(
            &mut conn,
            "s-b",
            2,
            "browser-test",
            "opus",
            "implement",
            "2026-09-08T10:20:00Z",
            0,
            1,
            0,
            300,
            0,
            0,
            1,
        )
        .await;
        seed_invocation(
            &mut conn,
            "s-b",
            3,
            "browser-test",
            "opus",
            "debugging",
            "2026-09-08T10:30:00Z",
            1,
            0,
            1,
            400,
            4,
            0,
            0,
        )
        .await;
        // flaky-cli: one model, one task class, never succeeded — 0% compatibility.
        seed_invocation(
            &mut conn,
            "s-f",
            0,
            "flaky-cli",
            "sonnet",
            "implement",
            "2026-09-08T12:00:00Z",
            0,
            0,
            0,
            10,
            0,
            0,
            0,
        )
        .await;
        seed_invocation(
            &mut conn,
            "s-f",
            1,
            "flaky-cli",
            "sonnet",
            "implement",
            "2026-09-08T12:10:00Z",
            0,
            1,
            0,
            10,
            0,
            0,
            0,
        )
        .await;

        let window = Window {
            from: "2026-09-01T00:00:00Z",
            to: "2026-10-01T00:00:00Z",
        };
        let roi = tool_roi(&pool, window).await.unwrap();
        assert_eq!(roi.len(), 2);
        let row = &roi[0];
        assert_eq!(row.tool, "browser-test");
        assert_eq!(row.calls, 4);
        assert!((row.success_rate - 0.5).abs() < 1e-9);
        assert!((row.downstream_task_success - 0.5).abs() < 1e-9);
        assert!((row.tool_error_rate - 0.25).abs() < 1e-9);
        assert!((row.avg_added_tokens - 250.0).abs() < 1e-9);
        assert!((row.avg_saved_turns - 1.5).abs() < 1e-9);
        assert!((row.rework_correlation - 0.25).abs() < 1e-9);
        assert!((row.user_correction_frequency - 0.25).abs() < 1e-9);
        assert!((row.model_compatibility - 1.0).abs() < 1e-9);
        assert!((row.task_compatibility - 1.0).abs() < 1e-9);

        let flaky = &roi[1];
        assert_eq!(flaky.tool, "flaky-cli");
        assert_eq!(flaky.calls, 2);
        assert!((flaky.success_rate - 0.0).abs() < 1e-9);
        assert!((flaky.tool_error_rate - 0.5).abs() < 1e-9);
        assert!((flaky.model_compatibility - 0.0).abs() < 1e-9);
        assert!((flaky.task_compatibility - 0.0).abs() < 1e-9);
    }

    #[tokio::test]
    async fn retirement_candidates_flag_a_one_invocation_seventy_three_day_old_skill() {
        // AC: a 1-invocation 73-day-old skill appears as a candidate,
        // carrying its evidence rows and a review flag.
        let pool = test_pool().await;
        let mut conn = pool.acquire().await.unwrap();
        reset_tool_invocations(&mut conn).await;

        seed_invocation(
            &mut conn,
            "s-legacy",
            0,
            "java-legacy-helper",
            "sonnet",
            "implement",
            "2026-06-01T09:00:00Z",
            0,
            0,
            0,
            1870,
            0,
            0,
            0,
        )
        .await;
        // 10 invocations: above the invocation ceiling, not a candidate.
        for i in 0..10i64 {
            seed_invocation(
                &mut conn,
                "s-busy",
                i,
                "busy-tool",
                "sonnet",
                "implement",
                "2026-08-01T09:00:00Z",
                1,
                0,
                1,
                100,
                1,
                0,
                0,
            )
            .await;
        }
        // 1 invocation but installed only 10 days ago: too young.
        seed_invocation(
            &mut conn,
            "s-young",
            0,
            "young-tool",
            "sonnet",
            "implement",
            "2026-08-30T09:00:00Z",
            0,
            1,
            0,
            100,
            0,
            0,
            0,
        )
        .await;

        let cfg = RetirementConfig {
            now: "2026-09-08T00:00:00Z".to_string(),
            installed_at: {
                let mut map = BTreeMap::new();
                map.insert(
                    "java-legacy-helper".to_string(),
                    "2026-06-27T00:00:00Z".to_string(),
                );
                map.insert("busy-tool".to_string(), "2026-06-27T00:00:00Z".to_string());
                map.insert("young-tool".to_string(), "2026-08-29T00:00:00Z".to_string());
                map
            },
            context_overhead_tokens: 1_870,
        };

        let candidates = retirement_candidates(&pool, &cfg).await.unwrap();
        assert_eq!(candidates.len(), 1);
        let c = &candidates[0];
        assert_eq!(c.name, "java-legacy-helper");
        assert_eq!(c.installed_days, 73);
        assert_eq!(c.invocations, 1);
        assert_eq!(c.useful_outcomes, 0);
        assert_eq!(c.context_overhead_tokens, 1_870);
        assert!(c.requires_review);
        assert_eq!(c.evidence.len(), 1);
        assert_eq!(c.evidence[0].session_id, "s-legacy");
        assert_eq!(c.evidence[0].seq, 0);
        assert_eq!(c.evidence[0].occurred_at, "2026-06-01T09:00:00Z");
        assert!(!c.evidence[0].succeeded);
    }

    #[test]
    fn wrapper_candidates_flag_repeated_failures_before_execution() {
        // §42: recommend a wrapper when failures repeatedly occur before
        // execution. 3 failures is the threshold; successes never count.
        let rows = vec![
            InvocationRow {
                tool_name: "shell".into(),
                status: "success".into(),
            },
            InvocationRow {
                tool_name: "shell".into(),
                status: "error".into(),
            },
            InvocationRow {
                tool_name: "shell".into(),
                status: "syntax_error".into(),
            },
            InvocationRow {
                tool_name: "shell".into(),
                status: "error".into(),
            },
            InvocationRow {
                tool_name: "shell".into(),
                status: "error".into(),
            },
            // Two failures only: below the threshold, not a candidate.
            InvocationRow {
                tool_name: "sql".into(),
                status: "error".into(),
            },
            InvocationRow {
                tool_name: "sql".into(),
                status: "error".into(),
            },
            InvocationRow {
                tool_name: "sql".into(),
                status: "success".into(),
            },
        ];
        let out = wrapper_candidates(&rows);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].tool_name, "shell");
        assert_eq!(out[0].invocation_count, 5);
        assert_eq!(out[0].failure_count, 4);
    }

    #[test]
    fn days_between_counts_calendar_days_across_month_boundaries() {
        // 2026-06-27 -> 2026-09-08 is exactly 73 days (the §14 example).
        assert_eq!(
            days_between("2026-06-27T00:00:00Z", "2026-09-08T00:00:00Z"),
            Ok(73)
        );
        // Month and leap-year boundaries.
        assert_eq!(
            days_between("2026-01-31T00:00:00Z", "2026-03-03T00:00:00Z"),
            Ok(31)
        );
        assert_eq!(
            days_between("2024-02-28T00:00:00Z", "2024-03-01T00:00:00Z"),
            Ok(2)
        );
        assert_eq!(
            days_between("2026-09-08T00:00:00Z", "2026-09-08T00:00:00Z"),
            Ok(0)
        );
        // Malformed input is an error, never a silent age.
        assert!(days_between("yesterday", "2026-09-08T00:00:00Z").is_err());
    }
}
