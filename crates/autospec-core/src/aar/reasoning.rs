//! Adaptive reasoning budgets and sampling profiles (AAR spec section 5).
//!
//! The load-bearing rule here is the negative one: more reasoning is not
//! assumed to be better. A larger budget is only selected when the measured
//! success rate at that budget is *materially* better on enough samples to
//! believe it. Absent evidence, the cheaper budget wins.

use std::collections::BTreeMap;

use super::classify::{Complexity, Risk, TaskClass, TaskClassification};

/// Abstract reasoning budget; adapters translate it to backend controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ReasoningBudget {
    Tiny,
    Normal,
    Complex,
    Exceptional,
}

impl ReasoningBudget {
    pub fn as_str(&self) -> &'static str {
        match self {
            ReasoningBudget::Tiny => "tiny",
            ReasoningBudget::Normal => "normal",
            ReasoningBudget::Complex => "complex",
            ReasoningBudget::Exceptional => "exceptional",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value.trim().to_ascii_lowercase().as_str() {
            "tiny" => ReasoningBudget::Tiny,
            "normal" => ReasoningBudget::Normal,
            "complex" => ReasoningBudget::Complex,
            "exceptional" => ReasoningBudget::Exceptional,
            _ => return None,
        })
    }

    pub fn all() -> [ReasoningBudget; 4] {
        [
            ReasoningBudget::Tiny,
            ReasoningBudget::Normal,
            ReasoningBudget::Complex,
            ReasoningBudget::Exceptional,
        ]
    }

    /// Next budget up, or `None` at the ceiling.
    pub fn escalate(&self) -> Option<Self> {
        match self {
            ReasoningBudget::Tiny => Some(ReasoningBudget::Normal),
            ReasoningBudget::Normal => Some(ReasoningBudget::Complex),
            ReasoningBudget::Complex => Some(ReasoningBudget::Exceptional),
            ReasoningBudget::Exceptional => None,
        }
    }
}

/// Token counts for each abstract budget. Configurable; these are the spec
/// section 5 starting values, not permanent assumptions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReasoningLimits {
    pub tiny: u32,
    pub normal: u32,
    pub complex: u32,
    pub exceptional: u32,
}

impl Default for ReasoningLimits {
    fn default() -> Self {
        Self {
            tiny: 512,
            normal: 2_048,
            complex: 4_096,
            exceptional: 8_192,
        }
    }
}

impl ReasoningLimits {
    pub fn tokens(&self, budget: ReasoningBudget) -> u32 {
        match budget {
            ReasoningBudget::Tiny => self.tiny,
            ReasoningBudget::Normal => self.normal,
            ReasoningBudget::Complex => self.complex,
            ReasoningBudget::Exceptional => self.exceptional,
        }
    }

    /// Reject a non-monotonic ladder; it would make escalation meaningless.
    pub fn validate(&self) -> Result<(), String> {
        if self.tiny == 0 {
            return Err("reasoning limit tiny must be greater than zero".to_string());
        }
        if !(self.tiny < self.normal && self.normal < self.complex && self.complex < self.exceptional)
        {
            return Err(
                "reasoning limits must increase strictly: tiny < normal < complex < exceptional"
                    .to_string(),
            );
        }
        Ok(())
    }
}

/// Measured success at one budget for one (model, task class) pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BudgetObservation {
    pub attempts: u32,
    pub successes: u32,
}

impl BudgetObservation {
    pub fn success_rate(&self) -> Option<f64> {
        if self.attempts == 0 {
            return None;
        }
        Some(f64::from(self.successes) / f64::from(self.attempts))
    }
}

/// Minimum attempts at both budgets before a comparison is believed.
pub const MIN_SAMPLES_FOR_BUDGET_COMPARISON: u32 = 10;

/// Success-rate delta below which a larger budget is not worth its cost.
pub const MATERIAL_SUCCESS_DELTA: f64 = 0.05;

/// History of budget outcomes, keyed by budget.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReasoningHistory {
    observations: BTreeMap<ReasoningBudget, BudgetObservation>,
}

impl ReasoningHistory {
    pub fn record(&mut self, budget: ReasoningBudget, success: bool) {
        let entry = self.observations.entry(budget).or_default();
        entry.attempts += 1;
        if success {
            entry.successes += 1;
        }
    }

    pub fn observation(&self, budget: ReasoningBudget) -> BudgetObservation {
        self.observations.get(&budget).copied().unwrap_or_default()
    }

    /// True when `larger` is measurably better than `smaller`.
    ///
    /// Returns false when either side is under-sampled: absence of evidence is
    /// not evidence that more reasoning helps.
    pub fn materially_better(&self, larger: ReasoningBudget, smaller: ReasoningBudget) -> bool {
        let larger_observation = self.observation(larger);
        let smaller_observation = self.observation(smaller);
        if larger_observation.attempts < MIN_SAMPLES_FOR_BUDGET_COMPARISON
            || smaller_observation.attempts < MIN_SAMPLES_FOR_BUDGET_COMPARISON
        {
            return false;
        }
        match (
            larger_observation.success_rate(),
            smaller_observation.success_rate(),
        ) {
            (Some(large), Some(small)) => large - small > MATERIAL_SUCCESS_DELTA,
            _ => false,
        }
    }

    /// Cheapest budget whose measured success is not materially worse than the
    /// best observed budget. Returns `None` when nothing is sampled enough.
    pub fn cheapest_adequate(&self) -> Option<ReasoningBudget> {
        let sampled: Vec<(ReasoningBudget, f64)> = ReasoningBudget::all()
            .into_iter()
            .filter_map(|budget| {
                let observation = self.observation(budget);
                if observation.attempts < MIN_SAMPLES_FOR_BUDGET_COMPARISON {
                    return None;
                }
                observation.success_rate().map(|rate| (budget, rate))
            })
            .collect();
        let best = sampled
            .iter()
            .map(|(_, rate)| *rate)
            .fold(f64::NEG_INFINITY, f64::max);
        sampled
            .into_iter()
            .find(|(_, rate)| best - rate <= MATERIAL_SUCCESS_DELTA)
            .map(|(budget, _)| budget)
    }
}

/// Non-task inputs that shift the budget.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReasoningContext {
    /// Retries already spent on this unit.
    pub retries: u32,
    /// Reviewer asked for rework at least once.
    pub reviewer_rejected: bool,
    /// Classifier confidence was below the tie-breaker threshold.
    pub low_confidence: bool,
    /// Operator asked for the cheapest viable configuration.
    pub cost_sensitive: bool,
    /// Operator asked for the lowest latency configuration.
    pub latency_sensitive: bool,
}

/// The chosen budget, its concrete token count and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReasoningSelection {
    pub budget: ReasoningBudget,
    pub tokens: u32,
    pub reasons: Vec<String>,
}

/// Choose a reasoning budget for one unit of work.
pub fn select_reasoning(
    classification: &TaskClassification,
    context: &ReasoningContext,
    history: &ReasoningHistory,
    limits: &ReasoningLimits,
) -> ReasoningSelection {
    let mut reasons = Vec::new();

    let mut budget = baseline_budget(classification);
    reasons.push(format!(
        "baseline {} from complexity={} class={}",
        budget.as_str(),
        classification.complexity.as_str(),
        classification.task_class.as_str()
    ));

    if classification.risk >= Risk::Critical && budget < ReasoningBudget::Complex {
        budget = ReasoningBudget::Complex;
        reasons.push("raised to complex: critical risk".to_string());
    }

    if context.low_confidence && budget < ReasoningBudget::Complex {
        budget = budget.escalate().unwrap_or(budget);
        reasons.push("raised one step: classification confidence below threshold".to_string());
    }

    if context.reviewer_rejected {
        if let Some(next) = budget.escalate() {
            budget = next;
            reasons.push("raised one step: reviewer requested rework".to_string());
        }
    }

    for retry in 0..context.retries {
        if let Some(next) = budget.escalate() {
            budget = next;
            reasons.push(format!("raised one step: retry {}", retry + 1));
        }
    }

    // Evidence overrides the ladder in both directions.
    if let Some(cheapest) = history.cheapest_adequate() {
        if cheapest < budget && !history.materially_better(budget, cheapest) {
            reasons.push(format!(
                "lowered to {}: measured success at {} is not materially better",
                cheapest.as_str(),
                budget.as_str()
            ));
            budget = cheapest;
        }
    }

    if (context.cost_sensitive || context.latency_sensitive)
        && budget > ReasoningBudget::Tiny
        && !history.materially_better(budget, ReasoningBudget::Normal)
        && classification.complexity <= Complexity::Medium
        && classification.risk < Risk::High
    {
        let lowered = ReasoningBudget::Normal.min(budget);
        if lowered < budget {
            reasons.push(format!(
                "lowered to {}: cost/latency preference with no measured benefit above it",
                lowered.as_str()
            ));
            budget = lowered;
        }
    }

    ReasoningSelection {
        budget,
        tokens: limits.tokens(budget),
        reasons,
    }
}

fn baseline_budget(classification: &TaskClassification) -> ReasoningBudget {
    let from_complexity = match classification.complexity {
        Complexity::Trivial => ReasoningBudget::Tiny,
        Complexity::Low => ReasoningBudget::Normal,
        Complexity::Medium => ReasoningBudget::Normal,
        Complexity::High => ReasoningBudget::Complex,
        Complexity::Exceptional => ReasoningBudget::Exceptional,
    };
    let from_class = match classification.task_class {
        TaskClass::Docs | TaskClass::Test => ReasoningBudget::Tiny,
        TaskClass::Research | TaskClass::Migration => ReasoningBudget::Complex,
        _ => ReasoningBudget::Tiny,
    };
    from_complexity.max(from_class)
}

/// A versioned, benchmarkable sampling configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct SamplingProfile {
    pub name: String,
    pub version: u32,
    pub temperature: f64,
    pub top_p: f64,
    pub top_k: u32,
    pub min_p: f64,
    pub repeat_penalty: f64,
    pub max_output_tokens: u32,
}

impl SamplingProfile {
    /// Starting point for Qwen-class thinking modes. Benchmarkable
    /// configuration, not a permanent assumption.
    pub fn qwen_thinking() -> Self {
        Self {
            name: "qwen-thinking".to_string(),
            version: 1,
            temperature: 0.6,
            top_p: 0.95,
            top_k: 20,
            min_p: 0.0,
            repeat_penalty: 1.0,
            max_output_tokens: 8_192,
        }
    }

    /// Starting point for Qwen-class instruct (non-thinking) modes.
    pub fn qwen_instruct() -> Self {
        Self {
            name: "qwen-instruct".to_string(),
            version: 1,
            temperature: 0.7,
            top_p: 0.8,
            top_k: 20,
            min_p: 0.0,
            repeat_penalty: 1.05,
            max_output_tokens: 4_096,
        }
    }

    pub fn identity(&self) -> String {
        format!("{}@v{}", self.name, self.version)
    }

    pub fn validate(&self) -> Result<(), String> {
        if !(0.0..=2.0).contains(&self.temperature) {
            return Err(format!("temperature {} out of range 0.0..=2.0", self.temperature));
        }
        if !(0.0..=1.0).contains(&self.top_p) {
            return Err(format!("top_p {} out of range 0.0..=1.0", self.top_p));
        }
        if !(0.0..=1.0).contains(&self.min_p) {
            return Err(format!("min_p {} out of range 0.0..=1.0", self.min_p));
        }
        if self.max_output_tokens == 0 {
            return Err("max_output_tokens must be greater than zero".to_string());
        }
        Ok(())
    }
}

/// A versioned set of sampling profiles keyed by name.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SamplingRegistry {
    pub version: String,
    profiles: BTreeMap<String, SamplingProfile>,
}

impl SamplingRegistry {
    pub fn new(version: impl Into<String>, profiles: Vec<SamplingProfile>) -> Self {
        Self {
            version: version.into(),
            profiles: profiles
                .into_iter()
                .map(|profile| (profile.name.clone(), profile))
                .collect(),
        }
    }

    pub fn default_registry() -> Self {
        Self::new(
            "sampling-v1",
            vec![SamplingProfile::qwen_thinking(), SamplingProfile::qwen_instruct()],
        )
    }

    pub fn get(&self, name: &str) -> Option<&SamplingProfile> {
        self.profiles.get(name)
    }

    pub fn names(&self) -> Vec<&str> {
        self.profiles.keys().map(String::as_str).collect()
    }

    /// Pick a sampling profile for a budget: thinking modes for budgets above
    /// tiny, instruct otherwise, falling back to any registered profile.
    pub fn for_budget(&self, budget: ReasoningBudget) -> Option<&SamplingProfile> {
        let preferred = if budget > ReasoningBudget::Tiny {
            "qwen-thinking"
        } else {
            "qwen-instruct"
        };
        self.get(preferred)
            .or_else(|| self.profiles.values().next())
    }
}
