//! §24 self-improvement pull-request assembly with §25 separation of
//! duties.
//!
//! Spec: `docs/specs/2026-09-08-continuous-improvement-engine.md` §21
//! (self-improvement loop), §24 (PR workflow), §25 (separation of
//! duties), §34 (storage).
//!
//! §24: "Self-improvement PRs MUST be clearly labeled:
//! autospec-improvement, agent-policy, model-routing, skill-change,
//! tool-change, context-optimization".
//!
//! §25: "The model that creates a self-improvement proposal SHOULD NOT
//! be the sole reviewer." This module enforces that as a hard
//! precondition of [`open_pr`]: a review model must be assigned, and it
//! must differ from the model that created the proposal.
//!
//! Scope (§24 + §3 non-goals): this module only *creates* the pull
//! request. It never merges, approves, or lands anything —
//! [`ImprovementPr::merged`] is always `false` and no merge path
//! exists in this module ("MUST NOT automatically merge self-modifying
//! changes").
//!
//! A proposal with zero stored evaluations (`proposal_evaluations`,
//! §34) cannot become a pull request: evaluations are the §21
//! measurable gate that stands between a draft and a PR.

use sqlx::{AnyPool, Row};

use crate::error::AutospecError;
use crate::insights::config::InsightsConfig;
use crate::insights::proposals::schema::Proposal;

/// The six §24 labels every self-improvement pull request MUST carry,
/// in the order they are listed in the spec.
pub const SECTION_24_LABELS: [&str; 6] = [
    "autospec-improvement",
    "agent-policy",
    "model-routing",
    "skill-change",
    "tool-change",
    "context-optimization",
];

/// One stored evaluation of a proposal (§34 `proposal_evaluations`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Evaluation {
    pub proposal_id: String,
    pub seq: i64,
    pub evaluator: String,
    pub verdict: String,
    pub rationale: Option<String>,
}

/// §34 `proposal_evaluations` DDL — backend-neutral (TEXT/INTEGER on
/// both Postgres and SQLite), idempotent, and column-compatible with
/// the subsystem migration that creates the same table: this module
/// never reads or writes the timestamp column, so both layouts work.
const ENSURE_EVALUATIONS_TABLE: &str = r#"CREATE TABLE IF NOT EXISTS proposal_evaluations (
    proposal_id TEXT NOT NULL,
    seq INTEGER NOT NULL,
    evaluator TEXT NOT NULL,
    verdict TEXT NOT NULL,
    rationale TEXT,
    PRIMARY KEY (proposal_id, seq)
)"#;

/// Create the §34 `proposal_evaluations` table if it does not exist yet.
pub async fn ensure_schema(pool: &AnyPool) -> Result<(), AutospecError> {
    sqlx::query::<sqlx::Any>(ENSURE_EVALUATIONS_TABLE)
        .execute(pool)
        .await
        .map_err(|error| AutospecError::state("insights schema", error.to_string()))?;
    Ok(())
}

/// Persist one evaluation. An evaluation is a verdict record: the
/// proposal id, the evaluator, and the verdict must all be non-empty
/// (§25 — a review without a verdict is not a review).
pub async fn store_evaluation(
    pool: &AnyPool,
    evaluation: &Evaluation,
) -> Result<(), AutospecError> {
    if evaluation.proposal_id.trim().is_empty() {
        return Err(AutospecError::validation(
            "store_evaluation() requires a non-empty proposal_id",
        ));
    }
    if evaluation.evaluator.trim().is_empty() {
        return Err(AutospecError::validation(
            "store_evaluation() requires a non-empty evaluator",
        ));
    }
    if evaluation.verdict.trim().is_empty() {
        return Err(AutospecError::validation(
            "every stored evaluation must carry a verdict (§25)",
        ));
    }
    ensure_schema(pool).await?;
    sqlx::query::<sqlx::Any>(
        "INSERT INTO proposal_evaluations (proposal_id, seq, evaluator, verdict, rationale)\n\
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&evaluation.proposal_id)
    .bind(evaluation.seq)
    .bind(&evaluation.evaluator)
    .bind(&evaluation.verdict)
    .bind(&evaluation.rationale)
    .execute(pool)
    .await
    .map_err(|error| AutospecError::state("proposal_evaluations", error.to_string()))?;
    Ok(())
}

/// Read all stored evaluations of one proposal, ordered by `seq`.
pub async fn load_evaluations(
    pool: &AnyPool,
    proposal_id: &str,
) -> Result<Vec<Evaluation>, AutospecError> {
    ensure_schema(pool).await?;
    let rows = sqlx::query::<sqlx::Any>(
        "SELECT proposal_id, seq, evaluator, verdict, rationale\n\
         FROM proposal_evaluations\n\
         WHERE proposal_id = ?\n\
         ORDER BY seq",
    )
    .bind(proposal_id)
    .fetch_all(pool)
    .await
    .map_err(|error| AutospecError::state("proposal_evaluations", error.to_string()))?;
    rows.into_iter()
        .map(|row| {
            Ok(Evaluation {
                proposal_id: row.try_get(0).map_err(|error| {
                    AutospecError::state("proposal_evaluations", error.to_string())
                })?,
                seq: row.try_get(1).map_err(|error| {
                    AutospecError::state("proposal_evaluations", error.to_string())
                })?,
                evaluator: row.try_get(2).map_err(|error| {
                    AutospecError::state("proposal_evaluations", error.to_string())
                })?,
                verdict: row.try_get(3).map_err(|error| {
                    AutospecError::state("proposal_evaluations", error.to_string())
                })?,
                rationale: row.try_get(4).map_err(|error| {
                    AutospecError::state("proposal_evaluations", error.to_string())
                })?,
            })
        })
        .collect()
}

/// A self-improvement pull request created from a proposal (§24).
///
/// `merged` is always `false`: §3 forbids automatic merging of
/// self-modifying changes, and the §25 gate keeps the assigned
/// reviewer in the merge decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImprovementPr {
    pub proposal_id: String,
    pub branch: String,
    pub labels: Vec<String>,
    pub reviewer_model: String,
    pub merged: bool,
}

/// Assemble the §24 self-improvement pull request for a validated
/// proposal.
///
/// Hard preconditions, each a `Validation` error:
///
/// * `self_improvement.allow_auto_pr` is on (§45 gate);
/// * a review model is assigned to the proposal;
/// * the review model differs from `created_by_model` (§25 separation
///   of duties);
/// * the proposal has at least one stored evaluation with a verdict
///   (§21 measurable gate, §34 storage).
///
/// The returned [`ImprovementPr`] always carries all six §24 labels
/// and `merged: false`.
pub async fn open_pr(
    pool: &AnyPool,
    proposal: &Proposal,
    cfg: &InsightsConfig,
) -> Result<ImprovementPr, AutospecError> {
    if !cfg.self_improvement.allow_auto_pr {
        return Err(AutospecError::validation(
            "open_pr() is disabled: self_improvement.allow_auto_pr is false (§45)",
        ));
    }
    let Some(reviewer_model) = proposal.review_model.as_deref().map(str::trim) else {
        return Err(AutospecError::validation(
            "open_pr() requires an assigned review_model before a self-improvement PR can be opened (§25)",
        ));
    };
    if reviewer_model.is_empty() {
        return Err(AutospecError::validation(
            "open_pr() requires a non-empty review_model (§25)",
        ));
    }
    if reviewer_model == proposal.created_by_model.trim() {
        return Err(AutospecError::validation(format!(
            "§25 separation of duties: reviewer model '{reviewer_model}' must differ from the creating model '{}'",
            proposal.created_by_model
        )));
    }
    let evaluations = load_evaluations(pool, &proposal.proposal_id).await?;
    if evaluations.is_empty() {
        return Err(AutospecError::validation(format!(
            "open_pr() requires at least one stored evaluation with a verdict; proposal {} has none",
            proposal.proposal_id
        )));
    }
    Ok(ImprovementPr {
        proposal_id: proposal.proposal_id.clone(),
        branch: format!("autospec-improvement/{}", proposal.proposal_id),
        labels: SECTION_24_LABELS
            .iter()
            .map(|label| label.to_string())
            .collect(),
        reviewer_model: reviewer_model.to_string(),
        merged: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::insights::proposals::schema::{
        EvaluationPlan, Evidence, ExpectedEffect, ProposalStatus, ProposalType,
    };
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    static PR_COUNTER: AtomicU32 = AtomicU32::new(0);

    /// Test pool: the disposable `AUTOSPEC_TEST_DB_URL` database when
    /// the operator provided one (Postgres 16 under Apptainer in the
    /// full validation), else a throwaway on-disk SQLite database (a
    /// real database, not a mock) so the storage path stays covered
    /// without one. Returns the pool and, for the SQLite case, the
    /// directory to clean up.
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
                let index = PR_COUNTER.fetch_add(1, Ordering::SeqCst);
                let dir = std::env::temp_dir()
                    .join(format!("autospec-pr-test-{}-{index}", std::process::id()));
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

    /// A unique id per test run so the shared `AUTOSPEC_TEST_DB_URL`
    /// database never collides between runs.
    fn unique(tag: &str) -> String {
        format!(
            "{tag}_{}_{}",
            std::process::id(),
            PR_COUNTER.fetch_add(1, Ordering::SeqCst)
        )
    }

    fn cfg(allow_auto_pr: bool) -> InsightsConfig {
        let mut cfg = InsightsConfig::default();
        cfg.self_improvement.allow_auto_pr = allow_auto_pr;
        cfg
    }

    fn proposal(proposal_id: &str, created_by: &str, review_model: Option<&str>) -> Proposal {
        let finding_id = format!("finding_{proposal_id}");
        Proposal {
            proposal_id: proposal_id.to_string(),
            finding_id: finding_id.clone(),
            kind: ProposalType::default(),
            title: "self-improvement PR test".to_string(),
            status: ProposalStatus::default(),
            rationale: "test".to_string(),
            evidence: vec![Evidence::finding_citation(&finding_id)],
            expected_effect: ExpectedEffect::default(),
            risks: vec![],
            affected_components: vec![],
            patch: String::new(),
            evaluation_plan: EvaluationPlan::default(),
            created_by_model: created_by.to_string(),
            review_model: review_model.map(str::to_string),
        }
    }

    fn evaluation(proposal_id: &str, seq: i64, verdict: &str) -> Evaluation {
        Evaluation {
            proposal_id: proposal_id.to_string(),
            seq,
            evaluator: "gpt-5.6-sol".to_string(),
            verdict: verdict.to_string(),
            rationale: Some("test evaluation".to_string()),
        }
    }

    async fn cleanup(pool: &AnyPool, proposal_ids: &[String]) {
        for id in proposal_ids {
            let _ =
                sqlx::query::<sqlx::Any>("DELETE FROM proposal_evaluations WHERE proposal_id = ?")
                    .bind(id)
                    .execute(pool)
                    .await;
        }
    }

    #[test]
    fn the_six_section_24_labels_are_declared_in_spec_order() {
        assert_eq!(
            SECTION_24_LABELS,
            [
                "autospec-improvement",
                "agent-policy",
                "model-routing",
                "skill-change",
                "tool-change",
                "context-optimization",
            ]
        );
    }

    /// The TDD anchor for §25: the model that creates a proposal must
    /// not be its (sole) reviewer.
    #[tokio::test]
    async fn separation_of_duties_reviewer_must_differ_from_creator() {
        let (pool, cleanup_dir) = test_pool().await.unwrap();
        let id = unique("proposal");
        let proposal = proposal(&id, "qwen3-32b", Some("qwen3-32b"));
        store_evaluation(&pool, &evaluation(&id, 1, "pass"))
            .await
            .unwrap();
        let err = open_pr(&pool, &proposal, &cfg(true)).await.unwrap_err();
        assert!(
            matches!(err, AutospecError::Validation { .. }),
            "expected a separation-of-duties validation error, got: {err:?}"
        );
        cleanup(&pool, &[id]).await;
        remove_cleanup_dir(cleanup_dir);
    }

    #[tokio::test]
    async fn open_pr_errors_when_allow_auto_pr_is_false() {
        let (pool, cleanup_dir) = test_pool().await.unwrap();
        let id = unique("proposal");
        let proposal = proposal(&id, "qwen3-32b", Some("gpt-5.6-sol"));
        store_evaluation(&pool, &evaluation(&id, 1, "pass"))
            .await
            .unwrap();
        let err = open_pr(&pool, &proposal, &cfg(false)).await.unwrap_err();
        assert!(
            matches!(err, AutospecError::Validation { .. }),
            "expected a validation error, got: {err:?}"
        );
        cleanup(&pool, &[id]).await;
        remove_cleanup_dir(cleanup_dir);
    }

    #[tokio::test]
    async fn open_pr_errors_when_no_review_model_is_assigned() {
        let (pool, cleanup_dir) = test_pool().await.unwrap();
        let id = unique("proposal");
        let proposal = proposal(&id, "qwen3-32b", None);
        store_evaluation(&pool, &evaluation(&id, 1, "pass"))
            .await
            .unwrap();
        let err = open_pr(&pool, &proposal, &cfg(true)).await.unwrap_err();
        assert!(
            matches!(err, AutospecError::Validation { .. }),
            "expected a validation error, got: {err:?}"
        );
        cleanup(&pool, &[id]).await;
        remove_cleanup_dir(cleanup_dir);
    }

    #[tokio::test]
    async fn open_pr_errors_with_zero_stored_evaluations() {
        let (pool, cleanup_dir) = test_pool().await.unwrap();
        let id = unique("proposal");
        let proposal = proposal(&id, "qwen3-32b", Some("gpt-5.6-sol"));
        let err = open_pr(&pool, &proposal, &cfg(true)).await.unwrap_err();
        assert!(
            matches!(err, AutospecError::Validation { .. }),
            "expected a validation error, got: {err:?}"
        );
        cleanup(&pool, &[id]).await;
        remove_cleanup_dir(cleanup_dir);
    }

    #[tokio::test]
    async fn open_pr_returns_the_labeled_unmerged_pr_when_all_gates_pass() {
        let (pool, cleanup_dir) = test_pool().await.unwrap();
        let id = unique("proposal");
        let proposal = proposal(&id, "qwen3-32b", Some("gpt-5.6-sol"));
        store_evaluation(&pool, &evaluation(&id, 1, "pass"))
            .await
            .unwrap();
        let pr = open_pr(&pool, &proposal, &cfg(true)).await.unwrap();
        assert_eq!(pr.proposal_id, id);
        assert_eq!(pr.branch, format!("autospec-improvement/{id}"));
        assert_eq!(
            pr.labels,
            SECTION_24_LABELS
                .iter()
                .map(|label| label.to_string())
                .collect::<Vec<_>>()
        );
        assert_eq!(pr.reviewer_model, "gpt-5.6-sol");
        assert!(!pr.merged, "§3: a self-improvement PR is never auto-merged");
        cleanup(&pool, &[id]).await;
        remove_cleanup_dir(cleanup_dir);
    }

    #[tokio::test]
    async fn store_evaluation_requires_a_verdict() {
        let (pool, cleanup_dir) = test_pool().await.unwrap();
        let id = unique("proposal");
        for evaluation in [
            Evaluation::default(),
            evaluation(&id, 1, ""),
            Evaluation {
                proposal_id: id.clone(),
                seq: 1,
                evaluator: "   ".to_string(),
                verdict: "pass".to_string(),
                rationale: None,
            },
        ] {
            let err = store_evaluation(&pool, &evaluation).await.unwrap_err();
            assert!(
                matches!(err, AutospecError::Validation { .. }),
                "expected a validation error for {evaluation:?}, got: {err:?}"
            );
        }
        cleanup(&pool, &[id]).await;
        remove_cleanup_dir(cleanup_dir);
    }

    #[tokio::test]
    async fn evaluations_roundtrip_in_seq_order() {
        let (pool, cleanup_dir) = test_pool().await.unwrap();
        let id = unique("proposal");
        store_evaluation(&pool, &evaluation(&id, 2, "fail"))
            .await
            .unwrap();
        store_evaluation(&pool, &evaluation(&id, 1, "pass"))
            .await
            .unwrap();
        let stored = load_evaluations(&pool, &id).await.unwrap();
        assert_eq!(stored.len(), 2);
        assert_eq!(stored[0].seq, 1);
        assert_eq!(stored[0].verdict, "pass");
        assert_eq!(stored[1].seq, 2);
        assert_eq!(stored[1].verdict, "fail");
        assert_eq!(stored[0].evaluator, "gpt-5.6-sol");
        assert_eq!(stored[0].rationale.as_deref(), Some("test evaluation"));
        cleanup(&pool, &[id]).await;
        remove_cleanup_dir(cleanup_dir);
    }

    #[tokio::test]
    async fn load_evaluations_returns_empty_for_unknown_proposal() {
        let (pool, cleanup_dir) = test_pool().await.unwrap();
        let stored = load_evaluations(&pool, &unique("proposal")).await.unwrap();
        assert!(stored.is_empty());
        remove_cleanup_dir(cleanup_dir);
    }
}
