//! Versioned architecture plans and the replanning flow.
//!
//! A plan owns HOW the Initiative will satisfy the Definition. Plans are
//! replaceable; requirements are not. Replanning therefore records what
//! changed about the repositories, never about the requirements.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::definition::{Definition, DefinitionChange};
use super::ids::{InitiativeId, PlanId, RequirementId, TaskId};
use super::repository::{RepositoryId, Workspace};
use super::task::TaskState;

/// One stream of work in an architecture plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkStream {
    /// A plan-local identifier, e.g. `WS-01`.
    pub id: String,
    /// What the stream does.
    pub summary: String,
    /// Requirements the stream serves; never empty.
    pub satisfies: Vec<RequirementId>,
    /// Repositories the stream touches.
    pub repositories: Vec<RepositoryId>,
}

/// A contract two repositories must agree on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrossRepositoryContract {
    /// A plan-local identifier, e.g. `CONTRACT-01`.
    pub id: String,
    /// The repository that publishes the contract.
    pub producer: RepositoryId,
    /// Repositories that depend on it.
    pub consumers: Vec<RepositoryId>,
    /// What the contract covers: an API, a schema, an SDK surface.
    pub surface: String,
    /// How compatibility is preserved during rollout.
    pub compatibility: String,
}

/// Whether a plan is the current one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
    /// The plan the Initiative is executing.
    Active,
    /// Replaced by a later version; kept immutable and queryable.
    Superseded,
}

/// A repository-aware, versioned implementation strategy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchitecturePlan {
    /// Stable plan identifier.
    pub id: PlanId,
    /// The Initiative the plan belongs to.
    pub initiative: InitiativeId,
    /// Monotonic plan version, independent of the Definition version.
    pub version: u32,
    /// The Definition version the plan was generated against.
    pub definition_version: u32,
    /// The repository revisions the plan was generated against.
    #[serde(default)]
    pub repository_snapshot: BTreeMap<RepositoryId, String>,
    /// Work streams, each mapped to requirements.
    #[serde(default)]
    pub work_streams: Vec<WorkStream>,
    /// Cross-repository contracts the plan introduces or changes.
    #[serde(default)]
    pub contracts: Vec<CrossRepositoryContract>,
    /// Assumptions the plan depends on; a broken one triggers replanning.
    #[serde(default)]
    pub assumptions: Vec<String>,
    /// Risks the plan accepts.
    #[serde(default)]
    pub risks: Vec<String>,
    /// Current status.
    pub status: PlanStatus,
    /// The plan that replaced this one.
    #[serde(default)]
    pub superseded_by: Option<PlanId>,
}

impl ArchitecturePlan {
    /// An active plan generated against `definition` and `workspace`.
    pub fn new(
        id: PlanId,
        initiative: InitiativeId,
        version: u32,
        definition: &Definition,
        workspace: &Workspace,
    ) -> Self {
        Self {
            id,
            initiative,
            version,
            definition_version: definition.version,
            repository_snapshot: workspace.revision_snapshot(),
            work_streams: Vec::new(),
            contracts: Vec::new(),
            assumptions: Vec::new(),
            risks: Vec::new(),
            status: PlanStatus::Active,
            superseded_by: None,
        }
    }

    /// Requirements at least one work stream serves.
    pub fn covered_requirements(&self) -> BTreeSet<RequirementId> {
        self.work_streams
            .iter()
            .flat_map(|stream| stream.satisfies.iter().cloned())
            .collect()
    }

    /// Repositories the plan touches.
    pub fn touched_repositories(&self) -> BTreeSet<RepositoryId> {
        self.work_streams
            .iter()
            .flat_map(|stream| stream.repositories.iter().cloned())
            .collect()
    }

    /// Reject a plan that cannot be executed against `definition` and `workspace`.
    pub fn validate(
        &self,
        definition: &Definition,
        workspace: &Workspace,
    ) -> Result<(), Vec<String>> {
        let mut problems = Vec::new();
        if self.version == 0 {
            problems.push("plan versions start at 1".to_string());
        }
        if self.definition_version != definition.version {
            problems.push(format!(
                "{} was generated against definition v{} but the current definition is v{}",
                self.id, self.definition_version, definition.version
            ));
        }

        let known = definition.requirement_ids();
        for stream in &self.work_streams {
            if stream.satisfies.is_empty() {
                problems.push(format!(
                    "work stream {} maps to no requirement",
                    stream.id
                ));
            }
            for requirement in &stream.satisfies {
                if !known.contains(requirement) {
                    problems.push(format!(
                        "work stream {} maps to unknown {requirement}",
                        stream.id
                    ));
                }
            }
            for repository in &stream.repositories {
                if !workspace.contains(repository) {
                    problems.push(format!(
                        "work stream {} touches undiscovered repository {}",
                        stream.id,
                        repository.as_str()
                    ));
                }
            }
        }

        // Every plan records the repository revisions it was generated from.
        for repository in self.touched_repositories() {
            if !self.repository_snapshot.contains_key(&repository) {
                problems.push(format!(
                    "{} records no revision for {}",
                    self.id,
                    repository.as_str()
                ));
            }
        }

        for contract in &self.contracts {
            if contract.consumers.is_empty() {
                problems.push(format!("contract {} has no consumer", contract.id));
            }
            if contract.consumers.contains(&contract.producer) {
                problems.push(format!(
                    "contract {} lists its producer as a consumer",
                    contract.id
                ));
            }
        }

        for requirement in definition.actionable_requirements() {
            if !self.covered_requirements().contains(&requirement.id) {
                problems.push(format!("{} is not covered by any work stream", requirement.id));
            }
        }

        if problems.is_empty() {
            Ok(())
        } else {
            Err(problems)
        }
    }
}

/// Why a plan was replaced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplanReason {
    /// A repository moved under the plan.
    RepositoryDrift,
    /// A plan assumption turned out to be false.
    BrokenAssumption,
    /// Implementation found an architectural contradiction and escalated.
    ImplementationEscalation,
    /// A permission the plan assumed is not available.
    MissingPermission,
    /// Tests, review, or integration failed in a way a retry cannot fix.
    EvidenceFailure,
}

impl ReplanReason {
    /// The stable wire name.
    pub fn as_str(&self) -> &'static str {
        match self {
            ReplanReason::RepositoryDrift => "repository_drift",
            ReplanReason::BrokenAssumption => "broken_assumption",
            ReplanReason::ImplementationEscalation => "implementation_escalation",
            ReplanReason::MissingPermission => "missing_permission",
            ReplanReason::EvidenceFailure => "evidence_failure",
        }
    }
}

/// Why a replan was refused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReplanRefusal {
    /// The new plan silently changed the requirements.
    RequirementsChanged(DefinitionChange),
    /// The new plan is not a later version of the old one.
    NonMonotonicVersion {
        /// The version being replaced.
        previous: u32,
        /// The version offered.
        next: u32,
    },
    /// The plan being replaced is already superseded.
    AlreadySuperseded(PlanId),
}

impl ReplanRefusal {
    /// A human-readable explanation.
    pub fn message(&self) -> String {
        match self {
            ReplanRefusal::RequirementsChanged(change) => format!(
                "replanning may not change requirements (added {}, removed {}, modified {}); use change control",
                change.added.len(),
                change.removed.len(),
                change.modified.len()
            ),
            ReplanRefusal::NonMonotonicVersion { previous, next } => {
                format!("a replan must publish a later plan version: v{previous} -> v{next}")
            }
            ReplanRefusal::AlreadySuperseded(plan) => {
                format!("{plan} is already superseded")
            }
        }
    }
}

/// The audit record of one replan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplanRecord {
    /// Why the plan was replaced.
    pub reason: ReplanReason,
    /// The plan version being replaced.
    pub from_version: u32,
    /// The plan version replacing it.
    pub to_version: u32,
    /// The plan being replaced.
    pub from_plan: PlanId,
    /// The plan replacing it.
    pub to_plan: PlanId,
    /// Assumptions that changed.
    pub changed_assumptions: Vec<String>,
    /// Tasks the new plan supersedes.
    pub superseded_tasks: Vec<TaskId>,
    /// Completed work the new plan keeps.
    pub preserved_tasks: Vec<TaskId>,
    /// Tasks that must be recomputed.
    pub impacted_tasks: Vec<TaskId>,
    /// Proof the requirements did not move.
    pub definition_change: DefinitionChange,
}

impl ReplanRecord {
    /// Whether the replan left the requirements untouched.
    pub fn preserves_requirements(&self) -> bool {
        self.definition_change.is_empty()
    }
}

/// Replace `previous` with `next`, preserving verified work.
///
/// The graph is consulted, not mutated: the caller applies the record so the
/// superseded graph version stays immutable and queryable.
pub fn replan(
    previous: &ArchitecturePlan,
    next: &ArchitecturePlan,
    previous_definition: &Definition,
    next_definition: &Definition,
    graph: &super::dag::TaskGraph,
    reason: ReplanReason,
) -> Result<ReplanRecord, ReplanRefusal> {
    if previous.status == PlanStatus::Superseded {
        return Err(ReplanRefusal::AlreadySuperseded(previous.id.clone()));
    }
    if next.version <= previous.version {
        return Err(ReplanRefusal::NonMonotonicVersion {
            previous: previous.version,
            next: next.version,
        });
    }
    let definition_change = DefinitionChange::between(previous_definition, next_definition);
    if !definition_change.is_empty() {
        return Err(ReplanRefusal::RequirementsChanged(definition_change));
    }

    let mut superseded_tasks = Vec::new();
    let mut preserved_tasks = Vec::new();
    let mut impacted_tasks = Vec::new();
    for task in graph.tasks() {
        if task.state == TaskState::Verified {
            preserved_tasks.push(task.id.clone());
            continue;
        }
        if task.state == TaskState::Superseded {
            continue;
        }
        superseded_tasks.push(task.id.clone());
        impacted_tasks.push(task.id.clone());
    }

    // Only the region downstream of superseded work needs recomputation.
    let downstream = superseded_tasks
        .iter()
        .flat_map(|task| graph.descendants(task))
        .collect::<BTreeSet<_>>();
    for task in downstream {
        if !impacted_tasks.contains(&task) {
            impacted_tasks.push(task);
        }
    }
    impacted_tasks.sort();

    let changed_assumptions = next
        .assumptions
        .iter()
        .filter(|assumption| !previous.assumptions.contains(assumption))
        .cloned()
        .collect();

    Ok(ReplanRecord {
        reason,
        from_version: previous.version,
        to_version: next.version,
        from_plan: previous.id.clone(),
        to_plan: next.id.clone(),
        changed_assumptions,
        superseded_tasks,
        preserved_tasks,
        impacted_tasks,
        definition_change,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::initiative::dag::TaskGraph;
    use crate::initiative::definition::{
        AcceptanceCriterion, Provenance, Requirement, RequirementKind,
    };
    use crate::initiative::ids::{CriterionId, GraphId};
    use crate::initiative::repository::{Capability, RepositoryRecord};
    use crate::initiative::task::Task;
    use std::collections::BTreeSet;

    fn initiative() -> InitiativeId {
        InitiativeId::parse("INIT-2026-0042").expect("valid initiative id")
    }

    fn plan_id(text: &str) -> PlanId {
        PlanId::parse(text).expect("valid plan id")
    }

    fn requirement_id(text: &str) -> RequirementId {
        RequirementId::parse(text).expect("valid requirement id")
    }

    fn repository(text: &str) -> RepositoryId {
        RepositoryId::parse(text).expect("valid repository id")
    }

    fn workspace() -> Workspace {
        let mut workspace = Workspace::new();
        for (index, name) in ["autospec", "gateway"].into_iter().enumerate() {
            workspace.insert(RepositoryRecord {
                id: repository(&format!("github.com/InferWeave/{name}")),
                revision: Some(format!("rev{index}")),
                default_branch: Some("main".to_string()),
                credential_reference: None,
                capabilities: BTreeSet::from([
                    Capability::Read,
                    Capability::Branches,
                    Capability::Push,
                    Capability::PullRequests,
                ]),
                languages: Vec::new(),
                build_systems: Vec::new(),
                validation_commands: Vec::new(),
            });
        }
        workspace
    }

    fn definition(version: u32) -> Definition {
        let mut definition = Definition::new(initiative(), version, "sha256:spec");
        definition.requirements = vec![Requirement {
            id: requirement_id("REQ-001"),
            statement: "REQ-001 holds".to_string(),
            kind: RequirementKind::Functional,
            acceptance: vec![AcceptanceCriterion {
                id: CriterionId::from_sequence(1, 3),
                statement: "checked by a command".to_string(),
                objectively_verifiable: true,
                provenance: Provenance::section("Acceptance Criteria"),
            }],
            provenance: Provenance::section("Goals"),
            candidate_repositories: Vec::new(),
            open_questions: Vec::new(),
        }];
        definition
    }

    fn plan(id: &str, version: u32) -> ArchitecturePlan {
        let definition = definition(1);
        let mut plan = ArchitecturePlan::new(
            plan_id(id),
            initiative(),
            version,
            &definition,
            &workspace(),
        );
        plan.work_streams = vec![WorkStream {
            id: "WS-01".to_string(),
            summary: "normalize the definition".to_string(),
            satisfies: vec![requirement_id("REQ-001")],
            repositories: vec![repository("github.com/InferWeave/autospec")],
        }];
        plan
    }

    fn graph() -> TaskGraph {
        let mut graph = TaskGraph::new(
            GraphId::parse("DAG-0007").expect("valid graph id"),
            1,
            plan_id("PLAN-ARCH-0003"),
        );
        graph.insert(Task::implementation(
            TaskId::parse("TASK-0001").expect("valid task id"),
            repository("github.com/InferWeave/autospec"),
            vec![requirement_id("REQ-001")],
            1,
        ));
        graph.insert(
            Task::implementation(
                TaskId::parse("TASK-0002").expect("valid task id"),
                repository("github.com/InferWeave/gateway"),
                vec![requirement_id("REQ-001")],
                1,
            )
            .depending_on(vec![TaskId::parse("TASK-0001").expect("valid task id")]),
        );
        graph
    }

    #[test]
    fn a_plan_is_versioned_independently_of_the_definition() {
        let definition = definition(1);
        let first = plan("PLAN-ARCH-0003", 1);
        let second = plan("PLAN-ARCH-0004", 2);

        assert_eq!(first.definition_version, second.definition_version);
        assert_ne!(first.version, second.version);
        first
            .validate(&definition, &workspace())
            .expect("v1 is valid");
        second
            .validate(&definition, &workspace())
            .expect("v2 is valid against the same definition");
    }

    #[test]
    fn a_plan_must_record_the_revision_of_every_repository_it_touches() {
        let mut plan = plan("PLAN-ARCH-0003", 1);
        plan.repository_snapshot.clear();

        let problems = plan
            .validate(&definition(1), &workspace())
            .expect_err("a plan without revisions is rejected");

        assert!(problems.iter().any(|problem| problem.contains("records no revision")));
    }

    #[test]
    fn every_work_stream_must_map_to_a_requirement() {
        let mut plan = plan("PLAN-ARCH-0003", 1);
        plan.work_streams[0].satisfies.clear();

        let problems = plan
            .validate(&definition(1), &workspace())
            .expect_err("unmapped work streams are rejected");

        assert!(problems.iter().any(|problem| problem.contains("maps to no requirement")));
    }

    #[test]
    fn a_plan_generated_against_an_older_definition_is_rejected() {
        let plan = plan("PLAN-ARCH-0003", 1);

        let problems = plan
            .validate(&definition(2), &workspace())
            .expect_err("stale definition version");

        assert!(problems[0].contains("definition v1"), "{problems:?}");
    }

    #[test]
    fn replanning_after_repository_drift_preserves_verified_work() {
        let mut graph = graph();
        graph
            .get_mut(&TaskId::parse("TASK-0001").expect("valid task id"))
            .expect("task exists")
            .state = TaskState::Verified;

        let record = replan(
            &plan("PLAN-ARCH-0003", 1),
            &plan("PLAN-ARCH-0004", 2),
            &definition(1),
            &definition(1),
            &graph,
            ReplanReason::RepositoryDrift,
        )
        .expect("a drift replan is allowed");

        assert!(record.preserves_requirements());
        assert_eq!(
            record.preserved_tasks,
            vec![TaskId::parse("TASK-0001").expect("valid task id")]
        );
        assert_eq!(
            record.superseded_tasks,
            vec![TaskId::parse("TASK-0002").expect("valid task id")]
        );
        assert_eq!(record.reason.as_str(), "repository_drift");
    }

    #[test]
    fn replanning_recomputes_only_the_impacted_region() {
        let mut graph = graph();
        graph.insert(
            Task::implementation(
                TaskId::parse("TASK-0003").expect("valid task id"),
                repository("github.com/InferWeave/gateway"),
                vec![requirement_id("REQ-001")],
                1,
            )
            .depending_on(vec![TaskId::parse("TASK-0002").expect("valid task id")]),
        );
        graph
            .get_mut(&TaskId::parse("TASK-0001").expect("valid task id"))
            .expect("task exists")
            .state = TaskState::Verified;

        let record = replan(
            &plan("PLAN-ARCH-0003", 1),
            &plan("PLAN-ARCH-0004", 2),
            &definition(1),
            &definition(1),
            &graph,
            ReplanReason::BrokenAssumption,
        )
        .expect("replan allowed");

        assert!(!record.impacted_tasks.contains(&TaskId::parse("TASK-0001").expect("valid")));
        assert_eq!(record.impacted_tasks.len(), 2);
    }

    #[test]
    fn a_replan_that_would_change_a_requirement_is_refused() {
        let mut changed = definition(1);
        changed.requirements[0].statement = "REQ-001 holds differently".to_string();

        let refusal = replan(
            &plan("PLAN-ARCH-0003", 1),
            &plan("PLAN-ARCH-0004", 2),
            &definition(1),
            &changed,
            &graph(),
            ReplanReason::BrokenAssumption,
        )
        .expect_err("requirements may not move during a replan");

        assert!(matches!(refusal, ReplanRefusal::RequirementsChanged(_)));
        assert!(refusal.message().contains("change control"));
    }

    #[test]
    fn a_replan_must_publish_a_later_plan_version() {
        let refusal = replan(
            &plan("PLAN-ARCH-0004", 2),
            &plan("PLAN-ARCH-0003", 1),
            &definition(1),
            &definition(1),
            &graph(),
            ReplanReason::BrokenAssumption,
        )
        .expect_err("versions must move forward");

        assert!(matches!(refusal, ReplanRefusal::NonMonotonicVersion { .. }));
    }

    #[test]
    fn an_already_superseded_plan_may_not_be_replanned_again() {
        let mut previous = plan("PLAN-ARCH-0003", 1);
        previous.status = PlanStatus::Superseded;

        let refusal = replan(
            &previous,
            &plan("PLAN-ARCH-0004", 2),
            &definition(1),
            &definition(1),
            &graph(),
            ReplanReason::BrokenAssumption,
        )
        .expect_err("superseded plans are immutable");

        assert!(matches!(refusal, ReplanRefusal::AlreadySuperseded(_)));
    }
}
