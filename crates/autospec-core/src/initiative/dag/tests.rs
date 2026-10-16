use super::*;
use crate::initiative::collision::AffectedSurface;
use crate::initiative::definition::{
    AcceptanceCriterion, Definition, Provenance, Requirement, RequirementKind,
};
use crate::initiative::ids::{CriterionId, InitiativeId};
use crate::initiative::repository::RepositoryRecord;
use crate::initiative::roles::AgentRole;
use crate::initiative::task::Lease;

fn task_id(text: &str) -> TaskId {
    TaskId::parse(text).expect("valid task id")
}

fn requirement_id(text: &str) -> RequirementId {
    RequirementId::parse(text).expect("valid requirement id")
}

fn repository(text: &str) -> RepositoryId {
    RepositoryId::parse(text).expect("valid repository id")
}

fn writable(text: &str) -> RepositoryRecord {
    RepositoryRecord {
        id: repository(text),
        revision: Some("aaa1111".to_string()),
        default_branch: Some("main".to_string()),
        credential_reference: Some("app-installation/example".to_string()),
        capabilities: BTreeSet::from([
            Capability::Read,
            Capability::Issues,
            Capability::Branches,
            Capability::Push,
            Capability::PullRequests,
            Capability::Workflows,
        ]),
        languages: Vec::new(),
        build_systems: Vec::new(),
        validation_commands: Vec::new(),
    }
}

/// Three repositories across two owners, as the acceptance criteria require.
fn workspace() -> Workspace {
    let mut workspace = Workspace::new();
    workspace.insert(writable("github.com/InferWeave/autospec"));
    workspace.insert(writable("github.com/InferWeave/autospec-orchestrator"));
    workspace.insert(writable("github.com/OtherOrg/frontend"));
    workspace
}

fn definition() -> Definition {
    let mut definition = Definition::new(
        InitiativeId::parse("INIT-2026-0042").expect("valid initiative id"),
        1,
        "sha256:spec",
    );
    definition.requirements = ["REQ-001", "REQ-002"]
        .into_iter()
        .enumerate()
        .map(|(index, id)| Requirement {
            id: requirement_id(id),
            statement: format!("{id} holds"),
            kind: RequirementKind::Functional,
            acceptance: vec![AcceptanceCriterion {
                id: CriterionId::from_sequence(index as u32 + 1, 3),
                statement: "checked by a command".to_string(),
                objectively_verifiable: true,
                provenance: Provenance::section("Acceptance Criteria"),
            }],
            provenance: Provenance::section("Goals"),
            candidate_repositories: Vec::new(),
            open_questions: Vec::new(),
        })
        .collect();
    definition
}

/// The three-repository, two-organization graph from the specification.
fn graph() -> TaskGraph {
    let mut graph = TaskGraph::new(
        GraphId::parse("DAG-0007").expect("valid graph id"),
        3,
        PlanId::parse("PLAN-ARCH-0003").expect("valid plan id"),
    );
    graph.insert(Task::implementation(
        task_id("TASK-001"),
        repository("github.com/InferWeave/autospec"),
        vec![requirement_id("REQ-001")],
        3,
    ));
    graph.insert(
        Task::implementation(
            task_id("TASK-002"),
            repository("github.com/InferWeave/autospec-orchestrator"),
            vec![requirement_id("REQ-001"), requirement_id("REQ-002")],
            3,
        )
        .depending_on(vec![task_id("TASK-001")]),
    );
    graph.insert(
        Task::implementation(
            task_id("TASK-003"),
            repository("github.com/OtherOrg/frontend"),
            vec![requirement_id("REQ-002")],
            3,
        )
        .depending_on(vec![task_id("TASK-002")]),
    );
    graph
}

fn verify(graph: &mut TaskGraph, id: &str) {
    let task = graph.get_mut(&task_id(id)).expect("task exists");
    task.state = TaskState::Verified;
}

#[test]
fn a_graph_spanning_three_repositories_and_two_owners_validates() {
    let workspace = workspace();

    graph()
        .validate(&definition(), &workspace)
        .expect("the specification's own example graph is valid");

    assert!(workspace.is_multi_organization());
    assert_eq!(workspace.repositories.len(), 3);
}

#[test]
fn cross_repository_dependencies_are_expressed_as_task_ids() {
    let edges = graph().cross_repository_edges();

    assert_eq!(
        edges,
        vec![
            (task_id("TASK-001"), task_id("TASK-002")),
            (task_id("TASK-002"), task_id("TASK-003")),
        ]
    );
}

#[test]
fn a_dependency_on_a_task_outside_the_graph_is_rejected() {
    let mut graph = graph();
    graph
        .get_mut(&task_id("TASK-001"))
        .expect("task exists")
        .depends_on = vec![task_id("TASK-404")];

    let violations = graph
        .validate(&definition(), &workspace())
        .expect_err("dangling dependency");

    assert!(violations.contains(&GraphViolation::MissingDependency {
        task: task_id("TASK-001"),
        dependency: task_id("TASK-404"),
    }));
}

#[test]
fn a_dependency_cycle_is_reported_with_its_path() {
    let mut graph = graph();
    graph
        .get_mut(&task_id("TASK-001"))
        .expect("task exists")
        .depends_on = vec![task_id("TASK-003")];

    let violation = graph.topological_order().expect_err("cycle");

    assert!(violation.message().starts_with("dependency cycle detected"));
}

#[test]
fn a_task_mapping_to_an_unknown_requirement_is_rejected() {
    let mut graph = graph();
    graph
        .get_mut(&task_id("TASK-001"))
        .expect("task exists")
        .satisfies = vec![requirement_id("REQ-999")];

    let violations = graph
        .validate(&definition(), &workspace())
        .expect_err("unknown requirement");

    assert!(violations
        .iter()
        .any(|violation| matches!(violation, GraphViolation::UnknownRequirement { .. })));
}

#[test]
fn a_requirement_with_no_task_is_reported_as_uncovered() {
    let mut graph = graph();
    graph.tasks.remove(&task_id("TASK-003"));
    graph
        .get_mut(&task_id("TASK-002"))
        .expect("task exists")
        .satisfies = vec![requirement_id("REQ-001")];

    let violations = graph
        .validate(&definition(), &workspace())
        .expect_err("uncovered requirement");

    assert!(violations.contains(&GraphViolation::UncoveredRequirement {
        requirement: requirement_id("REQ-002"),
    }));
}

#[test]
fn only_the_root_task_is_ready_before_anything_is_verified() {
    let workspace = workspace();
    let schedule = graph().schedule(SchedulingContext {
        workspace: &workspace,
        now: 0,
    });

    assert_eq!(schedule.ready, vec![task_id("TASK-001")]);
    assert_eq!(
        schedule.block_reason(&task_id("TASK-002")),
        Some(&BlockReason::DependencyUnverified {
            dependency: task_id("TASK-001"),
            state: TaskState::Defined,
        })
    );
    assert_eq!(
        schedule.block_reason(&task_id("TASK-003")),
        Some(&BlockReason::AncestorBlocked {
            ancestor: task_id("TASK-002"),
        })
    );
}

#[test]
fn independent_branches_are_released_concurrently() {
    let mut graph = graph();
    graph.insert(Task::implementation(
        task_id("TASK-004"),
        repository("github.com/OtherOrg/frontend"),
        vec![requirement_id("REQ-002")],
        3,
    ));
    let workspace = workspace();

    let schedule = graph.schedule(SchedulingContext {
        workspace: &workspace,
        now: 0,
    });

    assert_eq!(
        schedule.ready,
        vec![task_id("TASK-001"), task_id("TASK-004")]
    );
    assert_eq!(schedule.ready_repositories(&graph).len(), 2);
}

#[test]
fn a_missing_permission_blocks_only_the_affected_branch() {
    let mut graph = graph();
    // An independent branch in a repository the Initiative can write to.
    graph.insert(Task::implementation(
        task_id("TASK-004"),
        repository("github.com/InferWeave/autospec"),
        vec![requirement_id("REQ-002")],
        3,
    ));
    let mut workspace = workspace();
    // The other organization granted read access only.
    workspace.insert(RepositoryRecord::read_only(repository(
        "github.com/OtherOrg/frontend",
    )));
    verify(&mut graph, "TASK-001");
    verify(&mut graph, "TASK-002");

    let schedule = graph.schedule(SchedulingContext {
        workspace: &workspace,
        now: 0,
    });

    assert!(schedule.is_ready(&task_id("TASK-004")));
    assert!(matches!(
        schedule.block_reason(&task_id("TASK-003")),
        Some(BlockReason::MissingCapability { .. })
    ));
    assert_eq!(schedule.permission_blocked(), vec![&task_id("TASK-003")]);
}

#[test]
fn a_permission_block_propagates_to_descendants_only() {
    let mut graph = graph();
    graph.insert(
        Task::implementation(
            task_id("TASK-005"),
            repository("github.com/InferWeave/autospec"),
            vec![requirement_id("REQ-002")],
            3,
        )
        .depending_on(vec![task_id("TASK-003")]),
    );
    let mut workspace = workspace();
    workspace.insert(RepositoryRecord::read_only(repository(
        "github.com/OtherOrg/frontend",
    )));
    verify(&mut graph, "TASK-001");
    verify(&mut graph, "TASK-002");

    let schedule = graph.schedule(SchedulingContext {
        workspace: &workspace,
        now: 0,
    });

    assert_eq!(
        graph.descendants(&task_id("TASK-003")),
        BTreeSet::from([task_id("TASK-005")])
    );
    assert!(matches!(
        schedule.block_reason(&task_id("TASK-005")),
        Some(BlockReason::AncestorBlocked { .. })
    ));
}

#[test]
fn an_exclusivity_key_serializes_two_otherwise_ready_tasks() {
    let mut graph = graph();
    let mut second = Task::implementation(
        task_id("TASK-004"),
        repository("github.com/InferWeave/autospec"),
        vec![requirement_id("REQ-002")],
        3,
    );
    second.exclusivity = Some("autospec-migrations".to_string());
    graph.insert(second);
    graph
        .get_mut(&task_id("TASK-001"))
        .expect("task exists")
        .exclusivity = Some("autospec-migrations".to_string());
    let workspace = workspace();

    let schedule = graph.schedule(SchedulingContext {
        workspace: &workspace,
        now: 0,
    });

    assert_eq!(schedule.ready, vec![task_id("TASK-001")]);
    assert_eq!(
        schedule.block_reason(&task_id("TASK-004")),
        Some(&BlockReason::ExclusivityHeld {
            key: "autospec-migrations".to_string(),
            holder: task_id("TASK-001"),
        })
    );
}

#[test]
fn concurrency_waves_group_independent_work() {
    let mut graph = graph();
    graph.insert(Task::implementation(
        task_id("TASK-004"),
        repository("github.com/InferWeave/autospec"),
        vec![requirement_id("REQ-002")],
        3,
    ));

    let waves = graph.concurrency_waves().expect("acyclic");

    assert_eq!(waves[0], vec![task_id("TASK-001"), task_id("TASK-004")]);
    assert_eq!(waves[1], vec![task_id("TASK-002")]);
    assert_eq!(waves[2], vec![task_id("TASK-003")]);
}

#[test]
fn integration_work_is_an_explicit_graph_node() {
    let mut graph = graph();
    graph.insert(
        Task::implementation(
            task_id("TASK-010"),
            repository("github.com/InferWeave/autospec"),
            vec![requirement_id("REQ-002")],
            3,
        )
        .as_kind(TaskKind::Integration, AgentRole::TestEngineer)
        .depending_on(vec![task_id("TASK-003")]),
    );

    let integration = graph.integration_tasks();

    assert_eq!(integration.len(), 1);
    assert_eq!(integration[0].id, task_id("TASK-010"));
    assert!(!graph.is_complete());
}

fn affecting(id: &str, surfaces: &[(&str, &[&str])]) -> Task {
    let mut task = Task::implementation(
        task_id(id),
        repository("github.com/InferWeave/autospec"),
        vec![requirement_id("REQ-001")],
        3,
    );
    task.affects = surfaces
        .iter()
        .map(|(path, symbols)| AffectedSurface {
            path: (*path).to_string(),
            symbols: symbols.iter().map(|symbol| (*symbol).to_string()).collect(),
        })
        .collect();
    task
}

fn surface_graph(tasks: Vec<Task>) -> TaskGraph {
    let mut graph = TaskGraph::new(
        GraphId::parse("DAG-0008").expect("valid graph id"),
        1,
        PlanId::parse("PLAN-ARCH-0003").expect("valid plan id"),
    );
    for task in tasks {
        graph.insert(task);
    }
    graph
}

fn schedule_at(graph: &TaskGraph, now: u64) -> Schedule {
    graph.schedule(SchedulingContext {
        workspace: &workspace(),
        now,
    })
}

fn running(mut task: Task, lease: Option<Lease>) -> Task {
    task.state = TaskState::Running;
    task.lease = lease;
    task
}

#[test]
fn two_ready_tasks_sharing_a_file_release_only_the_first_in_order() {
    let graph = surface_graph(vec![
        affecting("TASK-001", &[("src/a.rs", &[])]),
        affecting("TASK-002", &[("src/a.rs", &[])]),
    ]);

    let schedule = schedule_at(&graph, 0);

    assert_eq!(schedule.ready, vec![task_id("TASK-001")]);
    assert_eq!(
        schedule.block_reason(&task_id("TASK-002")),
        Some(&BlockReason::PredictedCollision {
            holder: task_id("TASK-001"),
            shared: vec!["src/a.rs".to_string()],
        })
    );
    assert!(schedule
        .block_reason(&task_id("TASK-002"))
        .expect("blocked")
        .message()
        .contains("TASK-001"));
}

#[test]
fn two_ready_tasks_on_different_files_are_both_released() {
    let graph = surface_graph(vec![
        affecting("TASK-001", &[("src/a.rs", &[])]),
        affecting("TASK-002", &[("src/b.rs", &[])]),
    ]);

    let schedule = schedule_at(&graph, 0);

    assert_eq!(
        schedule.ready,
        vec![task_id("TASK-001"), task_id("TASK-002")]
    );
}

#[test]
fn disjoint_symbols_in_one_file_do_not_block() {
    let graph = surface_graph(vec![
        affecting("TASK-001", &[("src/a.rs", &["one"])]),
        affecting("TASK-002", &[("src/a.rs", &["two"])]),
    ]);

    assert_eq!(schedule_at(&graph, 0).ready.len(), 2);
}

#[test]
fn a_running_task_with_a_live_lease_holds_its_surface() {
    let live = Lease {
        session_name: "pi-1".to_string(),
        expires_at: 100,
    };
    let graph = surface_graph(vec![
        running(affecting("TASK-001", &[("src/a.rs", &[])]), Some(live)),
        affecting("TASK-002", &[("src/a.rs", &[])]),
    ]);

    let schedule = schedule_at(&graph, 99);

    assert!(schedule.ready.is_empty());
    assert!(matches!(
        schedule.block_reason(&task_id("TASK-002")),
        Some(BlockReason::PredictedCollision { holder, .. }) if *holder == task_id("TASK-001")
    ));
}

#[test]
fn a_running_task_with_an_expired_lease_does_not_hold_its_surface() {
    let expired = Lease {
        session_name: "pi-1".to_string(),
        expires_at: 100,
    };
    let graph = surface_graph(vec![
        running(affecting("TASK-001", &[("src/a.rs", &[])]), Some(expired)),
        affecting("TASK-002", &[("src/a.rs", &[])]),
    ]);

    assert_eq!(schedule_at(&graph, 100).ready, vec![task_id("TASK-002")]);
}

#[test]
fn a_running_task_without_a_lease_record_still_holds_its_surface() {
    // `Task::transition_to` clears the lease once a task leaves LEASED, so a
    // RUNNING task normally carries none. It is live, as it is for exclusivity.
    let graph = surface_graph(vec![
        running(affecting("TASK-001", &[("src/a.rs", &[])]), None),
        affecting("TASK-002", &[("src/a.rs", &[])]),
    ]);

    assert!(schedule_at(&graph, 0).ready.is_empty());
}

#[test]
fn a_collision_blocked_task_blocks_its_descendants() {
    let graph = surface_graph(vec![
        affecting("TASK-001", &[("src/a.rs", &[])]),
        affecting("TASK-002", &[("src/a.rs", &[])]),
        affecting("TASK-003", &[]).depending_on(vec![task_id("TASK-002")]),
    ]);

    assert!(matches!(
        schedule_at(&graph, 0).block_reason(&task_id("TASK-003")),
        Some(BlockReason::AncestorBlocked { ancestor }) if *ancestor == task_id("TASK-002")
    ));
}

#[test]
fn exclusivity_is_checked_before_a_predicted_collision() {
    let mut first = affecting("TASK-001", &[("src/a.rs", &[])]);
    let mut second = affecting("TASK-002", &[("src/a.rs", &[])]);
    first.exclusivity = Some("migrations".to_string());
    second.exclusivity = Some("migrations".to_string());
    let graph = surface_graph(vec![first, second]);

    assert!(matches!(
        schedule_at(&graph, 0).block_reason(&task_id("TASK-002")),
        Some(BlockReason::ExclusivityHeld { .. })
    ));
}

#[test]
fn tasks_that_declare_nothing_are_never_blocked_by_prediction() {
    let graph = surface_graph(vec![
        affecting("TASK-001", &[("src/a.rs", &[])]),
        affecting("TASK-002", &[]),
    ]);

    assert_eq!(schedule_at(&graph, 0).ready.len(), 2);
}
