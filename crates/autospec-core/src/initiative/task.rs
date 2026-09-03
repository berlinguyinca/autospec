//! Executable tasks and their lifecycle.

use std::collections::BTreeSet;
use std::fmt;

use serde::{Deserialize, Serialize};

use super::ids::{RequirementId, TaskId};
use super::repository::{Capability, RepositoryId};
use super::roles::AgentRole;

/// The lifecycle state of a task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TaskState {
    /// Known to the graph, not yet evaluated for release.
    Defined,
    /// Held back by a dependency, a permission, or an exclusivity conflict.
    Blocked,
    /// Releasable now.
    Ready,
    /// Claimed by a scheduler, not yet started.
    Leased,
    /// An agent session is working on it.
    Running,
    /// Implementation is done; tests have not reported.
    AwaitingTest,
    /// Tests passed; review has not reported.
    AwaitingReview,
    /// A reviewer asked for changes.
    ChangesRequested,
    /// Failed in a way that may succeed on retry.
    FailedRetryable,
    /// Failed in a way that needs a replan or a human.
    FailedTerminal,
    /// Replaced by a later plan version.
    Superseded,
    /// Its requirements are verified by an independent role.
    Verified,
}

impl TaskState {
    /// The stable wire name.
    pub fn as_str(&self) -> &'static str {
        match self {
            TaskState::Defined => "DEFINED",
            TaskState::Blocked => "BLOCKED",
            TaskState::Ready => "READY",
            TaskState::Leased => "LEASED",
            TaskState::Running => "RUNNING",
            TaskState::AwaitingTest => "AWAITING_TEST",
            TaskState::AwaitingReview => "AWAITING_REVIEW",
            TaskState::ChangesRequested => "CHANGES_REQUESTED",
            TaskState::FailedRetryable => "FAILED_RETRYABLE",
            TaskState::FailedTerminal => "FAILED_TERMINAL",
            TaskState::Superseded => "SUPERSEDED",
            TaskState::Verified => "VERIFIED",
        }
    }

    /// Parse the stable wire name.
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "DEFINED" => Ok(TaskState::Defined),
            "BLOCKED" => Ok(TaskState::Blocked),
            "READY" => Ok(TaskState::Ready),
            "LEASED" => Ok(TaskState::Leased),
            "RUNNING" => Ok(TaskState::Running),
            "AWAITING_TEST" => Ok(TaskState::AwaitingTest),
            "AWAITING_REVIEW" => Ok(TaskState::AwaitingReview),
            "CHANGES_REQUESTED" => Ok(TaskState::ChangesRequested),
            "FAILED_RETRYABLE" => Ok(TaskState::FailedRetryable),
            "FAILED_TERMINAL" => Ok(TaskState::FailedTerminal),
            "SUPERSEDED" => Ok(TaskState::Superseded),
            "VERIFIED" => Ok(TaskState::Verified),
            other => Err(format!("unknown task state: {other}")),
        }
    }

    /// Whether the task has finished for good.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            TaskState::Verified | TaskState::FailedTerminal | TaskState::Superseded
        )
    }

    /// Whether the task is occupying an agent session right now.
    pub fn is_active(&self) -> bool {
        matches!(self, TaskState::Leased | TaskState::Running)
    }

    /// Whether the task is waiting to be released to a scheduler.
    pub fn is_releasable(&self) -> bool {
        matches!(
            self,
            TaskState::Defined
                | TaskState::Blocked
                | TaskState::Ready
                | TaskState::ChangesRequested
                | TaskState::FailedRetryable
        )
    }

    /// Whether dependents may start once this task reaches this state.
    ///
    /// Only independent verification satisfies a dependency: evidence never
    /// outranks the requirement it is evidence for.
    pub fn satisfies_dependents(&self) -> bool {
        matches!(self, TaskState::Verified)
    }

    /// Whether `next` is a legal successor state.
    pub fn can_transition_to(&self, next: TaskState) -> bool {
        use TaskState::*;
        if *self == next {
            return true;
        }
        // A replan may supersede any task that has not already been verified.
        if next == Superseded {
            return *self != Verified;
        }
        match self {
            Defined => matches!(next, Blocked | Ready | FailedTerminal),
            Blocked => matches!(next, Ready | Defined | FailedTerminal),
            Ready => matches!(next, Leased | Blocked | FailedTerminal),
            Leased => matches!(next, Running | Ready | FailedRetryable | FailedTerminal),
            Running => matches!(
                next,
                AwaitingTest | AwaitingReview | FailedRetryable | FailedTerminal
            ),
            AwaitingTest => matches!(
                next,
                AwaitingReview | ChangesRequested | FailedRetryable | FailedTerminal
            ),
            AwaitingReview => matches!(next, Verified | ChangesRequested | FailedTerminal),
            ChangesRequested => matches!(next, Ready | Leased | FailedTerminal),
            FailedRetryable => matches!(next, Ready | Blocked | FailedTerminal),
            FailedTerminal | Superseded | Verified => false,
        }
    }
}

impl fmt::Display for TaskState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// What a task produces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    /// Code, configuration, or documentation changes.
    Implementation,
    /// Tests and adversarial cases.
    Test,
    /// Independent review.
    Review,
    /// Cross-repository integration.
    Integration,
    /// Final verification against the Definition.
    Verification,
    /// Orchestration bookkeeping that satisfies no requirement of its own.
    Meta,
}

impl TaskKind {
    /// The stable wire name.
    pub fn as_str(&self) -> &'static str {
        match self {
            TaskKind::Implementation => "implementation",
            TaskKind::Test => "test",
            TaskKind::Review => "review",
            TaskKind::Integration => "integration",
            TaskKind::Verification => "verification",
            TaskKind::Meta => "meta",
        }
    }

    /// Whether the task must map to at least one requirement.
    pub fn requires_requirement_mapping(&self) -> bool {
        !matches!(self, TaskKind::Meta)
    }

    /// The repository capabilities this kind of task needs by default.
    pub fn default_capabilities(&self) -> BTreeSet<Capability> {
        match self {
            TaskKind::Implementation | TaskKind::Test => Capability::write_set(),
            TaskKind::Integration => BTreeSet::from([Capability::Read, Capability::Workflows]),
            TaskKind::Review | TaskKind::Verification | TaskKind::Meta => {
                BTreeSet::from([Capability::Read])
            }
        }
    }
}

/// A scheduler lease on a task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lease {
    /// The Pi session holding the lease.
    pub session_name: String,
    /// Unix seconds at which the lease expires and the task may be re-released.
    pub expires_at: u64,
}

impl Lease {
    /// Whether the lease is still held at `now`.
    pub fn is_live(&self, now: u64) -> bool {
        now < self.expires_at
    }
}

/// One executable unit of Initiative work.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    /// Stable, globally unique identifier.
    pub id: TaskId,
    /// A short human summary.
    pub summary: String,
    /// The repository the work happens in.
    pub repository: RepositoryId,
    /// What the task produces.
    pub kind: TaskKind,
    /// The agent role the task is dispatched to.
    pub role: AgentRole,
    /// Requirements the task contributes to.
    #[serde(default)]
    pub satisfies: Vec<RequirementId>,
    /// Tasks that must be verified first, by AutoSpec task id.
    #[serde(default)]
    pub depends_on: Vec<TaskId>,
    /// Declared outputs.
    #[serde(default)]
    pub outputs: Vec<String>,
    /// Commands that validate the outputs.
    #[serde(default)]
    pub validation: Vec<String>,
    /// Repository capabilities the task needs before it may be released.
    #[serde(default)]
    pub required_capabilities: BTreeSet<Capability>,
    /// An exclusivity key; two live tasks may not share one.
    #[serde(default)]
    pub exclusivity: Option<String>,
    /// The architecture plan version the task was generated from.
    pub plan_version: u32,
    /// Current lifecycle state.
    pub state: TaskState,
    /// The lease, when one is held.
    #[serde(default)]
    pub lease: Option<Lease>,
    /// How many attempts have been dispatched.
    #[serde(default)]
    pub attempts: u32,
}

impl Task {
    /// A `DEFINED` implementation task with the default capability set.
    pub fn implementation(
        id: TaskId,
        repository: RepositoryId,
        satisfies: Vec<RequirementId>,
        plan_version: u32,
    ) -> Self {
        Self {
            summary: format!("implement {id}"),
            id,
            repository,
            kind: TaskKind::Implementation,
            role: AgentRole::Implementer,
            satisfies,
            depends_on: Vec::new(),
            outputs: Vec::new(),
            validation: Vec::new(),
            required_capabilities: TaskKind::Implementation.default_capabilities(),
            exclusivity: None,
            plan_version,
            state: TaskState::Defined,
            lease: None,
            attempts: 0,
        }
    }

    /// The same task with dependencies attached.
    pub fn depending_on(mut self, dependencies: Vec<TaskId>) -> Self {
        self.depends_on = dependencies;
        self
    }

    /// The same task with a different kind and role.
    pub fn as_kind(mut self, kind: TaskKind, role: AgentRole) -> Self {
        self.required_capabilities = kind.default_capabilities();
        self.summary = format!("{} {}", kind.as_str(), self.id);
        self.kind = kind;
        self.role = role;
        self
    }

    /// Move to `next`, rejecting an illegal transition.
    pub fn transition_to(&mut self, next: TaskState) -> Result<(), String> {
        if !self.state.can_transition_to(next) {
            return Err(format!(
                "{}: illegal transition {} -> {}",
                self.id, self.state, next
            ));
        }
        if next != TaskState::Leased {
            self.lease = None;
        }
        self.state = next;
        Ok(())
    }

    /// Claim the task for `session_name` until `expires_at`.
    pub fn lease(&mut self, session_name: impl Into<String>, expires_at: u64) -> Result<(), String> {
        if self.state != TaskState::Ready {
            return Err(format!("{} is not READY to lease ({})", self.id, self.state));
        }
        self.state = TaskState::Leased;
        self.attempts += 1;
        self.lease = Some(Lease {
            session_name: session_name.into(),
            expires_at,
        });
        Ok(())
    }

    /// Whether an expired lease should be reclaimed at `now`.
    pub fn lease_expired(&self, now: u64) -> bool {
        matches!(&self.lease, Some(lease) if !lease.is_live(now))
    }

    /// Reject a task that cannot be scheduled.
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut problems = Vec::new();
        if self.kind.requires_requirement_mapping() && self.satisfies.is_empty() {
            problems.push(format!(
                "{} is a {} task and must map to at least one requirement",
                self.id,
                self.kind.as_str()
            ));
        }
        if self.depends_on.contains(&self.id) {
            problems.push(format!("{} depends on itself", self.id));
        }
        if self.plan_version == 0 {
            problems.push(format!("{} has no architecture plan version", self.id));
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

    fn task_id(text: &str) -> TaskId {
        TaskId::parse(text).expect("valid task id")
    }

    fn repository() -> RepositoryId {
        RepositoryId::parse("github.com/InferWeave/autospec").expect("valid repository id")
    }

    fn requirement(text: &str) -> RequirementId {
        RequirementId::parse(text).expect("valid requirement id")
    }

    fn task() -> Task {
        Task::implementation(
            task_id("TASK-0001"),
            repository(),
            vec![requirement("REQ-001")],
            1,
        )
    }

    #[test]
    fn task_states_round_trip_through_their_wire_names() {
        for state in [
            TaskState::Defined,
            TaskState::Blocked,
            TaskState::Ready,
            TaskState::Leased,
            TaskState::Running,
            TaskState::AwaitingTest,
            TaskState::AwaitingReview,
            TaskState::ChangesRequested,
            TaskState::FailedRetryable,
            TaskState::FailedTerminal,
            TaskState::Superseded,
            TaskState::Verified,
        ] {
            assert_eq!(TaskState::parse(state.as_str()), Ok(state));
        }
    }

    #[test]
    fn only_verification_satisfies_a_dependent_task() {
        assert!(TaskState::Verified.satisfies_dependents());
        assert!(!TaskState::AwaitingReview.satisfies_dependents());
        assert!(!TaskState::Running.satisfies_dependents());
    }

    #[test]
    fn the_happy_path_transitions_are_legal() {
        let mut task = task();

        for next in [
            TaskState::Ready,
            TaskState::Leased,
            TaskState::Running,
            TaskState::AwaitingTest,
            TaskState::AwaitingReview,
            TaskState::Verified,
        ] {
            task.transition_to(next).expect("legal transition");
        }

        assert_eq!(task.state, TaskState::Verified);
    }

    #[test]
    fn implementation_may_not_declare_itself_verified() {
        let mut task = task();
        task.state = TaskState::Running;

        let error = task
            .transition_to(TaskState::Verified)
            .expect_err("running work cannot self-verify");

        assert!(error.contains("illegal transition"), "{error}");
    }

    #[test]
    fn a_verified_task_is_never_superseded_by_a_replan() {
        let mut task = task();
        task.state = TaskState::Verified;

        assert!(task.transition_to(TaskState::Superseded).is_err());
    }

    #[test]
    fn an_unverified_task_may_be_superseded_by_a_replan() {
        let mut task = task();
        task.state = TaskState::Running;

        task.transition_to(TaskState::Superseded)
            .expect("a replan supersedes in-flight work");
    }

    #[test]
    fn leasing_requires_a_ready_task_and_counts_the_attempt() {
        let mut task = task();

        assert!(task.lease("aspec-INIT-0042-TASK-0001-impl-a1", 100).is_err());

        task.transition_to(TaskState::Ready).expect("ready");
        task.lease("aspec-INIT-0042-TASK-0001-impl-a1", 100)
            .expect("leasable");

        assert_eq!(task.state, TaskState::Leased);
        assert_eq!(task.attempts, 1);
        assert!(!task.lease_expired(99));
        assert!(task.lease_expired(100));
    }

    #[test]
    fn leaving_the_leased_state_drops_the_lease() {
        let mut task = task();
        task.transition_to(TaskState::Ready).expect("ready");
        task.lease("session", 100).expect("leasable");

        task.transition_to(TaskState::Running).expect("running");

        assert!(task.lease.is_none());
    }

    #[test]
    fn a_non_meta_task_must_map_to_a_requirement() {
        let mut task = task();
        task.satisfies.clear();

        let problems = task.validate().expect_err("unmapped work is rejected");

        assert!(problems[0].contains("must map to at least one requirement"));
    }

    #[test]
    fn a_meta_task_needs_no_requirement_mapping() {
        let mut task = task().as_kind(TaskKind::Meta, AgentRole::Architect);
        task.satisfies.clear();

        task.validate().expect("meta tasks are exempt");
        assert_eq!(
            task.required_capabilities,
            BTreeSet::from([Capability::Read])
        );
    }
}
