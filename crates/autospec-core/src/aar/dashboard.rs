//! Live performance card and benchmark-derived routing advice (AAR M12,
//! issue #3327).
//!
//! The dashboard is a provider-neutral projection of execution state: one live
//! card per in-flight work item, a historical summary of completed issues, and
//! routing advice that stays on static heuristics until a profile has enough
//! benchmark samples to earn measured advice. Advice may trade profiles inside
//! the locked model family; it never moves the family, so a fast afternoon of
//! numbers from another vendor cannot swap the local coding model out from
//! under the fleet.
//!
//! The shell reader `scripts/pi-performance-dashboard.sh` renders the same
//! contract from a JSONL ledger. The two implementations stay in lockstep on
//! the percentile rule (nearest-rank, ceil, integer arithmetic) and the
//! Wilson lower bound (reused from `outcome::ProfileStats`).

use super::classify::TaskClass;
use super::outcome::ProfileStats;

/// Sample threshold below which a profile stays on static selection.
pub const DEFAULT_MIN_SAMPLES: u32 = 20;

/// Model family the advice is locked to for this deployment.
pub const DEFAULT_LOCKED_MODEL_FAMILY: &str = "qwen3.8";

/// Wilson lower bound a profile's success rate must clear before cost decides.
pub const MIN_SUCCESS_RATE: f64 = 0.8;

/// The execution fields the live card must always expose (issue #3327 AC1).
pub const LIVE_CARD_FIELDS: [&str; 12] = [
    "state",
    "node_id",
    "profile",
    "turns",
    "context_tokens",
    "ttft_ms",
    "decode_tokens_per_second",
    "cache_hit_rate",
    "tool_ms",
    "test_ms",
    "repair_count",
    "queue_ms",
];

/// One in-flight work item as the dashboard should show it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LiveWorkItem {
    pub issue_id: String,
    pub state: String,
    pub node_id: String,
    pub profile: String,
    pub turns: u32,
    pub context_tokens: u64,
    pub ttft_ms: u64,
    pub decode_tokens_per_second: f64,
    /// Fraction of the prompt served from cache, 0.0..=1.0.
    pub cache_hit_rate: f64,
    pub tool_ms: u64,
    pub test_ms: u64,
    pub repair_count: u32,
    pub queue_ms: u64,
}

impl LiveWorkItem {
    /// Reject a card whose values cannot be true on a live session.
    pub fn validate(&self) -> Result<(), String> {
        if !(0.0..=1.0).contains(&self.cache_hit_rate) {
            return Err(format!(
                "cache_hit_rate {} out of range 0.0..=1.0",
                self.cache_hit_rate
            ));
        }
        if self.decode_tokens_per_second < 0.0 {
            return Err("decode_tokens_per_second must not be negative".to_string());
        }
        Ok(())
    }

    /// The card as rendered text: the issue id plus every required execution
    /// field, one labelled line each.
    pub fn render(&self) -> String {
        let mut out = format!("  issue_id: {}\n", self.issue_id);
        out.push_str(&format!("  state: {}\n", self.state));
        out.push_str(&format!("  node_id: {}\n", self.node_id));
        out.push_str(&format!("  profile: {}\n", self.profile));
        out.push_str(&format!("  turns: {}\n", self.turns));
        out.push_str(&format!("  context_tokens: {}\n", self.context_tokens));
        out.push_str(&format!("  ttft_ms: {}\n", self.ttft_ms));
        out.push_str(&format!(
            "  decode_tokens_per_second: {:.1}\n",
            self.decode_tokens_per_second
        ));
        out.push_str(&format!("  cache_hit_rate: {:.2}\n", self.cache_hit_rate));
        out.push_str(&format!("  tool_ms: {}\n", self.tool_ms));
        out.push_str(&format!("  test_ms: {}\n", self.test_ms));
        out.push_str(&format!("  repair_count: {}\n", self.repair_count));
        out.push_str(&format!("  queue_ms: {}\n", self.queue_ms));
        out
    }
}

/// One completed issue's measured record.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct IssueSample {
    pub issue_id: String,
    pub duration_ms: u64,
    pub succeeded: bool,
    /// Set when the issue produced a passing change, whatever its final state.
    pub time_to_passing_change_ms: Option<u64>,
}

/// The historical half of the dashboard.
#[derive(Debug, Clone, PartialEq)]
pub struct HistorySummary {
    pub samples: usize,
    pub p50_ms: u64,
    pub p90_ms: u64,
    pub p95_ms: u64,
    pub successful_issues_per_hour: f64,
    pub median_time_to_passing_ms: Option<u64>,
}

/// Nearest-rank percentile (ceil) over an unsorted slice.
///
/// rank = ceil(p/100 * n), 1-indexed, computed in exact integer arithmetic
/// (`(p * n).div_ceil(100)`) so the rule is exact: the shell reader computes
/// the same value and the two must not drift.
pub fn percentile_nearest_rank(values: &[u64], pct: u32) -> Option<u64> {
    if values.is_empty() || pct == 0 || pct > 100 {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let n = sorted.len() as u64;
    let rank = (u64::from(pct) * n).div_ceil(100);
    sorted.get(rank as usize - 1).copied()
}

/// Summarize completed issues over a measurement window.
pub fn summarize_history(samples: &[IssueSample], window_hours: f64) -> HistorySummary {
    let durations: Vec<u64> = samples.iter().map(|sample| sample.duration_ms).collect();
    let passing: Vec<u64> = samples
        .iter()
        .filter_map(|sample| sample.time_to_passing_change_ms)
        .collect();
    let successes = samples.iter().filter(|sample| sample.succeeded).count();
    let per_hour = if window_hours > 0.0 {
        successes as f64 / window_hours
    } else {
        0.0
    };
    HistorySummary {
        samples: samples.len(),
        p50_ms: percentile_nearest_rank(&durations, 50).unwrap_or(0),
        p90_ms: percentile_nearest_rank(&durations, 90).unwrap_or(0),
        p95_ms: percentile_nearest_rank(&durations, 95).unwrap_or(0),
        successful_issues_per_hour: per_hour,
        median_time_to_passing_ms: percentile_nearest_rank(&passing, 50),
    }
}

/// Operator configuration for the advisory.
#[derive(Debug, Clone, PartialEq)]
pub struct AdviceConfig {
    pub min_samples: u32,
    /// Profile the static heuristics pick until benchmark evidence is sufficient.
    pub static_profile: String,
    /// Advice may trade profiles inside this family and never outside it.
    pub locked_model_family: String,
}

impl Default for AdviceConfig {
    fn default() -> Self {
        Self {
            min_samples: DEFAULT_MIN_SAMPLES,
            static_profile: "qwen3.8-coding-local".to_string(),
            locked_model_family: DEFAULT_LOCKED_MODEL_FAMILY.to_string(),
        }
    }
}

impl AdviceConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.min_samples == 0 {
            return Err("min_samples must be at least 1".to_string());
        }
        if self.static_profile.trim().is_empty() {
            return Err("static_profile must be set".to_string());
        }
        if self.locked_model_family.trim().is_empty() {
            return Err("locked_model_family must be set".to_string());
        }
        Ok(())
    }
}

/// Accumulated benchmark evidence for one (family, profile) pair.
#[derive(Debug, Clone, PartialEq)]
pub struct ProfileSample {
    pub model_family: String,
    pub profile_key: String,
    pub samples: u32,
    pub successes: u32,
    pub mean_cost_micros: u64,
    pub mean_wall_ms: u64,
}

impl ProfileSample {
    /// Wilson lower bound on the success rate.
    ///
    /// Reused from the outcome optimizer so a profile cannot pass one bar and
    /// fail the other; a raw rate lets 3-for-3 outrank 95-for-100.
    pub fn success_lower_bound(&self) -> f64 {
        ProfileStats {
            profile_key: self.profile_key.clone(),
            task_class: TaskClass::Bugfix,
            reasoning_budget: String::new(),
            samples: self.samples,
            successes: self.successes,
            mean_cost_micros: self.mean_cost_micros,
            mean_latency_ms: self.mean_wall_ms,
        }
        .success_lower_bound()
    }
}

/// Where the advice came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdviceSource {
    /// Static heuristics: not enough benchmark evidence, or none in-family.
    Static,
    /// A profile with sufficient, eligible benchmark evidence won.
    Benchmark,
}

impl AdviceSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            AdviceSource::Static => "static",
            AdviceSource::Benchmark => "benchmark",
        }
    }
}

/// The advisory, and why.
#[derive(Debug, Clone, PartialEq)]
pub struct RoutingAdvice {
    pub source: AdviceSource,
    /// Always `config.locked_model_family`; see `advise`.
    pub model_family: String,
    pub profile: String,
    pub rationale: Vec<String>,
}

/// Recommend a profile from benchmark evidence, staying inside the locked
/// family.
///
/// A candidate is eligible when it is in the locked family, has at least
/// `min_samples` samples, and clears the Wilson success bar. The cheapest
/// eligible profile (then shortest wall, then key) wins; with no eligible
/// candidate the static selection is kept. The returned model family is the
/// locked family in every case.
pub fn advise(candidates: &[ProfileSample], config: &AdviceConfig) -> RoutingAdvice {
    let mut rationale = Vec::new();
    let mut order: Vec<usize> = (0..candidates.len()).collect();
    order.sort_by(|left, right| {
        candidates[*left]
            .profile_key
            .cmp(&candidates[*right].profile_key)
            .then_with(|| {
                candidates[*left]
                    .model_family
                    .cmp(&candidates[*right].model_family)
            })
    });

    let mut eligible: Vec<usize> = Vec::new();
    for &index in &order {
        let candidate = &candidates[index];
        if candidate.model_family != config.locked_model_family {
            rationale.push(format!(
                "{}: model_family {} is not the locked family {}; excluded",
                candidate.profile_key, candidate.model_family, config.locked_model_family
            ));
            continue;
        }
        if candidate.samples < config.min_samples {
            rationale.push(format!(
                "{}: {} samples below configured {}",
                candidate.profile_key, candidate.samples, config.min_samples
            ));
            continue;
        }
        let bound = candidate.success_lower_bound();
        if bound < MIN_SUCCESS_RATE {
            rationale.push(format!(
                "{}: success lower bound {:.2} below {:.2}",
                candidate.profile_key, bound, MIN_SUCCESS_RATE
            ));
            continue;
        }
        rationale.push(format!(
            "{}: eligible (lower bound {:.2}, cost {} micros, wall {} ms)",
            candidate.profile_key, bound, candidate.mean_cost_micros, candidate.mean_wall_ms
        ));
        eligible.push(index);
    }

    eligible.sort_by(|left, right| {
        candidates[*left]
            .mean_cost_micros
            .cmp(&candidates[*right].mean_cost_micros)
            .then_with(|| {
                candidates[*left]
                    .mean_wall_ms
                    .cmp(&candidates[*right].mean_wall_ms)
            })
            .then_with(|| {
                candidates[*left]
                    .profile_key
                    .cmp(&candidates[*right].profile_key)
            })
    });

    match eligible.first().copied() {
        Some(winner) => {
            rationale.push(format!(
                "selected {}: cheapest eligible profile under locked family {}",
                candidates[winner].profile_key, config.locked_model_family
            ));
            RoutingAdvice {
                source: AdviceSource::Benchmark,
                model_family: config.locked_model_family.clone(),
                profile: candidates[winner].profile_key.clone(),
                rationale,
            }
        }
        None => {
            rationale.push(format!(
                "keeping static profile {}: no eligible benchmark candidate",
                config.static_profile
            ));
            RoutingAdvice {
                source: AdviceSource::Static,
                model_family: config.locked_model_family.clone(),
                profile: config.static_profile.clone(),
                rationale,
            }
        }
    }
}
