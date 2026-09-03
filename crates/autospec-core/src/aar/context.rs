//! Context minimization and cache-friendly prompt assembly (AAR spec sections
//! 6 and 11).
//!
//! Model context is working memory, not repository memory. Retrieval starts
//! narrow and widens only when a round produced no usable evidence, and the
//! assembled prompt puts every stable segment before the cache boundary so a
//! provider's prefix cache can hold it across steps.

use sha2::{Digest, Sha256};

use super::classify::{Complexity, TaskClass, TaskClassification};

/// Ordered prompt segments. The discriminant order *is* the required prompt
/// order from spec section 11.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ContextSegment {
    HarnessInstructions,
    Tools,
    ModelRules,
    RepositoryInstructions,
    Role,
    Task,
    State,
    RetrievedCode,
    LatestResult,
}

impl ContextSegment {
    pub fn as_str(&self) -> &'static str {
        match self {
            ContextSegment::HarnessInstructions => "harness_instructions",
            ContextSegment::Tools => "tools",
            ContextSegment::ModelRules => "model_rules",
            ContextSegment::RepositoryInstructions => "repository_instructions",
            ContextSegment::Role => "role",
            ContextSegment::Task => "task",
            ContextSegment::State => "state",
            ContextSegment::RetrievedCode => "retrieved_code",
            ContextSegment::LatestResult => "latest_result",
        }
    }

    /// Segments before the cache boundary are stable across steps of a session.
    pub fn is_stable(&self) -> bool {
        matches!(
            self,
            ContextSegment::HarnessInstructions
                | ContextSegment::Tools
                | ContextSegment::ModelRules
                | ContextSegment::RepositoryInstructions
        )
    }
}

/// Marker rendered between the stable prefix and the volatile suffix.
pub const CACHE_BOUNDARY: &str = "---------- cache boundary ----------";

/// One rendered prompt segment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptBlock {
    pub segment: ContextSegment,
    pub content: String,
}

impl PromptBlock {
    pub fn new(segment: ContextSegment, content: impl Into<String>) -> Self {
        Self {
            segment,
            content: content.into(),
        }
    }
}

/// A prompt whose stable prefix is separated from its volatile suffix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheFriendlyPrompt {
    blocks: Vec<PromptBlock>,
}

impl CacheFriendlyPrompt {
    /// Assemble blocks into cache-friendly order.
    ///
    /// Rejects duplicate segments rather than silently keeping one: a duplicate
    /// almost always means a caller appended volatile text into a stable
    /// segment, which invalidates the prefix cache on every step.
    pub fn assemble(blocks: Vec<PromptBlock>) -> Result<Self, String> {
        let mut seen = Vec::new();
        for block in &blocks {
            if seen.contains(&block.segment) {
                return Err(format!("duplicate prompt segment {}", block.segment.as_str()));
            }
            seen.push(block.segment);
        }
        let mut blocks = blocks;
        blocks.sort_by_key(|block| block.segment);
        Ok(Self { blocks })
    }

    pub fn blocks(&self) -> &[PromptBlock] {
        &self.blocks
    }

    /// Text before the cache boundary.
    pub fn stable_prefix(&self) -> String {
        render(self.blocks.iter().filter(|block| block.segment.is_stable()))
    }

    /// Text after the cache boundary.
    pub fn volatile_suffix(&self) -> String {
        render(self.blocks.iter().filter(|block| !block.segment.is_stable()))
    }

    /// Stable identity of the prefix, for provider prefix-cache keys.
    pub fn stable_prefix_hash(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.stable_prefix().as_bytes());
        let digest = hasher.finalize();
        digest.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    pub fn render(&self) -> String {
        let prefix = self.stable_prefix();
        let suffix = self.volatile_suffix();
        match (prefix.is_empty(), suffix.is_empty()) {
            (true, _) => suffix,
            (false, true) => prefix,
            (false, false) => format!("{prefix}\n{CACHE_BOUNDARY}\n{suffix}"),
        }
    }

    /// Rough token estimate (4 characters per token) for budgeting only.
    ///
    /// Real counts come from harness context measurement; this is for deciding
    /// whether to even attempt a request.
    pub fn estimated_tokens(&self) -> u64 {
        (self.render().len() as u64).div_ceil(4)
    }

    pub fn estimated_stable_tokens(&self) -> u64 {
        (self.stable_prefix().len() as u64).div_ceil(4)
    }
}

fn render<'a>(blocks: impl Iterator<Item = &'a PromptBlock>) -> String {
    blocks
        .map(|block| format!("[{}]\n{}", block.segment.as_str(), block.content.trim_end()))
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Retrieval mechanisms available to the explorer, cheapest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RetrievalStrategy {
    PathSearch,
    TextSearch,
    SymbolSearch,
    AstIntelligence,
    Tests,
    DependencyGraph,
    GitHistory,
    IssueLinks,
    PriorFindings,
    RepositoryMap,
    Embeddings,
}

impl RetrievalStrategy {
    pub fn as_str(&self) -> &'static str {
        match self {
            RetrievalStrategy::PathSearch => "path_search",
            RetrievalStrategy::TextSearch => "text_search",
            RetrievalStrategy::SymbolSearch => "symbol_search",
            RetrievalStrategy::AstIntelligence => "ast_intelligence",
            RetrievalStrategy::Tests => "tests",
            RetrievalStrategy::DependencyGraph => "dependency_graph",
            RetrievalStrategy::GitHistory => "git_history",
            RetrievalStrategy::IssueLinks => "issue_links",
            RetrievalStrategy::PriorFindings => "prior_findings",
            RetrievalStrategy::RepositoryMap => "repository_map",
            RetrievalStrategy::Embeddings => "embeddings",
        }
    }
}

/// One rung of the retrieval ladder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetrievalStep {
    pub strategy: RetrievalStrategy,
    pub max_files: usize,
    pub rationale: String,
}

/// How much repository context an execution may pull in, and in what order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextPolicy {
    /// Full conversation history is never injected by default.
    pub include_full_history: bool,
    pub max_retrieved_files: usize,
    pub max_retrieved_lines: usize,
    pub max_expansion_rounds: u8,
    pub ladder: Vec<RetrievalStep>,
}

impl Default for ContextPolicy {
    fn default() -> Self {
        Self {
            include_full_history: false,
            max_retrieved_files: 8,
            max_retrieved_lines: 1_200,
            max_expansion_rounds: 3,
            ladder: Vec::new(),
        }
    }
}

impl ContextPolicy {
    /// The step to run for `round`, or `None` once expansion is exhausted.
    ///
    /// A round that produced evidence stops the ladder: widening after a hit
    /// spends context on material the model did not need.
    pub fn next_step(&self, round: u8, previous_round_found_evidence: bool) -> Option<&RetrievalStep> {
        if previous_round_found_evidence || round >= self.max_expansion_rounds {
            return None;
        }
        self.ladder.get(usize::from(round))
    }
}

/// Build a retrieval ladder and budget from the classification.
pub fn context_policy_for(classification: &TaskClassification) -> ContextPolicy {
    let (max_files, max_lines, rounds) = match classification.complexity {
        Complexity::Trivial => (3, 400, 1),
        Complexity::Low => (5, 800, 2),
        Complexity::Medium => (8, 1_200, 3),
        Complexity::High => (16, 2_400, 4),
        Complexity::Exceptional => (24, 4_000, 5),
    };

    // Narrow and targeted first, then generic widening, then broad. The
    // class-specific rung goes before text/symbol search because it is the
    // *most* targeted step for that class -- neighbouring tests are the point
    // of a test task, not a widening of it. Appending them instead put them
    // past the round ceiling for exactly the classes that needed them.
    let mut ladder = vec![RetrievalStep {
        strategy: RetrievalStrategy::PathSearch,
        max_files: 3.min(max_files),
        rationale: "start from the paths the work item already names".to_string(),
    }];

    match classification.task_class {
        TaskClass::Bugfix => ladder.push(RetrievalStep {
            strategy: RetrievalStrategy::GitHistory,
            max_files,
            rationale: "find the commit that introduced the behaviour".to_string(),
        }),
        TaskClass::Test => ladder.push(RetrievalStep {
            strategy: RetrievalStrategy::Tests,
            max_files,
            rationale: "match the conventions of neighbouring tests".to_string(),
        }),
        TaskClass::Refactor | TaskClass::Migration => ladder.push(RetrievalStep {
            strategy: RetrievalStrategy::DependencyGraph,
            max_files,
            rationale: "enumerate every caller before changing a signature".to_string(),
        }),
        TaskClass::Research => ladder.push(RetrievalStep {
            strategy: RetrievalStrategy::PriorFindings,
            max_files,
            rationale: "reuse prior AutoSpec findings before re-deriving them".to_string(),
        }),
        _ => {}
    }

    ladder.push(RetrievalStep {
        strategy: RetrievalStrategy::TextSearch,
        max_files: 5.min(max_files),
        rationale: "widen to literal identifiers and error strings".to_string(),
    });
    ladder.push(RetrievalStep {
        strategy: RetrievalStrategy::SymbolSearch,
        max_files,
        rationale: "resolve definitions and call sites for the symbols found".to_string(),
    });

    match classification.task_class {
        TaskClass::Bugfix => ladder.push(RetrievalStep {
            strategy: RetrievalStrategy::Tests,
            max_files,
            rationale: "locate a test that already covers the surface".to_string(),
        }),
        TaskClass::Research => ladder.push(RetrievalStep {
            strategy: RetrievalStrategy::RepositoryMap,
            max_files,
            rationale: "orient in unfamiliar areas of the repository".to_string(),
        }),
        _ => {}
    }

    if classification.requires_long_context {
        ladder.push(RetrievalStep {
            strategy: RetrievalStrategy::Embeddings,
            max_files,
            rationale: "last resort for work spanning unfamiliar areas".to_string(),
        });
    }

    ladder.truncate(usize::from(rounds));

    ContextPolicy {
        include_full_history: false,
        max_retrieved_files: max_files,
        max_retrieved_lines: max_lines,
        max_expansion_rounds: rounds,
        ladder,
    }
}

/// Outcome of checking an assembled prompt against a context policy and a
/// node's free context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextFit {
    pub estimated_tokens: u64,
    pub free_context: u64,
    pub fits: bool,
    pub reasons: Vec<String>,
}

/// Check whether a prompt plus its projected growth fits the available window.
pub fn check_context_fit(
    prompt: &CacheFriendlyPrompt,
    free_context: u64,
    projected_growth: u64,
) -> ContextFit {
    let estimated = prompt.estimated_tokens();
    let required = estimated.saturating_add(projected_growth);
    let fits = required <= free_context;
    let mut reasons = vec![format!(
        "estimated_prompt_tokens={estimated} projected_growth={projected_growth} free_context={free_context}"
    )];
    if !fits {
        reasons.push(format!(
            "prompt requires {required} tokens but only {free_context} are free"
        ));
    }
    ContextFit {
        estimated_tokens: estimated,
        free_context,
        fits,
        reasons,
    }
}
