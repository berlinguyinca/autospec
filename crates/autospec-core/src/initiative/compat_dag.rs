//! Adapters from the two older dependency graphs to the canonical task DAG.
//!
//! `graph::issue_dag::IssueGraph` (parallel decomposition) and
//! `graph::order::execution_order` over `spec::SpecMetadata` (spec ordering)
//! each describe work and its dependencies. These functions build the
//! canonical [`TaskGraph`] from either, so the old and the canonical graph
//! agree on order. They do not replace the old types; the field-by-field table
//! is `docs/contracts/initiative-dag-model-mapping.md` section "DAG models".
//!
//! Task ids are assigned `TASK-0001` upward in sorted source-id order, which
//! [`task_ids`] exposes so a caller can correlate source ids with tasks.
//!
//! Ownership is not dropped: exclusive and shared-write surfaces both become
//! [`Task::affects`]. A graph the old model rejected as cyclic is rejected
//! here, and an edge that names an unknown id is rejected rather than ignored.

use std::collections::{BTreeMap, BTreeSet};

use super::collision::AffectedSurface;
use super::dag::{GraphViolation, TaskGraph};
use super::ids::{GraphId, PlanId, TaskId};
use super::repository::RepositoryId;
use super::task::Task;
use crate::graph::issue_dag::IssueGraph;
use crate::graph::metadata::OwnedSurface;
use crate::spec::SpecMetadata;

/// Deterministic task ids for source ids: sorted, de-duplicated, then
/// `TASK-0001` upward.
pub fn task_ids<'a>(ids: impl IntoIterator<Item = &'a str>) -> BTreeMap<String, TaskId> {
    let sorted: BTreeSet<&str> = ids.into_iter().collect();
    sorted
        .into_iter()
        .enumerate()
        .map(|(index, id)| {
            let sequence = u32::try_from(index + 1).unwrap_or(u32::MAX);
            (id.to_string(), TaskId::from_sequence(sequence, 4))
        })
        .collect()
}

/// `dir/**` is the old glob form of the directory `dir/`.
fn surface_path(path: &str) -> String {
    match path.strip_suffix("/**") {
        Some(directory) => format!("{directory}/"),
        None => path.to_string(),
    }
}

fn affected(surface: &OwnedSurface) -> AffectedSurface {
    AffectedSurface {
        path: surface_path(&surface.path),
        symbols: surface.symbols.clone(),
    }
}

/// One source node, reduced to what the canonical task needs.
struct Node {
    id: String,
    title: String,
    affects: Vec<AffectedSurface>,
    depends_on: Vec<String>,
}

fn invalid(task: TaskId, problem: String) -> GraphViolation {
    GraphViolation::InvalidTask { task, problem }
}

fn build(
    nodes: Vec<Node>,
    repository: &RepositoryId,
    plan: PlanId,
) -> Result<TaskGraph, GraphViolation> {
    let ids = task_ids(nodes.iter().map(|node| node.id.as_str()));
    let mut seen = BTreeSet::new();
    let mut graph = TaskGraph::new(GraphId::from_sequence(1, 4), 1, plan);
    for node in nodes {
        let id = ids[&node.id].clone();
        if !seen.insert(node.id.clone()) {
            return Err(invalid(id, format!("duplicate source id {}", node.id)));
        }
        let mut depends_on = Vec::new();
        for dependency in &node.depends_on {
            let Some(target) = ids.get(dependency) else {
                return Err(invalid(
                    id,
                    format!("{} depends on unknown {dependency}", node.id),
                ));
            };
            if !depends_on.contains(target) {
                depends_on.push(target.clone());
            }
        }
        let mut task =
            Task::implementation(id, repository.clone(), Vec::new(), 1).depending_on(depends_on);
        task.summary = node.title;
        task.affects = node.affects;
        graph.insert(task);
    }
    graph.topological_order()?;
    Ok(graph)
}

/// The canonical graph for an [`IssueGraph`].
///
/// `PlannedIssue.ownership.exclusive` and `shared_write` become `affects`;
/// each hard edge becomes a `depends_on` of its successor.
pub fn task_graph_from_issue_graph(
    graph: &IssueGraph,
    repository: RepositoryId,
    plan: PlanId,
) -> Result<TaskGraph, GraphViolation> {
    let ids = task_ids(graph.issues.iter().map(|issue| issue.id.as_str()));
    let mut depends: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for edge in &graph.hard_edges {
        if !ids.contains_key(&edge.predecessor) || !ids.contains_key(&edge.successor) {
            let known = ids
                .get(&edge.predecessor)
                .or_else(|| ids.get(&edge.successor))
                .cloned()
                .unwrap_or_else(|| TaskId::from_sequence(0, 4));
            return Err(invalid(
                known,
                format!(
                    "edge {} -> {} names an unknown issue",
                    edge.predecessor, edge.successor
                ),
            ));
        }
        depends
            .entry(edge.successor.as_str())
            .or_default()
            .push(edge.predecessor.clone());
    }
    let nodes = graph
        .issues
        .iter()
        .map(|issue| Node {
            id: issue.id.clone(),
            title: issue.title.clone(),
            affects: issue
                .ownership
                .exclusive
                .iter()
                .chain(&issue.ownership.shared_write)
                .map(affected)
                .collect(),
            depends_on: depends.remove(issue.id.as_str()).unwrap_or_default(),
        })
        .collect();
    build(nodes, &repository, plan)
}

/// The canonical graph for a list of specs.
///
/// `files_to_touch` become `affects`; `dependencies` become `depends_on`.
pub fn task_graph_from_specs(
    specs: &[SpecMetadata],
    repository: RepositoryId,
    plan: PlanId,
) -> Result<TaskGraph, GraphViolation> {
    let nodes = specs
        .iter()
        .map(|spec| Node {
            id: spec.id.as_str().to_string(),
            title: spec.title.clone(),
            affects: spec
                .files_to_touch
                .iter()
                .map(|path| AffectedSurface {
                    path: surface_path(path),
                    symbols: Vec::new(),
                })
                .collect(),
            depends_on: spec.dependencies.clone(),
        })
        .collect();
    build(nodes, &repository, plan)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::execution_order;
    use crate::graph::issue_dag::{DependencyEdge, PlannedIssue};
    use crate::graph::metadata::{ConcurrencyMetadata, DependencyReason, Ownership};
    use crate::spec::{SpecId, SpecStatus, SpecVersion};

    fn repository() -> RepositoryId {
        RepositoryId::parse("github.com/InferWeave/autospec").expect("valid repository id")
    }

    fn plan() -> PlanId {
        PlanId::from_sequence(1, 4)
    }

    fn surface(path: &str, symbols: &[&str]) -> OwnedSurface {
        OwnedSurface {
            path: path.to_string(),
            symbols: symbols.iter().map(|s| (*s).to_string()).collect(),
        }
    }

    fn issue(
        id: &str,
        exclusive: Vec<OwnedSurface>,
        shared_write: Vec<OwnedSurface>,
    ) -> PlannedIssue {
        PlannedIssue {
            id: id.to_string(),
            title: format!("issue {id}"),
            ownership: Ownership {
                exclusive,
                shared_read: vec![surface("docs/read-only.md", &[])],
                shared_write,
            },
            concurrency: ConcurrencyMetadata {
                parallel_safe: true,
                conflict_domains: Vec::new(),
            },
        }
    }

    fn edge(predecessor: &str, successor: &str) -> DependencyEdge {
        DependencyEdge {
            predecessor: predecessor.to_string(),
            successor: successor.to_string(),
            reason: DependencyReason::ConsumesNewInterface,
            artifact: None,
        }
    }

    fn diamond() -> IssueGraph {
        IssueGraph {
            issues: ["a", "b", "c", "d"]
                .iter()
                .map(|id| issue(id, Vec::new(), Vec::new()))
                .collect(),
            hard_edges: vec![
                edge("a", "b"),
                edge("a", "c"),
                edge("b", "d"),
                edge("c", "d"),
            ],
        }
    }

    fn spec(id: &str, version: &str, dependencies: &[&str], files: &[&str]) -> SpecMetadata {
        SpecMetadata {
            id: SpecId::new(id).expect("valid spec id"),
            title: format!("spec {id}"),
            version: SpecVersion::new(version).expect("valid version"),
            status: SpecStatus::Ready,
            objective: String::new(),
            dependencies: dependencies.iter().map(|d| (*d).to_string()).collect(),
            acceptance_criteria: Vec::new(),
            validation_command: String::new(),
            blocking_gates: Vec::new(),
            files_to_touch: files.iter().map(|f| (*f).to_string()).collect(),
            run_budget: String::new(),
        }
    }

    #[test]
    fn a_diamond_has_the_same_waves_in_both_models() {
        let source = diamond();
        let adapted = task_graph_from_issue_graph(&source, repository(), plan()).expect("acyclic");
        let ids = task_ids(source.issues.iter().map(|issue| issue.id.as_str()));
        let expected: Vec<Vec<TaskId>> = source
            .waves()
            .expect("acyclic")
            .iter()
            .map(|wave| wave.iter().map(|id| ids[id].clone()).collect())
            .collect();
        assert_eq!(adapted.concurrency_waves().expect("acyclic"), expected);
        assert_eq!(expected.len(), 3);
    }

    #[test]
    fn a_cyclic_issue_graph_is_rejected_as_a_cycle() {
        let mut source = diamond();
        source.hard_edges.push(edge("d", "a"));
        let error = task_graph_from_issue_graph(&source, repository(), plan()).expect_err("cycle");
        assert!(matches!(error, GraphViolation::Cycle { .. }), "{error:?}");
    }

    #[test]
    fn an_edge_naming_an_unknown_issue_is_rejected() {
        let mut source = diamond();
        source.hard_edges.push(edge("ghost", "a"));
        let error =
            task_graph_from_issue_graph(&source, repository(), plan()).expect_err("unknown");
        assert!(
            matches!(error, GraphViolation::InvalidTask { .. }),
            "{error:?}"
        );
    }

    #[test]
    fn a_duplicate_issue_id_is_rejected() {
        let mut source = diamond();
        source.issues.push(issue("a", Vec::new(), Vec::new()));
        let error = task_graph_from_issue_graph(&source, repository(), plan()).expect_err("dup");
        assert!(
            matches!(error, GraphViolation::InvalidTask { .. }),
            "{error:?}"
        );
    }

    #[test]
    fn ownership_becomes_affects_and_a_glob_becomes_a_directory() {
        let source = IssueGraph {
            issues: vec![issue(
                "a",
                vec![surface("a/**", &[]), surface("src/x.rs", &["f"])],
                vec![surface("shared.rs", &[])],
            )],
            hard_edges: Vec::new(),
        };
        let adapted = task_graph_from_issue_graph(&source, repository(), plan()).expect("ok");
        let task = adapted.tasks().next().expect("one task");
        let paths: Vec<&str> = task.affects.iter().map(|s| s.path.as_str()).collect();
        assert_eq!(paths, vec!["a/", "src/x.rs", "shared.rs"]);
        assert_eq!(task.affects[1].symbols, vec!["f"]);
        assert_eq!(task.summary, "issue a");
    }

    #[test]
    fn spec_execution_order_is_a_valid_topological_order_of_the_adapted_graph() {
        let specs = vec![
            spec("v1-base", "V1", &[], &["crates/a/"]),
            spec("v2-mid", "V2", &["v1-base"], &["crates/b.rs"]),
            spec("v3-top", "V3", &["v2-mid", "v1-base"], &[]),
        ];
        let adapted = task_graph_from_specs(&specs, repository(), plan()).expect("acyclic");
        let ids = task_ids(specs.iter().map(|spec| spec.id.as_str()));
        let order: Vec<TaskId> = execution_order(&specs)
            .expect("acyclic")
            .iter()
            .map(|id| ids[id].clone())
            .collect();
        for (position, id) in order.iter().enumerate() {
            let task = adapted.get(id).expect("task exists");
            for dependency in &task.depends_on {
                let before = order
                    .iter()
                    .position(|entry| entry == dependency)
                    .expect("listed");
                assert!(before < position, "{dependency} must precede {id}");
            }
        }
        assert_eq!(order.len(), 3);
        let base = adapted.get(&ids["v1-base"]).expect("base");
        assert_eq!(base.affects[0].path, "crates/a/");
    }

    #[test]
    fn a_spec_depending_on_a_missing_spec_is_rejected() {
        let specs = vec![spec("v1-base", "V1", &["v9-ghost"], &[])];
        let error = task_graph_from_specs(&specs, repository(), plan()).expect_err("missing");
        assert!(
            matches!(error, GraphViolation::InvalidTask { .. }),
            "{error:?}"
        );
    }

    #[test]
    fn a_cyclic_spec_list_is_rejected_as_a_cycle() {
        let specs = vec![
            spec("v1-a", "V1", &["v2-b"], &[]),
            spec("v2-b", "V2", &["v1-a"], &[]),
        ];
        let error = task_graph_from_specs(&specs, repository(), plan()).expect_err("cycle");
        assert!(matches!(error, GraphViolation::Cycle { .. }), "{error:?}");
    }
}
