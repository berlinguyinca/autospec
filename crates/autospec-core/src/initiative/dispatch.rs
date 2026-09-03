//! The Pi invocation contract.
//!
//! Every agent session is invoked with an explicit role, model policy,
//! repository and worktree scope, the authoritative artifacts it may rely on,
//! and the output contract it must satisfy. Secrets are injected by the
//! runtime and never appear in the contract itself.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::ids::{AttemptId, InitiativeId, RequirementId, TaskId, TaskPlanId};
use super::repository::{reject_secret_material, RepositoryId};
use super::roles::{AgentRole, SessionIdentity};
use super::routing::RoutingDecision;

/// The isolated checkout one task runs in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeScope {
    /// The repository the worktree belongs to.
    pub repository: RepositoryId,
    /// The worktree path, unique per Initiative and task.
    pub worktree: PathBuf,
    /// The branch created inside the worktree.
    pub branch: String,
}

impl WorktreeScope {
    /// The isolated worktree for `task` in `repository`.
    ///
    /// The path and branch are derived from Initiative and task identity, so
    /// two concurrent tasks can never share a checkout.
    pub fn for_task(
        root: &std::path::Path,
        initiative: &InitiativeId,
        task: &TaskId,
        repository: RepositoryId,
    ) -> Self {
        let slug = format!(
            "{}-{}-{}",
            repository.host(),
            repository.owner(),
            repository.name()
        );
        Self {
            worktree: root
                .join(initiative.short())
                .join(task.as_str())
                .join(slug),
            branch: format!("aspec/{}/{}", initiative.short(), task.as_str()),
            repository,
        }
    }
}

/// The model policy a session is invoked under.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelPolicy {
    /// The capability class the role asked for.
    pub capability_class: String,
    /// The model the router selected.
    pub selected_model: String,
    /// Whether fallback was permitted.
    pub fallback_allowed: bool,
    /// How many preferred models were skipped.
    pub fallback_depth: usize,
}

/// One Pi session invocation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PiInvocation {
    /// The Initiative.
    pub initiative_id: InitiativeId,
    /// The task, when the invocation is task-scoped.
    #[serde(default)]
    pub task_id: Option<TaskId>,
    /// The attempt.
    pub attempt_id: AttemptId,
    /// The role being invoked.
    pub role: AgentRole,
    /// The unique Pi session name.
    pub session_name: String,
    /// The repository and worktree the session may touch.
    #[serde(default)]
    pub scope: Option<WorktreeScope>,
    /// The requirements the session is accountable to.
    #[serde(default)]
    pub requirements: Vec<RequirementId>,
    /// The task plan the session implements, when there is one.
    #[serde(default)]
    pub task_plan: Option<TaskPlanId>,
    /// The model policy.
    pub model_policy: ModelPolicy,
    /// The output schema the session must satisfy.
    pub output_contract: String,
    /// Authoritative artifacts the session may rely on, by registry path.
    #[serde(default)]
    pub artifacts: BTreeMap<String, String>,
}

impl PiInvocation {
    /// Build the invocation for a routing decision.
    pub fn from_decision(
        decision: &RoutingDecision,
        session: &SessionIdentity,
        scope: Option<WorktreeScope>,
        requirements: Vec<RequirementId>,
        output_contract: impl Into<String>,
        fallback_allowed: bool,
    ) -> Self {
        Self {
            initiative_id: session.initiative.clone(),
            task_id: session.task.clone(),
            attempt_id: session.attempt.clone(),
            role: session.role,
            session_name: session.session_name().to_string(),
            scope,
            requirements,
            task_plan: None,
            model_policy: ModelPolicy {
                capability_class: decision.capability_class.clone(),
                selected_model: decision.selected.clone(),
                fallback_allowed,
                fallback_depth: decision.fallback_depth,
            },
            output_contract: output_contract.into(),
            artifacts: BTreeMap::new(),
        }
    }

    /// Attach the task plan the session must follow.
    pub fn with_task_plan(mut self, task_plan: TaskPlanId) -> Self {
        self.task_plan = Some(task_plan);
        self
    }

    /// Attach an authoritative artifact by registry path.
    pub fn with_artifact(mut self, name: impl Into<String>, path: impl Into<String>) -> Self {
        self.artifacts.insert(name.into(), path.into());
        self
    }

    /// Reject an invocation that would carry secret material into a prompt.
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut problems = Vec::new();
        let mut check = |field: &str, value: &str| {
            if let Err(problem) = reject_secret_material(field, value) {
                problems.push(problem);
            }
        };

        check("session_name", &self.session_name);
        check("model_policy.selected_model", &self.model_policy.selected_model);
        check("output_contract", &self.output_contract);
        for (name, path) in &self.artifacts {
            check(&format!("artifacts.{name}"), path);
        }
        if let Some(scope) = &self.scope {
            check("scope.branch", &scope.branch);
            check("scope.worktree", &scope.worktree.to_string_lossy());
        }

        if self.role.is_producing() && self.scope.is_none() {
            problems.push("an implementation session needs an isolated worktree".to_string());
        }
        if self.output_contract.trim().is_empty() {
            problems.push("every invocation declares an output contract".to_string());
        }

        if problems.is_empty() {
            Ok(())
        } else {
            Err(problems)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::initiative::routing::{
        ModelCatalog, ModelClass, ModelDescriptor, PrivacyTier, QuotaState, RoleRequirements,
    };
    use std::path::Path;

    fn initiative() -> InitiativeId {
        InitiativeId::parse("INIT-2026-0042").expect("valid initiative id")
    }

    fn task(text: &str) -> TaskId {
        TaskId::parse(text).expect("valid task id")
    }

    fn repository(text: &str) -> RepositoryId {
        RepositoryId::parse(text).expect("valid repository id")
    }

    fn catalog() -> ModelCatalog {
        ModelCatalog::new(vec![ModelDescriptor {
            id: "remote/frontier".to_string(),
            provider: "remote".to_string(),
            class: ModelClass::Frontier,
            vision: false,
            tools: true,
            context_tokens: 400_000,
            local: false,
            privacy: PrivacyTier::Private,
            cost_per_1k_millicents: 1_500,
        }])
    }

    fn invocation(role: AgentRole, scope: Option<WorktreeScope>) -> PiInvocation {
        let decision = catalog()
            .select(
                role,
                Some(task("TASK-0017")),
                &RoleRequirements::for_role(role),
                &QuotaState::all_available(&["remote/frontier"], 1_000_000),
            )
            .expect("routed");
        let session = decision.session(initiative(), AttemptId::from_sequence(3, 3));
        PiInvocation::from_decision(
            &decision,
            &session,
            scope,
            vec![RequirementId::parse("REQ-012").expect("valid requirement id")],
            "implementation-result.schema.json",
            true,
        )
    }

    fn scope(task_id: &str) -> WorktreeScope {
        WorktreeScope::for_task(
            Path::new("/worktrees"),
            &initiative(),
            &task(task_id),
            repository("github.com/InferWeave/autospec-orchestrator"),
        )
    }

    #[test]
    fn the_invocation_matches_the_contract_in_the_specification() {
        let invocation = invocation(AgentRole::Implementer, Some(scope("TASK-0017")));

        assert_eq!(invocation.session_name, "aspec-INIT-0042-TASK-0017-impl-a3");
        assert_eq!(invocation.role, AgentRole::Implementer);
        assert_eq!(invocation.model_policy.capability_class, "coding-high");
        assert_eq!(
            invocation.output_contract,
            "implementation-result.schema.json"
        );
        invocation.validate().expect("valid invocation");
    }

    #[test]
    fn two_concurrent_tasks_never_share_a_worktree_or_branch() {
        let first = scope("TASK-0017");
        let second = scope("TASK-0018");

        assert_ne!(first.worktree, second.worktree);
        assert_ne!(first.branch, second.branch);
        assert_eq!(first.branch, "aspec/INIT-0042/TASK-0017");
        assert!(first
            .worktree
            .ends_with("INIT-0042/TASK-0017/github.com-InferWeave-autospec-orchestrator"));
    }

    #[test]
    fn one_task_in_two_repositories_gets_two_worktrees() {
        let orchestrator = scope("TASK-0017");
        let frontend = WorktreeScope::for_task(
            Path::new("/worktrees"),
            &initiative(),
            &task("TASK-0017"),
            repository("github.com/OtherOrg/frontend"),
        );

        assert_ne!(orchestrator.worktree, frontend.worktree);
    }

    #[test]
    fn an_implementation_session_without_a_worktree_is_refused() {
        let problems = invocation(AgentRole::Implementer, None)
            .validate()
            .expect_err("implementation needs isolation");

        assert!(problems[0].contains("isolated worktree"));
    }

    #[test]
    fn a_review_session_needs_no_worktree_of_its_own() {
        invocation(AgentRole::Reviewer, None)
            .validate()
            .expect("review reads the change, it does not build it");
    }

    #[test]
    fn an_invocation_carrying_secret_material_is_refused() {
        let invocation = invocation(AgentRole::Implementer, Some(scope("TASK-0017")))
            .with_artifact("token", "ghp_livetokenmaterial");

        let problems = invocation.validate().expect_err("secrets are refused");

        assert!(problems[0].contains("credential reference"), "{problems:?}");
    }

    #[test]
    fn an_invocation_without_an_output_contract_is_refused() {
        let mut invocation = invocation(AgentRole::Implementer, Some(scope("TASK-0017")));
        invocation.output_contract = String::new();

        let problems = invocation.validate().expect_err("no output contract");

        assert!(problems
            .iter()
            .any(|problem| problem.contains("output contract")));
    }

    #[test]
    fn the_invocation_serializes_for_the_audit_log() {
        let invocation = invocation(AgentRole::Implementer, Some(scope("TASK-0017")))
            .with_task_plan(TaskPlanId::new(&task("TASK-0017"), 2))
            .with_artifact("definition", "definition/definition-v1.json");

        let rendered = serde_json::to_string(&invocation).expect("serializable");

        assert!(rendered.contains("\"TASKPLAN-0017-v2\""), "{rendered}");
        assert!(rendered.contains("\"role\":\"implementer\""), "{rendered}");
    }
}
