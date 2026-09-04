//! AutoSpec Initiatives: the cross-repository planning and orchestration model.
//!
//! An Initiative is the top-level coordination unit. It owns a Specification,
//! its normalized Definition, a discovered multi-repository workspace, a
//! versioned architecture plan, an executable task DAG, the evidence produced
//! against it, and the GitHub projection of all of that. It deliberately does
//! not belong to a repository or an organization.

pub mod dag;
pub mod definition;
pub mod dispatch;
pub mod ids;
pub mod plan;
pub mod projection;
pub mod repository;
pub mod roles;
pub mod routing;
pub mod store;
pub mod task;
pub mod traceability;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use dag::{Schedule, SchedulingContext, TaskGraph};
use definition::Definition;
use ids::{GraphId, InitiativeId, PlanId};
use plan::ArchitecturePlan;
use projection::ProjectTarget;
use repository::{RepositoryId, Workspace};
use task::TaskState;
use traceability::{CompletionGate, CoverageMatrix, CoverageState};

/// Where an Initiative has got to in the canonical lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InitiativeStage {
    /// A Specification exists.
    Specified,
    /// The Specification has been normalized into a Definition.
    Defined,
    /// The workspace has been discovered.
    Discovered,
    /// An architecture plan exists.
    Planned,
    /// A task graph exists.
    Scheduled,
    /// Tasks are running.
    Executing,
    /// Implementation is done; integration gates are running.
    Integrating,
    /// Integration passed; final verification is running.
    Verifying,
    /// Every requirement is verified or explicitly waived.
    Complete,
}

impl InitiativeStage {
    /// The stable wire name.
    pub fn as_str(&self) -> &'static str {
        match self {
            InitiativeStage::Specified => "specified",
            InitiativeStage::Defined => "defined",
            InitiativeStage::Discovered => "discovered",
            InitiativeStage::Planned => "planned",
            InitiativeStage::Scheduled => "scheduled",
            InitiativeStage::Executing => "executing",
            InitiativeStage::Integrating => "integrating",
            InitiativeStage::Verifying => "verifying",
            InitiativeStage::Complete => "complete",
        }
    }
}

/// The Initiative record itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Initiative {
    /// Stable identifier.
    pub id: InitiativeId,
    /// A human-readable slug.
    pub slug: String,
    /// Path to the Specification, relative to the artifact registry.
    pub spec: String,
    /// The Definition version currently in force.
    pub definition_version: u32,
    /// Repositories the Initiative spans, across any number of owners.
    #[serde(default)]
    pub repositories: Vec<RepositoryId>,
    /// The active architecture plan.
    #[serde(default)]
    pub architecture_plan: Option<PlanId>,
    /// The active task graph.
    #[serde(default)]
    pub task_graph: Option<GraphId>,
    /// GitHub Projects the Initiative is projected into.
    #[serde(default)]
    pub github_projects: Vec<ProjectTarget>,
}

impl Initiative {
    /// A newly opened Initiative with no plan yet.
    pub fn new(id: InitiativeId, slug: impl Into<String>, spec: impl Into<String>) -> Self {
        Self {
            id,
            slug: slug.into(),
            spec: spec.into(),
            definition_version: 0,
            repositories: Vec::new(),
            architecture_plan: None,
            task_graph: None,
            github_projects: Vec::new(),
        }
    }

    /// Reject an Initiative record that contradicts itself.
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut problems = Vec::new();
        if self.slug.trim().is_empty() {
            problems.push("an initiative needs a slug".to_string());
        }
        if self.spec.trim().is_empty() {
            problems.push("an initiative needs a specification".to_string());
        }
        if self.task_graph.is_some() && self.architecture_plan.is_none() {
            problems.push("a task graph requires an architecture plan".to_string());
        }
        if problems.is_empty() {
            Ok(())
        } else {
            Err(problems)
        }
    }
}

/// A rendered snapshot of one Initiative.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InitiativeStatus {
    /// The Initiative.
    pub initiative: InitiativeId,
    /// Where it has got to.
    pub stage: InitiativeStage,
    /// How many repositories it spans.
    pub repository_count: usize,
    /// How many distinct `host/owner` scopes it spans.
    pub owner_scope_count: usize,
    /// How many tasks sit in each lifecycle state.
    pub task_states: BTreeMap<String, usize>,
    /// How many requirements sit in each coverage state.
    pub requirement_states: BTreeMap<String, usize>,
    /// Tasks releasable right now.
    pub ready_tasks: usize,
    /// Tasks held back, with the reason.
    pub blocked_tasks: BTreeMap<String, String>,
    /// Requirements that are not objectively verifiable yet.
    pub definition_gaps: BTreeMap<String, String>,
    /// Whether the Initiative may complete.
    pub completion: CompletionGate,
}

/// Assemble the snapshot the dashboard and CLI render.
pub fn status(
    initiative: &Initiative,
    definition: &Definition,
    plan: &ArchitecturePlan,
    graph: &TaskGraph,
    workspace: &Workspace,
    coverage: &CoverageMatrix,
    now: u64,
) -> InitiativeStatus {
    let schedule = graph.schedule(SchedulingContext { workspace, now });
    let completion = coverage.completion_gate();

    let mut task_states: BTreeMap<String, usize> = BTreeMap::new();
    for task in graph.tasks() {
        *task_states
            .entry(task.state.as_str().to_string())
            .or_insert(0) += 1;
    }

    let requirement_states = coverage
        .summary()
        .into_iter()
        .map(|(state, count)| (state.as_str().to_string(), count))
        .collect::<BTreeMap<_, _>>();

    InitiativeStatus {
        initiative: initiative.id.clone(),
        stage: stage(initiative, graph, coverage, plan, &completion),
        repository_count: workspace.repositories.len(),
        owner_scope_count: workspace.owner_scopes().len(),
        task_states,
        requirement_states,
        ready_tasks: schedule.ready.len(),
        blocked_tasks: blocked_summary(&schedule),
        definition_gaps: BTreeMap::new(),
        completion,
    }
    .with_definition_gaps(definition)
}

impl InitiativeStatus {
    /// Fold the Definition's own gaps into the snapshot.
    ///
    /// A requirement that is not yet objectively verifiable holds the whole
    /// Initiative at `defined`: planning against it would be planning against
    /// something no verifier can check.
    fn with_definition_gaps(mut self, definition: &Definition) -> Self {
        for gap in definition.gaps() {
            self.definition_gaps.insert(
                gap.requirement().as_str().to_string(),
                "no objectively verifiable acceptance criterion yet".to_string(),
            );
        }
        if !self.definition_gaps.is_empty() && self.stage > InitiativeStage::Defined {
            self.stage = InitiativeStage::Defined;
        }
        self
    }
}

/// Render the blocked set as a flat map for the snapshot.
fn blocked_summary(schedule: &Schedule) -> BTreeMap<String, String> {
    schedule
        .blocked
        .iter()
        .map(|(task, reason)| (task.as_str().to_string(), reason.message()))
        .collect()
}

/// Infer the lifecycle stage from the artifacts and evidence that exist.
fn stage(
    initiative: &Initiative,
    graph: &TaskGraph,
    coverage: &CoverageMatrix,
    plan: &ArchitecturePlan,
    completion: &CompletionGate,
) -> InitiativeStage {
    if completion.complete && graph.is_complete() {
        return InitiativeStage::Complete;
    }
    if coverage
        .statuses
        .values()
        .any(|status| status.state == CoverageState::Reviewed)
    {
        return InitiativeStage::Verifying;
    }
    if graph
        .integration_tasks()
        .iter()
        .any(|task| task.state.is_active())
    {
        return InitiativeStage::Integrating;
    }
    if graph
        .tasks()
        .any(|task| task.state.is_active() || task.state == TaskState::AwaitingReview)
    {
        return InitiativeStage::Executing;
    }
    if initiative.task_graph.is_some() && !graph.tasks.is_empty() {
        return InitiativeStage::Scheduled;
    }
    if !plan.work_streams.is_empty() {
        return InitiativeStage::Planned;
    }
    if !initiative.repositories.is_empty() {
        return InitiativeStage::Discovered;
    }
    if initiative.definition_version > 0 {
        return InitiativeStage::Defined;
    }
    InitiativeStage::Specified
}

#[cfg(test)]
mod tests {
    use super::*;
    use definition::{AcceptanceCriterion, Provenance, Requirement, RequirementKind};
    use ids::{CriterionId, EvidenceId, RequirementId, TaskId};
    use plan::WorkStream;
    use repository::{Capability, RepositoryRecord};
    use std::collections::BTreeSet;
    use task::Task;
    use traceability::{EvidenceKind, EvidenceOutcome, EvidenceRecord};

    fn initiative_id() -> InitiativeId {
        InitiativeId::parse("INIT-2026-0042").expect("valid initiative id")
    }

    fn repository(text: &str) -> RepositoryId {
        RepositoryId::parse(text).expect("valid repository id")
    }

    fn requirement_id(text: &str) -> RequirementId {
        RequirementId::parse(text).expect("valid requirement id")
    }

    fn task_id(text: &str) -> TaskId {
        TaskId::parse(text).expect("valid task id")
    }

    fn workspace() -> Workspace {
        let mut workspace = Workspace::new();
        for name in [
            "github.com/InferWeave/autospec",
            "github.com/InferWeave/autospec-orchestrator",
            "github.com/OtherOrg/frontend",
        ] {
            workspace.insert(RepositoryRecord {
                id: repository(name),
                revision: Some("aaa1111".to_string()),
                default_branch: Some("main".to_string()),
                credential_reference: None,
                capabilities: BTreeSet::from([
                    Capability::Read,
                    Capability::Issues,
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

    fn definition() -> Definition {
        let mut definition = Definition::new(initiative_id(), 1, "sha256:spec");
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

    fn architecture_plan() -> ArchitecturePlan {
        let mut plan = ArchitecturePlan::new(
            PlanId::parse("PLAN-ARCH-0003").expect("valid plan id"),
            initiative_id(),
            1,
            &definition(),
            &workspace(),
        );
        plan.work_streams = vec![WorkStream {
            id: "WS-01".to_string(),
            summary: "orchestrator core".to_string(),
            satisfies: vec![requirement_id("REQ-001")],
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
        graph
    }

    fn initiative() -> Initiative {
        let mut initiative =
            Initiative::new(initiative_id(), "planning-orchestration-v2", "spec/spec.md");
        initiative.definition_version = 1;
        initiative.repositories = workspace().repositories.keys().cloned().collect();
        initiative.architecture_plan =
            Some(PlanId::parse("PLAN-ARCH-0003").expect("valid plan id"));
        initiative.task_graph = Some(GraphId::parse("DAG-0007").expect("valid graph id"));
        initiative
    }

    fn verified_evidence() -> Vec<EvidenceRecord> {
        [
            (EvidenceKind::Implementation, EvidenceOutcome::Pass, "impl"),
            (EvidenceKind::Test, EvidenceOutcome::Pass, "test"),
            (EvidenceKind::Review, EvidenceOutcome::Approved, "rev"),
            (
                EvidenceKind::Verification,
                EvidenceOutcome::Verified,
                "verify",
            ),
        ]
        .into_iter()
        .enumerate()
        .map(|(index, (kind, outcome, token))| EvidenceRecord {
            id: EvidenceId::from_sequence(index as u32 + 1, 3),
            kind,
            outcome,
            task: task_id("TASK-0001"),
            requirements: vec![requirement_id("REQ-001")],
            session_name: format!("aspec-INIT-0042-TASK-0001-{token}-a1"),
            reference: "artifact://evidence".to_string(),
        })
        .collect()
    }

    #[test]
    fn an_initiative_belongs_to_no_single_repository() {
        let initiative = initiative();

        assert_eq!(initiative.repositories.len(), 3);
        assert_eq!(
            initiative
                .repositories
                .iter()
                .map(RepositoryId::owner)
                .collect::<BTreeSet<_>>()
                .len(),
            2
        );
        initiative.validate().expect("valid initiative");
    }

    #[test]
    fn a_task_graph_without_an_architecture_plan_is_rejected() {
        let mut initiative = initiative();
        initiative.architecture_plan = None;

        let problems = initiative.validate().expect_err("plan is required first");

        assert!(problems[0].contains("requires an architecture plan"));
    }

    #[test]
    fn status_reports_scheduling_coverage_and_the_completion_gate() {
        let graph = graph();
        let workspace = workspace();
        let coverage = CoverageMatrix::build(&definition(), &architecture_plan(), &graph, &[], &[]);

        let status = status(
            &initiative(),
            &definition(),
            &architecture_plan(),
            &graph,
            &workspace,
            &coverage,
            0,
        );

        assert_eq!(status.stage, InitiativeStage::Scheduled);
        assert_eq!(status.repository_count, 3);
        assert_eq!(status.owner_scope_count, 2);
        assert_eq!(status.ready_tasks, 1);
        assert_eq!(status.task_states["DEFINED"], 1);
        assert!(!status.completion.complete);
    }

    #[test]
    fn an_initiative_is_complete_only_when_every_requirement_is_verified() {
        let mut graph = graph();
        graph
            .get_mut(&task_id("TASK-0001"))
            .expect("task exists")
            .state = TaskState::Verified;
        let coverage = CoverageMatrix::build(
            &definition(),
            &architecture_plan(),
            &graph,
            &verified_evidence(),
            &[],
        );

        let status = status(
            &initiative(),
            &definition(),
            &architecture_plan(),
            &graph,
            &workspace(),
            &coverage,
            0,
        );

        assert_eq!(status.stage, InitiativeStage::Complete);
        assert!(status.completion.complete);
        assert_eq!(status.requirement_states["verified"], 1);
    }

    #[test]
    fn a_definition_gap_holds_the_initiative_at_the_defined_stage() {
        let mut definition = definition();
        definition.requirements[0].acceptance[0].objectively_verifiable = false;
        let graph = graph();
        let coverage = CoverageMatrix::build(&definition, &architecture_plan(), &graph, &[], &[]);

        let status = status(
            &initiative(),
            &definition,
            &architecture_plan(),
            &graph,
            &workspace(),
            &coverage,
            0,
        );

        assert_eq!(status.stage, InitiativeStage::Defined);
        assert!(status.definition_gaps["REQ-001"].contains("objectively verifiable"));
        assert!(status.blocked_tasks.is_empty());
    }

    #[test]
    fn the_status_snapshot_serializes_for_the_dashboard() {
        let graph = graph();
        let coverage = CoverageMatrix::build(&definition(), &architecture_plan(), &graph, &[], &[]);

        let rendered = serde_json::to_string(&status(
            &initiative(),
            &definition(),
            &architecture_plan(),
            &graph,
            &workspace(),
            &coverage,
            0,
        ))
        .expect("serializable");

        assert!(rendered.contains("\"stage\":\"scheduled\""), "{rendered}");
        assert!(rendered.contains("\"owner_scope_count\":2"), "{rendered}");
    }
}
