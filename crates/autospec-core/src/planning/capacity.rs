//! Fleet capacity resolution for parallel decomposition (issue #3821).
//!
//! Spec: `docs/specs/2026-09-08-parallel-decomposition-fleet-saturation.md`
//! §6 (Fleet Capacity Model) and §7.1 (Target Initial Width).
//!
//! Invariants:
//! - one resolver, reused by every caller;
//! - a hostile config value can never drive unbounded fan-out (the result is
//!   always clamped to the closed range `MIN..=MAX` supported agents);
//! - a missing config file falls back to the next source, it never fails the
//!   run.

use std::fs;
use std::path::Path;
use std::str::FromStr;

use crate::AutospecError;
use yaml_edit::{Document, Mapping};

/// Lower bound of the supported fleet capacity range (§6.1).
pub const MIN_SUPPORTED_AGENTS: u32 = 10;
/// Upper bound of the supported fleet capacity range (§6.1).
pub const MAX_SUPPORTED_AGENTS: u32 = 100;
/// Default decomposition target when no other source is present (§6.2).
pub const DEFAULT_TARGET_AGENTS: u32 = 32;
/// Fraction of the fleet the initial wave should keep busy (§7.1).
pub const TARGET_INITIAL_WIDTH_RATIO: f64 = 0.6;

/// The five capacity sources (§6.2), highest precedence first.
#[derive(Debug, Clone, Copy)]
pub struct CapacityInputs {
    /// Explicit invocation flag (`--agents`). Highest precedence.
    pub agents_flag: Option<i64>,
    /// `planning.parallelism.target_agents` from the project config.
    pub config_target_agents: Option<i64>,
    /// Orchestrator-discovered effective implementation capacity.
    pub orchestrator_capacity: Option<i64>,
    /// `AUTOSPEC_AGENT_CAPACITY` environment variable.
    pub agent_capacity_env: Option<i64>,
    /// Fallback default; `DEFAULT_TARGET_AGENTS` unless overridden.
    pub default_agents: u32,
}

impl Default for CapacityInputs {
    fn default() -> Self {
        Self {
            agents_flag: None,
            config_target_agents: None,
            orchestrator_capacity: None,
            agent_capacity_env: None,
            default_agents: DEFAULT_TARGET_AGENTS,
        }
    }
}

/// Resolve the effective fleet capacity: the first present source wins
/// (flag, config, orchestrator, env, default) and the result is always
/// clamped to the supported range.
pub fn resolve_capacity(inputs: &CapacityInputs) -> u32 {
    let raw = inputs
        .agents_flag
        .or(inputs.config_target_agents)
        .or(inputs.orchestrator_capacity)
        .or(inputs.agent_capacity_env)
        .unwrap_or(inputs.default_agents as i64);
    clamp_capacity(raw)
}

/// Bound an arbitrary capacity value to the closed range 10..=100.
pub fn clamp_capacity(value: i64) -> u32 {
    if value < MIN_SUPPORTED_AGENTS as i64 {
        MIN_SUPPORTED_AGENTS
    } else if value > MAX_SUPPORTED_AGENTS as i64 {
        MAX_SUPPORTED_AGENTS
    } else {
        value as u32
    }
}

/// Target initial wave width for `issue_count` issues at `capacity` agents:
/// `min(capacity, ceil(issue_count * 0.60))` (§7.1). A target, not a
/// correctness gate.
pub fn target_initial_width(issue_count: usize, capacity: u32) -> usize {
    let width_from_issues = issue_count.saturating_mul(60).div_ceil(100);
    width_from_issues.min(capacity as usize)
}

/// Read `planning.parallelism.target_agents` from a project config YAML file.
///
/// A missing file yields `Ok(None)` so the resolver falls back to the next
/// source; a present but malformed file is an error.
pub fn config_target_agents(path: &Path) -> Result<Option<i64>, AutospecError> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(AutospecError::Io {
                operation: "read".to_string(),
                path: path.display().to_string(),
                source: error.to_string(),
            })
        }
    };
    let source = String::from_utf8(bytes).map_err(|error| AutospecError::Parse {
        context: "planning config".to_string(),
        message: error.to_string(),
    })?;
    let document = Document::from_str(&source)
        .map_err(|error| parse_error("planning config", error.to_string()))?;
    let Some(root) = document.as_mapping() else {
        return Err(parse_error("planning config", "root must be a mapping"));
    };
    let Some(planning) = mapping_value(&root, "planning", "planning")? else {
        return Ok(None);
    };
    let Some(parallelism) = mapping_value(&planning, "parallelism", "planning.parallelism")? else {
        return Ok(None);
    };
    let Some(target) = parallelism.get("target_agents") else {
        return Ok(None);
    };
    let label = "planning.parallelism.target_agents";
    let scalar = target
        .as_scalar()
        .ok_or_else(|| parse_error(label, "must be a scalar"))?;
    let text = scalar.as_string();
    let value = text
        .parse::<i64>()
        .map_err(|_| parse_error(label, format!("must be an integer, got: {text}")))?;
    Ok(Some(value))
}

fn mapping_value(
    mapping: &Mapping,
    key: &str,
    label: &str,
) -> Result<Option<Mapping>, AutospecError> {
    let Some(node) = mapping.get(key) else {
        return Ok(None);
    };
    node.as_mapping()
        .cloned()
        .map(Some)
        .ok_or_else(|| parse_error(label, "must be a mapping"))
}

fn parse_error(context: &str, message: impl Into<String>) -> AutospecError {
    AutospecError::Parse {
        context: context.to_string(),
        message: message.into(),
    }
}
