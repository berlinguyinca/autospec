//! Shared fixtures for the Agentic RAG evaluation suites (spec section 55).
//!
//! The fake sources are deterministic and hold no state beyond what a test
//! hands them, so the suites can run in any order and in the same process
//! without interfering with each other.

#![allow(dead_code)]

use autospec_core::rag::authority::SourceAuthority;
use autospec_core::rag::evidence::{Evidence, EvidenceBuilder, EvidenceCapture, Privacy, QueryProvenance, SourceLocation};
use autospec_core::rag::score::Score;
use autospec_core::rag::scope::RetrievalScope;
use autospec_core::rag::source::{
    KnowledgeSource, SearchMode, SearchRequest, SearchResult, SourceKind,
};

/// Fixed retrieval timestamp used across the suites.
pub const NOW: u64 = 1_756_800_000;

/// Fixed base revision used across the suites.
pub const REVISION: &str = "9a223af";

/// A committed scope at [`REVISION`].
pub fn scope() -> RetrievalScope {
    RetrievalScope::committed("autospec", REVISION)
}

/// A capture context for a role.
pub fn capture(scope: RetrievalScope, role: &str) -> EvidenceCapture {
    EvidenceCapture::new(
        scope,
        QueryProvenance {
            original: "how does routing choose a node?".to_string(),
            rewritten: None,
            iteration: 1,
        },
        NOW,
        role,
    )
}

/// Build an evidence item with the given relevance.
pub fn evidence(
    id: &str,
    kind: SourceKind,
    uri: &str,
    content: &str,
    authority: SourceAuthority,
    relevance: u16,
) -> Evidence {
    evidence_in(id, kind, uri, content, authority, relevance, scope())
}

/// Build an evidence item captured at a specific scope.
pub fn evidence_in(
    id: &str,
    kind: SourceKind,
    uri: &str,
    content: &str,
    authority: SourceAuthority,
    relevance: u16,
    scope: RetrievalScope,
) -> Evidence {
    let capture = capture(scope, "planner");
    // Distinct ids get distinct line ranges: two items quoting the same file at
    // the same lines are the same fact, and the evaluator is right to collapse
    // them, so a fixture that reused one range would test deduplication rather
    // than whatever the calling test meant to exercise.
    let start = line_start_for(id);
    EvidenceBuilder::new(
        id,
        kind,
        SourceLocation::lines(uri, start, start + 19).expect("valid line range"),
        authority,
        content,
        &capture,
    )
    .privacy(Privacy::Internal)
    .scores(
        Score::from_permille(950),
        Score::from_permille(relevance),
        Score::ONE,
    )
    .build()
    .expect("valid evidence")
}

/// Deterministic, distinct line start for an evidence id.
fn line_start_for(id: &str) -> u32 {
    let sum: u32 = id.bytes().map(u32::from).sum();
    sum * 20 + 1
}

/// A source that answers with a fixed set of evidence for every request.
pub struct StaticSource {
    kind: SourceKind,
    modes: Vec<SearchMode>,
    evidence: Vec<Evidence>,
}

impl StaticSource {
    /// A source answering every listed mode with `evidence`.
    pub fn new(kind: SourceKind, modes: Vec<SearchMode>, evidence: Vec<Evidence>) -> Self {
        Self {
            kind,
            modes,
            evidence,
        }
    }

    /// A semantic-only source.
    pub fn semantic(kind: SourceKind, evidence: Vec<Evidence>) -> Self {
        Self::new(kind, vec![SearchMode::Semantic], evidence)
    }
}

impl KnowledgeSource for StaticSource {
    fn kind(&self) -> SourceKind {
        self.kind
    }

    fn supported_modes(&self) -> &[SearchMode] {
        &self.modes
    }

    fn search(&self, request: &SearchRequest) -> Result<SearchResult, String> {
        let limit = request.limit as usize;
        if self.evidence.len() > limit {
            return Ok(SearchResult::truncated(
                self.evidence[..limit].to_vec(),
            ));
        }
        Ok(SearchResult::found(self.evidence.clone()))
    }
}

/// A source that always fails, for degradation tests.
pub struct FailingSource {
    kind: SourceKind,
    modes: Vec<SearchMode>,
}

impl FailingSource {
    /// A source of `kind` that fails every semantic search.
    pub fn new(kind: SourceKind) -> Self {
        Self {
            kind,
            modes: vec![SearchMode::Semantic],
        }
    }
}

impl KnowledgeSource for FailingSource {
    fn kind(&self) -> SourceKind {
        self.kind
    }

    fn supported_modes(&self) -> &[SearchMode] {
        &self.modes
    }

    fn search(&self, _request: &SearchRequest) -> Result<SearchResult, String> {
        Err("source unavailable".to_string())
    }
}

/// A source that counts how many times it was queried.
pub struct CountingSource {
    kind: SourceKind,
    modes: Vec<SearchMode>,
    evidence: Vec<Evidence>,
    calls: std::cell::Cell<u32>,
}

impl CountingSource {
    /// A counting semantic source.
    pub fn new(kind: SourceKind, evidence: Vec<Evidence>) -> Self {
        Self {
            kind,
            modes: vec![SearchMode::Semantic],
            evidence,
            calls: std::cell::Cell::new(0),
        }
    }

    /// Queries received so far.
    pub fn calls(&self) -> u32 {
        self.calls.get()
    }
}

impl KnowledgeSource for CountingSource {
    fn kind(&self) -> SourceKind {
        self.kind
    }

    fn supported_modes(&self) -> &[SearchMode] {
        &self.modes
    }

    fn search(&self, _request: &SearchRequest) -> Result<SearchResult, String> {
        self.calls.set(self.calls.get() + 1);
        Ok(SearchResult::found(self.evidence.clone()))
    }
}
