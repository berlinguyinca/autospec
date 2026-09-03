//! Versioned model capability profiles (AAR spec section 4).
//!
//! A profile is keyed by model, version, quantization, backend and hardware
//! class because those combinations behave differently enough to matter. Scores
//! come from controlled AutoSpec benchmarks and are then *adjusted* — never
//! replaced — by production outcomes, using a shrinkage weight so a handful of
//! production runs cannot overturn a benchmark.

use std::collections::BTreeMap;

use super::classify::Capability;

/// Benchmark scores on the capability axes from spec section 4.
///
/// Every score is `0.0..=1.0`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct CapabilityScores {
    pub coding: f64,
    pub debugging: f64,
    pub planning: f64,
    pub review: f64,
    pub repository_reasoning: f64,
    pub tool_use: f64,
    pub textual_analysis: f64,
    pub documentation: f64,
    pub vision: f64,
    pub context_handling: f64,
    pub concurrency: f64,
}

impl CapabilityScores {
    pub fn uniform(value: f64) -> Self {
        Self {
            coding: value,
            debugging: value,
            planning: value,
            review: value,
            repository_reasoning: value,
            tool_use: value,
            textual_analysis: value,
            documentation: value,
            vision: value,
            context_handling: value,
            concurrency: value,
        }
    }

    pub fn score(&self, capability: Capability) -> f64 {
        match capability {
            Capability::Coding => self.coding,
            Capability::Debugging => self.debugging,
            Capability::Planning => self.planning,
            Capability::Review => self.review,
            Capability::RepositoryReasoning => self.repository_reasoning,
            Capability::ToolUse => self.tool_use,
            Capability::TextualAnalysis => self.textual_analysis,
            Capability::Documentation => self.documentation,
            Capability::Vision => self.vision,
            Capability::ContextHandling => self.context_handling,
            Capability::Concurrency => self.concurrency,
        }
    }

    pub fn set(&mut self, capability: Capability, value: f64) {
        let slot = match capability {
            Capability::Coding => &mut self.coding,
            Capability::Debugging => &mut self.debugging,
            Capability::Planning => &mut self.planning,
            Capability::Review => &mut self.review,
            Capability::RepositoryReasoning => &mut self.repository_reasoning,
            Capability::ToolUse => &mut self.tool_use,
            Capability::TextualAnalysis => &mut self.textual_analysis,
            Capability::Documentation => &mut self.documentation,
            Capability::Vision => &mut self.vision,
            Capability::ContextHandling => &mut self.context_handling,
            Capability::Concurrency => &mut self.concurrency,
        };
        *slot = value;
    }
}

/// Production outcomes accumulated for one profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ProfileObservations {
    pub tasks: u32,
    pub successes: u32,
}

/// Sample count at which production outcomes carry half the weight.
///
/// Below it the benchmark dominates; above it production dominates. Fixed
/// rather than tuned so a profile's blended score is reproducible.
pub const OBSERVATION_SHRINKAGE_K: f64 = 20.0;

impl ProfileObservations {
    pub fn success_rate(&self) -> Option<f64> {
        if self.tasks == 0 {
            return None;
        }
        Some(f64::from(self.successes) / f64::from(self.tasks))
    }

    /// Weight production outcomes get when blended with a benchmark score.
    pub fn weight(&self) -> f64 {
        let tasks = f64::from(self.tasks);
        tasks / (tasks + OBSERVATION_SHRINKAGE_K)
    }
}

/// One versioned model capability profile.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelProfile {
    pub model_id: String,
    pub model_version: String,
    pub quantization: String,
    pub backend: String,
    pub hardware_class: String,
    pub model_class: String,
    pub provider: String,
    pub context_window: u64,
    pub supports_vision: bool,
    pub supports_web: bool,
    pub max_concurrent_sessions: u32,
    /// Micro-dollars (1e-6 USD) per 1000 prompt tokens.
    pub cost_per_1k_prompt_micros: u64,
    /// Micro-dollars (1e-6 USD) per 1000 output tokens.
    pub cost_per_1k_output_micros: u64,
    pub is_local: bool,
    pub scores: CapabilityScores,
    pub observations: ProfileObservations,
    /// Bumped whenever benchmark scores are regenerated.
    pub profile_version: u32,
}

impl ModelProfile {
    /// Stable identity for telemetry and statistics keys.
    pub fn key(&self) -> String {
        format!(
            "{}@{}/{}/{}/{}",
            self.model_id, self.model_version, self.quantization, self.backend, self.hardware_class
        )
    }

    /// Benchmark score adjusted by production outcomes.
    ///
    /// Production shifts the benchmark toward the observed success rate in
    /// proportion to how much evidence exists, so one bad afternoon does not
    /// erase a benchmark and a thousand good runs are not ignored.
    pub fn blended_score(&self, capability: Capability) -> f64 {
        let benchmark = self.scores.score(capability);
        match self.observations.success_rate() {
            None => benchmark,
            Some(observed) => {
                let weight = self.observations.weight();
                (benchmark * (1.0 - weight) + observed * weight).clamp(0.0, 1.0)
            }
        }
    }

    /// Mean blended score across the capabilities a task requires.
    pub fn capability_fit(&self, required: &[Capability]) -> f64 {
        if required.is_empty() {
            return 0.0;
        }
        let total: f64 = required
            .iter()
            .map(|capability| self.blended_score(*capability))
            .sum();
        total / required.len() as f64
    }

    /// Weakest blended score across the required capabilities.
    pub fn weakest_capability(&self, required: &[Capability]) -> Option<(Capability, f64)> {
        required
            .iter()
            .map(|capability| (*capability, self.blended_score(*capability)))
            .min_by(|left, right| {
                left.1
                    .partial_cmp(&right.1)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    }

    pub fn estimated_cost_micros(&self, prompt_tokens: u64, output_tokens: u64) -> u64 {
        let prompt = prompt_tokens.saturating_mul(self.cost_per_1k_prompt_micros) / 1000;
        let output = output_tokens.saturating_mul(self.cost_per_1k_output_micros) / 1000;
        prompt.saturating_add(output)
    }
}

/// What a role needs from a model before it may be assigned (spec section 19).
#[derive(Debug, Clone, PartialEq)]
pub struct ModelRequirements {
    pub model_class: String,
    pub model_allowlist: Vec<String>,
    pub required_capabilities: Vec<Capability>,
    pub minimum_capability_score: f64,
    pub requires_vision: bool,
    pub requires_web: bool,
    pub minimum_context_free: u64,
    pub prefer_local: bool,
}

impl Default for ModelRequirements {
    fn default() -> Self {
        Self {
            model_class: "coding-local".to_string(),
            model_allowlist: Vec::new(),
            required_capabilities: vec![Capability::Coding],
            minimum_capability_score: 0.5,
            requires_vision: false,
            requires_web: false,
            minimum_context_free: 8_000,
            prefer_local: true,
        }
    }
}

/// A profile that satisfied every hard requirement, with its fit score.
#[derive(Debug, Clone, PartialEq)]
pub struct ProfileMatch {
    pub profile: ModelProfile,
    pub fit: f64,
}

/// A profile that did not, with the first reason it failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileRejection {
    pub key: String,
    pub reason: String,
}

/// Ranked candidates plus the audit trail of what was excluded and why.
#[derive(Debug, Clone, PartialEq)]
pub struct ProfileResolution {
    pub matches: Vec<ProfileMatch>,
    pub rejections: Vec<ProfileRejection>,
    pub registry_version: String,
}

impl ProfileResolution {
    pub fn best(&self) -> Option<&ModelProfile> {
        self.matches.first().map(|entry| &entry.profile)
    }
}

/// A versioned collection of profiles.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ModelProfileRegistry {
    pub version: String,
    profiles: Vec<ModelProfile>,
}

impl ModelProfileRegistry {
    pub fn new(version: impl Into<String>, profiles: Vec<ModelProfile>) -> Self {
        Self {
            version: version.into(),
            profiles,
        }
    }

    pub fn profiles(&self) -> &[ModelProfile] {
        &self.profiles
    }

    pub fn get(&self, key: &str) -> Option<&ModelProfile> {
        self.profiles.iter().find(|profile| profile.key() == key)
    }

    pub fn by_model_id(&self, model_id: &str) -> Vec<&ModelProfile> {
        self.profiles
            .iter()
            .filter(|profile| profile.model_id == model_id)
            .collect()
    }

    /// Record a production outcome against a profile key.
    pub fn record_outcome(&mut self, key: &str, success: bool) -> Result<(), String> {
        let profile = self
            .profiles
            .iter_mut()
            .find(|profile| profile.key() == key)
            .ok_or_else(|| format!("unknown model profile: {key}"))?;
        profile.observations.tasks += 1;
        if success {
            profile.observations.successes += 1;
        }
        Ok(())
    }

    /// Rank every profile that satisfies the hard requirements.
    ///
    /// Hard requirements are filters, not score contributions: a profile that
    /// cannot serve the required class, cannot hold the projected context, or
    /// lacks vision/web when the task needs it is rejected outright rather than
    /// out-scored, because a high score cannot make it able to do the work.
    pub fn resolve(&self, requirements: &ModelRequirements) -> ProfileResolution {
        let mut matches = Vec::new();
        let mut rejections = Vec::new();

        for profile in &self.profiles {
            if let Some(reason) = reject_reason(profile, requirements) {
                rejections.push(ProfileRejection {
                    key: profile.key(),
                    reason,
                });
                continue;
            }
            matches.push(ProfileMatch {
                fit: profile.capability_fit(&requirements.required_capabilities),
                profile: profile.clone(),
            });
        }

        matches.sort_by(|left, right| {
            let preference = |entry: &ProfileMatch| {
                if requirements.prefer_local && entry.profile.is_local {
                    1
                } else {
                    0
                }
            };
            preference(right)
                .cmp(&preference(left))
                .then_with(|| {
                    right
                        .fit
                        .partial_cmp(&left.fit)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .then_with(|| left.profile.key().cmp(&right.profile.key()))
        });

        ProfileResolution {
            matches,
            rejections,
            registry_version: self.version.clone(),
        }
    }
}

fn reject_reason(profile: &ModelProfile, requirements: &ModelRequirements) -> Option<String> {
    if !requirements.model_allowlist.is_empty()
        && !requirements
            .model_allowlist
            .iter()
            .any(|allowed| allowed == &profile.model_id)
    {
        return Some(format!("model {} not in allowlist", profile.model_id));
    }
    if !requirements.model_class.is_empty() && profile.model_class != requirements.model_class {
        return Some(format!(
            "model class {} != required {}",
            profile.model_class, requirements.model_class
        ));
    }
    if requirements.requires_vision && !profile.supports_vision {
        return Some("model does not support vision".to_string());
    }
    if requirements.requires_web && !profile.supports_web {
        return Some("model does not support web access".to_string());
    }
    if profile.context_window < requirements.minimum_context_free {
        return Some(format!(
            "context window {} < required {}",
            profile.context_window, requirements.minimum_context_free
        ));
    }
    for capability in &requirements.required_capabilities {
        let score = profile.blended_score(*capability);
        if score < requirements.minimum_capability_score {
            return Some(format!(
                "{} score {score:.2} < minimum {:.2}",
                capability.as_str(),
                requirements.minimum_capability_score
            ));
        }
    }
    None
}

/// Per-capability overrides expressed as a map, for config-driven registries.
pub fn scores_from_map(defaults: f64, overrides: &BTreeMap<Capability, f64>) -> CapabilityScores {
    let mut scores = CapabilityScores::uniform(defaults);
    for (capability, value) in overrides {
        scores.set(*capability, *value);
    }
    scores
}

impl ModelProfileRegistry {
    /// A starter registry so the CLI and tests have something concrete.
    ///
    /// The capability scores here are placeholders with the right shape, not
    /// measurements. They are replaced by the benchmark and advisory framework
    /// (spec section 16); treat any score in this function as unmeasured until
    /// a benchmark run has written over it.
    pub fn starter() -> Self {
        Self::new(
            "starter-v1",
            vec![
                ModelProfile {
                    model_id: "qwen3.8-27b".to_string(),
                    model_version: "3.8".to_string(),
                    quantization: "q4_k_m".to_string(),
                    backend: "vllm".to_string(),
                    hardware_class: "rtx4090".to_string(),
                    model_class: "coding-local".to_string(),
                    provider: "inferweave".to_string(),
                    context_window: 65_536,
                    supports_vision: false,
                    supports_web: false,
                    max_concurrent_sessions: 3,
                    cost_per_1k_prompt_micros: 0,
                    cost_per_1k_output_micros: 0,
                    is_local: true,
                    scores: CapabilityScores {
                        coding: 0.74,
                        debugging: 0.68,
                        planning: 0.62,
                        review: 0.64,
                        repository_reasoning: 0.66,
                        tool_use: 0.72,
                        textual_analysis: 0.70,
                        documentation: 0.71,
                        vision: 0.0,
                        context_handling: 0.65,
                        concurrency: 0.60,
                    },
                    observations: ProfileObservations::default(),
                    profile_version: 1,
                },
                ModelProfile {
                    model_id: "qwen3.8-27b".to_string(),
                    model_version: "3.8".to_string(),
                    quantization: "bf16".to_string(),
                    backend: "vllm".to_string(),
                    hardware_class: "dual-turing".to_string(),
                    model_class: "coding-local".to_string(),
                    provider: "inferweave".to_string(),
                    context_window: 32_768,
                    supports_vision: false,
                    supports_web: false,
                    max_concurrent_sessions: 2,
                    cost_per_1k_prompt_micros: 0,
                    cost_per_1k_output_micros: 0,
                    is_local: true,
                    scores: CapabilityScores {
                        coding: 0.78,
                        debugging: 0.72,
                        planning: 0.66,
                        review: 0.68,
                        repository_reasoning: 0.70,
                        tool_use: 0.74,
                        textual_analysis: 0.72,
                        documentation: 0.73,
                        vision: 0.0,
                        context_handling: 0.60,
                        concurrency: 0.50,
                    },
                    observations: ProfileObservations::default(),
                    profile_version: 1,
                },
                ModelProfile {
                    model_id: "cloud-frontier".to_string(),
                    model_version: "latest".to_string(),
                    quantization: "none".to_string(),
                    backend: "provider-api".to_string(),
                    hardware_class: "cloud".to_string(),
                    model_class: "coding-cloud".to_string(),
                    provider: "cloud".to_string(),
                    context_window: 200_000,
                    supports_vision: true,
                    supports_web: true,
                    max_concurrent_sessions: 16,
                    cost_per_1k_prompt_micros: 3_000,
                    cost_per_1k_output_micros: 15_000,
                    is_local: false,
                    scores: CapabilityScores::uniform(0.90),
                    observations: ProfileObservations::default(),
                    profile_version: 1,
                },
            ],
        )
    }
}
