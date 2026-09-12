//! Persistent resource ledger: the one storage boundary for the `resources`
//! table (spec `docs/specs/2026-08-16-resource-lifecycle-cleanup-design.md`
//! §12/§24.4).
//!
//! [`ResourceLedger`] is the query surface the `autospec resources` CLI
//! (spec §24.4) and the ledger-sync consumers read. It is deliberately
//! synchronous at the edge: the async sqlx pool is confined to this module
//! behind a single current-thread runtime (the same bounding condition as
//! `resources::db`, ADR 0001 D5), so callers never name a `Future`.
//!
//! Reads are exact and fail closed: a ledger row whose `state`,
//! `ownership`, or `resource_type` is not a canonical value is a hard
//! `Err` — it is never coerced, skipped, or printed as a blank column.
//!
//! The table lands as ledger migration 1 ([`MIGRATION_1_RESOURCES`], spec
//! §12: the `resources` table plus its four indexes). Writes MUST use
//! transactions (spec §12): [`ResourceLedger::upsert`] (and
//! [`ResourceLedger::insert`]) each run inside one transaction.
//! [`ResourceLedger::apply_migrations`] is idempotent and records the
//! ledger schema version in the ledger's own
//! `resources_schema_migrations` table — 0 when nothing has been applied,
//! 1 after migration 1. The `autospec resources` CLI is a strictly
//! read-only consumer and never calls either write path.

use sha2::{Digest, Sha256};
use sqlx::{AnyPool, Row};
use tokio::runtime::Runtime;

use crate::error::AutospecError;

use super::model::{ManagedResource, OwnershipClass, ResourceState, ResourceType};

/// Ledger migration 1 (spec §12) — the `resources` table and its four
/// indexes, `idx_resources_run_id`, `idx_resources_state`,
/// `idx_resources_type`, and `idx_resources_lease`. Applied idempotently by
/// [`ResourceLedger::apply_migrations`]: the `IF NOT EXISTS` guards mean a
/// re-run — or an `open` over an existing database — never rewrites an
/// existing table or index.
pub const MIGRATION_1_RESOURCES: &str = "CREATE TABLE IF NOT EXISTS resources (\n\
     id TEXT PRIMARY KEY,\n\
     run_id TEXT NOT NULL,\n\
     work_item_id TEXT,\n\
     repository_id TEXT,\n\
     worker_id TEXT,\n\
     resource_type TEXT NOT NULL,\n\
     external_id TEXT NOT NULL,\n\
     state TEXT NOT NULL,\n\
     ownership TEXT NOT NULL,\n\
     cleanup_policy_json TEXT NOT NULL,\n\
     created_at TEXT NOT NULL,\n\
     updated_at TEXT NOT NULL,\n\
     lease_expires_at TEXT,\n\
     last_heartbeat_at TEXT,\n\
     cleanup_attempts INTEGER NOT NULL DEFAULT 0,\n\
     last_cleanup_error TEXT,\n\
     metadata_json TEXT NOT NULL\n\
     );\n\
     CREATE INDEX IF NOT EXISTS idx_resources_run_id ON resources(run_id);\n\
     CREATE INDEX IF NOT EXISTS idx_resources_state ON resources(state);\n\
     CREATE INDEX IF NOT EXISTS idx_resources_type ON resources(resource_type);\n\
     CREATE INDEX IF NOT EXISTS idx_resources_lease ON resources(lease_expires_at);";

/// The ledger's own migration tracking table. Kept separate from the
/// shared `autospec_migrations` table (owned by `resources::db`'s
/// file-based subsystem migrations, whose `resources` range is 1xxxxxx)
/// because this ledger's DDL lives in this file, not in
/// `migrations/resources/`, and its schema version is the ledger's own
/// (0 → 1), not a subsystem file-migration version.
const SCHEMA_MIGRATIONS_TABLE_SQL: &str =
    "CREATE TABLE IF NOT EXISTS resources_schema_migrations (\n\
     version INTEGER PRIMARY KEY,\n\
     description TEXT NOT NULL,\n\
     checksum TEXT NOT NULL\n\
     )";

/// The highest ledger schema version recorded, or NULL when none has been
/// applied yet (i.e. the ledger is at schema version 0).
const SELECT_APPLIED_VERSION_SQL: &str = "SELECT MAX(version) FROM resources_schema_migrations";

const RECORD_MIGRATION_SQL: &str =
    "INSERT INTO resources_schema_migrations (version, description, checksum) \n\
     VALUES (?, ?, ?)";

/// Every SELECT below carries the SAME ordered column list (duplicated
/// verbatim, because a `const` cannot interpolate another `const` —
/// `concat!` takes literals only), so the row mapping in `row_to_resource`
/// and the SQL can never drift apart. Every statement in this module is a
/// `const`: no SQL string is ever built with `format!` on caller input.
const SELECT_ALL_SQL: &str = "SELECT id, run_id, work_item_id, repository_id, worker_id, \
     resource_type, external_id, state, ownership, cleanup_policy_json, \
     created_at, updated_at, lease_expires_at, last_heartbeat_at, \
     cleanup_attempts, last_cleanup_error, metadata_json FROM resources ORDER BY id";
const SELECT_BY_RUN_SQL: &str = "SELECT id, run_id, work_item_id, repository_id, worker_id, \
     resource_type, external_id, state, ownership, cleanup_policy_json, \
     created_at, updated_at, lease_expires_at, last_heartbeat_at, \
     cleanup_attempts, last_cleanup_error, metadata_json \
     FROM resources WHERE run_id = ? ORDER BY id";
const SELECT_BY_TYPE_SQL: &str = "SELECT id, run_id, work_item_id, repository_id, worker_id, \
     resource_type, external_id, state, ownership, cleanup_policy_json, \
     created_at, updated_at, lease_expires_at, last_heartbeat_at, \
     cleanup_attempts, last_cleanup_error, metadata_json \
     FROM resources WHERE resource_type = ? ORDER BY id";
const SELECT_BY_STATE_SQL: &str = "SELECT id, run_id, work_item_id, repository_id, worker_id, \
     resource_type, external_id, state, ownership, cleanup_policy_json, \
     created_at, updated_at, lease_expires_at, last_heartbeat_at, \
     cleanup_attempts, last_cleanup_error, metadata_json \
     FROM resources WHERE state = ? ORDER BY id";
const SELECT_BY_ID_SQL: &str = "SELECT id, run_id, work_item_id, repository_id, worker_id, \
     resource_type, external_id, state, ownership, cleanup_policy_json, \
     created_at, updated_at, lease_expires_at, last_heartbeat_at, \
     cleanup_attempts, last_cleanup_error, metadata_json FROM resources WHERE id = ?";
const COUNT_BY_TYPE_AND_STATE_SQL: &str = "SELECT resource_type, state, COUNT(*) \n\
     FROM resources GROUP BY resource_type, state ORDER BY resource_type, state";

/// Portable upsert (SQLite 3.24+ and PostgreSQL both speak the standard
/// `ON CONFLICT ... DO UPDATE` form) — one statement, one transaction.
const UPSERT_SQL: &str = "INSERT INTO resources (\n\
     id, run_id, work_item_id, repository_id, worker_id,\n\
     resource_type, external_id, state, ownership, cleanup_policy_json,\n\
     created_at, updated_at, lease_expires_at, last_heartbeat_at,\n\
     cleanup_attempts, last_cleanup_error, metadata_json\n\
     ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)\n\
     ON CONFLICT (id) DO UPDATE SET\n\
       run_id = excluded.run_id,\n\
       work_item_id = excluded.work_item_id,\n\
       repository_id = excluded.repository_id,\n\
       worker_id = excluded.worker_id,\n\
       resource_type = excluded.resource_type,\n\
       external_id = excluded.external_id,\n\
       state = excluded.state,\n\
       ownership = excluded.ownership,\n\
       cleanup_policy_json = excluded.cleanup_policy_json,\n\
       created_at = excluded.created_at,\n\
       updated_at = excluded.updated_at,\n\
       lease_expires_at = excluded.lease_expires_at,\n\
       last_heartbeat_at = excluded.last_heartbeat_at,\n\
       cleanup_attempts = excluded.cleanup_attempts,\n\
       last_cleanup_error = excluded.last_cleanup_error,\n\
       metadata_json = excluded.metadata_json";

fn checksum(bytes: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(bytes);
    digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// The persistent resource ledger behind the shared autospec database.
///
/// Not `Sync`: the runtime is current-thread, so a ledger belongs to the
/// thread that opened it (the CLI is single-threaded; tests open one per
/// test thread).
#[derive(Debug)]
pub struct ResourceLedger {
    pool: AnyPool,
    runtime: Runtime,
}

impl ResourceLedger {
    /// Open (or bootstrap) the ledger at `url` — the same `sqlite://` /
    /// `postgres://` grammar [`super::db::open_shared_db`] accepts. Parent
    /// directories and a missing SQLite file are created, and the
    /// `resources` table plus its indexes are ensured idempotently, so
    /// `list` on a fresh database is a queryable empty ledger, never a
    /// "no such table" error.
    pub fn open(url: &str) -> Result<Self, AutospecError> {
        // `enable_time` is mandatory: the sqlx pool schedules its idle
        // reaper on a tokio timer, and a timer-less runtime panics on the
        // first pool operation.
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .map_err(|error| {
                AutospecError::state("resource ledger", format!("build runtime: {error}"))
            })?;
        let pool = runtime.block_on(async {
            let pool = super::db::open_shared_db(url).await?;
            ensure_migrations(&pool).await?;
            Ok::<AnyPool, AutospecError>(pool)
        })?;
        Ok(Self { pool, runtime })
    }

    /// Apply ledger migration 1 ([`MIGRATION_1_RESOURCES`]) to the open
    /// database. Idempotent: an already-recorded version 1 is a no-op, so
    /// calling it twice — or re-opening the same database — advances the
    /// recorded schema version from `0` to `1` exactly once. Runs on both
    /// backends the shared database supports (SQLite and PostgreSQL).
    pub fn apply_migrations(&self) -> Result<(), AutospecError> {
        let pool = &self.pool;
        self.runtime
            .block_on(async { ensure_migrations(pool).await })
    }

    /// The ledger schema version recorded in `resources_schema_migrations`
    /// — `0` when no migration has been recorded, `1` after migration 1.
    pub fn schema_version(&self) -> Result<i64, AutospecError> {
        let pool = &self.pool;
        self.runtime.block_on(async {
            let row = sqlx::query(SELECT_APPLIED_VERSION_SQL)
                .fetch_one(pool)
                .await
                .map_err(|error| AutospecError::state("resource ledger", error.to_string()))?;
            Ok(row
                .try_get::<Option<i64>, _>(0)
                .unwrap_or(None)
                .unwrap_or(0))
        })
    }

    /// Every ledger row, ordered by `id`.
    pub fn list_all(&self) -> Result<Vec<ManagedResource>, AutospecError> {
        self.fetch(SELECT_ALL_SQL, &[])
    }

    /// The rows owned by one run, ordered by `id`.
    pub fn list_by_run(&self, run_id: &str) -> Result<Vec<ManagedResource>, AutospecError> {
        self.fetch(SELECT_BY_RUN_SQL, &[run_id.to_string()])
    }

    /// The rows in one lifecycle state, ordered by `id`.
    pub fn list_by_state(
        &self,
        state: ResourceState,
    ) -> Result<Vec<ManagedResource>, AutospecError> {
        self.fetch(SELECT_BY_STATE_SQL, &[state.as_str().to_string()])
    }

    /// The rows of one resource type, ordered by `id`.
    pub fn list_by_type(
        &self,
        resource_type: ResourceType,
    ) -> Result<Vec<ManagedResource>, AutospecError> {
        self.fetch(SELECT_BY_TYPE_SQL, &[resource_type.as_str().to_string()])
    }

    /// The one record with this `id`, or `None` when it is absent.
    pub fn get(&self, id: &str) -> Result<Option<ManagedResource>, AutospecError> {
        let rows = self.fetch(SELECT_BY_ID_SQL, &[id.to_string()])?;
        Ok(rows.into_iter().next())
    }

    /// Per-bucket row counts: one `(resource_type, state, count)` triple per
    /// distinct (type, state) pair present in the ledger, in (type, state)
    /// order. A bucket with no rows simply does not appear; a corrupted row
    /// (unknown `resource_type` or `state` text) is a hard `Err`.
    pub fn count_by_type_and_state(
        &self,
    ) -> Result<Vec<(ResourceType, ResourceState, i64)>, AutospecError> {
        let pool = &self.pool;
        let rows = self.runtime.block_on(async {
            sqlx::query(COUNT_BY_TYPE_AND_STATE_SQL)
                .fetch_all(pool)
                .await
                .map_err(|error| AutospecError::state("resource ledger", error.to_string()))
        })?;
        rows.iter()
            .map(|row| {
                let resource_type = row
                    .try_get::<String, _>(0)
                    .map_err(row_error)?
                    .parse::<ResourceType>()?;
                let state = row
                    .try_get::<String, _>(1)
                    .map_err(row_error)?
                    .parse::<ResourceState>()?;
                let count = row.try_get::<i64, _>(2).map_err(row_error)?;
                Ok((resource_type, state, count))
            })
            .collect()
    }

    /// Insert-or-update one row, keyed on `id`, inside one transaction
    /// (spec §12: writes MUST use transactions). The same `id` upserted
    /// twice leaves exactly one row, holding the newest values. Portable
    /// across both backends via the standard `ON CONFLICT (id) DO UPDATE`
    /// form.
    pub fn upsert(&mut self, resource: &ManagedResource) -> Result<(), AutospecError> {
        let pool = &self.pool;
        self.runtime.block_on(async {
            let mut tx = pool
                .begin()
                .await
                .map_err(|error| AutospecError::state("resource ledger", error.to_string()))?;
            sqlx::query(UPSERT_SQL)
                .bind(&resource.id)
                .bind(&resource.run_id)
                .bind(&resource.work_item_id)
                .bind(&resource.repository_id)
                .bind(&resource.worker_id)
                .bind(resource.resource_type.as_str())
                .bind(&resource.external_id)
                .bind(resource.state.as_str())
                .bind(resource.ownership.as_str())
                .bind(resource.cleanup_policy.to_string())
                .bind(&resource.created_at)
                .bind(&resource.updated_at)
                .bind(&resource.lease_expires_at)
                .bind(&resource.last_heartbeat_at)
                .bind(resource.cleanup_attempts as i64)
                .bind(&resource.last_cleanup_error)
                .bind(resource.metadata.to_string())
                .execute(&mut *tx)
                .await
                .map_err(|error| {
                    AutospecError::state(
                        "resource ledger",
                        format!("upsert {}: {error}", resource.id),
                    )
                })?;
            tx.commit()
                .await
                .map_err(|error| AutospecError::state("resource ledger", error.to_string()))?;
            Ok(())
        })
    }

    /// Insert one row, in its own transaction (spec §12: writes MUST use
    /// transactions). A primary key collision is a hard `Err`, never an
    /// overwrite — callers that want last-write-wins use [`Self::upsert`].
    pub fn insert(&self, resource: &ManagedResource) -> Result<(), AutospecError> {
        let pool = &self.pool;
        self.runtime.block_on(async {
            sqlx::query(
                "INSERT INTO resources (\n\
                 id, run_id, work_item_id, repository_id, worker_id,\n\
                 resource_type, external_id, state, ownership, cleanup_policy_json,\n\
                 created_at, updated_at, lease_expires_at, last_heartbeat_at,\n\
                 cleanup_attempts, last_cleanup_error, metadata_json\n\
                 ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&resource.id)
            .bind(&resource.run_id)
            .bind(&resource.work_item_id)
            .bind(&resource.repository_id)
            .bind(&resource.worker_id)
            .bind(resource.resource_type.as_str())
            .bind(&resource.external_id)
            .bind(resource.state.as_str())
            .bind(resource.ownership.as_str())
            .bind(resource.cleanup_policy.to_string())
            .bind(&resource.created_at)
            .bind(&resource.updated_at)
            .bind(&resource.lease_expires_at)
            .bind(&resource.last_heartbeat_at)
            .bind(resource.cleanup_attempts as i64)
            .bind(&resource.last_cleanup_error)
            .bind(resource.metadata.to_string())
            .execute(pool)
            .await
            .map_err(|error| {
                AutospecError::state(
                    "resource ledger",
                    format!("insert {}: {error}", resource.id),
                )
            })?;
            Ok(())
        })
    }

    fn fetch(&self, sql: &str, binds: &[String]) -> Result<Vec<ManagedResource>, AutospecError> {
        let pool = &self.pool;
        let rows = self.runtime.block_on(async {
            let mut query = sqlx::query(sql);
            for bind in binds {
                query = query.bind(bind);
            }
            query
                .fetch_all(pool)
                .await
                .map_err(|error| AutospecError::state("resource ledger", error.to_string()))
        })?;
        rows.iter().map(row_to_resource).collect()
    }
}

fn row_to_resource(row: &sqlx::any::AnyRow) -> Result<ManagedResource, AutospecError> {
    let cleanup_attempts = row
        .try_get::<i64, _>(14)
        .map_err(|error| AutospecError::state("resource ledger row", error.to_string()))?;
    let resource_type = row
        .try_get::<String, _>(5)
        .map_err(row_error)?
        .parse::<ResourceType>()?;
    let state = row
        .try_get::<String, _>(7)
        .map_err(row_error)?
        .parse::<ResourceState>()?;
    let ownership = row
        .try_get::<String, _>(8)
        .map_err(row_error)?
        .parse::<OwnershipClass>()?;
    let policy_json = row.try_get::<String, _>(9).map_err(row_error)?;
    let metadata_json = row.try_get::<String, _>(16).map_err(row_error)?;

    Ok(ManagedResource {
        id: row.try_get(0).map_err(row_error)?,
        run_id: row.try_get(1).map_err(row_error)?,
        work_item_id: row.try_get(2).map_err(row_error)?,
        repository_id: row.try_get(3).map_err(row_error)?,
        worker_id: row.try_get(4).map_err(row_error)?,
        resource_type,
        external_id: row.try_get(6).map_err(row_error)?,
        state,
        ownership,
        cleanup_policy: serde_json::from_str(&policy_json).map_err(|error| {
            AutospecError::parse(
                "cleanup_policy_json",
                format!("invalid cleanup policy JSON: {error}"),
            )
        })?,
        created_at: row.try_get(10).map_err(row_error)?,
        updated_at: row.try_get(11).map_err(row_error)?,
        lease_expires_at: row.try_get(12).map_err(row_error)?,
        last_heartbeat_at: row.try_get(13).map_err(row_error)?,
        cleanup_attempts: u32::try_from(cleanup_attempts).map_err(|_| {
            AutospecError::parse(
                "cleanup_attempts",
                format!("cleanup_attempts {cleanup_attempts} does not fit u32"),
            )
        })?,
        last_cleanup_error: row.try_get(15).map_err(row_error)?,
        metadata: serde_json::from_str(&metadata_json).map_err(|error| {
            AutospecError::parse("metadata_json", format!("invalid metadata JSON: {error}"))
        })?,
    })
}

fn row_error(error: sqlx::Error) -> AutospecError {
    AutospecError::state("resource ledger row", error.to_string())
}

/// Ensure ledger migration 1 has been applied to `pool` (idempotent):
/// create the tracking table, check the recorded version, and — only when
/// nothing is recorded yet — run [`MIGRATION_1_RESOURCES`] and record
/// version 1 in one transaction.
async fn ensure_migrations(pool: &AnyPool) -> Result<(), AutospecError> {
    sqlx::query(SCHEMA_MIGRATIONS_TABLE_SQL)
        .execute(pool)
        .await
        .map_err(|error| AutospecError::state("resource ledger", error.to_string()))?;

    let row = sqlx::query(SELECT_APPLIED_VERSION_SQL)
        .fetch_one(pool)
        .await
        .map_err(|error| AutospecError::state("resource ledger", error.to_string()))?;
    let applied = row.try_get::<Option<i64>, _>(0).unwrap_or(None);
    if applied.is_some_and(|version| version >= 1) {
        return Ok(());
    }

    let mut tx = pool
        .begin()
        .await
        .map_err(|error| AutospecError::state("resource ledger", error.to_string()))?;
    for statement in MIGRATION_1_RESOURCES.split(';') {
        let statement = statement.trim();
        if statement.is_empty() {
            continue;
        }
        sqlx::query(statement)
            .execute(&mut *tx)
            .await
            .map_err(|error| {
                AutospecError::state(
                    "resources migration 1",
                    format!("apply migration 1: {error}"),
                )
            })?;
    }
    sqlx::query(RECORD_MIGRATION_SQL)
        .bind(1i64)
        .bind("resources table + four spec §12 indexes")
        .bind(checksum(MIGRATION_1_RESOURCES.as_bytes()))
        .execute(&mut *tx)
        .await
        .map_err(|error| {
            AutospecError::state(
                "resources migration 1",
                format!("record version 1: {error}"),
            )
        })?;
    tx.commit()
        .await
        .map_err(|error| AutospecError::state("resources migration 1", error.to_string()))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Mutex;

    static DIR_COUNTER: Mutex<u32> = Mutex::new(0);

    fn fresh_tmp_dir() -> PathBuf {
        let mut counter = DIR_COUNTER.lock().unwrap();
        *counter += 1;
        let dir = std::env::temp_dir().join(format!(
            "autospec-ledger-test-{}-{}",
            std::process::id(),
            *counter
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn resource(
        id: &str,
        run_id: &str,
        resource_type: ResourceType,
        state: ResourceState,
        ownership: OwnershipClass,
    ) -> ManagedResource {
        ManagedResource {
            id: id.to_string(),
            run_id: run_id.to_string(),
            work_item_id: Some(format!("3185-{id}")),
            repository_id: Some("berlinguyinca/autospec".to_string()),
            worker_id: None,
            resource_type,
            external_id: format!("/tmp/wt-{id}"),
            state,
            ownership,
            cleanup_policy: serde_json::json!({"ttl_seconds": 10800}),
            created_at: "2026-09-02T00:00:00Z".to_string(),
            updated_at: "2026-09-02T00:05:00Z".to_string(),
            lease_expires_at: Some("2026-09-02T03:00:00Z".to_string()),
            last_heartbeat_at: None,
            cleanup_attempts: 2,
            last_cleanup_error: Some("busy".to_string()),
            metadata: serde_json::json!({"origin": "test"}),
        }
    }

    #[test]
    fn open_bootstraps_a_fresh_database_into_a_queryable_empty_ledger() {
        let dir = fresh_tmp_dir();
        let url = format!("sqlite://{}/ledger.db", dir.display());
        let ledger = ResourceLedger::open(&url).expect("open a fresh ledger");
        assert!(
            ledger
                .list_all()
                .expect("list on a fresh ledger")
                .is_empty(),
            "a fresh ledger is an empty ledger, not an error"
        );
        drop(ledger);
        // Re-open on the same file: the idempotent schema must not collide.
        let again = ResourceLedger::open(&url).expect("re-open the same ledger");
        assert!(again.list_all().expect("re-list").is_empty());
    }

    #[test]
    fn open_rejects_a_schemeless_url_instead_of_defaulting() {
        let error = ResourceLedger::open("not-a-url").expect_err("no scheme must fail");
        assert!(matches!(error, AutospecError::Validation { .. }));
    }

    #[test]
    fn insert_and_get_round_trip_every_field() {
        let dir = fresh_tmp_dir();
        let url = format!("sqlite://{}/ledger.db", dir.display());
        let ledger = ResourceLedger::open(&url).unwrap();
        let original = resource(
            "res-1",
            "run-1",
            ResourceType::GitWorktree,
            ResourceState::Active,
            OwnershipClass::RunExclusive,
        );
        ledger.insert(&original).expect("insert");
        let back = ledger.get("res-1").expect("get").expect("row present");
        assert_eq!(back, original);
        assert_eq!(
            ledger.get("missing").expect("get missing").as_ref(),
            None,
            "an absent id is None, never a blank record"
        );
    }

    #[test]
    fn insert_with_a_duplicate_primary_key_fails_loudly() {
        let dir = fresh_tmp_dir();
        let url = format!("sqlite://{}/ledger.db", dir.display());
        let ledger = ResourceLedger::open(&url).unwrap();
        let first = resource(
            "res-1",
            "run-1",
            ResourceType::GitBranch,
            ResourceState::Released,
            OwnershipClass::RepoShared,
        );
        ledger.insert(&first).unwrap();
        ledger
            .insert(&first)
            .expect_err("a duplicate id must fail, never overwrite");
        assert_eq!(ledger.list_all().unwrap().len(), 1);
    }

    #[test]
    fn list_all_orders_by_id() {
        let dir = fresh_tmp_dir();
        let url = format!("sqlite://{}/ledger.db", dir.display());
        let ledger = ResourceLedger::open(&url).unwrap();
        for id in ["c", "a", "b"] {
            ledger
                .insert(&resource(
                    id,
                    "run-1",
                    ResourceType::TempFile,
                    ResourceState::Missing,
                    OwnershipClass::External,
                ))
                .unwrap();
        }
        let ids: Vec<String> = ledger
            .list_all()
            .unwrap()
            .into_iter()
            .map(|row| row.id)
            .collect();
        assert_eq!(ids, vec!["a", "b", "c"]);
    }

    #[test]
    fn list_by_run_filters_to_one_run() {
        let dir = fresh_tmp_dir();
        let url = format!("sqlite://{}/ledger.db", dir.display());
        let ledger = ResourceLedger::open(&url).unwrap();
        ledger
            .insert(&resource(
                "res-a",
                "run-1",
                ResourceType::DockerContainer,
                ResourceState::Active,
                OwnershipClass::RunExclusive,
            ))
            .unwrap();
        ledger
            .insert(&resource(
                "res-b",
                "run-2",
                ResourceType::DockerContainer,
                ResourceState::Active,
                OwnershipClass::RunExclusive,
            ))
            .unwrap();
        let rows = ledger.list_by_run("run-1").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "res-a");
        assert!(ledger.list_by_run("nobody").unwrap().is_empty());
    }

    #[test]
    fn list_by_type_filters_to_one_type() {
        let dir = fresh_tmp_dir();
        let url = format!("sqlite://{}/ledger.db", dir.display());
        let ledger = ResourceLedger::open(&url).unwrap();
        ledger
            .insert(&resource(
                "res-a",
                "run-1",
                ResourceType::GitWorktree,
                ResourceState::Active,
                OwnershipClass::RunExclusive,
            ))
            .unwrap();
        ledger
            .insert(&resource(
                "res-b",
                "run-1",
                ResourceType::DockerImage,
                ResourceState::Active,
                OwnershipClass::RepoShared,
            ))
            .unwrap();
        let rows = ledger.list_by_type(ResourceType::GitWorktree).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "res-a");
        assert!(ledger
            .list_by_type(ResourceType::BuildCache)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn external_ownership_survives_the_round_trip_verbatim() {
        // The counter-team's misclassification challenge: an External row
        // must come back as External, never coerced to a blank or a
        // more-permissive class.
        let dir = fresh_tmp_dir();
        let url = format!("sqlite://{}/ledger.db", dir.display());
        let ledger = ResourceLedger::open(&url).unwrap();
        let external = resource(
            "res-ext",
            "run-1",
            ResourceType::DockerNetwork,
            ResourceState::Active,
            OwnershipClass::External,
        );
        ledger.insert(&external).unwrap();
        let back = ledger.get("res-ext").unwrap().unwrap();
        assert_eq!(back.ownership, OwnershipClass::External);
        assert_eq!(back.ownership.as_str(), "external");
        assert!(!back.ownership.is_reclaimable());
    }

    // ── Migration 1, apply_migrations, schema version ──────────────

    #[test]
    fn migration_one_creates_the_resources_table_and_the_four_spec_named_indexes() {
        let dir = fresh_tmp_dir();
        let url = format!("sqlite://{}/ledger.db", dir.display());
        let ledger = ResourceLedger::open(&url).expect("open a fresh ledger");

        assert_eq!(
            ledger.schema_version().expect("schema version"),
            1,
            "migration 1 must record schema version 1"
        );

        let pool = &ledger.pool;
        let found = ledger.runtime.block_on(async {
            let rows = sqlx::query(
                "SELECT type, name FROM sqlite_master \n\
                     WHERE type IN ('table', 'index') AND name IN \n\
                     ('resources', 'idx_resources_run_id', 'idx_resources_state', \n\
                      'idx_resources_type', 'idx_resources_lease')",
            )
            .fetch_all(pool)
            .await
            .expect("query sqlite_master");
            rows.iter()
                .map(|row| {
                    format!(
                        "{}:{}",
                        row.try_get::<String, _>(0).unwrap(),
                        row.try_get::<String, _>(1).unwrap()
                    )
                })
                .collect::<Vec<_>>()
        });
        for expected in [
            "table:resources",
            "index:idx_resources_run_id",
            "index:idx_resources_state",
            "index:idx_resources_type",
            "index:idx_resources_lease",
        ] {
            assert!(
                found.iter().any(|name| name == expected),
                "{expected} is missing after migration 1; found {found:?}"
            );
        }
    }

    #[test]
    fn apply_migrations_advances_the_recorded_schema_version_from_zero_to_one() {
        let dir = fresh_tmp_dir();
        let url = format!("sqlite://{}/ledger.db", dir.display());

        // Schema version 0: a fresh database records no ledger migration.
        let pre = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        let raw_pool =
            pre.block_on(async { super::super::db::open_shared_db(&url).await.unwrap() });
        let pre_state = pre.block_on(async {
            sqlx::query(
                "SELECT COUNT(*) FROM sqlite_master \n\
                 WHERE name IN ('resources', 'resources_schema_migrations')",
            )
            .fetch_one(&raw_pool)
            .await
            .unwrap()
            .try_get::<i64, _>(0)
            .unwrap()
        });
        assert_eq!(pre_state, 0, "a fresh database is at schema version 0");
        drop(raw_pool);
        drop(pre);

        let ledger = ResourceLedger::open(&url).expect("open applies migration 1");
        assert_eq!(
            ledger.schema_version().expect("schema version"),
            1,
            "apply_migrations must advance the recorded version from 0 to 1"
        );

        // Exactly one recorded migration, and a double-apply must leave it
        // at 1 with the same single record (idempotent).
        let pool = &ledger.pool;
        let recorded = ledger.runtime.block_on(async {
            sqlx::query("SELECT COUNT(*) FROM resources_schema_migrations")
                .fetch_one(pool)
                .await
                .unwrap()
                .try_get::<i64, _>(0)
                .unwrap()
        });
        assert_eq!(recorded, 1, "exactly one recorded migration");
        ledger
            .apply_migrations()
            .expect("double-apply must be a no-op");
        assert_eq!(ledger.schema_version().unwrap(), 1);
    }

    #[test]
    fn apply_migrations_records_version_one_and_the_four_indexes_on_postgres_16() {
        let Ok(url) = std::env::var("AUTOSPEC_TEST_DB_URL") else {
            eprintln!(
                // autospec:allow-output — test SKIP notice
                "SKIP apply_migrations_on_postgres_16: AUTOSPEC_TEST_DB_URL is not set"
            );
            return;
        };
        let ledger =
            ResourceLedger::open(&url).expect("open against the PostgreSQL 16 test database");
        assert_eq!(
            ledger.schema_version().expect("schema version"),
            1,
            "migration 1 must record schema version 1 on PostgreSQL 16"
        );
        let indexes = ledger.runtime.block_on(async {
            sqlx::query(
                "SELECT COUNT(*) FROM pg_indexes \n\
                 WHERE tablename = 'resources' AND indexname IN \n\
                 ('idx_resources_run_id', 'idx_resources_state', \n\
                  'idx_resources_type', 'idx_resources_lease')",
            )
            .fetch_one(&ledger.pool)
            .await
            .unwrap()
            .try_get::<i64, _>(0)
            .unwrap()
        });
        assert_eq!(
            indexes, 4,
            "all four spec §12 indexes must exist on PostgreSQL 16"
        );
    }

    // ── upsert ────────────────────────────────────────────────────────

    #[test]
    fn upserting_the_same_id_twice_leaves_exactly_one_row_with_the_newer_values() {
        let dir = fresh_tmp_dir();
        let url = format!("sqlite://{}/ledger.db", dir.display());
        let mut ledger = ResourceLedger::open(&url).unwrap();
        let first = resource(
            "res-1",
            "run-1",
            ResourceType::GitWorktree,
            ResourceState::Active,
            OwnershipClass::RunExclusive,
        );
        ledger.upsert(&first).expect("first upsert inserts");
        let mut second = first.clone();
        second.state = ResourceState::Retained;
        second.cleanup_attempts = 7;
        ledger
            .upsert(&second)
            .expect("second upsert must not collide");

        let rows = ledger.list_all().unwrap();
        assert_eq!(
            rows.len(),
            1,
            "upsert of the same id twice must leave SELECT count(*) at 1"
        );
        let back = ledger.get("res-1").unwrap().unwrap();
        assert_eq!(back.state, ResourceState::Retained);
        assert_eq!(back.cleanup_attempts, 7);
    }

    #[test]
    fn upsert_round_trips_every_field_on_both_insert_and_update() {
        let dir = fresh_tmp_dir();
        let url = format!("sqlite://{}/ledger.db", dir.display());
        let mut ledger = ResourceLedger::open(&url).unwrap();
        let original = resource(
            "res-1",
            "run-1",
            ResourceType::DockerVolume,
            ResourceState::Creating,
            OwnershipClass::GlobalShared,
        );
        ledger.upsert(&original).unwrap();
        let mut updated = original.clone();
        updated.state = ResourceState::Active;
        updated.last_heartbeat_at = Some("2026-09-02T00:10:00Z".to_string());
        ledger.upsert(&updated).unwrap();
        let back = ledger.get("res-1").unwrap().unwrap();
        assert_eq!(back, updated);
    }

    // ── list_by_state, count_by_type_and_state ────────────────────────

    #[test]
    fn list_by_state_filters_to_one_state() {
        let dir = fresh_tmp_dir();
        let url = format!("sqlite://{}/ledger.db", dir.display());
        let mut ledger = ResourceLedger::open(&url).unwrap();
        ledger
            .upsert(&resource(
                "res-a",
                "run-1",
                ResourceType::TempFile,
                ResourceState::Active,
                OwnershipClass::RunExclusive,
            ))
            .unwrap();
        ledger
            .upsert(&resource(
                "res-b",
                "run-1",
                ResourceType::TempFile,
                ResourceState::Released,
                OwnershipClass::RunExclusive,
            ))
            .unwrap();
        ledger
            .upsert(&resource(
                "res-c",
                "run-2",
                ResourceType::DockerImage,
                ResourceState::Active,
                OwnershipClass::RepoShared,
            ))
            .unwrap();

        let active = ledger.list_by_state(ResourceState::Active).unwrap();
        let ids: Vec<String> = active.iter().map(|row| row.id.clone()).collect();
        assert_eq!(ids, vec!["res-a", "res-c"]);
        let released = ledger.list_by_state(ResourceState::Released).unwrap();
        assert_eq!(released.len(), 1);
        assert_eq!(released[0].id, "res-b");
        assert!(ledger
            .list_by_state(ResourceState::Quarantined)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn count_by_type_and_state_reports_one_bucket_per_type_state_pair() {
        let dir = fresh_tmp_dir();
        let url = format!("sqlite://{}/ledger.db", dir.display());
        let mut ledger = ResourceLedger::open(&url).unwrap();
        ledger
            .upsert(&resource(
                "res-a",
                "run-1",
                ResourceType::GitWorktree,
                ResourceState::Active,
                OwnershipClass::RunExclusive,
            ))
            .unwrap();
        ledger
            .upsert(&resource(
                "res-b",
                "run-1",
                ResourceType::GitWorktree,
                ResourceState::Active,
                OwnershipClass::RunExclusive,
            ))
            .unwrap();
        ledger
            .upsert(&resource(
                "res-c",
                "run-1",
                ResourceType::GitWorktree,
                ResourceState::Released,
                OwnershipClass::RunExclusive,
            ))
            .unwrap();
        ledger
            .upsert(&resource(
                "res-d",
                "run-1",
                ResourceType::DockerImage,
                ResourceState::Active,
                OwnershipClass::RepoShared,
            ))
            .unwrap();

        let counts = ledger.count_by_type_and_state().unwrap();
        assert_eq!(
            counts,
            vec![
                (ResourceType::DockerImage, ResourceState::Active, 1),
                (ResourceType::GitWorktree, ResourceState::Active, 2),
                (ResourceType::GitWorktree, ResourceState::Released, 1),
            ]
        );
    }

    #[test]
    fn list_by_state_over_six_thousand_five_hundred_rows_completes_in_under_two_seconds() {
        let dir = fresh_tmp_dir();
        let url = format!("sqlite://{}/ledger.db", dir.display());
        let ledger = ResourceLedger::open(&url).unwrap();

        // Seed 6,500 rows in one transaction, half Active / half Creating.
        let pool = &ledger.pool;
        ledger.runtime.block_on(async {
            let mut tx = pool.begin().await.unwrap();
            for i in 0..6500 {
                let state = ResourceState::ALL[i % 2]; // odd i -> Active
                sqlx::query(
                    "INSERT INTO resources (\n\
                     id, run_id, resource_type, external_id, state, ownership,\n\
                     cleanup_policy_json, created_at, updated_at, metadata_json\n\
                     ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                )
                .bind(format!("res-seed-{i}"))
                .bind(format!("run-seed-{}", i % 50))
                .bind(ResourceType::ALL[i % 12].as_str())
                .bind(format!("/tmp/seed-{i}"))
                .bind(state.as_str())
                .bind(OwnershipClass::RunExclusive.as_str())
                .bind("{}")
                .bind("2026-09-02T00:00:00Z")
                .bind("2026-09-02T00:00:00Z")
                .bind("{}")
                .execute(&mut *tx)
                .await
                .unwrap();
            }
            tx.commit().await.unwrap();
        });

        let start = std::time::Instant::now();
        let rows = ledger
            .list_by_state(ResourceState::Active)
            .expect("list_by_state over the seeded ledger");
        let elapsed = start.elapsed();

        assert_eq!(rows.len(), 3250, "exactly half the seed is Active");
        assert!(
            elapsed < std::time::Duration::from_secs(2),
            "list_by_state over 6,500 rows must stay under 2 seconds; took {elapsed:?}"
        );
    }

    // ── negative cases ────────────────────────────────────────────────

    #[test]
    fn unknown_ownership_text_yields_an_error_instead_of_defaulting_to_run_exclusive() {
        let dir = fresh_tmp_dir();
        let url = format!("sqlite://{}/ledger.db", dir.display());
        let ledger = ResourceLedger::open(&url).unwrap();
        let id = "res-bogus-ownership";

        // A corrupted/forged ownership value written straight to the table
        // (the ledger's own write paths cannot produce one — they go
        // through the typed model).
        let pool = &ledger.pool;
        ledger.runtime.block_on(async {
            sqlx::query(
                "INSERT INTO resources (\n\
                 id, run_id, resource_type, external_id, state, ownership,\n\
                 cleanup_policy_json, created_at, updated_at, metadata_json\n\
                 ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(id)
            .bind("run-1")
            .bind("git_branch")
            .bind("/tmp/bogus")
            .bind("active")
            .bind("made-up-ownership")
            .bind("{}")
            .bind("2026-09-02T00:00:00Z")
            .bind("2026-09-02T00:00:00Z")
            .bind("{}")
            .execute(pool)
            .await
            .unwrap();
        });

        let error = ledger
            .get(id)
            .expect_err("unknown ownership text must be a hard Err");
        assert!(
            matches!(
                error,
                AutospecError::Parse {
                    ref context,
                    ..
                } if context == "ownership_class"
            ),
            "expected an ownership_class parse error, got: {error}"
        );
        // Reads fail closed: the corrupted row poisons the run's list
        // instead of silently dropping or coercing it.
        let listed = ledger
            .list_by_run("run-1")
            .expect_err("reads must fail closed on a corrupted row");
        assert!(
            matches!(listed, AutospecError::Parse { .. }),
            "list_by_run must not coerce unknown ownership text"
        );
    }

    #[test]
    fn external_rows_never_appear_in_a_reclaimable_filter() {
        let dir = fresh_tmp_dir();
        let url = format!("sqlite://{}/ledger.db", dir.display());
        let mut ledger = ResourceLedger::open(&url).unwrap();
        ledger
            .upsert(&resource(
                "res-run-exclusive",
                "run-1",
                ResourceType::GitWorktree,
                ResourceState::Active,
                OwnershipClass::RunExclusive,
            ))
            .unwrap();
        ledger
            .upsert(&resource(
                "res-external",
                "run-1",
                ResourceType::DockerNetwork,
                ResourceState::Active,
                OwnershipClass::External,
            ))
            .unwrap();

        let rows = ledger.list_by_state(ResourceState::Active).unwrap();
        let reclaimable: Vec<String> = rows
            .iter()
            .filter(|row| row.ownership.is_reclaimable())
            .map(|row| row.id.clone())
            .collect();
        assert!(
            reclaimable.contains(&"res-run-exclusive".to_string()),
            "a RunExclusive row is reclaimable-eligible"
        );
        assert!(
            !reclaimable.contains(&"res-external".to_string()),
            "an External row must never appear in a reclaimable filter"
        );
    }
}
