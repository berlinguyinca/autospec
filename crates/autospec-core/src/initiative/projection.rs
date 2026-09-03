//! The GitHub projection.
//!
//! GitHub is a view of the orchestration state, never its source
//! (architectural invariant 6). The projection is derived from canonical state
//! every time, so losing or rebuilding a Project destroys nothing, and a
//! synchronization failure degrades the view without touching the truth.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::dag::{Schedule, TaskGraph};
use super::ids::{InitiativeId, TaskId};
use super::repository::{Capability, RepositoryId, Workspace};
use super::task::{Task, TaskState};
use super::traceability::CoverageMatrix;

/// The GitHub Project an Initiative is projected into.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectTarget {
    /// The organization or user owning the Project.
    pub owner: String,
    /// The Project number.
    pub project_number: u64,
}

/// One task projected as an issue in its owning repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueProjection {
    /// The task this issue represents.
    pub task: TaskId,
    /// The repository the issue lives in.
    pub repository: RepositoryId,
    /// The linked issue number, once one exists.
    #[serde(default)]
    pub issue_number: Option<u64>,
    /// The issue title.
    pub title: String,
    /// Issue body metadata: Initiative, task, requirements, plan, dependencies.
    pub metadata: BTreeMap<String, String>,
}

/// One row in the Project view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectItem {
    /// The task the row represents.
    pub task: TaskId,
    /// Field values shown in the Project.
    pub fields: BTreeMap<String, String>,
}

/// Whether the last synchronization succeeded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "status", content = "detail")]
pub enum SyncStatus {
    /// GitHub reflects canonical state.
    Synced,
    /// GitHub is stale; canonical state is unaffected.
    Degraded(String),
}

/// The complete human-facing projection of an Initiative.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GithubProjection {
    /// The Initiative being projected.
    pub initiative: InitiativeId,
    /// The Project the rows belong to, when one is configured.
    #[serde(default)]
    pub project: Option<ProjectTarget>,
    /// Issue projections by task.
    pub issues: BTreeMap<TaskId, IssueProjection>,
    /// Project rows, for tasks whose repositories permit Project mutation.
    pub items: Vec<ProjectItem>,
    /// Tasks that cannot be projected, with the reason.
    pub unprojectable: BTreeMap<TaskId, String>,
    /// Whether GitHub currently reflects canonical state.
    pub sync_status: SyncStatus,
}

impl GithubProjection {
    /// Derive the projection from canonical state.
    ///
    /// This is the only way a projection is produced, which is what makes a
    /// lost Project recoverable: rebuild it and the rows come back identical.
    pub fn build(
        initiative: &InitiativeId,
        graph: &TaskGraph,
        workspace: &Workspace,
        coverage: &CoverageMatrix,
        schedule: &Schedule,
        project: Option<ProjectTarget>,
    ) -> Self {
        let mut issues = BTreeMap::new();
        let mut items = Vec::new();
        let mut unprojectable = BTreeMap::new();

        for task in graph.tasks() {
            if task.state == TaskState::Superseded {
                continue;
            }
            let record = workspace.get(&task.repository);
            if !record.is_some_and(|record| record.grants(Capability::Issues)) {
                unprojectable.insert(
                    task.id.clone(),
                    format!(
                        "{} does not grant the issues capability",
                        task.repository.as_str()
                    ),
                );
                continue;
            }

            issues.insert(
                task.id.clone(),
                IssueProjection {
                    task: task.id.clone(),
                    repository: task.repository.clone(),
                    issue_number: None,
                    title: format!("{}: {}", task.id, task.summary),
                    metadata: issue_metadata(initiative, task, graph),
                },
            );

            let projectable = project.is_some()
                && record.is_some_and(|record| record.grants(Capability::ProjectMutation));
            if projectable {
                items.push(ProjectItem {
                    task: task.id.clone(),
                    fields: project_fields(initiative, task, coverage, schedule),
                });
            }
        }

        Self {
            initiative: initiative.clone(),
            project,
            issues,
            items,
            unprojectable,
            sync_status: SyncStatus::Synced,
        }
    }

    /// Mark the projection stale without touching canonical state.
    pub fn degrade(&mut self, reason: impl Into<String>) -> &mut Self {
        self.sync_status = SyncStatus::Degraded(reason.into());
        self
    }

    /// Whether the projection currently reflects canonical state.
    pub fn is_synced(&self) -> bool {
        self.sync_status == SyncStatus::Synced
    }

    /// Attach an issue number discovered or created on GitHub.
    pub fn link_issue(&mut self, task: &TaskId, issue_number: u64) -> Result<(), String> {
        let issue = self
            .issues
            .get_mut(task)
            .ok_or_else(|| format!("{task} has no issue projection"))?;
        issue.issue_number = Some(issue_number);
        Ok(())
    }

    /// The distinct owners the projection spans.
    pub fn owner_scopes(&self) -> BTreeSet<String> {
        self.issues
            .values()
            .map(|issue| issue.repository.credential_scope())
            .collect()
    }

    /// Apply a manual GitHub change according to `policy`.
    pub fn reconcile(
        &self,
        observed: &[ObservedIssue],
        policy: ReconciliationPolicy,
    ) -> ReconciliationOutcome {
        let mut actions = Vec::new();
        for change in observed {
            let known = self.issues.contains_key(&change.task);
            let action = match (known, policy) {
                (false, _) => ReconciliationAction::Rejected {
                    task: change.task.clone(),
                    reason: "no canonical task for this issue".to_string(),
                },
                (true, ReconciliationPolicy::Import) => ReconciliationAction::Imported {
                    task: change.task.clone(),
                    fields: change.fields.clone(),
                },
                (true, ReconciliationPolicy::Reject) => ReconciliationAction::Rejected {
                    task: change.task.clone(),
                    reason: "manual GitHub edits are rejected under this policy".to_string(),
                },
                (true, ReconciliationPolicy::ApprovalRequired) => {
                    ReconciliationAction::AwaitingApproval {
                        task: change.task.clone(),
                        fields: change.fields.clone(),
                    }
                }
                (true, ReconciliationPolicy::Drift) => ReconciliationAction::DriftRecorded {
                    task: change.task.clone(),
                    fields: change.fields.clone(),
                },
            };
            actions.push(action);
        }

        let canonical_mutated = policy == ReconciliationPolicy::Import
            && actions
                .iter()
                .any(|action| matches!(action, ReconciliationAction::Imported { .. }));
        ReconciliationOutcome {
            policy,
            actions,
            canonical_mutated,
        }
    }
}

/// Issue body metadata, so a rebuilt issue carries its orchestration identity.
fn issue_metadata(
    initiative: &InitiativeId,
    task: &Task,
    graph: &TaskGraph,
) -> BTreeMap<String, String> {
    BTreeMap::from([
        ("initiative".to_string(), initiative.as_str().to_string()),
        ("task".to_string(), task.id.as_str().to_string()),
        (
            "requirements".to_string(),
            join(task.satisfies.iter().map(|id| id.as_str())),
        ),
        ("plan_version".to_string(), task.plan_version.to_string()),
        (
            "depends_on".to_string(),
            join(task.depends_on.iter().map(TaskId::as_str)),
        ),
        (
            "cross_repository_dependencies".to_string(),
            join(
                graph
                    .cross_repository_edges()
                    .iter()
                    .filter(|(_, downstream)| downstream == &task.id)
                    .map(|(upstream, _)| upstream.as_str()),
            ),
        ),
    ])
}

/// The Project row for one task.
fn project_fields(
    initiative: &InitiativeId,
    task: &Task,
    coverage: &CoverageMatrix,
    schedule: &Schedule,
) -> BTreeMap<String, String> {
    let requirement_coverage = task
        .satisfies
        .iter()
        .map(|requirement| {
            let state = coverage
                .status(requirement)
                .map(|status| status.state.as_str())
                .unwrap_or("defined");
            format!("{requirement}={state}")
        })
        .collect::<Vec<_>>()
        .join(" ");

    BTreeMap::from([
        ("Initiative".to_string(), initiative.as_str().to_string()),
        ("Task ID".to_string(), task.id.as_str().to_string()),
        (
            "Repository".to_string(),
            task.repository.as_str().to_string(),
        ),
        ("Owner".to_string(), task.repository.owner().to_string()),
        ("Stage".to_string(), task.kind.as_str().to_string()),
        ("Status".to_string(), task.state.as_str().to_string()),
        ("Agent role".to_string(), task.role.as_str().to_string()),
        ("Plan version".to_string(), task.plan_version.to_string()),
        ("Requirement coverage".to_string(), requirement_coverage),
        (
            "Blocker".to_string(),
            schedule
                .block_reason(&task.id)
                .map(|reason| reason.message())
                .unwrap_or_default(),
        ),
    ])
}

/// Join identifiers into a stable comma-separated field value.
fn join<'a>(values: impl Iterator<Item = &'a str>) -> String {
    values.collect::<Vec<_>>().join(",")
}

/// A change someone made directly on GitHub.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservedIssue {
    /// The task the issue claims to represent.
    pub task: TaskId,
    /// The repository the issue lives in.
    pub repository: RepositoryId,
    /// The issue number.
    pub issue_number: u64,
    /// The field values observed on GitHub.
    pub fields: BTreeMap<String, String>,
}

/// How manual GitHub changes are handled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReconciliationPolicy {
    /// Adopt the manual change into canonical state.
    Import,
    /// Discard the manual change.
    Reject,
    /// Hold the manual change for a human decision.
    ApprovalRequired,
    /// Keep canonical state and record the divergence.
    Drift,
}

/// What reconciliation did with one observed change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "action")]
pub enum ReconciliationAction {
    /// Adopted into canonical state.
    Imported {
        /// The task.
        task: TaskId,
        /// The adopted field values.
        fields: BTreeMap<String, String>,
    },
    /// Discarded.
    Rejected {
        /// The task.
        task: TaskId,
        /// Why it was discarded.
        reason: String,
    },
    /// Held for a human decision.
    AwaitingApproval {
        /// The task.
        task: TaskId,
        /// The proposed field values.
        fields: BTreeMap<String, String>,
    },
    /// Recorded as divergence, canonical state unchanged.
    DriftRecorded {
        /// The task.
        task: TaskId,
        /// The diverging field values.
        fields: BTreeMap<String, String>,
    },
}

/// The result of reconciling manual GitHub changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReconciliationOutcome {
    /// The policy applied.
    pub policy: ReconciliationPolicy,
    /// What happened to each observed change.
    pub actions: Vec<ReconciliationAction>,
    /// Whether canonical state was changed as a result.
    pub canonical_mutated: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::initiative::dag::SchedulingContext;
    use crate::initiative::definition::{
        AcceptanceCriterion, Definition, Provenance, Requirement, RequirementKind,
    };
    use crate::initiative::ids::{CriterionId, GraphId, PlanId, RequirementId};
    use crate::initiative::plan::{ArchitecturePlan, WorkStream};
    use crate::initiative::repository::RepositoryRecord;
    use crate::initiative::task::Task;

    fn initiative() -> InitiativeId {
        InitiativeId::parse("INIT-2026-0042").expect("valid initiative id")
    }

    fn task_id(text: &str) -> TaskId {
        TaskId::parse(text).expect("valid task id")
    }

    fn requirement_id(text: &str) -> RequirementId {
        RequirementId::parse(text).expect("valid requirement id")
    }

    fn repository(text: &str) -> RepositoryId {
        RepositoryId::parse(text).expect("valid repository id")
    }

    fn record(text: &str, capabilities: BTreeSet<Capability>) -> RepositoryRecord {
        RepositoryRecord {
            id: repository(text),
            revision: Some("aaa1111".to_string()),
            default_branch: Some("main".to_string()),
            credential_reference: None,
            capabilities,
            languages: Vec::new(),
            build_systems: Vec::new(),
            validation_commands: Vec::new(),
        }
    }

    fn full_capabilities() -> BTreeSet<Capability> {
        BTreeSet::from([
            Capability::Read,
            Capability::Issues,
            Capability::Branches,
            Capability::Push,
            Capability::PullRequests,
            Capability::ProjectMutation,
        ])
    }

    fn workspace() -> Workspace {
        let mut workspace = Workspace::new();
        workspace.insert(record(
            "github.com/InferWeave/autospec",
            full_capabilities(),
        ));
        // The second organization granted issues but not Project mutation.
        workspace.insert(record(
            "github.com/OtherOrg/frontend",
            BTreeSet::from([
                Capability::Read,
                Capability::Issues,
                Capability::Branches,
                Capability::Push,
                Capability::PullRequests,
            ]),
        ));
        workspace
    }

    fn definition() -> Definition {
        let mut definition = Definition::new(initiative(), 1, "sha256:spec");
        definition.requirements = ["REQ-001", "REQ-002"]
            .into_iter()
            .enumerate()
            .map(|(index, id)| Requirement {
                id: requirement_id(id),
                statement: format!("{id} holds"),
                kind: RequirementKind::Functional,
                acceptance: vec![AcceptanceCriterion {
                    id: CriterionId::from_sequence(index as u32 + 1, 3),
                    statement: "checked".to_string(),
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

    fn plan() -> ArchitecturePlan {
        let mut plan = ArchitecturePlan::new(
            PlanId::parse("PLAN-ARCH-0003").expect("valid plan id"),
            initiative(),
            1,
            &definition(),
            &workspace(),
        );
        plan.work_streams = vec![WorkStream {
            id: "WS-01".to_string(),
            summary: "projection".to_string(),
            satisfies: vec![requirement_id("REQ-001"), requirement_id("REQ-002")],
            repositories: vec![repository("github.com/InferWeave/autospec")],
        }];
        plan
    }

    fn graph() -> TaskGraph {
        let mut graph = TaskGraph::new(
            GraphId::parse("DAG-0007").expect("valid graph id"),
            1,
            PlanId::parse("PLAN-ARCH-0003").expect("valid plan id"),
        );
        graph.insert(Task::implementation(
            task_id("TASK-0001"),
            repository("github.com/InferWeave/autospec"),
            vec![requirement_id("REQ-001")],
            1,
        ));
        graph.insert(
            Task::implementation(
                task_id("TASK-0002"),
                repository("github.com/OtherOrg/frontend"),
                vec![requirement_id("REQ-002")],
                1,
            )
            .depending_on(vec![task_id("TASK-0001")]),
        );
        graph
    }

    fn projection(workspace: &Workspace) -> GithubProjection {
        let graph = graph();
        let coverage = CoverageMatrix::build(&definition(), &plan(), &graph, &[], &[]);
        let schedule = graph.schedule(SchedulingContext {
            workspace,
            now: 0,
        });
        GithubProjection::build(
            &initiative(),
            &graph,
            workspace,
            &coverage,
            &schedule,
            Some(ProjectTarget {
                owner: "InferWeave".to_string(),
                project_number: 12,
            }),
        )
    }

    #[test]
    fn every_task_is_projected_into_an_issue_in_its_own_repository() {
        let projection = projection(&workspace());

        assert_eq!(projection.issues.len(), 2);
        assert_eq!(
            projection.issues[&task_id("TASK-0002")].repository.owner(),
            "OtherOrg"
        );
        assert_eq!(projection.owner_scopes().len(), 2);
    }

    #[test]
    fn issue_metadata_carries_the_orchestration_identity() {
        let projection = projection(&workspace());

        let metadata = &projection.issues[&task_id("TASK-0002")].metadata;

        assert_eq!(metadata["initiative"], "INIT-2026-0042");
        assert_eq!(metadata["task"], "TASK-0002");
        assert_eq!(metadata["requirements"], "REQ-002");
        assert_eq!(metadata["depends_on"], "TASK-0001");
        assert_eq!(metadata["cross_repository_dependencies"], "TASK-0001");
    }

    #[test]
    fn a_project_row_exists_only_where_project_mutation_is_permitted() {
        let projection = projection(&workspace());

        assert_eq!(projection.items.len(), 1);
        assert_eq!(projection.items[0].task, task_id("TASK-0001"));
        assert_eq!(projection.items[0].fields["Owner"], "InferWeave");
        assert_eq!(projection.items[0].fields["Status"], "DEFINED");
    }

    #[test]
    fn a_repository_without_the_issues_capability_is_reported_as_unprojectable() {
        let mut workspace = workspace();
        workspace.insert(RepositoryRecord::read_only(repository(
            "github.com/OtherOrg/frontend",
        )));

        let projection = projection(&workspace);

        assert_eq!(projection.issues.len(), 1);
        assert!(projection.unprojectable[&task_id("TASK-0002")]
            .contains("does not grant the issues capability"));
    }

    #[test]
    fn a_lost_project_rebuilds_identically_from_canonical_state() {
        let workspace = workspace();
        let mut original = projection(&workspace);
        original.link_issue(&task_id("TASK-0001"), 412).expect("linked");

        let rebuilt = projection(&workspace);

        assert_eq!(rebuilt.items, original.items);
        assert_eq!(
            rebuilt.issues[&task_id("TASK-0001")].metadata,
            original.issues[&task_id("TASK-0001")].metadata
        );
        // Only the GitHub-side issue number is lost, and it is not canonical.
        assert_eq!(rebuilt.issues[&task_id("TASK-0001")].issue_number, None);
    }

    #[test]
    fn a_synchronization_failure_degrades_the_view_without_touching_the_graph() {
        let workspace = workspace();
        let graph = graph();
        let mut projection = projection(&workspace);

        projection.degrade("GitHub returned 502 for the Project mutation");

        assert!(!projection.is_synced());
        assert_eq!(graph.get(&task_id("TASK-0001")).map(|task| task.state), Some(TaskState::Defined));
        assert!(graph.validate(&definition(), &workspace).is_ok());
    }

    #[test]
    fn manual_edits_are_rejected_or_imported_according_to_the_policy() {
        let projection = projection(&workspace());
        let observed = vec![ObservedIssue {
            task: task_id("TASK-0001"),
            repository: repository("github.com/InferWeave/autospec"),
            issue_number: 412,
            fields: BTreeMap::from([("Status".to_string(), "VERIFIED".to_string())]),
        }];

        let rejected = projection.reconcile(&observed, ReconciliationPolicy::Reject);
        let imported = projection.reconcile(&observed, ReconciliationPolicy::Import);
        let drifted = projection.reconcile(&observed, ReconciliationPolicy::Drift);
        let approval = projection.reconcile(&observed, ReconciliationPolicy::ApprovalRequired);

        assert!(!rejected.canonical_mutated);
        assert!(imported.canonical_mutated);
        assert!(!drifted.canonical_mutated);
        assert!(!approval.canonical_mutated);
        assert!(matches!(
            drifted.actions[0],
            ReconciliationAction::DriftRecorded { .. }
        ));
        assert!(matches!(
            approval.actions[0],
            ReconciliationAction::AwaitingApproval { .. }
        ));
    }

    #[test]
    fn an_issue_with_no_canonical_task_is_always_rejected() {
        let projection = projection(&workspace());
        let observed = vec![ObservedIssue {
            task: task_id("TASK-0999"),
            repository: repository("github.com/InferWeave/autospec"),
            issue_number: 999,
            fields: BTreeMap::new(),
        }];

        let outcome = projection.reconcile(&observed, ReconciliationPolicy::Import);

        assert!(!outcome.canonical_mutated);
        assert!(matches!(
            outcome.actions[0],
            ReconciliationAction::Rejected { .. }
        ));
    }
}
