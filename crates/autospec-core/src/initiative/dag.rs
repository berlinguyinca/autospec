//! The canonical cross-repository task DAG and its scheduler.
//!
//! Dependencies are expressed with AutoSpec task ids, so an edge may cross a
//! repository, an owner, or a host without changing shape. Independent
//! branches are released concurrently, and a branch that cannot run — a
//! missing permission, an unverified dependency — blocks only itself and its
//! descendants.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use serde::{Deserialize, Serialize};

use super::definition::Definition;
use super::ids::{GraphId, PlanId, RequirementId, TaskId};
use super::repository::{Capability, RepositoryId, Workspace};
use super::task::{Task, TaskKind, TaskState};

/// A structural problem that makes the graph unexecutable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "violation")]
pub enum GraphViolation {
    /// A task depends on a task that is not in the graph.
    MissingDependency {
        /// The dependent task.
        task: TaskId,
        /// The dependency that does not exist.
        dependency: TaskId,
    },
    /// The dependency edges form a cycle.
    Cycle {
        /// The cycle, starting and ending on the same task.
        cycle: Vec<TaskId>,
    },
    /// A task maps to a requirement the Definition does not contain.
    UnknownRequirement {
        /// The task.
        task: TaskId,
        /// The requirement it claims to satisfy.
        requirement: RequirementId,
    },
    /// A task maps to a statement the Definition declared out of scope.
    NonGoalRequirement {
        /// The task.
        task: TaskId,
        /// The non-goal it claims to satisfy.
        requirement: RequirementId,
    },
    /// A task targets a repository workspace discovery never saw.
    UnknownRepository {
        /// The task.
        task: TaskId,
        /// The repository it targets.
        repository: RepositoryId,
    },
    /// A task is internally inconsistent.
    InvalidTask {
        /// The task.
        task: TaskId,
        /// What is wrong with it.
        problem: String,
    },
    /// An actionable requirement has no task at all.
    UncoveredRequirement {
        /// The requirement no task maps to.
        requirement: RequirementId,
    },
}

impl GraphViolation {
    /// A human-readable explanation.
    pub fn message(&self) -> String {
        match self {
            GraphViolation::MissingDependency { task, dependency } => {
                format!("{task} depends on missing {dependency}")
            }
            GraphViolation::Cycle { cycle } => format!(
                "dependency cycle detected: {}",
                cycle
                    .iter()
                    .map(TaskId::as_str)
                    .collect::<Vec<_>>()
                    .join(" -> ")
            ),
            GraphViolation::UnknownRequirement { task, requirement } => {
                format!("{task} maps to unknown {requirement}")
            }
            GraphViolation::NonGoalRequirement { task, requirement } => {
                format!("{task} maps to non-goal {requirement}")
            }
            GraphViolation::UnknownRepository { task, repository } => {
                format!(
                    "{task} targets undiscovered repository {}",
                    repository.as_str()
                )
            }
            GraphViolation::InvalidTask { task, problem } => format!("{task}: {problem}"),
            GraphViolation::UncoveredRequirement { requirement } => {
                format!("{requirement} has no task in the graph")
            }
        }
    }
}

/// Why a task cannot be released yet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "reason")]
pub enum BlockReason {
    /// A direct dependency is not verified yet.
    DependencyUnverified {
        /// The dependency.
        dependency: TaskId,
        /// The state it is in.
        state: TaskState,
    },
    /// The task's repository does not grant what the task needs.
    MissingCapability {
        /// The repository.
        repository: RepositoryId,
        /// The capabilities the Initiative does not hold there.
        capabilities: BTreeSet<Capability>,
    },
    /// An ancestor is blocked, so this branch cannot advance.
    AncestorBlocked {
        /// The nearest blocked ancestor.
        ancestor: TaskId,
    },
    /// Another live task holds the exclusivity key.
    ExclusivityHeld {
        /// The exclusivity key.
        key: String,
        /// The task holding it.
        holder: TaskId,
    },
}

impl BlockReason {
    /// A human-readable explanation.
    pub fn message(&self) -> String {
        match self {
            BlockReason::DependencyUnverified { dependency, state } => {
                format!("waiting on {dependency} ({state})")
            }
            BlockReason::MissingCapability {
                repository,
                capabilities,
            } => format!(
                "{} does not grant {}",
                repository.as_str(),
                capabilities
                    .iter()
                    .map(Capability::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            BlockReason::AncestorBlocked { ancestor } => format!("ancestor {ancestor} is blocked"),
            BlockReason::ExclusivityHeld { key, holder } => {
                format!("exclusivity key {key} is held by {holder}")
            }
        }
    }

    /// Whether the block is a permission problem rather than a work ordering one.
    pub fn is_permission_block(&self) -> bool {
        matches!(self, BlockReason::MissingCapability { .. })
    }
}

/// What the scheduler decided on one pass.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Schedule {
    /// Tasks that may be leased now, in deterministic order.
    pub ready: Vec<TaskId>,
    /// Tasks that may not, with the first reason found.
    pub blocked: BTreeMap<TaskId, BlockReason>,
}

impl Schedule {
    /// Whether `task` is releasable on this pass.
    pub fn is_ready(&self, task: &TaskId) -> bool {
        self.ready.contains(task)
    }

    /// Why `task` is held back, if it is.
    pub fn block_reason(&self, task: &TaskId) -> Option<&BlockReason> {
        self.blocked.get(task)
    }

    /// The distinct repositories the ready set touches.
    pub fn ready_repositories(&self, graph: &TaskGraph) -> BTreeSet<RepositoryId> {
        self.ready
            .iter()
            .filter_map(|id| graph.get(id))
            .map(|task| task.repository.clone())
            .collect()
    }

    /// Tasks blocked because the Initiative lacks a permission somewhere.
    pub fn permission_blocked(&self) -> Vec<&TaskId> {
        self.blocked
            .iter()
            .filter(|(_, reason)| reason.is_permission_block())
            .map(|(task, _)| task)
            .collect()
    }
}

/// Everything the scheduler needs besides the graph itself.
#[derive(Debug, Clone, Copy)]
pub struct SchedulingContext<'a> {
    /// The repository manifest, which carries the permission records.
    pub workspace: &'a Workspace,
    /// Unix seconds, used to expire leases.
    pub now: u64,
}

/// The canonical executable dependency DAG for one Initiative.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskGraph {
    /// Stable graph identifier.
    pub id: GraphId,
    /// Monotonic version; a replan publishes a new one.
    pub version: u32,
    /// The architecture plan this graph was derived from.
    pub plan: PlanId,
    /// Tasks by identifier.
    #[serde(default)]
    pub tasks: BTreeMap<TaskId, Task>,
}

impl TaskGraph {
    /// An empty graph for `plan`.
    pub fn new(id: GraphId, version: u32, plan: PlanId) -> Self {
        Self {
            id,
            version,
            plan,
            tasks: BTreeMap::new(),
        }
    }

    /// Add or replace a task.
    pub fn insert(&mut self, task: Task) -> &mut Self {
        self.tasks.insert(task.id.clone(), task);
        self
    }

    /// Look up a task.
    pub fn get(&self, id: &TaskId) -> Option<&Task> {
        self.tasks.get(id)
    }

    /// Look up a task for mutation.
    pub fn get_mut(&mut self, id: &TaskId) -> Option<&mut Task> {
        self.tasks.get_mut(id)
    }

    /// Every task, in identifier order.
    pub fn tasks(&self) -> impl Iterator<Item = &Task> {
        self.tasks.values()
    }

    /// Reject a graph that cannot be executed against `definition` and `workspace`.
    pub fn validate(
        &self,
        definition: &Definition,
        workspace: &Workspace,
    ) -> Result<(), Vec<GraphViolation>> {
        let mut violations = Vec::new();
        let known_requirements = definition.requirement_ids();
        let non_goals = definition
            .non_goals()
            .into_iter()
            .map(|requirement| requirement.id.clone())
            .collect::<BTreeSet<_>>();

        for task in self.tasks.values() {
            if let Err(problems) = task.validate() {
                violations.extend(problems.into_iter().map(|problem| {
                    GraphViolation::InvalidTask {
                        task: task.id.clone(),
                        problem,
                    }
                }));
            }
            for dependency in &task.depends_on {
                if !self.tasks.contains_key(dependency) {
                    violations.push(GraphViolation::MissingDependency {
                        task: task.id.clone(),
                        dependency: dependency.clone(),
                    });
                }
            }
            for requirement in &task.satisfies {
                if !known_requirements.contains(requirement) {
                    violations.push(GraphViolation::UnknownRequirement {
                        task: task.id.clone(),
                        requirement: requirement.clone(),
                    });
                } else if non_goals.contains(requirement) {
                    violations.push(GraphViolation::NonGoalRequirement {
                        task: task.id.clone(),
                        requirement: requirement.clone(),
                    });
                }
            }
            if !workspace.contains(&task.repository) {
                violations.push(GraphViolation::UnknownRepository {
                    task: task.id.clone(),
                    repository: task.repository.clone(),
                });
            }
        }

        if violations.is_empty() {
            if let Err(violation) = self.topological_order() {
                violations.push(violation);
            }
        }

        let covered = self.mapped_requirements();
        for requirement in definition.actionable_requirements() {
            if !covered.contains(&requirement.id) {
                violations.push(GraphViolation::UncoveredRequirement {
                    requirement: requirement.id.clone(),
                });
            }
        }

        if violations.is_empty() {
            Ok(())
        } else {
            Err(violations)
        }
    }

    /// Every requirement at least one task maps to.
    pub fn mapped_requirements(&self) -> BTreeSet<RequirementId> {
        self.tasks
            .values()
            .filter(|task| task.state != TaskState::Superseded)
            .flat_map(|task| task.satisfies.iter().cloned())
            .collect()
    }

    /// Tasks in dependency order, or the cycle that prevents one.
    pub fn topological_order(&self) -> Result<Vec<TaskId>, GraphViolation> {
        let mut visiting = BTreeSet::new();
        let mut visited = BTreeSet::new();
        let mut stack = Vec::new();
        let mut order = Vec::new();

        for id in self.tasks.keys() {
            self.visit(id, &mut visiting, &mut visited, &mut stack, &mut order)?;
        }
        Ok(order)
    }

    /// Depth-first visit that records a cycle instead of recursing forever.
    fn visit(
        &self,
        id: &TaskId,
        visiting: &mut BTreeSet<TaskId>,
        visited: &mut BTreeSet<TaskId>,
        stack: &mut Vec<TaskId>,
        order: &mut Vec<TaskId>,
    ) -> Result<(), GraphViolation> {
        if visited.contains(id) {
            return Ok(());
        }
        if visiting.contains(id) {
            let start = stack.iter().position(|entry| entry == id).unwrap_or(0);
            let mut cycle = stack[start..].to_vec();
            cycle.push(id.clone());
            return Err(GraphViolation::Cycle { cycle });
        }

        visiting.insert(id.clone());
        stack.push(id.clone());
        if let Some(task) = self.tasks.get(id) {
            for dependency in &task.depends_on {
                if self.tasks.contains_key(dependency) {
                    self.visit(dependency, visiting, visited, stack, order)?;
                }
            }
        }
        stack.pop();
        visiting.remove(id);
        visited.insert(id.clone());
        order.push(id.clone());
        Ok(())
    }

    /// Dependency edges whose two ends live in different repositories.
    pub fn cross_repository_edges(&self) -> Vec<(TaskId, TaskId)> {
        let mut edges = Vec::new();
        for task in self.tasks.values() {
            for dependency in &task.depends_on {
                let Some(upstream) = self.tasks.get(dependency) else {
                    continue;
                };
                if upstream.repository != task.repository {
                    edges.push((dependency.clone(), task.id.clone()));
                }
            }
        }
        edges
    }

    /// Every task reachable from `id` by following dependents.
    pub fn descendants(&self, id: &TaskId) -> BTreeSet<TaskId> {
        let mut dependents: BTreeMap<&TaskId, Vec<&TaskId>> = BTreeMap::new();
        for task in self.tasks.values() {
            for dependency in &task.depends_on {
                dependents.entry(dependency).or_default().push(&task.id);
            }
        }

        let mut found = BTreeSet::new();
        let mut queue = VecDeque::from([id.clone()]);
        while let Some(current) = queue.pop_front() {
            for dependent in dependents.get(&current).into_iter().flatten() {
                if found.insert((*dependent).clone()) {
                    queue.push_back((*dependent).clone());
                }
            }
        }
        found
    }

    /// Decide which tasks may be released now.
    pub fn schedule(&self, context: SchedulingContext<'_>) -> Schedule {
        let order = self.topological_order().unwrap_or_default();
        let mut schedule = Schedule::default();
        let mut held: BTreeMap<String, TaskId> = BTreeMap::new();

        for task in self.tasks.values() {
            if task.state.is_active() && !task.lease_expired(context.now) {
                if let Some(key) = &task.exclusivity {
                    held.entry(key.clone()).or_insert_with(|| task.id.clone());
                }
            }
        }

        for id in order {
            let Some(task) = self.tasks.get(&id) else {
                continue;
            };
            if !task.state.is_releasable() {
                continue;
            }
            match self.block_reason(task, &schedule, &held, context) {
                Some(reason) => {
                    schedule.blocked.insert(id.clone(), reason);
                }
                None => {
                    if let Some(key) = &task.exclusivity {
                        held.insert(key.clone(), id.clone());
                    }
                    schedule.ready.push(id.clone());
                }
            }
        }

        schedule
    }

    /// The first reason `task` cannot be released, if any.
    fn block_reason(
        &self,
        task: &Task,
        schedule: &Schedule,
        held: &BTreeMap<String, TaskId>,
        context: SchedulingContext<'_>,
    ) -> Option<BlockReason> {
        for dependency in &task.depends_on {
            if schedule.blocked.contains_key(dependency) {
                return Some(BlockReason::AncestorBlocked {
                    ancestor: dependency.clone(),
                });
            }
            let Some(upstream) = self.tasks.get(dependency) else {
                return Some(BlockReason::AncestorBlocked {
                    ancestor: dependency.clone(),
                });
            };
            if !upstream.state.satisfies_dependents() {
                return Some(BlockReason::DependencyUnverified {
                    dependency: dependency.clone(),
                    state: upstream.state,
                });
            }
        }

        let missing = match context.workspace.get(&task.repository) {
            Some(record) => record.missing(&task.required_capabilities),
            None => task.required_capabilities.clone(),
        };
        if !missing.is_empty() {
            return Some(BlockReason::MissingCapability {
                repository: task.repository.clone(),
                capabilities: missing,
            });
        }

        if let Some(key) = &task.exclusivity {
            if let Some(holder) = held.get(key) {
                return Some(BlockReason::ExclusivityHeld {
                    key: key.clone(),
                    holder: holder.clone(),
                });
            }
        }

        None
    }

    /// Tasks grouped into waves that may execute concurrently.
    ///
    /// Wave `n` depends only on waves before it, so every task inside one wave
    /// is safe to run at the same time.
    pub fn concurrency_waves(&self) -> Result<Vec<Vec<TaskId>>, GraphViolation> {
        let order = self.topological_order()?;
        let mut depth: BTreeMap<TaskId, usize> = BTreeMap::new();
        for id in &order {
            let task_depth = self
                .tasks
                .get(id)
                .map(|task| {
                    task.depends_on
                        .iter()
                        .filter_map(|dependency| depth.get(dependency))
                        .map(|parent| parent + 1)
                        .max()
                        .unwrap_or(0)
                })
                .unwrap_or(0);
            depth.insert(id.clone(), task_depth);
        }

        let mut waves: Vec<Vec<TaskId>> = Vec::new();
        for (id, task_depth) in depth {
            if waves.len() <= task_depth {
                waves.resize_with(task_depth + 1, Vec::new);
            }
            waves[task_depth].push(id);
        }
        Ok(waves)
    }

    /// Integration tasks, which must be explicit nodes rather than implied work.
    pub fn integration_tasks(&self) -> Vec<&Task> {
        self.tasks
            .values()
            .filter(|task| task.kind == TaskKind::Integration)
            .collect()
    }

    /// Whether every non-superseded task is verified.
    pub fn is_complete(&self) -> bool {
        self.tasks
            .values()
            .filter(|task| task.state != TaskState::Superseded)
            .all(|task| task.state == TaskState::Verified)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::initiative::definition::{
        AcceptanceCriterion, Definition, Provenance, Requirement, RequirementKind,
    };
    use crate::initiative::ids::{CriterionId, InitiativeId};
    use crate::initiative::repository::RepositoryRecord;
    use crate::initiative::roles::AgentRole;

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
}
