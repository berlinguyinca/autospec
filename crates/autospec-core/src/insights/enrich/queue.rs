//! Resumable enrichment job queue (issue #3848, spec §37).
//!
//! Jobs live in the `enrichment_jobs` table (dependency issue #3827's
//! job-row pattern) as backend-neutral DDL shared by the SQLite and
//! Postgres `AnyPool` backends (ADR 0001 D10). A killed worker never
//! loses progress: each job carries a `cursor` into its payload and a
//! crash-safe status transition (`running` jobs are recovered to
//! `pending` at the start of every run), so a queue stopped after 5 of
//! 20 jobs resumes and completes the remaining 15.
//!
//! Dispatch gates: every batch is re-redacted through
//! [`super::redact::Redactor`] and every session must pass
//! [`super::redact::repo_allowed`] before it reaches the
//! [`super::Enricher`] — even though the queue only ever holds redacted
//! payload evidence.

use sqlx::{AnyPool, Row};

use crate::error::AutospecError;
use crate::insights::enrich::redact::{repo_allowed, Redactor};
use crate::insights::enrich::{Enricher, Enrichment, EnrichmentBatch, EnrichmentJob, JobStatus};

/// Create the `enrichment_jobs` table if absent (idempotent, shared by
/// both backends).
const ENRICHMENT_JOBS_DDL: &str = "CREATE TABLE IF NOT EXISTS enrichment_jobs (\
    id TEXT PRIMARY KEY, \
    session_id TEXT NOT NULL, \
    stage TEXT NOT NULL, \
    repo TEXT, \
    payload TEXT NOT NULL, \
    enrichments TEXT NOT NULL, \
    total INTEGER NOT NULL, \
    cursor INTEGER NOT NULL, \
    attempts INTEGER NOT NULL, \
    status TEXT NOT NULL \
)";

/// Tunables for [`run_queue`].
#[derive(Debug, Clone)]
pub struct QueueConfig {
    /// Enricher attempts before a job is marked `failed` (default 3).
    pub max_attempts: u64,
    /// Payload items per dispatch (default 1).
    pub chunk_size: u64,
    /// Repository allowlist (spec §39); empty denies every session.
    pub allowed_repos: Vec<String>,
}

impl Default for QueueConfig {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            chunk_size: 1,
            allowed_repos: Vec::new(),
        }
    }
}

/// Counters from one [`run_queue`] pass.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct QueueOutcome {
    pub completed: u64,
    pub skipped: u64,
}

fn map_error(operation: &str, error: sqlx::Error) -> AutospecError {
    AutospecError::state("enrichment_jobs", format!("{operation}: {error}"))
}

pub(super) async fn ensure_enrichment_jobs(pool: &AnyPool) -> Result<(), AutospecError> {
    sqlx::query(ENRICHMENT_JOBS_DDL)
        .execute(pool)
        .await
        .map_err(|error| map_error("ensure schema", error))
        .map(|_| ())
}

fn job_payload(job: &EnrichmentJob) -> Result<String, AutospecError> {
    serde_json::to_string(&job.payload)
        .map_err(|error| AutospecError::parse("enrichment payload", error.to_string()))
}

/// Insert or ignore jobs (idempotent on `id`).
pub async fn enqueue_jobs(pool: &AnyPool, jobs: &[EnrichmentJob]) -> Result<(), AutospecError> {
    ensure_enrichment_jobs(pool).await?;
    for job in jobs {
        if job.payload.len() as u64 != job.total {
            return Err(AutospecError::validation(format!(
                "job {}: total {} does not match payload length {}",
                job.id,
                job.total,
                job.payload.len()
            )));
        }
        sqlx::query(
            "INSERT INTO enrichment_jobs (\
             id, session_id, stage, repo, payload, enrichments, total, cursor, attempts, status) \
             VALUES ($1, $2, $3, $4, $5, '[]', $6, 0, 0, 'pending') \
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(&job.id)
        .bind(&job.session_id)
        .bind(&job.stage)
        .bind(&job.repo)
        .bind(job_payload(job)?)
        .bind(job.total as i64)
        .execute(pool)
        .await
        .map_err(|error| map_error("enqueue", error))?;
    }
    Ok(())
}

/// Read one job row.
pub(super) async fn load_job(pool: &AnyPool, id: &str) -> Result<EnrichmentJob, AutospecError> {
    let row = sqlx::query(
        "SELECT session_id, stage, repo, payload, total, cursor, attempts, status \
         FROM enrichment_jobs WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|error| map_error("load job", error))?
    .ok_or_else(|| AutospecError::state("enrichment_jobs", format!("job {id} not found")))?;
    let payload: Vec<String> = serde_json::from_str(
        &row.try_get::<String, _>(3)
            .map_err(|error| map_error("decode payload", error))?,
    )
    .map_err(|error| AutospecError::parse("enrichment payload", error.to_string()))?;
    Ok(EnrichmentJob {
        id: id.to_string(),
        session_id: row
            .try_get::<String, _>(0)
            .map_err(|e| map_error("row", e))?,
        stage: row
            .try_get::<String, _>(1)
            .map_err(|e| map_error("row", e))?,
        repo: row
            .try_get::<Option<String>, _>(2)
            .map_err(|e| map_error("row", e))?,
        payload,
        total: row.try_get::<i64, _>(4).map_err(|e| map_error("row", e))? as u64,
        cursor: row.try_get::<i64, _>(5).map_err(|e| map_error("row", e))? as u64,
        attempts: row.try_get::<i64, _>(6).map_err(|e| map_error("row", e))? as u64,
        status: JobStatus::from_str(
            &row.try_get::<String, _>(7)
                .map_err(|e| map_error("row", e))?,
        ),
    })
}

async fn update_job(
    pool: &AnyPool,
    id: &str,
    status: JobStatus,
    cursor: u64,
    attempts: u64,
) -> Result<(), AutospecError> {
    sqlx::query("UPDATE enrichment_jobs SET status = $1, cursor = $2, attempts = $3 WHERE id = $4")
        .bind(status.as_str())
        .bind(cursor as i64)
        .bind(attempts as i64)
        .bind(id)
        .execute(pool)
        .await
        .map_err(|error| map_error("update job", error))?;
    Ok(())
}

async fn append_enrichments(
    pool: &AnyPool,
    id: &str,
    new: &[Enrichment],
) -> Result<(), AutospecError> {
    let row = sqlx::query("SELECT enrichments FROM enrichment_jobs WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .map_err(|error| map_error("read enrichments", error))?;
    let raw = row
        .try_get::<String, _>(0)
        .map_err(|e| map_error("row", e))?;
    let existing: Vec<serde_json::Value> = serde_json::from_str(&raw)
        .map_err(|error| AutospecError::parse("enrichment results", error.to_string()))?;
    let mut merged = existing;
    for enrichment in new {
        merged.push(
            serde_json::to_value(enrichment)
                .map_err(|error| AutospecError::parse("enrichment results", error.to_string()))?,
        );
    }
    sqlx::query("UPDATE enrichment_jobs SET enrichments = $1 WHERE id = $2")
        .bind(
            serde_json::to_string(&merged)
                .map_err(|error| AutospecError::parse("enrichment results", error.to_string()))?,
        )
        .bind(id)
        .execute(pool)
        .await
        .map_err(|error| map_error("append enrichments", error))?;
    Ok(())
}

/// A worker that died mid-run leaves its job `running`; recovery resets
/// those to `pending` so the next pass resumes from the stored cursor.
pub async fn recover_running(pool: &AnyPool) -> Result<u64, AutospecError> {
    ensure_enrichment_jobs(pool).await?;
    let result =
        sqlx::query("UPDATE enrichment_jobs SET status = 'pending' WHERE status = 'running'")
            .execute(pool)
            .await
            .map_err(|error| map_error("recover running", error))?;
    Ok(result.rows_affected())
}

async fn claim_next(pool: &AnyPool) -> Result<Option<EnrichmentJob>, AutospecError> {
    let row =
        sqlx::query("SELECT id FROM enrichment_jobs WHERE status = 'pending' ORDER BY id LIMIT 1")
            .fetch_optional(pool)
            .await
            .map_err(|error| map_error("claim job", error))?;
    let Some(id) = row.and_then(|r| r.try_get::<String, _>(0).ok()) else {
        return Ok(None);
    };
    // Claim flips status only: the stored cursor and attempts survive so a
    // resumed job continues exactly where it stopped.
    sqlx::query("UPDATE enrichment_jobs SET status = 'running' WHERE id = $1")
        .bind(&id)
        .execute(pool)
        .await
        .map_err(|error| map_error("claim job", error))?;
    Ok(Some(load_job(pool, &id).await?))
}

/// Drain the queue: claim pending jobs (lowest id first), pass each batch
/// through the redaction gate and the repo allowlist, dispatch to
/// `enricher`, and persist progress.
///
/// On an enricher error the job's `attempts` is incremented, the cursor is
/// saved, and the run stops with an error — re-run [`run_queue`] to resume
/// (a job past `max_attempts` is marked `failed` and stays queryable).
pub async fn run_queue(
    pool: &AnyPool,
    enricher: &dyn Enricher,
    cfg: &QueueConfig,
) -> Result<QueueOutcome, AutospecError> {
    ensure_enrichment_jobs(pool).await?;
    recover_running(pool).await?;
    let mut outcome = QueueOutcome::default();
    loop {
        let Some(job) = claim_next(pool).await? else {
            return Ok(outcome);
        };
        if !repo_allowed(job.repo.as_deref(), &cfg.allowed_repos) {
            eprintln!(
                "insights.enrich: job {} skipped: repo {:?} is not in the allowlist; \
                 session {} stays unenriched",
                job.id, job.repo, job.session_id
            );
            update_job(pool, &job.id, JobStatus::Skipped, job.cursor, job.attempts).await?;
            outcome.skipped += 1;
            continue;
        }
        let mut cursor = job.cursor;
        let mut failed = false;
        while cursor < job.total && !failed {
            let end = (cursor + cfg.chunk_size.max(1)).min(job.total);
            let items: Vec<String> = job.payload[cursor as usize..end as usize]
                .iter()
                .map(|item| Redactor::redact(item))
                .collect();
            let batch = EnrichmentBatch {
                session_id: job.session_id.clone(),
                stage: job.stage.clone(),
                repo: job.repo.clone(),
                cursor,
                items,
            };
            match enricher.enrich(&batch) {
                Ok(enrichments) => {
                    append_enrichments(pool, &job.id, &enrichments).await?;
                    cursor = end;
                }
                Err(error) => {
                    let attempts = job.attempts + 1;
                    let status = if attempts >= cfg.max_attempts {
                        JobStatus::Failed
                    } else {
                        JobStatus::Pending
                    };
                    update_job(pool, &job.id, status, cursor, attempts).await?;
                    eprintln!(
                        "insights.enrich: job {} attempt {}/{} {} ({}): {error}",
                        job.id,
                        attempts,
                        cfg.max_attempts,
                        status.as_str(),
                        if status == JobStatus::Pending {
                            "retryable"
                        } else {
                            "giving up"
                        }
                    );
                    failed = true;
                }
            }
        }
        if failed {
            return Err(AutospecError::state(
                "enrichment_jobs",
                format!(
                    "job {} failed; queue stopped, re-run run_queue to resume",
                    job.id
                ),
            ));
        }
        update_job(pool, &job.id, JobStatus::Done, job.total, job.attempts).await?;
        outcome.completed += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());
    static COUNTER: AtomicU32 = AtomicU32::new(0);

    /// Test database URL: `AUTOSPEC_TEST_DB_URL` (disposable PostgreSQL 16
    /// under Apptainer in the operator full run) when set; otherwise a
    /// disposable per-test SQLite file. Always a real database — no
    /// database mocks.
    fn test_db_url() -> String {
        match std::env::var("AUTOSPEC_TEST_DB_URL") {
            Ok(url) if !url.trim().is_empty() => url,
            _ => format!(
                "sqlite://{}/autospec-enrich-queue-{}-{}.db",
                std::env::temp_dir().display(),
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::SeqCst)
            ),
        }
    }

    async fn open_test_pool() -> AnyPool {
        let pool = crate::resources::db::open_shared_db(&test_db_url())
            .await
            .expect("test database must open");
        ensure_enrichment_jobs(&pool).await.expect("ddl");
        // The caller holds ENV_LOCK and all queue tests share one database,
        // so clear rows left by earlier runs before this test claims jobs.
        sqlx::query("DELETE FROM enrichment_jobs")
            .execute(&pool)
            .await
            .expect("table clear");
        pool
    }

    fn unique(prefix: &str) -> String {
        format!(
            "{prefix}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        )
    }

    fn job(id: &str, repo: Option<&str>, items: &[&str]) -> EnrichmentJob {
        EnrichmentJob {
            id: id.to_string(),
            session_id: format!("session-{id}"),
            stage: "embedding".into(),
            repo: repo.map(str::to_string),
            payload: items.iter().map(|item| item.to_string()).collect(),
            total: items.len() as u64,
            cursor: 0,
            attempts: 0,
            status: JobStatus::Pending,
        }
    }

    /// Stub enricher: records every batch it receives (proving what
    /// actually reached the backend) and returns one canned enrichment
    /// per item. Optional failure injection after N calls.
    struct StubEnricher {
        batches: Mutex<Vec<EnrichmentBatch>>,
        fail_after: Mutex<Option<usize>>,
    }

    impl StubEnricher {
        fn new() -> Self {
            Self {
                batches: Mutex::new(Vec::new()),
                fail_after: Mutex::new(None),
            }
        }
        fn fail_after(self, n: usize) -> Self {
            *self.fail_after.lock().unwrap() = Some(n);
            self
        }
    }

    impl Enricher for StubEnricher {
        fn enrich(&self, batch: &EnrichmentBatch) -> Result<Vec<Enrichment>, AutospecError> {
            let count;
            {
                let mut guard = self.batches.lock().unwrap();
                count = guard.len();
                guard.push(batch.clone());
            }
            if let Some(limit) = *self.fail_after.lock().unwrap() {
                if count + 1 > limit {
                    return Err(AutospecError::other("injected enricher failure"));
                }
            }
            Ok(batch
                .items
                .iter()
                .enumerate()
                .map(|(i, item)| Enrichment {
                    session_id: batch.session_id.clone(),
                    stage: batch.stage.clone(),
                    index: batch.cursor + i as u64,
                    kind: "stub".into(),
                    value: item.clone(),
                    model: "stub".into(),
                })
                .collect())
        }
    }

    async fn statuses(pool: &AnyPool, prefix: &str) -> Vec<(String, String)> {
        sqlx::raw_sql(
            format!("SELECT id, status FROM enrichment_jobs WHERE id LIKE '{prefix}%' ORDER BY id")
                .as_str(),
        )
        .fetch_all(pool)
        .await
        .unwrap()
        .iter()
        .map(|row| {
            (
                row.try_get::<String, _>(0).unwrap(),
                row.try_get::<String, _>(1).unwrap(),
            )
        })
        .collect()
    }

    /// TDD redaction-gate test: a stub Enricher proves every payload item
    /// that reaches it has passed `Redactor::redact` — no secret shape
    /// survives the gate.
    #[tokio::test]
    async fn redaction_gate_secrets_never_reach_the_enricher() {
        let _guard = ENV_LOCK.lock().unwrap();
        let pool = open_test_pool().await;
        let id = unique("redact");
        let secret = "AKIAIOSFODNN7EXAMPLE"; // linter:allow-SECURITY AWS doc example key, non-secret fixture
        enqueue_jobs(
            &pool,
            &[job(
                &id,
                Some("acme/web"),
                &[
                    format!("deploy key {secret} for prod").as_str(),
                    "clean payload",
                ],
            )],
        )
        .await
        .unwrap();

        let enricher = StubEnricher::new();
        let cfg = QueueConfig {
            allowed_repos: vec!["acme".into()],
            ..Default::default()
        };
        let outcome = run_queue(&pool, &enricher, &cfg).await.unwrap();
        assert_eq!(outcome.completed, 1);

        let batches = enricher.batches.lock().unwrap();
        assert_eq!(batches.len(), 2, "chunk_size 1 dispatches per item");
        let first = &batches[0].items[0];
        assert!(!first.contains(secret), "AWS key leaked: {first}");
        assert!(first.contains("[REDACTED:aws-access-key]"), "{first}");
        assert_eq!(&batches[1].items[0], "clean payload");
    }

    /// `repo_allowed == false`: the session is skipped, never dispatched,
    /// and the reason is logged (skipped status is the queryable record).
    #[tokio::test]
    async fn repo_not_allowed_session_stays_unenriched() {
        let _guard = ENV_LOCK.lock().unwrap();
        let pool = open_test_pool().await;
        let id = unique("denied");
        enqueue_jobs(&pool, &[job(&id, Some("sensitive/core"), &["payload"])])
            .await
            .unwrap();

        let enricher = StubEnricher::new();
        let cfg = QueueConfig {
            allowed_repos: vec!["acme".into()],
            ..Default::default()
        };
        let outcome = run_queue(&pool, &enricher, &cfg).await.unwrap();
        assert_eq!(
            outcome,
            QueueOutcome {
                completed: 0,
                skipped: 1
            }
        );
        assert!(
            enricher.batches.lock().unwrap().is_empty(),
            "a denied session must never be dispatched"
        );
        let row =
            sqlx::raw_sql(format!("SELECT status FROM enrichment_jobs WHERE id = '{id}'").as_str())
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(row.try_get::<String, _>(0).unwrap(), "skipped");
    }

    /// A queue stopped after 5 of 20 jobs resumes and completes the
    /// remaining 15.
    #[tokio::test]
    async fn queue_stopped_after_five_of_twenty_resumes_and_completes_fifteen() {
        let _guard = ENV_LOCK.lock().unwrap();
        let pool = open_test_pool().await;
        let prefix = unique("resume20");
        let ids: Vec<String> = (0..20).map(|i| format!("{prefix}-{i:02}")).collect();
        let jobs: Vec<EnrichmentJob> = ids
            .iter()
            .map(|id| job(id, Some("acme/web"), &["payload"]))
            .collect();
        enqueue_jobs(&pool, &jobs).await.unwrap();

        // Run 1: the enricher dies after 5 successful jobs.
        let cfg = QueueConfig {
            allowed_repos: vec!["acme".into()],
            ..Default::default()
        };
        let dead = StubEnricher::new().fail_after(5);
        assert!(
            run_queue(&pool, &dead, &cfg).await.is_err(),
            "the run must stop with an error"
        );
        let done = statuses(&pool, &prefix)
            .await
            .into_iter()
            .filter(|(_, s)| s == "done")
            .count();
        assert_eq!(done, 5, "exactly 5 jobs completed before the stop");

        // Run 2: a healthy enricher resumes and finishes the 15.
        let healthy = StubEnricher::new();
        let outcome = run_queue(&pool, &healthy, &cfg).await.unwrap();
        assert_eq!(outcome.completed, 15);
        let all = statuses(&pool, &prefix).await;
        assert!(all.iter().all(|(_, s)| s == "done"), "{all:?}");
        assert_eq!(healthy.batches.lock().unwrap().len(), 15);
    }

    /// A stopped queue resumes from its cursor: no item before the cursor
    /// is re-dispatched, and the job that failed carries attempts > 0.
    #[tokio::test]
    async fn resume_continues_from_stored_cursor() {
        let _guard = ENV_LOCK.lock().unwrap();
        let pool = open_test_pool().await;
        let id = unique("cursor");
        let items: Vec<String> = (0..6).map(|i| format!("item-{i}")).collect();
        let items_ref: Vec<&str> = items.iter().map(String::as_str).collect();
        enqueue_jobs(&pool, &[job(&id, Some("acme/web"), &items_ref)])
            .await
            .unwrap();

        let cfg = QueueConfig {
            allowed_repos: vec!["acme".into()],
            chunk_size: 2,
            ..Default::default()
        };
        // Run 1: the second batch (items 2..4) fails.
        let flaky = StubEnricher::new().fail_after(1);
        assert!(run_queue(&pool, &flaky, &cfg).await.is_err());
        let row = sqlx::raw_sql(
            format!("SELECT cursor, attempts, status FROM enrichment_jobs WHERE id = '{id}'")
                .as_str(),
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(row.try_get::<i64, _>(0).unwrap(), 2, "cursor saved mid-job");
        assert_eq!(row.try_get::<i64, _>(1).unwrap(), 1, "attempts recorded");
        assert_eq!(
            row.try_get::<String, _>(2).unwrap(),
            "pending",
            "row remains queryable"
        );

        // Run 2: resume starts at item 2 — items 0..2 are never re-sent.
        let resumed = StubEnricher::new();
        run_queue(&pool, &resumed, &cfg).await.unwrap();
        let batches = resumed.batches.lock().unwrap();
        assert_eq!(
            batches[0].cursor, 2,
            "first resumed batch starts at the stored cursor"
        );
        assert_eq!(batches[0].items, vec!["item-2", "item-3"]);
        assert!(!batches
            .iter()
            .flat_map(|b| b.items.iter())
            .any(|item| item == "item-0" || item == "item-1"));
    }

    /// Exhausted attempts mark the job `failed` (not lost) and the row
    /// stays queryable.
    #[tokio::test]
    async fn exhausted_attempts_mark_job_failed_and_queryable() {
        let _guard = ENV_LOCK.lock().unwrap();
        let pool = open_test_pool().await;
        let id = unique("exhaust");
        enqueue_jobs(&pool, &[job(&id, Some("acme/web"), &["payload"])])
            .await
            .unwrap();

        let cfg = QueueConfig {
            allowed_repos: vec!["acme".into()],
            max_attempts: 3,
            ..Default::default()
        };
        let broken = StubEnricher::new().fail_after(0);
        for _ in 0..2 {
            assert!(run_queue(&pool, &broken, &cfg).await.is_err());
        }
        assert!(
            run_queue(&pool, &broken, &cfg).await.is_err(),
            "the exhausting run still stops with an error"
        );
        let row = sqlx::raw_sql(
            format!("SELECT attempts, status FROM enrichment_jobs WHERE id = '{id}'").as_str(),
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(row.try_get::<i64, _>(0).unwrap(), 3);
        assert_eq!(row.try_get::<String, _>(1).unwrap(), "failed");
    }

    /// A killed worker leaves its job `running`; the next run recovers it
    /// and finishes the job.
    #[tokio::test]
    async fn killed_worker_running_job_is_recovered() {
        let _guard = ENV_LOCK.lock().unwrap();
        let pool = open_test_pool().await;
        let id = unique("killed");
        enqueue_jobs(&pool, &[job(&id, Some("acme/web"), &["payload"])])
            .await
            .unwrap();
        // Simulate a kill mid-run: the row is left `running`.
        sqlx::query("UPDATE enrichment_jobs SET status = 'running' WHERE id = $1")
            .bind(&id)
            .execute(&pool)
            .await
            .unwrap();

        let recovered = recover_running(&pool).await.unwrap();
        assert_eq!(recovered, 1);

        let cfg = QueueConfig {
            allowed_repos: vec!["acme".into()],
            ..Default::default()
        };
        let enricher = StubEnricher::new();
        let outcome = run_queue(&pool, &enricher, &cfg).await.unwrap();
        assert_eq!(outcome.completed, 1);
        let row =
            sqlx::raw_sql(format!("SELECT status FROM enrichment_jobs WHERE id = '{id}'").as_str())
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(row.try_get::<String, _>(0).unwrap(), "done");
    }

    /// Enqueued payload (source evidence) survives the run and is
    /// queryable after completion.
    #[tokio::test]
    async fn source_evidence_is_preserved_and_queryable() {
        let _guard = ENV_LOCK.lock().unwrap();
        let pool = open_test_pool().await;
        let id = unique("evidence");
        enqueue_jobs(
            &pool,
            &[job(&id, Some("acme/web"), &["payload one", "payload two"])],
        )
        .await
        .unwrap();
        let cfg = QueueConfig {
            allowed_repos: vec!["acme".into()],
            ..Default::default()
        };
        run_queue(&pool, &StubEnricher::new(), &cfg).await.unwrap();

        let row = sqlx::raw_sql(
            format!(
                "SELECT payload, enrichments, session_id FROM enrichment_jobs WHERE id = '{id}'"
            )
            .as_str(),
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            row.try_get::<String, _>(0).unwrap(),
            r#"["payload one","payload two"]"#
        );
        let enrichments: serde_json::Value =
            serde_json::from_str(&row.try_get::<String, _>(1).unwrap()).unwrap();
        assert_eq!(enrichments.as_array().unwrap().len(), 2);
        assert_eq!(
            row.try_get::<String, _>(2).unwrap(),
            format!("session-{id}")
        );
    }
}
