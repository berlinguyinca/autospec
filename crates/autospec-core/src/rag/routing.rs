//! InferWeave capability declaration and context-aware routing (spec sections
//! 23 and 24).
//!
//! The RAG subsystem declares what a subtask *needs* and lets InferWeave choose
//! a node; it never names a model. Section 24 adds the constraint that makes
//! this more than a filter: a faster node must not be selected if it lacks the
//! free context capacity, and among eligible nodes the tightest fit wins so
//! large contiguous capacity stays available for the next large request.

/// How much reasoning a retrieval subtask needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ReasoningClass {
    /// Classification, rewriting, scoring.
    Small,
    /// Code relationship analysis.
    Medium,
    /// Architecture synthesis and planning.
    Strong,
}

impl ReasoningClass {
    /// Stable wire identifier.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Small => "small",
            Self::Medium => "medium",
            Self::Strong => "strong",
        }
    }
}

/// Whether latency or throughput matters more for a subtask.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LatencyPriority {
    /// Inside the retrieval loop; the agent is waiting.
    High,
    /// Batchable.
    Normal,
}

impl LatencyPriority {
    /// Stable wire identifier.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::High => "high",
            Self::Normal => "normal",
        }
    }
}

/// The retrieval-side model subtasks (spec section 23).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RagModelTask {
    /// Classify what the task needs.
    TaskClassification,
    /// Rewrite a query.
    QueryRewriting,
    /// Score retrieved evidence.
    RelevanceScoring,
    /// Work out how code relates.
    CodeRelationshipAnalysis,
    /// Synthesize an architecture answer.
    ArchitectureSynthesis,
    /// Produce an implementation plan.
    ImplementationPlan,
}

impl RagModelTask {
    /// Stable wire identifier.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TaskClassification => "task_classification",
            Self::QueryRewriting => "query_rewriting",
            Self::RelevanceScoring => "relevance_scoring",
            Self::CodeRelationshipAnalysis => "code_relationship_analysis",
            Self::ArchitectureSynthesis => "architecture_synthesis",
            Self::ImplementationPlan => "implementation_plan",
        }
    }

    /// The capability requirement section 23 assigns this subtask.
    pub fn capabilities(self, estimated_context_tokens: u32) -> ModelCapabilities {
        let (reasoning, coding, latency) = match self {
            Self::TaskClassification | Self::QueryRewriting | Self::RelevanceScoring => {
                (ReasoningClass::Small, false, LatencyPriority::High)
            }
            Self::CodeRelationshipAnalysis => (ReasoningClass::Medium, true, LatencyPriority::High),
            Self::ArchitectureSynthesis => (ReasoningClass::Strong, false, LatencyPriority::Normal),
            Self::ImplementationPlan => (ReasoningClass::Strong, true, LatencyPriority::Normal),
        };
        ModelCapabilities {
            reasoning_class: reasoning,
            coding,
            min_context: estimated_context_tokens,
            structured_output: true,
            latency_priority: latency,
        }
    }
}

/// What a retrieval subtask requires of a model (spec section 23).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelCapabilities {
    /// Minimum reasoning class.
    pub reasoning_class: ReasoningClass,
    /// Whether the subtask needs a coding model.
    pub coding: bool,
    /// Context tokens the request will occupy.
    pub min_context: u32,
    /// Whether structured output is required.
    pub structured_output: bool,
    /// Latency sensitivity.
    pub latency_priority: LatencyPriority,
}

/// A candidate node as InferWeave reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeCandidate {
    /// Node identifier.
    pub id: String,
    /// Reasoning class the node's model provides.
    pub reasoning_class: ReasoningClass,
    /// Whether the node's model is a coding model.
    pub coding: bool,
    /// Whether the node supports structured output.
    pub structured_output: bool,
    /// Free context tokens right now.
    pub free_context_tokens: u32,
    /// Relative speed; higher is faster.
    pub speed_rank: u32,
    /// Free seats.
    pub available_seats: u32,
}

/// Why a node was rejected, for the routing view (spec section 36.6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeRejection {
    /// Node identifier.
    pub node_id: String,
    /// Rejection reason.
    pub reason: String,
}

/// The outcome of a routing decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutingDecision {
    /// The chosen node, when one was eligible.
    pub selected: Option<NodeCandidate>,
    /// Nodes that were filtered out, and why.
    pub rejected: Vec<NodeRejection>,
    /// Context tokens the request was sized at, including the safety margin.
    pub required_context_tokens: u32,
}

/// Extra context reserved beyond the estimate, in permille of the estimate.
///
/// The token estimate is an approximation (see `compression::estimate_tokens`);
/// routing to a node with exactly the estimated capacity would fail whenever
/// the real tokenizer counts higher, and a failed request costs more than a
/// slightly larger node.
const CONTEXT_SAFETY_MARGIN_PERMILLE: u32 = 100;

/// Choose a node for a retrieval subtask.
///
/// Filtering runs in section 24's order — capability, then context capacity —
/// and only then does packing choose among survivors. A node is never selected
/// on speed alone.
pub fn select_node(
    capabilities: &ModelCapabilities,
    candidates: &[NodeCandidate],
) -> RoutingDecision {
    let required = capabilities
        .min_context
        .saturating_add(capabilities.min_context / 1000 * CONTEXT_SAFETY_MARGIN_PERMILLE)
        .max(capabilities.min_context);
    let mut rejected = Vec::new();
    let mut eligible = Vec::new();

    for candidate in candidates {
        if candidate.reasoning_class < capabilities.reasoning_class {
            rejected.push(NodeRejection {
                node_id: candidate.id.clone(),
                reason: format!(
                    "reasoning class {} below required {}",
                    candidate.reasoning_class.as_str(),
                    capabilities.reasoning_class.as_str()
                ),
            });
            continue;
        }
        if capabilities.coding && !candidate.coding {
            rejected.push(NodeRejection {
                node_id: candidate.id.clone(),
                reason: "coding model required".to_string(),
            });
            continue;
        }
        if capabilities.structured_output && !candidate.structured_output {
            rejected.push(NodeRejection {
                node_id: candidate.id.clone(),
                reason: "structured output required".to_string(),
            });
            continue;
        }
        if candidate.available_seats == 0 {
            rejected.push(NodeRejection {
                node_id: candidate.id.clone(),
                reason: "no available seats".to_string(),
            });
            continue;
        }
        if candidate.free_context_tokens < required {
            rejected.push(NodeRejection {
                node_id: candidate.id.clone(),
                reason: format!(
                    "free context {} below required {}",
                    candidate.free_context_tokens, required
                ),
            });
            continue;
        }
        eligible.push(candidate.clone());
    }

    // Pack from lower free context upward (section 24), preserving the large
    // contiguous windows for requests that will need them. Speed breaks ties
    // among nodes with equal capacity, and the id breaks ties after that so
    // the decision is reproducible.
    eligible.sort_by(|left, right| {
        left.free_context_tokens
            .cmp(&right.free_context_tokens)
            .then(right.speed_rank.cmp(&left.speed_rank))
            .then(left.id.cmp(&right.id))
    });

    RoutingDecision {
        selected: eligible.first().cloned(),
        rejected,
        required_context_tokens: required,
    }
}
