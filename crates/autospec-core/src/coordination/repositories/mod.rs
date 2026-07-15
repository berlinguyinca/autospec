use std::collections::{BTreeMap, BTreeSet};

mod json;

pub use json::parse_repository_routing_input_json;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RepositoryEvidence {
    pub name: String,
    pub archived: bool,
    pub pushed_at: Option<String>,
    pub readme: String,
    pub module_paths: Vec<String>,
    pub packages: Vec<String>,
    pub dependency_references: Vec<String>,
    pub revival_requested: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryFinding {
    pub repository: String,
    pub fingerprint: String,
    pub title: String,
    pub evidence: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RepositoryRoutingInput {
    pub repositories: Vec<RepositoryEvidence>,
    pub findings: Vec<RepositoryFinding>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalTarget {
    pub repository: String,
    pub score: i64,
    pub reasons: Vec<String>,
    pub routed_fingerprints: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DoNotFileRepository {
    pub repository: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutedFinding {
    pub target_repository: String,
    pub source_repository: String,
    pub fingerprint: String,
    pub title: String,
    pub evidence: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RepositoryRoutingReport {
    pub canonical_targets: Vec<CanonicalTarget>,
    pub do_not_file_by_default: Vec<DoNotFileRepository>,
    pub routed_findings: Vec<RoutedFinding>,
}

pub fn plan_repository_routing(input: &RepositoryRoutingInput) -> RepositoryRoutingReport {
    let by_name = input
        .repositories
        .iter()
        .map(|repository| (repository.name.clone(), repository.clone()))
        .collect::<BTreeMap<_, _>>();
    let scores = repository_scores(input);
    let component_for = repository_components(input);
    let mut component_members: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for repository in by_name.keys() {
        let root = component_for
            .get(repository)
            .cloned()
            .unwrap_or_else(|| repository.clone());
        component_members
            .entry(root)
            .or_default()
            .push(repository.clone());
    }

    let mut selected_for_component = BTreeMap::new();
    for (component, mut members) in component_members {
        members.sort();
        let selected = members
            .into_iter()
            .max_by(|left, right| compare_repository_scores(left, right, &scores))
            .unwrap_or_else(|| component.clone());
        selected_for_component.insert(component, selected);
    }

    let target_for_repository = by_name
        .keys()
        .map(|repository| {
            let component = component_for
                .get(repository)
                .cloned()
                .unwrap_or_else(|| repository.clone());
            let target = selected_for_component
                .get(&component)
                .cloned()
                .unwrap_or_else(|| repository.clone());
            (repository.clone(), target)
        })
        .collect::<BTreeMap<_, _>>();

    let mut seen_fingerprints = BTreeSet::new();
    let mut fingerprints_by_target: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut routed_findings = Vec::new();
    for finding in &input.findings {
        if finding.fingerprint.trim().is_empty()
            || !seen_fingerprints.insert(finding.fingerprint.clone())
        {
            continue;
        }
        let target = target_for_repository
            .get(&finding.repository)
            .cloned()
            .unwrap_or_else(|| finding.repository.clone());
        fingerprints_by_target
            .entry(target.clone())
            .or_default()
            .insert(finding.fingerprint.clone());
        routed_findings.push(RoutedFinding {
            target_repository: target,
            source_repository: finding.repository.clone(),
            fingerprint: finding.fingerprint.clone(),
            title: finding.title.clone(),
            evidence: finding.evidence.clone(),
        });
    }

    let mut canonical_targets = selected_for_component
        .values()
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|repository| {
            let score = scores.get(&repository).cloned().unwrap_or_default();
            let routed_fingerprints = routed_fingerprints(&mut fingerprints_by_target, &repository);
            CanonicalTarget {
                repository: repository.clone(),
                score: score.score,
                reasons: score.reasons,
                routed_fingerprints,
            }
        })
        .collect::<Vec<_>>();
    canonical_targets.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.repository.cmp(&right.repository))
    });

    let mut do_not_file_by_default = input
        .repositories
        .iter()
        .filter(|repository| archived_split_repository(repository, &by_name))
        .map(|repository| DoNotFileRepository {
            repository: repository.name.clone(),
            reason: "archived_split_repository".to_string(),
        })
        .collect::<Vec<_>>();
    do_not_file_by_default.sort_by(|left, right| left.repository.cmp(&right.repository));

    RepositoryRoutingReport {
        canonical_targets,
        do_not_file_by_default,
        routed_findings,
    }
}

#[derive(Debug, Clone, Default)]
struct RepositoryScore {
    score: i64,
    reasons: Vec<String>,
}

fn routed_fingerprints(
    fingerprints_by_target: &mut BTreeMap<String, BTreeSet<String>>,
    repository: &str,
) -> Vec<String> {
    fingerprints_by_target
        .remove(repository)
        .unwrap_or_default()
        .into_iter()
        .collect()
}

fn repository_scores(input: &RepositoryRoutingInput) -> BTreeMap<String, RepositoryScore> {
    let mut scores = input
        .repositories
        .iter()
        .map(|repository| {
            let mut score = RepositoryScore::default();
            if repository.archived {
                score.score -= 50;
                score.reasons.push("archived_repository".to_string());
            } else {
                score.score += 50;
                score.reasons.push("active_repository".to_string());
            }
            if let Some(pushed_at) = &repository.pushed_at {
                score.score += pushed_at_score(pushed_at);
                score.reasons.push(format!("pushed_at:{pushed_at}"));
            }
            let readme = repository.readme.to_ascii_lowercase();
            if !repository.readme.trim().is_empty() {
                score.score += 5;
                score.reasons.push("readme_present".to_string());
            }
            if readme.contains("canonical") || readme.contains("primary") {
                score.score += 20;
                score.reasons.push("readme:canonical".to_string());
            }
            if readme.contains("archived") || readme.contains("superseded") {
                score.score -= 15;
                score.reasons.push("readme:archived".to_string());
            }
            for module_path in &repository.module_paths {
                score.score += 3;
                score.reasons.push(format!("module_path:{module_path}"));
            }
            for package in &repository.packages {
                score.score += 5;
                score.reasons.push(format!("package:{package}"));
            }
            if repository.revival_requested {
                score.score += 40;
                score.reasons.push("revival_requested".to_string());
            }
            (repository.name.clone(), score)
        })
        .collect::<BTreeMap<_, _>>();

    for repository in &input.repositories {
        for reference in &repository.dependency_references {
            if let Some(target) = scores.get_mut(reference) {
                target.score += 15;
                let reason = format!("dependency_reference_from:{}", repository.name);
                target.reasons.push(reason);
            }
        }
    }
    scores
}

fn pushed_at_score(pushed_at: &str) -> i64 {
    let digits = pushed_at
        .chars()
        .filter(char::is_ascii_digit)
        .take(8)
        .collect::<String>();
    digits
        .parse::<i64>()
        .map(|value| value / 1_000_000)
        .unwrap_or(1)
}

fn compare_repository_scores(
    left: &str,
    right: &str,
    scores: &BTreeMap<String, RepositoryScore>,
) -> std::cmp::Ordering {
    let left_score = scores
        .get(left)
        .map(|score| score.score)
        .unwrap_or_default();
    let right_score = scores
        .get(right)
        .map(|score| score.score)
        .unwrap_or_default();
    left_score.cmp(&right_score).then_with(|| right.cmp(left))
}

fn archived_split_repository(
    repository: &RepositoryEvidence,
    by_name: &BTreeMap<String, RepositoryEvidence>,
) -> bool {
    repository.archived
        && !repository.revival_requested
        && (repository.dependency_references.iter().any(|reference| {
            by_name
                .get(reference)
                .is_some_and(|target| !target.archived)
        }) || repository.readme.to_ascii_lowercase().contains("split")
            || repository
                .readme
                .to_ascii_lowercase()
                .contains("superseded"))
}

fn repository_components(input: &RepositoryRoutingInput) -> BTreeMap<String, String> {
    let names = input
        .repositories
        .iter()
        .map(|repository| repository.name.clone())
        .collect::<BTreeSet<_>>();
    let mut union = UnionFind::new(names.iter().cloned());
    for repository in &input.repositories {
        for reference in &repository.dependency_references {
            if names.contains(reference) {
                union.union(&repository.name, reference);
            }
        }
    }
    let shared_keys = shared_signal_groups(input);
    for repositories in shared_keys.values() {
        let mut iter = repositories.iter();
        if let Some(first) = iter.next() {
            for repository in iter {
                union.union(first, repository);
            }
        }
    }
    names
        .into_iter()
        .map(|repository| {
            let root = union.find(&repository);
            (repository, root)
        })
        .collect()
}

fn shared_signal_groups(input: &RepositoryRoutingInput) -> BTreeMap<String, BTreeSet<String>> {
    let mut groups: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for repository in &input.repositories {
        for package in &repository.packages {
            groups
                .entry(format!("package:{}", package.to_ascii_lowercase()))
                .or_default()
                .insert(repository.name.clone());
        }
        for module_path in &repository.module_paths {
            groups
                .entry(format!("module:{}", module_path.to_ascii_lowercase()))
                .or_default()
                .insert(repository.name.clone());
        }
    }
    groups.retain(|_, repositories| repositories.len() > 1);
    groups
}

#[derive(Debug, Clone)]
struct UnionFind {
    parent: BTreeMap<String, String>,
}

impl UnionFind {
    fn new(values: impl IntoIterator<Item = String>) -> Self {
        Self {
            parent: values
                .into_iter()
                .map(|value| (value.clone(), value))
                .collect(),
        }
    }

    fn find(&mut self, value: &str) -> String {
        let parent = self
            .parent
            .get(value)
            .cloned()
            .unwrap_or_else(|| value.to_string());
        if parent == value {
            parent
        } else {
            let root = self.find(&parent);
            self.parent.insert(value.to_string(), root.clone());
            root
        }
    }

    fn union(&mut self, left: &str, right: &str) {
        let left_root = self.find(left);
        let right_root = self.find(right);
        if left_root != right_root {
            let (parent, child) = if left_root <= right_root {
                (left_root, right_root)
            } else {
                (right_root, left_root)
            };
            self.parent.insert(child, parent);
        }
    }
}
