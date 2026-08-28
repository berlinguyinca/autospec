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
        Self::Repository {
            value: value.into(),
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
    let mut records = store
        .snapshot()
        .repositories
        .iter()
        .cloned()
        .map(|record| (normalize_repository(&record.repository), record))
        .collect::<BTreeMap<_, _>>();
    let mut edges = store
        .snapshot()
        .relationships
        .iter()
        .cloned()
        .map(|edge| (edge.dedupe_key(), edge))
        .collect::<BTreeMap<_, _>>();
    let mut queue = VecDeque::new();
    let mut queued = BTreeSet::new();
    let mut out_of_bound = BTreeSet::new();
    let mut inaccessible = BTreeSet::new();
    let mut expansions = 0;

    for seed in policy
        .repository_seeds
        .iter()
        .chain(options.repositories.iter())
    {
        let repository = normalize_github_repository(seed).ok_or_else(|| {
            ManagedProjectError::new(format!("invalid explicit GitHub repository seed: {seed}"))
        })?;
        admit_record(
            repository,
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
        match workspace_admission(&path, policy) {
            Admission::Admitted(repository) => {
                records
                    .entry(repository.clone())
                    .or_insert(RepositoryRecord {
                        repository: repository.clone(),
                        entry_kind: "workspace".to_owned(),
                    });
                enqueue(&mut queue, &mut queued, repository, path);
            }
            Admission::OutOfBound(repository) => {
                out_of_bound.insert(repository);
            }
            Admission::Inaccessible(identity) => {
                inaccessible.insert(identity);
            }
        }
    }

    while let Some(target) = queue.pop_front() {
        for discovery in scan_repository(&target.path)? {
            retain_discovery(
                discovery,
                &target,
                policy,
                &mut Retention {
                    records: &mut records,
                    edges: &mut edges,
                    queue: &mut queue,
                    queued: &mut queued,
                    out_of_bound: &mut out_of_bound,
                    inaccessible: &mut inaccessible,
                    expansions: &mut expansions,
                },
            );
        }
    }

    let mut report = report::build(
        records,
        edges,
        &existing_repositories,
        &existing_edges,
        out_of_bound.len(),
        inaccessible.len(),
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

fn retain_discovery(
    discovery: Discovery,
    source: &ScanTarget,
    policy: &ManagedProjectPolicy,
    retention: &mut Retention<'_>,
) {
    let (admission, evidence, location, state, path) = match discovery {
        Discovery::Repository {
            value,
            evidence,
            location,
        } => (
            repository_admission(&value, policy),
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
    if repository == source.repository {
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
    let edge = relationship(policy, source, &repository, &evidence, &location, state);
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
    source: &ScanTarget,
    target: &str,
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
        _ => RelationshipKind::DependsOn,
    };
    RelationshipEdge {
        product_key: policy.product_key.clone(),
        kind,
        source: source.repository.clone(),
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
