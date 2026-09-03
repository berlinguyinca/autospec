//! Outcome scoring and profile recommendation (AAR spec section 15).
//!
//! The objective is not the best profile; it is the *cheapest* profile that
//! still clears the configured quality bar. V1 is deterministic rules over
//! measured statistics, and a hard policy always overrides what the statistics
//! recommend.

use super::classify::TaskClass;

/// The section 19 outcome shape, recorded per execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ExecutionOutcome {
    pub success: bool,
    pub tests_passed: bool,
    pub review_passed: bool,
    pub latency_ms: u64,
    pub prompt_tokens: u64,
    pub cached_prompt_tokens: u64,
    pub reasoning_tokens: u64,
    pub output_tokens: u64,
    pub retries: u32,
}

/// Extra signals that separate "passed" from "passed cleanly".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OutcomeSignals {
    pub acceptance_criteria_total: u32,
    pub acceptance_criteria_met: u32,
    pub regressions_introduced: u32,
    pub human_corrections: u32,
    /// Files changed that the task did not call for.
    pub unnecessary_changes: u32,
    /// Total edits, to spot churn.
    pub edits: u32,
    pub policy_violations: u32,
}

/// A composite score with its parts, so a low score is explainable.
#[derive(Debug, Clone, PartialEq)]
pub struct OutcomeScore {
    pub quality: f64,
    pub efficiency: f64,
    pub composite: f64,
    pub reasons: Vec<String>,
}

/// Score one execution on quality and efficiency.
pub fn score_outcome(outcome: &ExecutionOutcome, signals: &OutcomeSignals) -> OutcomeScore {
    let mut reasons = Vec::new();
    let mut quality: f64 = 0.0;

    if outcome.success {
        quality += 0.4;
        reasons.push("task succeeded (+0.40)".to_string());
    }
    if outcome.tests_passed {
        quality += 0.2;
        reasons.push("tests passed (+0.20)".to_string());
    }
    if outcome.review_passed {
        quality += 0.2;
        reasons.push("independent review passed (+0.20)".to_string());
    }
    if signals.acceptance_criteria_total > 0 {
        let coverage = f64::from(signals.acceptance_criteria_met)
            / f64::from(signals.acceptance_criteria_total);
        quality += 0.2 * coverage.clamp(0.0, 1.0);
        reasons.push(format!("acceptance coverage {coverage:.2} (+{:.2})", 0.2 * coverage));
    }
    for (count, penalty, label) in [
        (signals.regressions_introduced, 0.25, "regression"),
        (signals.human_corrections, 0.15, "human correction"),
        (signals.policy_violations, 0.20, "policy violation"),
        (signals.unnecessary_changes, 0.05, "unnecessary change"),
    ] {
        if count > 0 {
            let deduction = penalty * f64::from(count);
            quality -= deduction;
            reasons.push(format!("{count} {label}(s) (-{deduction:.2})"));
        }
    }
    let quality = quality.clamp(0.0, 1.0);

    let mut efficiency: f64 = 1.0;
    if outcome.retries > 0 {
        let deduction = 0.15 * f64::from(outcome.retries);
        efficiency -= deduction;
        reasons.push(format!("{} retries (-{deduction:.2})", outcome.retries));
    }
    if outcome.prompt_tokens > 0 {
        let cache_rate = outcome.cached_prompt_tokens as f64 / outcome.prompt_tokens as f64;
        efficiency += 0.1 * cache_rate;
        reasons.push(format!("cache hit rate {cache_rate:.2} (+{:.2})", 0.1 * cache_rate));
    }
    if signals.edits > 0 && signals.unnecessary_changes > 0 {
        let churn = f64::from(signals.unnecessary_changes) / f64::from(signals.edits);
        efficiency -= 0.2 * churn.clamp(0.0, 1.0);
        reasons.push(format!("edit churn {churn:.2}"));
    }
    let efficiency = efficiency.clamp(0.0, 1.0);

    OutcomeScore {
        quality,
        efficiency,
        composite: quality * 0.75 + efficiency * 0.25,
        reasons,
    }
}

/// Accumulated statistics for one (profile, task class) pair.
#[derive(Debug, Clone, PartialEq)]
pub struct ProfileStats {
    pub profile_key: String,
    pub task_class: TaskClass,
    pub reasoning_budget: String,
    pub samples: u32,
    pub successes: u32,
    pub mean_latency_ms: u64,
    pub mean_cost_micros: u64,
}

impl ProfileStats {
    pub fn success_rate(&self) -> f64 {
        if self.samples == 0 {
            return 0.0;
        }
        f64::from(self.successes) / f64::from(self.samples)
    }

    /// Wilson lower bound at roughly 95% confidence.
    ///
    /// A raw rate lets 3-for-3 outrank 95-for-100; the lower bound does not,
    /// which is the whole point of requiring evidence before switching.
    pub fn success_lower_bound(&self) -> f64 {
        if self.samples == 0 {
            return 0.0;
        }
        let n = f64::from(self.samples);
        let p = self.success_rate();
        let z = 1.96_f64;
        let z2 = z * z;
        let denominator = 1.0 + z2 / n;
        let center = p + z2 / (2.0 * n);
        let margin = z * ((p * (1.0 - p) / n) + z2 / (4.0 * n * n)).sqrt();
        ((center - margin) / denominator).clamp(0.0, 1.0)
    }
}

/// The bar a profile must clear before cost may decide.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QualityThreshold {
    pub min_success_rate: f64,
    pub min_samples: u32,
}

impl Default for QualityThreshold {
    fn default() -> Self {
        Self {
            min_success_rate: 0.8,
            min_samples: 10,
        }
    }
}

/// What the optimizer suggests, and why.
#[derive(Debug, Clone, PartialEq)]
pub struct Recommendation {
    pub profile_key: Option<String>,
    pub reasoning_budget: Option<String>,
    pub rationale: Vec<String>,
    /// Set when a hard policy replaced the measured recommendation.
    pub overridden_by_policy: bool,
}

/// Recommend the cheapest profile clearing the quality threshold.
pub fn recommend(candidates: &[ProfileStats], threshold: &QualityThreshold) -> Recommendation {
    let mut rationale = Vec::new();
    let mut eligible: Vec<&ProfileStats> = Vec::new();

    for candidate in candidates {
        if candidate.samples < threshold.min_samples {
            rationale.push(format!(
                "{}: {} samples below minimum {}",
                candidate.profile_key, candidate.samples, threshold.min_samples
            ));
            continue;
        }
        let bound = candidate.success_lower_bound();
        if bound < threshold.min_success_rate {
            rationale.push(format!(
                "{}: success lower bound {bound:.2} below threshold {:.2}",
                candidate.profile_key, threshold.min_success_rate
            ));
            continue;
        }
        rationale.push(format!(
            "{}: eligible (lower bound {bound:.2}, cost {} micros, latency {} ms)",
            candidate.profile_key, candidate.mean_cost_micros, candidate.mean_latency_ms
        ));
        eligible.push(candidate);
    }

    eligible.sort_by(|left, right| {
        left.mean_cost_micros
            .cmp(&right.mean_cost_micros)
            .then_with(|| left.mean_latency_ms.cmp(&right.mean_latency_ms))
            .then_with(|| left.profile_key.cmp(&right.profile_key))
    });

    match eligible.first() {
        Some(best) => {
            rationale.push(format!(
                "selected {}: cheapest configuration meeting the quality threshold",
                best.profile_key
            ));
            Recommendation {
                profile_key: Some(best.profile_key.clone()),
                reasoning_budget: Some(best.reasoning_budget.clone()),
                rationale,
                overridden_by_policy: false,
            }
        }
        None => {
            rationale
                .push("no candidate met the quality threshold; keeping the configured default"
                    .to_string());
            Recommendation {
                profile_key: None,
                reasoning_budget: None,
                rationale,
                overridden_by_policy: false,
            }
        }
    }
}

/// A hard policy pin that learned recommendations may not override.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HardPolicy {
    /// Profiles that may never be selected, whatever the statistics say.
    pub denied_profiles: Vec<String>,
    /// Profile the operator pinned for this task class.
    pub pinned_profile: Option<String>,
    pub pinned_reasoning_budget: Option<String>,
}

/// Apply hard policy over a learned recommendation.
pub fn apply_policy_override(
    recommendation: Recommendation,
    policy: &HardPolicy,
) -> Recommendation {
    let mut recommendation = recommendation;

    if let Some(selected) = recommendation.profile_key.clone() {
        if policy.denied_profiles.contains(&selected) {
            recommendation
                .rationale
                .push(format!("policy denies {selected}; recommendation dropped"));
            recommendation.profile_key = None;
            recommendation.reasoning_budget = None;
            recommendation.overridden_by_policy = true;
        }
    }

    if let Some(pinned) = &policy.pinned_profile {
        if recommendation.profile_key.as_ref() != Some(pinned) {
            recommendation
                .rationale
                .push(format!("policy pins {pinned}; learned recommendation overridden"));
            recommendation.overridden_by_policy = true;
        }
        recommendation.profile_key = Some(pinned.clone());
    }

    if let Some(budget) = &policy.pinned_reasoning_budget {
        if recommendation.reasoning_budget.as_ref() != Some(budget) {
            recommendation
                .rationale
                .push(format!("policy pins reasoning budget {budget}"));
            recommendation.overridden_by_policy = true;
        }
        recommendation.reasoning_budget = Some(budget.clone());
    }

    recommendation
}
