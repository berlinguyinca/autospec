//! Retrieval traces (spec sections 35 and 54).
//!
//! Every execution produces one. The trace answers section 54's question — "why
//! did AutoSpec believe this?" — and it is deliberately *not* prompt material:
//! section 35 says traces are observable but need not be inserted into the
//! downstream prompt, and putting them there would spend the context the
//! retrieval just worked to conserve.

use crate::rag::budget::StopReason;
use crate::rag::evaluator::SufficiencyDecision;
use crate::rag::score::Score;
use crate::rag::source::{SearchMode, SourceKind};

/// What happened in one step of the loop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TraceEvent {
    /// A query was issued to a source.
    Query {
        /// Query text as issued.
        query: String,
        /// Lookup shape.
        mode: SearchMode,
        /// Source queried.
        source: SourceKind,
        /// Evidence items returned.
        results: usize,
        /// Whether the source truncated its answer.
        truncated: bool,
    },
    /// A query was planned but suppressed as a repeat.
    QuerySuppressed {
        /// Query text.
        query: String,
        /// Why it was suppressed.
        reason: String,
    },
    /// A source failed.
    SourceFailure {
        /// Source that failed.
        source: SourceKind,
        /// Failure detail.
        detail: String,
    },
    /// A graph traversal ran.
    GraphTraversal {
        /// Origin node.
        origin: String,
        /// Depth requested.
        depth: u32,
        /// Nodes reached.
        reached: usize,
    },
    /// Evidence was evaluated.
    Evaluation {
        /// The verdict.
        decision: SufficiencyDecision,
        /// Why.
        reason: String,
        /// Mean relevance at the time.
        mean_relevance: Score,
    },
    /// A cache lookup was made.
    CacheLookup {
        /// Cache key.
        key: String,
        /// Whether it hit.
        hit: bool,
    },
    /// A contradiction was recorded.
    ContradictionFound {
        /// Contradiction identifier.
        contradiction_id: String,
        /// What the sides disagree about.
        topic: String,
    },
    /// Evidence was dropped because its source's staleness rule rejected it.
    EvidenceStale {
        /// Evidence identifier.
        evidence_id: String,
        /// Source the evidence came from.
        source: SourceKind,
        /// Why it is stale.
        reason: String,
    },
    /// Content was withheld as a likely injection.
    EvidenceQuarantined {
        /// Evidence identifier.
        evidence_id: String,
        /// Markers that matched.
        markers: Vec<String>,
    },
    /// A retrieval-side model call was made.
    ModelCall {
        /// What the model was asked to do.
        purpose: String,
        /// Capability class requested.
        capability: String,
        /// Tokens charged.
        tokens: u32,
    },
}

impl TraceEvent {
    /// Stable event name for dashboards and metrics.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Query { .. } => "query",
            Self::QuerySuppressed { .. } => "query_suppressed",
            Self::SourceFailure { .. } => "source_failure",
            Self::GraphTraversal { .. } => "graph_traversal",
            Self::Evaluation { .. } => "evaluation",
            Self::CacheLookup { .. } => "cache_lookup",
            Self::ContradictionFound { .. } => "contradiction_found",
            Self::EvidenceStale { .. } => "evidence_stale",
            Self::EvidenceQuarantined { .. } => "evidence_quarantined",
            Self::ModelCall { .. } => "model_call",
        }
    }

    /// One line for the trace view (spec section 36.2).
    pub fn describe(&self) -> String {
        match self {
            Self::Query {
                query,
                mode,
                source,
                results,
                truncated,
            } => format!(
                "query [{}] \"{}\" -> {} ({} result(s){})",
                mode.as_str(),
                query,
                source.as_str(),
                results,
                if *truncated { ", truncated" } else { "" }
            ),
            Self::QuerySuppressed { query, reason } => {
                format!("suppressed \"{query}\": {reason}")
            }
            Self::SourceFailure { source, detail } => {
                format!("source {} failed: {detail}", source.as_str())
            }
            Self::GraphTraversal {
                origin,
                depth,
                reached,
            } => format!("traverse {origin} depth {depth} -> {reached} node(s)"),
            Self::Evaluation {
                decision,
                reason,
                mean_relevance,
            } => format!(
                "evaluation {}: {reason} (mean relevance {mean_relevance})",
                decision.as_str()
            ),
            Self::CacheLookup { key, hit } => {
                format!("cache {} for {key}", if *hit { "hit" } else { "miss" })
            }
            Self::ContradictionFound {
                contradiction_id,
                topic,
            } => format!("contradiction {contradiction_id} on {topic}"),
            Self::EvidenceStale {
                evidence_id,
                source,
                reason,
            } => format!(
                "dropped {evidence_id} from {}: {reason}",
                source.as_str()
            ),
            Self::EvidenceQuarantined {
                evidence_id,
                markers,
            } => format!(
                "quarantined {evidence_id}: injection markers {}",
                markers.join(", ")
            ),
            Self::ModelCall {
                purpose,
                capability,
                tokens,
            } => format!("model call {purpose} [{capability}] {tokens} token(s)"),
        }
    }
}

/// One iteration of the agentic loop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceIteration {
    /// 1-based iteration number.
    pub number: u32,
    /// Events in the order they occurred.
    pub events: Vec<TraceEvent>,
}

/// A complete record of one retrieval execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetrievalTrace {
    retrieval_id: String,
    task_id: String,
    role: String,
    question: String,
    iterations: Vec<TraceIteration>,
    stop_reason: Option<StopReason>,
}

impl RetrievalTrace {
    /// Open a trace for an execution.
    pub fn new(
        retrieval_id: impl Into<String>,
        task_id: impl Into<String>,
        role: impl Into<String>,
        question: impl Into<String>,
    ) -> Self {
        Self {
            retrieval_id: retrieval_id.into(),
            task_id: task_id.into(),
            role: role.into(),
            question: question.into(),
            iterations: Vec::new(),
            stop_reason: None,
        }
    }

    /// Trace identifier.
    pub fn retrieval_id(&self) -> &str {
        &self.retrieval_id
    }

    /// Task the retrieval served.
    pub fn task_id(&self) -> &str {
        &self.task_id
    }

    /// Role the retrieval served.
    pub fn role(&self) -> &str {
        &self.role
    }

    /// The question asked.
    pub fn question(&self) -> &str {
        &self.question
    }

    /// Recorded iterations.
    pub fn iterations(&self) -> &[TraceIteration] {
        &self.iterations
    }

    /// Why retrieval stopped, once it has.
    pub fn stop_reason(&self) -> Option<&StopReason> {
        self.stop_reason.as_ref()
    }

    /// Open a new iteration.
    pub fn begin_iteration(&mut self, number: u32) {
        self.iterations.push(TraceIteration {
            number,
            events: Vec::new(),
        });
    }

    /// Record an event in the current iteration.
    ///
    /// An event recorded before any iteration opens starts iteration 1, so a
    /// pre-loop cache lookup is never dropped on the floor.
    pub fn record(&mut self, event: TraceEvent) {
        if self.iterations.is_empty() {
            self.begin_iteration(1);
        }
        self.iterations
            .last_mut()
            .expect("an iteration was just ensured")
            .events
            .push(event);
    }

    /// Close the trace with the reason retrieval stopped.
    pub fn finish(&mut self, reason: StopReason) {
        self.stop_reason = Some(reason);
    }

    /// Total events across all iterations.
    pub fn event_count(&self) -> usize {
        self.iterations
            .iter()
            .map(|iteration| iteration.events.len())
            .sum()
    }

    /// Count events of one kind.
    pub fn count_events(&self, name: &str) -> usize {
        self.iterations
            .iter()
            .flat_map(|iteration| iteration.events.iter())
            .filter(|event| event.name() == name)
            .count()
    }

    /// Render the trace as the indented text of section 35.
    pub fn render(&self) -> String {
        let mut rendered = format!(
            "retrieval {} (task {}, role {})\nquestion: {}\n",
            self.retrieval_id, self.task_id, self.role, self.question
        );
        for iteration in &self.iterations {
            rendered.push_str(&format!("\nIteration {}\n", iteration.number));
            for event in &iteration.events {
                rendered.push_str(&format!("  {}\n", event.describe()));
            }
        }
        match &self.stop_reason {
            Some(reason) => {
                rendered.push_str(&format!("\nstopped: {}\n", reason.describe()));
            }
            None => rendered.push_str("\nstopped: still running\n"),
        }
        rendered
    }
}
