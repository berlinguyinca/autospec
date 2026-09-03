//! Evidence evaluation and sufficiency (spec section 12).
//!
//! The rule this module exists to enforce is section 12's last line: a high
//! similarity score is not, by itself, a reason to accept a retrieval result.
//! Sufficiency here is a conjunction — enough distinct items, from a high
//! enough authority, fresh, and covering every aspect the caller asked about —
//! and any one of those failing produces a concrete `next_queries` list rather
//! than a bare "insufficient".

use crate::rag::authority::{AuthorityLadder, SourceAuthority};
use crate::rag::evidence::Evidence;
use crate::rag::policy::RetrievalPolicy;
use crate::rag::score::Score;

/// The evaluator's verdict on a set of evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SufficiencyDecision {
    /// Enough evidence; the loop may stop.
    Sufficient,
    /// An item at the top of the ladder answered the question outright.
    Authoritative,
    /// More retrieval is needed.
    Insufficient,
}

impl SufficiencyDecision {
    /// Stable wire identifier.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sufficient => "sufficient",
            Self::Authoritative => "authoritative",
            Self::Insufficient => "insufficient",
        }
    }

    /// Return `true` when retrieval may stop on this verdict.
    pub const fn is_sufficient(self) -> bool {
        matches!(self, Self::Sufficient | Self::Authoritative)
    }
}

/// A full assessment of the evidence gathered so far.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceAssessment {
    /// The verdict.
    pub decision: SufficiencyDecision,
    /// Why, in one sentence.
    pub reason: String,
    /// Concrete follow-up queries when insufficient (spec section 12).
    pub next_queries: Vec<String>,
    /// Mean relevance across the accepted evidence.
    pub mean_relevance: Score,
    /// Best authority class present.
    pub best_authority: Option<SourceAuthority>,
    /// Aspects of the question with no supporting evidence.
    pub uncovered_aspects: Vec<String>,
    /// Evidence ids dropped as duplicates.
    pub duplicates: Vec<String>,
    /// Evidence ids dropped as stale.
    pub stale: Vec<String>,
}

impl EvidenceAssessment {
    /// Return `true` when retrieval may stop.
    pub fn is_sufficient(&self) -> bool {
        self.decision.is_sufficient()
    }
}

/// What the caller wants covered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvaluationRequest {
    /// The question as asked.
    pub question: String,
    /// Aspects that must each be supported by at least one item.
    ///
    /// Named explicitly rather than inferred, because "did we cover the
    /// callers as well as the definition" is the difference between section
    /// 12's worked `insufficient` example and a premature stop.
    pub required_aspects: Vec<String>,
    /// Minimum distinct items before the evidence can be called sufficient.
    pub min_evidence_items: usize,
}

impl EvaluationRequest {
    /// A request with no aspect requirements.
    pub fn new(question: impl Into<String>) -> Self {
        Self {
            question: question.into(),
            required_aspects: Vec::new(),
            min_evidence_items: 1,
        }
    }

    /// Require that each aspect be covered.
    pub fn requiring(mut self, aspects: impl IntoIterator<Item = String>) -> Self {
        self.required_aspects = aspects.into_iter().collect();
        self
    }

    /// Require a minimum number of distinct items.
    pub fn with_min_items(mut self, minimum: usize) -> Self {
        self.min_evidence_items = minimum;
        self
    }
}

/// Deterministic evidence evaluator.
///
/// Section 12 permits a low-cost model here. This implementation is the
/// deterministic floor the model call sits above: it always runs, it costs no
/// tokens, and a model that disagrees with it can only tighten the verdict via
/// [`EvidenceEvaluator::narrow_with`], never loosen one.
#[derive(Debug, Clone, Default)]
pub struct EvidenceEvaluator {
    ladder: AuthorityLadder,
    stale_ids: Vec<String>,
}

impl EvidenceEvaluator {
    /// An evaluator using the default authority ladder.
    pub fn new() -> Self {
        Self::default()
    }

    /// An evaluator using a project-specific ladder.
    pub fn with_ladder(ladder: AuthorityLadder) -> Self {
        Self {
            ladder,
            stale_ids: Vec::new(),
        }
    }

    /// Mark evidence the freshness policy rejected.
    pub fn mark_stale(&mut self, evidence_id: impl Into<String>) {
        let id = evidence_id.into();
        if !self.stale_ids.contains(&id) {
            self.stale_ids.push(id);
        }
    }

    /// Drop duplicates, returning the retained evidence and the dropped ids.
    ///
    /// When two items are duplicates the higher-authority one is kept: the same
    /// text quoted from the specification and from a blog post is one fact, and
    /// the citation the caller sees should be the specification.
    pub fn deduplicate(&self, evidence: &[Evidence]) -> (Vec<Evidence>, Vec<String>) {
        let mut retained: Vec<Evidence> = Vec::new();
        let mut dropped = Vec::new();
        for item in evidence {
            match retained
                .iter()
                .position(|held| held.duplicates(item) || same_fact(held, item))
            {
                None => retained.push(item.clone()),
                Some(index) => {
                    if self
                        .ladder
                        .outranks(item.authority(), retained[index].authority())
                    {
                        dropped.push(retained[index].id().to_string());
                        retained[index] = item.clone();
                    } else {
                        dropped.push(item.id().to_string());
                    }
                }
            }
        }
        (retained, dropped)
    }

    /// Assess the evidence gathered so far against a role policy.
    pub fn evaluate(
        &self,
        request: &EvaluationRequest,
        policy: &RetrievalPolicy,
        evidence: &[Evidence],
    ) -> EvidenceAssessment {
        let (retained, duplicates) = self.deduplicate(evidence);
        let (fresh, stale): (Vec<Evidence>, Vec<Evidence>) = retained
            .into_iter()
            .partition(|item| !self.stale_ids.iter().any(|id| id == item.id()));
        let stale_ids = stale.iter().map(|item| item.id().to_string()).collect();

        let relevances = fresh.iter().map(Evidence::relevance).collect::<Vec<_>>();
        let mean_relevance = Score::mean(&relevances);
        let best_authority = fresh.iter().map(Evidence::authority).max();
        let uncovered = uncovered_aspects(&request.required_aspects, &fresh);

        let mut next_queries = Vec::new();
        for aspect in &uncovered {
            next_queries.push(aspect.clone());
        }

        if fresh.is_empty() {
            return EvidenceAssessment {
                decision: SufficiencyDecision::Insufficient,
                reason: "no usable evidence was retrieved".to_string(),
                next_queries: if next_queries.is_empty() {
                    vec![request.question.clone()]
                } else {
                    next_queries
                },
                mean_relevance,
                best_authority,
                uncovered_aspects: uncovered,
                duplicates,
                stale: stale_ids,
            };
        }

        // An explicit user requirement or the accepted specification answering
        // the question outright ends the loop: nothing further down the ladder
        // can overturn it, so more retrieval only spends budget.
        let authoritative = best_authority.is_some_and(|authority| {
            self.ladder.rank(authority) == 0
                && uncovered.is_empty()
                && mean_relevance.at_least(policy.sufficiency_threshold())
        });
        if authoritative {
            return EvidenceAssessment {
                decision: SufficiencyDecision::Authoritative,
                reason: "the highest-authority source answered the question".to_string(),
                next_queries: Vec::new(),
                mean_relevance,
                best_authority,
                uncovered_aspects: uncovered,
                duplicates,
                stale: stale_ids,
            };
        }

        if fresh.len() < request.min_evidence_items {
            let reason = format!(
                "{} distinct item(s) retrieved, {} required",
                fresh.len(),
                request.min_evidence_items
            );
            if next_queries.is_empty() {
                next_queries.push(request.question.clone());
            }
            return EvidenceAssessment {
                decision: SufficiencyDecision::Insufficient,
                reason,
                next_queries,
                mean_relevance,
                best_authority,
                uncovered_aspects: uncovered,
                duplicates,
                stale: stale_ids,
            };
        }

        if !uncovered.is_empty() {
            return EvidenceAssessment {
                decision: SufficiencyDecision::Insufficient,
                reason: format!("no evidence covers: {}", uncovered.join(", ")),
                next_queries,
                mean_relevance,
                best_authority,
                uncovered_aspects: uncovered,
                duplicates,
                stale: stale_ids,
            };
        }

        if !mean_relevance.at_least(policy.sufficiency_threshold()) {
            return EvidenceAssessment {
                decision: SufficiencyDecision::Insufficient,
                reason: format!(
                    "mean relevance {} is below the {} threshold for role {}",
                    mean_relevance,
                    policy.sufficiency_threshold(),
                    policy.role().as_str()
                ),
                next_queries: vec![request.question.clone()],
                mean_relevance,
                best_authority,
                uncovered_aspects: uncovered,
                duplicates,
                stale: stale_ids,
            };
        }

        EvidenceAssessment {
            decision: SufficiencyDecision::Sufficient,
            reason: format!(
                "{} item(s) cover every required aspect at mean relevance {mean_relevance}",
                fresh.len()
            ),
            next_queries: Vec::new(),
            mean_relevance,
            best_authority,
            uncovered_aspects: uncovered,
            duplicates,
            stale: stale_ids,
        }
    }

    /// Fold a model evaluator's verdict into a deterministic one.
    ///
    /// The model may only downgrade `Sufficient` to `Insufficient` and add
    /// follow-up queries. It cannot promote insufficient evidence to
    /// sufficient, which is section 12's rule against accepting a result
    /// because a score looked high.
    pub fn narrow_with(
        base: EvidenceAssessment,
        model_decision: SufficiencyDecision,
        model_reason: impl Into<String>,
        model_next_queries: Vec<String>,
    ) -> EvidenceAssessment {
        if model_decision.is_sufficient() {
            return base;
        }
        let mut narrowed = base;
        narrowed.decision = SufficiencyDecision::Insufficient;
        narrowed.reason = model_reason.into();
        for query in model_next_queries {
            if !narrowed.next_queries.contains(&query) {
                narrowed.next_queries.push(query);
            }
        }
        narrowed
    }
}

/// Two items assert the same fact when a summary and its source cover the same
/// location at the same source state.
fn same_fact(left: &Evidence, right: &Evidence) -> bool {
    left.location().citation() == right.location().citation()
        && left.scope().cache_fragment() == right.scope().cache_fragment()
        && left.form() == right.form()
}

fn uncovered_aspects(required: &[String], evidence: &[Evidence]) -> Vec<String> {
    required
        .iter()
        .filter(|aspect| {
            let needle = aspect.to_lowercase();
            !evidence.iter().any(|item| {
                item.content().to_lowercase().contains(&needle)
                    || item.location().citation().to_lowercase().contains(&needle)
                    || item
                        .location()
                        .symbol
                        .as_deref()
                        .is_some_and(|symbol| symbol.to_lowercase().contains(&needle))
            })
        })
        .cloned()
        .collect()
}
