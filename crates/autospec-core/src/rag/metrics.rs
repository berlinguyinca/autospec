//! Retrieval metrics (spec sections 37 and 38).
//!
//! Counters are derived from a completed trace rather than incremented at call
//! sites. One source of truth means the dashboard and the metrics endpoint can
//! never disagree about what a run did, and a metric can be added later without
//! finding every place that should have counted it.

use crate::rag::budget::{BudgetLedger, StopReason};
use crate::rag::context_package::ContextPackage;
use crate::rag::trace::{RetrievalTrace, TraceEvent};

/// The counters section 37 requires, for one retrieval.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RetrievalMetrics {
    /// Loop iterations run.
    pub iterations: u64,
    /// Source queries issued.
    pub source_queries: u64,
    /// Queries suppressed as repeats.
    pub queries_suppressed: u64,
    /// Graph traversals run.
    pub graph_traversals: u64,
    /// Evidence items in the returned package.
    pub evidence_items: u64,
    /// Context tokens supplied to the calling model.
    pub context_tokens: u64,
    /// Context tokens the budget allowed but the package did not use.
    pub context_tokens_saved: u64,
    /// Cache hits.
    pub cache_hits: u64,
    /// Cache misses.
    pub cache_misses: u64,
    /// Contradictions surfaced.
    pub contradictions_detected: u64,
    /// Evaluations that returned insufficient.
    pub insufficient_evidence: u64,
    /// Source failures.
    pub retrieval_failures: u64,
    /// Evidence withheld as a likely injection.
    pub evidence_quarantined: u64,
    /// Evidence dropped as stale.
    pub evidence_stale: u64,
    /// Model tokens spent on retrieval-side calls.
    pub model_tokens: u64,
}

impl RetrievalMetrics {
    /// Derive counters from a finished trace, its ledger, and its package.
    pub fn from_execution(
        trace: &RetrievalTrace,
        ledger: &BudgetLedger,
        package: Option<&ContextPackage>,
    ) -> Self {
        let mut metrics = Self {
            iterations: trace.iterations().len() as u64,
            model_tokens: u64::from(ledger.model_tokens()),
            ..Self::default()
        };
        for event in trace
            .iterations()
            .iter()
            .flat_map(|iteration| iteration.events.iter())
        {
            match event {
                TraceEvent::Query { .. } => metrics.source_queries += 1,
                TraceEvent::QuerySuppressed { .. } => metrics.queries_suppressed += 1,
                TraceEvent::GraphTraversal { .. } => metrics.graph_traversals += 1,
                TraceEvent::SourceFailure { .. } => metrics.retrieval_failures += 1,
                TraceEvent::ContradictionFound { .. } => metrics.contradictions_detected += 1,
                TraceEvent::EvidenceQuarantined { .. } => metrics.evidence_quarantined += 1,
                TraceEvent::EvidenceStale { .. } => metrics.evidence_stale += 1,
                TraceEvent::CacheLookup { hit, .. } => {
                    if *hit {
                        metrics.cache_hits += 1;
                    } else {
                        metrics.cache_misses += 1;
                    }
                }
                TraceEvent::Evaluation { decision, .. } => {
                    if !decision.is_sufficient() {
                        metrics.insufficient_evidence += 1;
                    }
                }
                TraceEvent::ModelCall { .. } => {}
            }
        }
        if let Some(package) = package {
            metrics.evidence_items = package.all_evidence().len() as u64;
            metrics.context_tokens = u64::from(package.actual_tokens());
            metrics.context_tokens_saved = u64::from(
                package
                    .requested_tokens()
                    .saturating_sub(package.actual_tokens()),
            );
        }
        metrics
    }

    /// Cache hit ratio in permille.
    pub fn cache_hit_ratio_permille(&self) -> u16 {
        let total = self.cache_hits + self.cache_misses;
        if total == 0 {
            return 0;
        }
        ((self.cache_hits * 1000 + total / 2) / total) as u16
    }

    /// Add another retrieval's counters into this one.
    pub fn merge(&mut self, other: &Self) {
        self.iterations += other.iterations;
        self.source_queries += other.source_queries;
        self.queries_suppressed += other.queries_suppressed;
        self.graph_traversals += other.graph_traversals;
        self.evidence_items += other.evidence_items;
        self.context_tokens += other.context_tokens;
        self.context_tokens_saved += other.context_tokens_saved;
        self.cache_hits += other.cache_hits;
        self.cache_misses += other.cache_misses;
        self.contradictions_detected += other.contradictions_detected;
        self.insufficient_evidence += other.insufficient_evidence;
        self.retrieval_failures += other.retrieval_failures;
        self.evidence_quarantined += other.evidence_quarantined;
        self.evidence_stale += other.evidence_stale;
        self.model_tokens += other.model_tokens;
    }

    /// Render as Prometheus-style `name value` lines, sorted by name.
    pub fn render(&self) -> String {
        let mut lines = vec![
            format!("rag_iterations_total {}", self.iterations),
            format!("rag_source_queries_total {}", self.source_queries),
            format!("rag_queries_suppressed_total {}", self.queries_suppressed),
            format!("rag_graph_traversals_total {}", self.graph_traversals),
            format!("rag_evidence_items_total {}", self.evidence_items),
            format!("rag_context_tokens_total {}", self.context_tokens),
            format!("rag_context_tokens_saved {}", self.context_tokens_saved),
            format!(
                "rag_cache_hit_ratio_permille {}",
                self.cache_hit_ratio_permille()
            ),
            format!(
                "rag_contradictions_detected_total {}",
                self.contradictions_detected
            ),
            format!(
                "rag_insufficient_evidence_total {}",
                self.insufficient_evidence
            ),
            format!("rag_retrieval_failures_total {}", self.retrieval_failures),
            format!(
                "rag_evidence_quarantined_total {}",
                self.evidence_quarantined
            ),
            format!("rag_stale_evidence_total {}", self.evidence_stale),
            format!("rag_model_tokens_total {}", self.model_tokens),
        ];
        lines.sort();
        lines.join("\n")
    }
}

/// Context efficiency for the dashboard's section 36.7 view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextEfficiency {
    /// Tokens in the whole searchable corpus.
    pub searchable_tokens: u64,
    /// Tokens retrieved before compression and budgeting.
    pub retrieved_tokens: u64,
    /// Tokens actually supplied to the calling model.
    pub supplied_tokens: u64,
}

impl ContextEfficiency {
    /// Fraction of the corpus that reached the model, in parts per million.
    ///
    /// Parts per million rather than permille because the whole point of the
    /// subsystem is that this number is tiny; permille would round most healthy
    /// runs to zero.
    pub fn supplied_fraction_ppm(&self) -> u64 {
        if self.searchable_tokens == 0 {
            return 0;
        }
        (self.supplied_tokens * 1_000_000) / self.searchable_tokens
    }

    /// Fraction of retrieved tokens that survived into the package, in permille.
    ///
    /// A low number means retrieval is casting too wide a net; a number near
    /// 1000 means compression is doing nothing and the budget may be loose.
    pub fn retrieval_utilization_permille(&self) -> u16 {
        if self.retrieved_tokens == 0 {
            return 0;
        }
        let ratio = (self.supplied_tokens * 1000) / self.retrieved_tokens;
        ratio.min(1000) as u16
    }
}

/// Outcome label for a completed retrieval, for per-status metric labels.
pub fn outcome_label(reason: &StopReason) -> &'static str {
    if reason.is_satisfied() {
        "satisfied"
    } else {
        reason.as_str()
    }
}
