//! Fixed top-K retrieval, the baseline Agentic RAG is measured against
//! (spec sections 56 and 57.15).
//!
//! This is the mental model the subsystem replaces: one query, take the top K
//! results by similarity, concatenate them into the prompt. It is implemented
//! here rather than described so the comparison in section 56 is between two
//! running implementations over the same sources, not between a measurement and
//! a recollection.
//!
//! It is deliberately faithful to the baseline's weaknesses. It issues exactly
//! one query, never reformulates, does not check coverage, does not
//! deduplicate, and spends its whole budget whether or not the evidence answers
//! the question.

use crate::rag::compression::estimate_tokens;
use crate::rag::evidence::Evidence;
use crate::rag::scope::RetrievalScope;
use crate::rag::source::{SearchMode, SearchRequest, SourceKind, SourceRegistry};

/// What a fixed top-K retrieval returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaselineOutcome {
    /// The chunks selected, in source-ranked order.
    pub evidence: Vec<Evidence>,
    /// Queries issued; always one.
    pub queries: u32,
    /// Tokens the concatenated chunks occupy.
    pub context_tokens: u32,
}

impl BaselineOutcome {
    /// Aspects of the question with no supporting chunk.
    ///
    /// The baseline never computes this for itself — it has no evaluator — but
    /// the benchmark needs it to compare answer quality rather than only cost.
    pub fn uncovered_aspects(&self, required: &[String]) -> Vec<String> {
        required
            .iter()
            .filter(|aspect| {
                let needle = aspect.to_lowercase();
                !self
                    .evidence
                    .iter()
                    .any(|item| item.content().to_lowercase().contains(&needle))
            })
            .cloned()
            .collect()
    }

    /// Render the chunks the way a prompt builder would: concatenated, with no
    /// provenance and no trust fence.
    pub fn render(&self) -> String {
        self.evidence
            .iter()
            .map(Evidence::content)
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

/// Retrieve a fixed number of chunks with one semantic query per source.
pub fn retrieve_top_k(
    registry: &SourceRegistry,
    sources: &[SourceKind],
    question: &str,
    scope: &RetrievalScope,
    k: u32,
) -> BaselineOutcome {
    let mut evidence = Vec::new();
    let mut queries = 0;
    for source in sources {
        if !registry.contains(*source) {
            continue;
        }
        let request = SearchRequest::new(question, SearchMode::Semantic, scope.clone(), k);
        queries += 1;
        if let Ok(result) = registry.search(*source, &request) {
            evidence.extend(result.evidence);
        }
    }
    // Rank by relevance alone — the baseline has no notion of authority, and
    // that is exactly the behavior section 12 warns about.
    evidence.sort_by_key(|item| std::cmp::Reverse(item.relevance()));
    evidence.truncate(k as usize);
    let context_tokens = evidence
        .iter()
        .map(|item| estimate_tokens(item.content()))
        .sum();
    BaselineOutcome {
        evidence,
        queries,
        context_tokens,
    }
}
