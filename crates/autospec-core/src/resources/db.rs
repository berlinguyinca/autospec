//! Shared storage boundary for the one autospec database (ADR 0001 D5, D7,
//! D10; spec `docs/specs/2026-08-16-resource-lifecycle-cleanup-design.md`
//! §7.2/§12).
//!
//! The database is global: ONE file named `autospec.db` under
//! `~/.autospec/state/` by default, overridable with `AUTOSPEC_DB_URL`.
//! Both the SQLite and the Postgres backend are real (D10); async (sqlx +
//! tokio) is confined to this module so the rest of `autospec-core` stays
//! synchronous (D5 bounding condition, AS-AEO-001 §66.1).
//!
//! Migrations are namespaced per subsystem under
//! `crates/autospec-core/migrations/<subsystem>/` so the resource subsystem
//! and AS-AEO-001 Epic 2 can both add migrations to the shared database
//! without colliding (D7). Application state is tracked per subsystem in the
//! shared `autospec_migrations` table; version ranges: `resources` owns
//! `1xxxxxx`, `core` owns `2xxxxxx`, `insights` owns the development range
//! `3xxxxxx` (3000001-3999999).

use std::path::Path;

use sha2::{Digest, Sha256};
use sqlx::any::AnyPoolOptions;
use sqlx::AnyPool;
use sqlx::Executor;
use sqlx::Row;

use crate::error::AutospecError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Backend {
    Sqlite,
    Postgres,
}

/// Split `url` into its backend and the remainder after the scheme.
///
/// Only `sqlite://` and `postgres://` (or `postgresql://`) are accepted; any
/// other — or missing — scheme is rejected, never defaulted.
fn parse_url(url: &str) -> Result<(Backend, &str), AutospecError> {
    let (scheme, rest) = url.split_once("://").ok_or_else(|| {
        AutospecError::validation(format!(
            "database URL {url:?} has no scheme; expected sqlite:// or postgres://"
        ))
    })?;
    match scheme {
        "sqlite" => {
            if rest.is_empty() {
                return Err(AutospecError::validation(
                    "sqlite URL has no path after the scheme",
                ));
            }
            Ok((Backend::Sqlite, rest))
        }
        "postgres" | "postgresql" => Ok((Backend::Postgres, rest)),
        other => Err(AutospecError::validation(format!(
            "unknown database scheme {other:?}; expected sqlite or postgres (never defaulted)"
        ))),
    }
}

fn map_open_error(url: &str, error: sqlx::Error) -> AutospecError {
    AutospecError::Io {
        operation: "open database".to_string(),
        path: url.to_string(),
        source: error.to_string(),
    }
}

/// Default database location: `sqlite://$HOME/.autospec/state/autospec.db`
/// (D7: ONE database; D10: one file named `autospec.db`, global, under
/// `~/.autospec`).
pub fn default_db_url(home: &Path) -> String {
    format!("sqlite://{}/.autospec/state/autospec.db", home.display())
}

fn home_dir() -> Result<String, AutospecError> {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map_err(|_| {
            AutospecError::validation(
                "no HOME or USERPROFILE set; cannot resolve the default database path",
            )
        })
}

/// Resolve the database URL: `$AUTOSPEC_DB_URL` when set and non-empty, else
/// the default `sqlite://$HOME/.autospec/state/autospec.db`. A bad value
/// (empty, non-UTF-8) fails loudly; it is never silently replaced by the
/// default.
pub fn resolve_db_url() -> Result<String, AutospecError> {
    match std::env::var("AUTOSPEC_DB_URL") {
        Ok(value) if !value.trim().is_empty() => Ok(value),
        Ok(_) => Err(AutospecError::validation(
            "AUTOSPEC_DB_URL is set but empty; refusing to fall back to the default",
        )),
        Err(std::env::VarError::NotPresent) => Ok(default_db_url(Path::new(&home_dir()?))),
        Err(std::env::VarError::NotUnicode(_)) => Err(AutospecError::validation(
            "AUTOSPEC_DB_URL is not valid UTF-8",
        )),
    }
}

/// Open the shared autospec database at `url`, returning a backend-neutral
/// [`AnyPool`] (D10: both the SQLite and the Postgres backend are real).
///
/// `sqlite://` URLs accept a filesystem path after the scheme: parent
/// directories are created, the file is created if missing, and the
/// SQLite-only `journal_mode=WAL` pragma is applied (WAL is persisted in the
/// database file, so it holds for every pooled connection; sqlx-sqlite also
/// applies `foreign_keys=ON` and a `busy_timeout` to every SQLite
/// connection by default). `postgres://` (or `postgresql://`) URLs are
/// passed through to the Postgres backend untouched; an unreadable SQLite
/// path or an unreachable Postgres server fails loudly instead of
/// degrading.
pub async fn open_shared_db(url: &str) -> Result<AnyPool, AutospecError> {
    let (backend, rest) = parse_url(url)?;
    // `create_if_missing` is off when sqlx-sqlite parses a URL; `mode=rwc`
    // is the URL spelling of "create the file if it does not exist".
    let connect_url = match (backend, rest.contains('?')) {
        (Backend::Sqlite, false) => format!("{url}?mode=rwc"),
        (Backend::Sqlite, true) => format!("{url}&mode=rwc"),
        _ => url.to_string(),
    };
    if backend == Backend::Sqlite {
        let path = Path::new(rest);
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|error| {
                    AutospecError::io(
                        "create database parent directory",
                        parent.display().to_string(),
                        error,
                    )
                })?;
            }
        }
    }

    // One-time registration of the compiled-in SQLite/Postgres drivers with
    // the Any (runtime-generic) connection; cheap to repeat.
    sqlx::any::install_default_drivers();

    let pool: AnyPool = AnyPoolOptions::new()
        .connect(&connect_url)
        .await
        .map_err(|error| map_open_error(url, error))?;

    if backend == Backend::Sqlite {
        // SQLite-only pragmas: WAL is a persistent, file-level setting, so
        // applying it once on open covers every pooled connection.
        let mut conn = pool
            .acquire()
            .await
            .map_err(|error| map_open_error(url, error))?;
        (&mut *conn)
            .execute(sqlx::raw_sql("PRAGMA journal_mode = WAL"))
            .await
            .map_err(|error| map_open_error(url, error))?;
    }

    Ok(pool)
}

/// Open the shared database at the resolved default URL
/// ([`resolve_db_url`]).
pub async fn open_shared_db_default() -> Result<AnyPool, AutospecError> {
    open_shared_db(&resolve_db_url()?).await
}

/// Migration subsystem owned by the resource lifecycle epic (#3185).
pub const SUBSYSTEM_RESOURCES: &str = "resources";
/// Migration subsystem owned by the AS-AEO-001 persistence layer (Epic 2).
pub const SUBSYSTEM_CORE: &str = "core";
/// Migration subsystem owned by the continuous improvement engine (insights),
/// development range 3xxxxxx (3000001-3999999).
pub const SUBSYSTEM_INSIGHTS: &str = "insights";

/// One embedded, subsystem-namespaced migration (D7).
#[derive(Debug)]
struct EmbeddedMigration {
    version: i64,
    description: &'static str,
    sql: &'static str,
}

const RESOURCES_MIGRATIONS: [EmbeddedMigration; 1] = [EmbeddedMigration {
    version: 1000001,
    description: "init",
    sql: include_str!("../../migrations/resources/1000001_init.sql"),
}];

const CORE_MIGRATIONS: [EmbeddedMigration; 1] = [EmbeddedMigration {
    version: 2000001,
    description: "init",
    sql: include_str!("../../migrations/core/2000001_init.sql"),
}];

const INSIGHTS_MIGRATIONS: [EmbeddedMigration; 1] = [EmbeddedMigration {
    version: 3000001,
    description: "init",
    sql: include_str!("../../migrations/insights/3000001_init.sql"),
}];

fn migrations_for_subsystem(
    subsystem: &str,
) -> Result<&'static [EmbeddedMigration], AutospecError> {
    match subsystem {
        SUBSYSTEM_RESOURCES => Ok(&RESOURCES_MIGRATIONS),
        SUBSYSTEM_CORE => Ok(&CORE_MIGRATIONS),
        SUBSYSTEM_INSIGHTS => Ok(&INSIGHTS_MIGRATIONS),
        other => Err(AutospecError::validation(format!(
            "unknown migration subsystem {other:?}"
        ))),
    }
}

fn checksum(bytes: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(bytes);
    digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Apply `subsystem`'s namespaced migrations to the shared database.
/// Idempotent: already-applied versions are tracked per subsystem in
/// `autospec_migrations` and skipped on re-run, so two subsystems can add
/// migrations to the ONE database without colliding (D7).
pub async fn apply_subsystem_migrations(
    pool: &AnyPool,
    subsystem: &str,
) -> Result<(), AutospecError> {
    let migrations = migrations_for_subsystem(subsystem)?;
    let mut conn = pool
        .acquire()
        .await
        .map_err(|error| AutospecError::state(subsystem, error.to_string()))?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS autospec_migrations (\n\
         subsystem TEXT NOT NULL,\n\
         version TEXT NOT NULL,\n\
         description TEXT NOT NULL,\n\
         checksum TEXT NOT NULL,\n\
         PRIMARY KEY (subsystem, version)\n\
         )",
    )
    .execute(&mut *conn)
    .await
    .map_err(|error| AutospecError::state(subsystem, error.to_string()))?;

    let rows = sqlx::query("SELECT version FROM autospec_migrations WHERE subsystem = ?")
        .bind(subsystem)
        .fetch_all(&mut *conn)
        .await
        .map_err(|error| AutospecError::state(subsystem, error.to_string()))?;
    let applied: std::collections::BTreeSet<String> = rows
        .iter()
        .filter_map(|row| row.try_get::<String, _>(0).ok())
        .collect();

    for migration in migrations {
        let version = migration.version.to_string();
        if applied.contains(&version) {
            continue;
        }
        sqlx::query(migration.sql)
            .execute(&mut *conn)
            .await
            .map_err(|error| {
                AutospecError::state(
                    format!(
                        "{subsystem} migration {version} ({})",
                        migration.description
                    ),
                    error.to_string(),
                )
            })?;
        sqlx::query(
            "INSERT INTO autospec_migrations (subsystem, version, description, checksum) \n\
             VALUES (?, ?, ?, ?)",
        )
        .bind(subsystem)
        .bind(version)
        .bind(migration.description)
        .bind(checksum(migration.sql.as_bytes()))
        .execute(&mut *conn)
        .await
        .map_err(|error| AutospecError::state(subsystem, error.to_string()))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::Row;
    use std::path::PathBuf;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());
    static DIR_COUNTER: Mutex<u32> = Mutex::new(0);

    fn fresh_tmp_dir() -> PathBuf {
        let mut counter = DIR_COUNTER.lock().unwrap();
        *counter += 1;
        let dir = std::env::temp_dir().join(format!(
            "autospec-db-test-{}-{}",
            std::process::id(),
            *counter
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    async fn scalar_text(pool: &AnyPool, sql: &str) -> String {
        let mut conn = pool.acquire().await.unwrap();
        let row = sqlx::raw_sql(sql).fetch_one(&mut *conn).await.unwrap();
        row.try_get::<String, _>(0).unwrap()
    }

    async fn scalar_i64(pool: &AnyPool, sql: &str) -> i64 {
        let mut conn = pool.acquire().await.unwrap();
        let row = sqlx::raw_sql(sql).fetch_one(&mut *conn).await.unwrap();
        row.try_get::<i64, _>(0).unwrap()
    }

    #[test]
    fn default_db_url_is_one_autospec_db_under_home_dot_autospec() {
        assert_eq!(
            default_db_url(Path::new("/home/agent")),
            "sqlite:///home/agent/.autospec/state/autospec.db"
        );
    }

    #[test]
    fn resolve_db_url_honors_autospec_db_url_and_rejects_empty() {
        let _guard = ENV_LOCK.lock().unwrap();
        let previous = std::env::var("AUTOSPEC_DB_URL");
        std::env::set_var(
            "AUTOSPEC_DB_URL",
            "postgres://agent@127.0.0.1:5432/autospec",
        );
        assert_eq!(
            resolve_db_url().unwrap(),
            "postgres://agent@127.0.0.1:5432/autospec"
        );
        std::env::set_var("AUTOSPEC_DB_URL", "");
        assert!(
            matches!(
                resolve_db_url().unwrap_err(),
                AutospecError::Validation { .. }
            ),
            "an empty AUTOSPEC_DB_URL must fail loudly, never default"
        );
        match previous {
            Ok(value) => std::env::set_var("AUTOSPEC_DB_URL", value),
            Err(_) => std::env::remove_var("AUTOSPEC_DB_URL"),
        }
    }

    #[test]
    fn parse_url_rejects_unknown_and_missing_schemes() {
        assert!(matches!(
            parse_url("mysql://127.0.0.1/autospec"),
            Err(AutospecError::Validation { .. })
        ));
        assert!(matches!(
            parse_url("not-a-url"),
            Err(AutospecError::Validation { .. })
        ));
        assert!(matches!(
            parse_url("sqlite://"),
            Err(AutospecError::Validation { .. })
        ));
        assert_eq!(
            parse_url("sqlite:///tmp/autospec.db").unwrap(),
            (Backend::Sqlite, "/tmp/autospec.db")
        );
        assert_eq!(
            parse_url("postgres://127.0.0.1:5432/autospec").unwrap(),
            (Backend::Postgres, "127.0.0.1:5432/autospec")
        );
        assert_eq!(
            parse_url("postgresql://127.0.0.1:5432/autospec").unwrap(),
            (Backend::Postgres, "127.0.0.1:5432/autospec")
        );
    }

    #[tokio::test]
    async fn opens_a_real_sqlite_database_with_the_sqlite_only_pragmas() {
        let dir = fresh_tmp_dir();
        let url = format!("sqlite://{}/nested/deep/autospec.db", dir.display());
        let pool = open_shared_db(&url)
            .await
            .expect("parent directories must be created");

        assert!(dir.join("nested/deep/autospec.db").exists());
        assert_eq!(scalar_text(&pool, "PRAGMA journal_mode").await, "wal");
        assert_eq!(scalar_i64(&pool, "PRAGMA foreign_keys").await, 1);
        assert!(
            scalar_i64(&pool, "PRAGMA busy_timeout").await > 0,
            "busy_timeout must be set on SQLite"
        );
    }

    #[tokio::test]
    async fn unreadable_sqlite_path_fails_loudly() {
        let dir = fresh_tmp_dir();
        let blocker = dir.join("blocker");
        std::fs::write(&blocker, b"a file, not a directory").unwrap();
        let url = format!("sqlite://{}/nested/autospec.db", blocker.display());
        let error = open_shared_db(&url).await.unwrap_err();
        assert!(
            matches!(error, AutospecError::Io { .. }),
            "expected a loud IO failure, got: {error}"
        );
    }

    #[tokio::test]
    async fn unknown_scheme_is_rejected_not_defaulted() {
        let error = open_shared_db("mysql://127.0.0.1/autospec")
            .await
            .unwrap_err();
        assert!(
            matches!(error, AutospecError::Validation { .. }),
            "expected a scheme rejection, got: {error}"
        );
    }

    #[tokio::test]
    async fn postgres_url_is_accepted_and_fails_on_connect_not_on_scheme() {
        // Nothing listens on 127.0.0.1:1, so this must fail with a connect
        // (IO) error — proof the postgres:// scheme is accepted by
        // open_shared_db and routed to the Postgres backend, never rejected
        // as an unknown scheme and never redirected to SQLite.
        let url = "postgres://autospec:secret@127.0.0.1:1/autospec?connect_timeout=2";
        let error = open_shared_db(url).await.unwrap_err();
        assert!(
            !matches!(error, AutospecError::Validation { .. }),
            "postgres:// must be accepted as a scheme, got: {error}"
        );
    }

    #[tokio::test]
    async fn two_subsystems_apply_migrations_without_collision() {
        let dir = fresh_tmp_dir();
        let url = format!("sqlite://{}/autospec.db", dir.display());
        let pool = open_shared_db(&url).await.unwrap();

        apply_subsystem_migrations(&pool, SUBSYSTEM_RESOURCES)
            .await
            .expect("resources migrations must apply");
        apply_subsystem_migrations(&pool, SUBSYSTEM_CORE)
            .await
            .expect("core migrations must apply beside resources");

        assert_eq!(
            scalar_i64(&pool, "SELECT COUNT(*) FROM autospec_migrations").await,
            2
        );

        // Idempotent: a second pass (Epic 2 re-run) must not collide.
        apply_subsystem_migrations(&pool, SUBSYSTEM_RESOURCES)
            .await
            .unwrap();
        apply_subsystem_migrations(&pool, SUBSYSTEM_CORE)
            .await
            .unwrap();
        assert_eq!(
            scalar_i64(&pool, "SELECT COUNT(*) FROM autospec_migrations").await,
            2
        );
    }

    #[test]
    fn unknown_migration_subsystem_is_rejected() {
        assert!(matches!(
            migrations_for_subsystem("bench").unwrap_err(),
            AutospecError::Validation { .. }
        ));
    }

    /// The 16 core tables of spec §34 (continuous improvement engine).
    const INSIGHTS_TABLES: [&str; 16] = [
        "sessions",
        "session_events",
        "session_summaries",
        "user_interventions",
        "tool_invocations",
        "model_invocations",
        "git_events",
        "ci_events",
        "review_findings",
        "quality_findings",
        "patterns",
        "finding_evidence",
        "improvement_proposals",
        "proposal_evaluations",
        "configuration_versions",
        "post_change_measurements",
    ];

    #[test]
    fn insights_migrations_for_subsystem_returns_one_embedded_item() {
        let migrations = migrations_for_subsystem(SUBSYSTEM_INSIGHTS)
            .expect("insights must be a registered migration subsystem");
        assert_eq!(migrations.len(), 1);
        assert_eq!(migrations[0].version, 3000001);
    }

    #[test]
    fn insights_migrations_versions_stay_inside_the_development_range() {
        let migrations = migrations_for_subsystem(SUBSYSTEM_INSIGHTS).unwrap();
        assert!(!migrations.is_empty());
        for migration in migrations {
            assert!(
                (3000001..=3999999).contains(&migration.version),
                "insights version {} left the development range 3000001-3999999",
                migration.version
            );
        }
    }

    #[tokio::test]
    async fn insights_migrations_create_sixteen_tables_on_sqlite() {
        let dir = fresh_tmp_dir();
        let url = format!("sqlite://{}/autospec.db", dir.display());
        let pool = open_shared_db(&url).await.unwrap();
        apply_subsystem_migrations(&pool, SUBSYSTEM_INSIGHTS)
            .await
            .expect("insights migrations must apply on SQLite");
        for table in INSIGHTS_TABLES {
            let count = scalar_i64(
                &pool,
                &format!(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = '{table}'"
                ),
            )
            .await;
            assert_eq!(count, 1, "table {table} is missing from SQLite");
        }
    }

    #[tokio::test]
    async fn insights_migrations_double_apply_inserts_exactly_one_row() {
        let dir = fresh_tmp_dir();
        let url = format!("sqlite://{}/autospec.db", dir.display());
        let pool = open_shared_db(&url).await.unwrap();
        apply_subsystem_migrations(&pool, SUBSYSTEM_INSIGHTS)
            .await
            .unwrap();
        apply_subsystem_migrations(&pool, SUBSYSTEM_INSIGHTS)
            .await
            .expect("second apply must be a no-op, not a collision");
        assert_eq!(
            scalar_i64(
                &pool,
                "SELECT COUNT(*) FROM autospec_migrations WHERE subsystem = 'insights'"
            )
            .await,
            1
        );
    }

    #[tokio::test]
    async fn insights_migrations_apply_on_postgres_16() {
        let Ok(url) = std::env::var("AUTOSPEC_TEST_DB_URL") else {
            eprintln!(
                "SKIP insights_migrations_apply_on_postgres_16: \
                 AUTOSPEC_TEST_DB_URL is not set"
            );
            return;
        };
        let pool = open_shared_db(&url)
            .await
            .expect("AUTOSPEC_TEST_DB_URL must point at a reachable PostgreSQL 16");
        apply_subsystem_migrations(&pool, SUBSYSTEM_INSIGHTS)
            .await
            .expect("insights migrations must apply on PostgreSQL 16");
        apply_subsystem_migrations(&pool, SUBSYSTEM_INSIGHTS)
            .await
            .expect("double-apply must be idempotent on PostgreSQL 16");
        assert_eq!(
            scalar_i64(
                &pool,
                "SELECT COUNT(*) FROM autospec_migrations WHERE subsystem = 'insights'"
            )
            .await,
            1,
            "double-apply must leave exactly 1 autospec_migrations row"
        );
        let list = INSIGHTS_TABLES
            .iter()
            .map(|table| format!("'{table}'"))
            .collect::<Vec<_>>()
            .join(", ");
        let count = scalar_i64(
            &pool,
            &format!(
                "SELECT COUNT(*) FROM information_schema.tables \
                 WHERE table_schema = 'public' AND table_name IN ({list})"
            ),
        )
        .await;
        assert_eq!(count, 16, "all 16 §34 tables must exist on PostgreSQL 16");
    }
}
