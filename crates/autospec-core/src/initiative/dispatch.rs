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
    /// The branch the finished work will merge into. Never assumed across
    /// repositories; the base ref defaults to it.
    #[serde(default)]
    pub target_branch: String,
    /// The ref the implementer starts from. Defaults to [`Self::target_branch`];
    /// a pipeline may override it, and any override is recorded as-is.
    #[serde(default)]
    pub base_ref: String,
    /// The exact commit `base_ref` resolved to when the dispatch was recorded.
    ///
    /// Recorded with every dispatch so a merge failure can be attributed
    /// rather than guessed at.
    #[serde(default)]
    pub base_commit: Option<String>,
}

impl WorktreeScope {
    /// The isolated worktree for `task` in `repository`.
    ///
    /// The path and branch are derived from Initiative and task identity, so
    /// two concurrent tasks can never share a checkout. The base ref is
    /// derived from `target_branch` — where the work will merge — never from a
    /// remembered constant, so a pipeline cannot silently clone a stale base.
    pub fn for_task(
        root: &std::path::Path,
        initiative: &InitiativeId,
        task: &TaskId,
        repository: RepositoryId,
        target_branch: impl Into<String>,
    ) -> Self {
        let slug = format!(
            "{}-{}-{}",
            repository.host(),
            repository.owner(),
            repository.name()
        );
        let target_branch = target_branch.into();
        Self {
            worktree: root.join(initiative.short()).join(task.as_str()).join(slug),
            branch: format!("aspec/{}/{}", initiative.short(), task.as_str()),
            repository,
            target_branch: target_branch.clone(),
            base_ref: target_branch,
            base_commit: None,
        }
    }

    /// Override the base ref the implementer starts from.
    ///
    /// The override is recorded as-is so a stale override is attributable.
    pub fn with_base_ref(mut self, base_ref: impl Into<String>) -> Self {
        self.base_ref = base_ref.into();
        self
    }

    /// Record the exact commit the base ref resolved to at dispatch time.
    pub fn recorded_against(mut self, base_commit: impl Into<String>) -> Self {
        self.base_commit = Some(base_commit.into());
        self
    }
}

/// A VCS capability the base-freshness gate relies on.
///
/// The pipeline asks only for the one number it can act on — how many commits
/// the base is behind the target (git: `rev-list --count base..target`).
/// Implementations may wrap git, another VCS, or an in-memory table for tests;
/// nothing here assumes a particular host or tool.
pub trait BaseFreshnessProbe {
    /// The number of commits on `target` that `base` does not contain.
    fn behind_target(&self, base: &str, target: &str) -> Result<u64, String>;
}

/// How far behind the target a base may sit and still be allowed to dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FreshnessPolicy {
    /// The base may be at most this many commits behind the target. The
    /// default is zero: *any* commits behind is stale, and spending an
    /// agent-hour on a stale base is pure waste.
    pub max_behind: u64,
}

impl Default for FreshnessPolicy {
    fn default() -> Self {
        Self { max_behind: 0 }
    }
}

/// The outcome of a base-freshness gate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FreshnessVerdict {
    /// The base is close enough to the target to dispatch.
    Fresh { behind: u64 },
    /// The base sits behind the target farther than the policy allows; the
    /// dispatch must be refused, or the branch rebased onto the target first.
    Stale { behind: u64, max_behind: u64 },
}

impl FreshnessVerdict {
    /// The measured distance, commits behind the target.
    pub fn behind(&self) -> u64 {
        match self {
            FreshnessVerdict::Fresh { behind } => *behind,
            FreshnessVerdict::Stale { behind, .. } => *behind,
        }
    }

    /// Whether the dispatch must be refused (or rebased first).
    pub fn is_stale(&self) -> bool {
        matches!(self, FreshnessVerdict::Stale { .. })
    }
}

/// Gate a dispatch on base freshness.
///
/// The distance is measured against the recorded base commit when one exists,
/// so a recorded dispatch is always checked against what actually ran, and
/// otherwise against the base ref itself.
pub fn gate_dispatch(
    probe: &dyn BaseFreshnessProbe,
    scope: &WorktreeScope,
    policy: &FreshnessPolicy,
) -> Result<FreshnessVerdict, String> {
    let base = scope.base_commit.as_deref().unwrap_or(&scope.base_ref);
    let behind = probe
        .behind_target(base, &scope.target_branch)
        .map_err(|error| format!("base freshness probe failed: {error}"))?;
    if behind > policy.max_behind {
        Ok(FreshnessVerdict::Stale {
            behind,
            max_behind: policy.max_behind,
        })
    } else {
        Ok(FreshnessVerdict::Fresh { behind })
    }
}

/// Which of the three different problems a merge conflict actually is.
///
/// The three causes look identical at merge time but have three different
/// fixes: refuse-or-rebase at dispatch, re-check and rebase now, or a genuine
/// content conflict to resolve by hand.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MergeConflictCause {
    /// The base was already behind the target by more than the policy allows
    /// when the dispatch was recorded. Fix: the dispatch should have been
    /// refused or rebased first.
    StaleAtDispatch {
        behind_at_dispatch: u64,
        max_behind: u64,
    },
    /// The base was within policy at dispatch, but the target moved on since.
    /// Fix: rebase the branch onto the current target and re-verify.
    DriftSinceDispatch {
        behind_at_dispatch: u64,
        behind_at_merge: u64,
    },
    /// The target has not moved in a way that explains the conflict; the
    /// change genuinely collides with content that landed.
    ContentConflict { behind_at_merge: u64 },
}

impl MergeConflictCause {
    /// The stable wire name.
    pub fn as_str(&self) -> &'static str {
        match self {
            MergeConflictCause::StaleAtDispatch { .. } => "stale_at_dispatch",
            MergeConflictCause::DriftSinceDispatch { .. } => "drift_since_dispatch",
            MergeConflictCause::ContentConflict { .. } => "content_conflict",
        }
    }
}

/// Classify a merge conflict from the measured base distance at dispatch and
/// at merge, under the policy that governed the dispatch.
pub fn classify_merge_conflict(
    behind_at_dispatch: u64,
    behind_at_merge: u64,
    policy: &FreshnessPolicy,
) -> MergeConflictCause {
    if behind_at_dispatch > policy.max_behind {
        MergeConflictCause::StaleAtDispatch {
            behind_at_dispatch,
            max_behind: policy.max_behind,
        }
    } else if behind_at_merge > behind_at_dispatch {
        MergeConflictCause::DriftSinceDispatch {
            behind_at_dispatch,
            behind_at_merge,
        }
    } else {
        MergeConflictCause::ContentConflict { behind_at_merge }
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
        check(
            "model_policy.selected_model",
            &self.model_policy.selected_model,
        );
        check("output_contract", &self.output_contract);
        for (name, path) in &self.artifacts {
            check(&format!("artifacts.{name}"), path);
        }
        if let Some(scope) = &self.scope {
            check("scope.branch", &scope.branch);
            check("scope.worktree", &scope.worktree.to_string_lossy());
        }

        if self.role.is_producing() {
            match &self.scope {
                None => problems
                    .push("an implementation session needs an isolated worktree".to_string()),
                Some(scope) if scope.base_commit.is_none() => problems.push(
                    "an implementation dispatch records the exact base commit it started from"
                        .to_string(),
                ),
                _ => {}
            }
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
            "main",
        )
        .recorded_against("d0621f87")
    }

    /// An in-memory probe standing in for a VCS: maps (base, target) to the
    /// commit distance the real tool would report.
    struct TableProbe(std::collections::BTreeMap<(String, String), u64>);

    impl BaseFreshnessProbe for TableProbe {
        fn behind_target(&self, base: &str, target: &str) -> Result<u64, String> {
            self.0
                .get(&(base.to_string(), target.to_string()))
                .copied()
                .ok_or_else(|| format!("no distance recorded for {base}..{target}"))
        }
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
            "trunk",
        );

        assert_ne!(orchestrator.worktree, frontend.worktree);
    }

    #[test]
    fn the_base_ref_defaults_to_the_branch_the_work_will_merge_into() {
        let scope = WorktreeScope::for_task(
            Path::new("/worktrees"),
            &initiative(),
            &task("TASK-0017"),
            repository("github.com/InferWeave/autospec-orchestrator"),
            "trunk",
        );

        assert_eq!(scope.target_branch, "trunk");
        assert_eq!(scope.base_ref, "trunk");
    }

    #[test]
    fn an_overridden_base_ref_is_recorded_as_is() {
        let scope = scope("TASK-0017").with_base_ref("integration/2026-08");

        assert_eq!(scope.base_ref, "integration/2026-08");
        assert_eq!(scope.target_branch, "main");
    }

    #[test]
    fn the_base_commit_appears_in_the_dispatch_record() {
        let invocation = invocation(AgentRole::Implementer, Some(scope("TASK-0017")));

        let rendered = serde_json::to_string(&invocation).expect("serializable");

        assert!(
            rendered.contains("\"base_commit\":\"d0621f87\""),
            "{rendered}"
        );
        assert!(
            rendered.contains("\"target_branch\":\"main\""),
            "{rendered}"
        );
    }

    #[test]
    fn a_producing_dispatch_without_a_recorded_base_commit_is_refused() {
        let scope = WorktreeScope::for_task(
            Path::new("/worktrees"),
            &initiative(),
            &task("TASK-0017"),
            repository("github.com/InferWeave/autospec-orchestrator"),
            "main",
        );
        let invocation = invocation(AgentRole::Implementer, Some(scope));

        let problems = invocation.validate().expect_err("base commit is mandatory");

        assert!(
            problems
                .iter()
                .any(|problem| problem.contains("base commit")),
            "{problems:?}"
        );
    }

    #[test]
    fn a_fresh_base_dispatches() {
        let probe = TableProbe(std::collections::BTreeMap::from([(
            ("d0621f87".to_string(), "main".to_string()),
            0,
        )]));
        let scope = scope("TASK-0017");

        let verdict =
            gate_dispatch(&probe, &scope, &FreshnessPolicy::default()).expect("probe answered");

        assert_eq!(verdict, FreshnessVerdict::Fresh { behind: 0 });
        assert!(!verdict.is_stale());
    }

    #[test]
    fn a_stale_base_is_refused_with_the_distance_reported() {
        // The incident: branched 25 commits behind main.
        let probe = TableProbe(std::collections::BTreeMap::from([(
            ("d0621f87".to_string(), "main".to_string()),
            25,
        )]));
        let scope = scope("TASK-0017");

        let verdict =
            gate_dispatch(&probe, &scope, &FreshnessPolicy::default()).expect("probe answered");

        assert_eq!(
            verdict,
            FreshnessVerdict::Stale {
                behind: 25,
                max_behind: 0
            }
        );
        assert!(verdict.is_stale());
        assert_eq!(verdict.behind(), 25);
    }

    #[test]
    fn the_staleness_threshold_is_configurable() {
        let probe = TableProbe(std::collections::BTreeMap::from([
            (("aaa".to_string(), "main".to_string()), 5),
            (("bbb".to_string(), "main".to_string()), 25),
        ]));
        let policy = FreshnessPolicy { max_behind: 10 };

        let within = gate_dispatch(&probe, &scope("TASK-0017").recorded_against("aaa"), &policy)
            .expect("probe answered");
        let beyond = gate_dispatch(&probe, &scope("TASK-0017").recorded_against("bbb"), &policy)
            .expect("probe answered");

        assert_eq!(within, FreshnessVerdict::Fresh { behind: 5 });
        assert_eq!(
            beyond,
            FreshnessVerdict::Stale {
                behind: 25,
                max_behind: 10
            }
        );
    }

    #[test]
    fn a_gate_against_an_unanswered_probe_is_an_error_not_a_guess() {
        let probe = TableProbe(std::collections::BTreeMap::new());
        let scope = scope("TASK-0017");

        let error = gate_dispatch(&probe, &scope, &FreshnessPolicy::default())
            .expect_err("no distance is not a fresh base");

        assert!(error.contains("probe failed"), "{error}");
    }

    #[test]
    fn a_conflict_reports_stale_base_at_dispatch() {
        let cause = classify_merge_conflict(25, 30, &FreshnessPolicy::default());

        assert_eq!(
            cause,
            MergeConflictCause::StaleAtDispatch {
                behind_at_dispatch: 25,
                max_behind: 0
            }
        );
        assert_eq!(cause.as_str(), "stale_at_dispatch");
    }

    #[test]
    fn a_conflict_reports_drift_after_a_fresh_dispatch() {
        let cause = classify_merge_conflict(0, 7, &FreshnessPolicy::default());

        assert_eq!(
            cause,
            MergeConflictCause::DriftSinceDispatch {
                behind_at_dispatch: 0,
                behind_at_merge: 7
            }
        );
        assert_eq!(cause.as_str(), "drift_since_dispatch");
    }

    #[test]
    fn a_conflict_with_no_base_movement_is_genuine_content() {
        let cause = classify_merge_conflict(0, 0, &FreshnessPolicy::default());

        assert_eq!(
            cause,
            MergeConflictCause::ContentConflict { behind_at_merge: 0 }
        );
        assert_eq!(cause.as_str(), "content_conflict");
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
