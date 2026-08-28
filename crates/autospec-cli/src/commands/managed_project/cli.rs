use super::super::autonomous::accountability::github::{GhCli, GithubTransport};
use super::{
    onboard_repositories, reconcile_issue, resolve_or_create_project, retry_pending_projections,
    ManagedProjectError, ManagedProjectStore, OnboardingOptions, OnboardingReport,
};
use autospec_core::autonomous::config::AutonomousConfig;
use autospec_core::managed_project::{
    RelationshipEdge, RelationshipEvidence, RelationshipKind, RelationshipState,
};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

pub fn run(args: &[String]) -> Result<(), ManagedProjectError> {
    run_with_transport(args, &mut GhCli)
}

pub(crate) fn run_with_transport<T: GithubTransport>(
    args: &[String],
    github: &mut T,
) -> Result<(), ManagedProjectError> {
    if args.is_empty()
        || args
            .iter()
            .any(|argument| matches!(argument.as_str(), "--help" | "-h"))
    {
        println!("autospec project\n\nUSAGE:\n    autospec project resolve --repo-dir PATH\n    autospec project sync --repo-dir PATH [--issue-url URL]\n    autospec project onboard --repo-dir PATH [--repo OWNER/NAME]... [--workspace PATH] [--spawned-from IDENTITY] [--dry-run]");
        return Ok(());
    }
    let command = &args[0];
    let options = parse_options(&args[1..])?;
    validate_options(command, &options)?;
    let repo_dir = options
        .repo_dir
        .clone()
        .ok_or_else(|| ManagedProjectError::new("autospec project requires --repo-dir"))?;
    let policy = load_managed_policy(&repo_dir)?;
    validate_explicit_seeds(&policy, &options.repositories)?;
    let state_root = repo_dir.join(".autospec/state");
    let mut store = if options.dry_run {
        ManagedProjectStore::open_read_only(&state_root, &policy.product_key)?
    } else {
        ManagedProjectStore::open(&state_root, &policy.product_key)?
    };

    match command.as_str() {
        "resolve" => resolve(&mut store, github, &policy),
        "sync" => sync(&mut store, github, &policy, options.issue_url.as_deref()),
        "onboard" => onboard(store, github, policy, &repo_dir, options),
        other => Err(ManagedProjectError::new(format!(
            "unknown autospec project subcommand: {other}"
        ))),
    }
}

fn resolve<T: GithubTransport>(
    store: &mut ManagedProjectStore,
    github: &mut T,
    policy: &autospec_core::managed_project::ManagedProjectPolicy,
) -> Result<(), ManagedProjectError> {
    let project = resolve_or_create_project(store, github, policy, policy.product_key.as_str())?;
    println!(
        "{}",
        json!({
            "node_id": project.node_id,
            "number": project.number,
            "owner": project.owner,
            "title": project.title,
            "url": project.url,
        })
    );
    Ok(())
}

fn sync<T: GithubTransport>(
    store: &mut ManagedProjectStore,
    github: &mut T,
    policy: &autospec_core::managed_project::ManagedProjectPolicy,
    issue_url: Option<&str>,
) -> Result<(), ManagedProjectError> {
    let project = resolve_or_create_project(store, github, policy, policy.product_key.as_str())?;
    if let Some(issue_url) = issue_url {
        reconcile_issue(store, github, policy, issue_url)?;
    }
    retry_pending_projections(store, github, policy)?;
    println!(
        "{}",
        json!({
            "pending_projection": store.snapshot().pending_projections.len(),
            "project_url": project.url,
        })
    );
    Ok(())
}

fn onboard<T: GithubTransport>(
    mut store: ManagedProjectStore,
    github: &mut T,
    policy: autospec_core::managed_project::ManagedProjectPolicy,
    repo_dir: &Path,
    options: ProjectOptions,
) -> Result<(), ManagedProjectError> {
    let project_url = if !options.dry_run {
        let project =
            resolve_or_create_project(&mut store, github, &policy, policy.product_key.as_str())?;
        retry_pending_projections(&mut store, github, &policy)?;
        Some(project.url)
    } else {
        store.snapshot().project_url.clone()
    };
    let created_repository = options.repositories.first().cloned();
    let report = onboard_repositories(
        &mut store,
        &policy,
        &OnboardingOptions {
            repo_dir: repo_dir.to_path_buf(),
            repositories: options.repositories,
            workspaces: options.workspaces,
            dry_run: options.dry_run,
        },
    )?;
    if !options.dry_run {
        record_repository_relationships(
            &mut store,
            &policy,
            &report,
            options.spawned_from.as_deref(),
            created_repository.as_deref(),
        )?;
    }
    println!("{}", report_json(&report, project_url.as_deref()));
    Ok(())
}

fn record_repository_relationships(
    store: &mut ManagedProjectStore,
    policy: &autospec_core::managed_project::ManagedProjectPolicy,
    report: &OnboardingReport,
    spawned_from: Option<&str>,
    created_repository: Option<&str>,
) -> Result<(), ManagedProjectError> {
    for repository in &report.repositories {
        store.record_edge(RelationshipEdge {
            product_key: policy.product_key.clone(),
            kind: RelationshipKind::Contains,
            source: format!("product:{}", policy.product_key),
            target: repository.repository.clone(),
            evidence: RelationshipEvidence {
                kind: "repository-onboarded".to_owned(),
                location: repository.entry_kind.clone(),
                discovered_at: "managed-project-onboard".to_owned(),
                confidence: 100,
            },
            state: RelationshipState::Active,
        })?;
    }
    if let (Some(identity), Some(repository)) = (spawned_from, created_repository) {
        let repository = super::normalize_github_repository(repository).ok_or_else(|| {
            ManagedProjectError::new("--spawned-from requires a valid --repo value")
        })?;
        store.record_edge(RelationshipEdge {
            product_key: policy.product_key.clone(),
            kind: RelationshipKind::SpawnedFrom,
            source: repository,
            target: identity.to_owned(),
            evidence: RelationshipEvidence {
                kind: "verified-repository-creation".to_owned(),
                location: identity.to_owned(),
                discovered_at: "managed-project-onboard".to_owned(),
                confidence: 100,
            },
            state: RelationshipState::Active,
        })?;
    }
    Ok(())
}

fn validate_explicit_seeds(
    policy: &autospec_core::managed_project::ManagedProjectPolicy,
    repositories: &[String],
) -> Result<(), ManagedProjectError> {
    for seed in policy.repository_seeds.iter().chain(repositories) {
        if super::normalize_github_repository(seed).is_none() {
            return Err(ManagedProjectError::new(format!(
                "invalid explicit GitHub repository seed: {seed}"
            )));
        }
    }
    Ok(())
}

#[derive(Default)]
struct ProjectOptions {
    repo_dir: Option<PathBuf>,
    repositories: Vec<String>,
    workspaces: Vec<PathBuf>,
    issue_url: Option<String>,
    spawned_from: Option<String>,
    dry_run: bool,
}

fn parse_options(args: &[String]) -> Result<ProjectOptions, ManagedProjectError> {
    let mut options = ProjectOptions::default();
    let mut index = 0;
    while index < args.len() {
        let argument = &args[index];
        match argument.as_str() {
            "--repo-dir" | "--repo" | "--workspace" | "--issue-url" | "--spawned-from" => {
                let value = args.get(index + 1).ok_or_else(|| {
                    ManagedProjectError::new(format!("{argument} requires a value"))
                })?;
                set_value(&mut options, argument, value)?;
                index += 2;
            }
            "--dry-run" if !options.dry_run => {
                options.dry_run = true;
                index += 1;
            }
            "--dry-run" => return Err(ManagedProjectError::new("duplicate --dry-run")),
            _ => {
                return Err(ManagedProjectError::new(format!(
                    "unknown autospec project option: {argument}"
                )))
            }
        }
    }
    Ok(options)
}

fn set_value(
    options: &mut ProjectOptions,
    argument: &str,
    value: &str,
) -> Result<(), ManagedProjectError> {
    match argument {
        "--repo-dir" if options.repo_dir.is_none() => options.repo_dir = Some(value.into()),
        "--repo-dir" => return Err(ManagedProjectError::new("duplicate --repo-dir")),
        "--repo" => options.repositories.push(value.to_owned()),
        "--workspace" => options.workspaces.push(value.into()),
        "--issue-url" if options.issue_url.is_none() => options.issue_url = Some(value.to_owned()),
        "--issue-url" => return Err(ManagedProjectError::new("duplicate --issue-url")),
        "--spawned-from" if options.spawned_from.is_none() && !value.trim().is_empty() => {
            options.spawned_from = Some(value.to_owned())
        }
        "--spawned-from" if value.trim().is_empty() => {
            return Err(ManagedProjectError::new("--spawned-from must not be empty"))
        }
        "--spawned-from" => return Err(ManagedProjectError::new("duplicate --spawned-from")),
        _ => unreachable!(),
    }
    Ok(())
}

fn validate_options(command: &str, options: &ProjectOptions) -> Result<(), ManagedProjectError> {
    let invalid = match command {
        "resolve" => {
            !options.repositories.is_empty()
                || !options.workspaces.is_empty()
                || options.issue_url.is_some()
                || options.spawned_from.is_some()
                || options.dry_run
        }
        "sync" => {
            !options.repositories.is_empty()
                || !options.workspaces.is_empty()
                || options.spawned_from.is_some()
                || options.dry_run
        }
        "onboard" => {
            options.issue_url.is_some()
                || (options.spawned_from.is_some() && options.repositories.len() != 1)
        }
        _ => false,
    };
    if invalid {
        Err(ManagedProjectError::new(format!(
            "invalid option for project {command}"
        )))
    } else {
        Ok(())
    }
}

fn load_managed_policy(
    repo_dir: &Path,
) -> Result<autospec_core::managed_project::ManagedProjectPolicy, ManagedProjectError> {
    let path = repo_dir.join(".autospec/autonomous.yml");
    let source = fs::read_to_string(&path).map_err(|error| {
        ManagedProjectError::new(format!("cannot read {}: {error}", path.display()))
    })?;
    let config = AutonomousConfig::parse(&source).map_err(ManagedProjectError::new)?;
    config
        .project_board
        .managed_policy()
        .cloned()
        .ok_or_else(|| ManagedProjectError::new("project_board.mode must be managed"))
}

fn report_json(report: &OnboardingReport, project_url: Option<&str>) -> Value {
    json!({
        "created": report.created,
        "adopted": report.adopted,
        "updated": report.updated,
        "unchanged": report.unchanged,
        "proposed": report.proposed,
        "out_of_bound": report.out_of_bound,
        "inaccessible": report.inaccessible,
        "pending_projection": report.pending_projection,
        "project_url": project_url,
        "repositories": report.repositories,
        "edges": report.edges,
    })
}
