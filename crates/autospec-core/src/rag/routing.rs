//! InferWeave capability declaration and context-aware routing (spec sections
//! 23 and 24).
//!
//! The RAG subsystem declares what a subtask *needs* and lets InferWeave choose
//! a node; it never names a model. Section 24 adds the constraint that makes
//! this more than a filter: a faster node must not be selected if it lacks the
//! free context capacity, and among eligible nodes the tightest fit wins so
//! large contiguous capacity stays available for the next large request.
//!
//! Free seats rank candidates rather than rejecting them: a pool whose nodes
//! are all saturated (zero free seats) still returns a node, flagged
//! `saturated_fallback`, so dispatch is never dead under full load. The
//! run-scoped `SeatLedger` records each dispatch against the chosen node so
//! consecutive calls in one run de-concentrate instead of converging on a
//! single worker; it is caller-held state, and `select_node` stays pure.

use std::collections::BTreeMap;

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
    /// The chosen node, when one passed the hard capability filters.
    /// Total over that set: a fully saturated pool still yields a node (see
    /// `saturated_fallback`). `None` only when no candidate passes the
    /// filters at all.
    pub selected: Option<NodeCandidate>,
    /// Nodes that were filtered out, and why.
    pub rejected: Vec<NodeRejection>,
    /// Context tokens the request was sized at, including the safety margin.
    pub required_context_tokens: u32,
    /// True when the chosen node had no free seat at decision time — either
    /// it reported zero or this run's in-flight dispatches consumed its last
    /// one. The caller should queue the dispatch behind the node rather than
    /// treat it as immediately runnable.
    pub saturated_fallback: bool,
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
///
/// Free seats are a ranking tier rather than a hard rejection (issue #3754):
/// the pool may be fully saturated and the dispatch still has to go
/// somewhere, so the least-saturated eligible node takes it and the decision
/// is flagged `saturated_fallback`.
pub fn select_node(
    capabilities: &ModelCapabilities,
    candidates: &[NodeCandidate],
) -> RoutingDecision {
    select_node_with_reservations(capabilities, candidates, &BTreeMap::new())
}

/// `select_node` with a run-scoped in-flight seat map: a node's effective
/// seats are `available_seats` minus the seats this run has already
/// dispatched to it, so a second call in the same loop sees the first call's
/// choice.
fn select_node_with_reservations(
    capabilities: &ModelCapabilities,
    candidates: &[NodeCandidate],
    in_flight: &BTreeMap<String, u32>,
) -> RoutingDecision {
    let required = capabilities
        .min_context
        .saturating_add(capabilities.min_context / 1000 * CONTEXT_SAFETY_MARGIN_PERMILLE)
        .max(capabilities.min_context);
    let mut rejected = Vec::new();
    // (candidate, signed free seats, in-flight seats). Signed so a node whose
    // reported seats the run has already consumed ranks below un-picked nodes
    // even when `available_seats` is already zero.
    let mut eligible: Vec<(NodeCandidate, i64, u32)> = Vec::new();

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
        let in_flight_here = in_flight.get(&candidate.id).copied().unwrap_or(0);
        let signed_seats = candidate.available_seats as i64 - in_flight_here as i64;
        eligible.push((candidate.clone(), signed_seats, in_flight_here));
    }

    // Rank by signed free seats (least-saturated first), then pack from lower
    // free context upward (section 24), preserving the large contiguous
    // windows for requests that will need them. Fewer in-flight dispatches
    // break seat ties, then speed, and the id breaks ties after that so the
    // decision is reproducible.
    eligible.sort_by(
        |(left, left_seats, left_flight), (right, right_seats, right_flight)| {
            right_seats
                .cmp(left_seats)
                .then(left_flight.cmp(right_flight))
                .then(left.free_context_tokens.cmp(&right.free_context_tokens))
                .then(right.speed_rank.cmp(&left.speed_rank))
                .then(left.id.cmp(&right.id))
        },
    );

    let (selected, saturated_fallback) = match eligible.first() {
        Some((node, signed_seats, _)) => (Some(node.clone()), *signed_seats <= 0),
        None => (None, false),
    };

    RoutingDecision {
        selected,
        rejected,
        required_context_tokens: required,
        saturated_fallback,
    }
}

/// Run-scoped in-flight seat counter (issue #3754).
///
/// One ledger per dispatch run. Each `select` that routes a node records one
/// in-flight seat against it, so the next call in the loop ranks that node
/// lower and consecutive dispatches spread across the pool instead of
/// converging on one worker — including when the pool is fully saturated and
/// only the ledger's own counts distinguish the workers. Call `release` when a
/// dispatch completes. The ledger is caller-held state; `select_node` stays
/// pure.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SeatLedger {
    in_flight: BTreeMap<String, u32>,
}

impl SeatLedger {
    /// An empty ledger: no dispatches in flight yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Route one dispatch, recording the chosen node's seat as in-flight so
    /// the next call in the run sees it.
    pub fn select(
        &mut self,
        capabilities: &ModelCapabilities,
        candidates: &[NodeCandidate],
    ) -> RoutingDecision {
        let decision = select_node_with_reservations(capabilities, candidates, &self.in_flight);
        if let Some(selected) = &decision.selected {
            *self.in_flight.entry(selected.id.clone()).or_insert(0) += 1;
        }
        decision
    }

    /// Release `seats` this ledger previously recorded against `node_id`.
    /// Saturates at zero: a release without a matching dispatch is a no-op
    /// rather than a way to fabricate free seats.
    pub fn release(&mut self, node_id: &str, seats: u32) {
        if let Some(current) = self.in_flight.get_mut(node_id) {
            *current = current.saturating_sub(seats);
            if *current == 0 {
                self.in_flight.remove(node_id);
            }
        }
    }

    /// Seats currently recorded in-flight against `node_id`.
    pub fn in_flight(&self, node_id: &str) -> u32 {
        self.in_flight.get(node_id).copied().unwrap_or(0)
    }
}
