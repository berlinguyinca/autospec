use super::super::autonomous::accountability::github::{GhCli, GithubCommand, GithubTransport};
use super::portfolio::manifest::{PlanCompletionPolicy, PlanItemRole, PrimaryScopeSelector};
use super::{
    journal_issue_projection, normalize_issue_url, onboard_repositories, resolve_or_create_project,
    retry_pending_projections, tracked_issue_urls, ManagedProjectError, ManagedProjectStore,
    OnboardingOptions, OnboardingReport,
};
use autospec_core::autonomous::config::AutonomousConfig;
use autospec_core::managed_project::{
    ManagedProjectIdentity, PortfolioId, RelationshipEdge, RelationshipEvidence, RelationshipKind,
    RelationshipState, SourceSpecIdentity, SpecPortfolioIdentity,
};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;

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
        println!("autospec project\n\nUSAGE:\n    autospec project resolve --repo-dir PATH\n    autospec project sync --repo-dir PATH [--issue-url URL]\n    autospec project onboard --repo-dir PATH [--repo OWNER/NAME]... [--workspace PATH] [--issue-url URL]... [--owner OWNER --allow PATTERN]... [--spawned-from IDENTITY] [--dry-run]\n    autospec project active-edges --repo-dir PATH --board-url URL");
        return Ok(Value::Null);
    }
    let command = &args[0];
    let mut options = parse_options(&args[1..])?;
    validate_options(command, &options)?;
    let repo_dir = options
        .repo_dir
        .clone()
        .ok_or_else(|| ManagedProjectError::new("autospec project requires --repo-dir"))?;
    let board = load_project_board_config(&repo_dir)?;
    if command == "active-edges" && board.managed_policy().is_none() {
        return Ok(json!([]));
    }
    // An unconfigured board is an optional subsystem, not a failure: `resolve`
    // doubles as the cheap preflight probe and `sync` degrades to SKIPPED so a
    // fresh clone or worktree (where the gitignored operator config is absent)
    // never hard-fails a mandatory pipeline step.
    if (command == "resolve" || command == "sync") && board.managed_policy().is_none() {
        return Ok(json!({
            "outcome": "skipped",
            "reason": "project board not configured",
        }));
    }
    let policy = board
        .managed_policy()
        .cloned()
        .ok_or_else(|| ManagedProjectError::new("project_board.mode must be managed"))?;
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
            ManagedProjectStore::open_product_read_only(&state_root, &policy.product_key)?
        } else {
            ManagedProjectStore::open_product_read_only(&legacy_root, &policy.product_key)?
        }
    } else {
        ManagedProjectStore::open_product_global(
            &state_root,
            Some(&legacy_root),
            &policy.product_key,
        )?
    };
    if command == "onboard" && !read_only {
        for issue_url in &options.issue_urls {
            journal_issue_projection(&mut store, issue_url)?;
        }
    }
    populate_owner_repositories(&policy, &mut options, github)?;
    validate_explicit_seeds(&policy, &options.repositories)?;
    let explicitly_admitted = policy
        .repository_seeds
        .iter()
        .chain(options.repositories.iter())
        .filter_map(|repository| super::normalize_github_repository(repository))
        .collect::<std::collections::BTreeSet<_>>();
    let selected_issue_discovery = if command == "onboard" {
        load_selected_issue_relationships(
            &policy,
            &options.issue_urls,
            &explicitly_admitted,
            github,
        )?
    } else {
        OnboardingReport::default()
    };

    match command.as_str() {
        "resolve" => resolve(&mut store, github, &policy),
        "sync" => sync(
            &mut store,
            github,
            &policy,
            options.issue_urls.first().map(String::as_str),
        ),
        "onboard" => onboard(
            store,
            github,
            policy,
            &repo_dir,
            options,
            selected_issue_discovery,
        ),
        "active-edges" => active_edges(&store, options.board_url.as_deref()),
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
    let journaled = issue_url.is_some();
    if let Some(issue_url) = issue_url {
        journal_issue_projection(store, issue_url)?;
    }
    let result = (|| {
        let project =
            resolve_or_create_project(store, github, policy, policy.product_key.as_str())?;
        for tracked_issue in tracked_issue_urls(store) {
            let projection = journal_issue_projection(store, &tracked_issue)?;
            store.ensure_projection_pending(&projection)?;
        }
        retry_pending_projections(store, github, policy)?;
        ack_repository_projections(store, policy)?;
        Ok(json!({
            "outcome": "reconciled",
            "pending_projection": store.snapshot().pending_projections.len(),
            "project_url": project.url,
        }))
    })();
    result.map_err(|error: ManagedProjectError| {
        if journaled {
            ManagedProjectError::new(format!("journaled_projection_pending: {error}"))
        } else {
            error
        }
    })
}

fn onboard<T: GithubTransport>(
    mut store: ManagedProjectStore,
    github: &mut T,
    policy: autospec_core::managed_project::ManagedProjectPolicy,
    repo_dir: &Path,
    options: ProjectOptions,
    selected_issue_discovery: OnboardingReport,
) -> Result<Value, ManagedProjectError> {
    let created_repository = options.repositories.first().cloned();
    let selected_issues = options.issue_urls.clone();
    let mut report = onboard_repositories(
        &mut store,
        &policy,
        &OnboardingOptions {
            repo_dir: repo_dir.to_path_buf(),
            repositories: options.repositories,
            workspaces: options.workspaces,
            dry_run: options.dry_run,
        },
    )?;
    merge_selected_issue_discovery(
        &mut store,
        &mut report,
        selected_issue_discovery,
        options.dry_run,
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

fn load_selected_issue_relationships<T: GithubTransport>(
    policy: &autospec_core::managed_project::ManagedProjectPolicy,
    issue_urls: &[String],
    explicitly_admitted: &std::collections::BTreeSet<String>,
    github: &mut T,
) -> Result<OnboardingReport, ManagedProjectError> {
    let mut report = OnboardingReport::default();
    for issue_url in issue_urls {
        let remainder = issue_url
            .strip_prefix("https://github.com/")
            .ok_or_else(|| ManagedProjectError::new("selected issue URL is not canonical"))?;
        let parts = remainder.split('/').collect::<Vec<_>>();
        let number = parts
            .get(3)
            .and_then(|number| number.parse::<u64>().ok())
            .filter(|number| *number > 0)
            .ok_or_else(|| ManagedProjectError::new("selected issue URL has no positive number"))?;
        let repository = parts
            .get(0..2)
            .map(|parts| parts.join("/"))
            .ok_or_else(|| ManagedProjectError::new("selected issue URL has no repository"))?;
        let Ok(output) = github.execute(GithubCommand::ViewIssue { repository, number }) else {
            report.inaccessible += 1;
            continue;
        };
        let Ok(issue) = serde_json::from_str::<Value>(&output) else {
            report.inaccessible += 1;
            continue;
        };
        let Some(returned_url) = issue.get("url").and_then(Value::as_str) else {
            report.inaccessible += 1;
            continue;
        };
        if normalize_issue_url(returned_url)? != *issue_url {
            return Err(ManagedProjectError::new(
                "selected issue response does not match the requested issue",
            ));
        }
        let Some(body) = issue.get("body").and_then(Value::as_str) else {
            report.inaccessible += 1;
            continue;
        };
        let discovered = super::discover_remote_issue_relationships(policy, issue_url, body)?;
        report.out_of_bound += discovered.out_of_bound;
        report.inaccessible += discovered.inaccessible;
        report.repositories.extend(discovered.repositories);
        report.edges.extend(discovered.edges);
    }
    report
        .repositories
        .sort_by(|left, right| left.repository.cmp(&right.repository));
    report
        .repositories
        .dedup_by(|left, right| left.repository == right.repository);
    report
        .repositories
        .retain(|record| !explicitly_admitted.contains(&record.repository));
    if report.repositories.len() > policy.discovery_max_repos {
        let excluded = report.repositories.split_off(policy.discovery_max_repos);
        report.out_of_bound += excluded.len();
    }
    let mut admitted = report
        .repositories
        .iter()
        .map(|record| record.repository.clone())
        .collect::<std::collections::BTreeSet<_>>();
    admitted.extend(issue_urls.iter().filter_map(|issue_url| {
        issue_url
            .strip_prefix("https://github.com/")
            .and_then(|remainder| remainder.split_once("/issues/"))
            .map(|(repository, _)| repository.to_owned())
    }));
    report.edges.retain(|edge| {
        [&edge.source, &edge.target].into_iter().all(|identity| {
            super::onboard::field_repository(identity)
                .and_then(super::normalize_github_repository)
                .is_none_or(|repository| admitted.contains(&repository))
        })
    });
    report.edges.sort_by_key(RelationshipEdge::dedupe_key);
    report.edges.dedup_by_key(|edge| edge.dedupe_key());
    Ok(report)
}

fn merge_selected_issue_discovery(
    store: &mut ManagedProjectStore,
    report: &mut OnboardingReport,
    discovered: OnboardingReport,
    dry_run: bool,
) -> Result<(), ManagedProjectError> {
    report.out_of_bound += discovered.out_of_bound;
    report.inaccessible += discovered.inaccessible;
    let mut known_repositories = report
        .repositories
        .iter()
        .map(|record| record.repository.clone())
        .collect::<std::collections::BTreeSet<_>>();
    for repository in discovered.repositories {
        if !known_repositories.insert(repository.repository.clone()) {
            continue;
        }
        report.created += 1;
        if !dry_run {
            store.record_repository(repository.clone())?;
        }
        report.repositories.push(repository);
    }
    report
        .repositories
        .sort_by(|left, right| left.repository.cmp(&right.repository));
    let mut known = report
        .edges
        .iter()
        .map(RelationshipEdge::dedupe_key)
        .collect::<std::collections::BTreeSet<_>>();
    for edge in discovered.edges {
        if !known.insert(edge.dedupe_key()) {
            continue;
        }
        report.updated += 1;
        if edge.state == RelationshipState::Proposed {
            report.proposed += 1;
        }
        if !dry_run {
            store.record_edge(edge.clone())?;
        }
        report.edges.push(edge);
    }
    report.edges.sort_by_key(RelationshipEdge::dedupe_key);
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
    board_url: Option<String>,
    dry_run: bool,
}

fn parse_options(args: &[String]) -> Result<ProjectOptions, ManagedProjectError> {
    let mut options = ProjectOptions::default();
    let mut index = 0;
    while index < args.len() {
        let argument = &args[index];
        match argument.as_str() {
            "--repo-dir" | "--repo" | "--workspace" | "--issue-url" | "--spawned-from"
            | "--owner" | "--allow" | "--board-url" => {
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
        "--board-url" if options.board_url.is_none() && !value.trim().is_empty() => {
            options.board_url = Some(value.to_owned())
        }
        "--board-url" if value.trim().is_empty() => {
            return Err(ManagedProjectError::new("--board-url must not be empty"))
        }
        "--board-url" => return Err(ManagedProjectError::new("duplicate --board-url")),
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
                || options.board_url.is_some()
                || options.dry_run
        }
        "sync" => {
            !options.repositories.is_empty()
                || !options.workspaces.is_empty()
                || options.spawned_from.is_some()
                || options.owner.is_some()
                || !options.allow.is_empty()
                || options.board_url.is_some()
                || options.dry_run
                || options.issue_urls.len() > 1
        }
        "onboard" => {
            options.board_url.is_some()
                || (options.spawned_from.is_some() && options.repositories.len() != 1)
        }
        "active-edges" => {
            !options.repositories.is_empty()
                || !options.workspaces.is_empty()
                || !options.issue_urls.is_empty()
                || options.spawned_from.is_some()
                || options.owner.is_some()
                || !options.allow.is_empty()
                || options.dry_run
                || options.board_url.is_none()
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

fn load_project_board_config(
    repo_dir: &Path,
) -> Result<autospec_core::autonomous::config::ProjectBoardConfig, ManagedProjectError> {
    let path = repo_dir.join(".autospec/autonomous.yml");
    // `.autospec/autonomous.yml` is operator-local and gitignored, so it never
    // reaches a fresh clone, worktree, or CI runner. Absence means "board not
    // configured"; only a genuine read error is a failure.
    let source = match fs::read_to_string(&path) {
        Ok(source) => source,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(AutonomousConfig::parse("")
                .map_err(ManagedProjectError::new)?
                .project_board);
        }
        Err(error) => {
            return Err(ManagedProjectError::new(format!(
                "cannot read {}: {error}",
                path.display()
            )));
        }
    };
    Ok(AutonomousConfig::parse(&source)
        .map_err(ManagedProjectError::new)?
        .project_board)
}

fn active_edges(
    store: &ManagedProjectStore,
    board_url: Option<&str>,
) -> Result<Value, ManagedProjectError> {
    let requested =
        board_url.ok_or_else(|| ManagedProjectError::new("active-edges requires --board-url"))?;
    let bound = store
        .snapshot()
        .project_url
        .as_deref()
        .ok_or_else(|| ManagedProjectError::new("managed project has no bound board"))?;
    if normalize_board_url(requested) != normalize_board_url(bound) {
        return Err(ManagedProjectError::new(
            "requested board does not match the managed project binding",
        ));
    }
    Ok(serde_json::to_value(super::active_dependency_graph(
        store.snapshot(),
    ))?)
}

fn normalize_board_url(value: &str) -> String {
    value.trim().trim_end_matches('/').to_ascii_lowercase()
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

pub(crate) fn managed_state_root(_repo_dir: &Path) -> Result<PathBuf, ManagedProjectError> {
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

pub(crate) fn bound_project_url(
    repo_dir: &Path,
    policy: &autospec_core::managed_project::ManagedProjectPolicy,
) -> Result<Option<String>, ManagedProjectError> {
    let state_root = managed_state_root(repo_dir)?;
    let legacy_root = repo_dir.join(".autospec/state");
    let store = if state_root
        .join("projects")
        .join(policy.product_key.as_str())
        .exists()
    {
        ManagedProjectStore::open_product_read_only(&state_root, &policy.product_key)?
    } else {
        ManagedProjectStore::open_product_read_only(&legacy_root, &policy.product_key)?
    };
    Ok(store.snapshot().project_url.clone())
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

// ── `autospec portfolio validate|apply|reconcile` surface ──────────────────────────
//
// The only public transaction over a frozen `autospec.portfolio-plan.v1` manifest.
// This boundary parses the manifest, the explicit `--portfolio` identity,
// `--project-owner`, and `--dry-run`, and reports one of three stable result states:
// `complete`, `blocked`, or `degraded`. Human-readable stdout prints the bound Project
// URL before any issue URL, and the JSON result carries the same identities. An
// explicit owner is carried through unchanged and never falls back to another owner.

const PORTFOLIO_USAGE: &str = "autospec portfolio\n\nUSAGE:\n    autospec portfolio validate --manifest PATH [--dry-run] [--state-dir PATH] [--portfolio ID] [--project-owner OWNER]\n    autospec portfolio apply --manifest PATH --portfolio ID --state-dir PATH [--dry-run] [--project-owner OWNER]\n    autospec portfolio reconcile --manifest PATH --portfolio ID --state-dir PATH [--project-owner OWNER]\n\nRESULT STATES:\n    complete   every required initial projection is acknowledged\n    blocked    a preflight or identity check failed before mutation\n    degraded   a previously complete portfolio has retryable pending work";

pub fn run_portfolio(args: &[String]) -> Result<(), super::super::CommandFailure> {
    if args.is_empty()
        || args
            .iter()
            .any(|argument| matches!(argument.as_str(), "--help" | "-h"))
    {
        println!("{PORTFOLIO_USAGE}");
        return Ok(());
    }
    let command = args[0].clone();
    let failure =
        |error: ManagedProjectError| super::super::CommandFailure::diagnostic(error.to_string());
    let options = parse_portfolio_options(&args[1..]).map_err(&failure)?;
    validate_portfolio_options(&command, &options).map_err(&failure)?;
    let outcome = execute_portfolio_command(&command, &options).map_err(&failure)?;
    render_portfolio_outcome(&outcome);
    if outcome.exit_code == 0 {
        Ok(())
    } else {
        let message = outcome.json["diagnostics"]
            .as_array()
            .map(|diagnostics| {
                diagnostics
                    .iter()
                    .filter_map(|diagnostic| diagnostic["detail"].as_str())
                    .filter(|detail| !detail.is_empty())
                    .collect::<Vec<_>>()
                    .join("; ")
            })
            .unwrap_or_default();
        Err(super::super::CommandFailure::status(
            message,
            outcome.exit_code,
        ))
    }
}

#[derive(Default)]
struct PortfolioOptions {
    manifest: Option<PathBuf>,
    portfolio: Option<String>,
    state_dir: Option<PathBuf>,
    project_owner: Option<String>,
    dry_run: bool,
}

fn parse_portfolio_options(args: &[String]) -> Result<PortfolioOptions, ManagedProjectError> {
    let mut options = PortfolioOptions::default();
    let mut index = 0;
    while index < args.len() {
        let argument = &args[index];
        match argument.as_str() {
            "--manifest" | "--portfolio" | "--state-dir" | "--project-owner" => {
                let value = args.get(index + 1).ok_or_else(|| {
                    ManagedProjectError::new(format!("{argument} requires a value"))
                })?;
                set_portfolio_value(&mut options, argument, value)?;
                index += 2;
            }
            "--dry-run" if !options.dry_run => {
                options.dry_run = true;
                index += 1;
            }
            "--dry-run" => return Err(ManagedProjectError::new("duplicate --dry-run")),
            _ => {
                return Err(ManagedProjectError::new(format!(
                    "unknown autospec portfolio option: {argument}"
                )))
            }
        }
    }
    Ok(options)
}

fn set_portfolio_value(
    options: &mut PortfolioOptions,
    argument: &str,
    value: &str,
) -> Result<(), ManagedProjectError> {
    match argument {
        "--manifest" if options.manifest.is_none() && !value.trim().is_empty() => {
            options.manifest = Some(value.into())
        }
        "--manifest" if value.trim().is_empty() => {
            return Err(ManagedProjectError::new("--manifest must not be empty"))
        }
        "--manifest" => return Err(ManagedProjectError::new("duplicate --manifest")),
        "--portfolio" if options.portfolio.is_none() && !value.trim().is_empty() => {
            options.portfolio = Some(value.to_owned())
        }
        "--portfolio" if value.trim().is_empty() => {
            return Err(ManagedProjectError::new("--portfolio must not be empty"))
        }
        "--portfolio" => return Err(ManagedProjectError::new("duplicate --portfolio")),
        "--state-dir" if options.state_dir.is_none() && !value.trim().is_empty() => {
            options.state_dir = Some(value.into())
        }
        "--state-dir" if value.trim().is_empty() => {
            return Err(ManagedProjectError::new("--state-dir must not be empty"))
        }
        "--state-dir" => return Err(ManagedProjectError::new("duplicate --state-dir")),
        "--project-owner" if options.project_owner.is_none() && !value.trim().is_empty() => {
            options.project_owner = Some(value.to_owned())
        }
        "--project-owner" if value.trim().is_empty() => {
            return Err(ManagedProjectError::new(
                "--project-owner must not be empty",
            ))
        }
        "--project-owner" => return Err(ManagedProjectError::new("duplicate --project-owner")),
        _ => unreachable!(),
    }
    Ok(())
}

fn validate_portfolio_options(
    command: &str,
    options: &PortfolioOptions,
) -> Result<(), ManagedProjectError> {
    match command {
        "validate" => {
            if options.manifest.is_none() {
                return Err(ManagedProjectError::new(
                    "portfolio validate requires --manifest",
                ));
            }
            if options.state_dir.is_some() && !options.dry_run {
                return Err(ManagedProjectError::new(
                    "--state-dir requires --dry-run for portfolio validate",
                ));
            }
        }
        "apply" | "reconcile" => {
            if options.manifest.is_none() {
                return Err(ManagedProjectError::new(format!(
                    "portfolio {command} requires --manifest"
                )));
            }
            if options.portfolio.is_none() {
                return Err(ManagedProjectError::new(format!(
                    "portfolio {command} requires --portfolio"
                )));
            }
            if options.state_dir.is_none() {
                return Err(ManagedProjectError::new(format!(
                    "portfolio {command} requires --state-dir"
                )));
            }
            if command == "reconcile" && options.dry_run {
                return Err(ManagedProjectError::new(
                    "--dry-run is not valid for portfolio reconcile",
                ));
            }
        }
        other => {
            return Err(ManagedProjectError::new(format!(
                "unknown autospec portfolio subcommand: {other}"
            )))
        }
    }
    Ok(())
}

struct PortfolioOutcome {
    exit_code: i32,
    project_url: Option<String>,
    issue_urls: Vec<String>,
    json: Value,
}

fn render_portfolio_outcome(outcome: &PortfolioOutcome) {
    if let Some(url) = &outcome.project_url {
        println!("{url}");
    }
    for url in &outcome.issue_urls {
        println!("{url}");
    }
    println!(
        "{}",
        serde_json::to_string(&outcome.json).expect("portfolio outcome serializes to JSON")
    );
}

fn diagnostic_value(code: &str, detail: &str, exit: i32) -> Value {
    json!({"code": code, "detail": detail, "exit": exit})
}

fn plan_diagnostic(violation: &super::portfolio::manifest::PlanViolation) -> Value {
    diagnostic_value(
        violation.code().as_str(),
        violation.detail(),
        violation.exit_code(),
    )
}

fn scope_diagnostic(violation: &super::portfolio::ScopeViolation) -> Value {
    diagnostic_value(
        violation.code().as_str(),
        violation.detail(),
        violation.exit_code(),
    )
}

#[allow(clippy::too_many_arguments)] // every field is a stable JSON output slot
fn portfolio_outcome(
    command: &str,
    options: &PortfolioOptions,
    plan: Option<&super::portfolio::manifest::PortfolioPlan>,
    result: &str,
    primary_scope: Option<String>,
    project: Option<Value>,
    issue_urls: Vec<String>,
    pending_operations: usize,
    pending_projections: usize,
    dry_run_mutations: Option<u64>,
    diagnostics: Vec<Value>,
) -> PortfolioOutcome {
    let (portfolio_id, project_owner, plan_digest, item_count, repository_count, capabilities) =
        plan.map_or_else(
            || {
                (
                    Value::Null,
                    Value::Null,
                    Value::Null,
                    Value::Null,
                    Value::Null,
                    Value::Object(Default::default()),
                )
            },
            |plan| {
                let mut capabilities = serde_json::Map::new();
                for facts in plan.repositories() {
                    capabilities.insert(
                        facts.repository().to_owned(),
                        Value::from(facts.capability().as_str()),
                    );
                }
                (
                    Value::from(plan.portfolio_id().as_str()),
                    Value::from(plan.project_owner()),
                    Value::from(plan.plan_digest()),
                    Value::from(plan.items().len()),
                    Value::from(plan.repositories().len()),
                    Value::Object(capabilities),
                )
            },
        );
    let exit_code = match result {
        "complete" | "degraded" => 0,
        _ => diagnostics
            .iter()
            .filter_map(|diagnostic| diagnostic.get("exit").and_then(Value::as_u64))
            .map(|exit| i32::try_from(exit).unwrap_or(1))
            .max()
            .unwrap_or(1),
    };
    let project_url = project
        .as_ref()
        .and_then(|project| project.get("url").and_then(Value::as_str))
        .map(str::to_owned);
    let json = json!({
        "command": command,
        "result": result,
        "portfolio_id": portfolio_id,
        "project_owner": project_owner,
        "plan_digest": plan_digest,
        "project": project,
        "issue_urls": issue_urls,
        "primary_scope": primary_scope,
        "item_count": item_count,
        "repository_count": repository_count,
        "capabilities": capabilities,
        "pending_operations": pending_operations,
        "pending_projections": pending_projections,
        "dry_run": options.dry_run,
        "dry_run_mutations": dry_run_mutations,
        "diagnostics": diagnostics,
    });
    PortfolioOutcome {
        exit_code,
        project_url,
        issue_urls,
        json,
    }
}

fn blocked_portfolio_outcome(
    command: &str,
    options: &PortfolioOptions,
    plan: Option<&super::portfolio::manifest::PortfolioPlan>,
    primary_scope: Option<String>,
    diagnostics: Vec<Value>,
) -> PortfolioOutcome {
    portfolio_outcome(
        command,
        options,
        plan,
        "blocked",
        primary_scope,
        None,
        Vec::new(),
        0,
        0,
        None,
        diagnostics,
    )
}

fn execute_portfolio_command(
    command: &str,
    options: &PortfolioOptions,
) -> Result<PortfolioOutcome, ManagedProjectError> {
    let manifest_path = options
        .manifest
        .clone()
        .expect("portfolio subcommand validation requires --manifest");
    let contents = fs::read_to_string(&manifest_path).map_err(|error| {
        ManagedProjectError::new(format!(
            "cannot read portfolio manifest {}: {error}",
            manifest_path.display()
        ))
    })?;
    let (draft, file_digest) = match parse_plan_manifest(&contents) {
        Ok(parsed) => parsed,
        Err(ManifestError::Hard(error)) => return Err(error),
        Err(ManifestError::Plan(violation)) => {
            return Ok(blocked_portfolio_outcome(
                command,
                options,
                None,
                None,
                vec![plan_diagnostic(&violation)],
            ))
        }
    };
    let draft = match draft.into_draft() {
        Ok(draft) => draft,
        Err(ManifestError::Hard(error)) => return Err(error),
        Err(ManifestError::Plan(violation)) => {
            return Ok(blocked_portfolio_outcome(
                command,
                options,
                None,
                None,
                vec![plan_diagnostic(&violation)],
            ))
        }
    };
    let plan = match super::portfolio::manifest::PortfolioPlan::from_parts(draft) {
        Ok(plan) => plan,
        Err(violation) => {
            return Ok(blocked_portfolio_outcome(
                command,
                options,
                None,
                None,
                vec![plan_diagnostic(&violation)],
            ))
        }
    };
    if let Some(file_digest) = file_digest {
        if file_digest != plan.plan_digest() {
            let violation = super::portfolio::manifest::PlanViolation::new(
                super::portfolio::manifest::PlanViolationCode::DigestMismatch,
                format!(
                    "manifest plan digest {file_digest} does not match the frozen plan digest {}",
                    plan.plan_digest()
                ),
            );
            return Ok(blocked_portfolio_outcome(
                command,
                options,
                Some(&plan),
                None,
                vec![plan_diagnostic(&violation)],
            ));
        }
    }
    if let Err(violation) = plan.validate() {
        return Ok(blocked_portfolio_outcome(
            command,
            options,
            Some(&plan),
            None,
            vec![plan_diagnostic(&violation)],
        ));
    }
    let scope = match super::portfolio::select_primary_scope(&plan) {
        Ok(scope) => scope,
        Err(violation) => {
            return Ok(blocked_portfolio_outcome(
                command,
                options,
                Some(&plan),
                None,
                vec![scope_diagnostic(&violation)],
            ))
        }
    };
    match command {
        "validate" => validate_portfolio_command(&plan, &scope, options),
        "apply" | "reconcile" => transaction_portfolio_command(command, &plan, &scope, options),
        other => Err(ManagedProjectError::new(format!(
            "unknown autospec portfolio subcommand: {other}"
        ))),
    }
}

fn validate_portfolio_command(
    plan: &super::portfolio::manifest::PortfolioPlan,
    scope: &super::portfolio::PrimaryScope,
    options: &PortfolioOptions,
) -> Result<PortfolioOutcome, ManagedProjectError> {
    if !options.dry_run {
        return Ok(portfolio_outcome(
            "validate",
            options,
            Some(plan),
            "complete",
            Some(scope.as_str()),
            None,
            Vec::new(),
            0,
            0,
            None,
            Vec::new(),
        ));
    }
    let target = match options.state_dir.as_ref() {
        None => super::portfolio::DryRunTarget::InMemory,
        Some(path) if path.is_dir() => super::portfolio::DryRunTarget::Journal(path.clone()),
        Some(path) => {
            return Ok(blocked_portfolio_outcome(
                "validate",
                options,
                Some(plan),
                Some(scope.as_str()),
                vec![diagnostic_value(
                    "DRY_RUN_JOURNAL_MISSING",
                    &format!("journal path `{}` does not exist", path.display()),
                    1,
                )],
            ))
        }
    };
    match super::portfolio::validate_plan_dry_run(plan, target) {
        Ok(report) => Ok(portfolio_outcome(
            "validate",
            options,
            Some(plan),
            "complete",
            Some(scope.as_str()),
            None,
            Vec::new(),
            0,
            0,
            Some(report.mutations().total()),
            Vec::new(),
        )),
        Err(super::portfolio::DryRunError::Plan(violation)) => Ok(blocked_portfolio_outcome(
            "validate",
            options,
            Some(plan),
            Some(scope.as_str()),
            vec![plan_diagnostic(&violation)],
        )),
        Err(super::portfolio::DryRunError::Scope(violation)) => Ok(blocked_portfolio_outcome(
            "validate",
            options,
            Some(plan),
            Some(scope.as_str()),
            vec![scope_diagnostic(&violation)],
        )),
        Err(super::portfolio::DryRunError::JournalMissing(path)) => Ok(blocked_portfolio_outcome(
            "validate",
            options,
            Some(plan),
            Some(scope.as_str()),
            vec![diagnostic_value(
                "DRY_RUN_JOURNAL_MISSING",
                &format!("journal path `{}` does not exist", path.display()),
                1,
            )],
        )),
        Err(super::portfolio::DryRunError::Witness(detail)) => Ok(blocked_portfolio_outcome(
            "validate",
            options,
            Some(plan),
            Some(scope.as_str()),
            vec![diagnostic_value("DRY_RUN_WITNESS", &detail, 1)],
        )),
    }
}

fn transaction_portfolio_command(
    command: &str,
    plan: &super::portfolio::manifest::PortfolioPlan,
    scope: &super::portfolio::PrimaryScope,
    options: &PortfolioOptions,
) -> Result<PortfolioOutcome, ManagedProjectError> {
    let requested_portfolio = match PortfolioId::new(options.portfolio.clone().expect("validated"))
    {
        Ok(id) => id,
        Err(detail) => {
            return Ok(blocked_portfolio_outcome(
                command,
                options,
                Some(plan),
                Some(scope.as_str()),
                vec![diagnostic_value("PORTFOLIO_ID_INVALID", &detail, 1)],
            ))
        }
    };
    if requested_portfolio != *plan.portfolio_id() {
        return Ok(blocked_portfolio_outcome(
            command,
            options,
            Some(plan),
            Some(scope.as_str()),
            vec![diagnostic_value(
                "PORTFOLIO_ID_MISMATCH",
                &format!(
                    "--portfolio {requested_portfolio} does not match manifest portfolio id {}",
                    plan.portfolio_id()
                ),
                1,
            )],
        ));
    }
    if let Some(explicit) = options.project_owner.as_deref() {
        let canonical = explicit.trim().to_ascii_lowercase();
        if canonical != plan.project_owner() {
            return Ok(blocked_portfolio_outcome(
                command,
                options,
                Some(plan),
                Some(scope.as_str()),
                vec![diagnostic_value(
                    "PROJECT_OWNER_MISMATCH",
                    &format!(
                        "explicit --project-owner `{explicit}` does not match frozen plan owner `{}`; an explicit owner never falls back",
                        plan.project_owner()
                    ),
                    1,
                )],
            ));
        }
    }
    let identity = ManagedProjectIdentity::SpecPortfolio(SpecPortfolioIdentity::new(
        plan.source_spec().clone(),
    ));
    let state_dir = options
        .state_dir
        .clone()
        .expect("portfolio transaction validation requires --state-dir");
    let store = if options.dry_run {
        ManagedProjectStore::open_read_only(&state_dir, &identity)
    } else {
        ManagedProjectStore::open(&state_dir, &identity)
    };
    let store = match store {
        Ok(store) => store,
        Err(error) => {
            return Ok(blocked_portfolio_outcome(
                command,
                options,
                Some(plan),
                Some(scope.as_str()),
                vec![diagnostic_value("STATE_UNREADABLE", &error.to_string(), 1)],
            ))
        }
    };
    let snapshot = match store.portfolio_snapshot() {
        Some(snapshot) => snapshot.clone(),
        None => {
            return Ok(blocked_portfolio_outcome(
                command,
                options,
                Some(plan),
                Some(scope.as_str()),
                vec![diagnostic_value(
                    "NO_PROJECT_BINDING",
                    "no verified primary Project precedes issue admission; the portfolio is not provisioned",
                    1,
                )],
            ))
        }
    };
    if snapshot["owner"].as_str() != Some(plan.project_owner()) {
        return Ok(blocked_portfolio_outcome(
            command,
            options,
            Some(plan),
            Some(scope.as_str()),
            vec![diagnostic_value(
                "PROJECT_OWNER_MISMATCH",
                &format!(
                    "provisioned portfolio owner `{}` does not match frozen plan owner `{}`; no fallback",
                    snapshot["owner"],
                    plan.project_owner()
                ),
                1,
            )],
        ));
    }
    if snapshot["plan_digest"].as_str() != Some(plan.plan_digest()) {
        return Ok(blocked_portfolio_outcome(
            command,
            options,
            Some(plan),
            Some(scope.as_str()),
            vec![diagnostic_value(
                "PLAN_DIGEST_MISMATCH",
                &format!(
                    "provisioned plan digest {} differs from manifest plan digest {}; a changed plan digest stops for explicit plan revision reconciliation",
                    snapshot["plan_digest"],
                    plan.plan_digest()
                ),
                1,
            )],
        ));
    }
    let issue_urls = store
        .portfolio_item_bindings()
        .iter()
        .filter_map(|binding| binding.get("issue_url").and_then(Value::as_str))
        .filter(|url| url.starts_with("https://github.com/"))
        .map(str::to_owned)
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    let project = json!({
        "owner": snapshot["owner"],
        "number": snapshot["project_number"],
        "url": snapshot["project_url"],
    });
    let pending_operations = store
        .portfolio_operation_states()
        .into_iter()
        .filter(|(_, state)| state != "acknowledged")
        .count();
    let pending_projections = store.snapshot().pending_projections.len();
    let result = if pending_operations == 0 && pending_projections == 0 {
        "complete"
    } else {
        "degraded"
    };
    Ok(portfolio_outcome(
        command,
        options,
        Some(plan),
        result,
        Some(scope.as_str()),
        Some(project),
        issue_urls,
        pending_operations,
        pending_projections,
        None,
        Vec::new(),
    ))
}

// ── Frozen plan manifest parsing ──────────────────────────────────────────────
//
// The manifest is the canonical `autospec.portfolio-plan.v1` YAML document
// (`PortfolioPlan::canonical_yaml`). It is parsed back into a `PlanDraft` so
// `PortfolioPlan::from_parts` recomputes the digest and any edit to a frozen
// field surfaces as `DIGEST_MISMATCH` instead of being silently accepted.

enum ManifestError {
    Hard(ManagedProjectError),
    Plan(super::portfolio::manifest::PlanViolation),
}

impl ManifestError {
    fn hard(message: impl Into<String>) -> Self {
        Self::Hard(ManagedProjectError::new(message.into()))
    }
}

/// The inverse of `yaml::yaml_scalar`: accepts only the double-quoted form the renderer
/// emits (backslash, quote, newline, tab, and carriage return are escaped; everything
/// else is literal) and rejects anything else, so a hand-edited manifest is either
/// rejected here or, if it re-parses, caught by the digest check.
fn parse_quoted_scalar(token: &str) -> Option<String> {
    let bytes = token.as_bytes();
    if bytes.len() < 2 || bytes[0] != b'"' || bytes[bytes.len() - 1] != b'"' {
        return None;
    }
    let mut out = String::new();
    let mut rest = &bytes[1..bytes.len() - 1];
    while !rest.is_empty() {
        match rest[0] {
            b'\\' => {
                if rest.len() < 2 {
                    return None;
                }
                let unescaped = match rest[1] {
                    b'"' => '"',
                    b'\\' => '\\',
                    b'n' => '\n',
                    b't' => '\t',
                    b'r' => '\r',
                    _ => return None,
                };
                out.push(unescaped);
                rest = &rest[2..];
            }
            b'"' => return None,
            _ => {
                let text = std::str::from_utf8(rest).ok()?;
                let character = text.chars().next()?;
                out.push(character);
                rest = &rest[character.len_utf8()..];
            }
        }
    }
    Some(out)
}

fn manifest_scalar_value(token: &str) -> Result<String, ManifestError> {
    parse_quoted_scalar(token).ok_or_else(|| {
        ManifestError::hard(format!("unquoted scalar in portfolio manifest: {token}"))
    })
}

#[derive(Default)]
struct RepositoryEntry {
    id: Option<String>,
    capability: Option<String>,
    observed_revision: Option<Option<String>>,
}

impl RepositoryEntry {
    fn into_facts(self) -> Result<super::portfolio::manifest::RepositoryFacts, ManifestError> {
        let id = self
            .id
            .ok_or_else(|| ManifestError::hard("repository entry is missing `id`"))?;
        let capability = self
            .capability
            .ok_or_else(|| ManifestError::hard("repository entry is missing `capability`"))?;
        let revision = self.observed_revision.unwrap_or(None);
        Ok(match capability.as_str() {
            "available" => match revision {
                Some(revision) => {
                    super::portfolio::manifest::RepositoryFacts::available(id, revision)
                }
                None => super::portfolio::manifest::RepositoryFacts::reachable(id),
            },
            "unavailable" => {
                if revision.is_some() {
                    return Err(ManifestError::hard(
                        "unavailable repository must not carry `observed_revision`",
                    ));
                }
                super::portfolio::manifest::RepositoryFacts::unavailable(id)
            }
            "unknown" => {
                if revision.is_some() {
                    return Err(ManifestError::hard(
                        "unprobed repository must not carry `observed_revision`",
                    ));
                }
                super::portfolio::manifest::RepositoryFacts::unprobed(id)
            }
            other => {
                return Err(ManifestError::hard(format!(
                    "repository capability `{other}` must be available, unavailable, or unknown"
                )))
            }
        })
    }
}

#[derive(Default)]
struct ItemEntry {
    key: Option<String>,
    role: Option<String>,
    repository: Option<String>,
    completion: Option<String>,
    depends_on: Vec<String>,
    local_parents: Vec<String>,
    depends_on_seen: bool,
    local_parents_seen: bool,
}

impl ItemEntry {
    fn into_item(self) -> Result<super::portfolio::manifest::PlanItem, ManifestError> {
        let item_key = self
            .key
            .ok_or_else(|| ManifestError::hard("item entry is missing `key`"))?;
        let role = self
            .role
            .ok_or_else(|| ManifestError::hard("item entry is missing `role`"))?;
        let repository = self
            .repository
            .ok_or_else(|| ManifestError::hard("item entry is missing `repository`"))?;
        let completion = self
            .completion
            .ok_or_else(|| ManifestError::hard("item entry is missing `completion_policy`"))?;
        let role = match role.as_str() {
            "source-tracker" => PlanItemRole::SourceTracker,
            "repo-tracker" => PlanItemRole::RepoTracker,
            "prerequisite" => PlanItemRole::Prerequisite,
            "implementation" => PlanItemRole::Implementation,
            "audit" => PlanItemRole::Audit,
            other => {
                return Err(ManifestError::hard(format!(
                    "item role `{other}` must be source-tracker, repo-tracker, prerequisite, implementation, or audit"
                )))
            }
        };
        let completion = match completion.as_str() {
            "self" => PlanCompletionPolicy::SelfClosing,
            "cascade" => PlanCompletionPolicy::Cascade,
            "portfolio-gate" => PlanCompletionPolicy::PortfolioGate,
            other => {
                return Err(ManifestError::hard(format!(
                    "completion_policy `{other}` must be self, cascade, or portfolio-gate"
                )))
            }
        };
        let depends_on = self
            .depends_on
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        let local_parents = self
            .local_parents
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        super::portfolio::manifest::PlanItem::new(
            &item_key,
            repository,
            role,
            completion,
            &depends_on,
            &local_parents,
        )
        .map_err(ManifestError::Plan)
    }
}

#[derive(PartialEq)]
enum ManifestSection {
    Top,
    Repositories,
    Items,
}

fn parse_plan_manifest(contents: &str) -> Result<(PlanDraftValue, Option<String>), ManifestError> {
    let mut header: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    let mut primary_scope: Option<String> = None;
    let mut repositories: Vec<RepositoryEntry> = Vec::new();
    let mut items: Vec<ItemEntry> = Vec::new();
    let mut current_repo: Option<RepositoryEntry> = None;
    let mut current_item: Option<ItemEntry> = None;
    let mut item_list: Option<&'static str> = None;
    let mut section = ManifestSection::Top;

    for raw_line in contents.lines() {
        let line = raw_line.trim_end();
        if line.is_empty() {
            return Err(ManifestError::hard(
                "portfolio manifest contains a blank line",
            ));
        }
        if !line.starts_with(' ') {
            item_list = None;
            section = ManifestSection::Top;
            match line {
                "repositories:" => section = ManifestSection::Repositories,
                "items:" => section = ManifestSection::Items,
                _ => {
                    let (key, value) = line.split_once(": ").ok_or_else(|| {
                        ManifestError::hard(format!("malformed portfolio manifest line: {line}"))
                    })?;
                    match key {
                        "schema" | "portfolio_id" | "project_owner" | "source_spec"
                        | "plan_digest" => {
                            if header
                                .insert(key.to_owned(), manifest_scalar_value(value)?)
                                .is_some()
                            {
                                return Err(ManifestError::hard(format!(
                                    "duplicate `{key}` in portfolio manifest"
                                )));
                            }
                        }
                        "primary_scope" => {
                            if primary_scope.is_some() {
                                return Err(ManifestError::hard(
                                    "duplicate `primary_scope` in portfolio manifest",
                                ));
                            }
                            primary_scope = match value {
                                "null" => None,
                                value => Some(manifest_scalar_value(value)?),
                            };
                        }
                        other => {
                            return Err(ManifestError::hard(format!(
                                "unknown portfolio manifest key `{other}`"
                            )))
                        }
                    }
                }
            }
            continue;
        }
        if section == ManifestSection::Top {
            return Err(ManifestError::hard(format!(
                "unexpected indented line in portfolio manifest: {line}"
            )));
        }
        match section {
            ManifestSection::Repositories => {
                if let Some(field) = line.strip_prefix("  - id: ") {
                    if let Some(repo) = current_repo.take() {
                        repositories.push(repo);
                    }
                    current_repo = Some(RepositoryEntry {
                        id: Some(manifest_scalar_value(field)?),
                        ..RepositoryEntry::default()
                    });
                } else if let Some(field) = line.strip_prefix("    capability: ") {
                    let repo = current_repo.as_mut().ok_or_else(|| {
                        ManifestError::hard("repository `capability` before `id`")
                    })?;
                    if repo.capability.is_some() {
                        return Err(ManifestError::hard(
                            "duplicate `capability` in portfolio manifest",
                        ));
                    }
                    repo.capability = Some(manifest_scalar_value(field)?);
                } else if let Some(field) = line.strip_prefix("    observed_revision: ") {
                    let repo = current_repo.as_mut().ok_or_else(|| {
                        ManifestError::hard("repository `observed_revision` before `id`")
                    })?;
                    if repo.observed_revision.is_some() {
                        return Err(ManifestError::hard(
                            "duplicate `observed_revision` in portfolio manifest",
                        ));
                    }
                    repo.observed_revision = Some(match field {
                        "null" => None,
                        value => Some(manifest_scalar_value(value)?),
                    });
                } else {
                    return Err(ManifestError::hard(format!(
                        "malformed repository line in portfolio manifest: {line}"
                    )));
                }
            }
            ManifestSection::Items => {
                if let Some(field) = line.strip_prefix("  - key: ") {
                    if let Some(item) = current_item.take() {
                        items.push(item);
                    }
                    current_item = Some(ItemEntry {
                        key: Some(manifest_scalar_value(field)?),
                        ..ItemEntry::default()
                    });
                    item_list = None;
                } else if let Some(item) = current_item.as_mut() {
                    if let Some(field) = line.strip_prefix("    role: ") {
                        if item.role.is_some() {
                            return Err(ManifestError::hard(
                                "duplicate `role` in portfolio manifest",
                            ));
                        }
                        item.role = Some(manifest_scalar_value(field)?);
                        item_list = None;
                    } else if let Some(field) = line.strip_prefix("    repository: ") {
                        if item.repository.is_some() {
                            return Err(ManifestError::hard(
                                "duplicate `repository` in portfolio manifest",
                            ));
                        }
                        item.repository = Some(manifest_scalar_value(field)?);
                        item_list = None;
                    } else if let Some(field) = line.strip_prefix("    completion_policy: ") {
                        if item.completion.is_some() {
                            return Err(ManifestError::hard(
                                "duplicate `completion_policy` in portfolio manifest",
                            ));
                        }
                        item.completion = Some(manifest_scalar_value(field)?);
                        item_list = None;
                    } else if line == "    depends_on: []" {
                        if item.depends_on_seen {
                            return Err(ManifestError::hard(
                                "duplicate `depends_on` in portfolio manifest",
                            ));
                        }
                        item.depends_on_seen = true;
                        item_list = None;
                    } else if line == "    local_parents: []" {
                        if item.local_parents_seen {
                            return Err(ManifestError::hard(
                                "duplicate `local_parents` in portfolio manifest",
                            ));
                        }
                        item.local_parents_seen = true;
                        item_list = None;
                    } else if line == "    depends_on:" {
                        if item.depends_on_seen {
                            return Err(ManifestError::hard(
                                "duplicate `depends_on` in portfolio manifest",
                            ));
                        }
                        item.depends_on_seen = true;
                        item_list = Some("depends_on");
                    } else if line == "    local_parents:" {
                        if item.local_parents_seen {
                            return Err(ManifestError::hard(
                                "duplicate `local_parents` in portfolio manifest",
                            ));
                        }
                        item.local_parents_seen = true;
                        item_list = Some("local_parents");
                    } else if let Some(field) = line.strip_prefix("      - ") {
                        let list = if item_list == Some("depends_on") {
                            &mut item.depends_on
                        } else if item_list == Some("local_parents") {
                            &mut item.local_parents
                        } else {
                            return Err(ManifestError::hard(format!(
                                "unexpected list entry in portfolio manifest: {line}"
                            )));
                        };
                        list.push(manifest_scalar_value(field)?);
                    } else {
                        return Err(ManifestError::hard(format!(
                            "malformed item line in portfolio manifest: {line}"
                        )));
                    }
                } else {
                    return Err(ManifestError::hard(format!(
                        "malformed item line in portfolio manifest: {line}"
                    )));
                }
            }
            ManifestSection::Top => unreachable!(),
        }
    }
    if let Some(repo) = current_repo {
        repositories.push(repo);
    }
    if let Some(item) = current_item {
        items.push(item);
    }
    let schema = header
        .remove("schema")
        .ok_or_else(|| ManifestError::hard("portfolio manifest is missing `schema`"))?;
    super::portfolio::manifest::PortfolioPlan::check_schema(&schema)
        .map_err(ManifestError::Plan)?;
    let source_spec = SourceSpecIdentity::from_str(
        &header
            .remove("source_spec")
            .ok_or_else(|| ManifestError::hard("portfolio manifest is missing `source_spec`"))?,
    )
    .map_err(|detail| ManifestError::hard(format!("invalid source_spec: {detail}")))?;
    let project_owner = header
        .remove("project_owner")
        .ok_or_else(|| ManifestError::hard("portfolio manifest is missing `project_owner`"))?;
    let _portfolio_id = header
        .remove("portfolio_id")
        .ok_or_else(|| ManifestError::hard("portfolio manifest is missing `portfolio_id`"))?;
    let plan_digest = header.remove("plan_digest");
    if !header.is_empty() {
        return Err(ManifestError::hard(
            "portfolio manifest carries an unexpected key",
        ));
    }
    let primary_scope = match primary_scope {
        None => None,
        Some(selector) => match selector.as_str() {
            "spec-portfolio" => Some(PrimaryScopeSelector::SpecPortfolio),
            other => {
                let product = other
                    .strip_prefix("product:")
                    .filter(|product| !product.is_empty())
                    .ok_or_else(|| {
                        ManifestError::hard(format!(
                            "primary_scope `{selector}` must be `spec-portfolio` or `product:KEY`"
                        ))
                    })?;
                Some(PrimaryScopeSelector::Product(product.to_owned()))
            }
        },
    };
    Ok((
        PlanDraftValue {
            source_spec,
            project_owner,
            repositories: repositories
                .into_iter()
                .map(RepositoryEntry::into_facts)
                .collect::<Result<Vec<_>, _>>()?,
            items: items
                .into_iter()
                .map(ItemEntry::into_item)
                .collect::<Result<Vec<_>, _>>()?,
            primary_scope,
        },
        plan_digest,
    ))
}

struct PlanDraftValue {
    source_spec: SourceSpecIdentity,
    project_owner: String,
    repositories: Vec<super::portfolio::manifest::RepositoryFacts>,
    items: Vec<super::portfolio::manifest::PlanItem>,
    primary_scope: Option<PrimaryScopeSelector>,
}

impl PlanDraftValue {
    fn into_draft(self) -> Result<super::portfolio::manifest::PlanDraft, ManifestError> {
        let mut draft = super::portfolio::manifest::PlanDraft::new(
            Some(self.source_spec),
            Some(&self.project_owner),
            self.repositories,
            self.items,
        );
        if let Some(selector) = self.primary_scope {
            draft = draft.with_primary_scope(selector);
        }
        Ok(draft)
    }
}
