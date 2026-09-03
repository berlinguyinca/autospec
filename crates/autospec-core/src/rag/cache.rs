//! Revision-aware retrieval caching (spec sections 25, 41.4, 47).
//!
//! Section 25's rule is absolute and is the reason this cache exists as a type
//! rather than a `BTreeMap`: an answer derived from commit `abc123` must never
//! be served as current evidence for commit `def456`. The key therefore embeds
//! the source state, and a worktree's dirty set is part of that state, so an
//! uncommitted edit invalidates its own cache entries.

use std::collections::BTreeMap;

use crate::rag::evidence::Evidence;
use crate::rag::scope::RetrievalScope;
use crate::rag::source::{SearchRequest, SourceKind};

/// What a cache entry holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CacheClass {
    /// A source query's results.
    Query,
    /// A symbol lookup.
    Symbol,
    /// A graph traversal.
    GraphTraversal,
    /// A file-level summary.
    FileSummary,
    /// A module-level summary.
    ModuleSummary,
    /// An evidence evaluation.
    Evaluation,
    /// A retrieval-side model response.
    ModelResponse,
}

impl CacheClass {
    /// Stable wire identifier.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Query => "query",
            Self::Symbol => "symbol",
            Self::GraphTraversal => "graph_traversal",
            Self::FileSummary => "file_summary",
            Self::ModuleSummary => "module_summary",
            Self::Evaluation => "evaluation",
            Self::ModelResponse => "model_response",
        }
    }
}

/// A cache key that always carries the source state it was computed at.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct CacheKey {
    class: CacheClass,
    source: SourceKind,
    scope_fragment: String,
    request_fragment: String,
}

impl CacheKey {
    /// Key a source query.
    pub fn for_request(class: CacheClass, source: SourceKind, request: &SearchRequest) -> Self {
        Self {
            class,
            source,
            scope_fragment: request.scope.cache_fragment(),
            request_fragment: request.cache_fragment(),
        }
    }

    /// Key a derived artifact — a summary, traversal, or model response — that
    /// is not a source query.
    pub fn derived(
        class: CacheClass,
        source: SourceKind,
        scope: &RetrievalScope,
        discriminator: impl Into<String>,
    ) -> Self {
        Self {
            class,
            source,
            scope_fragment: scope.cache_fragment(),
            request_fragment: discriminator.into(),
        }
    }

    /// The class of entry this key addresses.
    pub fn class(&self) -> CacheClass {
        self.class
    }

    /// Flat string form, used by external cache backends.
    pub fn as_string(&self) -> String {
        format!(
            "{}/{}/{}/{}",
            self.class.as_str(),
            self.source.as_str(),
            self.scope_fragment,
            self.request_fragment
        )
    }

    /// Return `true` when this key was computed at `scope`.
    pub fn matches_scope(&self, scope: &RetrievalScope) -> bool {
        self.scope_fragment == scope.cache_fragment()
    }
}

/// A cached set of evidence and the state it was computed at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheEntry {
    /// The cached evidence.
    pub evidence: Vec<Evidence>,
    /// The scope the entry was computed at.
    pub scope: RetrievalScope,
    /// When it was stored, seconds since the Unix epoch.
    pub stored_at: u64,
}

/// Hit and miss counters for the cache dashboard (spec section 36.5).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CacheStats {
    /// Lookups served from the cache.
    pub hits: u64,
    /// Lookups that had to go to the source.
    pub misses: u64,
    /// Entries evicted by an invalidation.
    pub invalidations: u64,
    /// Lookups rejected because the entry's scope no longer matched.
    pub scope_rejections: u64,
    /// Estimated evidence items the cache avoided re-retrieving.
    pub items_saved: u64,
}

impl CacheStats {
    /// Hit ratio in permille, avoiding floating point.
    pub fn hit_ratio_permille(&self) -> u16 {
        let total = self.hits + self.misses;
        if total == 0 {
            return 0;
        }
        ((self.hits * 1000 + total / 2) / total) as u16
    }
}

/// An in-memory, revision-aware retrieval cache.
///
/// A local proxy or a distributed backend (sections 26 and 27) implements the
/// same key discipline; this type is the reference behavior and the one the
/// tests pin.
#[derive(Debug, Clone, Default)]
pub struct RetrievalCache {
    entries: BTreeMap<String, CacheEntry>,
    stats: CacheStats,
}

impl RetrievalCache {
    /// An empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// Store evidence under a key.
    ///
    /// Worktree-scoped evidence is not stored at all: section 47 forbids
    /// uncommitted source from one worktree reaching another, and a shared
    /// cache is exactly the leak path. Returns `false` when the entry was
    /// refused.
    pub fn store(&mut self, key: &CacheKey, entry: CacheEntry) -> bool {
        if entry.scope.worktree_id().is_some() {
            return false;
        }
        self.entries.insert(key.as_string(), entry);
        true
    }

    /// Look up evidence valid for `scope`.
    ///
    /// A stored entry whose scope no longer matches the caller's is a miss, not
    /// a hit: this is where section 41.4's stale-cache mitigation actually
    /// takes effect.
    pub fn get(&mut self, key: &CacheKey, scope: &RetrievalScope) -> Option<Vec<Evidence>> {
        let Some(entry) = self.entries.get(&key.as_string()) else {
            self.stats.misses += 1;
            return None;
        };
        if !entry.scope.may_share_with(scope) {
            self.stats.misses += 1;
            self.stats.scope_rejections += 1;
            return None;
        }
        self.stats.hits += 1;
        self.stats.items_saved += entry.evidence.len() as u64;
        Some(entry.evidence.clone())
    }

    /// Drop every entry computed at a revision, for use as a commit hook.
    ///
    /// Returns the number of entries dropped.
    pub fn invalidate_revision(&mut self, repository: &str, revision: &str) -> usize {
        let prefix = format!("{repository}@{revision}");
        let doomed = self
            .entries
            .iter()
            .filter(|(_, entry)| entry.scope.cache_fragment().starts_with(&prefix))
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        for key in &doomed {
            self.entries.remove(key);
        }
        self.stats.invalidations += doomed.len() as u64;
        doomed.len()
    }

    /// Drop every entry for a repository.
    pub fn invalidate_repository(&mut self, repository: &str) -> usize {
        let prefix = format!("{repository}@");
        let doomed = self
            .entries
            .iter()
            .filter(|(_, entry)| entry.scope.cache_fragment().starts_with(&prefix))
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        for key in &doomed {
            self.entries.remove(key);
        }
        self.stats.invalidations += doomed.len() as u64;
        doomed.len()
    }

    /// Number of stored entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Return `true` when nothing is stored.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Hit and miss counters.
    pub fn stats(&self) -> CacheStats {
        self.stats
    }
}
