//! Role tool policies and bounded task capsules (AAR spec section 9).
//!
//! A role policy names the tool surface a session may use. Policies are
//! provider-neutral: AutoSpec picks one per role and the harness adapter
//! (Pi today) maps it onto its own flag surface, so AutoSpec callers never
//! spell out a harness tool name.
//!
//! A task capsule is the minimum complete briefing for one role session:
//! goal, acceptance criteria, constraints, relevant files, tests and
//! non-goals. Capsules are bounded so a session prompt stays compact, and a
//! reviewer capsule never carries the builder's reasoning transcript: the
//! reviewer must judge the change, not the builder's story about it.

use super::topology::AgentRole;

/// A provider-neutral role tool policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RolePolicy {
    /// Read the codebase and produce the plan. Never edits.
    Planner,
    /// Explore and gather evidence. Never edits.
    Scout,
    /// Read code and run tests. Edits nothing.
    Test,
    /// Judge a change independently. Never edits.
    Reviewer,
    /// Read, edit, write and run commands. The only mutating policy.
    Builder,
    /// Gather diagnostics to hand the decision to a human. Edits nothing.
    Escalation,
}

impl RolePolicy {
    /// The tool surface this role may use, in stable order.
    pub const fn tools(self) -> &'static [&'static str] {
        match self {
            RolePolicy::Planner => &["read", "grep", "glob"],
            RolePolicy::Scout => &["read", "grep", "glob"],
            RolePolicy::Test => &["read", "grep", "glob", "bash"],
            RolePolicy::Reviewer => &["read", "grep", "glob"],
            RolePolicy::Builder => &["read", "grep", "glob", "edit", "write", "bash"],
            RolePolicy::Escalation => &["read", "grep", "glob", "bash"],
        }
    }

    /// Read-only policies may inspect and run, but never mutate files.
    pub fn is_read_only(self) -> bool {
        !self.tools().contains(&"edit") && !self.tools().contains(&"write")
    }

    /// The policy for a dispatchable role.
    pub const fn for_role(role: AgentRole) -> Self {
        match role {
            AgentRole::Coordinator => RolePolicy::Escalation,
            AgentRole::Explorer => RolePolicy::Scout,
            AgentRole::Planner => RolePolicy::Planner,
            AgentRole::Implementer | AgentRole::DocumentationWriter => RolePolicy::Builder,
            AgentRole::Tester => RolePolicy::Test,
            AgentRole::Reviewer
            | AgentRole::UiEvaluator
            | AgentRole::SecurityReviewer
            | AgentRole::PerformanceReviewer => RolePolicy::Reviewer,
        }
    }
}

/// Maximum characters for the capsule goal.
pub const MAX_GOAL_CHARS: usize = 500;
/// Maximum items in any single capsule section.
pub const MAX_SECTION_ITEMS: usize = 32;
/// Maximum characters per section item.
pub const MAX_SECTION_ITEM_CHARS: usize = 400;

/// The minimum complete briefing for one role session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskCapsule {
    pub role: AgentRole,
    pub goal: String,
    pub acceptance_criteria: Vec<String>,
    pub constraints: Vec<String>,
    pub relevant_files: Vec<String>,
    pub tests: Vec<String>,
    pub non_goals: Vec<String>,
    /// The builder's reasoning transcript. Present on builder capsules only;
    /// [`TaskCapsule::for_reviewer`] strips it before a reviewer sees the
    /// capsule.
    pub builder_reasoning: Option<String>,
}

impl TaskCapsule {
    pub fn new(role: AgentRole, goal: impl Into<String>) -> Self {
        Self {
            role,
            goal: goal.into(),
            acceptance_criteria: Vec::new(),
            constraints: Vec::new(),
            relevant_files: Vec::new(),
            tests: Vec::new(),
            non_goals: Vec::new(),
            builder_reasoning: None,
        }
    }

    pub fn with_acceptance_criteria(
        mut self,
        items: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.acceptance_criteria = items.into_iter().map(Into::into).collect();
        self
    }

    pub fn with_constraints(mut self, items: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.constraints = items.into_iter().map(Into::into).collect();
        self
    }

    pub fn with_relevant_files(
        mut self,
        items: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.relevant_files = items.into_iter().map(Into::into).collect();
        self
    }

    pub fn with_tests(mut self, items: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.tests = items.into_iter().map(Into::into).collect();
        self
    }

    pub fn with_non_goals(mut self, items: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.non_goals = items.into_iter().map(Into::into).collect();
        self
    }

    pub fn with_builder_reasoning(mut self, transcript: impl Into<String>) -> Self {
        self.builder_reasoning = Some(transcript.into());
        self
    }

    /// Check the capsule stays inside its bounds.
    pub fn validate(&self) -> Result<(), String> {
        let goal = self.goal.trim();
        if goal.is_empty() {
            return Err("task capsule requires a goal".to_string());
        }
        if goal.chars().count() > MAX_GOAL_CHARS {
            return Err(format!(
                "task capsule goal exceeds {MAX_GOAL_CHARS} characters"
            ));
        }
        Self::check_section("acceptance criteria", &self.acceptance_criteria)?;
        Self::check_section("constraints", &self.constraints)?;
        Self::check_section("relevant files", &self.relevant_files)?;
        Self::check_section("tests", &self.tests)?;
        Self::check_section("non-goals", &self.non_goals)?;
        if self
            .builder_reasoning
            .as_deref()
            .is_some_and(|transcript| transcript.trim().is_empty())
        {
            return Err("builder reasoning must not be blank".to_string());
        }
        Ok(())
    }

    fn check_section(name: &str, items: &[String]) -> Result<(), String> {
        if items.len() > MAX_SECTION_ITEMS {
            return Err(format!("{name} exceeds {MAX_SECTION_ITEMS} items"));
        }
        for item in items {
            let item = item.trim();
            if item.is_empty() {
                return Err(format!("{name} contains an empty item"));
            }
            if item.chars().count() > MAX_SECTION_ITEM_CHARS {
                return Err(format!(
                    "{name} item exceeds {MAX_SECTION_ITEM_CHARS} characters"
                ));
            }
        }
        Ok(())
    }

    /// The reviewer view of this capsule: same scope, reviewer role, and the
    /// builder's reasoning transcript removed.
    pub fn for_reviewer(&self) -> TaskCapsule {
        let mut capsule = TaskCapsule {
            role: AgentRole::Reviewer,
            ..self.clone()
        };
        capsule.builder_reasoning = None;
        capsule
    }

    /// Render the capsule as the compact block a session prompt embeds.
    pub fn render(&self) -> String {
        let mut rendered = format!(
            "# Task capsule: {}\n\n## Goal\n{}\n",
            self.role.as_str(),
            self.goal.trim()
        );
        rendered.push_str(&Self::render_section(
            "Acceptance criteria",
            &self.acceptance_criteria,
        ));
        rendered.push_str(&Self::render_section("Constraints", &self.constraints));
        rendered.push_str(&Self::render_section(
            "Relevant files",
            &self.relevant_files,
        ));
        rendered.push_str(&Self::render_section("Tests", &self.tests));
        rendered.push_str(&Self::render_section("Non-goals", &self.non_goals));
        if let Some(transcript) = &self.builder_reasoning {
            rendered.push_str(&format!("\n## Builder reasoning\n{}\n", transcript.trim()));
        }
        rendered
    }

    fn render_section(title: &str, items: &[String]) -> String {
        let mut section = format!("\n## {title}\n");
        if items.is_empty() {
            section.push_str("_none_\n");
        } else {
            for item in items {
                section.push_str(&format!("- {}\n", item.trim()));
            }
        }
        section
    }
}
