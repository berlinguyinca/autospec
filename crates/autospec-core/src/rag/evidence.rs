//! The Evidence object (spec sections 10 and 11).
//!
//! Every fact that reaches an agent is one of these. Evidence is immutable once
//! captured: the retrieval that produced it is a point-in-time observation of a
//! specific revision, and rewriting it later would make the trace lie about
//! what the agent actually saw.

use crate::rag::authority::SourceAuthority;
use crate::rag::score::Score;
use crate::rag::scope::RetrievalScope;
use crate::rag::source::SourceKind;

/// Where in a source an evidence item was read from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceLocation {
    /// Canonical URI, e.g. `github://autospec/crates/core/src/router.rs`.
    pub uri: String,
    /// Symbol the evidence covers, when the source is code.
    pub symbol: Option<String>,
    /// Inclusive 1-based start line.
    pub line_start: Option<u32>,
    /// Inclusive 1-based end line.
    pub line_end: Option<u32>,
}

impl SourceLocation {
    /// A whole-document location.
    pub fn document(uri: impl Into<String>) -> Self {
        Self {
            uri: uri.into(),
            symbol: None,
            line_start: None,
            line_end: None,
        }
    }

    /// A line range within a document.
    pub fn lines(uri: impl Into<String>, start: u32, end: u32) -> Result<Self, String> {
        if start == 0 {
            return Err("line numbers are 1-based".to_string());
        }
        if end < start {
            return Err(format!("line range ends before it starts: {start}..{end}"));
        }
        Ok(Self {
            uri: uri.into(),
            symbol: None,
            line_start: Some(start),
            line_end: Some(end),
        })
    }

    /// Attach the symbol this location covers.
    pub fn with_symbol(mut self, symbol: impl Into<String>) -> Self {
        self.symbol = Some(symbol.into());
        self
    }

    /// Human-readable citation, e.g. `path/router.rs:120-197`.
    pub fn citation(&self) -> String {
        match (self.line_start, self.line_end) {
            (Some(start), Some(end)) if start == end => format!("{}:{start}", self.uri),
            (Some(start), Some(end)) => format!("{}:{start}-{end}", self.uri),
            _ => self.uri.clone(),
        }
    }
}

/// How the content reached the agent, tracked so a summary is never mistaken
/// for a quotation (spec section 11).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentForm {
    /// Verbatim source text.
    Raw,
    /// Deterministically extracted (a line range, a symbol body).
    Extracted,
    /// Summarized by a model. Downstream claims cannot be traced to a line.
    ModelSummarized,
}

impl ContentForm {
    /// Stable wire identifier.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Raw => "raw",
            Self::Extracted => "extracted",
            Self::ModelSummarized => "model_summarized",
        }
    }

    /// Return `true` when a model rewrote the content.
    pub const fn is_model_transformed(self) -> bool {
        matches!(self, Self::ModelSummarized)
    }
}

/// Privacy classification, inherited by derived summaries (spec section 53).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Privacy {
    /// Safe to reuse across projects.
    Public,
    /// Confined to the project it came from.
    Internal,
    /// Confined to the originating repository and its authorized readers.
    Private,
}

impl Privacy {
    /// Stable wire identifier.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Internal => "internal",
            Self::Private => "private",
        }
    }

    /// Strictest of two classifications.
    pub fn strictest(self, other: Self) -> Self {
        if self >= other {
            self
        } else {
            other
        }
    }
}

/// A typed edge from this evidence to another graph node (spec section 14).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceRelationship {
    /// Relation name, e.g. `implements`.
    pub relation: String,
    /// Target node identifier.
    pub target: String,
}

/// The query that produced this evidence, original and rewritten.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryProvenance {
    /// The caller's question as asked.
    pub original: String,
    /// The query actually issued to the source, when rewritten.
    pub rewritten: Option<String>,
    /// Which loop iteration issued it, 1-based.
    pub iteration: u32,
}

/// The retrieval-wide facts every item captured in one loop shares: the source
/// state, the query that found it, when it was read, and which role it was read
/// for.
///
/// Grouped rather than passed item by item because they are identical across a
/// query's whole result set, and repeating them per item invites the four to
/// drift apart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceCapture {
    /// Source state the evidence was read at.
    pub scope: RetrievalScope,
    /// The query, original and rewritten.
    pub query: QueryProvenance,
    /// Retrieval time, seconds since the Unix epoch.
    pub retrieved_at: u64,
    /// Role the retrieval served.
    pub agent_role: String,
}

impl EvidenceCapture {
    /// Build a capture context.
    pub fn new(
        scope: RetrievalScope,
        query: QueryProvenance,
        retrieved_at: u64,
        agent_role: impl Into<String>,
    ) -> Self {
        Self {
            scope,
            query,
            retrieved_at,
            agent_role: agent_role.into(),
        }
    }
}

/// A single retrieved fact with complete provenance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Evidence {
    id: String,
    source_kind: SourceKind,
    location: SourceLocation,
    scope: RetrievalScope,
    authority: SourceAuthority,
    privacy: Privacy,
    form: ContentForm,
    content: String,
    content_hash: String,
    confidence: Score,
    relevance: Score,
    freshness: Score,
    retrieved_at: u64,
    source_timestamp: Option<u64>,
    relationships: Vec<EvidenceRelationship>,
    query: QueryProvenance,
    derived_from: Vec<String>,
    agent_role: String,
}

impl Evidence {
    /// Evidence identifier, unique within a retrieval.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Which adapter produced it.
    pub fn source_kind(&self) -> SourceKind {
        self.source_kind
    }

    /// Where it was read from.
    pub fn location(&self) -> &SourceLocation {
        &self.location
    }

    /// The repository/revision/worktree state it was read at.
    pub fn scope(&self) -> &RetrievalScope {
        &self.scope
    }

    /// Authority class of the source.
    pub fn authority(&self) -> SourceAuthority {
        self.authority
    }

    /// Privacy classification.
    pub fn privacy(&self) -> Privacy {
        self.privacy
    }

    /// Whether the content is raw, extracted, or model-summarized.
    pub fn form(&self) -> ContentForm {
        self.form
    }

    /// The retrieved text.
    pub fn content(&self) -> &str {
        &self.content
    }

    /// Stable `sha256:`-prefixed hash of the content.
    pub fn content_hash(&self) -> &str {
        &self.content_hash
    }

    /// Confidence that the content is what it claims to be.
    pub fn confidence(&self) -> Score {
        self.confidence
    }

    /// Assessed relevance to the asking task.
    pub fn relevance(&self) -> Score {
        self.relevance
    }

    /// Assessed freshness relative to the source's staleness policy.
    pub fn freshness(&self) -> Score {
        self.freshness
    }

    /// Retrieval timestamp, seconds since the Unix epoch.
    pub fn retrieved_at(&self) -> u64 {
        self.retrieved_at
    }

    /// Source's own last-modified timestamp, when the adapter knows it.
    pub fn source_timestamp(&self) -> Option<u64> {
        self.source_timestamp
    }

    /// Typed graph edges asserted by this evidence.
    pub fn relationships(&self) -> &[EvidenceRelationship] {
        &self.relationships
    }

    /// The query, original and rewritten, that produced it.
    pub fn query(&self) -> &QueryProvenance {
        &self.query
    }

    /// Evidence ids this item was derived from; empty for directly retrieved
    /// evidence, populated for summaries (spec section 11).
    pub fn derived_from(&self) -> &[String] {
        &self.derived_from
    }

    /// Role of the agent the retrieval was performed for.
    pub fn agent_role(&self) -> &str {
        &self.agent_role
    }

    /// A one-line citation naming the source, revision and lines.
    pub fn citation(&self) -> String {
        let state = match self.scope.worktree_id() {
            Some(worktree) => format!("{}@{}", worktree, self.scope.revision()),
            None => self.scope.revision().to_string(),
        };
        format!(
            "{} [{}] {} ({})",
            self.location.citation(),
            state,
            self.authority.as_str(),
            self.form.as_str()
        )
    }

    /// Return `true` when this evidence carries the same content, from the same
    /// place, at the same source state, as `other`.
    ///
    /// Two adapters that surface the same file are a duplicate; the same file
    /// at two revisions is not (spec section 12).
    pub fn duplicates(&self, other: &Self) -> bool {
        self.content_hash == other.content_hash
            && self.location.uri == other.location.uri
            && self.scope.cache_fragment() == other.scope.cache_fragment()
    }
}

/// Builder for [`Evidence`], which has no public constructor because a
/// half-populated provenance record is worse than none.
#[derive(Debug, Clone)]
pub struct EvidenceBuilder {
    id: String,
    source_kind: SourceKind,
    location: SourceLocation,
    scope: RetrievalScope,
    authority: SourceAuthority,
    privacy: Privacy,
    form: ContentForm,
    content: String,
    confidence: Score,
    relevance: Score,
    freshness: Score,
    retrieved_at: u64,
    source_timestamp: Option<u64>,
    relationships: Vec<EvidenceRelationship>,
    query: QueryProvenance,
    derived_from: Vec<String>,
    agent_role: String,
}

impl EvidenceBuilder {
    /// Start a builder with the fields no evidence item can omit.
    pub fn new(
        id: impl Into<String>,
        source_kind: SourceKind,
        location: SourceLocation,
        authority: SourceAuthority,
        content: impl Into<String>,
        capture: &EvidenceCapture,
    ) -> Self {
        Self {
            id: id.into(),
            source_kind,
            location,
            scope: capture.scope.clone(),
            authority,
            privacy: Privacy::Private,
            form: ContentForm::Raw,
            content: content.into(),
            confidence: Score::ONE,
            relevance: Score::ZERO,
            freshness: Score::ONE,
            retrieved_at: capture.retrieved_at,
            source_timestamp: None,
            relationships: Vec::new(),
            query: capture.query.clone(),
            derived_from: Vec::new(),
            agent_role: capture.agent_role.clone(),
        }
    }

    /// Set the privacy classification. Defaults to [`Privacy::Private`], the
    /// safe assumption when an adapter does not say.
    pub fn privacy(mut self, privacy: Privacy) -> Self {
        self.privacy = privacy;
        self
    }

    /// Set the content form.
    pub fn form(mut self, form: ContentForm) -> Self {
        self.form = form;
        self
    }

    /// Set confidence, relevance and freshness.
    pub fn scores(mut self, confidence: Score, relevance: Score, freshness: Score) -> Self {
        self.confidence = confidence;
        self.relevance = relevance;
        self.freshness = freshness;
        self
    }

    /// Set the source's own last-modified timestamp.
    pub fn source_timestamp(mut self, timestamp: u64) -> Self {
        self.source_timestamp = Some(timestamp);
        self
    }

    /// Add a typed graph edge.
    pub fn relationship(mut self, relation: impl Into<String>, target: impl Into<String>) -> Self {
        self.relationships.push(EvidenceRelationship {
            relation: relation.into(),
            target: target.into(),
        });
        self
    }

    /// Record the evidence ids this item was derived from.
    pub fn derived_from(mut self, ids: impl IntoIterator<Item = String>) -> Self {
        self.derived_from = ids.into_iter().collect();
        self
    }

    /// Record the role the retrieval served.
    pub fn agent_role(mut self, role: impl Into<String>) -> Self {
        self.agent_role = role.into();
        self
    }

    /// Finish the evidence item, computing its content hash.
    ///
    /// A model-summarized item must name the evidence it came from: an
    /// unattributed summary is exactly the untraceable claim section 11 exists
    /// to prevent.
    pub fn build(self) -> Result<Evidence, String> {
        if self.id.trim().is_empty() {
            return Err("evidence id must not be empty".to_string());
        }
        if self.form.is_model_transformed() && self.derived_from.is_empty() {
            return Err(format!(
                "model-summarized evidence {} must cite the evidence it was derived from",
                self.id
            ));
        }
        let content_hash = content_hash(&self.content);
        Ok(Evidence {
            id: self.id,
            source_kind: self.source_kind,
            location: self.location,
            scope: self.scope,
            authority: self.authority,
            privacy: self.privacy,
            form: self.form,
            content: self.content,
            content_hash,
            confidence: self.confidence,
            relevance: self.relevance,
            freshness: self.freshness,
            retrieved_at: self.retrieved_at,
            source_timestamp: self.source_timestamp,
            relationships: self.relationships,
            query: self.query,
            derived_from: self.derived_from,
            agent_role: self.agent_role,
        })
    }
}

/// Compute the `sha256:`-prefixed content hash used for deduplication and
/// revision-aware cache keys.
pub fn content_hash(content: &str) -> String {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(7 + digest.len() * 2);
    hex.push_str("sha256:");
    for byte in digest {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex
}

/// Derive the privacy of a summary from the evidence it was built on.
///
/// Section 53: a derived summary inherits the strictest classification of its
/// sources. A summary built from nothing is private, because "no sources" is
/// not evidence that the content is safe to share.
pub fn inherited_privacy(sources: &[Evidence]) -> Privacy {
    if sources.is_empty() {
        return Privacy::Private;
    }
    sources
        .iter()
        .map(Evidence::privacy)
        .fold(Privacy::Public, Privacy::strictest)
}
