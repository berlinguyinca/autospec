//! §22/§23 pre-merge evaluation harness (issue #3852).
//!
//! Spec: `docs/specs/2026-09-08-continuous-improvement-engine.md` §22-§23,
//! §34.
//!
//! [`evaluate`] replays the §23 golden fixture corpus under the baseline
//! configuration and the candidate configuration, aggregates each arm's §22
//! metrics (tool errors, tokens/task, user corrections, review rework,
//! success rate), computes the candidate-vs-baseline deltas, reaches a
//! [`Verdict`], and persists one §34 `proposal_evaluations` row keyed by the
//! proposal.
//!
//! The two arms always replay the *identical* fixture set — [`Fixture`]
//! carries both the baseline and the candidate measurement for one corpus
//! item, and [`EvaluationReport::fixture_ids`] is the same list for both
//! arms — so a delta can never be attributed to a differing task suite
//! (data-integrity counter-query: "are deltas computed from the same fixture
//! set?").
//!
//! Live A/B running against real sessions, PR creation and post-merge
//! measurement are out of scope for this module (see the issue's out-of-scope
//! list): this is historical replay over the stored corpus only, and nothing
//! here writes a file, applies a patch or touches the tree (§4.5).

use serde::{Deserialize, Serialize};
use sqlx::{AnyPool, Row};

use crate::error::AutospecError;

use super::proposals::schema::Proposal;

/// §34 `proposal_evaluations` DDL — backend-neutral (TEXT + JSON on both
/// Postgres and SQLite), idempotent.
const ENSURE_EVALUATIONS_TABLE: &str = r#"CREATE TABLE IF NOT EXISTS proposal_evaluations (
    proposal_id TEXT PRIMARY KEY,
    baseline JSON NOT NULL,
    candidate JSON NOT NULL,
    deltas JSON NOT NULL,
    verdict TEXT NOT NULL,
    fixture_ids JSON NOT NULL,
    created_at TEXT NOT NULL
)"#;

/// A delta below this magnitude (in percentage points) is treated as no
/// meaningful movement: the candidate is neither improved nor regressed on
/// that metric. §22 commits to percentage targets (`-25%`, `-10%`, `+5%`),
/// so a 5% band keeps an unchanged candidate `inconclusive` while a
/// committed improvement (e.g. −25% corrections) clears it.
pub const MEANINGFUL_DELTA_PCT: f64 = 5.0;

/// §22 metric set measured for one arm of a replay — the raw counts an
/// aggregated corpus value is derived from. `tasks` is the denominator for
/// the two per-task metrics (tokens/task, success rate).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct MetricSet {
    /// §22 "tool_errors" — count of tool errors.
    pub tool_errors: u64,
    /// §22 "tokens/task" numerator — total tokens consumed.
    pub tokens: u64,
    /// §22 "user corrections" — count of user re-steering corrections.
    pub user_corrections: u64,
    /// §22 "review rework" — count of review rework rounds.
    pub review_rework: u64,
    /// §22 "success rate" numerator — number of successful tasks.
    pub successes: u64,
    /// Denominator for tokens/task and success rate.
    pub tasks: u64,
}

impl MetricSet {
    /// §22 "tokens/task" — total tokens over the task count.
    pub fn tokens_per_task(&self) -> f64 {
        if self.tasks == 0 {
            0.0
        } else {
            self.tokens as f64 / self.tasks as f64
        }
    }

    /// §22 "success rate" — successes over the task count, in [0, 1].
    pub fn success_rate(&self) -> f64 {
        if self.tasks == 0 {
            0.0
        } else {
            self.successes as f64 / self.tasks as f64
        }
    }
}

/// Sum two metric sets element-wise — the corpus-level aggregation used to
/// fold per-fixture arm measurements into one arm-wide value.
fn add_metrics(acc: &mut MetricSet, other: &MetricSet) {
    acc.tool_errors = acc.tool_errors.saturating_add(other.tool_errors);
    acc.tokens = acc.tokens.saturating_add(other.tokens);
    acc.user_corrections = acc.user_corrections.saturating_add(other.user_corrections);
    acc.review_rework = acc.review_rework.saturating_add(other.review_rework);
    acc.successes = acc.successes.saturating_add(other.successes);
    acc.tasks = acc.tasks.saturating_add(other.tasks);
}

/// One golden §23 replay corpus item (issue #3841): a single fixture measured
/// under both the baseline configuration and the candidate configuration.
/// The two arm measurements are the replay driver's output for the same task
/// suite, so a delta is always over the same fixture set.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Fixture {
    /// Corpus-unique fixture identifier.
    pub fixture_id: String,
    /// Metrics measured replaying this fixture under the baseline config.
    pub baseline: MetricSet,
    /// Metrics measured replaying this fixture under the candidate config.
    pub candidate: MetricSet,
}

/// The five §22 candidate-vs-baseline deltas, as percentage changes of the
/// candidate value relative to the baseline value.
///
/// The sign follows the raw change, not the desirability: for the four
/// reduction metrics (tool_errors, tokens/task, user_corrections,
/// review_rework) a negative delta is an improvement, and for success rate a
/// positive delta is an improvement. A `None` delta means the baseline was
/// zero, so a percentage change is undefined (the verdict still reasons about
/// the 0→positive case from the raw counts).
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Deltas {
    /// §22 "tool_errors" — percentage change in tool errors.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_errors: Option<f64>,
    /// §22 "tokens/task" — percentage change in tokens per task.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens_per_task: Option<f64>,
    /// §22 "user corrections" — percentage change in user corrections.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_corrections: Option<f64>,
    /// §22 "review rework" — percentage change in review rework.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub review_rework: Option<f64>,
    /// §22 "success rate" — percentage change in success rate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub success_rate: Option<f64>,
}

/// §23 evaluation verdict for a proposal, derived from the §22 deltas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// The candidate meaningfully improved at least one §22 metric and
    /// regressed none.
    Improved,
    /// No §22 metric moved beyond the meaningful band — the candidate is
    /// indistinguishable from baseline.
    #[default]
    Inconclusive,
    /// The candidate meaningfully regressed at least one §22 metric.
    Regressed,
}

impl Verdict {
    /// The §34 `proposal_evaluations.verdict` wire value.
    pub fn as_str(&self) -> &'static str {
        match self {
            Verdict::Improved => "improved",
            Verdict::Inconclusive => "inconclusive",
            Verdict::Regressed => "regressed",
        }
    }
}

impl std::str::FromStr for Verdict {
    type Err = AutospecError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Ok(match value {
            "improved" => Verdict::Improved,
            "inconclusive" => Verdict::Inconclusive,
            "regressed" => Verdict::Regressed,
            other => {
                return Err(AutospecError::parse(
                    "proposal_evaluations.verdict",
                    format!("unknown §23 verdict {other:?}"),
                ))
            }
        })
    }
}

/// §23 evaluation report: the persisted evidence for one proposal's
/// baseline-vs-candidate comparison.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct EvaluationReport {
    /// §21 proposal id this evaluation scores.
    pub proposal_id: String,
    /// Corpus-aggregated baseline arm metrics.
    pub baseline: MetricSet,
    /// Corpus-aggregated candidate arm metrics.
    pub candidate: MetricSet,
    /// The five §22 candidate-vs-baseline deltas.
    pub deltas: Deltas,
    /// Derived verdict.
    pub verdict: Verdict,
    /// The corpus fixture ids both arms were replayed over — identical for
    /// both arms by construction.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fixture_ids: Vec<String>,
}

/// Percentage change of `candidate` relative to `baseline`; `None` when the
/// baseline is zero (an undefined ratio). The pure arithmetic at the heart of
/// the §22 delta computation.
fn pct_change(candidate: f64, baseline: f64) -> Option<f64> {
    if baseline == 0.0 {
        None
    } else {
        Some((candidate - baseline) / baseline * 100.0)
    }
}

/// Fold a corpus slice into one arm-wide metric set, element-wise.
fn aggregate(fixtures: &[Fixture], arm: impl Fn(&Fixture) -> &MetricSet) -> MetricSet {
    let mut acc = MetricSet::default();
    for fixture in fixtures {
        add_metrics(&mut acc, arm(fixture));
    }
    acc
}

/// Compute the five §22 candidate-vs-baseline deltas from two aggregated
/// metric sets.
pub fn compute_deltas(baseline: &MetricSet, candidate: &MetricSet) -> Deltas {
    Deltas {
        tool_errors: pct_change(candidate.tool_errors as f64, baseline.tool_errors as f64),
        tokens_per_task: pct_change(candidate.tokens_per_task(), baseline.tokens_per_task()),
        user_corrections: pct_change(
            candidate.user_corrections as f64,
            baseline.user_corrections as f64,
        ),
        review_rework: pct_change(
            candidate.review_rework as f64,
            baseline.review_rework as f64,
        ),
        success_rate: pct_change(candidate.success_rate(), baseline.success_rate()),
    }
}

/// Per-metric movement direction used to reach a verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Direction {
    Improved,
    Regressed,
    Neutral,
}

/// Classify one reduction metric (tool_errors, tokens/task, corrections,
/// rework): improvement is a negative delta. A `None` delta means the
/// baseline was zero; then any positive candidate value is a regression.
fn classify_reduction(delta: Option<f64>, candidate: f64) -> Direction {
    match delta {
        Some(d) if d <= -MEANINGFUL_DELTA_PCT => Direction::Improved,
        Some(d) if d >= MEANINGFUL_DELTA_PCT => Direction::Regressed,
        Some(_) => Direction::Neutral,
        None => {
            if candidate > 0.0 {
                Direction::Regressed
            } else {
                Direction::Neutral
            }
        }
    }
}

/// Classify the success-rate metric: improvement is a positive delta. A
/// `None` delta means the baseline was zero; then any positive candidate rate
/// is an improvement.
fn classify_success(delta: Option<f64>, candidate: f64) -> Direction {
    match delta {
        Some(d) if d >= MEANINGFUL_DELTA_PCT => Direction::Improved,
        Some(d) if d <= -MEANINGFUL_DELTA_PCT => Direction::Regressed,
        Some(_) => Direction::Neutral,
        None => {
            if candidate > 0.0 {
                Direction::Improved
            } else {
                Direction::Neutral
            }
        }
    }
}

/// §23 verdict from the §22 deltas and the raw candidate-arm metrics: a
/// regression on any metric is a regression; otherwise an improvement on any
/// metric is an improvement; otherwise the candidate is indistinguishable
/// from baseline.
pub fn decide_verdict(deltas: &Deltas, candidate: &MetricSet) -> Verdict {
    let directions = [
        classify_reduction(deltas.tool_errors, candidate.tool_errors as f64),
        classify_reduction(deltas.tokens_per_task, candidate.tokens_per_task()),
        classify_reduction(deltas.user_corrections, candidate.user_corrections as f64),
        classify_reduction(deltas.review_rework, candidate.review_rework as f64),
        classify_success(deltas.success_rate, candidate.success_rate()),
    ];
    if directions.iter().any(|d| *d == Direction::Regressed) {
        Verdict::Regressed
    } else if directions.iter().any(|d| *d == Direction::Improved) {
        Verdict::Improved
    } else {
        Verdict::Inconclusive
    }
}

/// Serialize any `Serialize` value to its portable TEXT form.
fn to_json<T: serde::Serialize>(value: &T) -> Result<String, AutospecError> {
    serde_json::to_string(value)
        .map_err(|error| AutospecError::state("proposal_evaluations", error.to_string()))
}

/// Decode one JSON column value from its portable TEXT form.
fn json_from_column<T: serde::de::DeserializeOwned>(raw: &str) -> Result<T, AutospecError> {
    serde_json::from_str(raw)
        .map_err(|error| AutospecError::state("proposal_evaluations", error.to_string()))
}

/// Create the §34 `proposal_evaluations` table if it does not exist yet.
pub async fn ensure_schema(pool: &AnyPool) -> Result<(), AutospecError> {
    sqlx::query::<sqlx::Any>(ENSURE_EVALUATIONS_TABLE)
        .execute(pool)
        .await
        .map_err(|error| AutospecError::state("insights evaluation schema", error.to_string()))?;
    Ok(())
}

/// Unix-epoch seconds as the portable `created_at` value.
fn now_unix_seconds() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as u128)
        .unwrap_or(0)
}

/// Persist one §34 `proposal_evaluations` row keyed by `proposal_id`. A
/// proposal is evaluated at most once; a second evaluation of the same
/// proposal replaces the earlier row (the report is the evidence for the
/// latest run).
pub async fn store_evaluation(
    pool: &AnyPool,
    report: &EvaluationReport,
) -> Result<(), AutospecError> {
    if report.proposal_id.trim().is_empty() {
        return Err(AutospecError::validation(
            "store_evaluation() requires a non-empty proposal_id",
        ));
    }
    ensure_schema(pool).await?;
    sqlx::query::<sqlx::Any>(
        "INSERT INTO proposal_evaluations (\n\
             proposal_id, baseline, candidate, deltas, verdict, fixture_ids, created_at\n\
         ) VALUES (?, ?, ?, ?, ?, ?, ?)\n\
         ON CONFLICT(proposal_id) DO UPDATE SET\n\
             baseline = excluded.baseline,\n\
             candidate = excluded.candidate,\n\
             deltas = excluded.deltas,\n\
             verdict = excluded.verdict,\n\
             fixture_ids = excluded.fixture_ids,\n\
             created_at = excluded.created_at",
    )
    .bind(&report.proposal_id)
    .bind(to_json(&report.baseline)?)
    .bind(to_json(&report.candidate)?)
    .bind(to_json(&report.deltas)?)
    .bind(report.verdict.as_str())
    .bind(to_json(&report.fixture_ids)?)
    .bind(now_unix_seconds().to_string())
    .execute(pool)
    .await
    .map_err(|error| AutospecError::state("proposal_evaluations", error.to_string()))?;
    Ok(())
}

/// Read one §34 `proposal_evaluations` row back by proposal id; `None` when
/// no such row exists.
pub async fn load_evaluation(
    pool: &AnyPool,
    proposal_id: &str,
) -> Result<Option<EvaluationReport>, AutospecError> {
    ensure_schema(pool).await?;
    let row = sqlx::query::<sqlx::Any>(
        "SELECT proposal_id, baseline, candidate, deltas, verdict, fixture_ids\n\
         FROM proposal_evaluations\n\
         WHERE proposal_id = ?",
    )
    .bind(proposal_id)
    .fetch_optional(pool)
    .await
    .map_err(|error| AutospecError::state("proposal_evaluations", error.to_string()))?;
    let Some(row) = row else {
        return Ok(None);
    };
    let decode = |index: usize| -> Result<String, AutospecError> {
        row.try_get(index)
            .map_err(|error| AutospecError::state("proposal_evaluations", error.to_string()))
    };
    let verdict: String = decode(4)?;
    Ok(Some(EvaluationReport {
        proposal_id: decode(0)?,
        baseline: json_from_column(&decode(1)?)?,
        candidate: json_from_column(&decode(2)?)?,
        deltas: json_from_column(&decode(3)?)?,
        verdict: verdict.parse()?,
        fixture_ids: json_from_column(&decode(5)?)?,
    }))
}

/// §23 pre-merge evaluation: replay `fixtures` under the baseline and the
/// candidate configuration and write one `proposal_evaluations` row for the
/// proposal.
///
/// The §22 measurability gate ([`Proposal::validate`]) is applied here — the
/// evaluation stage is where a proposal's measurable expected effects are
/// scored — so an unevaluable proposal is rejected before any row is written.
/// Both arms replay the identical fixture set (the [`Fixture`] carries both
/// arm measurements), so a delta is always over the same task suite.
///
/// Writes to `proposal_evaluations` and nothing else: no files, no patches,
/// no tree mutations (§4.5). Live A/B, PR creation and post-merge measurement
/// are out of scope.
pub async fn evaluate(
    pool: &AnyPool,
    proposal: &Proposal,
    fixtures: &[Fixture],
) -> Result<EvaluationReport, AutospecError> {
    // §22: the evaluation is where a proposal's measurability is enforced.
    proposal.validate()?;
    if fixtures.is_empty() {
        return Err(AutospecError::validation(
            "evaluate() requires a non-empty fixture corpus",
        ));
    }
    // Both arms replay the identical fixture set: aggregate baseline and
    // candidate arms from the same `fixtures` slice.
    let baseline = aggregate(fixtures, |fixture| &fixture.baseline);
    let candidate = aggregate(fixtures, |fixture| &fixture.candidate);
    let deltas = compute_deltas(&baseline, &candidate);
    let verdict = decide_verdict(&deltas, &candidate);
    let fixture_ids = fixtures
        .iter()
        .map(|fixture| fixture.fixture_id.clone())
        .collect();

    let report = EvaluationReport {
        proposal_id: proposal.proposal_id.clone(),
        baseline,
        candidate,
        deltas,
        verdict,
        fixture_ids,
    };
    store_evaluation(pool, &report).await?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::insights::proposals::schema::{
        EvaluationPlan, Evidence, ExpectedEffect, ProposalStatus, ProposalType,
    };
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    static SQLITE_COUNTER: AtomicU32 = AtomicU32::new(0);

    /// Test pool: the disposable `AUTOSPEC_TEST_DB_URL` database when the
    /// operator provided one (Postgres 16 under Apptainer in the full
    /// validation), else a throwaway on-disk SQLite database (a real
    /// database, not a mock) so the storage path stays covered without one.
    async fn test_pool() -> Result<(AnyPool, Option<PathBuf>), AutospecError> {
        match std::env::var("AUTOSPEC_TEST_DB_URL") {
            Ok(url) if !url.trim().is_empty() => Ok((
                crate::resources::db::open_shared_db(url.trim()).await?,
                None,
            )),
            Ok(_) => Err(AutospecError::validation(
                "AUTOSPEC_TEST_DB_URL is set but empty",
            )),
            Err(_) => {
                let index = SQLITE_COUNTER.fetch_add(1, Ordering::SeqCst);
                let dir = std::env::temp_dir().join(format!(
                    "autospec-evaluation-test-{}-{index}",
                    std::process::id()
                ));
                let url = format!("sqlite://{}/test.db", dir.display());
                Ok((crate::resources::db::open_shared_db(&url).await?, Some(dir)))
            }
        }
    }

    /// Remove the throwaway SQLite directory, if any.
    fn remove_cleanup_dir(cleanup_dir: Option<PathBuf>) {
        if let Some(dir) = cleanup_dir {
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    fn ms(
        tool_errors: u64,
        tokens: u64,
        user_corrections: u64,
        review_rework: u64,
        successes: u64,
        tasks: u64,
    ) -> MetricSet {
        MetricSet {
            tool_errors,
            tokens,
            user_corrections,
            review_rework,
            successes,
            tasks,
        }
    }

    /// A valid, measurable §21 proposal for evaluation (the §22 gate runs at
    /// the evaluation stage, so the proposal must carry a measurable effect).
    fn measurable_proposal(proposal_id: &str) -> Proposal {
        Proposal {
            proposal_id: proposal_id.to_string(),
            finding_id: "finding_001".to_string(),
            kind: ProposalType::Skill,
            title: "Candidate improvement".to_string(),
            status: ProposalStatus::default(),
            rationale: "recurring pattern".to_string(),
            evidence: vec![Evidence::finding_citation("finding_001")],
            expected_effect: ExpectedEffect {
                user_correction_reduction_pct: Some(25),
                ..ExpectedEffect::default()
            },
            risks: vec![],
            affected_components: vec![],
            patch: String::new(),
            evaluation_plan: EvaluationPlan::default(),
            created_by_model: "qwen3-32b".to_string(),
            review_model: None,
        }
    }

    fn fixture(id: &str, baseline: MetricSet, candidate: MetricSet) -> Fixture {
        Fixture {
            fixture_id: id.to_string(),
            baseline,
            candidate,
        }
    }

    // ---- TDD: delta arithmetic -----------------------------------------

    #[test]
    fn pct_change_is_zero_when_values_are_equal() {
        assert_eq!(pct_change(100.0, 100.0), Some(0.0));
    }

    #[test]
    fn pct_change_is_negative_for_a_reduction() {
        assert_eq!(pct_change(75.0, 100.0), Some(-25.0));
    }

    #[test]
    fn pct_change_is_positive_for_an_increase() {
        assert_eq!(pct_change(110.0, 100.0), Some(10.0));
    }

    #[test]
    fn pct_change_is_undefined_when_baseline_is_zero() {
        assert_eq!(pct_change(5.0, 0.0), None);
        assert_eq!(pct_change(0.0, 0.0), None);
    }

    #[test]
    fn compute_deltas_covers_the_five_section_22_metrics() {
        let baseline = ms(10, 100_000, 100, 20, 8, 10);
        let candidate = ms(5, 80_000, 75, 15, 9, 10);
        let deltas = compute_deltas(&baseline, &candidate);
        assert_eq!(deltas.tool_errors, Some(-50.0));
        assert_eq!(deltas.tokens_per_task, Some(-20.0));
        assert_eq!(deltas.user_corrections, Some(-25.0));
        assert_eq!(deltas.review_rework, Some(-25.0));
        // success rate 0.8 -> 0.9 = +12.5%
        assert_approx(deltas.success_rate, 12.5);
    }

    #[test]
    fn compute_deltas_uses_tokens_per_task_not_total_tokens() {
        // Different task counts, same totals: the per-task metric must differ.
        let baseline = ms(0, 100_000, 0, 0, 5, 10);
        let candidate = ms(0, 100_000, 0, 0, 5, 5);
        let deltas = compute_deltas(&baseline, &candidate);
        // 10k vs 20k tokens/task = +100%
        assert_eq!(deltas.tokens_per_task, Some(100.0));
    }

    // ---- Verdicts -------------------------------------------------------

    #[test]
    fn candidate_improving_corrections_by_25_percent_reports_improved() {
        let fixtures = [fixture(
            "f1",
            ms(10, 100_000, 100, 10, 8, 10),
            ms(10, 100_000, 75, 10, 8, 10),
        )];
        let baseline = aggregate(&fixtures, |f| &f.baseline);
        let candidate = aggregate(&fixtures, |f| &f.candidate);
        let deltas = compute_deltas(&baseline, &candidate);
        assert_eq!(deltas.user_corrections, Some(-25.0));
        assert_eq!(decide_verdict(&deltas, &candidate), Verdict::Improved);
    }

    #[test]
    fn unchanged_candidate_reports_inconclusive() {
        let fixtures = [fixture(
            "f1",
            ms(10, 100_000, 100, 10, 8, 10),
            ms(10, 100_000, 100, 10, 8, 10),
        )];
        let baseline = aggregate(&fixtures, |f| &f.baseline);
        let candidate = aggregate(&fixtures, |f| &f.candidate);
        let deltas = compute_deltas(&baseline, &candidate);
        assert_eq!(decide_verdict(&deltas, &candidate), Verdict::Inconclusive);
    }

    #[test]
    fn a_sub_threshold_change_is_inconclusive_not_improved() {
        // +4% success (below the 5% band) is not a meaningful improvement.
        let fixtures = [fixture(
            "f1",
            ms(10, 100_000, 100, 10, 96, 100),
            ms(10, 100_000, 100, 10, 100, 100),
        )];
        let baseline = aggregate(&fixtures, |f| &f.baseline);
        let candidate = aggregate(&fixtures, |f| &f.candidate);
        let deltas = compute_deltas(&baseline, &candidate);
        assert_approx(deltas.success_rate, 4.166_666_666_666_667);
        assert_eq!(decide_verdict(&deltas, &candidate), Verdict::Inconclusive);
    }

    #[test]
    fn a_regression_on_any_metric_is_a_regression() {
        // Success improved, but tool errors rose 50% -> the run is regressed.
        let fixtures = [fixture(
            "f1",
            ms(10, 100_000, 100, 10, 8, 10),
            ms(15, 100_000, 90, 10, 10, 10),
        )];
        let baseline = aggregate(&fixtures, |f| &f.baseline);
        let candidate = aggregate(&fixtures, |f| &f.candidate);
        let deltas = compute_deltas(&baseline, &candidate);
        assert_eq!(deltas.tool_errors, Some(50.0));
        assert_eq!(decide_verdict(&deltas, &candidate), Verdict::Regressed);
    }

    #[test]
    fn zero_baseline_movement_is_classified_from_the_raw_counts() {
        // tool_errors 0 -> 1: a regression (a reduction metric going 0 -> +ve).
        let fixtures = [fixture(
            "f1",
            ms(0, 100_000, 100, 10, 8, 10),
            ms(1, 100_000, 100, 10, 8, 10),
        )];
        let baseline = aggregate(&fixtures, |f| &f.baseline);
        let candidate = aggregate(&fixtures, |f| &f.candidate);
        let deltas = compute_deltas(&baseline, &candidate);
        assert_eq!(deltas.tool_errors, None);
        assert_eq!(decide_verdict(&deltas, &candidate), Verdict::Regressed);

        // success rate 0 -> 1: an improvement.
        let fixtures = [fixture(
            "f1",
            ms(10, 100_000, 100, 10, 0, 10),
            ms(10, 100_000, 100, 10, 10, 10),
        )];
        let baseline = aggregate(&fixtures, |f| &f.baseline);
        let candidate = aggregate(&fixtures, |f| &f.candidate);
        let deltas = compute_deltas(&baseline, &candidate);
        assert_eq!(deltas.success_rate, None);
        assert_eq!(decide_verdict(&deltas, &candidate), Verdict::Improved);
    }

    // ---- evaluate() over the DB ----------------------------------------

    #[tokio::test]
    async fn evaluate_writes_one_proposal_evaluations_row_per_proposal() {
        let (pool, cleanup_dir) = test_pool().await.unwrap();
        let proposal = measurable_proposal("proposal_eval_one");
        let fixtures = [
            fixture(
                "f1",
                ms(10, 100_000, 100, 10, 8, 10),
                ms(10, 100_000, 75, 10, 8, 10),
            ),
            fixture("f2", ms(5, 50_000, 50, 5, 4, 5), ms(5, 50_000, 40, 5, 4, 5)),
        ];
        let report = evaluate(&pool, &proposal, &fixtures).await.unwrap();
        assert_eq!(report.verdict, Verdict::Improved);
        assert_eq!(report.fixture_ids, vec!["f1".to_string(), "f2".to_string()]);

        // Exactly one row keyed by the proposal id.
        let count: i64 = sqlx::query::<sqlx::Any>(
            "SELECT COUNT(*) FROM proposal_evaluations WHERE proposal_id = ?",
        )
        .bind("proposal_eval_one")
        .fetch_one(&pool)
        .await
        .unwrap()
        .try_get(0)
        .unwrap();
        assert_eq!(count, 1);

        // The stored row round-trips the report, including the identical
        // fixture_ids on both arms.
        let stored = load_evaluation(&pool, "proposal_eval_one")
            .await
            .unwrap()
            .expect("row must be readable back");
        assert_eq!(stored.proposal_id, report.proposal_id);
        assert_eq!(stored.verdict, report.verdict);
        assert_eq!(stored.deltas, report.deltas);
        assert_eq!(stored.baseline, report.baseline);
        assert_eq!(stored.candidate, report.candidate);
        assert_eq!(stored.fixture_ids, report.fixture_ids);
        assert_eq!(stored.fixture_ids, vec!["f1".to_string(), "f2".to_string()]);

        // Cleanup.
        let _ = sqlx::query::<sqlx::Any>("DELETE FROM proposal_evaluations WHERE proposal_id = ?")
            .bind("proposal_eval_one")
            .execute(&pool)
            .await;
        remove_cleanup_dir(cleanup_dir);
    }

    #[tokio::test]
    async fn evaluate_reports_unchanged_candidate_as_inconclusive() {
        let (pool, cleanup_dir) = test_pool().await.unwrap();
        let proposal = measurable_proposal("proposal_eval_inconclusive");
        let fixtures = [fixture(
            "f1",
            ms(10, 100_000, 100, 10, 8, 10),
            ms(10, 100_000, 100, 10, 8, 10),
        )];
        let report = evaluate(&pool, &proposal, &fixtures).await.unwrap();
        assert_eq!(report.verdict, Verdict::Inconclusive);
        let _ = sqlx::query::<sqlx::Any>("DELETE FROM proposal_evaluations WHERE proposal_id = ?")
            .bind("proposal_eval_inconclusive")
            .execute(&pool)
            .await;
        remove_cleanup_dir(cleanup_dir);
    }

    #[tokio::test]
    async fn both_arms_replay_the_identical_fixture_set() {
        let (pool, cleanup_dir) = test_pool().await.unwrap();
        let proposal = measurable_proposal("proposal_eval_fair");
        let fixtures = [
            fixture(
                "f1",
                ms(10, 100_000, 100, 10, 8, 10),
                ms(10, 100_000, 75, 10, 8, 10),
            ),
            fixture("f2", ms(5, 50_000, 50, 5, 4, 5), ms(5, 50_000, 40, 5, 4, 5)),
            fixture("f3", ms(1, 10_000, 10, 1, 1, 1), ms(1, 10_000, 8, 1, 1, 1)),
        ];
        let report = evaluate(&pool, &proposal, &fixtures).await.unwrap();
        assert_eq!(
            report.fixture_ids,
            vec!["f1".to_string(), "f2".to_string(), "f3".to_string()]
        );
        // The fixture_ids list is a single list on the report, shared by both
        // arms; the stored row carries it verbatim.
        let stored = load_evaluation(&pool, "proposal_eval_fair")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.fixture_ids, report.fixture_ids);
        let _ = sqlx::query::<sqlx::Any>("DELETE FROM proposal_evaluations WHERE proposal_id = ?")
            .bind("proposal_eval_fair")
            .execute(&pool)
            .await;
        remove_cleanup_dir(cleanup_dir);
    }

    #[tokio::test]
    async fn evaluate_rejects_an_empty_fixture_corpus() {
        let (pool, cleanup_dir) = test_pool().await.unwrap();
        let proposal = measurable_proposal("proposal_eval_empty");
        let err = evaluate(&pool, &proposal, &[]).await.unwrap_err();
        assert!(
            matches!(err, AutospecError::Validation { .. }),
            "got {err:?}"
        );
        remove_cleanup_dir(cleanup_dir);
    }

    #[tokio::test]
    async fn evaluate_rejects_a_proposal_that_fails_the_section_22_gate() {
        let (pool, cleanup_dir) = test_pool().await.unwrap();
        // No measurable expected effect -> the §22 gate fails at evaluation.
        let mut proposal = measurable_proposal("proposal_eval_bad");
        proposal.expected_effect = ExpectedEffect::default();
        let fixtures = [fixture(
            "f1",
            ms(10, 100_000, 100, 10, 8, 10),
            ms(10, 100_000, 100, 10, 8, 10),
        )];
        let err = evaluate(&pool, &proposal, &fixtures).await.unwrap_err();
        assert!(
            matches!(err, AutospecError::Validation { .. }),
            "got {err:?}"
        );
        remove_cleanup_dir(cleanup_dir);
    }

    #[tokio::test]
    async fn load_evaluation_returns_none_for_unknown_proposal() {
        let (pool, cleanup_dir) = test_pool().await.unwrap();
        assert!(load_evaluation(&pool, "proposal_does_not_exist")
            .await
            .unwrap()
            .is_none());
        remove_cleanup_dir(cleanup_dir);
    }

    #[test]
    fn verdict_wire_names_roundtrip() {
        for verdict in [Verdict::Improved, Verdict::Inconclusive, Verdict::Regressed] {
            let parsed: Verdict = verdict.as_str().parse().unwrap();
            assert_eq!(parsed, verdict);
        }
        let err = "vibes".parse::<Verdict>().unwrap_err();
        assert!(matches!(err, AutospecError::Parse { .. }));
    }

    #[test]
    fn aggregate_folds_fixtures_element_wise() {
        let fixtures = [
            fixture("f1", ms(1, 100, 10, 1, 1, 2), ms(0, 0, 0, 0, 0, 0)),
            fixture("f2", ms(2, 200, 20, 2, 2, 3), ms(0, 0, 0, 0, 0, 0)),
            fixture("f3", ms(3, 300, 30, 3, 3, 5), ms(0, 0, 0, 0, 0, 0)),
        ];
        let baseline = aggregate(&fixtures, |f| &f.baseline);
        assert_eq!(baseline, ms(6, 600, 60, 6, 6, 10));
        assert_eq!(baseline.tokens_per_task(), 60.0);
        assert_eq!(baseline.success_rate(), 0.6);
    }

    /// Assert `Some(actual)` is within `1e-9` of `expected`.
    fn assert_approx(actual: Option<f64>, expected: f64) {
        let actual = actual.expect("delta should be defined");
        assert!(
            (actual - expected).abs() < 1e-9,
            "delta {actual} != expected {expected}"
        );
    }
}
