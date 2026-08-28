use super::{ManagedProjectError, ManagedProjectStore};
use autospec_core::managed_project::{
    ManagedProjectBinding, ManagedProjectPolicy, RelationshipEdge, RelationshipEvidence,
    RelationshipKind, RelationshipState, RepositoryRecord,
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};

#[path = "onboard/admission.rs"]
mod admission;

#[path = "onboard/cargo.rs"]
mod cargo;
#[path = "onboard/issues.rs"]
mod issues;
#[path = "onboard/line_formats.rs"]
mod line_formats;
#[path = "onboard/npm.rs"]
mod npm;
#[path = "onboard/report.rs"]
mod report;

pub(super) use admission::field_repository;
pub use admission::normalize_github_repository;
use admission::{
    allowed, normalize_repository, repository_admission, workspace_admission, Admission,
};

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
enum Discovery {
    Repository {
        value: String,
        target: String,
        source_issue: Option<u64>,
        kind: RelationshipKind,
        evidence: String,
        location: String,
    },
    Workspace {
        path: PathBuf,
        evidence: String,
        location: String,
    },
    Proposed {
        name: String,
        location: String,
    },
}

impl Discovery {
    fn repository(
        value: impl Into<String>,
        evidence: impl Into<String>,
        location: impl Into<String>,
    ) -> Self {
        let value = value.into();
        Self::Repository {
            target: value.clone(),
            value,
            source_issue: None,
            kind: RelationshipKind::DependsOn,
            evidence: evidence.into(),
            location: location.into(),
        }
    }

    fn typed_repository(
        value: impl Into<String>,
        target: impl Into<String>,
        source_issue: Option<u64>,
        kind: RelationshipKind,
        evidence: impl Into<String>,
        location: impl Into<String>,
    ) -> Self {
        Self::Repository {
            value: value.into(),
            target: target.into(),
            source_issue,
            kind,
            evidence: evidence.into(),
            location: location.into(),
        }
    }

    fn workspace(path: PathBuf, evidence: impl Into<String>, location: impl Into<String>) -> Self {
        Self::Workspace {
            path,
            evidence: evidence.into(),
            location: location.into(),
        }
    }

    fn proposed(name: impl Into<String>, location: impl Into<String>) -> Self {
        Self::Proposed {
            name: name.into(),
            location: location.into(),
        }
    }
}

struct Retention<'a> {
    records: &'a mut BTreeMap<String, RepositoryRecord>,
    edges: &'a mut BTreeMap<String, RelationshipEdge>,
    queue: &'a mut VecDeque<ScanTarget>,
    queued: &'a mut BTreeSet<String>,
    out_of_bound: &'a mut BTreeSet<String>,
    inaccessible: &'a mut BTreeSet<String>,
    expansions: &'a mut usize,
}

pub fn onboard_repositories(
    store: &mut ManagedProjectStore,
    policy: &ManagedProjectPolicy,
    options: &OnboardingOptions,
) -> Result<OnboardingReport, ManagedProjectError> {
    validate_policy(policy)?;
    let (existing_repositories, existing_edges, mut discovery) = baseline(store.snapshot());
    admit_explicit_seeds(&mut discovery, policy, options)?;
    admit_workspaces(&mut discovery, policy, options);
    scan_admitted_repositories(&mut discovery, policy)?;
    let mut report = report::build(
        discovery.records,
        discovery.edges,
        &existing_repositories,
        &existing_edges,
        discovery.out_of_bound.len(),
        discovery.inaccessible.len(),
        store.snapshot().pending_projections.len(),
    );
    if !options.dry_run {
        for record in &report.repositories {
            store.record_repository(record.clone())?;
        }
        for edge in &report.edges {
            store.record_edge(edge.clone())?;
        }
        report.pending_projection = store.snapshot().pending_projections.len();
    }
    Ok(report)
}

pub(crate) fn discover_remote_issue_relationships(
    policy: &ManagedProjectPolicy,
    issue_url: &str,
    body: &str,
) -> Result<OnboardingReport, ManagedProjectError> {
    let remainder = issue_url
        .strip_prefix("https://github.com/")
        .ok_or_else(|| ManagedProjectError::new("selected issue URL is not canonical"))?;
    let parts = remainder.split('/').collect::<Vec<_>>();
    if parts.len() != 4 || parts[2] != "issues" {
        return Err(ManagedProjectError::new(
            "selected issue URL is not canonical",
        ));
    }
    let repository = format!("{}/{}", parts[0], parts[1]);
    let number = parts[3]
        .parse::<u64>()
        .ok()
        .filter(|number| *number > 0)
        .ok_or_else(|| ManagedProjectError::new("selected issue URL has no positive number"))?;
    let mut state = DiscoveryState::new(BTreeMap::new(), BTreeMap::new());
    state.records.insert(
        repository.clone(),
        RepositoryRecord {
            repository: repository.clone(),
            entry_kind: "selected-issue".to_owned(),
        },
    );
    let source = ScanTarget {
        repository,
        path: PathBuf::new(),
    };
    for discovery in issues::scan_source(body, Some(number), issue_url) {
        retain_discovery(
            discovery,
            &source,
            policy,
            &mut Retention {
                records: &mut state.records,
                edges: &mut state.edges,
                queue: &mut state.queue,
                queued: &mut state.queued,
                out_of_bound: &mut state.out_of_bound,
                inaccessible: &mut state.inaccessible,
                expansions: &mut state.expansions,
            },
        );
    }
    let edges = state.edges.into_values().collect::<Vec<_>>();
    Ok(OnboardingReport {
        proposed: edges
            .iter()
            .filter(|edge| edge.state == RelationshipState::Proposed)
            .count(),
        out_of_bound: state.out_of_bound.len(),
        inaccessible: state.inaccessible.len(),
        repositories: state.records.into_values().collect(),
        edges,
        ..OnboardingReport::default()
    })
}

fn baseline(
    binding: &ManagedProjectBinding,
) -> (BTreeSet<String>, BTreeSet<String>, DiscoveryState) {
    let repositories = binding
        .repositories
        .iter()
        .map(|record| normalize_repository(&record.repository))
        .collect();
    let edges = binding
        .relationships
        .iter()
        .map(RelationshipEdge::dedupe_key)
        .collect();
    let records = binding
        .repositories
        .iter()
        .cloned()
        .map(|record| (normalize_repository(&record.repository), record))
        .collect();
    let edge_map = binding
        .relationships
        .iter()
        .cloned()
        .map(|edge| (edge.dedupe_key(), edge))
        .collect();
    (repositories, edges, DiscoveryState::new(records, edge_map))
}

struct DiscoveryState {
    records: BTreeMap<String, RepositoryRecord>,
    edges: BTreeMap<String, RelationshipEdge>,
    queue: VecDeque<ScanTarget>,
    queued: BTreeSet<String>,
    out_of_bound: BTreeSet<String>,
    inaccessible: BTreeSet<String>,
    expansions: usize,
}

impl DiscoveryState {
    fn new(
        records: BTreeMap<String, RepositoryRecord>,
        edges: BTreeMap<String, RelationshipEdge>,
    ) -> Self {
        Self {
            records,
            edges,
            queue: VecDeque::new(),
            queued: BTreeSet::new(),
            out_of_bound: BTreeSet::new(),
            inaccessible: BTreeSet::new(),
            expansions: 0,
        }
    }
}

fn admit_explicit_seeds(
    state: &mut DiscoveryState,
    policy: &ManagedProjectPolicy,
    options: &OnboardingOptions,
) -> Result<(), ManagedProjectError> {
    for seed in policy.repository_seeds.iter().chain(&options.repositories) {
        let repository = normalize_github_repository(seed).ok_or_else(|| {
            ManagedProjectError::new(format!("invalid explicit GitHub repository seed: {seed}"))
        })?;
        admit_record(
            repository,
            "explicit-seed",
            policy,
            &mut state.records,
            &mut state.out_of_bound,
        );
    }
    Ok(())
}

fn admit_workspaces(
    state: &mut DiscoveryState,
    policy: &ManagedProjectPolicy,
    options: &OnboardingOptions,
) {
    let mut paths = vec![options.repo_dir.clone()];
    paths.extend(options.workspaces.iter().cloned());
    paths.sort();
    paths.dedup();
    for path in paths {
        match workspace_admission(&path, policy) {
            Admission::Admitted(repository) => {
                state
                    .records
                    .entry(repository.clone())
                    .or_insert(RepositoryRecord {
                        repository: repository.clone(),
                        entry_kind: "workspace".to_owned(),
                    });
                enqueue(&mut state.queue, &mut state.queued, repository, path);
            }
            Admission::OutOfBound(repository) => {
                state.out_of_bound.insert(repository);
            }
            Admission::Inaccessible(identity) => {
                state.inaccessible.insert(identity);
            }
        }
    }
}

fn scan_admitted_repositories(
    state: &mut DiscoveryState,
    policy: &ManagedProjectPolicy,
) -> Result<(), ManagedProjectError> {
    while let Some(target) = state.queue.pop_front() {
        for discovery in scan_repository(&target.path)? {
            retain_discovery(
                discovery,
                &target,
                policy,
                &mut Retention {
                    records: &mut state.records,
                    edges: &mut state.edges,
                    queue: &mut state.queue,
                    queued: &mut state.queued,
                    out_of_bound: &mut state.out_of_bound,
                    inaccessible: &mut state.inaccessible,
                    expansions: &mut state.expansions,
                },
            );
        }
    }
    Ok(())
}

fn retain_discovery(
    discovery: Discovery,
    source: &ScanTarget,
    policy: &ManagedProjectPolicy,
    retention: &mut Retention<'_>,
) {
    let (admission, target, source_issue, kind, evidence, location, state, path) = match discovery {
        Discovery::Repository {
            value,
            target,
            source_issue,
            kind,
            evidence,
            location,
        } => (
            repository_admission(&value, policy),
            target,
            source_issue,
            kind,
            evidence,
            location,
            RelationshipState::Active,
            None,
        ),
        Discovery::Workspace {
            path,
            evidence,
            location,
        } => (
            workspace_admission(&path, policy),
            String::new(),
            None,
            RelationshipKind::DependsOn,
            evidence,
            location,
            RelationshipState::Active,
            Some(path),
        ),
        Discovery::Proposed { name, location } => (
            repository_admission(
                &format!("{}/{}", policy.owner.to_ascii_lowercase(), name),
                policy,
            ),
            String::new(),
            None,
            RelationshipKind::DependsOn,
            "name-similarity".to_owned(),
            location,
            RelationshipState::Proposed,
            None,
        ),
    };
    let repository = match admission {
        Admission::Admitted(repository) => repository,
        Admission::OutOfBound(repository) => {
            retention.out_of_bound.insert(repository);
            return;
        }
        Admission::Inaccessible(identity) => {
            retention.inaccessible.insert(identity);
            return;
        }
    };
    if repository == source.repository && source_issue.is_none() {
        return;
    }
    if !retention.records.contains_key(&repository) {
        if *retention.expansions >= policy.discovery_max_repos {
            return;
        }
        retention.records.insert(
            repository.clone(),
            RepositoryRecord {
                repository: repository.clone(),
                entry_kind: evidence.clone(),
            },
        );
        *retention.expansions += 1;
    }
    let target = if target.is_empty() {
        &repository
    } else {
        &target
    };
    let source_identity = source_issue.map_or_else(
        || source.repository.clone(),
        |number| format!("https://github.com/{}/issues/{number}", source.repository),
    );
    let edge = relationship(
        policy,
        &source_identity,
        target,
        kind,
        &evidence,
        &location,
        state,
    );
    retention.edges.entry(edge.dedupe_key()).or_insert(edge);
    if let Some(path) = path {
        enqueue(retention.queue, retention.queued, repository, path);
    }
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

fn scan_repository(path: &Path) -> Result<Vec<Discovery>, ManagedProjectError> {
    let mut discoveries = cargo::scan(path);
    discoveries.extend(npm::scan(path));
    discoveries.extend(line_formats::scan(path));
    discoveries.extend(issues::scan(path).map_err(|error| {
        ManagedProjectError::new(format!("cannot scan managed issue metadata: {error}"))
    })?);
    Ok(discoveries)
}

fn admit_record(
    repository: String,
    entry_kind: &str,
    policy: &ManagedProjectPolicy,
    records: &mut BTreeMap<String, RepositoryRecord>,
    out_of_bound: &mut BTreeSet<String>,
) {
    if !allowed(&repository, policy) {
        out_of_bound.insert(repository);
    } else {
        records
            .entry(repository.clone())
            .or_insert(RepositoryRecord {
                repository,
                entry_kind: entry_kind.to_owned(),
            });
    }
}

fn relationship(
    policy: &ManagedProjectPolicy,
    source: &str,
    target: &str,
    discovered_kind: RelationshipKind,
    evidence: &str,
    location: &str,
    state: RelationshipState,
) -> RelationshipEdge {
    let kind = match evidence {
        "source-spec" => RelationshipKind::Implements,
        "tracker" => RelationshipKind::Tracks,
        "fleet" | "manifest-dependency" if location.contains("workspace") => {
            RelationshipKind::Contains
        }
        _ => discovered_kind,
    };
    RelationshipEdge {
        product_key: policy.product_key.clone(),
        kind,
        source: source.to_owned(),
        target: target.to_owned(),
        evidence: RelationshipEvidence {
            kind: evidence.to_owned(),
            location: location.to_owned(),
            discovered_at: DISCOVERED_AT.to_owned(),
            confidence: if state == RelationshipState::Active {
                100
            } else {
                40
            },
        },
        state,
    }
}

fn enqueue(
    queue: &mut VecDeque<ScanTarget>,
    queued: &mut BTreeSet<String>,
    repository: String,
    path: PathBuf,
) {
    if queued.insert(repository.clone()) {
        queue.push_back(ScanTarget { repository, path });
    }
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
    let Ok(entries) = std::fs::read_dir(root.join(prefix)) else {
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

fn repository_name_references(line: &str) -> BTreeSet<String> {
    line.split(|character: char| !character.is_ascii_alphanumeric() && character != '-')
        .filter(|word| word.contains('-') && word.len() >= 5)
        .map(str::to_ascii_lowercase)
        .collect()
}
