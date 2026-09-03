//! Query planning and reformulation (spec sections 13 and 41.2).
//!
//! Reformulation here is deterministic. The subsystem may also ask a small
//! model for rewrites, but the rules below run first and for free: they cover
//! the transformations section 13 lists, and their output is reproducible,
//! which matters because a query that cannot be reproduced cannot be cached.

use crate::rag::source::{SearchMode, SourceKind};

/// One planned lookup: a query, the shape it takes, and where to send it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedQuery {
    /// The text to search for.
    pub query: String,
    /// The lookup shape.
    pub mode: SearchMode,
    /// Sources to try, highest priority first.
    pub sources: Vec<SourceKind>,
    /// Why the planner produced this query, recorded in the trace.
    pub rationale: String,
}

impl PlannedQuery {
    /// Build a planned query.
    pub fn new(
        query: impl Into<String>,
        mode: SearchMode,
        sources: Vec<SourceKind>,
        rationale: impl Into<String>,
    ) -> Self {
        Self {
            query: query.into(),
            mode,
            sources,
            rationale: rationale.into(),
        }
    }

    /// Identity used to detect a repeated query (spec section 41.1).
    pub fn signature(&self) -> String {
        format!(
            "{}::{}",
            self.mode.as_str(),
            self.query.trim().to_lowercase()
        )
    }
}

/// The deterministic reformulations of section 13.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reformulation {
    /// Natural language to a symbol lookup.
    NaturalLanguageToSymbol,
    /// A symbol to its call sites.
    SymbolToCallers,
    /// An interface to its implementations.
    SymbolToImplementations,
    /// A symbol to the tests covering it.
    SymbolToTests,
    /// A symbol to the specifications that mention it.
    SymbolToSpecifications,
    /// An error string to the source location that emits it.
    ErrorTextToSource,
    /// A specification requirement to the modules it affects.
    RequirementToModules,
    /// An issue to the pull requests referencing it.
    IssueToPullRequests,
}

impl Reformulation {
    /// Stable wire identifier.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NaturalLanguageToSymbol => "natural_language_to_symbol",
            Self::SymbolToCallers => "symbol_to_callers",
            Self::SymbolToImplementations => "symbol_to_implementations",
            Self::SymbolToTests => "symbol_to_tests",
            Self::SymbolToSpecifications => "symbol_to_specifications",
            Self::ErrorTextToSource => "error_text_to_source",
            Self::RequirementToModules => "requirement_to_modules",
            Self::IssueToPullRequests => "issue_to_pull_requests",
        }
    }
}

/// Turns a question into lookups, and turns an insufficient result into the
/// next round of lookups.
#[derive(Debug, Clone, Default)]
pub struct QueryPlanner {
    issued: Vec<String>,
}

impl QueryPlanner {
    /// A planner with no history.
    pub fn new() -> Self {
        Self::default()
    }

    /// Plan the opening queries for a question.
    ///
    /// The opening round always pairs a semantic search with any symbol-shaped
    /// term found in the question: semantic search alone reliably misses an
    /// exact identifier, and a symbol lookup alone misses the concept.
    pub fn plan_initial(&mut self, question: &str, sources: &[SourceKind]) -> Vec<PlannedQuery> {
        let mut planned = Vec::new();
        let semantic = PlannedQuery::new(
            question.trim(),
            SearchMode::Semantic,
            sources.to_vec(),
            "opening semantic search for the question as asked",
        );
        planned.push(semantic);

        for symbol in extract_symbols(question).into_iter().take(3) {
            planned.push(PlannedQuery::new(
                symbol,
                SearchMode::SymbolDefinition,
                filter_code_sources(sources),
                format!(
                    "{}: question names a symbol-shaped term",
                    Reformulation::NaturalLanguageToSymbol.as_str()
                ),
            ));
        }
        self.retain_novel(planned)
    }

    /// Plan follow-up queries after an insufficient result.
    ///
    /// `next_queries` is what the evaluator asked for (spec section 12);
    /// `known_symbols` are symbols already found, which drive the structural
    /// expansions section 13 lists.
    pub fn plan_followup(
        &mut self,
        next_queries: &[String],
        known_symbols: &[String],
        sources: &[SourceKind],
    ) -> Vec<PlannedQuery> {
        let mut planned = Vec::new();
        for request in next_queries {
            let (query, mode) = classify_request(request);
            planned.push(PlannedQuery::new(
                query,
                mode,
                sources_for_mode(mode, sources),
                "evaluator requested follow-up retrieval",
            ));
        }
        for symbol in known_symbols.iter().take(2) {
            for (mode, reformulation) in [
                (
                    SearchMode::Implementations,
                    Reformulation::SymbolToImplementations,
                ),
                (SearchMode::SymbolReferences, Reformulation::SymbolToCallers),
                (SearchMode::Tests, Reformulation::SymbolToTests),
            ] {
                planned.push(PlannedQuery::new(
                    symbol.clone(),
                    mode,
                    sources_for_mode(mode, sources),
                    format!("{}: expanding a known symbol", reformulation.as_str()),
                ));
            }
        }
        self.retain_novel(planned)
    }

    /// Return `true` when this exact query has already been issued.
    pub fn has_issued(&self, query: &PlannedQuery) -> bool {
        self.issued.contains(&query.signature())
    }

    /// Number of distinct queries issued.
    pub fn issued_count(&self) -> usize {
        self.issued.len()
    }

    /// Drop queries already issued and record the rest.
    ///
    /// Repeated-query suppression is the concrete half of section 41.1: a loop
    /// that re-asks the same question spends budget and learns nothing, and the
    /// novelty guard alone would only notice one iteration later.
    fn retain_novel(&mut self, planned: Vec<PlannedQuery>) -> Vec<PlannedQuery> {
        let mut kept = Vec::new();
        for query in planned {
            let signature = query.signature();
            if self.issued.contains(&signature) || query.query.trim().is_empty() {
                continue;
            }
            self.issued.push(signature);
            kept.push(query);
        }
        kept
    }
}

/// Extract symbol-shaped terms from free text.
///
/// A term qualifies when it looks like an identifier rather than a word:
/// dotted (`Scheduler.select`), path-separated (`Scheduler::select`),
/// snake_case, or CamelCase. A single lowercase English word never qualifies —
/// otherwise every question would generate useless symbol lookups.
pub fn extract_symbols(text: &str) -> Vec<String> {
    let mut symbols = Vec::new();
    for token in text.split(|character: char| {
        !(character.is_ascii_alphanumeric()
            || character == '_'
            || character == '.'
            || character == ':')
    }) {
        let token = token.trim_matches(|character| character == '.' || character == ':');
        if token.len() < 3 || !token.starts_with(|character: char| character.is_ascii_alphabetic())
        {
            continue;
        }
        if is_symbol_shaped(token) && !symbols.iter().any(|known| known == token) {
            symbols.push(token.to_string());
        }
    }
    symbols
}

fn is_symbol_shaped(token: &str) -> bool {
    let has_separator = token.contains('.') || token.contains("::") || token.contains('_');
    let has_inner_uppercase = token
        .chars()
        .skip(1)
        .any(|character| character.is_ascii_uppercase());
    let starts_uppercase = token.starts_with(|character: char| character.is_ascii_uppercase());
    has_separator || has_inner_uppercase || (starts_uppercase && token.len() >= 4)
}

fn classify_request(request: &str) -> (String, SearchMode) {
    let lowered = request.trim().to_lowercase();
    for (prefix, mode) in [
        ("implementations of ", SearchMode::Implementations),
        ("callers of ", SearchMode::SymbolReferences),
        ("references to ", SearchMode::SymbolReferences),
        ("tests covering ", SearchMode::Tests),
        ("tests containing ", SearchMode::Tests),
        ("symbol: ", SearchMode::SymbolDefinition),
    ] {
        if let Some(rest) = lowered.strip_prefix(prefix) {
            let start = request.len() - rest.len();
            return (request[start..].trim().to_string(), mode);
        }
    }
    (request.trim().to_string(), SearchMode::Semantic)
}

fn sources_for_mode(mode: SearchMode, sources: &[SourceKind]) -> Vec<SourceKind> {
    match mode {
        SearchMode::Implementations
        | SearchMode::SymbolDefinition
        | SearchMode::SymbolReferences => filter_code_sources(sources),
        SearchMode::Tests => sources
            .iter()
            .copied()
            .filter(|kind| matches!(kind, SourceKind::Test | SourceKind::Repository))
            .collect(),
        _ => sources.to_vec(),
    }
}

fn filter_code_sources(sources: &[SourceKind]) -> Vec<SourceKind> {
    let code = sources
        .iter()
        .copied()
        .filter(|kind| matches!(kind, SourceKind::Repository | SourceKind::Test))
        .collect::<Vec<_>>();
    if code.is_empty() {
        sources.to_vec()
    } else {
        code
    }
}
