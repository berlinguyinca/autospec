//! Execution policy assembly, decision records and explanations
//! (AAR spec sections 18 and 19).
//!
//! `decide` is the single entry point: work in, an execution policy plus the
//! audit trail out. Every decision records the policy version, the candidates
//! considered, the selection and the rationale, so a routing choice can be
//! explained months later without re-running it.

use serde::Serialize;

use super::classify::{
    classify, Capability, ClassificationInput, Complexity, Risk, TaskClassification,
    DEFAULT_TIE_BREAKER_THRESHOLD,
};
use super::context::{context_policy_for, ContextPolicy};
use super::escalation::EscalationPolicy;
use super::guards::{EditPolicy, StopPolicy};
use super::inferweave::{CapabilityRequest, LatencyPriority, SessionSeat};
use super::outcome::QualityThreshold;
use super::profile::{ModelProfileRegistry, ModelRequirements, ProfileResolution};
use super::reasoning::{
    select_reasoning, ReasoningContext, ReasoningHistory, ReasoningLimits, ReasoningSelection,
    SamplingProfile, SamplingRegistry,
};
use super::topology::{select_topology, AgentTopology, SeparationPolicy};

/// Bumped whenever a persisted policy field changes meaning.
pub const POLICY_SCHEMA_VERSION: u32 = 1;

/// The optimized execution policy for one unit of work (spec section 19).
#[derive(Debug, Clone, PartialEq)]
pub struct ExecutionPolicy {
    pub task_class: super::classify::TaskClass,
    pub complexity: Complexity,
    pub risk: Risk,
    pub topology: AgentTopology,
    pub model_requirements: ModelRequirements,
    pub reasoning: ReasoningSelection,
    pub sampling: Option<SamplingProfile>,
    pub context: ContextPolicy,
    pub editing: EditPolicy,
    pub stop: StopPolicy,
    pub escalation: EscalationPolicy,
}

/// Operator-versioned configuration the decision runs against.
#[derive(Debug, Clone, PartialEq)]
pub struct PolicyConfig {
    /// Version string recorded on every decision made under this config.
    pub policy_version: String,
    pub reasoning_limits: ReasoningLimits,
    pub sampling: SamplingRegistry,
    pub registry: ModelProfileRegistry,
    pub editing: EditPolicy,
    pub stop: StopPolicy,
    pub escalation: EscalationPolicy,
    pub separation: SeparationPolicy,
    pub quality_threshold: QualityThreshold,
    pub tie_breaker_threshold: f64,
    pub default_model_class: String,
    pub prefer_local: bool,
    pub latency_priority: LatencyPriority,
    /// Minimum capability score a profile must reach to be eligible.
    pub minimum_capability_score: f64,
    /// Tokens a session is expected to grow by beyond its current context.
    pub projected_context_growth: u64,
}

impl Default for PolicyConfig {
    fn default() -> Self {
        Self {
            policy_version: format!("aar-v{POLICY_SCHEMA_VERSION}"),
            reasoning_limits: ReasoningLimits::default(),
            sampling: SamplingRegistry::default_registry(),
            registry: ModelProfileRegistry::default(),
            editing: EditPolicy::default(),
            stop: StopPolicy::default(),
            escalation: EscalationPolicy::default(),
            separation: SeparationPolicy::default(),
            quality_threshold: QualityThreshold::default(),
            tie_breaker_threshold: DEFAULT_TIE_BREAKER_THRESHOLD,
            default_model_class: "coding-local".to_string(),
            prefer_local: true,
            latency_priority: LatencyPriority::Balanced,
            minimum_capability_score: 0.5,
            projected_context_growth: 16_000,
        }
    }
}

impl PolicyConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.policy_version.trim().is_empty() {
            return Err("policy config requires a version".to_string());
        }
        self.reasoning_limits.validate()?;
        if !(0.0..=1.0).contains(&self.minimum_capability_score) {
            return Err(format!(
                "minimum_capability_score {} out of range 0.0..=1.0",
                self.minimum_capability_score
            ));
        }
        if !(0.0..=1.0).contains(&self.tie_breaker_threshold) {
            return Err(format!(
                "tie_breaker_threshold {} out of range 0.0..=1.0",
                self.tie_breaker_threshold
            ));
        }
        Ok(())
    }
}

/// Serializable projection of a decision, for persistence and the dashboard.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DecisionRecord {
    pub schema_version: u32,
    pub policy_version: String,
    pub registry_version: String,
    pub task_class: String,
    pub complexity: String,
    pub risk: String,
    pub language: String,
    pub estimated_files: usize,
    pub capabilities: Vec<String>,
    pub requires_vision: bool,
    pub requires_web: bool,
    pub requires_long_context: bool,
    pub classification_confidence: f64,
    pub needs_tie_breaker: bool,
    pub classification_evidence: Vec<String>,
    pub roles: Vec<String>,
    pub isolated_contexts: bool,
    pub handoff: String,
    pub topology_reasons: Vec<String>,
    pub model_class: String,
    pub model_allowlist: Vec<String>,
    pub minimum_context_free: u64,
    pub selected_model: Option<String>,
    pub candidate_models: Vec<String>,
    pub rejected_models: Vec<String>,
    pub reasoning_budget: String,
    pub reasoning_tokens: u32,
    pub reasoning_reasons: Vec<String>,
    pub sampling_profile: Option<String>,
    pub max_retrieved_files: usize,
    pub include_full_history: bool,
    pub retrieval_ladder: Vec<String>,
    pub max_edit_lines: usize,
    pub max_new_file_lines: usize,
    pub stop_max_steps: u32,
    pub escalation_chain: Vec<String>,
    pub rationale: Vec<String>,
}

/// A decision plus everything needed to audit or explain it.
#[derive(Debug, Clone, PartialEq)]
pub struct PolicyDecision {
    pub policy_version: String,
    pub schema_version: u32,
    pub classification: TaskClassification,
    pub policy: ExecutionPolicy,
    pub resolution: ProfileResolution,
    pub capability_request: CapabilityRequest,
    pub rationale: Vec<String>,
}

impl PolicyDecision {
    pub fn selected_model(&self) -> Option<String> {
        self.resolution.best().map(|profile| profile.key())
    }

    pub fn record(&self) -> DecisionRecord {
        DecisionRecord {
            schema_version: self.schema_version,
            policy_version: self.policy_version.clone(),
            registry_version: self.resolution.registry_version.clone(),
            task_class: self.classification.task_class.as_str().to_string(),
            complexity: self.classification.complexity.as_str().to_string(),
            risk: self.classification.risk.as_str().to_string(),
            language: self.classification.language.clone(),
            estimated_files: self.classification.estimated_files,
            capabilities: self
                .classification
                .capabilities
                .iter()
                .map(|capability| capability.as_str().to_string())
                .collect(),
            requires_vision: self.classification.requires_vision,
            requires_web: self.classification.requires_web,
            requires_long_context: self.classification.requires_long_context,
            classification_confidence: self.classification.confidence,
            needs_tie_breaker: self
                .classification
                .needs_tie_breaker(DEFAULT_TIE_BREAKER_THRESHOLD),
            classification_evidence: self.classification.evidence.clone(),
            roles: self
                .policy
                .topology
                .roles
                .iter()
                .map(|role| role.as_str().to_string())
                .collect(),
            isolated_contexts: self.policy.topology.isolated_contexts,
            handoff: self.policy.topology.handoff.as_str().to_string(),
            topology_reasons: self.policy.topology.reasons.clone(),
            model_class: self.policy.model_requirements.model_class.clone(),
            model_allowlist: self.policy.model_requirements.model_allowlist.clone(),
            minimum_context_free: self.policy.model_requirements.minimum_context_free,
            selected_model: self.selected_model(),
            candidate_models: self
                .resolution
                .matches
                .iter()
                .map(|entry| format!("{} (fit {:.2})", entry.profile.key(), entry.fit))
                .collect(),
            rejected_models: self
                .resolution
                .rejections
                .iter()
                .map(|entry| format!("{}: {}", entry.key, entry.reason))
                .collect(),
            reasoning_budget: self.policy.reasoning.budget.as_str().to_string(),
            reasoning_tokens: self.policy.reasoning.tokens,
            reasoning_reasons: self.policy.reasoning.reasons.clone(),
            sampling_profile: self
                .policy
                .sampling
                .as_ref()
                .map(|profile| profile.identity()),
            max_retrieved_files: self.policy.context.max_retrieved_files,
            include_full_history: self.policy.context.include_full_history,
            retrieval_ladder: self
                .policy
                .context
                .ladder
                .iter()
                .map(|step| step.strategy.as_str().to_string())
                .collect(),
            max_edit_lines: self.policy.editing.max_edit_lines,
            max_new_file_lines: self.policy.editing.max_new_file_lines,
            stop_max_steps: self.policy.stop.max_steps,
            escalation_chain: self
                .policy
                .escalation
                .chain
                .iter()
                .map(|step| step.as_str().to_string())
                .collect(),
            rationale: self.rationale.clone(),
        }
    }

    pub fn to_json(&self) -> Result<String, String> {
        serde_json::to_string_pretty(&self.record()).map_err(|error| error.to_string())
    }

    /// Prose explanation in the shape of spec section 18.
    pub fn explain(&self) -> String {
        let model = self
            .selected_model()
            .unwrap_or_else(|| "no eligible model".to_string());
        let mut explanation = format!(
            "Selected {model} / pi / {} reasoning ({} tokens) because the task is a {} {} {}",
            self.policy.reasoning.budget.as_str(),
            self.policy.reasoning.tokens,
            self.classification.complexity.as_str(),
            self.classification.language,
            self.classification.task_class.as_str()
        );
        explanation.push_str(&format!(
            " at {} risk across ~{} file(s).",
            self.classification.risk.as_str(),
            self.classification.estimated_files
        ));

        if let Some(best) = self.resolution.best() {
            explanation.push_str(&format!(
                " The profile scores {:.2} on the required capabilities and its window of {} tokens covers the {} tokens this execution needs free.",
                best.capability_fit(&self.classification.capabilities),
                best.context_window,
                self.policy.model_requirements.minimum_context_free
            ));
            if let Some(rate) = best.observations.success_rate() {
                explanation.push_str(&format!(
                    " Measured production success is {:.0}% over {} tasks.",
                    rate * 100.0,
                    best.observations.tasks
                ));
            }
        } else {
            explanation.push_str(&format!(
                " No profile in registry {} satisfied the requirements; escalation applies.",
                self.resolution.registry_version
            ));
        }

        if !self.capability_request.prefix_cache_key.is_empty() {
            explanation.push_str(&format!(
                " Its stable prefix key is {}.",
                &self.capability_request.prefix_cache_key
            ));
        }

        let roles = self
            .policy
            .topology
            .roles
            .iter()
            .map(|role| role.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        explanation.push_str(&format!(" Topology: {roles}."));
        if self
            .policy
            .topology
            .roles
            .iter()
            .any(|role| role.is_producer())
            && self
                .policy
                .topology
                .roles
                .iter()
                .any(|role| role.is_independent_reviewer())
        {
            explanation.push_str(" Separation-of-duty requirements remain satisfied.");
        }
        explanation.push_str(&format!(" Policy version {}.", self.policy_version));
        explanation
    }
}

/// Classify one work item and produce its execution policy.
pub fn decide(
    input: &ClassificationInput,
    config: &PolicyConfig,
) -> Result<PolicyDecision, String> {
    config.validate()?;
    let classification = classify(input);
    decide_for_classification(classification, config)
}

/// Produce an execution policy for an already-classified work item.
///
/// Split out so a caller that ran an LLM tie-breaker can feed the corrected
/// classification back in without re-running the rubric.
pub fn decide_for_classification(
    classification: TaskClassification,
    config: &PolicyConfig,
) -> Result<PolicyDecision, String> {
    config.validate()?;
    let mut rationale = Vec::new();

    let topology = select_topology(&classification);
    rationale.extend(topology.reasons.iter().cloned());

    let context = context_policy_for(&classification);
    rationale.push(format!(
        "context budget: {} files / {} lines over {} expansion rounds, full history disabled",
        context.max_retrieved_files, context.max_retrieved_lines, context.max_expansion_rounds
    ));

    let minimum_context_free = projected_context_need(&classification, config);
    let model_requirements = ModelRequirements {
        model_class: config.default_model_class.clone(),
        model_allowlist: Vec::new(),
        required_capabilities: classification.capabilities.clone(),
        minimum_capability_score: config.minimum_capability_score,
        requires_vision: classification.requires_vision,
        requires_web: classification.requires_web,
        minimum_context_free,
        prefer_local: config.prefer_local,
    };
    let resolution = config.registry.resolve(&model_requirements);
    match resolution.best() {
        Some(profile) => rationale.push(format!(
            "model {} selected from {} eligible candidate(s)",
            profile.key(),
            resolution.matches.len()
        )),
        None => rationale.push(format!(
            "no eligible model in registry {}; {} candidate(s) rejected",
            resolution.registry_version,
            resolution.rejections.len()
        )),
    }

    let reasoning_context = ReasoningContext {
        low_confidence: classification.needs_tie_breaker(config.tie_breaker_threshold),
        ..ReasoningContext::default()
    };
    let reasoning = select_reasoning(
        &classification,
        &reasoning_context,
        &ReasoningHistory::default(),
        &config.reasoning_limits,
    );
    rationale.extend(reasoning.reasons.iter().cloned());

    let sampling = config.sampling.for_budget(reasoning.budget).cloned();

    let capability_request = CapabilityRequest {
        model_class: model_requirements.model_class.clone(),
        // Two profiles of one model (a quantization and a full-precision
        // build) must not name it twice: InferWeave reads this as a set.
        model_allowlist: resolution
            .matches
            .iter()
            .map(|entry| entry.profile.model_id.clone())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect(),
        minimum_context_free,
        prefer_local: config.prefer_local,
        session_affinity: true,
        prefix_cache_key: String::new(),
        latency_priority: config.latency_priority,
        session_id: String::new(),
        seat: SessionSeat {
            current_context_tokens: minimum_context_free
                .saturating_sub(config.projected_context_growth),
            projected_growth_tokens: config.projected_context_growth,
            kv_tokens: 0,
        },
    };

    let policy = ExecutionPolicy {
        task_class: classification.task_class,
        complexity: classification.complexity,
        risk: classification.risk,
        topology,
        model_requirements,
        reasoning,
        sampling,
        context,
        editing: config.editing.clone(),
        stop: config.stop.clone(),
        escalation: config.escalation.clone(),
    };

    Ok(PolicyDecision {
        policy_version: config.policy_version.clone(),
        schema_version: POLICY_SCHEMA_VERSION,
        classification,
        policy,
        resolution,
        capability_request,
        rationale,
    })
}

/// Free context an execution of this shape is expected to need.
///
/// Derived from the retrieval budget rather than a flat constant: asking for
/// more than the work needs takes nodes out of the eligible set for no gain.
fn projected_context_need(classification: &TaskClassification, config: &PolicyConfig) -> u64 {
    let per_file_tokens: u64 = 900;
    let retrieval = context_policy_for(classification);
    let retrieved = (retrieval.max_retrieved_files as u64).saturating_mul(per_file_tokens);
    let overhead: u64 = 6_000;
    let vision_extra = if classification.requires_vision {
        4_000
    } else {
        0
    };
    retrieved
        .saturating_add(overhead)
        .saturating_add(vision_extra)
        .saturating_add(config.projected_context_growth)
}

/// Capabilities a role needs beyond the task's own requirements.
pub fn role_capabilities(role: super::topology::AgentRole) -> Vec<Capability> {
    use super::topology::AgentRole;
    match role {
        AgentRole::Coordinator => vec![Capability::Planning, Capability::TextualAnalysis],
        AgentRole::Explorer => vec![Capability::RepositoryReasoning, Capability::ToolUse],
        AgentRole::Planner => vec![Capability::Planning],
        AgentRole::Implementer => vec![Capability::Coding, Capability::ToolUse],
        AgentRole::Tester => vec![Capability::Coding, Capability::ToolUse],
        AgentRole::Reviewer => vec![Capability::Review, Capability::RepositoryReasoning],
        AgentRole::DocumentationWriter => {
            vec![Capability::Documentation, Capability::TextualAnalysis]
        }
        AgentRole::UiEvaluator => vec![Capability::Vision],
        AgentRole::SecurityReviewer => vec![Capability::Review, Capability::RepositoryReasoning],
        AgentRole::PerformanceReviewer => vec![Capability::Review, Capability::Debugging],
    }
}
