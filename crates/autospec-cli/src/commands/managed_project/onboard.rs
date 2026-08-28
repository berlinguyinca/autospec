use super::{ManagedProjectError, ManagedProjectStore};
use autospec_core::managed_project::{
    ManagedProjectBinding, ManagedProjectPolicy, RelationshipEdge, RelationshipEvidence,
    RelationshipKind, RelationshipState, RepositoryRecord,
};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const DISCOVERED_AT: &str = "deterministic-local-scan";

#[derive(Clone, Debug)]
pub struct OnboardingOptions {
    pub repo_dir: PathBuf,
    pub repositories: Vec<String>,
    pub workspaces: Vec<PathBuf>,
    pub dry_run: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OnboardingReport {
    pub created: usize,
    pub adopted: usize,
    pub updated: usize,
    pub unchanged: usize,
    pub proposed: usize,
    pub out_of_bound: usize,
    pub inaccessible: usize,
    pub pending_projection: usize,
    pub repositories: Vec<RepositoryRecord>,
    pub edges: Vec<RelationshipEdge>,
}

#[derive(Clone, Debug)]
struct ScanTarget {
    repository: String,
    path: PathBuf,
}

#[derive(Clone, Debug)]
struct Candidate {
    repository: String,
    entry_kind: String,
    edge: Option<RelationshipEdge>,
    path: Option<PathBuf>,
}

pub fn onboard_repositories(
    store: &mut ManagedProjectStore,
    policy: &ManagedProjectPolicy,
    options: &OnboardingOptions,
) -> Result<OnboardingReport, ManagedProjectError> {
    validate_policy(policy)?;
    let existing_repositories = store
        .snapshot()
        .repositories
        .iter()
        .map(|record| normalize_repository(&record.repository))
        .collect::<BTreeSet<_>>();
    let existing_edges = store
        .snapshot()
        .relationships
        .iter()
        .map(RelationshipEdge::dedupe_key)
        .collect::<BTreeSet<_>>();
    let mut report = OnboardingReport::default();
    let mut records = BTreeMap::<String, RepositoryRecord>::new();
    let mut edges = BTreeMap::<String, RelationshipEdge>::new();
    let mut queued = BTreeSet::new();
    let mut queue = VecDeque::new();
    let mut out_of_bound = BTreeSet::new();
    let mut inaccessible = BTreeSet::new();

    for seed in policy
        .repository_seeds
        .iter()
        .chain(options.repositories.iter())
    {
        admit_repository(
            seed,
            "explicit-seed",
            policy,
            &mut records,
            &mut out_of_bound,
        );
    }

    let mut workspace_paths = vec![options.repo_dir.clone()];
    workspace_paths.extend(options.workspaces.iter().cloned());
    workspace_paths.sort();
    workspace_paths.dedup();
    for path in workspace_paths {
        match workspace_repository(&path) {
            Ok(repository) if allowed(&repository, policy) => {
                records
                    .entry(repository.clone())
                    .or_insert(RepositoryRecord {
                        repository: repository.clone(),
                        entry_kind: "workspace".to_owned(),
                    });
                if queued.len() < policy.discovery_max_repos && queued.insert(repository.clone()) {
                    queue.push_back(ScanTarget { repository, path });
                }
            }
            Ok(repository) => {
                out_of_bound.insert(repository);
            }
            Err(_) => {
                inaccessible.insert(path.display().to_string());
            }
        }
    }

    while let Some(target) = queue.pop_front() {
        for candidate in scan_repository(&target, policy)? {
            if !allowed(&candidate.repository, policy) {
                out_of_bound.insert(candidate.repository);
                continue;
            }
            let proposed = candidate
                .edge
                .as_ref()
                .is_some_and(|edge| edge.state == RelationshipState::Proposed);
            if let Some(edge) = candidate.edge {
                edges.entry(edge.dedupe_key()).or_insert(edge);
            }
            if proposed || records.contains_key(&candidate.repository) {
                continue;
            }
            if records.len() >= policy.discovery_max_repos {
                continue;
            }
            records.insert(
                candidate.repository.clone(),
                RepositoryRecord {
                    repository: candidate.repository.clone(),
                    entry_kind: candidate.entry_kind,
                },
            );
            if let Some(path) = candidate.path {
                if queued.len() < policy.discovery_max_repos
                    && queued.insert(candidate.repository.clone())
                {
                    queue.push_back(ScanTarget {
                        repository: candidate.repository,
                        path,
                    });
                }
            }
        }
    }

    report.out_of_bound = out_of_bound.len();
    report.inaccessible = inaccessible.len();
    report.repositories = records.into_values().collect();
    report.edges = edges.into_values().collect();
    report.proposed = report
        .edges
        .iter()
        .filter(|edge| edge.state == RelationshipState::Proposed)
        .count();

    for record in &report.repositories {
        if existing_repositories.contains(&record.repository) {
            report.unchanged += 1;
        } else if matches!(record.entry_kind.as_str(), "explicit-seed" | "workspace") {
            report.adopted += 1;
        } else {
            report.created += 1;
        }
        if !options.dry_run {
            store.record_repository(record.clone())?;
        }
    }
    for edge in &report.edges {
        if existing_edges.contains(&edge.dedupe_key()) {
            report.unchanged += 1;
        } else {
            report.updated += 1;
        }
        if !options.dry_run {
            store.record_edge(edge.clone())?;
        }
    }
    report.pending_projection = store.snapshot().pending_projections.len();
    Ok(report)
}

pub fn active_dependency_graph(binding: &ManagedProjectBinding) -> Vec<RelationshipEdge> {
    let mut edges = binding
        .relationships
        .iter()
        .filter(|edge| {
            edge.state == RelationshipState::Active
                && matches!(
                    edge.kind,
                    RelationshipKind::DependsOn | RelationshipKind::Blocks
                )
        })
        .cloned()
        .collect::<Vec<_>>();
    edges.sort_by_key(RelationshipEdge::dedupe_key);
    edges
}

pub fn normalize_github_repository(value: &str) -> Option<String> {
    let value = value
        .trim()
        .trim_matches(|character: char| "\"'`()[]{}<>,;".contains(character));
    let path = if let Some(path) = value.strip_prefix("git@github.com:") {
        path
    } else if let Some(path) = value.strip_prefix("ssh://git@github.com/") {
        path
    } else if let Some(path) = value.strip_prefix("https://github.com/") {
        path
    } else if let Some(path) = value.strip_prefix("http://github.com/") {
        path
    } else if let Some(path) = value.strip_prefix("github:") {
        path
    } else if let Some(path) = value.strip_prefix("github.com/") {
        path
    } else if value.matches('/').count() == 1 {
        value
    } else {
        return None;
    };
    let mut components = path.split('/');
    let owner = clean_component(components.next()?);
    let repository = clean_component(components.next()?);
    if owner.is_empty()
        || repository.is_empty()
        || !owner
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-')
        || !repository.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
    {
        return None;
    }
    Some(format!("{owner}/{repository}").to_ascii_lowercase())
}

fn clean_component(value: &str) -> &str {
    value
        .split(['#', '?'])
        .next()
        .unwrap_or_default()
        .trim_end_matches(".git")
        .trim_matches(|character: char| "\"'`()[]{}<>,;".contains(character))
}

fn normalize_repository(value: &str) -> String {
    normalize_github_repository(value).unwrap_or_else(|| value.trim().to_ascii_lowercase())
}

fn validate_policy(policy: &ManagedProjectPolicy) -> Result<(), ManagedProjectError> {
    if policy.owner.trim().is_empty()
        || policy.repo_allowlist.is_empty()
        || policy.discovery_max_repos == 0
    {
        return Err(ManagedProjectError::new(
            "managed onboarding requires owner, allowlist, and positive discovery bound",
        ));
    }
    Ok(())
}

fn allowed(repository: &str, policy: &ManagedProjectPolicy) -> bool {
    let repository = normalize_repository(repository);
    let Some((owner, _)) = repository.split_once('/') else {
        return false;
    };
    owner.eq_ignore_ascii_case(policy.owner.trim())
        && policy
            .repo_allowlist
            .iter()
            .any(|pattern| wildcard_match(&repository, &pattern.to_ascii_lowercase()))
}

fn wildcard_match(value: &str, pattern: &str) -> bool {
    match pattern.split_once('*') {
        Some((prefix, suffix)) => value.starts_with(prefix) && value.ends_with(suffix),
        None => value == pattern,
    }
}

fn admit_repository(
    value: &str,
    entry_kind: &str,
    policy: &ManagedProjectPolicy,
    records: &mut BTreeMap<String, RepositoryRecord>,
    out_of_bound: &mut BTreeSet<String>,
) {
    let Some(repository) = normalize_github_repository(value) else {
        return;
    };
    if allowed(&repository, policy) {
        records
            .entry(repository.clone())
            .or_insert(RepositoryRecord {
                repository,
                entry_kind: entry_kind.to_owned(),
            });
    } else {
        out_of_bound.insert(repository);
    }
}

fn workspace_repository(path: &Path) -> Result<String, ManagedProjectError> {
    let output = Command::new("git")
        .args([
            "-C",
            path.to_string_lossy().as_ref(),
            "remote",
            "get-url",
            "origin",
        ])
        .output()
        .map_err(|error| {
            ManagedProjectError::new(format!("cannot inspect workspace remote: {error}"))
        })?;
    if !output.status.success() {
        return Err(ManagedProjectError::new("workspace has no verified origin"));
    }
    normalize_github_repository(&String::from_utf8_lossy(&output.stdout))
        .ok_or_else(|| ManagedProjectError::new("workspace origin is not a GitHub repository"))
}

fn scan_repository(
    target: &ScanTarget,
    policy: &ManagedProjectPolicy,
) -> Result<Vec<Candidate>, ManagedProjectError> {
    let mut candidates = Vec::new();
    candidates.extend(local_workspace_candidates(target, policy));
    for (relative, evidence) in [
        (".gitmodules", "submodule"),
        ("Cargo.toml", "manifest-dependency"),
        ("package.json", "manifest-dependency"),
        ("pnpm-workspace.yaml", "manifest-dependency"),
        ("go.mod", "manifest-dependency"),
        ("autospec-fleet.yml", "fleet"),
        (".autospec/fleet.yml", "fleet"),
    ] {
        let path = target.path.join(relative);
        let Ok(source) = fs::read_to_string(&path) else {
            continue;
        };
        for repository in github_references(&source) {
            if repository == target.repository {
                continue;
            }
            candidates.push(exact_candidate(
                policy, target, repository, evidence, relative,
            ));
        }
    }
    for path in managed_issue_files(&target.path)? {
        let source = fs::read_to_string(&path).map_err(|error| {
            ManagedProjectError::new(format!("cannot read {}: {error}", path.display()))
        })?;
        let location = path
            .strip_prefix(&target.path)
            .unwrap_or(&path)
            .display()
            .to_string();
        for line in managed_relationship_lines(&source) {
            let evidence = if line.to_ascii_lowercase().contains("source spec") {
                "source-spec"
            } else if line.to_ascii_lowercase().contains("tracker") {
                "tracker"
            } else {
                "issue-reference"
            };
            let exact = github_references(line);
            if exact.is_empty() {
                for name in repository_name_references(line) {
                    let repository = format!("{}/{name}", policy.owner.to_ascii_lowercase());
                    if repository != target.repository && allowed(&repository, policy) {
                        candidates.push(Candidate {
                            repository: repository.clone(),
                            entry_kind: "proposed".to_owned(),
                            edge: Some(edge(
                                policy,
                                &target.repository,
                                &repository,
                                RelationshipKind::DependsOn,
                                "name-similarity",
                                &location,
                                (RelationshipState::Proposed, 40),
                            )),
                            path: None,
                        });
                    }
                }
            }
            for repository in exact {
                if repository != target.repository {
                    candidates.push(exact_candidate(
                        policy, target, repository, evidence, &location,
                    ));
                }
            }
        }
    }
    candidates.sort_by(|left, right| {
        left.repository
            .cmp(&right.repository)
            .then_with(|| left.entry_kind.cmp(&right.entry_kind))
            .then_with(|| {
                left.edge
                    .as_ref()
                    .map(RelationshipEdge::dedupe_key)
                    .cmp(&right.edge.as_ref().map(RelationshipEdge::dedupe_key))
            })
    });
    Ok(candidates)
}

fn exact_candidate(
    policy: &ManagedProjectPolicy,
    target: &ScanTarget,
    repository: String,
    evidence: &str,
    location: &str,
) -> Candidate {
    let kind = match evidence {
        "source-spec" => RelationshipKind::Implements,
        "tracker" => RelationshipKind::Tracks,
        "fleet" => RelationshipKind::Contains,
        _ => RelationshipKind::DependsOn,
    };
    Candidate {
        repository: repository.clone(),
        entry_kind: evidence.to_owned(),
        edge: Some(edge(
            policy,
            &target.repository,
            &repository,
            kind,
            evidence,
            location,
            (RelationshipState::Active, 100),
        )),
        path: None,
    }
}

fn local_workspace_candidates(
    target: &ScanTarget,
    policy: &ManagedProjectPolicy,
) -> Vec<Candidate> {
    let mut paths = BTreeSet::new();
    if let Ok(source) = fs::read_to_string(target.path.join("Cargo.toml")) {
        for member in cargo_workspace_members(&source) {
            paths.insert(target.path.join(member));
        }
    }
    if let Ok(source) = fs::read_to_string(target.path.join("package.json")) {
        if let Ok(value) = serde_json::from_str::<Value>(&source) {
            let workspaces = value
                .get("workspaces")
                .and_then(|value| value.as_array())
                .or_else(|| {
                    value
                        .get("workspaces")
                        .and_then(|value| value.get("packages"))
                        .and_then(|value| value.as_array())
                });
            for workspace in workspaces.into_iter().flatten().filter_map(Value::as_str) {
                paths.extend(expand_workspace_path(&target.path, workspace));
            }
        }
    }
    paths
        .into_iter()
        .filter_map(|path| {
            let repository = workspace_repository(&path).ok()?;
            if !allowed(&repository, policy) || repository == target.repository {
                return None;
            }
            Some(Candidate {
                repository: repository.clone(),
                entry_kind: "manifest-dependency".to_owned(),
                edge: Some(edge(
                    policy,
                    &target.repository,
                    &repository,
                    RelationshipKind::Contains,
                    "manifest-dependency",
                    "workspace-member",
                    (RelationshipState::Active, 100),
                )),
                path: Some(path),
            })
        })
        .collect()
}

fn cargo_workspace_members(source: &str) -> Vec<&str> {
    let mut members = Vec::new();
    let mut in_workspace = false;
    let mut in_members = false;
    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_workspace = trimmed == "[workspace]";
            in_members = false;
            continue;
        }
        if !in_workspace {
            continue;
        }
        if trimmed.starts_with("members") {
            in_members = true;
        }
        if in_members {
            members.extend(quoted_values(trimmed));
            if trimmed.contains(']') {
                in_members = false;
            }
        }
    }
    members
}

fn quoted_values(line: &str) -> Vec<&str> {
    let mut values = Vec::new();
    let mut remaining = line;
    while let Some(start) = remaining.find(['\"', '\'']) {
        remaining = &remaining[start + 1..];
        let Some(end) = remaining.find(['\"', '\'']) else {
            break;
        };
        values.push(&remaining[..end]);
        remaining = &remaining[end + 1..];
    }
    values
}

fn expand_workspace_path(root: &Path, workspace: &str) -> Vec<PathBuf> {
    let Some(prefix) = workspace.strip_suffix("/*") else {
        return vec![root.join(workspace)];
    };
    let directory = root.join(prefix);
    let Ok(entries) = fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut paths = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

fn edge(
    policy: &ManagedProjectPolicy,
    source: &str,
    target: &str,
    kind: RelationshipKind,
    evidence_kind: &str,
    location: &str,
    classification: (RelationshipState, u8),
) -> RelationshipEdge {
    RelationshipEdge {
        product_key: policy.product_key.clone(),
        kind,
        source: source.to_owned(),
        target: target.to_owned(),
        evidence: RelationshipEvidence {
            kind: evidence_kind.to_owned(),
            location: location.to_owned(),
            discovered_at: DISCOVERED_AT.to_owned(),
            confidence: classification.1,
        },
        state: classification.0,
    }
}

fn github_references(source: &str) -> BTreeSet<String> {
    source
        .split(|character: char| {
            !(character.is_ascii_alphanumeric()
                || matches!(character, '-' | '_' | '.' | '/' | ':' | '@' | '#' | '?'))
        })
        .filter(|token| !token.is_empty())
        .filter_map(normalize_github_repository)
        .collect()
}

fn managed_issue_files(root: &Path) -> Result<Vec<PathBuf>, ManagedProjectError> {
    let directory = root.join(".autospec/issues");
    let Ok(entries) = fs::read_dir(&directory) else {
        return Ok(Vec::new());
    };
    let mut paths = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "md"))
        .collect::<Vec<_>>();
    paths.sort();
    Ok(paths)
}

fn managed_relationship_lines(source: &str) -> impl Iterator<Item = &str> {
    let mut managed = false;
    source.lines().filter(move |line| {
        if line.starts_with("## ") {
            managed = line.to_ascii_lowercase().contains("autospec");
            return false;
        }
        managed && !line.trim().is_empty()
    })
}

fn repository_name_references(line: &str) -> BTreeSet<String> {
    line.split(|character: char| !character.is_ascii_alphanumeric() && character != '-')
        .filter(|word| word.contains('-') && word.len() >= 5)
        .map(str::to_ascii_lowercase)
        .collect()
}
