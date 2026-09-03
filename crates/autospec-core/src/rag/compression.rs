//! Hierarchical context compression (spec section 19).
//!
//! The six levels are a ladder from raw source to an architecture overview.
//! Section 19's operative sentence is that compression preserves source
//! references, so a compressed item is still an [`Evidence`] with the same
//! location and the same `derived_from` chain — the caller can always get back
//! to the lines.

use crate::rag::evidence::{
    ContentForm, Evidence, EvidenceBuilder, EvidenceCapture, inherited_privacy,
};
use crate::rag::score::Score;

/// How far evidence has been compressed (spec section 19).
///
/// Ordered least to most compressed, so `>` means "coarser".
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CompressionLevel {
    /// Verbatim retrieved content.
    Raw,
    /// One symbol's signature and behavior.
    Symbol,
    /// One file's structure and responsibilities.
    File,
    /// One module's role and public surface.
    Module,
    /// One repository's shape.
    Repository,
    /// The cross-repository architecture.
    Architecture,
}

/// Every level, least compressed first.
pub const ALL_LEVELS: [CompressionLevel; 6] = [
    CompressionLevel::Raw,
    CompressionLevel::Symbol,
    CompressionLevel::File,
    CompressionLevel::Module,
    CompressionLevel::Repository,
    CompressionLevel::Architecture,
];

impl CompressionLevel {
    /// Stable wire identifier.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Raw => "raw",
            Self::Symbol => "symbol",
            Self::File => "file",
            Self::Module => "module",
            Self::Repository => "repository",
            Self::Architecture => "architecture",
        }
    }

    /// The next coarser level, or `None` at the top.
    pub fn coarser(self) -> Option<Self> {
        let index = ALL_LEVELS.iter().position(|level| *level == self)?;
        ALL_LEVELS.get(index + 1).copied()
    }

    /// Approximate token ceiling one item at this level should occupy.
    ///
    /// Used to choose a level for a budget, not to truncate: an item that
    /// exceeds its level's ceiling is a signal to compress further, not to cut
    /// the content mid-sentence.
    pub const fn target_tokens(self) -> u32 {
        match self {
            Self::Raw => u32::MAX,
            Self::Symbol => 400,
            Self::File => 250,
            Self::Module => 180,
            Self::Repository => 120,
            Self::Architecture => 80,
        }
    }
}

/// Estimate the token cost of text.
///
/// A deterministic approximation — roughly four characters per token, with a
/// floor of one token per whitespace-separated word so a line of short
/// identifiers is not undercounted. Budgets need a number that is stable across
/// hosts more than they need a number that matches one tokenizer exactly; the
/// routing layer applies a safety margin on top (section 24).
pub fn estimate_tokens(text: &str) -> u32 {
    if text.is_empty() {
        return 0;
    }
    let characters = text.chars().count() as u32;
    let words = text.split_whitespace().count() as u32;
    characters.div_ceil(4).max(words)
}

/// Choose the coarsest level that is not needed, i.e. the finest level whose
/// estimated cost fits the budget.
///
/// Returns `None` when even the coarsest level overflows, which is the caller's
/// signal to drop items rather than compress further.
pub fn level_for_budget(items: usize, max_tokens: u32) -> Option<CompressionLevel> {
    if items == 0 {
        return Some(CompressionLevel::Raw);
    }
    let per_item = max_tokens / items as u32;
    ALL_LEVELS
        .iter()
        .copied()
        .find(|level| level.target_tokens() <= per_item)
}

/// Build a compressed summary evidence item from source evidence.
///
/// The summary keeps the first source's location so the citation still points
/// somewhere real, records every source id in `derived_from`, and inherits the
/// strictest privacy class present (section 53). Its form is
/// [`ContentForm::ModelSummarized`], which is what tells a downstream reader
/// that a claim in it cannot be traced to a specific line.
pub fn summarize(
    id: impl Into<String>,
    level: CompressionLevel,
    summary_text: impl Into<String>,
    sources: &[Evidence],
    retrieved_at: u64,
) -> Result<Evidence, String> {
    let first = sources
        .first()
        .ok_or_else(|| "a summary must be derived from at least one evidence item".to_string())?;
    let privacy = inherited_privacy(sources);
    // The summary can be no more authoritative and no fresher than its weakest
    // source: compression must not launder a stale blog post into current code.
    let authority = sources
        .iter()
        .map(Evidence::authority)
        .min()
        .unwrap_or_else(|| first.authority());
    let freshness = sources
        .iter()
        .map(Evidence::freshness)
        .min()
        .unwrap_or(Score::ZERO);
    let relevance = Score::mean(&sources.iter().map(Evidence::relevance).collect::<Vec<_>>());
    let confidence = sources
        .iter()
        .map(Evidence::confidence)
        .min()
        .unwrap_or(Score::ZERO);

    let capture = EvidenceCapture::new(
        first.scope().clone(),
        first.query().clone(),
        retrieved_at,
        first.agent_role(),
    );
    EvidenceBuilder::new(
        id,
        first.source_kind(),
        first.location().clone(),
        authority,
        summary_text,
        &capture,
    )
    .privacy(privacy)
    .form(ContentForm::ModelSummarized)
    .scores(confidence, relevance, freshness)
    .derived_from(sources.iter().map(|item| item.id().to_string()))
    .relationship("compressed_at", level.as_str())
    .build()
}
