//! §4.1/§20 deterministic proposal drafting and §34 storage.
//!
//! Spec: `docs/specs/2026-09-08-continuous-improvement-engine.md` §4.1,
//! §20-§21, §34.
//!
//! [`draft`] is deterministic (§4.1 "deterministic first, semantic second"):
//! given a finding id and the drafting model it produces one structurally
//! valid `draft` record — proposal id, finding citation, evidence row — and
//! writes it to the §34 `improvement_proposals` table **only**. Nothing here
//! touches the filesystem tree: the `patch` column is text on a row, and a
//! draft stays a draft (§4.5). Strong-model diagnosis, §22 measurable-effect
//! enrichment and §25 reviewer assignment happen in later stages, not here.

use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::{AnyPool, Row};

use crate::error::AutospecError;

use super::schema::{
    EvaluationPlan, Evidence, ExpectedEffect, Proposal, ProposalStatus, ProposalType,
};

/// §34 `improvement_proposals` DDL — backend-neutral (TEXT + JSON on both
/// Postgres and SQLite), idempotent.
const ENSURE_PROPOSALS_TABLE: &str = r#"CREATE TABLE IF NOT EXISTS improvement_proposals (
    proposal_id TEXT PRIMARY KEY,
    finding_id TEXT NOT NULL,
    type TEXT NOT NULL,
    title TEXT NOT NULL,
    status TEXT NOT NULL,
    rationale TEXT NOT NULL,
    evidence JSON NOT NULL,
    expected_effect JSON NOT NULL,
    risks JSON NOT NULL,
    affected_components JSON NOT NULL,
    patch TEXT NOT NULL,
    evaluation_plan JSON NOT NULL,
    created_by_model TEXT NOT NULL,
    review_model TEXT,
    created_at TEXT NOT NULL
)"#;

/// Serialize one JSON column value to its portable TEXT form.
fn json_column(value: &serde_json::Value) -> Result<String, AutospecError> {
    serde_json::to_string(value)
        .map_err(|error| AutospecError::state("improvement_proposals", error.to_string()))
}

/// Decode one JSON column value from its portable TEXT form.
fn json_from_column<T: serde::de::DeserializeOwned>(raw: &str) -> Result<T, AutospecError> {
    serde_json::from_str(raw)
        .map_err(|error| AutospecError::state("improvement_proposals", error.to_string()))
}

/// Create the §34 `improvement_proposals` table if it does not exist yet.
pub async fn ensure_schema(pool: &AnyPool) -> Result<(), AutospecError> {
    sqlx::query::<sqlx::Any>(ENSURE_PROPOSALS_TABLE)
        .execute(pool)
        .await
        .map_err(|error| AutospecError::state("insights schema", error.to_string()))?;
    Ok(())
}

/// Unix-epoch seconds as the portable `created_at` value.
fn now_unix_seconds() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as u128)
        .unwrap_or(0)
}

/// A deterministic, unique proposal id: `proposal_` + 16 hex chars of
/// SHA-256(finding_id, model, clock).
fn new_proposal_id(finding_id: &str, model: &str) -> String {
    let now = now_unix_seconds();
    let mut digest = Sha256::new();
    digest.update(finding_id.as_bytes());
    digest.update([0u8]);
    digest.update(model.as_bytes());
    digest.update([0u8]);
    digest.update(now.to_string().as_bytes());
    let hex: String = digest
        .finalize()
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("proposal_{hex}")
}

/// §4.1: deterministically draft a proposal for `finding_id` and store it in
/// the §34 `improvement_proposals` table.
///
/// The result is a structurally valid `draft` record: it cites the finding
/// and carries one `finding:<id>` evidence row, so it satisfies the
/// data-integrity contract every stored proposal satisfies. It deliberately
/// carries an *unmeasured* expected effect and no patch — the §22
/// measurability gate, strong-model diagnosis and patch generation are later
/// stages, and [`Proposal::validate`] stays red on the raw draft until they
/// run.
///
/// This function writes to `improvement_proposals` and nothing else: it does
/// not create, edit or delete any file, and the returned record is always
/// `status: draft`.
pub async fn draft(
    pool: &AnyPool,
    finding_id: &str,
    model: &str,
) -> Result<Proposal, AutospecError> {
    let finding_id = finding_id.trim();
    let model = model.trim();
    if finding_id.is_empty() {
        return Err(AutospecError::validation(
            "draft() requires a non-empty finding_id",
        ));
    }
    if model.is_empty() {
        return Err(AutospecError::validation(
            "draft() requires a non-empty model name",
        ));
    }
    let proposal = Proposal {
        proposal_id: new_proposal_id(finding_id, model),
        finding_id: finding_id.to_string(),
        kind: ProposalType::default(),
        title: format!("Draft improvement for finding {finding_id}"),
        status: ProposalStatus::default(),
        rationale: format!(
            "Deterministic draft for {finding_id} created by {model} (§4.1: deterministic \
             first, semantic second); strong-model diagnosis, §22 measurable-effect \
             enrichment and patch generation are later stages."
        ),
        evidence: vec![Evidence::finding_citation(finding_id)],
        expected_effect: ExpectedEffect::default(),
        risks: Vec::new(),
        affected_components: Vec::new(),
        patch: String::new(),
        evaluation_plan: EvaluationPlan::default(),
        created_by_model: model.to_string(),
        review_model: None,
    };
    store(pool, &proposal).await?;
    Ok(proposal)
}

/// Persist a proposal to the §34 `improvement_proposals` table.
///
/// Enforces the data-integrity contract: a non-empty `proposal_id`, exactly
/// one non-empty `finding_id`, and at least one identifiable evidence row
/// (§35). The §22 measurability gate ([`Proposal::validate`]) is *not*
/// applied here — drafts legitimately predate measurable-effect enrichment —
/// it is applied at the evaluation stage.
pub async fn store(pool: &AnyPool, proposal: &Proposal) -> Result<(), AutospecError> {
    if proposal.proposal_id.trim().is_empty() {
        return Err(AutospecError::validation(
            "store() requires a non-empty proposal_id",
        ));
    }
    if proposal.finding_id.trim().is_empty() {
        return Err(AutospecError::validation(
            "every stored proposal must cite one finding_id (§35)",
        ));
    }
    if proposal.evidence.iter().all(|row| !row.identified()) {
        return Err(AutospecError::validation(
            "every stored proposal must cite at least one identifiable evidence row (§35)",
        ));
    }
    ensure_schema(pool).await?;
    sqlx::query::<sqlx::Any>(
        "INSERT INTO improvement_proposals (\n\
             proposal_id, finding_id, type, title, status, rationale, evidence, expected_effect,\n\
             risks, affected_components, patch, evaluation_plan, created_by_model, review_model,\n\
             created_at\n\
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&proposal.proposal_id)
    .bind(&proposal.finding_id)
    .bind(proposal.kind.as_str())
    .bind(&proposal.title)
    .bind(proposal.status.as_str())
    .bind(&proposal.rationale)
    .bind(json_column(&json!(proposal.evidence))?)
    .bind(json_column(&json!(proposal.expected_effect))?)
    .bind(json_column(&json!(proposal.risks))?)
    .bind(json_column(&json!(proposal.affected_components))?)
    .bind(&proposal.patch)
    .bind(json_column(&json!(proposal.evaluation_plan))?)
    .bind(&proposal.created_by_model)
    .bind(&proposal.review_model)
    .bind(now_unix_seconds().to_string())
    .execute(pool)
    .await
    .map_err(|error| AutospecError::state("improvement_proposals", error.to_string()))?;
    Ok(())
}

/// Read one proposal back by id; `None` when no such row exists.
pub async fn load(pool: &AnyPool, proposal_id: &str) -> Result<Option<Proposal>, AutospecError> {
    ensure_schema(pool).await?;
    let row = sqlx::query::<sqlx::Any>(
        "SELECT proposal_id, finding_id, type, title, status, rationale, evidence,\n\
             expected_effect, risks, affected_components, patch, evaluation_plan,\n\
             created_by_model, review_model\n\
         FROM improvement_proposals\n\
         WHERE proposal_id = ?",
    )
    .bind(proposal_id)
    .fetch_optional(pool)
    .await
    .map_err(|error| AutospecError::state("improvement_proposals", error.to_string()))?;
    let Some(row) = row else {
        return Ok(None);
    };
    let decode = |index: usize| -> Result<String, AutospecError> {
        row.try_get(index)
            .map_err(|error| AutospecError::state("improvement_proposals", error.to_string()))
    };
    let kind: String = decode(2)?;
    let status: String = decode(4)?;
    let evidence: Vec<Evidence> = json_from_column(&decode(6)?)?;
    let expected_effect: ExpectedEffect = json_from_column(&decode(7)?)?;
    let risks: Vec<String> = json_from_column(&decode(8)?)?;
    let affected_components: Vec<String> = json_from_column(&decode(9)?)?;
    let evaluation_plan: EvaluationPlan = json_from_column(&decode(11)?)?;
    let review_model: Option<String> = row
        .try_get(13)
        .map_err(|error| AutospecError::state("improvement_proposals", error.to_string()))?;
    Ok(Some(Proposal {
        proposal_id: row
            .try_get(0)
            .map_err(|error| AutospecError::state("improvement_proposals", error.to_string()))?,
        finding_id: row
            .try_get(1)
            .map_err(|error| AutospecError::state("improvement_proposals", error.to_string()))?,
        kind: kind.parse()?,
        title: row
            .try_get(3)
            .map_err(|error| AutospecError::state("improvement_proposals", error.to_string()))?,
        status: status.parse()?,
        rationale: row
            .try_get(5)
            .map_err(|error| AutospecError::state("improvement_proposals", error.to_string()))?,
        evidence,
        expected_effect,
        risks,
        affected_components,
        patch: decode(10)?,
        evaluation_plan,
        created_by_model: row
            .try_get(12)
            .map_err(|error| AutospecError::state("improvement_proposals", error.to_string()))?,
        review_model,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::insights::proposals::schema::Evidence as EvidenceRow;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    static SQLITE_COUNTER: AtomicU32 = AtomicU32::new(0);

    /// Test pool: the disposable `AUTOSPEC_TEST_DB_URL` database when the
    /// operator provided one (Postgres 16 under Apptainer in the full
    /// validation), else a throwaway on-disk SQLite database (a real
    /// database, not a mock) so the storage path stays covered without one.
    /// Returns the pool and, for the SQLite case, the directory to clean up.
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
                    "autospec-proposals-test-{}-{index}",
                    std::process::id()
                ));
                let url = format!("sqlite://{}/test.db", dir.display());
                Ok((crate::resources::db::open_shared_db(&url).await?, Some(dir)))
            }
        }
    }

    async fn cleanup(pool: &AnyPool, proposal_ids: &[String]) {
        for id in proposal_ids {
            let _ =
                sqlx::query::<sqlx::Any>("DELETE FROM improvement_proposals WHERE proposal_id = ?")
                    .bind(id)
                    .execute(pool)
                    .await;
        }
    }

    /// Remove the throwaway SQLite directory, if any.
    fn remove_cleanup_dir(cleanup_dir: Option<PathBuf>) {
        if let Some(dir) = cleanup_dir {
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    fn raw_proposal(proposal_id: &str, finding_id: &str, evidence: Vec<EvidenceRow>) -> Proposal {
        Proposal {
            proposal_id: proposal_id.to_string(),
            finding_id: finding_id.to_string(),
            kind: ProposalType::AgentInstruction,
            title: "stored proposal".to_string(),
            status: ProposalStatus::default(),
            rationale: "test".to_string(),
            evidence,
            expected_effect: ExpectedEffect {
                token_reduction_pct: Some(5),
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

    #[tokio::test]
    async fn draft_writes_a_draft_row_citing_finding_and_evidence() {
        let (pool, cleanup_dir) = test_pool().await.unwrap();
        let finding_id = format!("finding_draft_{}", now_unix_seconds());
        let proposal = draft(&pool, &finding_id, "qwen3-32b").await.unwrap();
        assert_eq!(proposal.status, ProposalStatus::Draft);
        assert_eq!(proposal.finding_id, finding_id);
        assert_eq!(proposal.created_by_model, "qwen3-32b");
        assert!(proposal.proposal_id.starts_with("proposal_"));
        assert!(
            proposal.evidence.iter().any(EvidenceRow::identified),
            "draft must cite at least one evidence row"
        );
        let stored = load(&pool, &proposal.proposal_id).await.unwrap();
        let stored = stored.expect("draft must be readable back");
        assert_eq!(stored.finding_id, finding_id);
        assert_eq!(stored.status, ProposalStatus::Draft);
        assert_eq!(stored.created_by_model, "qwen3-32b");
        assert!(
            stored.evidence.iter().any(EvidenceRow::identified),
            "stored draft must cite at least one evidence row"
        );
        cleanup(&pool, &[proposal.proposal_id.clone()]).await;
        remove_cleanup_dir(cleanup_dir);
    }

    #[tokio::test]
    async fn draft_writes_only_the_improvement_proposals_table() {
        let (pool, cleanup_dir) = test_pool().await.unwrap();
        let before: Vec<String> = table_names(&pool).await;
        let proposal = draft(&pool, "finding_table_scope", "qwen3-32b")
            .await
            .unwrap();
        let after = table_names(&pool).await;
        let new_tables: Vec<&String> = after
            .iter()
            .filter(|table| !before.contains(table))
            .collect();
        assert!(
            new_tables.iter().all(|table| table.as_str() == "improvement_proposals"),
            "draft() must not create any table other than improvement_proposals; created: {new_tables:?}"
        );
        cleanup(&pool, &[proposal.proposal_id]).await;
        remove_cleanup_dir(cleanup_dir);
    }

    /// User table names visible to this pool: the SQLite catalog on the
    /// throwaway database, or the `public` schema on Postgres.
    async fn table_names(pool: &AnyPool) -> Vec<String> {
        let sqlite =
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'";
        let postgres =
            "SELECT table_name FROM information_schema.tables WHERE table_schema = 'public'";
        let rows = match sqlx::query::<sqlx::Any>(sqlite).fetch_all(pool).await {
            Ok(rows) => rows,
            Err(_) => sqlx::query::<sqlx::Any>(postgres)
                .fetch_all(pool)
                .await
                .unwrap_or_default(),
        };
        rows.into_iter()
            .map(|row| row.try_get::<String, _>(0).unwrap_or_default())
            .collect()
    }

    #[tokio::test]
    async fn draft_leaves_the_filesystem_tree_untouched() {
        let (pool, cleanup_dir) = test_pool().await.unwrap();
        let tree = std::env::temp_dir().join(format!(
            "autospec-proposals-tree-{}-{}",
            std::process::id(),
            SQLITE_COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&tree).unwrap();
        let before: Vec<String> = std::fs::read_dir(&tree)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();

        // A stored proposal carries its patch as text; storing it must not
        // materialize any file in the tree (§4.5, AC "leaves the tree untouched").
        let mut proposal = raw_proposal(
            "proposal_tree_untouched",
            "finding_tree",
            vec![EvidenceRow::finding_citation("finding_tree")],
        );
        proposal.patch =
            "--- a/AGENTS.md\n+++ b/AGENTS.md\n+Never use helper X for service code.\n".to_string();
        store(&pool, &proposal).await.unwrap();
        assert_eq!(
            load(&pool, "proposal_tree_untouched")
                .await
                .unwrap()
                .unwrap()
                .patch,
            proposal.patch,
            "patch text must round-trip as data"
        );

        let after: Vec<String> = std::fs::read_dir(&tree)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(before, after, "the tree must be untouched by draft/store");
        cleanup(&pool, &["proposal_tree_untouched".to_string()]).await;
        let _ = std::fs::remove_dir_all(&tree);
        remove_cleanup_dir(cleanup_dir);
    }

    #[tokio::test]
    async fn store_rejects_a_proposal_without_a_finding() {
        let (pool, cleanup_dir) = test_pool().await.unwrap();
        let proposal = raw_proposal(
            "proposal_no_finding",
            "   ",
            vec![EvidenceRow::finding_citation("finding_x")],
        );
        let err = store(&pool, &proposal).await.unwrap_err();
        assert!(
            matches!(err, AutospecError::Validation { .. }),
            "got {err:?}"
        );
        remove_cleanup_dir(cleanup_dir);
    }

    #[tokio::test]
    async fn store_rejects_a_proposal_without_identifiable_evidence() {
        let (pool, cleanup_dir) = test_pool().await.unwrap();
        let proposal = raw_proposal("proposal_no_evidence", "finding_x", vec![]);
        let err = store(&pool, &proposal).await.unwrap_err();
        assert!(
            matches!(err, AutospecError::Validation { .. }),
            "got {err:?}"
        );
        remove_cleanup_dir(cleanup_dir);
    }

    #[tokio::test]
    async fn draft_rejects_empty_inputs() {
        let (pool, cleanup_dir) = test_pool().await.unwrap();
        for (finding_id, model) in [("", "qwen3-32b"), ("finding_1", ""), ("   ", "  ")] {
            let err = draft(&pool, finding_id, model).await.unwrap_err();
            assert!(
                matches!(err, AutospecError::Validation { .. }),
                "got {err:?}"
            );
        }
        remove_cleanup_dir(cleanup_dir);
    }

    #[tokio::test]
    async fn load_returns_none_for_unknown_proposal_id() {
        let (pool, cleanup_dir) = test_pool().await.unwrap();
        assert!(load(&pool, "proposal_does_not_exist")
            .await
            .unwrap()
            .is_none());
        remove_cleanup_dir(cleanup_dir);
    }
}
