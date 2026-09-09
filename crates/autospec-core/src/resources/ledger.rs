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
//! Writes go through [`ResourceLedger::insert`] (spec §12: writes MUST use
//! transactions); the `autospec resources` CLI is a strictly read-only
//! consumer and never calls it.

use sqlx::{AnyPool, Row};
use tokio::runtime::Runtime;

use crate::error::AutospecError;

use super::model::{ManagedResource, OwnershipClass, ResourceState, ResourceType};

/// Spec §12 — the `resources` table, created idempotently on open so a
/// fresh database is a queryable empty ledger. The `IF NOT EXISTS` guards
/// mean an open never rewrites an existing table.
const RESOURCES_SCHEMA_SQL: [&str; 5] = [
    "CREATE TABLE IF NOT EXISTS resources (\n\
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
     )",
    "CREATE INDEX IF NOT EXISTS idx_resources_run_id ON resources(run_id)",
    "CREATE INDEX IF NOT EXISTS idx_resources_state ON resources(state)",
    "CREATE INDEX IF NOT EXISTS idx_resources_type ON resources(resource_type)",
    "CREATE INDEX IF NOT EXISTS idx_resources_lease ON resources(lease_expires_at)",
];

/// One shared, ordered column list for every read so the row mapping and
/// the SQL can never drift apart.
const RESOURCE_COLUMNS: &str = "id, run_id, work_item_id, repository_id, worker_id, \
     resource_type, external_id, state, ownership, cleanup_policy_json, \
     created_at, updated_at, lease_expires_at, last_heartbeat_at, \
     cleanup_attempts, last_cleanup_error, metadata_json";

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
            for statement in RESOURCES_SCHEMA_SQL {
                sqlx::query(statement)
                    .execute(&pool)
                    .await
                    .map_err(|error| {
                        AutospecError::state(
                            "resource ledger",
                            format!("ensure resources schema: {error}"),
                        )
                    })?;
            }
            Ok::<AnyPool, AutospecError>(pool)
        })?;
        Ok(Self { pool, runtime })
    }

    /// Every ledger row, ordered by `id`.
    pub fn list_all(&self) -> Result<Vec<ManagedResource>, AutospecError> {
        self.fetch(
            &format!("SELECT {RESOURCE_COLUMNS} FROM resources ORDER BY id"),
            &[],
        )
    }

    /// The rows owned by one run, ordered by `id`.
    pub fn list_by_run(&self, run_id: &str) -> Result<Vec<ManagedResource>, AutospecError> {
        self.fetch(
            &format!("SELECT {RESOURCE_COLUMNS} FROM resources WHERE run_id = ? ORDER BY id"),
            &[run_id.to_string()],
        )
    }

    /// The rows of one resource type, ordered by `id`.
    pub fn list_by_type(
        &self,
        resource_type: ResourceType,
    ) -> Result<Vec<ManagedResource>, AutospecError> {
        self.fetch(
            &format!(
                "SELECT {RESOURCE_COLUMNS} FROM resources WHERE resource_type = ? ORDER BY id"
            ),
            &[resource_type.as_str().to_string()],
        )
    }

    /// The one record with this `id`, or `None` when it is absent.
    pub fn get(&self, id: &str) -> Result<Option<ManagedResource>, AutospecError> {
        let rows = self.fetch(
            &format!("SELECT {RESOURCE_COLUMNS} FROM resources WHERE id = ?"),
            &[id.to_string()],
        )?;
        Ok(rows.into_iter().next())
    }

    /// Insert one row. Spec §39's creation transaction lands with the
    /// write path; a single statement is its own transaction. A primary
    /// key collision is a hard `Err`, never an overwrite.
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
}
