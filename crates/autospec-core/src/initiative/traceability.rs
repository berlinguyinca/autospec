//! Requirement traceability, coverage, and the final completion gate.
//!
//! Completion means verified requirement coverage, not closed issues
//! (architectural invariant 9). Every requirement exposes its implementation,
//! its tests, its review, and its independent verification, or it is not done.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::dag::TaskGraph;
use super::definition::Definition;
use super::ids::{EvidenceId, RequirementId, TaskId};
use super::plan::ArchitecturePlan;
use super::task::TaskState;

/// What kind of evidence a record carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    /// A change landed.
    Implementation,
    /// Tests ran.
    Test,
    /// An independent reviewer reported.
    Review,
    /// Cross-repository integration ran.
    Integration,
    /// Final verification against the Definition.
    Verification,
}

impl EvidenceKind {
    /// The stable wire name.
    pub fn as_str(&self) -> &'static str {
        match self {
            EvidenceKind::Implementation => "implementation",
            EvidenceKind::Test => "test",
            EvidenceKind::Review => "review",
            EvidenceKind::Integration => "integration",
            EvidenceKind::Verification => "verification",
        }
    }

    /// Whether the evidence judges work rather than producing it.
    pub fn is_judgement(&self) -> bool {
        matches!(self, EvidenceKind::Review | EvidenceKind::Verification)
    }
}

/// What the evidence reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceOutcome {
    /// The check succeeded.
    Pass,
    /// The check failed.
    Fail,
    /// A reviewer approved.
    Approved,
    /// A reviewer asked for changes.
    ChangesRequested,
    /// Verification confirmed the requirement holds.
    Verified,
}

impl EvidenceOutcome {
    /// The stable wire name.
    pub fn as_str(&self) -> &'static str {
        match self {
            EvidenceOutcome::Pass => "pass",
            EvidenceOutcome::Fail => "fail",
            EvidenceOutcome::Approved => "approved",
            EvidenceOutcome::ChangesRequested => "changes_requested",
            EvidenceOutcome::Verified => "verified",
        }
    }

    /// Whether the outcome is a positive result.
    pub fn is_positive(&self) -> bool {
        matches!(
            self,
            EvidenceOutcome::Pass | EvidenceOutcome::Approved | EvidenceOutcome::Verified
        )
    }
}

/// One piece of evidence produced by an agent session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceRecord {
    /// Stable evidence identifier.
    pub id: EvidenceId,
    /// What kind of evidence it is.
    pub kind: EvidenceKind,
    /// What it reported.
    pub outcome: EvidenceOutcome,
    /// The task the evidence belongs to.
    pub task: TaskId,
    /// Requirements the evidence speaks to.
    #[serde(default)]
    pub requirements: Vec<RequirementId>,
    /// The Pi session that produced it.
    pub session_name: String,
    /// A pointer to the artifact: a report path, a PR, a run id.
    pub reference: String,
}

/// An explicitly approved exception for an unverified requirement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Waiver {
    /// The requirement being waived.
    pub requirement: RequirementId,
    /// Why the exception is acceptable.
    pub reason: String,
    /// Who approved it; a waiver is never self-service.
    pub approved_by: String,
    /// Unix seconds the approval was recorded.
    pub approved_at: u64,
}

/// How far one requirement has progressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverageState {
    /// Normalized, but no plan covers it.
    Defined,
    /// A work stream covers it, but no task exists.
    Planned,
    /// A task exists but produced no implementation evidence.
    InProgress,
    /// Implementation evidence exists.
    Implemented,
    /// Tests passed.
    Tested,
    /// Review approved.
    Reviewed,
    /// Independently verified.
    Verified,
    /// Some evidence failed.
    Failed,
    /// Blocked by a permission or an unverifiable dependency.
    Blocked,
    /// Unverified, with an approved exception.
    Waived,
}

impl CoverageState {
    /// The stable wire name.
    pub fn as_str(&self) -> &'static str {
        match self {
            CoverageState::Defined => "defined",
            CoverageState::Planned => "planned",
            CoverageState::InProgress => "in_progress",
            CoverageState::Implemented => "implemented",
            CoverageState::Tested => "tested",
            CoverageState::Reviewed => "reviewed",
            CoverageState::Verified => "verified",
            CoverageState::Failed => "failed",
            CoverageState::Blocked => "blocked",
            CoverageState::Waived => "waived",
        }
    }

    /// Whether the requirement may be counted as complete.
    pub fn is_complete(&self) -> bool {
        matches!(self, CoverageState::Verified | CoverageState::Waived)
    }
}

/// One step in a requirement's evidence chain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceStep {
    /// What produced this step: a plan, a task, or a piece of evidence.
    pub node: String,
    /// A short description of what it contributes.
    pub detail: String,
}

/// The coverage record for one requirement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequirementStatus {
    /// The requirement.
    pub requirement: RequirementId,
    /// Plan work streams that cover it.
    pub work_streams: Vec<String>,
    /// Tasks that map to it.
    pub tasks: Vec<TaskId>,
    /// Evidence recorded for it.
    pub evidence: Vec<EvidenceId>,
    /// Where it has got to.
    pub state: CoverageState,
    /// The waiver, when one was approved.
    #[serde(default)]
    pub waiver: Option<Waiver>,
}

/// Whether the Initiative may complete.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompletionGate {
    /// Whether every requirement is verified or explicitly waived.
    pub complete: bool,
    /// Requirements that are neither verified nor waived.
    pub unverified: Vec<RequirementId>,
    /// Requirements completing only because of an approved waiver.
    pub waived: Vec<RequirementId>,
    /// Evidence that was rejected for lacking independence.
    pub rejected_evidence: Vec<String>,
}

/// The requirement coverage matrix for one Initiative.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoverageMatrix {
    /// Coverage by requirement.
    pub statuses: BTreeMap<RequirementId, RequirementStatus>,
    /// Evidence discarded because the producing session also implemented.
    pub rejected_evidence: Vec<String>,
}

impl CoverageMatrix {
    /// Build the matrix from the Definition, the plan, the graph, and evidence.
    ///
    /// Judgement evidence produced by a session that also implemented the task
    /// is discarded rather than counted: verification that is not independent
    /// is not verification.
    pub fn build(
        definition: &Definition,
        plan: &ArchitecturePlan,
        graph: &TaskGraph,
        evidence: &[EvidenceRecord],
        waivers: &[Waiver],
    ) -> Self {
        let implementation_sessions = implementation_sessions(evidence);
        let mut rejected_evidence = Vec::new();
        let mut accepted = Vec::new();
        for record in evidence {
            let collapsed = record.kind.is_judgement()
                && implementation_sessions
                    .get(&record.task)
                    .is_some_and(|sessions| sessions.contains(&record.session_name));
            if collapsed {
                rejected_evidence.push(format!(
                    "{}: {} evidence came from implementation session {}",
                    record.id, record.kind.as_str(), record.session_name
                ));
            } else {
                accepted.push(record);
            }
        }

        let mut statuses = BTreeMap::new();
        for requirement in definition.actionable_requirements() {
            let work_streams = plan
                .work_streams
                .iter()
                .filter(|stream| stream.satisfies.contains(&requirement.id))
                .map(|stream| stream.id.clone())
                .collect::<Vec<_>>();
            let tasks = graph
                .tasks()
                .filter(|task| {
                    task.state != TaskState::Superseded && task.satisfies.contains(&requirement.id)
                })
                .map(|task| task.id.clone())
                .collect::<Vec<_>>();
            let relevant = accepted
                .iter()
                .filter(|record| record.requirements.contains(&requirement.id))
                .copied()
                .collect::<Vec<_>>();
            let waiver = waivers
                .iter()
                .find(|waiver| waiver.requirement == requirement.id)
                .cloned();

            let state = coverage_state(graph, &work_streams, &tasks, &relevant, waiver.is_some());
            statuses.insert(
                requirement.id.clone(),
                RequirementStatus {
                    requirement: requirement.id.clone(),
                    work_streams,
                    tasks,
                    evidence: relevant.iter().map(|record| record.id.clone()).collect(),
                    state,
                    waiver,
                },
            );
        }

        Self {
            statuses,
            rejected_evidence,
        }
    }

    /// The coverage record for one requirement.
    pub fn status(&self, requirement: &RequirementId) -> Option<&RequirementStatus> {
        self.statuses.get(requirement)
    }

    /// How many requirements sit in each coverage state.
    pub fn summary(&self) -> BTreeMap<CoverageState, usize> {
        let mut summary = BTreeMap::new();
        for status in self.statuses.values() {
            *summary.entry(status.state).or_insert(0) += 1;
        }
        summary
    }

    /// The evidence chain for one requirement, from plan through verification.
    pub fn trace(
        &self,
        requirement: &RequirementId,
        evidence: &[EvidenceRecord],
    ) -> Vec<TraceStep> {
        let Some(status) = self.statuses.get(requirement) else {
            return Vec::new();
        };
        let mut steps = vec![TraceStep {
            node: requirement.as_str().to_string(),
            detail: "requirement".to_string(),
        }];
        for stream in &status.work_streams {
            steps.push(TraceStep {
                node: stream.clone(),
                detail: "work stream".to_string(),
            });
        }
        for task in &status.tasks {
            steps.push(TraceStep {
                node: task.as_str().to_string(),
                detail: "task".to_string(),
            });
        }
        for record in evidence {
            if status.evidence.contains(&record.id) {
                steps.push(TraceStep {
                    node: record.id.as_str().to_string(),
                    detail: format!("{} {}", record.kind.as_str(), record.outcome.as_str()),
                });
            }
        }
        if let Some(waiver) = &status.waiver {
            steps.push(TraceStep {
                node: format!("WAIVER-{}", waiver.requirement),
                detail: format!("approved by {}", waiver.approved_by),
            });
        }
        steps
    }

    /// Whether the Initiative may be declared complete.
    pub fn completion_gate(&self) -> CompletionGate {
        let mut unverified = Vec::new();
        let mut waived = Vec::new();
        for status in self.statuses.values() {
            match status.state {
                CoverageState::Verified => {}
                CoverageState::Waived => waived.push(status.requirement.clone()),
                _ => unverified.push(status.requirement.clone()),
            }
        }
        CompletionGate {
            complete: unverified.is_empty(),
            unverified,
            waived,
            rejected_evidence: self.rejected_evidence.clone(),
        }
    }
}

/// Sessions that produced implementation evidence, by task.
fn implementation_sessions(evidence: &[EvidenceRecord]) -> BTreeMap<TaskId, BTreeSet<String>> {
    let mut sessions: BTreeMap<TaskId, BTreeSet<String>> = BTreeMap::new();
    for record in evidence {
        if record.kind == EvidenceKind::Implementation {
            sessions
                .entry(record.task.clone())
                .or_default()
                .insert(record.session_name.clone());
        }
    }
    sessions
}

/// The furthest state the accepted evidence supports.
fn coverage_state(
    graph: &TaskGraph,
    work_streams: &[String],
    tasks: &[TaskId],
    evidence: &[&EvidenceRecord],
    waived: bool,
) -> CoverageState {
    if evidence
        .iter()
        .any(|record| !record.outcome.is_positive())
    {
        return CoverageState::Failed;
    }
    let has = |kind: EvidenceKind| {
        evidence
            .iter()
            .any(|record| record.kind == kind && record.outcome.is_positive())
    };

    if has(EvidenceKind::Verification) {
        return CoverageState::Verified;
    }
    if waived {
        return CoverageState::Waived;
    }
    if has(EvidenceKind::Review) {
        return CoverageState::Reviewed;
    }
    if has(EvidenceKind::Test) || has(EvidenceKind::Integration) {
        return CoverageState::Tested;
    }
    if has(EvidenceKind::Implementation) {
        return CoverageState::Implemented;
    }
    if tasks.is_empty() {
        return if work_streams.is_empty() {
            CoverageState::Defined
        } else {
            CoverageState::Planned
        };
    }
    let all_blocked = tasks.iter().all(|id| {
        graph
            .get(id)
            .is_some_and(|task| task.state == TaskState::Blocked)
    });
    if all_blocked {
        CoverageState::Blocked
    } else {
        CoverageState::InProgress
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::initiative::definition::{
        AcceptanceCriterion, Provenance, Requirement, RequirementKind,
    };
    use crate::initiative::ids::{CriterionId, GraphId, InitiativeId, PlanId};
    use crate::initiative::plan::WorkStream;
    use crate::initiative::repository::{Capability, RepositoryId, RepositoryRecord, Workspace};
    use crate::initiative::task::Task;

    fn requirement_id(text: &str) -> RequirementId {
        RequirementId::parse(text).expect("valid requirement id")
    }

    fn task_id(text: &str) -> TaskId {
        TaskId::parse(text).expect("valid task id")
    }

    fn repository() -> RepositoryId {
        RepositoryId::parse("github.com/InferWeave/autospec").expect("valid repository id")
    }

    fn workspace() -> Workspace {
        let mut workspace = Workspace::new();
        workspace.insert(RepositoryRecord {
            id: repository(),
            revision: Some("aaa1111".to_string()),
            default_branch: Some("main".to_string()),
            credential_reference: None,
            capabilities: Capability::write_set(),
            languages: Vec::new(),
            build_systems: Vec::new(),
            validation_commands: Vec::new(),
        });
        workspace
    }

    fn definition() -> Definition {
        let mut definition = Definition::new(
            InitiativeId::parse("INIT-2026-0042").expect("valid initiative id"),
            1,
            "sha256:spec",
        );
        definition.requirements = ["REQ-017", "REQ-018"]
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

    fn plan() -> ArchitecturePlan {
        let mut plan = ArchitecturePlan::new(
            PlanId::parse("PLAN-ARCH-0004").expect("valid plan id"),
            InitiativeId::parse("INIT-2026-0042").expect("valid initiative id"),
            1,
            &definition(),
            &workspace(),
        );
        plan.work_streams = vec![WorkStream {
            id: "PLAN-04".to_string(),
            summary: "coverage engine".to_string(),
            satisfies: vec![requirement_id("REQ-017"), requirement_id("REQ-018")],
            repositories: vec![repository()],
        }];
        plan
    }

    fn graph() -> TaskGraph {
        let mut graph = TaskGraph::new(
            GraphId::parse("DAG-0007").expect("valid graph id"),
            1,
            PlanId::parse("PLAN-ARCH-0004").expect("valid plan id"),
        );
        graph.insert(Task::implementation(
            task_id("TASK-123"),
            repository(),
            vec![requirement_id("REQ-017")],
            1,
        ));
        graph.insert(Task::implementation(
            task_id("TASK-128"),
            repository(),
            vec![requirement_id("REQ-018")],
            1,
        ));
        graph
    }

    fn evidence(
        id: &str,
        kind: EvidenceKind,
        outcome: EvidenceOutcome,
        task: &str,
        requirement: &str,
        session: &str,
    ) -> EvidenceRecord {
        EvidenceRecord {
            id: EvidenceId::parse(id).expect("valid evidence id"),
            kind,
            outcome,
            task: task_id(task),
            requirements: vec![requirement_id(requirement)],
            session_name: session.to_string(),
            reference: format!("artifact://{id}"),
        }
    }

    fn full_chain(requirement: &str, task: &str, offset: u32) -> Vec<EvidenceRecord> {
        vec![
            evidence(
                &format!("EV-{:03}", offset),
                EvidenceKind::Implementation,
                EvidenceOutcome::Pass,
                task,
                requirement,
                "aspec-INIT-0042-TASK-0123-impl-a1",
            ),
            evidence(
                &format!("EV-{:03}", offset + 1),
                EvidenceKind::Test,
                EvidenceOutcome::Pass,
                task,
                requirement,
                "aspec-INIT-0042-TASK-0123-test-a1",
            ),
            evidence(
                &format!("EV-{:03}", offset + 2),
                EvidenceKind::Review,
                EvidenceOutcome::Approved,
                task,
                requirement,
                "aspec-INIT-0042-TASK-0123-rev-a1",
            ),
            evidence(
                &format!("EV-{:03}", offset + 3),
                EvidenceKind::Verification,
                EvidenceOutcome::Verified,
                task,
                requirement,
                "aspec-INIT-0042-initiative-verify-a1",
            ),
        ]
    }

    #[test]
    fn a_requirement_traces_from_plan_through_verification() {
        let evidence = full_chain("REQ-017", "TASK-123", 10);
        let matrix = CoverageMatrix::build(&definition(), &plan(), &graph(), &evidence, &[]);

        let trace = matrix.trace(&requirement_id("REQ-017"), &evidence);
        let nodes = trace
            .iter()
            .map(|step| step.node.as_str())
            .collect::<Vec<_>>();

        assert_eq!(
            nodes,
            vec!["REQ-017", "PLAN-04", "TASK-123", "EV-010", "EV-011", "EV-012", "EV-013"]
        );
        assert_eq!(
            matrix.status(&requirement_id("REQ-017")).map(|status| status.state),
            Some(CoverageState::Verified)
        );
    }

    #[test]
    fn an_initiative_cannot_complete_while_a_requirement_is_unverified() {
        let evidence = full_chain("REQ-017", "TASK-123", 10);
        let matrix = CoverageMatrix::build(&definition(), &plan(), &graph(), &evidence, &[]);

        let gate = matrix.completion_gate();

        assert!(!gate.complete);
        assert_eq!(gate.unverified, vec![requirement_id("REQ-018")]);
    }

    #[test]
    fn an_approved_waiver_completes_a_requirement_that_was_never_verified() {
        let evidence = full_chain("REQ-017", "TASK-123", 10);
        let waiver = Waiver {
            requirement: requirement_id("REQ-018"),
            reason: "deferred to the next Initiative".to_string(),
            approved_by: "product-owner".to_string(),
            approved_at: 1_772_000_000,
        };

        let matrix =
            CoverageMatrix::build(&definition(), &plan(), &graph(), &evidence, &[waiver]);
        let gate = matrix.completion_gate();

        assert!(gate.complete);
        assert_eq!(gate.waived, vec![requirement_id("REQ-018")]);
    }

    #[test]
    fn verification_from_the_implementation_session_is_discarded() {
        let mut evidence = full_chain("REQ-017", "TASK-123", 10);
        evidence[3].session_name = "aspec-INIT-0042-TASK-0123-impl-a1".to_string();

        let matrix = CoverageMatrix::build(&definition(), &plan(), &graph(), &evidence, &[]);

        assert_eq!(
            matrix.status(&requirement_id("REQ-017")).map(|status| status.state),
            Some(CoverageState::Reviewed)
        );
        assert_eq!(matrix.rejected_evidence.len(), 1);
        assert!(matrix.rejected_evidence[0].contains("implementation session"));
        assert!(!matrix.completion_gate().complete);
    }

    #[test]
    fn failed_evidence_shows_as_failed_rather_than_partially_covered() {
        let mut evidence = full_chain("REQ-017", "TASK-123", 10);
        evidence[1].outcome = EvidenceOutcome::Fail;
        evidence.truncate(2);

        let matrix = CoverageMatrix::build(&definition(), &plan(), &graph(), &evidence, &[]);

        assert_eq!(
            matrix.status(&requirement_id("REQ-017")).map(|status| status.state),
            Some(CoverageState::Failed)
        );
    }

    #[test]
    fn a_requirement_whose_tasks_are_all_blocked_reports_as_blocked() {
        let mut graph = graph();
        graph
            .get_mut(&task_id("TASK-123"))
            .expect("task exists")
            .state = TaskState::Blocked;

        let matrix = CoverageMatrix::build(&definition(), &plan(), &graph, &[], &[]);

        assert_eq!(
            matrix.status(&requirement_id("REQ-017")).map(|status| status.state),
            Some(CoverageState::Blocked)
        );
    }

    #[test]
    fn progress_is_reported_per_state_rather_than_as_a_single_count() {
        let evidence = full_chain("REQ-017", "TASK-123", 10);
        let matrix = CoverageMatrix::build(&definition(), &plan(), &graph(), &evidence, &[]);

        let summary = matrix.summary();

        assert_eq!(summary.get(&CoverageState::Verified), Some(&1));
        assert_eq!(summary.get(&CoverageState::InProgress), Some(&1));
    }

    #[test]
    fn a_requirement_with_no_task_reports_as_planned() {
        let mut graph = graph();
        graph.tasks.remove(&task_id("TASK-128"));

        let matrix = CoverageMatrix::build(&definition(), &plan(), &graph, &[], &[]);

        assert_eq!(
            matrix.status(&requirement_id("REQ-018")).map(|status| status.state),
            Some(CoverageState::Planned)
        );
    }
}
