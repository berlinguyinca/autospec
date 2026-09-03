//! The agentic retrieval loop (spec sections 6, 32, 33, 40).
//!
//! This is the coordinator section 49 names. It runs the section 6 lifecycle —
//! plan, select sources, retrieve, evaluate, reformulate, stop — against a
//! [`SourceRegistry`], charging every step to a [`BudgetLedger`] and recording
//! every step in a [`RetrievalTrace`].
//!
//! It performs no I/O of its own. Sources do the retrieving, the caller
//! supplies wall-clock readings, and the loop stays a pure function of its
//! inputs — which is what makes the budget, stopping-rule and worktree tests
//! deterministic.

use crate::rag::budget::{BudgetLedger, RetrievalBudget, StopReason};
use crate::rag::cache::{CacheClass, CacheEntry, CacheKey, RetrievalCache};
use crate::rag::context_package::{ContextPackage, ContextPackageBuilder};
use crate::rag::contradiction::ContradictionSet;
use crate::rag::evaluator::{
    EvaluationRequest, EvidenceAssessment, EvidenceEvaluator, SufficiencyDecision,
};
use crate::rag::evidence::Evidence;
use crate::rag::freshness::{Freshness, FreshnessInput, FreshnessPolicy};
use crate::rag::injection;
use crate::rag::policy::{AgentRole, RetrievalPolicy};
use crate::rag::query::{PlannedQuery, QueryPlanner, extract_symbols};
use crate::rag::scope::RetrievalScope;
use crate::rag::source::{SearchRequest, SourceKind, SourceRegistry};
use crate::rag::trace::{RetrievalTrace, TraceEvent};

/// A retrieval request (spec section 32's `POST /v1/rag/retrieve`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetrievalRequest {
    /// Task the retrieval serves.
    pub task_id: String,
    /// Role the package is shaped for.
    pub role: AgentRole,
    /// The question.
    pub question: String,
    /// Source state to read.
    pub scope: RetrievalScope,
    /// Aspects that must each be covered before the evidence counts.
    pub required_aspects: Vec<String>,
    /// Minimum distinct evidence items.
    pub min_evidence_items: usize,
    /// Budget for this retrieval.
    pub budget: RetrievalBudget,
}

impl RetrievalRequest {
    /// Build a request with the role's default budget.
    pub fn new(
        task_id: impl Into<String>,
        role: AgentRole,
        question: impl Into<String>,
        scope: RetrievalScope,
    ) -> Self {
        Self {
            task_id: task_id.into(),
            role,
            question: question.into(),
            scope,
            required_aspects: Vec::new(),
            min_evidence_items: 1,
            budget: RetrievalBudget::default(),
        }
    }

    /// Require that each aspect be covered before stopping.
    pub fn requiring(mut self, aspects: impl IntoIterator<Item = String>) -> Self {
        self.required_aspects = aspects.into_iter().collect();
        self
    }

    /// Require a minimum number of distinct evidence items.
    pub fn with_min_items(mut self, minimum: usize) -> Self {
        self.min_evidence_items = minimum;
        self
    }

    /// Override the budget.
    pub fn with_budget(mut self, budget: RetrievalBudget) -> Self {
        self.budget = budget;
        self
    }
}

/// A retrieval result (spec section 33).
#[derive(Debug, Clone)]
pub struct RetrievalOutcome {
    /// Execution identifier.
    pub retrieval_id: String,
    /// Why the loop stopped.
    pub stop_reason: StopReason,
    /// The assembled package, absent only when assembly itself failed.
    pub package: Option<ContextPackage>,
    /// The final evaluator assessment.
    pub assessment: Option<EvidenceAssessment>,
    /// The full trace.
    pub trace: RetrievalTrace,
    /// Budget spend at exit.
    pub ledger: BudgetLedger,
    /// Set when the package could not be assembled.
    pub package_error: Option<String>,
}

impl RetrievalOutcome {
    /// Section 33's `status` field.
    pub fn status(&self) -> &'static str {
        if self.stop_reason.is_satisfied() {
            "sufficient"
        } else {
            "insufficient"
        }
    }
}

/// Runs the agentic retrieval loop.
pub struct RetrievalCoordinator<'a> {
    registry: &'a SourceRegistry,
    policy: RetrievalPolicy,
    freshness: FreshnessPolicy,
    cache: Option<&'a mut RetrievalCache>,
    now: u64,
    current_revision: String,
}

impl<'a> RetrievalCoordinator<'a> {
    /// Build a coordinator for a role.
    ///
    /// `now` and `current_revision` are supplied rather than read: freshness
    /// and cache validity are decisions about the caller's world, and a
    /// coordinator that read the clock could not be tested against a fixed
    /// expectation.
    pub fn new(
        registry: &'a SourceRegistry,
        role: AgentRole,
        now: u64,
        current_revision: impl Into<String>,
    ) -> Self {
        Self {
            registry,
            policy: RetrievalPolicy::for_role(role),
            freshness: FreshnessPolicy::default(),
            cache: None,
            now,
            current_revision: current_revision.into(),
        }
    }

    /// Override the role policy.
    pub fn with_policy(mut self, policy: RetrievalPolicy) -> Self {
        self.policy = policy;
        self
    }

    /// Override the freshness policy.
    pub fn with_freshness(mut self, freshness: FreshnessPolicy) -> Self {
        self.freshness = freshness;
        self
    }

    /// Attach a cache.
    pub fn with_cache(mut self, cache: &'a mut RetrievalCache) -> Self {
        self.cache = Some(cache);
        self
    }

    /// Execute the loop.
    pub fn retrieve(
        &mut self,
        retrieval_id: impl Into<String>,
        request: &RetrievalRequest,
    ) -> Result<RetrievalOutcome, String> {
        let retrieval_id = retrieval_id.into();
        let budget = self.policy.apply_to_budget(&request.budget);
        let mut ledger = BudgetLedger::new(budget)?;
        let mut trace = RetrievalTrace::new(
            retrieval_id.clone(),
            request.task_id.clone(),
            request.role.as_str(),
            request.question.clone(),
        );
        let mut planner = QueryPlanner::new();
        let mut evaluator = EvidenceEvaluator::new();
        let mut evidence: Vec<Evidence> = Vec::new();
        let mut known_symbols = extract_symbols(&request.question);
        let sources = self.ordered_sources();
        let evaluation_request = EvaluationRequest::new(request.question.clone())
            .requiring(request.required_aspects.clone())
            .with_min_items(request.min_evidence_items);

        let mut assessment: Option<EvidenceAssessment> = None;
        let mut planned = planner.plan_initial(&request.question, &sources);

        let stop_reason = loop {
            let iteration = match ledger.start_iteration() {
                Ok(iteration) => iteration,
                Err(reason) => break reason,
            };
            trace.begin_iteration(iteration);

            if planned.is_empty() {
                break StopReason::NoNewEvidence {
                    unproductive_iterations: ledger.unproductive_iterations(),
                };
            }

            let mut accepted_this_iteration = 0_u32;
            let mut budget_stop = None;
            for query in &planned {
                match self.run_query(query, request, &mut ledger, &mut trace, &mut evaluator) {
                    Ok(found) => {
                        for item in found {
                            if ledger.evidence_room() == 0 {
                                break;
                            }
                            if evidence.iter().any(|held| held.duplicates(&item)) {
                                continue;
                            }
                            if let Some(symbol) = item.location().symbol.clone() {
                                if !known_symbols.contains(&symbol) {
                                    known_symbols.push(symbol);
                                }
                            }
                            evidence.push(item);
                            accepted_this_iteration += 1;
                        }
                    }
                    Err(reason) => {
                        budget_stop = Some(reason);
                        break;
                    }
                }
            }
            ledger.record_evidence(accepted_this_iteration);
            if let Some(reason) = budget_stop {
                break reason;
            }

            let current = evaluator.evaluate(&evaluation_request, &self.policy, &evidence);
            trace.record(TraceEvent::Evaluation {
                decision: current.decision,
                reason: current.reason.clone(),
                mean_relevance: current.mean_relevance,
            });
            let next_queries = current.next_queries.clone();
            let satisfied = current.is_sufficient();
            let authoritative = current.decision == SufficiencyDecision::Authoritative;
            assessment = Some(current);

            if satisfied {
                break if authoritative {
                    StopReason::AuthoritativeAnswerFound
                } else {
                    StopReason::SufficientEvidence
                };
            }
            if let Some(reason) = ledger.exhausted() {
                break reason;
            }

            planned = planner.plan_followup(&next_queries, &known_symbols, &sources);
            if planned.is_empty() {
                trace.record(TraceEvent::QuerySuppressed {
                    query: request.question.clone(),
                    reason: "every follow-up query had already been issued".to_string(),
                });
                break StopReason::NoNewEvidence {
                    unproductive_iterations: ledger.unproductive_iterations().max(1),
                };
            }
        };

        trace.finish(stop_reason.clone());
        // Deduplicate before assembling: the loop admits anything with a new
        // content hash, but two adapters quoting the same lines are one fact,
        // and the package is what the agent pays context for.
        let (evidence, _) = evaluator.deduplicate(&evidence);
        let (package, package_error) = self.assemble(
            &retrieval_id,
            request,
            &evidence,
            &assessment,
            &stop_reason,
            &mut trace,
        );

        Ok(RetrievalOutcome {
            retrieval_id,
            stop_reason,
            package,
            assessment,
            trace,
            ledger,
            package_error,
        })
    }

    /// Sources for this role, priority order first, then the rest.
    fn ordered_sources(&self) -> Vec<SourceKind> {
        let mut available = self.registry.kinds();
        available.sort_by_key(|kind| (self.policy.source_rank(*kind), kind.as_str()));
        available
    }

    fn run_query(
        &mut self,
        query: &PlannedQuery,
        request: &RetrievalRequest,
        ledger: &mut BudgetLedger,
        trace: &mut RetrievalTrace,
        evaluator: &mut EvidenceEvaluator,
    ) -> Result<Vec<Evidence>, StopReason> {
        let mut found = Vec::new();
        for source in &query.sources {
            if !self.registry.contains(*source) {
                continue;
            }
            let limit = ledger.evidence_room().min(u32::from(u8::MAX));
            if limit == 0 {
                break;
            }
            let search = SearchRequest::new(
                query.query.clone(),
                query.mode,
                request.scope.clone(),
                limit,
            );
            let key = CacheKey::for_request(CacheClass::Query, *source, &search);

            if let Some(cached) = self.cache_get(&key, &request.scope) {
                trace.record(TraceEvent::CacheLookup {
                    key: key.as_string(),
                    hit: true,
                });
                found.extend(cached);
                continue;
            }
            trace.record(TraceEvent::CacheLookup {
                key: key.as_string(),
                hit: false,
            });

            ledger.charge_query(source.is_external())?;
            match self.registry.search(*source, &search) {
                Ok(result) => {
                    trace.record(TraceEvent::Query {
                        query: query.query.clone(),
                        mode: query.mode,
                        source: *source,
                        results: result.evidence.len(),
                        truncated: result.truncated,
                    });
                    let usable = self.filter_usable(result.evidence, *source, trace, evaluator);
                    self.cache_store(&key, &usable, &request.scope);
                    found.extend(usable);
                }
                Err(detail) => {
                    // A failing source degrades the retrieval; it does not end
                    // it. Another source may still answer, and the trace keeps
                    // the failure visible either way (section 41).
                    trace.record(TraceEvent::SourceFailure {
                        source: *source,
                        detail,
                    });
                }
            }
        }
        Ok(found)
    }

    /// Drop stale evidence and quarantine likely injections.
    fn filter_usable(
        &self,
        evidence: Vec<Evidence>,
        source: SourceKind,
        trace: &mut RetrievalTrace,
        evaluator: &mut EvidenceEvaluator,
    ) -> Vec<Evidence> {
        let (safe, quarantined) = injection::partition(&evidence);
        for (item, finding) in quarantined {
            trace.record(TraceEvent::EvidenceQuarantined {
                evidence_id: item.id().to_string(),
                markers: finding.markers,
            });
        }
        let mut usable = Vec::new();
        for item in safe {
            let input = FreshnessInput {
                captured_revision: item.scope().revision().to_string(),
                current_revision: self.current_revision.clone(),
                retrieved_at: item.retrieved_at(),
                now: self.now,
                superseded: false,
            };
            if self.freshness.assess(source, &input) == Freshness::Stale {
                // Dropped, and said so: section 54 asks why AutoSpec believed
                // something, and the evidence it decided not to believe is part
                // of that answer.
                evaluator.mark_stale(item.id());
                trace.record(TraceEvent::EvidenceStale {
                    evidence_id: item.id().to_string(),
                    source,
                    reason: format!(
                        "captured at {}, current revision is {}",
                        item.scope().revision(),
                        self.current_revision
                    ),
                });
                continue;
            }
            usable.push(item);
        }
        usable
    }

    fn cache_get(&mut self, key: &CacheKey, scope: &RetrievalScope) -> Option<Vec<Evidence>> {
        self.cache
            .as_mut()
            .and_then(|cache| cache.get(key, scope))
    }

    fn cache_store(&mut self, key: &CacheKey, evidence: &[Evidence], scope: &RetrievalScope) {
        if evidence.is_empty() {
            return;
        }
        if let Some(cache) = self.cache.as_mut() {
            cache.store(
                key,
                CacheEntry {
                    evidence: evidence.to_vec(),
                    scope: scope.clone(),
                    stored_at: self.now,
                },
            );
        }
    }

    /// Assemble the context package.
    ///
    /// Evidence that covers a required aspect is `required`; the rest is
    /// `supporting`, which is what the budget drops from first. When nothing
    /// was declared required, the highest-authority item carries the answer and
    /// becomes required so the package never returns a droppable core.
    fn assemble(
        &self,
        retrieval_id: &str,
        request: &RetrievalRequest,
        evidence: &[Evidence],
        assessment: &Option<EvidenceAssessment>,
        stop_reason: &StopReason,
        trace: &mut RetrievalTrace,
    ) -> (Option<ContextPackage>, Option<String>) {
        let mut builder = ContextPackageBuilder::new(
            format!("ctx_{retrieval_id}"),
            request.task_id.clone(),
            request.role,
            self.policy.max_context_tokens(),
        );
        if let Some(assessment) = assessment {
            builder = builder.summary_line(assessment.reason.clone());
            for aspect in &assessment.uncovered_aspects {
                builder = builder.unresolved(format!("no evidence found for: {aspect}"));
            }
            for query in &assessment.next_queries {
                builder = builder.next_action(format!("retrieve: {query}"));
            }
        }
        if !stop_reason.is_satisfied() {
            builder = builder.unresolved(stop_reason.describe());
        }

        let required_indices = required_indices(evidence, &request.required_aspects);
        for (index, item) in evidence.iter().enumerate() {
            builder = if required_indices.contains(&index) {
                builder.required(item.clone())
            } else {
                builder.supporting(item.clone())
            };
        }
        builder = builder.contradictions(ContradictionSet::new());

        match builder.build(stop_reason.clone()) {
            Ok(package) => (Some(package), None),
            Err(error) => {
                trace.record(TraceEvent::QuerySuppressed {
                    query: request.question.clone(),
                    reason: format!("context package assembly failed: {error}"),
                });
                (None, Some(error))
            }
        }
    }
}

fn required_indices(evidence: &[Evidence], aspects: &[String]) -> Vec<usize> {
    if evidence.is_empty() {
        return Vec::new();
    }
    if aspects.is_empty() {
        let best = evidence
            .iter()
            .enumerate()
            .max_by_key(|(index, item)| (item.authority(), item.relevance(), usize::MAX - index))
            .map(|(index, _)| index);
        return best.into_iter().collect();
    }
    let mut indices = Vec::new();
    for aspect in aspects {
        let needle = aspect.to_lowercase();
        if let Some((index, _)) = evidence.iter().enumerate().find(|(index, item)| {
            !indices.contains(index)
                && (item.content().to_lowercase().contains(&needle)
                    || item.location().citation().to_lowercase().contains(&needle))
        }) {
            indices.push(index);
        }
    }
    indices
}
