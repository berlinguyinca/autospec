//! `autospec resources` — read-only inspection of the persistent resource
//! ledger (spec `docs/specs/2026-08-16-resource-lifecycle-cleanup-design.md`
//! §24.4).
//!
//! This is the operator's only view of the ledger: `list` prints one row per
//! resource (id, type, state, ownership, run id) and `show` prints one full
//! record. No subcommand deletes or mutates a row — every query goes
//! through [`ResourceLedger`]'s read methods only.

use std::str::FromStr;

use autospec_core::resources::db::resolve_db_url;
use autospec_core::resources::model::{ManagedResource, ResourceType};
use autospec_core::resources::ResourceLedger;

use super::CommandFailure;

pub fn run(args: &[String]) -> Result<(), CommandFailure> {
    match args {
        [] => Err(CommandFailure::diagnostic(
            "autospec resources requires a subcommand (list | show)",
        )),
        [flag] if flag == "--help" || flag == "-h" => {
            print_help();
            Ok(())
        }
        [command, rest @ ..] if command == "list" => {
            let output = list(rest)?;
            if !output.is_empty() {
                println!("{output}");
            }
            Ok(())
        }
        [command, rest @ ..] if command == "show" => {
            println!("{}", show(rest)?);
            Ok(())
        }
        [command, ..] => Err(CommandFailure::diagnostic(format!(
            "unknown autospec resources command: {command}"
        ))),
    }
}

#[derive(Debug, Default)]
struct ListOptions {
    run: Option<String>,
    resource_type: Option<ResourceType>,
    json: bool,
}

fn parse_list_options(args: &[String]) -> Result<ListOptions, CommandFailure> {
    let mut options = ListOptions::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--run" => {
                if options.run.is_some() {
                    return Err(CommandFailure::diagnostic("duplicate --run flag"));
                }
                let value = next_value(args, index, "--run")?;
                options.run = Some(value);
                index += 2;
            }
            "--type" => {
                if options.resource_type.is_some() {
                    return Err(CommandFailure::diagnostic("duplicate --type flag"));
                }
                let value = next_value(args, index, "--type")?;
                let resource_type = ResourceType::from_str(&value)
                    .map_err(|error| CommandFailure::diagnostic(error.to_string()))?;
                options.resource_type = Some(resource_type);
                index += 2;
            }
            "--json" => {
                options.json = true;
                index += 1;
            }
            other => {
                return Err(CommandFailure::diagnostic(format!(
                    "unknown autospec resources list flag: {other}"
                )))
            }
        }
    }
    Ok(options)
}

fn next_value(args: &[String], index: usize, flag: &str) -> Result<String, CommandFailure> {
    args.get(index + 1).cloned().ok_or_else(|| {
        CommandFailure::diagnostic(format!(
            "{flag} requires a value (got none); expected: {flag} <value>"
        ))
    })
}

fn list(args: &[String]) -> Result<String, CommandFailure> {
    let options = parse_list_options(args)?;
    let rows = fetch_ledger_rows(options.run.as_deref(), options.resource_type)?;
    if options.json {
        serde_json::to_string(&rows).map_err(|error| {
            CommandFailure::diagnostic(format!("serialize resource rows: {error}"))
        })
    } else {
        Ok(rows.iter().map(render_row).collect::<Vec<_>>().join("\n"))
    }
}

/// Open the resolved ledger and read the matching rows. `--run` and
/// `--type` combine as an exact run filter followed by an exact
/// type filter (an enum equality, not a name match).
fn fetch_ledger_rows(
    run: Option<&str>,
    resource_type: Option<ResourceType>,
) -> Result<Vec<ManagedResource>, CommandFailure> {
    let url = resolve_db_url().map_err(CommandFailure::diagnostic)?;
    let ledger = ResourceLedger::open(&url)
        .map_err(|error| CommandFailure::diagnostic(error.to_string()))?;
    let rows = match run {
        Some(run) => ledger.list_by_run(run),
        None => ledger.list_all(),
    }
    .map_err(|error| CommandFailure::diagnostic(error.to_string()))?;
    match resource_type {
        Some(resource_type) => Ok(rows
            .into_iter()
            .filter(|row| row.resource_type == resource_type)
            .collect()),
        None => Ok(rows),
    }
}

/// One list row: id, type, state, ownership, run id — tab-separated, no
/// header, so the output is one line per resource and greppable by column.
pub(crate) fn render_row(resource: &ManagedResource) -> String {
    format!(
        "{}\t{}\t{}\t{}\t{}",
        resource.id,
        resource.resource_type.as_str(),
        resource.state.as_str(),
        resource.ownership.as_str(),
        resource.run_id
    )
}

fn show(args: &[String]) -> Result<String, CommandFailure> {
    let mut id: Option<String> = None;
    let mut json = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--json" => {
                json = true;
                index += 1;
            }
            value if !value.starts_with("--") => {
                if id.is_some() {
                    return Err(CommandFailure::diagnostic(
                        "autospec resources show accepts exactly one resource id",
                    ));
                }
                id = Some(value.to_string());
                index += 1;
            }
            other => {
                return Err(CommandFailure::diagnostic(format!(
                    "unknown autospec resources show flag: {other}"
                )))
            }
        }
    }
    let id = id.ok_or_else(|| {
        CommandFailure::diagnostic("autospec resources show requires a resource id")
    })?;

    let url = resolve_db_url().map_err(CommandFailure::diagnostic)?;
    let ledger = ResourceLedger::open(&url)
        .map_err(|error| CommandFailure::diagnostic(error.to_string()))?;
    let record = ledger
        .get(&id)
        .map_err(|error| CommandFailure::diagnostic(error.to_string()))?;
    match record {
        Some(record) if json => serde_json::to_string(&record).map_err(|error| {
            CommandFailure::diagnostic(format!("serialize resource record: {error}"))
        }),
        Some(record) => Ok(render_record(&record)),
        None => Err(CommandFailure::diagnostic(format!(
            "resource {id} not found in ledger"
        ))),
    }
}

/// The full record as `field: value` lines; absent optionals print `-` so
/// no column is ever blank-by-omission.
pub(crate) fn render_record(resource: &ManagedResource) -> String {
    let optional = |value: &Option<String>| value.clone().unwrap_or_else(|| "-".into());
    [
        format!("id: {}", resource.id),
        format!("run id: {}", resource.run_id),
        format!("work item id: {}", optional(&resource.work_item_id)),
        format!("repository id: {}", optional(&resource.repository_id)),
        format!("worker id: {}", optional(&resource.worker_id)),
        format!("type: {}", resource.resource_type.as_str()),
        format!("external id: {}", resource.external_id),
        format!("state: {}", resource.state.as_str()),
        format!("ownership: {}", resource.ownership.as_str()),
        format!("cleanup policy: {}", resource.cleanup_policy),
        format!("created at: {}", resource.created_at),
        format!("updated at: {}", resource.updated_at),
        format!("lease expires at: {}", optional(&resource.lease_expires_at)),
        format!(
            "last heartbeat at: {}",
            optional(&resource.last_heartbeat_at)
        ),
        format!("cleanup attempts: {}", resource.cleanup_attempts),
        format!(
            "last cleanup error: {}",
            optional(&resource.last_cleanup_error)
        ),
        format!("metadata: {}", resource.metadata),
    ]
    .join("\n")
}

fn print_help() {
    println!(
        r#"autospec resources

USAGE:
    autospec resources list [--run <run-id>] [--type <resource-type>] [--json]
    autospec resources show <resource-id> [--json]

SUBCOMMANDS:
    list    List ledger rows (one row per resource: id, type, state, ownership, run id)
    show    Show one ledger record by id; exits non-zero when the id is absent

FLAGS:
    --run <run-id>    list only the rows owned by one run
    --type <type>     list only one resource type (git_worktree, docker_container, ...)
    --json            emit JSON (an array for list, one object for show)

Read-only: no subcommand deletes or mutates a ledger row."#
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::CommandFailureKind;
    use autospec_core::resources::model::{OwnershipClass, ResourceState};
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());
    static DIR_COUNTER: Mutex<u32> = Mutex::new(0);

    /// Point the ledger at a fresh temp-dir SQLite file for the duration of
    /// `body`, restoring the previous `AUTOSPEC_DB_URL` afterward. Returns
    /// the temp dir (kept alive until the guard drops).
    /// Lock a test mutex, recovering from a poisoned guard (a panicked
    /// sibling test) instead of failing every later test on the thread pool.
    fn lock_poison_tolerant<T>(lock: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
        lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn with_temp_ledger<T>(body: impl FnOnce(&str) -> T) -> (T, std::path::PathBuf) {
        let _env_guard = lock_poison_tolerant(&ENV_LOCK);
        // The counter guard must be dropped before `body` runs: a panic in
        // `body` must not poison the counter and fail every later test.
        let dir = {
            let mut counter = lock_poison_tolerant(&DIR_COUNTER);
            *counter += 1;
            std::env::temp_dir().join(format!(
                "autospec-resources-cli-test-{}-{}",
                std::process::id(),
                *counter
            ))
        };
        std::fs::create_dir_all(&dir).unwrap();
        let url = format!("sqlite://{}/autospec.db", dir.display());
        let previous = std::env::var("AUTOSPEC_DB_URL");
        std::env::set_var("AUTOSPEC_DB_URL", &url);
        let result = body(&url);
        match previous {
            Ok(value) => std::env::set_var("AUTOSPEC_DB_URL", value),
            Err(_) => std::env::remove_var("AUTOSPEC_DB_URL"),
        }
        (result, dir)
    }

    fn seed_resource(
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
            cleanup_attempts: 0,
            last_cleanup_error: None,
            metadata: serde_json::json!({}),
        }
    }

    #[test]
    fn list_exits_zero_and_prints_nothing_against_an_empty_ledger() {
        let ((result, rows), _keep) = with_temp_ledger(|_url| {
            let result = run(&[String::from("list"), String::from("--json")]);
            let rows = fetch_ledger_rows(None, None).expect("read the empty ledger");
            (result, rows)
        });
        assert!(
            result.is_ok(),
            "an empty ledger is a valid state, not an error"
        );
        assert!(rows.is_empty());
        assert_eq!(
            rows.iter().map(render_row).collect::<Vec<_>>().join("\n"),
            ""
        );
    }

    #[test]
    fn list_json_emits_a_parseable_json_array_of_rows() {
        let ((result, rows), _keep) = with_temp_ledger(|url| {
            let url = url.to_string();
            let ledger = ResourceLedger::open(&url).expect("open ledger");
            ledger
                .insert(&seed_resource(
                    "res-1",
                    "run-1",
                    ResourceType::GitWorktree,
                    ResourceState::Active,
                    OwnershipClass::RunExclusive,
                ))
                .expect("seed row");
            let result = run(&[String::from("list"), String::from("--json")]);
            let rows = fetch_ledger_rows(None, None).expect("read rows");
            (result, rows)
        });
        assert!(result.is_ok());
        let json = serde_json::to_string(&rows).expect("serialize rows");
        let parsed: serde_json::Value =
            serde_json::from_str(&json).expect("list --json must parse as a JSON array");
        let rows = parsed
            .as_array()
            .expect("the top-level JSON value must be an array");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["id"], "res-1");
        assert_eq!(rows[0]["resource_type"], "git_worktree");
        assert_eq!(rows[0]["run_id"], "run-1");
    }

    #[test]
    fn show_on_an_unknown_id_fails_closed_with_a_nonzero_diagnostic() {
        let (result, _keep) =
            with_temp_ledger(|_url| run(&[String::from("show"), String::from("missing-id")]));
        let failure = result.expect_err("an absent id must fail, never print an empty record");
        assert_ne!(failure.exit_code, 0);
        assert!(
            matches!(failure.kind, CommandFailureKind::Diagnostic),
            "absent id is a diagnostic failure, got: {:?}",
            failure.kind
        );
        assert!(
            failure.message.contains("missing-id"),
            "the diagnostic names the missing id, got: {}",
            failure.message
        );
    }

    #[test]
    fn six_thousand_five_hundred_rows_print_exactly_six_thousand_five_hundred_lines() {
        let ((result, rows), _keep) = with_temp_ledger(|url| {
            let url = url.to_string();
            let ledger = ResourceLedger::open(&url).expect("open ledger");
            for index in 0..6500u32 {
                ledger
                    .insert(&seed_resource(
                        &format!("res-{index:05}"),
                        &format!("run-{}", index % 7),
                        ResourceType::TempDirectory,
                        ResourceState::Retained,
                        OwnershipClass::RunExclusive,
                    ))
                    .expect("seed row");
            }
            let result = run(&[String::from("list")]);
            let rows = fetch_ledger_rows(None, None).expect("read all rows");
            (result, rows)
        });
        assert!(result.is_ok());
        let output = rows.iter().map(render_row).collect::<Vec<_>>().join("\n");
        assert_eq!(output.lines().count(), 6500);
        for line in output.lines() {
            assert_eq!(
                line.split('\t').count(),
                5,
                "every line carries id, type, state, ownership, run id: {line}"
            );
        }
    }

    #[test]
    fn an_external_row_prints_its_ownership_not_a_blank_column() {
        let ((result, rows), _keep) = with_temp_ledger(|url| {
            let url = url.to_string();
            let ledger = ResourceLedger::open(&url).expect("open ledger");
            ledger
                .insert(&seed_resource(
                    "res-ext",
                    "run-1",
                    ResourceType::DockerNetwork,
                    ResourceState::Active,
                    OwnershipClass::External,
                ))
                .expect("seed row");
            let result = run(&[String::from("list")]);
            let rows = fetch_ledger_rows(None, None).expect("read rows");
            (result, rows)
        });
        assert!(result.is_ok());
        assert_eq!(rows.len(), 1);
        let line = render_row(&rows[0]);
        let ownership = line.split('\t').nth(3).expect("ownership column");
        assert_eq!(
            ownership, "external",
            "an External row prints its class verbatim, never a blank column"
        );
    }

    #[test]
    fn list_filters_by_run_and_by_type_exactly() {
        let ((result, by_run, by_type, both), _keep) = with_temp_ledger(|url| {
            let url = url.to_string();
            let ledger = ResourceLedger::open(&url).expect("open ledger");
            let seeds = [
                seed_resource(
                    "res-a",
                    "run-1",
                    ResourceType::GitWorktree,
                    ResourceState::Active,
                    OwnershipClass::RunExclusive,
                ),
                seed_resource(
                    "res-b",
                    "run-1",
                    ResourceType::DockerContainer,
                    ResourceState::Active,
                    OwnershipClass::RunExclusive,
                ),
                seed_resource(
                    "res-c",
                    "run-2",
                    ResourceType::GitWorktree,
                    ResourceState::Active,
                    OwnershipClass::RepoShared,
                ),
                seed_resource(
                    "res-d",
                    "run-2",
                    ResourceType::DockerContainer,
                    ResourceState::Active,
                    OwnershipClass::GlobalShared,
                ),
            ];
            for seed in &seeds {
                ledger.insert(seed).expect("seed row");
            }
            let result = run(&[
                String::from("list"),
                String::from("--run"),
                String::from("run-1"),
            ]);
            let by_run = fetch_ledger_rows(Some("run-1"), None).expect("by run");
            let by_type =
                fetch_ledger_rows(None, Some(ResourceType::GitWorktree)).expect("by type");
            let both = fetch_ledger_rows(Some("run-2"), Some(ResourceType::GitWorktree))
                .expect("by run and type");
            (result, by_run, by_type, both)
        });
        assert!(result.is_ok());
        assert_eq!(
            by_run.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
            vec!["res-a", "res-b"]
        );
        assert_eq!(
            by_type
                .iter()
                .map(|row| row.id.as_str())
                .collect::<Vec<_>>(),
            vec!["res-a", "res-c"]
        );
        assert_eq!(
            both.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
            vec!["res-c"]
        );
    }

    #[test]
    fn show_prints_the_full_record_and_json_round_trips_it() {
        let ((result, record), _keep) = with_temp_ledger(|url| {
            let url = url.to_string();
            let ledger = ResourceLedger::open(&url).expect("open ledger");
            ledger
                .insert(&seed_resource(
                    "res-1",
                    "run-1",
                    ResourceType::GitWorktree,
                    ResourceState::Active,
                    OwnershipClass::RunExclusive,
                ))
                .expect("seed row");
            let result = run(&[
                String::from("show"),
                String::from("res-1"),
                String::from("--json"),
            ]);
            let record = fetch_ledger_rows(None, None)
                .expect("read rows")
                .pop()
                .expect("seeded row");
            (result, record)
        });
        assert!(result.is_ok());
        let text = render_record(&record);
        for expected in [
            "id: res-1",
            "run id: run-1",
            "type: git_worktree",
            "state: active",
            "ownership: run_exclusive",
            "external id: /tmp/wt-res-1",
        ] {
            assert!(
                text.lines().any(|line| line == expected),
                "the record renders {expected:?}, got:\n{text}"
            );
        }
        let json = serde_json::to_string(&record).expect("serialize record");
        let parsed: serde_json::Value =
            serde_json::from_str(&json).expect("show --json must parse as a JSON object");
        assert_eq!(parsed["id"], "res-1");
        assert_eq!(parsed["ownership"], "run_exclusive");
    }

    #[test]
    fn unknown_subcommands_and_flags_are_diagnostics_not_panic_paths() {
        let cases = [
            vec![String::from("wipe")],
            vec![],
            vec![String::from("list"), String::from("--bogus")],
            vec![String::from("list"), String::from("--run")],
            vec![
                String::from("list"),
                String::from("--type"),
                String::from("not_a_resource_type"),
            ],
            vec![String::from("show")],
            vec![String::from("show"), String::from("a"), String::from("b")],
            vec![String::from("show"), String::from("--bogus")],
        ];
        for case in &cases {
            let failure = run(case).expect_err("these invocations must all fail");
            assert_ne!(failure.exit_code, 0);
            assert!(!failure.message.is_empty());
            assert!(
                matches!(failure.kind, CommandFailureKind::Diagnostic),
                "case {case:?} must be a diagnostic, got: {failure:?}"
            );
        }
    }
}
