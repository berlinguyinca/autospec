//! Agent topology and separation of duties (AAR spec section 8).
//!
//! Separation of duties is enforced programmatically rather than by prompt
//! prose: the same model instance may not both implement and independently
//! review a change, and no capacity or quota failure is allowed to weaken that.

use std::collections::BTreeSet;

use super::classify::{Complexity, Risk, TaskClass, TaskClassification};

/// Roles an AAR execution can dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AgentRole {
    Coordinator,
    Explorer,
    Planner,
    Implementer,
    Tester,
    Reviewer,
    DocumentationWriter,
    UiEvaluator,
    SecurityReviewer,
    PerformanceReviewer,
}

impl AgentRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            AgentRole::Coordinator => "coordinator",
            AgentRole::Explorer => "explorer",
            AgentRole::Planner => "planner",
            AgentRole::Implementer => "implementer",
            AgentRole::Tester => "tester",
            AgentRole::Reviewer => "reviewer",
            AgentRole::DocumentationWriter => "documentation_writer",
            AgentRole::UiEvaluator => "ui_evaluator",
            AgentRole::SecurityReviewer => "security_reviewer",
            AgentRole::PerformanceReviewer => "performance_reviewer",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value.trim().to_ascii_lowercase().replace('-', "_").as_str() {
            "coordinator" => AgentRole::Coordinator,
            "explorer" => AgentRole::Explorer,
            "planner" => AgentRole::Planner,
            "implementer" => AgentRole::Implementer,
            "tester" => AgentRole::Tester,
            "reviewer" => AgentRole::Reviewer,
            "documentation_writer" => AgentRole::DocumentationWriter,
            "ui_evaluator" => AgentRole::UiEvaluator,
            "security_reviewer" => AgentRole::SecurityReviewer,
            "performance_reviewer" => AgentRole::PerformanceReviewer,
            _ => return None,
        })
    }

    /// Roles that produce the change under review.
    pub fn is_producer(&self) -> bool {
        matches!(
            self,
            AgentRole::Implementer | AgentRole::DocumentationWriter
        )
    }

    /// Roles that must judge the change independently of who produced it.
    pub fn is_independent_reviewer(&self) -> bool {
        matches!(
            self,
            AgentRole::Reviewer
                | AgentRole::SecurityReviewer
                | AgentRole::PerformanceReviewer
                | AgentRole::UiEvaluator
        )
    }
}

/// How agents pass work to each other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandoffStyle {
    /// Structured summary and artifacts only. The default.
    StructuredSummary,
    /// Whole transcript. Only for debugging an agent, never for routine work.
    FullTranscript,
}

impl HandoffStyle {
    pub fn as_str(&self) -> &'static str {
        match self {
            HandoffStyle::StructuredSummary => "structured_summary",
            HandoffStyle::FullTranscript => "full_transcript",
        }
    }
}

/// The chosen set of roles and how they communicate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentTopology {
    pub roles: Vec<AgentRole>,
    pub isolated_contexts: bool,
    pub handoff: HandoffStyle,
    pub reasons: Vec<String>,
}

impl AgentTopology {
    pub fn is_single_agent(&self) -> bool {
        self.roles.len() <= 1
    }

    pub fn contains(&self, role: AgentRole) -> bool {
        self.roles.contains(&role)
    }
}

/// Choose single- or multi-agent execution for one classified unit.
pub fn select_topology(classification: &TaskClassification) -> AgentTopology {
    let mut roles: BTreeSet<AgentRole> = BTreeSet::new();
    let mut reasons = Vec::new();

    roles.insert(AgentRole::Implementer);

    if classification.task_class == TaskClass::Docs {
        roles.remove(&AgentRole::Implementer);
        roles.insert(AgentRole::DocumentationWriter);
        reasons.push("documentation writer replaces implementer for docs work".to_string());
    }

    if classification.complexity <= Complexity::Trivial && classification.risk == Risk::Low {
        reasons.push("single agent: trivial complexity and low risk".to_string());
        return AgentTopology {
            roles: roles.into_iter().collect(),
            isolated_contexts: false,
            handoff: HandoffStyle::StructuredSummary,
            reasons,
        };
    }

    roles.insert(AgentRole::Reviewer);
    reasons.push("independent reviewer required above trivial complexity".to_string());

    if classification.complexity >= Complexity::Medium {
        roles.insert(AgentRole::Explorer);
        roles.insert(AgentRole::Tester);
        reasons.push("explorer and tester added at medium complexity or above".to_string());
    }

    if classification.complexity >= Complexity::High
        || classification.task_class == TaskClass::Migration
    {
        roles.insert(AgentRole::Planner);
        roles.insert(AgentRole::Coordinator);
        reasons.push("planner and coordinator added for high-complexity or migration work".to_string());
    }

    if classification.risk >= Risk::Critical {
        roles.insert(AgentRole::SecurityReviewer);
        reasons.push("security reviewer added at critical risk".to_string());
    }

    if classification.requires_vision || classification.task_class == TaskClass::Ui {
        roles.insert(AgentRole::UiEvaluator);
        reasons.push("UI evaluator added for vision or UI work".to_string());
    }

    if classification.task_class == TaskClass::Research {
        roles.insert(AgentRole::Explorer);
        roles.insert(AgentRole::Planner);
        reasons.push("research runs explorer plus planner".to_string());
    }

    let roles: Vec<AgentRole> = roles.into_iter().collect();
    let isolated_contexts = roles.len() > 1;

    AgentTopology {
        roles,
        isolated_contexts,
        handoff: HandoffStyle::StructuredSummary,
        reasons,
    }
}

/// One role bound to a concrete model instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleAssignment {
    pub role: AgentRole,
    /// Profile key, so two different quantizations of one model are distinct.
    pub model_key: String,
    pub model_class: String,
    /// Distinct session; two roles sharing a session share context.
    pub session_id: String,
}

impl RoleAssignment {
    pub fn new(
        role: AgentRole,
        model_key: impl Into<String>,
        model_class: impl Into<String>,
        session_id: impl Into<String>,
    ) -> Self {
        Self {
            role,
            model_key: model_key.into(),
            model_class: model_class.into(),
            session_id: session_id.into(),
        }
    }
}

/// Which sharing the operator's policy permits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeparationPolicy {
    /// Planner and reviewer may share one eligible higher-class model.
    pub allow_planner_reviewer_sharing: bool,
}

impl Default for SeparationPolicy {
    fn default() -> Self {
        Self {
            allow_planner_reviewer_sharing: true,
        }
    }
}

/// Result of checking a set of assignments against separation rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeparationVerdict {
    pub satisfied: bool,
    pub violations: Vec<String>,
}

impl SeparationVerdict {
    pub fn satisfied() -> Self {
        Self {
            satisfied: true,
            violations: Vec::new(),
        }
    }
}

/// Enforce separation of duties over a set of role assignments.
///
/// The rule is about the *instance*, not the family: an implementer and a
/// reviewer on the same model key, or in the same session, is not an
/// independent review no matter how capable the model is.
pub fn enforce_separation(
    assignments: &[RoleAssignment],
    policy: &SeparationPolicy,
) -> SeparationVerdict {
    let mut violations = Vec::new();

    let producers: Vec<&RoleAssignment> = assignments
        .iter()
        .filter(|assignment| assignment.role.is_producer())
        .collect();
    let reviewers: Vec<&RoleAssignment> = assignments
        .iter()
        .filter(|assignment| assignment.role.is_independent_reviewer())
        .collect();

    for producer in &producers {
        for reviewer in &reviewers {
            if producer.model_key == reviewer.model_key {
                violations.push(format!(
                    "{} and {} share model instance {}",
                    producer.role.as_str(),
                    reviewer.role.as_str(),
                    producer.model_key
                ));
            }
            if producer.session_id == reviewer.session_id {
                violations.push(format!(
                    "{} and {} share session {}",
                    producer.role.as_str(),
                    reviewer.role.as_str(),
                    producer.session_id
                ));
            }
        }
    }

    if !policy.allow_planner_reviewer_sharing {
        let planners: Vec<&RoleAssignment> = assignments
            .iter()
            .filter(|assignment| assignment.role == AgentRole::Planner)
            .collect();
        for planner in planners {
            for reviewer in &reviewers {
                if planner.model_key == reviewer.model_key {
                    violations.push(format!(
                        "planner and {} share model instance {} while sharing is disabled",
                        reviewer.role.as_str(),
                        planner.model_key
                    ));
                }
            }
        }
    }

    if !producers.is_empty() && reviewers.is_empty() && assignments.len() > 1 {
        violations.push("multi-agent topology has a producer but no independent reviewer".to_string());
    }

    violations.sort();
    violations.dedup();
    SeparationVerdict {
        satisfied: violations.is_empty(),
        violations,
    }
}

/// Confirm a post-fallback assignment set is no weaker than the original.
///
/// Quota and capacity failures are the usual way separation quietly erodes, so
/// this is checked explicitly at every fallback rather than assumed.
pub fn preserves_separation_after_fallback(
    before: &[RoleAssignment],
    after: &[RoleAssignment],
    policy: &SeparationPolicy,
) -> SeparationVerdict {
    let before_verdict = enforce_separation(before, policy);
    let after_verdict = enforce_separation(after, policy);
    if before_verdict.satisfied && !after_verdict.satisfied {
        let mut violations = vec!["fallback weakened separation of duties".to_string()];
        violations.extend(after_verdict.violations);
        return SeparationVerdict {
            satisfied: false,
            violations,
        };
    }
    after_verdict
}

/// A structured handoff between two roles: summaries and artifacts, never a
/// whole transcript.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Handoff {
    pub from: AgentRole,
    pub to: AgentRole,
    pub summary: String,
    pub artifacts: Vec<String>,
    pub open_questions: Vec<String>,
}

impl Handoff {
    pub fn new(from: AgentRole, to: AgentRole, summary: impl Into<String>) -> Self {
        Self {
            from,
            to,
            summary: summary.into(),
            artifacts: Vec::new(),
            open_questions: Vec::new(),
        }
    }

    pub fn with_artifacts(mut self, artifacts: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.artifacts = artifacts.into_iter().map(Into::into).collect();
        self
    }

    pub fn with_open_questions(
        mut self,
        questions: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.open_questions = questions.into_iter().map(Into::into).collect();
        self
    }

    pub fn to_markdown(&self) -> String {
        let mut output = format!(
            "# Handoff: {} -> {}\n\n## Summary\n{}\n",
            self.from.as_str(),
            self.to.as_str(),
            self.summary.trim()
        );
        output.push_str("\n## Artifacts\n");
        if self.artifacts.is_empty() {
            output.push_str("_none_\n");
        } else {
            for artifact in &self.artifacts {
                output.push_str(&format!("- {artifact}\n"));
            }
        }
        output.push_str("\n## Open questions\n");
        if self.open_questions.is_empty() {
            output.push_str("_none_\n");
        } else {
            for question in &self.open_questions {
                output.push_str(&format!("- {question}\n"));
            }
        }
        output
    }
}
