//! Dynamic Memory Map — bounded, task-specific shared memory retrieval.
//!
//! Agents must not receive an indiscriminate dump of shared memory. They should
//! receive a small, task-specific map of relevant memory domains and retrieve
//! details only when required. This module renders and validates such a map and
//! defines the narrow provider contract used to back it.
//!
//! The provider abstraction is deliberately narrow and does **not** couple
//! AutoSpec to a specific memory service (OpenViking / MemPalace today, others
//! later). AutoSpec's existing shared-memory integration is the foundation;
//! this module only defines the contract AutoSpec needs on top of it.

use serde::{Deserialize, Serialize};

/// Versioned memory-map schema identity.
pub const MEMORY_MAP_SCHEMA: &str = "autospec.memory-map.v1";

/// Default token budget for a task-specific map.
pub const DEFAULT_MAP_TOKEN_BUDGET: usize = 3000;

/// The kind of a memory entry; drives provenance, confidence and retrieval.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MemoryKind {
    Fact,
    Decision,
    Procedure,
    Candidate,
    Superseded,
    Warning,
}

impl MemoryKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            MemoryKind::Fact => "fact",
            MemoryKind::Decision => "decision",
            MemoryKind::Procedure => "procedure",
            MemoryKind::Candidate => "candidate",
            MemoryKind::Superseded => "superseded",
            MemoryKind::Warning => "warning",
        }
    }
}

/// One referenced memory domain within a map.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryDomainRef {
    pub key: String,
    pub reason: String,
    /// high | medium | low
    pub relevance: String,
    pub refs: Vec<String>,
}

/// A map entry carrying provenance and confidence.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryEntryRef {
    pub summary: String,
    pub kind: MemoryKind,
    pub scope: Option<String>,
    pub confidence: f32,
    pub status: String,
    pub provenance: String,
    pub supersedes: Option<String>,
}

/// A bounded, task-specific memory map.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryMap {
    pub schema: String,
    pub task: String,
    pub repository: Option<String>,
    pub role: Option<String>,
    pub generated_at: String,
    pub domains: Vec<MemoryDomainRef>,
    pub recent_decisions: Vec<MemoryEntryRef>,
    pub validated_procedures: Vec<MemoryEntryRef>,
    pub warnings: Vec<MemoryEntryRef>,
    pub suggested_queries: Vec<Suggestion>,
    /// Explicit degraded-mode marker set when the provider was unavailable.
    pub degraded: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Suggestion {
    pub query: String,
    pub reason: String,
}

impl MemoryMap {
    pub fn new(task: impl Into<String>) -> Self {
        Self {
            schema: MEMORY_MAP_SCHEMA.to_string(),
            task: task.into(),
            repository: None,
            role: None,
            generated_at: String::new(),
            domains: Vec::new(),
            recent_decisions: Vec::new(),
            validated_procedures: Vec::new(),
            warnings: Vec::new(),
            suggested_queries: Vec::new(),
            degraded: false,
        }
    }

    /// Estimate the map size in "tokens" (chars/4 proxy), for budget checks.
    pub fn estimated_tokens(&self) -> usize {
        let json = serde_json::to_string(self).unwrap_or_default();
        json.chars().count() / 4
    }
}

/// The narrow provider contract. Implementations may shell out to a memory
/// service (e.g. MemPalace) or read a local store. AutoSpec never fabricates
/// memory: a provider that fails yields `Err`, and the caller degrades
/// explicitly rather than inventing entries.
pub trait MemoryProvider {
    /// Search for memory relevant to `query`, returning raw references.
    fn search(&self, query: &str) -> Result<Vec<MemoryEntryRef>, String>;

    /// Wake-up / baseline context for the current session/project.
    fn wake_up(&self) -> Result<Vec<MemoryEntryRef>, String>;

    /// Whether the provider is currently available.
    fn available(&self) -> bool;
}

/// Build a bounded map for a task by querying a provider and filtering to the
/// token budget. Entries that would exceed the budget are dropped rather than
/// truncated, so the map stays a set of references, not a payload dump.
pub fn generate_map(
    provider: &dyn MemoryProvider,
    task: &str,
    budget_tokens: usize,
) -> Result<MemoryMap, String> {
    if !provider.available() {
        // Degrade explicitly; never fabricate.
        let mut map = MemoryMap::new(task);
        map.degraded = true;
        return Ok(map);
    }

    let results = provider.search(task)?;
    let mut map = MemoryMap::new(task);
    let mut budget_used = 0usize;

    for entry in results {
        let approx = entry.summary.chars().count() / 4;
        if budget_used + approx > budget_tokens {
            continue; // bounded: drop rather than dump
        }
        budget_used += approx;
        match entry.kind {
            MemoryKind::Warning => map.warnings.push(entry),
            MemoryKind::Procedure => map.validated_procedures.push(entry),
            _ => map.recent_decisions.push(entry),
        }
    }
    Ok(map)
}

/// Resolve conflicting memory by preferring recent validated memory, but never
/// allowing a retrieved entry to outrank system policy. Returns the preferred
/// entry and a note about the conflict.
pub fn resolve_conflict(
    current: &MemoryEntryRef,
    candidate: &MemoryEntryRef,
) -> (MemoryEntryRef, bool) {
    // A superseded entry never wins.
    let preferred = if candidate.kind == MemoryKind::Superseded {
        current.clone()
    } else if candidate.confidence > current.confidence
        && candidate.status == "validated"
        && current.status != "validated"
    {
        candidate.clone()
    } else {
        current.clone()
    };
    let conflict = preferred.as_str() != current.as_str();
    (preferred, conflict)
}

impl MemoryEntryRef {
    fn as_str(&self) -> &str {
        &self.summary
    }
}

/// A memory provider that reads from a static set of entries (used by tests and
/// by offline/degraded callers that only need deterministic references).
#[derive(Debug, Default, Clone)]
pub struct StaticMemoryProvider {
    pub entries: Vec<MemoryEntryRef>,
    pub available: bool,
}

impl MemoryProvider for StaticMemoryProvider {
    fn search(&self, _query: &str) -> Result<Vec<MemoryEntryRef>, String> {
        if !self.available {
            return Err("memory unavailable".to_string());
        }
        Ok(self.entries.clone())
    }
    fn wake_up(&self) -> Result<Vec<MemoryEntryRef>, String> {
        if !self.available {
            return Err("memory unavailable".to_string());
        }
        Ok(self.entries.clone())
    }
    fn available(&self) -> bool {
        self.available
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(summary: &str, kind: MemoryKind, confidence: f32, status: &str) -> MemoryEntryRef {
        MemoryEntryRef {
            summary: summary.to_string(),
            kind,
            scope: None,
            confidence,
            status: status.to_string(),
            provenance: "test".to_string(),
            supersedes: None,
        }
    }

    #[test]
    fn unavailable_provider_degrades_explicitly_without_fabricating() {
        let provider = StaticMemoryProvider {
            entries: vec![entry("x", MemoryKind::Fact, 0.9, "validated")],
            available: false,
        };
        let map = generate_map(&provider, "task", DEFAULT_MAP_TOKEN_BUDGET).unwrap();
        assert!(map.degraded);
        assert!(map.recent_decisions.is_empty());
        assert!(map.validated_procedures.is_empty());
    }

    #[test]
    fn map_is_bounded_to_token_budget() {
        let provider = StaticMemoryProvider {
            entries: vec![
                entry(
                    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    MemoryKind::Fact,
                    0.9,
                    "validated",
                ),
                entry(
                    "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                    MemoryKind::Warning,
                    0.5,
                    "validated",
                ),
            ],
            available: true,
        };
        let map = generate_map(&provider, "task", 10).unwrap();
        // Both entries individually exceed a 10-token budget, so neither lands.
        assert!(map.recent_decisions.is_empty());
        assert!(map.warnings.is_empty());
    }

    #[test]
    fn map_buckets_by_kind_and_is_not_degraded_when_available() {
        let provider = StaticMemoryProvider {
            entries: vec![
                entry("decision one", MemoryKind::Decision, 0.9, "validated"),
                entry("procedure two", MemoryKind::Procedure, 0.8, "validated"),
                entry("warning three", MemoryKind::Warning, 0.4, "candidate"),
            ],
            available: true,
        };
        let map = generate_map(&provider, "task", DEFAULT_MAP_TOKEN_BUDGET).unwrap();
        assert!(!map.degraded);
        assert_eq!(map.recent_decisions.len(), 1);
        assert_eq!(map.validated_procedures.len(), 1);
        assert_eq!(map.warnings.len(), 1);
    }

    #[test]
    fn validated_candidate_wins_over_unvalidated_current() {
        let current = entry("old", MemoryKind::Fact, 0.9, "candidate");
        let candidate = entry("new", MemoryKind::Fact, 0.95, "validated");
        let (preferred, conflict) = resolve_conflict(&current, &candidate);
        assert!(conflict);
        assert_eq!(preferred.summary, "new");
    }

    #[test]
    fn superseded_never_wins() {
        let current = entry("stable", MemoryKind::Procedure, 0.8, "validated");
        let candidate = entry("obsolete", MemoryKind::Superseded, 0.99, "validated");
        let (preferred, _) = resolve_conflict(&current, &candidate);
        assert_eq!(preferred.summary, "stable");
    }
}
