//! The Pi invocation contract.
//!
//! Every agent session is invoked with an explicit role, model policy,
//! repository and worktree scope, the authoritative artifacts it may rely on,
//! and the output contract it must satisfy. Secrets are injected by the
//! runtime and never appear in the contract itself.
//!
//! Two properties of the dispatch are enforced here rather than left to the
//! runner, because an agent cannot observe either of them:
//!
//! 1. **Fetch, then branch from the freshest target.** A base that trails the
//!    target is refused with the distance reported, so work is never built
//!    against a tree where merged fixes do not exist.
//! 2. **A fresh worktree per run.** The path and branch belong to one attempt,
//!    the starting state must be clean, and committed work is captured before
//!    any teardown that could destroy it.
//!
//! Both gates run unconditionally: there is no flag, switch, or blanket
//! disable, and neither failure is visible to the agent that hits it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::ids::{AttemptId, InitiativeId, RequirementId, TaskId, TaskPlanId};
use super::repository::{reject_secret_material, RepositoryId};
use super::roles::{AgentRole, SessionIdentity};
use super::routing::RoutingDecision;

/// The branch an attempt merges into, and the exact commit it was cut from.
///
/// `target_branch` is resolved by the dispatcher from the repository the work
/// merges into and never from a constant baked into a runner: a runner
/// configured for `main` branches from the wrong place in every repository
/// whose integration branch is named something else. `base_commit` is recorded
/// so that a merge failure days later is attributable to a commit rather than
/// guessed at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BaseRef {
    /// The branch the work merges into.
    pub target_branch: String,
    /// The commit the worktree was created from.
    pub base_commit: String,
}

impl BaseRef {
    /// The short form of the base commit, for messages.
    pub fn short_base(&self) -> String {
        self.base_commit.chars().take(7).collect()
    }
}

/// Whether `value` is shaped like a git object id (full or abbreviated).
fn is_commit_oid(value: &str) -> bool {
    value.len() >= 7 && value.chars().all(|character| character.is_ascii_hexdigit())
}

/// Node-local scratch roots that may be wiped while a run still holds work.
const SCRATCH_ROOTS: [&str; 4] = ["/tmp", "/var/tmp", "/private/tmp", "/dev/shm"];

/// Is `path` under node-local scratch rather than durable storage?
///
/// A scratch worktree is fine while the run lives and useless after it: three
/// runs in one day committed work that was destroyed by the node clearing its
/// own temporary directory before the result was captured.
pub fn is_node_local_scratch(path: &Path) -> bool {
    SCRATCH_ROOTS
        .iter()
        .any(|root| path.starts_with(root) && path != Path::new(root))
}

/// What the dispatcher observed while preparing the checkout, before any agent
/// was dispatched into it.
///
/// Every field is a fact about the dispatch, not something the agent can know
/// or repair, which is why an invocation carrying a failing fact is refused
/// instead of warned about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckoutFacts {
    /// Set only when the fetch that produced the base ran immediately before
    /// branching. There is no cached-clone path through the pipeline.
    pub fetched_before_branch: bool,
    /// `git rev-list --count HEAD..<target>` measured after that fetch.
    pub behind_target: usize,
    /// Set only when this run created the worktree path itself, so a path from
    /// a previous run — or another live agent — cannot be handed to it.
    pub created_for_run: bool,
    /// Set only when `git status --porcelain` was empty before the agent
    /// started: a dirty starting state is a bug, not something to work around.
    pub clean_at_start: bool,
}

impl CheckoutFacts {
    /// The two hard rules, checked against what git actually reported.
    ///
    /// The distance is reported in the stale-base refusal so the operator sees
    /// how far behind the dispatch would have been, instead of discovering it
    /// at merge time.
    pub fn verify(&self, base: &BaseRef, worktree: &Path) -> Vec<String> {
        let mut problems = Vec::new();

        if base.target_branch.trim().is_empty() {
            problems.push(
                "a dispatch needs the branch its work merges into; a configured constant is not a target"
                    .to_string(),
            );
        }
        if !is_commit_oid(&base.base_commit) {
            problems.push(format!(
                "base commit `{}` is not a git object id: record the exact commit the worktree was cut from",
                base.base_commit
            ));
        }
        if !self.fetched_before_branch {
            problems.push(format!(
                "refusing to dispatch into {}: the base was not fetched immediately before branching. A cached clone is never a base.",
                worktree.display()
            ));
        }
        if self.behind_target > 0 {
            problems.push(format!(
                "refusing to dispatch into {}: base {} is {} commit(s) behind `{}`; fetch and branch from its tip",
                worktree.display(),
                base.short_base(),
                self.behind_target,
                base.target_branch
            ));
        }
        if !self.created_for_run {
            problems.push(format!(
                "refusing to dispatch into {}: the worktree was not created for this run. Worktrees are never reused or shared, not even sequentially.",
                worktree.display()
            ));
        }
        if !self.clean_at_start {
            problems.push(format!(
                "refusing to dispatch into {}: the worktree is not clean at start; `git status` would report the previous run's leavings as this agent's work.",
                worktree.display()
            ));
        }

        problems
    }
}

/// The isolated checkout one attempt runs in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeScope {
    /// The repository the worktree belongs to.
    pub repository: RepositoryId,
    /// The worktree path, unique per Initiative, task, and attempt.
    pub worktree: PathBuf,
    /// The branch created inside the worktree, unique per attempt.
    pub branch: String,
    /// The task the checkout belongs to.
    pub task: TaskId,
    /// The run this checkout belongs to.
    pub attempt: AttemptId,
    /// The branch the work merges into and the commit it started from.
    pub base: BaseRef,
    /// What the dispatcher observed while preparing the checkout.
    pub checkout: CheckoutFacts,
}

impl WorktreeScope {
    /// The isolated worktree for one attempt of `task` in `repository`.
    ///
    /// The path and branch are derived from Initiative, task, and attempt, so
    /// two concurrent tasks never share a checkout and a retry never inherits
    /// the previous attempt's workspace.
    pub fn for_attempt(
        root: &Path,
        initiative: &InitiativeId,
        task: &TaskId,
        attempt: &AttemptId,
        repository: RepositoryId,
        base: BaseRef,
        checkout: CheckoutFacts,
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
                .join(attempt.as_str())
                .join(slug),
            branch: format!(
                "aspec/{}/{}-a{}",
                initiative.short(),
                task.as_str(),
                attempt.sequence()
            ),
            attempt: attempt.clone(),
            task: task.clone(),
            repository,
            base,
            checkout,
        }
    }

    /// The path fragment this attempt owns: `<TASK-xxxx>/<ATTEMPT-n>`.
    fn attempt_slot(&self) -> PathBuf {
        Path::new(self.task.as_str()).join(self.attempt.as_str())
    }

    /// The gates that must hold before an agent is dispatched into this
    /// checkout.
    ///
    /// Called by [`PiInvocation::validate`], so every producing session passes
    /// through them; there is no way to build an invocation around them.
    pub fn verify(&self) -> Result<(), Vec<String>> {
        let mut problems = self.checkout.verify(&self.base, &self.worktree);

        // The path and branch must belong to this attempt, so a caller cannot
        // hand back a previous run's directory with the facts relabelled.
        let in_slot = self
            .worktree
            .parent()
            .is_some_and(|parent| parent.ends_with(self.attempt_slot()));
        if !in_slot {
            problems.push(format!(
                "worktree {} is not inside this attempt's slot {}; a worktree is created per run, never reused",
                self.worktree.display(),
                self.attempt
            ));
        }
        if !self
            .branch
            .contains(&format!("-a{}", self.attempt.sequence()))
        {
            problems.push(format!(
                "branch {} does not belong to attempt {}; two runs on one branch share a checkout",
                self.branch, self.attempt
            ));
        }

        if problems.is_empty() {
            Ok(())
        } else {
            Err(problems)
        }
    }
}

/// The record of one dispatched run.
///
/// Opened from a verified [`WorktreeScope`] only, so the base commit and the
/// worktree path of every run are on the record even when the run itself is a
/// mystery. A merge failure later is then attributable to a commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunRecord {
    /// The task, when the run is task-scoped.
    #[serde(default)]
    pub task_id: Option<TaskId>,
    /// The attempt the run belongs to.
    pub attempt_id: AttemptId,
    /// The repository the run worked in.
    pub repository: RepositoryId,
    /// The worktree created for this run and discarded after it.
    pub worktree: PathBuf,
    /// The branch created for this run.
    pub branch: String,
    /// The branch the work merges into.
    pub target_branch: String,
    /// The commit the worktree was cut from.
    pub base_commit: String,
    /// The commits the run produced, recorded at capture time.
    #[serde(default)]
    pub commits: Vec<String>,
    /// Where the commits and patch were captured to, if captured.
    #[serde(default)]
    pub capture_path: Option<PathBuf>,
}

impl RunRecord {
    /// Open the record for a scope that has passed the dispatch gates.
    pub fn begin(scope: &WorktreeScope) -> Result<Self, Vec<String>> {
        scope.verify()?;

        Ok(Self {
            task_id: Some(scope.task.clone()),
            attempt_id: scope.attempt.clone(),
            repository: scope.repository.clone(),
            worktree: scope.worktree.clone(),
            branch: scope.branch.clone(),
            target_branch: scope.base.target_branch.clone(),
            base_commit: scope.base.base_commit.clone(),
            commits: Vec::new(),
            capture_path: None,
        })
    }

    /// Capture the run's commits and patch before anything tears the worktree
    /// down.
    ///
    /// The destination must survive the worktree: outside it, and not on
    /// node-local scratch that is wiped with it.
    pub fn capture(
        &mut self,
        commits: Vec<String>,
        destination: impl Into<PathBuf>,
    ) -> Result<(), String> {
        let destination = destination.into();
        if destination.starts_with(&self.worktree) {
            return Err(format!(
                "capture path {} sits inside the worktree it is meant to survive",
                destination.display()
            ));
        }
        if is_node_local_scratch(&destination) {
            return Err(format!(
                "capture path {} is node-local scratch: it can be wiped with the worktree it is meant to survive",
                destination.display()
            ));
        }

        self.commits = commits;
        self.capture_path = Some(destination);
        Ok(())
    }

    /// Whether the worktree may be destroyed without losing committed work.
    ///
    /// A checkout on node-local scratch must be captured even when no commits
    /// were recorded: a run that never wrote its commit list down is exactly
    /// the run that loses work.
    pub fn safe_to_discard(&self) -> bool {
        self.capture_path.is_some()
            || (!is_node_local_scratch(&self.worktree) && self.commits.is_empty())
    }

    /// The teardown plan for this run, refused while un-captured commits are
    /// still only in the worktree.
    pub fn teardown(&self) -> Result<TeardownReceipt, String> {
        if !self.safe_to_discard() {
            return Err(format!(
                "refusing to tear down {}: {} commit(s) captured nowhere{}. Capture the commits and patch first.",
                self.worktree.display(),
                self.commits.len(),
                if is_node_local_scratch(&self.worktree) {
                    ", and the checkout itself is node-local scratch"
                } else {
                    ""
                }
            ));
        }

        Ok(TeardownReceipt {
            worktree: self.worktree.clone(),
            base_commit: self.base_commit.clone(),
            commits_captured: self.commits.len(),
            captured_to: self.capture_path.clone(),
        })
    }
}

/// Proof that a run's worktree was discarded only after its work was safe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeardownReceipt {
    /// The worktree removed.
    pub worktree: PathBuf,
    /// The base commit the discarded run started from.
    pub base_commit: String,
    /// How many commits were captured before removal.
    pub commits_captured: usize,
    /// Where they were captured to.
    #[serde(default)]
    pub captured_to: Option<PathBuf>,
}

impl TeardownReceipt {
    /// The argv of the git commands that discard the worktree, and nothing
    /// before them.
    pub fn commands(&self) -> Vec<Vec<String>> {
        vec![
            vec![
                "git".to_string(),
                "worktree".to_string(),
                "remove".to_string(),
                self.worktree.to_string_lossy().to_string(),
            ],
            vec![
                "git".to_string(),
                "worktree".to_string(),
                "prune".to_string(),
            ],
        ]
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

        if self.role.is_producing() && self.scope.is_none() {
            problems.push("an implementation session needs an isolated worktree".to_string());
        }
        // The two hard rules run for every scope, whatever the role: they are
        // properties of the dispatch, and no role is allowed to opt out.
        if let Some(scope) = &self.scope {
            problems.extend(scope.verify().err().unwrap_or_default());
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

    fn attempt(sequence: u32) -> AttemptId {
        AttemptId::from_sequence(sequence, 3)
    }

    fn base(target: &str) -> BaseRef {
        BaseRef {
            target_branch: target.to_string(),
            base_commit: "aaa1111aaa1111".to_string(),
        }
    }

    /// The facts a dispatcher reports when it did all four things.
    fn facts() -> CheckoutFacts {
        CheckoutFacts {
            fetched_before_branch: true,
            behind_target: 0,
            created_for_run: true,
            clean_at_start: true,
        }
    }

    fn scope(task_id: &str) -> WorktreeScope {
        scope_for(task_id, 3)
    }

    fn scope_for(task_id: &str, attempt_sequence: u32) -> WorktreeScope {
        WorktreeScope::for_attempt(
            Path::new("/worktrees"),
            &initiative(),
            &task(task_id),
            &attempt(attempt_sequence),
            repository("github.com/InferWeave/autospec-orchestrator"),
            base("main"),
            facts(),
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
        assert_eq!(first.branch, "aspec/INIT-0042/TASK-0017-a3");
        assert!(first
            .worktree
            .ends_with("TASK-0017/ATTEMPT-003/github.com-InferWeave-autospec-orchestrator"));
    }

    #[test]
    fn one_task_in_two_repositories_gets_two_worktrees() {
        let orchestrator = scope("TASK-0017");
        let frontend = WorktreeScope::for_attempt(
            Path::new("/worktrees"),
            &initiative(),
            &task("TASK-0017"),
            &attempt(3),
            repository("github.com/OtherOrg/frontend"),
            base("main"),
            facts(),
        );

        assert_ne!(orchestrator.worktree, frontend.worktree);
    }

    #[test]
    fn two_attempts_of_one_task_get_their_own_worktree_and_branch() {
        let first = scope_for("TASK-0017", 1);
        let retry = scope_for("TASK-0017", 2);

        assert_ne!(first.worktree, retry.worktree);
        assert_ne!(first.branch, retry.branch);
        assert!(
            retry
                .worktree
                .ends_with("TASK-0017/ATTEMPT-002/github.com-InferWeave-autospec-orchestrator"),
            "{}",
            retry.worktree.display()
        );
    }

    #[test]
    fn a_stale_base_is_refused_with_the_distance_reported() {
        let mut scoped = scope("TASK-0017");
        scoped.checkout.behind_target = 25;

        let problems = scoped.verify().expect_err("25 behind is not a base");

        assert!(
            problems[0].contains("25 commit(s) behind `main`"),
            "{problems:?}"
        );
    }

    #[test]
    fn a_base_that_was_not_fetched_immediately_before_branching_is_refused() {
        let mut scoped = scope("TASK-0017");
        scoped.checkout.fetched_before_branch = false;

        let problems = scoped.verify().expect_err("a cached clone is never a base");

        assert!(problems[0].contains("cached clone"), "{problems:?}");
    }

    #[test]
    fn a_dirty_starting_worktree_is_refused() {
        let mut scoped = scope("TASK-0017");
        scoped.checkout.clean_at_start = false;

        let problems = scoped.verify().expect_err("a dirty start is a bug");

        assert!(problems[0].contains("not clean at start"), "{problems:?}");
    }

    #[test]
    fn a_worktree_borrowed_from_a_previous_run_is_refused() {
        let mut scoped = scope("TASK-0017");
        scoped.checkout.created_for_run = false;

        let problems = scoped.verify().expect_err("worktrees are never reused");

        assert!(
            problems[0].contains("not created for this run"),
            "{problems:?}"
        );
    }

    #[test]
    fn a_per_task_path_reused_across_attempts_is_refused() {
        let mut retry = scope_for("TASK-0017", 2);
        // The old per-task layout: the same directory handed to every attempt.
        retry.worktree =
            Path::new("/worktrees/INIT-0042/TASK-0017/github.com-InferWeave-autospec-orchestrator")
                .to_path_buf();

        let problems = retry.verify().expect_err("that path belongs to no attempt");

        assert!(
            problems[0].contains("not inside this attempt's slot"),
            "{problems:?}"
        );
    }

    #[test]
    fn an_invocation_carrying_a_stale_base_is_refused() {
        let mut scoped = scope("TASK-0017");
        scoped.checkout.behind_target = 7;
        let invocation = invocation(AgentRole::Implementer, Some(scoped));

        let problems = invocation
            .validate()
            .expect_err("stale work is unmergeable work");

        assert!(
            problems
                .iter()
                .any(|problem| problem.contains("7 commit(s) behind")),
            "{problems:?}"
        );
    }

    #[test]
    fn a_base_commit_is_recorded_for_every_run() {
        let record = RunRecord::begin(&scope("TASK-0017")).expect("verified scope");

        assert_eq!(record.base_commit, "aaa1111aaa1111");
        assert_eq!(record.target_branch, "main");
        assert_eq!(record.attempt_id, attempt(3));
        assert_eq!(
            record.worktree,
            Path::new("/worktrees/INIT-0042/TASK-0017/ATTEMPT-003/github.com-InferWeave-autospec-orchestrator")
        );
    }

    #[test]
    fn a_run_record_refuses_to_open_on_an_unverified_scope() {
        let mut scoped = scope("TASK-0017");
        scoped.checkout.clean_at_start = false;

        let problems = RunRecord::begin(&scoped).expect_err("no record, no dispatch");

        assert!(problems[0].contains("not clean at start"), "{problems:?}");
    }

    #[test]
    fn teardown_is_refused_while_commits_are_captured_nowhere() {
        let mut record = RunRecord::begin(&scope("TASK-0017")).expect("verified scope");
        record.commits = vec!["bbb2222bbb2222".to_string()];

        let error = record.teardown().expect_err("three runs died this way");

        assert!(error.contains("1 commit(s) captured nowhere"), "{error}");
    }

    #[test]
    fn work_is_captured_before_teardown_and_the_receipt_names_the_base() {
        let mut record = RunRecord::begin(&scope("TASK-0017")).expect("verified scope");
        record
            .capture(
                vec!["bbb2222bbb2222".to_string()],
                "/var/autospec-runs/INIT-0042/TASK-0017/ATTEMPT-003",
            )
            .expect("durable capture");

        let receipt = record.teardown().expect("safe to discard now");

        assert_eq!(receipt.commits_captured, 1);
        assert_eq!(receipt.base_commit, "aaa1111aaa1111");
        assert_eq!(
            receipt.commands()[0],
            vec![
                "git".to_string(),
                "worktree".to_string(),
                "remove".to_string(),
                "/worktrees/INIT-0042/TASK-0017/ATTEMPT-003/github.com-InferWeave-autospec-orchestrator"
                    .to_string(),
            ]
        );
    }

    #[test]
    fn capturing_into_node_local_scratch_is_refused() {
        let mut record = RunRecord::begin(&scope("TASK-0017")).expect("verified scope");

        let error = record
            .capture(vec!["bbb2222bbb2222".to_string()], "/tmp/capture")
            .expect_err("scratch does not survive the node");

        assert!(error.contains("node-local scratch"), "{error}");
        assert!(record.capture_path.is_none(), "{record:?}");
    }

    #[test]
    fn an_empty_run_may_be_torn_down_without_a_capture() {
        let record = RunRecord::begin(&scope("TASK-0017")).expect("verified scope");

        assert!(record.safe_to_discard());
        assert_eq!(
            record.teardown().expect("nothing to lose").commits_captured,
            0
        );
    }

    #[test]
    fn a_scratch_worktree_is_never_torn_down_uncaptured() {
        let mut scoped = scope("TASK-0017");
        scoped.worktree = Path::new(
            "/tmp/autospec-executor/INIT-0042/TASK-0017/ATTEMPT-003/github.com-InferWeave-autospec-orchestrator",
        )
        .to_path_buf();
        let mut record = RunRecord::begin(&scoped).expect("scratch is fine while the run lives");

        let error = record
            .teardown()
            .expect_err("scratch is wiped with the node");
        assert!(error.contains("node-local scratch"), "{error}");

        record
            .capture(
                Vec::new(),
                "/var/autospec-runs/INIT-0042/TASK-0017/ATTEMPT-003",
            )
            .expect("durable capture");
        assert!(record.teardown().is_ok(), "{:?}", record.capture_path);
    }

    #[test]
    fn node_local_scratch_is_recognised() {
        assert!(is_node_local_scratch(Path::new(
            "/tmp/autospec-executor/INIT-0042/TASK-0017"
        )));
        assert!(!is_node_local_scratch(Path::new(
            "/worktrees/INIT-0042/TASK-0017"
        )));
        assert!(!is_node_local_scratch(Path::new("/tmp")));
    }

    #[test]
    fn the_run_record_serializes_for_the_audit_log() {
        let record = RunRecord::begin(&scope("TASK-0017")).expect("verified scope");

        let rendered = serde_json::to_string(&record).expect("serializable");

        assert!(
            rendered.contains("\"base_commit\":\"aaa1111aaa1111\""),
            "{rendered}"
        );
        assert!(rendered.contains("ATTEMPT-003"), "{rendered}");
    }

    #[test]
    fn the_gates_have_no_disable_hatch() {
        let source = include_str!("dispatch.rs");

        // Assembled at runtime so this test does not contain the tokens it bans.
        let forbidden = [
            String::from("env") + "::var",
            String::from("std::") + "env",
            String::from("AUTOSPEC") + "_",
        ];
        for forbidden in &forbidden {
            assert!(
                !source.contains(forbidden),
                "the dispatch gates read {forbidden}: they must run unconditionally"
            );
        }
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
