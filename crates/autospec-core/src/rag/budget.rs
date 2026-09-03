//! Retrieval budgets and stopping rules (spec sections 39, 40, 41.1).
//!
//! Section 40 requires the result to state *why* retrieval stopped, so the
//! ledger records the first limit that was actually reached rather than
//! reporting a generic exhaustion. Wall-clock is supplied by the caller: the
//! core is pure, and a clock read here would make every budget test
//! time-dependent.

/// Configured ceilings for one retrieval (spec section 39).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetrievalBudget {
    /// Maximum agentic loop iterations.
    pub max_iterations: u32,
    /// Maximum source queries across all iterations.
    pub max_queries: u32,
    /// Maximum queries that leave the local installation.
    pub max_external_queries: u32,
    /// Maximum evidence items retained.
    pub max_evidence_items: u32,
    /// Maximum tokens in the returned context package.
    pub max_context_tokens: u32,
    /// Maximum model tokens spent on planning, evaluation and summarization.
    pub max_model_tokens: u32,
    /// Maximum elapsed wall-clock seconds.
    pub max_wall_clock_seconds: u32,
    /// Iterations without new evidence before the loop gives up (section 41.1's
    /// novelty guard).
    pub max_unproductive_iterations: u32,
}

impl Default for RetrievalBudget {
    /// The `rag:` defaults from specification section 39.
    fn default() -> Self {
        Self {
            max_iterations: 10,
            max_queries: 30,
            max_external_queries: 5,
            max_evidence_items: 100,
            max_context_tokens: 50_000,
            max_model_tokens: 100_000,
            max_wall_clock_seconds: 120,
            max_unproductive_iterations: 2,
        }
    }
}

impl RetrievalBudget {
    /// Reject a budget that cannot run a single useful iteration.
    ///
    /// A zero here does not throttle retrieval, it produces a retrieval that
    /// returns "budget exhausted" before asking anything — a silent no-op the
    /// caller would read as "no evidence exists".
    pub fn validate(&self) -> Result<(), String> {
        let zero_fields = [
            ("max_iterations", self.max_iterations),
            ("max_queries", self.max_queries),
            ("max_evidence_items", self.max_evidence_items),
            ("max_context_tokens", self.max_context_tokens),
            ("max_wall_clock_seconds", self.max_wall_clock_seconds),
            (
                "max_unproductive_iterations",
                self.max_unproductive_iterations,
            ),
        ];
        for (name, value) in zero_fields {
            if value == 0 {
                return Err(format!("{name} must be greater than zero"));
            }
        }
        if self.max_queries < self.max_external_queries {
            return Err(format!(
                "max_external_queries ({}) exceeds max_queries ({})",
                self.max_external_queries, self.max_queries
            ));
        }
        Ok(())
    }

    /// Narrow this budget to the tighter of each limit.
    ///
    /// Role and per-project overrides (section 39) may only tighten: a role
    /// policy must not be able to spend past the administrator's ceiling.
    pub fn tighten(&self, other: &Self) -> Self {
        Self {
            max_iterations: self.max_iterations.min(other.max_iterations),
            max_queries: self.max_queries.min(other.max_queries),
            max_external_queries: self.max_external_queries.min(other.max_external_queries),
            max_evidence_items: self.max_evidence_items.min(other.max_evidence_items),
            max_context_tokens: self.max_context_tokens.min(other.max_context_tokens),
            max_model_tokens: self.max_model_tokens.min(other.max_model_tokens),
            max_wall_clock_seconds: self
                .max_wall_clock_seconds
                .min(other.max_wall_clock_seconds),
            max_unproductive_iterations: self
                .max_unproductive_iterations
                .min(other.max_unproductive_iterations),
        }
    }
}

/// Why a retrieval loop stopped (spec section 40).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    /// The evaluator judged the evidence sufficient.
    SufficientEvidence,
    /// An item at the top of the authority ladder answered the question.
    AuthoritativeAnswerFound,
    /// A named budget limit was reached.
    BudgetExhausted(BudgetLimit),
    /// Iterations produced nothing new.
    NoNewEvidence { unproductive_iterations: u32 },
    /// The caller cancelled.
    Cancelled,
    /// A blocking contradiction needs project-level resolution.
    BlockingContradiction { contradiction_id: String },
    /// Security policy denied a retrieval the loop required.
    SecurityPolicy { detail: String },
    /// A source failed in a way the loop could not route around.
    RetrievalFailure { detail: String },
}

impl StopReason {
    /// Stable wire identifier.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SufficientEvidence => "sufficient_evidence",
            Self::AuthoritativeAnswerFound => "authoritative_answer_found",
            Self::BudgetExhausted(_) => "budget_exhausted",
            Self::NoNewEvidence { .. } => "no_new_evidence",
            Self::Cancelled => "cancelled",
            Self::BlockingContradiction { .. } => "blocking_contradiction",
            Self::SecurityPolicy { .. } => "security_policy",
            Self::RetrievalFailure { .. } => "retrieval_failure",
        }
    }

    /// Return `true` when the loop gathered what it was asked for.
    ///
    /// Everything else is a partial answer, and the caller must be able to tell
    /// the two apart before acting on the package.
    pub fn is_satisfied(&self) -> bool {
        matches!(
            self,
            Self::SufficientEvidence | Self::AuthoritativeAnswerFound
        )
    }

    /// A sentence for the trace and the returned package.
    pub fn describe(&self) -> String {
        match self {
            Self::SufficientEvidence => "evidence sufficiency threshold reached".to_string(),
            Self::AuthoritativeAnswerFound => {
                "an authoritative source answered the question".to_string()
            }
            Self::BudgetExhausted(limit) => format!("budget exhausted: {}", limit.as_str()),
            Self::NoNewEvidence {
                unproductive_iterations,
            } => format!("no new evidence after {unproductive_iterations} iteration(s)"),
            Self::Cancelled => "task cancelled".to_string(),
            Self::BlockingContradiction { contradiction_id } => {
                format!("blocking contradiction {contradiction_id} requires resolution")
            }
            Self::SecurityPolicy { detail } => {
                format!("security policy prevented required retrieval: {detail}")
            }
            Self::RetrievalFailure { detail } => format!("retrieval failed: {detail}"),
        }
    }
}

/// Which ceiling a [`StopReason::BudgetExhausted`] hit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetLimit {
    /// `max_iterations`.
    Iterations,
    /// `max_queries`.
    Queries,
    /// `max_external_queries`.
    ExternalQueries,
    /// `max_evidence_items`.
    EvidenceItems,
    /// `max_model_tokens`.
    ModelTokens,
    /// `max_wall_clock_seconds`.
    WallClock,
}

impl BudgetLimit {
    /// Stable wire identifier.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Iterations => "max_iterations",
            Self::Queries => "max_queries",
            Self::ExternalQueries => "max_external_queries",
            Self::EvidenceItems => "max_evidence_items",
            Self::ModelTokens => "max_model_tokens",
            Self::WallClock => "max_wall_clock_seconds",
        }
    }
}

/// Running spend against a [`RetrievalBudget`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BudgetLedger {
    budget: RetrievalBudget,
    iterations: u32,
    queries: u32,
    external_queries: u32,
    evidence_items: u32,
    model_tokens: u32,
    elapsed_seconds: u32,
    unproductive_iterations: u32,
}

impl BudgetLedger {
    /// Open a ledger against a validated budget.
    pub fn new(budget: RetrievalBudget) -> Result<Self, String> {
        budget.validate()?;
        Ok(Self {
            budget,
            iterations: 0,
            queries: 0,
            external_queries: 0,
            evidence_items: 0,
            model_tokens: 0,
            elapsed_seconds: 0,
            unproductive_iterations: 0,
        })
    }

    /// The budget being spent against.
    pub fn budget(&self) -> &RetrievalBudget {
        &self.budget
    }

    /// Iterations started.
    pub fn iterations(&self) -> u32 {
        self.iterations
    }

    /// Source queries issued.
    pub fn queries(&self) -> u32 {
        self.queries
    }

    /// External source queries issued.
    pub fn external_queries(&self) -> u32 {
        self.external_queries
    }

    /// Evidence items retained.
    pub fn evidence_items(&self) -> u32 {
        self.evidence_items
    }

    /// Model tokens spent on retrieval-side model calls.
    pub fn model_tokens(&self) -> u32 {
        self.model_tokens
    }

    /// Wall-clock seconds reported by the caller.
    pub fn elapsed_seconds(&self) -> u32 {
        self.elapsed_seconds
    }

    /// Consecutive iterations that produced no new evidence.
    pub fn unproductive_iterations(&self) -> u32 {
        self.unproductive_iterations
    }

    /// Report elapsed time. Time never runs backwards in the ledger, so a
    /// caller passing a stale reading cannot buy back a spent budget.
    pub fn observe_elapsed(&mut self, elapsed_seconds: u32) {
        self.elapsed_seconds = self.elapsed_seconds.max(elapsed_seconds);
    }

    /// Charge one iteration, or refuse when a limit is already reached.
    pub fn start_iteration(&mut self) -> Result<u32, StopReason> {
        if let Some(reason) = self.exhausted() {
            return Err(reason);
        }
        self.iterations += 1;
        Ok(self.iterations)
    }

    /// Charge one source query.
    pub fn charge_query(&mut self, external: bool) -> Result<(), StopReason> {
        if self.queries >= self.budget.max_queries {
            return Err(StopReason::BudgetExhausted(BudgetLimit::Queries));
        }
        if external && self.external_queries >= self.budget.max_external_queries {
            return Err(StopReason::BudgetExhausted(BudgetLimit::ExternalQueries));
        }
        self.queries += 1;
        if external {
            self.external_queries += 1;
        }
        Ok(())
    }

    /// Charge model tokens spent on planning, evaluation or summarization.
    pub fn charge_model_tokens(&mut self, tokens: u32) -> Result<(), StopReason> {
        let spent = self.model_tokens.saturating_add(tokens);
        if spent > self.budget.max_model_tokens {
            return Err(StopReason::BudgetExhausted(BudgetLimit::ModelTokens));
        }
        self.model_tokens = spent;
        Ok(())
    }

    /// Record how many new evidence items an iteration retained.
    ///
    /// Returns the accepted count, which is clipped at `max_evidence_items`;
    /// an iteration that overshoots keeps what fits rather than discarding the
    /// whole batch.
    pub fn record_evidence(&mut self, accepted: u32) -> u32 {
        let room = self
            .budget
            .max_evidence_items
            .saturating_sub(self.evidence_items);
        let kept = accepted.min(room);
        self.evidence_items += kept;
        if kept == 0 {
            self.unproductive_iterations += 1;
        } else {
            self.unproductive_iterations = 0;
        }
        kept
    }

    /// Remaining room for evidence items.
    pub fn evidence_room(&self) -> u32 {
        self.budget
            .max_evidence_items
            .saturating_sub(self.evidence_items)
    }

    /// The first limit reached, if any.
    ///
    /// Checked in the order a caller would want reported: the cheap structural
    /// limits first, wall-clock last, so a run that trips two at once names the
    /// one that actually governed it.
    pub fn exhausted(&self) -> Option<StopReason> {
        if self.iterations >= self.budget.max_iterations {
            return Some(StopReason::BudgetExhausted(BudgetLimit::Iterations));
        }
        if self.queries >= self.budget.max_queries {
            return Some(StopReason::BudgetExhausted(BudgetLimit::Queries));
        }
        if self.evidence_items >= self.budget.max_evidence_items {
            return Some(StopReason::BudgetExhausted(BudgetLimit::EvidenceItems));
        }
        if self.model_tokens >= self.budget.max_model_tokens {
            return Some(StopReason::BudgetExhausted(BudgetLimit::ModelTokens));
        }
        if self.elapsed_seconds >= self.budget.max_wall_clock_seconds {
            return Some(StopReason::BudgetExhausted(BudgetLimit::WallClock));
        }
        if self.unproductive_iterations >= self.budget.max_unproductive_iterations {
            return Some(StopReason::NoNewEvidence {
                unproductive_iterations: self.unproductive_iterations,
            });
        }
        None
    }
}
