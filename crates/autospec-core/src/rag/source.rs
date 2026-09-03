//! Knowledge sources and the adapter interface (spec sections 8 and 50).
//!
//! The trait is synchronous because the AutoSpec Rust core carries no async
//! runtime; adapters that need I/O run it on the caller's thread. Section 50
//! notes its Rust sketch is illustrative and the implementation follows the
//! existing stack.

use std::collections::BTreeMap;

use crate::rag::authority::SourceAuthority;
use crate::rag::evidence::{Evidence, Privacy};
use crate::rag::scope::RetrievalScope;

/// The source families a retrieval can draw on (spec section 8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SourceKind {
    /// Repository text, symbols, references, history.
    Repository,
    /// Issues, pull requests, reviews, project boards.
    GitHub,
    /// AutoSpec specifications and implementation plans.
    Specification,
    /// Architectural decision records.
    Adr,
    /// Observational memory.
    Memory,
    /// Tests, CI results, regression history.
    Test,
    /// Runtime telemetry.
    Runtime,
    /// Local and generated documentation.
    Documentation,
    /// External web retrieval; policy-gated.
    Web,
}

/// Every source kind, in a stable order.
pub const ALL_SOURCE_KINDS: [SourceKind; 9] = [
    SourceKind::Repository,
    SourceKind::GitHub,
    SourceKind::Specification,
    SourceKind::Adr,
    SourceKind::Memory,
    SourceKind::Test,
    SourceKind::Runtime,
    SourceKind::Documentation,
    SourceKind::Web,
];

impl SourceKind {
    /// Stable wire identifier.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Repository => "repository",
            Self::GitHub => "github",
            Self::Specification => "specification",
            Self::Adr => "adr",
            Self::Memory => "memory",
            Self::Test => "test",
            Self::Runtime => "runtime",
            Self::Documentation => "documentation",
            Self::Web => "web",
        }
    }

    /// Parse a wire identifier.
    pub fn parse(text: &str) -> Result<Self, String> {
        ALL_SOURCE_KINDS
            .iter()
            .copied()
            .find(|kind| kind.as_str() == text)
            .ok_or_else(|| format!("unknown source kind: {text}"))
    }

    /// The authority class evidence from this source defaults to.
    ///
    /// An adapter may override per item — a superseded ADR is
    /// `HistoricalImplementation`, not `CurrentAdr` — but the default keeps an
    /// adapter that says nothing from landing at the top of the ladder.
    pub const fn default_authority(self) -> SourceAuthority {
        match self {
            Self::Repository => SourceAuthority::Implementation,
            Self::GitHub => SourceAuthority::Discussion,
            Self::Specification => SourceAuthority::AcceptedSpecification,
            Self::Adr => SourceAuthority::CurrentAdr,
            Self::Memory => SourceAuthority::ProjectMemory,
            Self::Test => SourceAuthority::CurrentTests,
            Self::Runtime => SourceAuthority::Implementation,
            Self::Documentation => SourceAuthority::OfficialDocumentation,
            Self::Web => SourceAuthority::ExternalCommunity,
        }
    }

    /// Return `true` when reaching this source leaves the local installation.
    ///
    /// External calls have their own budget line (section 39) because they cost
    /// latency and quota that local index reads do not.
    pub const fn is_external(self) -> bool {
        matches!(self, Self::GitHub | Self::Web)
    }
}

/// What a caller asks one source for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchRequest {
    /// The query text or symbol.
    pub query: String,
    /// The shape of the lookup, which lets an adapter dispatch without parsing
    /// the query string.
    pub mode: SearchMode,
    /// Source state to read.
    pub scope: RetrievalScope,
    /// Maximum items the caller will accept.
    pub limit: u32,
    /// Adapter-specific filters, ordered for reproducible cache keys.
    pub filters: BTreeMap<String, String>,
}

impl SearchRequest {
    /// Build a request with no filters.
    pub fn new(
        query: impl Into<String>,
        mode: SearchMode,
        scope: RetrievalScope,
        limit: u32,
    ) -> Self {
        Self {
            query: query.into(),
            mode,
            scope,
            limit,
            filters: BTreeMap::new(),
        }
    }

    /// Add an adapter-specific filter.
    pub fn filter(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.filters.insert(key.into(), value.into());
        self
    }

    /// Deterministic cache key fragment for this request.
    pub fn cache_fragment(&self) -> String {
        let filters = self
            .filters
            .iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>()
            .join("&");
        format!(
            "{}|{}|{}|{}|{}",
            self.mode.as_str(),
            self.query,
            self.limit,
            self.scope.cache_fragment(),
            filters
        )
    }
}

/// The shape of a lookup (spec section 13's rewrite targets).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SearchMode {
    /// Natural-language semantic search.
    Semantic,
    /// Literal text match.
    Literal,
    /// Locate a named symbol's definition.
    SymbolDefinition,
    /// Find call sites of a symbol.
    SymbolReferences,
    /// Find implementations of an interface.
    Implementations,
    /// Find tests covering a symbol or behavior.
    Tests,
    /// Look up a document by identifier.
    DocumentLookup,
    /// Traverse the knowledge graph.
    GraphTraversal,
}

impl SearchMode {
    /// Stable wire identifier.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Semantic => "semantic",
            Self::Literal => "literal",
            Self::SymbolDefinition => "symbol_definition",
            Self::SymbolReferences => "symbol_references",
            Self::Implementations => "implementations",
            Self::Tests => "tests",
            Self::DocumentLookup => "document_lookup",
            Self::GraphTraversal => "graph_traversal",
        }
    }
}

/// What a source returns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchResult {
    /// Evidence found, adapter-ranked.
    pub evidence: Vec<Evidence>,
    /// `true` when the source had more to give than `limit` allowed.
    pub truncated: bool,
    /// Diagnostic the trace records when a source declined or degraded.
    pub note: Option<String>,
}

impl SearchResult {
    /// A complete result.
    pub fn found(evidence: Vec<Evidence>) -> Self {
        Self {
            evidence,
            truncated: false,
            note: None,
        }
    }

    /// A result the source truncated.
    pub fn truncated(evidence: Vec<Evidence>) -> Self {
        Self {
            evidence,
            truncated: true,
            note: None,
        }
    }

    /// An empty result with an explanation.
    pub fn empty(note: impl Into<String>) -> Self {
        Self {
            evidence: Vec::new(),
            truncated: false,
            note: Some(note.into()),
        }
    }
}

/// A retrievable knowledge source (spec section 50).
pub trait KnowledgeSource {
    /// Which family this adapter serves.
    fn kind(&self) -> SourceKind;

    /// Modes this adapter can answer. The planner will not route a mode the
    /// adapter does not declare, so a hallucinated query shape fails at
    /// planning rather than producing an empty result the loop then retries
    /// (spec section 41.2).
    fn supported_modes(&self) -> &[SearchMode];

    /// Strictest privacy classification content from this source can carry.
    fn privacy_ceiling(&self) -> Privacy {
        Privacy::Private
    }

    /// Execute a search.
    fn search(&self, request: &SearchRequest) -> Result<SearchResult, String>;
}

/// The set of sources a retrieval may use, and the modes they answer.
#[derive(Default)]
pub struct SourceRegistry {
    sources: Vec<Box<dyn KnowledgeSource>>,
}

impl SourceRegistry {
    /// An empty registry.
    pub fn new() -> Self {
        Self {
            sources: Vec::new(),
        }
    }

    /// Register an adapter.
    ///
    /// One adapter per source kind: two adapters answering `repository` would
    /// make evidence ordering depend on registration order.
    pub fn register(&mut self, source: Box<dyn KnowledgeSource>) -> Result<(), String> {
        let kind = source.kind();
        if self.contains(kind) {
            return Err(format!(
                "source kind {} is already registered",
                kind.as_str()
            ));
        }
        self.sources.push(source);
        Ok(())
    }

    /// Return `true` when a kind has an adapter.
    pub fn contains(&self, kind: SourceKind) -> bool {
        self.sources.iter().any(|source| source.kind() == kind)
    }

    /// Look up the adapter for a kind.
    pub fn get(&self, kind: SourceKind) -> Option<&dyn KnowledgeSource> {
        self.sources
            .iter()
            .find(|source| source.kind() == kind)
            .map(|source| source.as_ref())
    }

    /// Registered kinds, in a stable order.
    pub fn kinds(&self) -> Vec<SourceKind> {
        let mut kinds = self
            .sources
            .iter()
            .map(|source| source.kind())
            .collect::<Vec<_>>();
        kinds.sort();
        kinds
    }

    /// Kinds that can answer `mode`, in a stable order.
    pub fn kinds_supporting(&self, mode: SearchMode) -> Vec<SourceKind> {
        let mut kinds = self
            .sources
            .iter()
            .filter(|source| source.supported_modes().contains(&mode))
            .map(|source| source.kind())
            .collect::<Vec<_>>();
        kinds.sort();
        kinds
    }

    /// Execute a request against one source.
    pub fn search(
        &self,
        kind: SourceKind,
        request: &SearchRequest,
    ) -> Result<SearchResult, String> {
        let source = self
            .get(kind)
            .ok_or_else(|| format!("no adapter registered for source {}", kind.as_str()))?;
        if !source.supported_modes().contains(&request.mode) {
            return Err(format!(
                "source {} does not support mode {}",
                kind.as_str(),
                request.mode.as_str()
            ));
        }
        source.search(request)
    }
}

impl std::fmt::Debug for SourceRegistry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SourceRegistry")
            .field("kinds", &self.kinds())
            .finish()
    }
}
