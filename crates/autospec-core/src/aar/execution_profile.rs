//! Model-agnostic execution profiles with enforceable budgets (issue #3317).
//!
//! An execution profile is a named budget envelope for one session tier: how
//! much reasoning, context, turns, repairs, and wall clock a run may spend.
//! The profile references a model by id only — no provider, backend, or
//! hardware appears in the contract — so the same profile works on whatever
//! runtime serves that model, mirroring the rest of the AAR module.
//!
//! The profile name is fully derived from the model family and the tier
//! (`{family-slug}-{tier}`), which makes the family binding checkable: a
//! profile named for the `qwen3.8` family with a different family fails
//! schema validation instead of silently routing work at the wrong ceiling.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The three execution tiers a profile may carry.
pub const TIERS: [&str; 3] = ["fast", "coding", "deep"];

/// Version tag for the built-in registry.
pub const REGISTRY_VERSION: &str = "execution-profiles-v1";

/// Enforceable budgets for one execution profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionBudgets {
    /// Maximum reasoning tokens the harness may spend per turn.
    pub reasoning_tokens: u32,
    /// Hard prompt/context ceiling in tokens; a session that reaches it stops.
    pub hard_context_limit: u32,
    /// Maximum agent turns (model + tool round trips) per session.
    pub max_turns: u32,
    /// Maximum repair/retry attempts; zero forbids repair entirely.
    pub max_repairs: u32,
    /// Wall-clock budget for the whole session, in milliseconds.
    pub wall_clock_ms: u64,
}

impl ExecutionBudgets {
    /// Schema validation for the budget envelope.
    pub fn validate(&self) -> Result<(), String> {
        if self.reasoning_tokens == 0 {
            return Err("execution budgets require a non-zero reasoning token budget".to_string());
        }
        if self.hard_context_limit == 0 {
            return Err("execution budgets require a non-zero hard context limit".to_string());
        }
        if self.max_turns == 0 {
            return Err("execution budgets require a non-zero turn budget".to_string());
        }
        if self.wall_clock_ms == 0 {
            return Err("execution budgets require a non-zero wall-clock budget".to_string());
        }
        Ok(())
    }
}

/// One model-agnostic execution profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionProfile {
    /// Full profile name, `{family-slug}-{tier}`, e.g. `qwen38-fast`.
    pub name: String,
    /// Model family the profile is defined for, e.g. `qwen3.8`.
    pub family: String,
    /// Model id the profile maps to, e.g. `qwen3.8-27b`.
    pub model: String,
    pub budgets: ExecutionBudgets,
}

/// Slug used to bind a model family to profile names: `qwen3.8` -> `qwen38`.
pub fn family_slug(family: &str) -> String {
    family
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect()
}

impl ExecutionProfile {
    /// Schema validation for the profile.
    ///
    /// The family binding is enforced by construction: the name must be
    /// `{family-slug}-{tier}` for one of the known tiers, and the model must
    /// belong to the declared family.
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("execution profile requires a name".to_string());
        }
        for ch in self.name.chars() {
            if !(ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-') {
                return Err(format!(
                    "execution profile name {} may only contain lowercase letters, digits and hyphens",
                    self.name
                ));
            }
        }
        if self.name.starts_with('-') || self.name.ends_with('-') || self.name.contains("--") {
            return Err(format!(
                "execution profile name {} must not start or end with a hyphen or contain consecutive hyphens",
                self.name
            ));
        }
        let slug = family_slug(&self.family);
        if slug.is_empty() {
            return Err(
                "execution profile requires a model family with alphanumeric characters"
                    .to_string(),
            );
        }
        let tier = self.name.strip_prefix(&format!("{slug}-")).ok_or_else(|| {
            format!(
                "execution profile {} does not belong to family {} (expected name prefix {}-)",
                self.name, self.family, slug
            )
        })?;
        if !TIERS.contains(&tier) {
            return Err(format!(
                "execution profile {} has unknown tier {}; expected one of {}",
                self.name,
                tier,
                TIERS.join("|")
            ));
        }
        if self.model.trim().is_empty() {
            return Err(format!("execution profile {} requires a model", self.name));
        }
        if !self.model.starts_with(self.family.as_str()) {
            return Err(format!(
                "execution profile {} maps model {} outside family {}",
                self.name, self.model, self.family
            ));
        }
        self.budgets.validate()
    }

    /// The tier this profile runs at (`fast`, `coding`, or `deep`).
    ///
    /// Requires the name to be well-formed; use `validate` first for
    /// untrusted input.
    pub fn tier(&self) -> &str {
        let slug = family_slug(&self.family);
        self.name.strip_prefix(&format!("{slug}-")).unwrap_or("")
    }

    /// Deterministic key/value view of the effective profile, for embedding
    /// in session metadata and telemetry.
    pub fn session_metadata(&self) -> BTreeMap<String, String> {
        let mut meta = BTreeMap::new();
        meta.insert("aar.execution_profile".to_string(), self.name.clone());
        meta.insert(
            "aar.execution_profile.family".to_string(),
            self.family.clone(),
        );
        meta.insert(
            "aar.execution_profile.model".to_string(),
            self.model.clone(),
        );
        meta.insert(
            "aar.execution_profile.budgets.reasoning_tokens".to_string(),
            self.budgets.reasoning_tokens.to_string(),
        );
        meta.insert(
            "aar.execution_profile.budgets.hard_context_limit".to_string(),
            self.budgets.hard_context_limit.to_string(),
        );
        meta.insert(
            "aar.execution_profile.budgets.max_turns".to_string(),
            self.budgets.max_turns.to_string(),
        );
        meta.insert(
            "aar.execution_profile.budgets.max_repairs".to_string(),
            self.budgets.max_repairs.to_string(),
        );
        meta.insert(
            "aar.execution_profile.budgets.wall_clock_ms".to_string(),
            self.budgets.wall_clock_ms.to_string(),
        );
        meta
    }
}

/// A set of execution profiles with a schema version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionProfileRegistry {
    pub version: String,
    #[serde(default)]
    pub profiles: Vec<ExecutionProfile>,
}

impl ExecutionProfileRegistry {
    pub fn new(version: impl Into<String>, profiles: Vec<ExecutionProfile>) -> Self {
        Self {
            version: version.into(),
            profiles,
        }
    }

    /// Schema validation for the whole registry.
    pub fn validate(&self) -> Result<(), String> {
        if self.version.trim().is_empty() {
            return Err("execution profile registry requires a version".to_string());
        }
        let mut seen = Vec::new();
        for profile in &self.profiles {
            if seen.contains(&profile.name) {
                return Err(format!("duplicate execution profile name {}", profile.name));
            }
            seen.push(profile.name.clone());
            profile.validate()?;
        }
        Ok(())
    }

    pub fn get(&self, name: &str) -> Option<&ExecutionProfile> {
        self.profiles.iter().find(|profile| profile.name == name)
    }

    pub fn names(&self) -> Vec<String> {
        self.profiles
            .iter()
            .map(|profile| profile.name.clone())
            .collect()
    }
}

/// Built-in registry mapping the three `qwen38` tiers to Qwen3.8-27B.
///
/// The mapping is by model id only: no provider, backend, or hardware is
/// named, so the same registry works on whatever runtime serves the model.
pub fn default_registry() -> ExecutionProfileRegistry {
    fn qwen38(name: &str, budgets: ExecutionBudgets) -> ExecutionProfile {
        ExecutionProfile {
            name: name.to_string(),
            family: "qwen3.8".to_string(),
            model: "qwen3.8-27b".to_string(),
            budgets,
        }
    }

    ExecutionProfileRegistry::new(
        REGISTRY_VERSION,
        vec![
            // Fast tier: shallow reasoning, small hard context ceiling, few turns.
            qwen38(
                "qwen38-fast",
                ExecutionBudgets {
                    reasoning_tokens: 512,
                    hard_context_limit: 24_000,
                    max_turns: 6,
                    max_repairs: 1,
                    wall_clock_ms: 1_800_000,
                },
            ),
            // Coding tier: the workhorse dispatch, more turns and context.
            qwen38(
                "qwen38-coding",
                ExecutionBudgets {
                    reasoning_tokens: 4_096,
                    hard_context_limit: 32_768,
                    max_turns: 10,
                    max_repairs: 2,
                    wall_clock_ms: 3_600_000,
                },
            ),
            // Deep tier: exceptional reasoning over a large context, but the
            // turn budget stays small — depth comes from reasoning, not churn.
            qwen38(
                "qwen38-deep",
                ExecutionBudgets {
                    reasoning_tokens: 8_192,
                    hard_context_limit: 65_536,
                    max_turns: 6,
                    max_repairs: 2,
                    wall_clock_ms: 7_200_000,
                },
            ),
        ],
    )
}
