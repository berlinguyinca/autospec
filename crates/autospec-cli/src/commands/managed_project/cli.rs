use super::super::autonomous::accountability::github::{GhCli, GithubCommand, GithubTransport};
use super::{
    journal_issue_projection, normalize_issue_url, onboard_repositories, resolve_or_create_project,
    retry_pending_projections, ManagedProjectError, ManagedProjectStore, OnboardingOptions,
    OnboardingReport,
};
use autospec_core::autonomous::config::AutonomousConfig;
use autospec_core::managed_project::{
    RelationshipEdge, RelationshipEvidence, RelationshipKind, RelationshipState,
};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

pub fn run(args: &[String]) -> Result<(), ManagedProjectError> {
    let outcome = run_with_transport(args, &mut GhCli)?;
    if !outcome.is_null() {
        println!("{outcome}");
    }
    Ok(())
}

pub(crate) fn run_with_transport<T: GithubTransport>(
    args: &[String],
    github: &mut T,
) -> Result<Value, ManagedProjectError> {
    if args.is_empty()
        || args
            .iter()
            .any(|argument| matches!(argument.as_str(), "--help" | "-h"))
    {
        println!("autospec project\n\nUSAGE:\n    autospec project resolve --repo-dir PATH\n    autospec project sync --repo-dir PATH [--issue-url URL]\n    autospec project onboard --repo-dir PATH [--repo OWNER/NAME]... [--workspace PATH] [--issue-url URL]... [--owner OWNER --allow PATTERN]... [--spawned-from IDENTITY] [--dry-run]\n    autospec project active-edges --repo-dir PATH");
        return Ok(Value::Null);
    }
    let command = &args[0];
    let mut options = parse_options(&args[1..])?;
    validate_options(command, &options)?;
    let repo_dir = options
        .repo_dir
        .clone()
        .ok_or_else(|| ManagedProjectError::new("autospec project requires --repo-dir"))?;
    let policy = load_managed_policy(&repo_dir)?;
    populate_owner_repositories(&policy, &mut options, github)?;
    add_issue_repositories(&mut options)?;
    validate_explicit_seeds(&policy, &options.repositories)?;
    validate_issue_boundaries(&policy, &options.issue_urls)?;
    let state_root = managed_state_root(&repo_dir)?;
    let legacy_root = repo_dir.join(".autospec/state");
    let read_only = options.dry_run || command == "active-edges";
    let mut store = if read_only {
        if state_root
            .join("projects")
            .join(policy.product_key.as_str())
            .exists()
        {
            ManagedProjectStore::open_read_only(&state_root, &policy.product_key)?
        } else {
            ManagedProjectStore::open_read_only(&legacy_root, &policy.product_key)?
        }
    } else {
        ManagedProjectStore::open_global(&state_root, Some(&legacy_root), &policy.product_key)?
    };

    match command.as_str() {
        "resolve" => resolve(&mut store, github, &policy),
        "sync" => sync(
            &mut store,
            github,
            &policy,
            options.issue_urls.first().map(String::as_str),
        ),
        "onboard" => onboard(store, github, policy, &repo_dir, options),
        "active-edges" => Ok(serde_json::to_value(super::active_dependency_graph(
            store.snapshot(),
        ))?),
        other => Err(ManagedProjectError::new(format!(
            "unknown autospec project subcommand: {other}"
        ))),
    }
}

fn resolve<T: GithubTransport>(
    store: &mut ManagedProjectStore,
    github: &mut T,
    policy: &autospec_core::managed_project::ManagedProjectPolicy,
) -> Result<Value, ManagedProjectError> {
    let project = resolve_or_create_project(store, github, policy, policy.product_key.as_str())?;
    Ok(json!({
        "outcome": "reconciled",
        "node_id": project.node_id,
        "number": project.number,
        "owner": project.owner,
        "title": project.title,
        "url": project.url,
    }))
}

fn sync<T: GithubTransport>(
    store: &mut ManagedProjectStore,
    github: &mut T,
    policy: &autospec_core::managed_project::ManagedProjectPolicy,
    issue_url: Option<&str>,
) -> Result<Value, ManagedProjectError> {
    if let Some(issue_url) = issue_url {
        journal_issue_projection(store, issue_url)?;
    }
    let project = resolve_or_create_project(store, github, policy, policy.product_key.as_str())?;
    if let Some(issue_url) = issue_url {
        super::reconcile_issue(store, github, policy, issue_url)?;
    }
    retry_pending_projections(store, github, policy)?;
    ack_repository_projections(store, policy)?;
    Ok(json!({
        "outcome": "reconciled",
        "pending_projection": store.snapshot().pending_projections.len(),
        "project_url": project.url,
    }))
}

fn onboard<T: GithubTransport>(
    mut store: ManagedProjectStore,
    github: &mut T,
    policy: autospec_core::managed_project::ManagedProjectPolicy,
    repo_dir: &Path,
    options: ProjectOptions,
) -> Result<Value, ManagedProjectError> {
    let created_repository = options.repositories.first().cloned();
    let selected_issues = options.issue_urls.clone();
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
        enqueue_repository_projections(&mut store, &policy, &report)?;
        for issue_url in &selected_issues {
            journal_issue_projection(&mut store, issue_url)?;
        }
    }
    if options.dry_run {
        return Ok(report_json(
            &report,
            store.snapshot().project_url.as_deref(),
            "dry_run",
            None,
            store.snapshot().pending_projections.len(),
            selected_issues.len(),
            0,
        ));
    }

    match resolve_or_create_project(&mut store, github, &policy, policy.product_key.as_str())
        .and_then(|project| {
            retry_pending_projections(&mut store, github, &policy)?;
            ack_repository_projections(&mut store, &policy)?;
            Ok(project)
        }) {
        Ok(project) => {
            let reconciled = reconciled_issue_count(&store, &selected_issues);
            Ok(report_json(
                &report,
                Some(&project.url),
                "reconciled",
                None,
                store.snapshot().pending_projections.len(),
                selected_issues.len(),
                reconciled,
            ))
        }
        Err(error) if error.is_journaled_projection_pending() => {
            let reconciled = reconciled_issue_count(&store, &selected_issues);
            Ok(report_json(
                &report,
                store.snapshot().project_url.as_deref(),
                "journaled_projection_pending",
                Some(&error.to_string()),
                store.snapshot().pending_projections.len(),
                selected_issues.len(),
                reconciled,
            ))
        }
        Err(error) => Err(error),
    }
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
        if !report
            .repositories
            .iter()
            .any(|record| record.repository == repository)
        {
            return Ok(());
        }
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

fn enqueue_repository_projections(
    store: &mut ManagedProjectStore,
    policy: &autospec_core::managed_project::ManagedProjectPolicy,
    report: &OnboardingReport,
) -> Result<(), ManagedProjectError> {
    for repository in &report.repositories {
        store.enqueue_projection(format!(
            "repository:register:{}:{}",
            policy.product_key, repository.repository
        ))?;
    }
    Ok(())
}

fn ack_repository_projections(
    store: &mut ManagedProjectStore,
    policy: &autospec_core::managed_project::ManagedProjectPolicy,
) -> Result<(), ManagedProjectError> {
    let prefix = format!("repository:register:{}:", policy.product_key);
    let pending = store
        .snapshot()
        .pending_projections
        .iter()
        .filter(|projection| projection.starts_with(&prefix))
        .cloned()
        .collect::<Vec<_>>();
    for projection in pending {
        store.ack_projection(&projection)?;
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
    issue_urls: Vec<String>,
    spawned_from: Option<String>,
    owner: Option<String>,
    allow: Vec<String>,
    dry_run: bool,
}

fn parse_options(args: &[String]) -> Result<ProjectOptions, ManagedProjectError> {
    let mut options = ProjectOptions::default();
    let mut index = 0;
    while index < args.len() {
        let argument = &args[index];
        match argument.as_str() {
            "--repo-dir" | "--repo" | "--workspace" | "--issue-url" | "--spawned-from"
            | "--owner" | "--allow" => {
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
        "--issue-url" => options.issue_urls.push(value.to_owned()),
        "--spawned-from" if options.spawned_from.is_none() && !value.trim().is_empty() => {
            options.spawned_from = Some(value.to_owned())
        }
        "--spawned-from" if value.trim().is_empty() => {
            return Err(ManagedProjectError::new("--spawned-from must not be empty"))
        }
        "--spawned-from" => return Err(ManagedProjectError::new("duplicate --spawned-from")),
        "--owner" if options.owner.is_none() && !value.trim().is_empty() => {
            options.owner = Some(value.to_owned())
        }
        "--owner" if value.trim().is_empty() => {
            return Err(ManagedProjectError::new("--owner must not be empty"))
        }
        "--owner" => return Err(ManagedProjectError::new("duplicate --owner")),
        "--allow" if !value.trim().is_empty() => options.allow.push(value.to_owned()),
        "--allow" => return Err(ManagedProjectError::new("--allow must not be empty")),
        _ => unreachable!(),
    }
    Ok(())
}

fn validate_options(command: &str, options: &ProjectOptions) -> Result<(), ManagedProjectError> {
    if command == "onboard" && options.owner.is_some() && options.allow.is_empty() {
        return Err(ManagedProjectError::new("--owner requires --allow"));
    }
    if command == "onboard" && options.owner.is_none() && !options.allow.is_empty() {
        return Err(ManagedProjectError::new("--allow requires --owner"));
    }
    if command == "onboard" && options.owner.is_some() && options.spawned_from.is_some() {
        return Err(ManagedProjectError::new(
            "--spawned-from cannot be combined with --owner",
        ));
    }
    let invalid = match command {
        "resolve" => {
            !options.repositories.is_empty()
                || !options.workspaces.is_empty()
                || !options.issue_urls.is_empty()
                || options.spawned_from.is_some()
                || options.owner.is_some()
                || !options.allow.is_empty()
                || options.dry_run
        }
        "sync" => {
            !options.repositories.is_empty()
                || !options.workspaces.is_empty()
                || options.spawned_from.is_some()
                || options.owner.is_some()
                || !options.allow.is_empty()
                || options.dry_run
                || options.issue_urls.len() > 1
        }
        "onboard" => options.spawned_from.is_some() && options.repositories.len() != 1,
        "active-edges" => {
            !options.repositories.is_empty()
                || !options.workspaces.is_empty()
                || !options.issue_urls.is_empty()
                || options.spawned_from.is_some()
                || options.owner.is_some()
                || !options.allow.is_empty()
                || options.dry_run
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

fn populate_owner_repositories<T: GithubTransport>(
    policy: &autospec_core::managed_project::ManagedProjectPolicy,
    options: &mut ProjectOptions,
    github: &mut T,
) -> Result<(), ManagedProjectError> {
    let Some(owner) = options.owner.as_deref() else {
        return Ok(());
    };
    if options.allow.is_empty() {
        return Err(ManagedProjectError::new("--owner requires --allow"));
    }
    if !owner.eq_ignore_ascii_case(&policy.owner) {
        return Err(ManagedProjectError::new(
            "--owner must match the managed project owner",
        ));
    }
    let allow = options
        .allow
        .iter()
        .map(|pattern| validate_owner_pattern(owner, pattern))
        .collect::<Result<Vec<_>, _>>()?;
    let output = github
        .execute(GithubCommand::ListOwnerRepositories {
            owner: owner.to_owned(),
            limit: policy.discovery_max_repos,
        })
        .map_err(|error| {
            ManagedProjectError::new(format!("cannot enumerate owner repositories: {error}"))
        })?;
    let repositories: Value = serde_json::from_str(&output).map_err(|error| {
        ManagedProjectError::new(format!("invalid owner repository response: {error}"))
    })?;
    let repositories = repositories.as_array().ok_or_else(|| {
        ManagedProjectError::new("invalid owner repository response: expected an array")
    })?;
    if repositories.len() > policy.discovery_max_repos {
        return Err(ManagedProjectError::new(format!(
            "owner repository response exceeds discovery_max_repos {}",
            policy.discovery_max_repos
        )));
    }
    for repository in repositories {
        let value = repository
            .get("nameWithOwner")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                ManagedProjectError::new("invalid owner repository response: missing nameWithOwner")
            })?;
        let repository = super::normalize_github_repository(value).ok_or_else(|| {
            ManagedProjectError::new("invalid owner repository response: malformed repository")
        })?;
        if allow
            .iter()
            .any(|pattern| matches_allow_pattern(&repository, pattern))
        {
            options.repositories.push(repository);
        }
    }
    options.repositories.sort();
    options.repositories.dedup();
    Ok(())
}

fn validate_owner_pattern(owner: &str, pattern: &str) -> Result<String, ManagedProjectError> {
    let normalized = pattern.trim().to_ascii_lowercase();
    let owner_prefix = format!("{}/", owner.trim().to_ascii_lowercase());
    if normalized == format!("{owner_prefix}*") {
        return Ok(normalized);
    }
    let base = normalized.strip_suffix('*').unwrap_or(&normalized);
    if base.contains('*')
        || !base.starts_with(&owner_prefix)
        || super::normalize_github_repository(base).is_none()
    {
        return Err(ManagedProjectError::new(format!(
            "invalid --allow repository pattern: {pattern}"
        )));
    }
    Ok(normalized)
}

fn matches_allow_pattern(repository: &str, pattern: &str) -> bool {
    pattern.strip_suffix('*').map_or_else(
        || repository == pattern,
        |prefix| repository.starts_with(prefix),
    )
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

fn report_json(
    report: &OnboardingReport,
    project_url: Option<&str>,
    outcome: &str,
    error: Option<&str>,
    pending_projection: usize,
    selected_issues: usize,
    reconciled_issues: usize,
) -> Value {
    json!({
        "outcome": outcome,
        "created": report.created,
        "adopted": report.adopted,
        "updated": report.updated,
        "unchanged": report.unchanged,
        "proposed": report.proposed,
        "out_of_bound": report.out_of_bound,
        "inaccessible": report.inaccessible,
        "pending_projection": pending_projection,
        "selected_issues": selected_issues,
        "reconciled_issues": reconciled_issues,
        "project_url": project_url,
        "error": error,
        "repositories": report.repositories,
        "edges": report.edges,
    })
}

fn add_issue_repositories(options: &mut ProjectOptions) -> Result<(), ManagedProjectError> {
    for issue_url in &mut options.issue_urls {
        *issue_url = normalize_issue_url(issue_url)?;
        let repository = super::normalize_github_repository(issue_url).ok_or_else(|| {
            ManagedProjectError::new("--issue-url must identify a GitHub issue repository")
        })?;
        options.repositories.push(repository);
    }
    options.repositories.sort();
    options.repositories.dedup();
    Ok(())
}

fn managed_state_root(_repo_dir: &Path) -> Result<PathBuf, ManagedProjectError> {
    #[cfg(test)]
    return Ok(_repo_dir.join(".autospec/state"));
    #[cfg(not(test))]
    if let Some(root) = std::env::var_os("AUTOSPEC_HOME") {
        return Ok(PathBuf::from(root));
    }
    #[cfg(not(test))]
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join(".autospec"))
        .ok_or_else(|| ManagedProjectError::new("HOME is required when AUTOSPEC_HOME is unset"))
}

fn validate_issue_boundaries(
    policy: &autospec_core::managed_project::ManagedProjectPolicy,
    issue_urls: &[String],
) -> Result<(), ManagedProjectError> {
    for issue_url in issue_urls {
        let repository = super::normalize_github_repository(issue_url).ok_or_else(|| {
            ManagedProjectError::new("--issue-url must identify a GitHub issue repository")
        })?;
        let owner_matches = repository
            .split_once('/')
            .is_some_and(|(owner, _)| owner.eq_ignore_ascii_case(&policy.owner));
        let allowed = policy.repo_allowlist.iter().any(|pattern| {
            let pattern = pattern.to_ascii_lowercase();
            pattern.strip_suffix('*').map_or_else(
                || repository == pattern,
                |prefix| repository.starts_with(prefix),
            )
        });
        if !owner_matches || !allowed {
            return Err(ManagedProjectError::new(format!(
                "selected issue is outside the managed repository boundary: {issue_url}"
            )));
        }
    }
    Ok(())
}

fn reconciled_issue_count(store: &ManagedProjectStore, issue_urls: &[String]) -> usize {
    issue_urls
        .iter()
        .filter(|issue_url| {
            !store
                .snapshot()
                .pending_projections
                .iter()
                .any(|projection| projection.ends_with(issue_url.as_str()))
        })
        .count()
}
