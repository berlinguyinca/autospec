use super::OnboardingReport;
use autospec_core::managed_project::{RelationshipEdge, RelationshipState, RepositoryRecord};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn build(
    records: BTreeMap<String, RepositoryRecord>,
    edges: BTreeMap<String, RelationshipEdge>,
    existing_repositories: &BTreeSet<String>,
    existing_edges: &BTreeSet<String>,
    out_of_bound: usize,
    inaccessible: usize,
    pending_projection: usize,
) -> OnboardingReport {
    let repositories = records.into_values().collect::<Vec<_>>();
    let edges = edges.into_values().collect::<Vec<_>>();
    let unchanged = repositories
        .iter()
        .filter(|record| existing_repositories.contains(&record.repository))
        .count()
        + edges
            .iter()
            .filter(|edge| existing_edges.contains(&edge.dedupe_key()))
            .count();
    let adopted = repositories
        .iter()
        .filter(|record| {
            !existing_repositories.contains(&record.repository)
                && matches!(record.entry_kind.as_str(), "explicit-seed" | "workspace")
        })
        .count();
    let created = repositories
        .iter()
        .filter(|record| {
            !existing_repositories.contains(&record.repository)
                && !matches!(record.entry_kind.as_str(), "explicit-seed" | "workspace")
        })
        .count();
    let updated = edges
        .iter()
        .filter(|edge| !existing_edges.contains(&edge.dedupe_key()))
        .count();
    let proposed = edges
        .iter()
        .filter(|edge| edge.state == RelationshipState::Proposed)
        .count();
    OnboardingReport {
        created,
        adopted,
        updated,
        unchanged,
        proposed,
        out_of_bound,
        inaccessible,
        pending_projection,
        repositories,
        edges,
    }
}
