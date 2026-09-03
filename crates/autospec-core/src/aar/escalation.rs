//! Fallback and escalation (AAR spec section 13).
//!
//! Two rules make this more than a list: quota and capacity are checked
//! *before* an assignment is made, not after it fails, and every candidate
//! assignment is re-checked against separation of duties. A quota failure that
//! collapses the implementer and the reviewer onto one model is not an
//! acceptable fallback, it is a policy violation with a plausible excuse.

use std::collections::BTreeMap;

use super::profile::{ModelProfileRegistry, ModelRequirements};
use super::reasoning::ReasoningBudget;
use super::topology::{
    preserves_separation_after_fallback, RoleAssignment, SeparationPolicy,
};

/// Rungs of the escalation chain, cheapest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum EscalationStep {
    SameModelLargerBudget,
    AlternateModelInClass,
    HigherModelClass,
    CloudProviderFallback,
    HumanEscalation,
}

impl EscalationStep {
    pub fn as_str(&self) -> &'static str {
        match self {
            EscalationStep::SameModelLargerBudget => "same_model_larger_budget",
            EscalationStep::AlternateModelInClass => "alternate_model_in_class",
            EscalationStep::HigherModelClass => "higher_model_class",
            EscalationStep::CloudProviderFallback => "cloud_provider_fallback",
            EscalationStep::HumanEscalation => "human_escalation",
        }
    }
}

/// The chain to walk, plus its hard invariants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EscalationPolicy {
    pub chain: Vec<EscalationStep>,
    pub max_attempts: u32,
    /// Never relax; kept explicit so a config that tries to is visibly wrong.
    pub preserve_separation: bool,
    /// Ordered model classes, cheapest first, for the higher-class rung.
    pub class_ladder: Vec<String>,
}

impl Default for EscalationPolicy {
    fn default() -> Self {
        Self {
            chain: vec![
                EscalationStep::SameModelLargerBudget,
                EscalationStep::AlternateModelInClass,
                EscalationStep::HigherModelClass,
                EscalationStep::CloudProviderFallback,
                EscalationStep::HumanEscalation,
            ],
            max_attempts: 4,
            preserve_separation: true,
            class_ladder: vec![
                "coding-local".to_string(),
                "coding-local-large".to_string(),
                "coding-cloud".to_string(),
            ],
        }
    }
}

/// One attempt already made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attempt {
    pub assignment: RoleAssignment,
    pub budget: ReasoningBudget,
    pub provider: String,
    pub step: Option<EscalationStep>,
}

/// Remaining capacity per provider, checked before assignment.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct QuotaState {
    remaining: BTreeMap<String, u32>,
}

impl QuotaState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&mut self, provider: impl Into<String>, remaining: u32) {
        self.remaining.insert(provider.into(), remaining);
    }

    /// Unknown providers are treated as available; only a recorded zero blocks.
    pub fn has_capacity(&self, provider: &str) -> bool {
        self.remaining.get(provider).copied().unwrap_or(u32::MAX) > 0
    }

    pub fn consume(&mut self, provider: &str) {
        if let Some(remaining) = self.remaining.get_mut(provider) {
            *remaining = remaining.saturating_sub(1);
        }
    }
}

/// What the escalator decided to do next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EscalationOutcome {
    /// Try this configuration next.
    Retry {
        step: EscalationStep,
        assignment: RoleAssignment,
        budget: ReasoningBudget,
        provider: String,
        rationale: Vec<String>,
    },
    /// Nothing eligible remains; a human must decide.
    Escalate { rationale: Vec<String> },
}

/// Everything the escalator needs about the current execution.
#[derive(Debug, Clone, PartialEq)]
pub struct EscalationContext<'a> {
    pub policy: &'a EscalationPolicy,
    pub registry: &'a ModelProfileRegistry,
    pub requirements: &'a ModelRequirements,
    pub separation_policy: &'a SeparationPolicy,
    /// Every role assignment currently in force, including the failing one.
    pub current_assignments: &'a [RoleAssignment],
    pub quota: &'a QuotaState,
    pub attempts: &'a [Attempt],
}

/// Pick the next attempt after a failure, or escalate to a human.
pub fn next_attempt(context: &EscalationContext<'_>, failed: &Attempt) -> EscalationOutcome {
    let mut rationale = Vec::new();
    let attempts_made = context.attempts.len() as u32;
    if attempts_made >= context.policy.max_attempts {
        rationale.push(format!(
            "escalating to human: {attempts_made} attempts reached max_attempts {}",
            context.policy.max_attempts
        ));
        return EscalationOutcome::Escalate { rationale };
    }

    let start = failed
        .step
        .and_then(|step| context.policy.chain.iter().position(|entry| *entry == step))
        .map(|index| index + 1)
        .unwrap_or(0);

    for step in context.policy.chain.iter().skip(start).copied() {
        match step {
            EscalationStep::SameModelLargerBudget => {
                let Some(next_budget) = failed.budget.escalate() else {
                    rationale.push("same model, larger budget: already at ceiling".to_string());
                    continue;
                };
                if !context.quota.has_capacity(&failed.provider) {
                    rationale.push(format!(
                        "same model, larger budget: provider {} has no capacity",
                        failed.provider
                    ));
                    continue;
                }
                rationale.push(format!(
                    "same model {} at budget {}",
                    failed.assignment.model_key,
                    next_budget.as_str()
                ));
                return EscalationOutcome::Retry {
                    step,
                    assignment: failed.assignment.clone(),
                    budget: next_budget,
                    provider: failed.provider.clone(),
                    rationale,
                };
            }
            EscalationStep::AlternateModelInClass => {
                if let Some(outcome) = try_class(
                    context,
                    failed,
                    step,
                    &failed.assignment.model_class,
                    &mut rationale,
                ) {
                    return outcome;
                }
            }
            EscalationStep::HigherModelClass => {
                let higher = higher_classes(&context.policy.class_ladder, &failed.assignment.model_class);
                if higher.is_empty() {
                    rationale.push("higher model class: no class above current".to_string());
                }
                for class in higher {
                    if let Some(outcome) = try_class(context, failed, step, &class, &mut rationale) {
                        return outcome;
                    }
                }
            }
            EscalationStep::CloudProviderFallback => {
                for class in &context.policy.class_ladder {
                    if !class.contains("cloud") {
                        continue;
                    }
                    if let Some(outcome) = try_class(context, failed, step, class, &mut rationale) {
                        return outcome;
                    }
                }
                rationale.push("cloud fallback: no eligible cloud profile".to_string());
            }
            EscalationStep::HumanEscalation => {
                rationale.push("escalating to human: chain exhausted".to_string());
                return EscalationOutcome::Escalate { rationale };
            }
        }
    }

    rationale.push("escalating to human: no eligible configuration remained".to_string());
    EscalationOutcome::Escalate { rationale }
}

fn try_class(
    context: &EscalationContext<'_>,
    failed: &Attempt,
    step: EscalationStep,
    class: &str,
    rationale: &mut Vec<String>,
) -> Option<EscalationOutcome> {
    let mut requirements = context.requirements.clone();
    requirements.model_class = class.to_string();
    let resolution = context.registry.resolve(&requirements);

    for candidate in &resolution.matches {
        let key = candidate.profile.key();
        if key == failed.assignment.model_key {
            continue;
        }
        if !context.quota.has_capacity(&candidate.profile.provider) {
            rationale.push(format!(
                "{}: provider {} has no capacity",
                key, candidate.profile.provider
            ));
            continue;
        }

        let assignment = RoleAssignment::new(
            failed.assignment.role,
            key.clone(),
            candidate.profile.model_class.clone(),
            format!("{}-{}", failed.assignment.session_id, step.as_str()),
        );

        let proposed: Vec<RoleAssignment> = context
            .current_assignments
            .iter()
            .map(|existing| {
                if existing.role == assignment.role {
                    assignment.clone()
                } else {
                    existing.clone()
                }
            })
            .collect();

        if context.policy.preserve_separation {
            let verdict = preserves_separation_after_fallback(
                context.current_assignments,
                &proposed,
                context.separation_policy,
            );
            if !verdict.satisfied {
                rationale.push(format!(
                    "{key}: rejected, would break separation of duties ({})",
                    verdict.violations.join("; ")
                ));
                continue;
            }
        }

        rationale.push(format!("{key}: eligible in class {class}"));
        return Some(EscalationOutcome::Retry {
            step,
            assignment,
            budget: failed.budget,
            provider: candidate.profile.provider.clone(),
            rationale: rationale.clone(),
        });
    }

    if resolution.matches.is_empty() {
        rationale.push(format!("class {class}: no profile satisfied requirements"));
    }
    None
}

fn higher_classes(ladder: &[String], current: &str) -> Vec<String> {
    match ladder.iter().position(|class| class == current) {
        Some(index) => ladder[index + 1..].to_vec(),
        None => ladder.to_vec(),
    }
}
