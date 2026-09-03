//! Agent roles, Pi session identity, and separation of duties.
//!
//! Implementation is judged against the requirements by a role and session
//! that did not produce it (architectural invariant 4). The rules hold under
//! model fallback: falling back to another model may never collapse two roles
//! into one session.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use super::ids::{AttemptId, InitiativeId, TaskId};

/// A standard AutoSpec agent role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRole {
    /// Creates and refines specifications when asked.
    SpecAuthor,
    /// Plans implementation strategy across repositories.
    Architect,
    /// Plans one task against its current worktree.
    TaskPlanner,
    /// Changes code, configuration, and documentation.
    Implementer,
    /// Writes tests, adversarial cases, and regression validation.
    TestEngineer,
    /// Reviews vision, browser, and UI acceptance.
    UxReviewer,
    /// Reviews code and architecture independently.
    Reviewer,
    /// Verifies the final result against `REQ-*` and `AC-*`.
    SpecVerifier,
}

impl AgentRole {
    /// The stable wire name.
    pub fn as_str(&self) -> &'static str {
        match self {
            AgentRole::SpecAuthor => "spec-author",
            AgentRole::Architect => "architect",
            AgentRole::TaskPlanner => "task-planner",
            AgentRole::Implementer => "implementer",
            AgentRole::TestEngineer => "test-engineer",
            AgentRole::UxReviewer => "ux-reviewer",
            AgentRole::Reviewer => "reviewer",
            AgentRole::SpecVerifier => "spec-verifier",
        }
    }

    /// The short token used inside Pi session names.
    pub fn session_token(&self) -> &'static str {
        match self {
            AgentRole::SpecAuthor => "spec",
            AgentRole::Architect => "arch",
            AgentRole::TaskPlanner => "plan",
            AgentRole::Implementer => "impl",
            AgentRole::TestEngineer => "test",
            AgentRole::UxReviewer => "ux",
            AgentRole::Reviewer => "rev",
            AgentRole::SpecVerifier => "verify",
        }
    }

    /// Parse the stable wire name.
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "spec-author" => Ok(AgentRole::SpecAuthor),
            "architect" => Ok(AgentRole::Architect),
            "task-planner" => Ok(AgentRole::TaskPlanner),
            "implementer" => Ok(AgentRole::Implementer),
            "test-engineer" => Ok(AgentRole::TestEngineer),
            "ux-reviewer" => Ok(AgentRole::UxReviewer),
            "reviewer" => Ok(AgentRole::Reviewer),
            "spec-verifier" => Ok(AgentRole::SpecVerifier),
            other => Err(format!("unknown agent role: {other}")),
        }
    }

    /// Every standard role, in lifecycle order.
    pub fn all() -> [AgentRole; 8] {
        [
            AgentRole::SpecAuthor,
            AgentRole::Architect,
            AgentRole::TaskPlanner,
            AgentRole::Implementer,
            AgentRole::TestEngineer,
            AgentRole::UxReviewer,
            AgentRole::Reviewer,
            AgentRole::SpecVerifier,
        ]
    }

    /// Whether the role produces the work under judgement.
    pub fn is_producing(&self) -> bool {
        matches!(self, AgentRole::Implementer)
    }

    /// Whether the role judges work produced by another role.
    pub fn is_judging(&self) -> bool {
        matches!(
            self,
            AgentRole::Reviewer | AgentRole::UxReviewer | AgentRole::SpecVerifier
        )
    }
}

impl fmt::Display for AgentRole {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// The identity of one Pi session.
///
/// `session_name` is the address of the process the role actually ran in. It
/// is derived by default, but a dispatch that reuses an existing session can
/// say so, which is exactly what the separation checks look for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionIdentity {
    /// The Initiative the session belongs to.
    pub initiative: InitiativeId,
    /// The task the session works on, when the role is task-scoped.
    #[serde(default)]
    pub task: Option<TaskId>,
    /// The attempt this session belongs to.
    pub attempt: AttemptId,
    /// The role the session was invoked with.
    pub role: AgentRole,
    /// The model actually selected for the session.
    pub model: String,
    /// The Pi session the role ran in.
    pub session_name: String,
}

impl SessionIdentity {
    /// Build a task-scoped session identity with a freshly derived session.
    pub fn for_task(
        initiative: InitiativeId,
        task: TaskId,
        attempt: AttemptId,
        role: AgentRole,
        model: impl Into<String>,
    ) -> Self {
        let session_name = derive_session_name(&initiative, Some(&task), role, &attempt);
        Self {
            initiative,
            task: Some(task),
            attempt,
            role,
            model: model.into(),
            session_name,
        }
    }

    /// Build an Initiative-scoped session identity, for architecture and
    /// verification roles that are not task-local.
    pub fn for_initiative(
        initiative: InitiativeId,
        attempt: AttemptId,
        role: AgentRole,
        model: impl Into<String>,
    ) -> Self {
        let session_name = derive_session_name(&initiative, None, role, &attempt);
        Self {
            initiative,
            task: None,
            attempt,
            role,
            model: model.into(),
            session_name,
        }
    }

    /// The same role and scope, dispatched into an existing session.
    ///
    /// This is how a collapsed dispatch is represented; the policy below is
    /// what decides whether it is allowed.
    pub fn reusing_session(mut self, session_name: impl Into<String>) -> Self {
        self.session_name = session_name.into();
        self
    }

    /// The Pi session the role ran in.
    pub fn session_name(&self) -> &str {
        &self.session_name
    }
}

/// The canonical Pi session name, e.g. `aspec-INIT-0042-TASK-0017-impl-a3`.
pub fn derive_session_name(
    initiative: &InitiativeId,
    task: Option<&TaskId>,
    role: AgentRole,
    attempt: &AttemptId,
) -> String {
    let scope = match task {
        Some(task) => task.as_str(),
        None => "initiative",
    };
    format!(
        "aspec-{}-{}-{}-a{}",
        initiative.short(),
        scope,
        role.session_token(),
        attempt.sequence()
    )
}

/// A separation of duties rule that was broken.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "rule", content = "detail")]
pub enum SeparationViolation {
    /// A judging role reused the implementation session.
    JudgeSharesImplementerSession {
        /// The judging role.
        role: AgentRole,
        /// The session both roles ran in.
        session_name: String,
    },
    /// Final verification ran in a session that also implemented.
    VerifierImplemented {
        /// The session both roles ran in.
        session_name: String,
    },
    /// Planning and implementation shared a session.
    PlannerSharesImplementerSession {
        /// The session both roles ran in.
        session_name: String,
    },
    /// Testing and implementation shared a session.
    TesterSharesImplementerSession {
        /// The session both roles ran in.
        session_name: String,
    },
}

impl SeparationViolation {
    /// A human-readable explanation.
    pub fn message(&self) -> String {
        match self {
            SeparationViolation::JudgeSharesImplementerSession { role, session_name } => {
                format!("{role} may not judge work produced by its own session {session_name}")
            }
            SeparationViolation::VerifierImplemented { session_name } => format!(
                "final verification may not run in implementation session {session_name}"
            ),
            SeparationViolation::PlannerSharesImplementerSession { session_name } => {
                format!("planning and implementation may not share session {session_name}")
            }
            SeparationViolation::TesterSharesImplementerSession { session_name } => {
                format!("testing and implementation may not share session {session_name}")
            }
        }
    }
}

/// How strongly a rule is enforced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Enforcement {
    /// The dispatch is refused.
    Blocking,
    /// The dispatch is recorded with a warning.
    Advisory,
}

/// The outcome of checking a set of sessions against the separation rules.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeparationReport {
    /// Violations that block the dispatch.
    pub blocking: Vec<SeparationViolation>,
    /// Violations that are recorded but permitted.
    pub advisory: Vec<SeparationViolation>,
}

impl SeparationReport {
    /// Whether the checked sessions may proceed.
    pub fn is_permitted(&self) -> bool {
        self.blocking.is_empty()
    }
}

/// The separation of duties policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeparationPolicy {
    /// How planner/implementer sharing is treated; the specification says SHOULD.
    pub planner_separation: Enforcement,
    /// How tester/implementer sharing is treated; the specification says SHOULD.
    pub tester_separation: Enforcement,
}

impl Default for SeparationPolicy {
    fn default() -> Self {
        Self {
            planner_separation: Enforcement::Advisory,
            tester_separation: Enforcement::Advisory,
        }
    }
}

impl SeparationPolicy {
    /// A policy that blocks the SHOULD-level rules too.
    pub fn strict() -> Self {
        Self {
            planner_separation: Enforcement::Blocking,
            tester_separation: Enforcement::Blocking,
        }
    }

    /// Check a set of sessions that acted on the same work.
    pub fn check(&self, sessions: &[SessionIdentity]) -> SeparationReport {
        let mut by_name: BTreeMap<&str, Vec<&SessionIdentity>> = BTreeMap::new();
        for session in sessions {
            by_name
                .entry(session.session_name())
                .or_default()
                .push(session);
        }

        let mut report = SeparationReport::default();
        for (session_name, grouped) in by_name {
            if !grouped.iter().any(|session| session.role.is_producing()) {
                continue;
            }
            for session in &grouped {
                self.record(session.role, session_name, &mut report);
            }
        }
        report
    }

    /// Record whatever rule `role` breaks by sharing an implementation session.
    fn record(&self, role: AgentRole, session_name: &str, report: &mut SeparationReport) {
        let session_name = session_name.to_string();
        match role {
            AgentRole::SpecVerifier => {
                report
                    .blocking
                    .push(SeparationViolation::VerifierImplemented { session_name });
            }
            AgentRole::Reviewer | AgentRole::UxReviewer => {
                report
                    .blocking
                    .push(SeparationViolation::JudgeSharesImplementerSession { role, session_name });
            }
            AgentRole::TaskPlanner | AgentRole::Architect => {
                push(
                    self.planner_separation,
                    SeparationViolation::PlannerSharesImplementerSession { session_name },
                    report,
                );
            }
            AgentRole::TestEngineer => {
                push(
                    self.tester_separation,
                    SeparationViolation::TesterSharesImplementerSession { session_name },
                    report,
                );
            }
            AgentRole::Implementer | AgentRole::SpecAuthor => {}
        }
    }

    /// Whether `role` may run on `model` given the sessions already dispatched.
    ///
    /// Model fallback narrows the eligible set; it may never reuse the session
    /// that produced the work under judgement.
    pub fn permits_dispatch(
        &self,
        candidate: &SessionIdentity,
        existing: &[SessionIdentity],
    ) -> SeparationReport {
        let mut sessions = existing.to_vec();
        sessions.push(candidate.clone());
        self.check(&sessions)
    }
}

/// File a violation as blocking or advisory according to `enforcement`.
fn push(
    enforcement: Enforcement,
    violation: SeparationViolation,
    report: &mut SeparationReport,
) {
    match enforcement {
        Enforcement::Blocking => report.blocking.push(violation),
        Enforcement::Advisory => report.advisory.push(violation),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn initiative() -> InitiativeId {
        InitiativeId::parse("INIT-2026-0042").expect("valid initiative id")
    }

    fn task() -> TaskId {
        TaskId::parse("TASK-0017").expect("valid task id")
    }

    fn session(role: AgentRole, attempt: u32, model: &str) -> SessionIdentity {
        SessionIdentity::for_task(
            initiative(),
            task(),
            AttemptId::from_sequence(attempt, 3),
            role,
            model,
        )
    }

    #[test]
    fn session_names_follow_the_invocation_contract() {
        let identity = session(AgentRole::Implementer, 3, "coding-high");

        assert_eq!(identity.session_name(), "aspec-INIT-0042-TASK-0017-impl-a3");
    }

    #[test]
    fn initiative_scoped_sessions_name_the_initiative_instead_of_a_task() {
        let identity = SessionIdentity::for_initiative(
            initiative(),
            AttemptId::from_sequence(1, 3),
            AgentRole::Architect,
            "reasoning-high",
        );

        assert_eq!(identity.session_name(), "aspec-INIT-0042-initiative-arch-a1");
    }

    #[test]
    fn distinct_sessions_for_implementation_and_review_are_permitted() {
        let report = SeparationPolicy::default().check(&[
            session(AgentRole::Implementer, 3, "coding-high"),
            session(AgentRole::Reviewer, 4, "reasoning-high"),
        ]);

        assert!(report.is_permitted());
        assert!(report.advisory.is_empty());
    }

    #[test]
    fn a_session_that_implements_and_verifies_is_blocked() {
        let implementer = session(AgentRole::Implementer, 3, "coding-high");
        let verifier = session(AgentRole::SpecVerifier, 4, "reasoning-high")
            .reusing_session(implementer.session_name());

        let report = SeparationPolicy::default().check(&[implementer, verifier]);

        assert!(!report.is_permitted());
        assert!(report.blocking[0].message().contains("final verification"));
    }

    #[test]
    fn model_fallback_may_not_collapse_a_review_into_the_implementer_session() {
        let implementer = session(AgentRole::Implementer, 3, "coding-high");
        let fallback_reviewer = session(AgentRole::Reviewer, 4, "coding-standard")
            .reusing_session(implementer.session_name());

        let report =
            SeparationPolicy::default().permits_dispatch(&fallback_reviewer, &[implementer]);

        assert!(!report.is_permitted());
        assert!(matches!(
            report.blocking[0],
            SeparationViolation::JudgeSharesImplementerSession {
                role: AgentRole::Reviewer,
                ..
            }
        ));
    }

    #[test]
    fn a_ux_review_sharing_the_implementation_session_is_blocked() {
        let implementer = session(AgentRole::Implementer, 3, "coding-high");
        let ux = session(AgentRole::UxReviewer, 5, "vision-high")
            .reusing_session(implementer.session_name());

        let report = SeparationPolicy::default().check(&[implementer, ux]);

        assert!(!report.is_permitted());
    }

    #[test]
    fn planner_and_tester_sharing_are_advisory_by_default_and_blocking_under_strict_policy() {
        let implementer = session(AgentRole::Implementer, 3, "coding-high");
        let sessions = vec![
            implementer.clone(),
            session(AgentRole::TaskPlanner, 2, "reasoning-high")
                .reusing_session(implementer.session_name()),
            session(AgentRole::TestEngineer, 6, "coding-standard")
                .reusing_session(implementer.session_name()),
        ];

        let default_report = SeparationPolicy::default().check(&sessions);
        let strict_report = SeparationPolicy::strict().check(&sessions);

        assert!(default_report.is_permitted());
        assert_eq!(default_report.advisory.len(), 2);
        assert_eq!(strict_report.blocking.len(), 2);
        assert!(!strict_report.is_permitted());
    }

    #[test]
    fn sessions_without_an_implementer_are_never_flagged() {
        let report = SeparationPolicy::strict().check(&[
            session(AgentRole::TaskPlanner, 1, "reasoning-high"),
            session(AgentRole::Reviewer, 1, "reasoning-high")
                .reusing_session("aspec-INIT-0042-TASK-0017-plan-a1"),
        ]);

        assert!(report.is_permitted());
        assert!(report.advisory.is_empty());
    }

    #[test]
    fn roles_round_trip_through_their_wire_names() {
        for role in AgentRole::all() {
            assert_eq!(AgentRole::parse(role.as_str()), Ok(role));
            assert!(role.is_producing() != role.is_judging() || !role.is_producing());
        }
    }
}
