//! Agentic RAG configuration (spec section 51).
//!
//! Parsed from the `agentic_rag:` block. The parser is deliberate about two
//! things: an unknown key is an error rather than a silent no-op, and a
//! disabled source cannot be re-enabled by a role policy. A typo in a security
//! setting that reads as "default" is the failure mode this avoids.

use std::collections::BTreeMap;

use crate::rag::budget::RetrievalBudget;
use crate::rag::policy::{ALL_ROLES, AgentRole, PolicySet, RetrievalPolicy};
use crate::rag::source::{ALL_SOURCE_KINDS, SourceKind};

/// Whether a source may be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceAvailability {
    /// Always available.
    Enabled,
    /// Never available.
    Disabled,
    /// Available only when the task's policy explicitly permits it.
    PolicyGated,
}

impl SourceAvailability {
    /// Stable wire identifier.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Enabled => "true",
            Self::Disabled => "false",
            Self::PolicyGated => "policy",
        }
    }

    /// Parse a `true` / `false` / `policy` value.
    pub fn parse(text: &str) -> Result<Self, String> {
        match text.trim() {
            "true" | "yes" | "on" => Ok(Self::Enabled),
            "false" | "no" | "off" => Ok(Self::Disabled),
            "policy" => Ok(Self::PolicyGated),
            other => Err(format!(
                "source availability must be true, false or policy, found: {other}"
            )),
        }
    }

    /// Return `true` when a retrieval may reach this source.
    ///
    /// `policy_permits` is the caller's decision for a gated source; a disabled
    /// source ignores it entirely.
    pub fn allows(self, policy_permits: bool) -> bool {
        match self {
            Self::Enabled => true,
            Self::Disabled => false,
            Self::PolicyGated => policy_permits,
        }
    }
}

/// Graph traversal settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphConfig {
    /// Whether graph retrieval is available.
    pub enabled: bool,
    /// Maximum traversal depth.
    pub max_depth: u32,
}

impl Default for GraphConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_depth: 4,
        }
    }
}

/// Cache settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheConfig {
    /// Whether caching is available.
    pub enabled: bool,
    /// Whether cache keys embed the source revision.
    pub revision_aware: bool,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            revision_aware: true,
        }
    }
}

impl CacheConfig {
    /// Reject a cache that is on but not revision-aware.
    ///
    /// Section 25 makes revision-awareness the property that keeps a cached
    /// answer from commit `abc123` out of a retrieval at `def456`. A cache
    /// without it is not a slower cache, it is a source of wrong evidence.
    pub fn validate(&self) -> Result<(), String> {
        if self.enabled && !self.revision_aware {
            return Err(
                "cache.revision_aware cannot be false while the cache is enabled: \
                 a revision-blind cache serves stale evidence as current"
                    .to_string(),
            );
        }
        Ok(())
    }
}

/// The parsed `agentic_rag:` configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RagConfig {
    /// Whether the subsystem is active.
    pub enabled: bool,
    /// Default budget for every retrieval.
    pub default_budget: RetrievalBudget,
    /// Role policies.
    pub policies: PolicySet,
    /// Per-source availability.
    pub sources: BTreeMap<SourceKind, SourceAvailability>,
    /// Graph settings.
    pub graph: GraphConfig,
    /// Cache settings.
    pub cache: CacheConfig,
}

impl Default for RagConfig {
    /// The defaults from specification section 51.
    fn default() -> Self {
        let mut sources = BTreeMap::new();
        for kind in ALL_SOURCE_KINDS {
            let availability = match kind {
                SourceKind::Web => SourceAvailability::PolicyGated,
                _ => SourceAvailability::Enabled,
            };
            sources.insert(kind, availability);
        }
        Self {
            enabled: true,
            default_budget: RetrievalBudget {
                max_iterations: 8,
                max_queries: 24,
                max_evidence_items: 80,
                max_context_tokens: 40_000,
                ..RetrievalBudget::default()
            },
            policies: PolicySet::default(),
            sources,
            graph: GraphConfig::default(),
            cache: CacheConfig::default(),
        }
    }
}

impl RagConfig {
    /// Availability of one source.
    pub fn availability(&self, kind: SourceKind) -> SourceAvailability {
        self.sources
            .get(&kind)
            .copied()
            .unwrap_or(SourceAvailability::Disabled)
    }

    /// Return `true` when a role may reach a source.
    ///
    /// Three things must agree. The administrator's setting is the ceiling: a
    /// disabled source is unreachable whatever anyone else says. The role's
    /// policy must list the source at all — priority or deprioritized, since
    /// deprioritized means "last", not "forbidden". And a policy-gated source
    /// additionally needs the *task* to have asked for it, which is what
    /// section 8.9 means by external research being used when the task
    /// explicitly requires it: a role listing the web is not a standing licence
    /// to browse.
    pub fn source_allowed(
        &self,
        kind: SourceKind,
        role: AgentRole,
        task_permits_gated: bool,
    ) -> bool {
        let policy = self.policies.policy(role);
        let listed = policy.priority_sources().contains(&kind)
            || policy.deprioritized_sources().contains(&kind);
        if !listed {
            return false;
        }
        self.availability(kind).allows(task_permits_gated)
    }

    /// The effective budget for a role.
    pub fn budget_for(&self, role: AgentRole) -> RetrievalBudget {
        self.policies
            .policy(role)
            .apply_to_budget(&self.default_budget)
    }

    /// Validate the whole configuration.
    pub fn validate(&self) -> Result<(), String> {
        self.default_budget.validate()?;
        self.cache.validate()?;
        if self.graph.enabled && self.graph.max_depth == 0 {
            return Err("graph.max_depth must be greater than zero when the graph is enabled"
                .to_string());
        }
        for role in ALL_ROLES {
            let budget = self.budget_for(role);
            budget.validate().map_err(|error| {
                format!("role {} budget is invalid: {error}", role.as_str())
            })?;
        }
        Ok(())
    }

    /// Apply a flat `key = value` override, as a CLI flag or environment
    /// variable would.
    ///
    /// Keys use the dotted paths of section 51, e.g.
    /// `default.max_iterations`, `sources.web`, `roles.planner.max_context_tokens`.
    pub fn apply_override(&mut self, key: &str, value: &str) -> Result<(), String> {
        let parts = key.split('.').collect::<Vec<_>>();
        match parts.as_slice() {
            ["enabled"] => {
                self.enabled = parse_bool(value)?;
            }
            ["default", field] => {
                apply_budget_field(&mut self.default_budget, field, value)?;
            }
            ["sources", source] => {
                let kind = SourceKind::parse(source)?;
                self.sources.insert(kind, SourceAvailability::parse(value)?);
            }
            ["graph", "enabled"] => self.graph.enabled = parse_bool(value)?,
            ["graph", "max_depth"] => self.graph.max_depth = parse_u32(value)?,
            ["cache", "enabled"] => self.cache.enabled = parse_bool(value)?,
            ["cache", "revision_aware"] => self.cache.revision_aware = parse_bool(value)?,
            ["roles", role, "max_context_tokens"] => {
                let role = AgentRole::parse(role)?;
                let policy = self
                    .policies
                    .policy(role)
                    .clone()
                    .with_max_context_tokens(parse_u32(value)?);
                self.policies.set(policy);
            }
            _ => return Err(format!("unknown agentic_rag configuration key: {key}")),
        }
        Ok(())
    }

    /// Render the configuration as the YAML of section 51.
    pub fn render_yaml(&self) -> String {
        let mut yaml = String::from("agentic_rag:\n");
        yaml.push_str(&format!("  enabled: {}\n\n", self.enabled));
        yaml.push_str("  default:\n");
        yaml.push_str(&format!(
            "    max_iterations: {}\n    max_queries: {}\n    max_evidence: {}\n    max_context_tokens: {}\n\n",
            self.default_budget.max_iterations,
            self.default_budget.max_queries,
            self.default_budget.max_evidence_items,
            self.default_budget.max_context_tokens
        ));
        yaml.push_str("  roles:\n");
        for policy in self.policies.policies() {
            yaml.push_str(&format!(
                "    {}:\n      policy: {}\n      max_context_tokens: {}\n",
                policy.role().as_str(),
                policy.name(),
                policy.max_context_tokens()
            ));
        }
        yaml.push_str("\n  sources:\n");
        for kind in ALL_SOURCE_KINDS {
            yaml.push_str(&format!(
                "    {}: {}\n",
                kind.as_str(),
                self.availability(kind).as_str()
            ));
        }
        yaml.push_str(&format!(
            "\n  graph:\n    enabled: {}\n    max_depth: {}\n",
            self.graph.enabled, self.graph.max_depth
        ));
        yaml.push_str(&format!(
            "\n  cache:\n    enabled: {}\n    revision_aware: {}\n",
            self.cache.enabled, self.cache.revision_aware
        ));
        yaml
    }

    /// The policy for a role, with the configuration's context ceiling applied.
    pub fn policy_for(&self, role: AgentRole) -> RetrievalPolicy {
        self.policies.policy(role).clone()
    }
}

fn apply_budget_field(budget: &mut RetrievalBudget, field: &str, value: &str) -> Result<(), String> {
    let parsed = parse_u32(value)?;
    match field {
        "max_iterations" => budget.max_iterations = parsed,
        "max_queries" => budget.max_queries = parsed,
        "max_external_queries" => budget.max_external_queries = parsed,
        "max_evidence" | "max_evidence_items" => budget.max_evidence_items = parsed,
        "max_context_tokens" => budget.max_context_tokens = parsed,
        "max_model_tokens" => budget.max_model_tokens = parsed,
        "max_wall_clock_seconds" => budget.max_wall_clock_seconds = parsed,
        "max_unproductive_iterations" => budget.max_unproductive_iterations = parsed,
        other => return Err(format!("unknown budget field: {other}")),
    }
    Ok(())
}

fn parse_bool(value: &str) -> Result<bool, String> {
    match value.trim() {
        "true" | "yes" | "on" | "1" => Ok(true),
        "false" | "no" | "off" | "0" => Ok(false),
        other => Err(format!("expected a boolean, found: {other}")),
    }
}

fn parse_u32(value: &str) -> Result<u32, String> {
    value
        .trim()
        .parse()
        .map_err(|_| format!("expected a non-negative integer, found: {value}"))
}
