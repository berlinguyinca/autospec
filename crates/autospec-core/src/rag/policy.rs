//! Role-specific retrieval policies (spec sections 7 and 20).
//!
//! Section 7 is explicit that a single global top-K configuration must not be
//! used for every agent. A policy is therefore three things: which sources the
//! role prefers and in what order, how many tokens its package may occupy, and
//! how hard it insists on independent verification.

use crate::rag::budget::RetrievalBudget;
use crate::rag::score::Score;
use crate::rag::source::SourceKind;

/// The AutoSpec agent roles that consume retrieval (spec section 7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AgentRole {
    /// Writes specifications.
    Specification,
    /// Turns an accepted specification into a plan.
    Planner,
    /// Writes code in a worktree.
    Implementation,
    /// Reviews a diff against spec and plan.
    Reviewer,
    /// Designs and writes tests.
    Test,
    /// Writes user-facing documentation.
    Documentation,
    /// Verifies rendered UX and accessibility.
    UxVerification,
    /// The low-cost model that scores retrieved evidence.
    RetrievalEvaluator,
}

/// Every role, in a stable order.
pub const ALL_ROLES: [AgentRole; 8] = [
    AgentRole::Specification,
    AgentRole::Planner,
    AgentRole::Implementation,
    AgentRole::Reviewer,
    AgentRole::Test,
    AgentRole::Documentation,
    AgentRole::UxVerification,
    AgentRole::RetrievalEvaluator,
];

impl AgentRole {
    /// Stable wire identifier.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Specification => "spec",
            Self::Planner => "planner",
            Self::Implementation => "implementation",
            Self::Reviewer => "reviewer",
            Self::Test => "test",
            Self::Documentation => "documentation",
            Self::UxVerification => "ux",
            Self::RetrievalEvaluator => "retrieval_evaluator",
        }
    }

    /// Parse a wire identifier.
    pub fn parse(text: &str) -> Result<Self, String> {
        ALL_ROLES
            .iter()
            .copied()
            .find(|role| role.as_str() == text)
            .ok_or_else(|| format!("unknown agent role: {text}"))
    }

    /// The named policy this role runs by default (spec section 51).
    pub const fn policy_name(self) -> &'static str {
        match self {
            Self::Specification => "architecture_first",
            Self::Planner => "interfaces_and_dependencies",
            Self::Implementation => "worktree_local",
            Self::Reviewer => "independent_verification",
            Self::Test => "behavior_and_regression",
            Self::Documentation => "user_visible_surface",
            Self::UxVerification => "rendered_surface",
            Self::RetrievalEvaluator => "minimal",
        }
    }
}

/// A role's retrieval configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetrievalPolicy {
    role: AgentRole,
    name: &'static str,
    priority_sources: Vec<SourceKind>,
    deprioritized_sources: Vec<SourceKind>,
    max_context_tokens: u32,
    sufficiency_threshold: Score,
    independent_verification: bool,
}

impl RetrievalPolicy {
    /// The default policy for a role.
    pub fn for_role(role: AgentRole) -> Self {
        let (priority, deprioritized, tokens) = role_defaults(role);
        Self {
            role,
            name: role.policy_name(),
            priority_sources: priority,
            deprioritized_sources: deprioritized,
            max_context_tokens: tokens,
            sufficiency_threshold: default_sufficiency(role),
            independent_verification: matches!(role, AgentRole::Reviewer | AgentRole::Test),
        }
    }

    /// The role this policy serves.
    pub fn role(&self) -> AgentRole {
        self.role
    }

    /// The policy name from section 51.
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// Sources this role consults first, highest priority first.
    pub fn priority_sources(&self) -> &[SourceKind] {
        &self.priority_sources
    }

    /// Sources this role consults only when the priority sources fall short.
    pub fn deprioritized_sources(&self) -> &[SourceKind] {
        &self.deprioritized_sources
    }

    /// The role's context ceiling (spec section 20).
    pub fn max_context_tokens(&self) -> u32 {
        self.max_context_tokens
    }

    /// The mean relevance the evaluator must reach to call evidence sufficient.
    pub fn sufficiency_threshold(&self) -> Score {
        self.sufficiency_threshold
    }

    /// Return `true` when this role must retrieve its own evidence rather than
    /// inherit another agent's conclusions (spec section 22).
    pub fn requires_independent_verification(&self) -> bool {
        self.independent_verification
    }

    /// Override the context ceiling, for example when the routed model has a
    /// smaller window (spec section 20).
    pub fn with_max_context_tokens(mut self, tokens: u32) -> Self {
        self.max_context_tokens = tokens;
        self
    }

    /// Override the sufficiency threshold.
    pub fn with_sufficiency_threshold(mut self, threshold: Score) -> Self {
        self.sufficiency_threshold = threshold;
        self
    }

    /// Rank a source for this role; lower sorts first.
    ///
    /// Unlisted sources sort after priority sources and before deprioritized
    /// ones: a source the policy never mentions is neither preferred nor
    /// suppressed.
    pub fn source_rank(&self, kind: SourceKind) -> usize {
        if let Some(index) = self
            .priority_sources
            .iter()
            .position(|candidate| *candidate == kind)
        {
            return index;
        }
        if self.deprioritized_sources.contains(&kind) {
            return self.priority_sources.len() + 1;
        }
        self.priority_sources.len()
    }

    /// Narrow a budget to this role's context ceiling.
    pub fn apply_to_budget(&self, budget: &RetrievalBudget) -> RetrievalBudget {
        let mut narrowed = budget.clone();
        narrowed.max_context_tokens = budget.max_context_tokens.min(self.max_context_tokens);
        narrowed
    }
}

fn role_defaults(role: AgentRole) -> (Vec<SourceKind>, Vec<SourceKind>, u32) {
    use SourceKind::{
        Adr, Documentation, GitHub, Memory, Repository, Runtime, Specification, Test, Web,
    };
    match role {
        // Conceptual and contractual knowledge over line-level detail (7.1).
        AgentRole::Specification => (
            vec![Specification, Adr, Documentation, GitHub, Memory, Repository],
            vec![Runtime, Web],
            60_000,
        ),
        // Interfaces, dependencies and the components a change will touch (7.2).
        AgentRole::Planner => (
            vec![Specification, Repository, Adr, Test, Documentation, Memory],
            vec![Web],
            50_000,
        ),
        // Narrow and task-local (7.3).
        AgentRole::Implementation => (
            vec![Repository, Test, Specification, Documentation],
            vec![GitHub, Runtime, Web],
            30_000,
        ),
        // Independently retrieved authority, not the implementer's reasoning (7.4).
        AgentRole::Reviewer => (
            vec![Specification, Repository, Adr, Test, GitHub, Documentation],
            vec![Memory, Web],
            40_000,
        ),
        // Behavior, boundaries and regression history (7.5).
        AgentRole::Test => (
            vec![Specification, Test, Repository, Runtime, Memory],
            vec![GitHub, Web],
            30_000,
        ),
        // The user-visible surface (7.6).
        AgentRole::Documentation => (
            vec![Specification, Documentation, Repository, Adr],
            vec![Runtime, Memory, Web],
            30_000,
        ),
        // Rendered output and design system (7.7).
        AgentRole::UxVerification => (
            vec![Specification, Documentation, Repository, Test, Runtime],
            vec![GitHub, Memory, Web],
            30_000,
        ),
        // Just enough to score what it was handed (section 20).
        AgentRole::RetrievalEvaluator => (vec![], vec![Web], 8_000),
    }
}

fn default_sufficiency(role: AgentRole) -> Score {
    match role {
        // A wrong specification costs every downstream implementer cycle, so it
        // holds the highest bar; an implementation agent working inside an
        // accepted plan can act on less.
        AgentRole::Specification | AgentRole::Reviewer => Score::from_permille(800),
        AgentRole::Planner | AgentRole::Test => Score::from_permille(750),
        AgentRole::Implementation | AgentRole::Documentation | AgentRole::UxVerification => {
            Score::from_permille(700)
        }
        AgentRole::RetrievalEvaluator => Score::from_permille(600),
    }
}

/// The role policies a project runs, with per-role overrides applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicySet {
    policies: Vec<RetrievalPolicy>,
}

impl Default for PolicySet {
    fn default() -> Self {
        Self {
            policies: ALL_ROLES.iter().copied().map(RetrievalPolicy::for_role).collect(),
        }
    }
}

impl PolicySet {
    /// The policy for a role.
    pub fn policy(&self, role: AgentRole) -> &RetrievalPolicy {
        self.policies
            .iter()
            .find(|policy| policy.role() == role)
            .expect("policy set always contains every role")
    }

    /// Replace one role's policy.
    pub fn set(&mut self, policy: RetrievalPolicy) {
        let role = policy.role();
        if let Some(slot) = self
            .policies
            .iter_mut()
            .find(|candidate| candidate.role() == role)
        {
            *slot = policy;
        }
    }

    /// Every policy, in role order.
    pub fn policies(&self) -> &[RetrievalPolicy] {
        &self.policies
    }
}
