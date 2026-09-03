//! Retrieval traces, metrics and query reformulation
//! (spec sections 13, 35, 37, 41.1, 41.2, 54).

mod rag_support;

use autospec_core::rag::authority::SourceAuthority;
use autospec_core::rag::budget::{BudgetLedger, RetrievalBudget, StopReason};
use autospec_core::rag::coordinator::{RetrievalCoordinator, RetrievalRequest};
use autospec_core::rag::evaluator::SufficiencyDecision;
use autospec_core::rag::metrics::{outcome_label, RetrievalMetrics};
use autospec_core::rag::policy::AgentRole;
use autospec_core::rag::query::{extract_symbols, QueryPlanner};
use autospec_core::rag::score::Score;
use autospec_core::rag::source::{KnowledgeSource, SearchMode, SourceKind, SourceRegistry};
use autospec_core::rag::trace::{RetrievalTrace, TraceEvent};
use rag_support::{evidence, scope, StaticSource, NOW, REVISION};

#[test]
fn symbol_shaped_terms_are_extracted_and_english_words_are_not() {
    let symbols = extract_symbols("how does RoutePlanner.select use node_registry capacity?");

    assert!(symbols.contains(&"RoutePlanner.select".to_string()));
    assert!(symbols.contains(&"node_registry".to_string()));
    assert!(
        !symbols.contains(&"capacity".to_string()),
        "a plain lowercase word is not a symbol lookup"
    );
}

#[test]
fn the_opening_round_pairs_a_semantic_search_with_a_symbol_lookup() {
    let mut planner = QueryPlanner::new();

    let planned = planner.plan_initial(
        "how does RoutePlanner choose a node?",
        &[SourceKind::Repository, SourceKind::Specification],
    );

    assert!(planned
        .iter()
        .any(|query| query.mode == SearchMode::Semantic));
    assert!(planned
        .iter()
        .any(|query| query.mode == SearchMode::SymbolDefinition));
}

#[test]
fn an_evaluator_request_is_rewritten_into_the_right_lookup_shape() {
    let mut planner = QueryPlanner::new();

    let planned = planner.plan_followup(
        &[
            "implementations of Scheduler".to_string(),
            "callers of Scheduler.select_node".to_string(),
        ],
        &[],
        &[SourceKind::Repository],
    );

    assert_eq!(planned[0].mode, SearchMode::Implementations);
    assert_eq!(planned[0].query, "Scheduler");
    assert_eq!(planned[1].mode, SearchMode::SymbolReferences);
    assert_eq!(planned[1].query, "Scheduler.select_node");
}

#[test]
fn a_known_symbol_is_expanded_into_implementations_callers_and_tests() {
    let mut planner = QueryPlanner::new();

    let planned = planner.plan_followup(
        &[],
        &["FairShareScheduler".to_string()],
        &[SourceKind::Repository, SourceKind::Test],
    );

    let modes = planned.iter().map(|query| query.mode).collect::<Vec<_>>();
    assert!(modes.contains(&SearchMode::Implementations));
    assert!(modes.contains(&SearchMode::SymbolReferences));
    assert!(modes.contains(&SearchMode::Tests));
}

#[test]
fn a_query_already_issued_is_never_planned_again() {
    let mut planner = QueryPlanner::new();
    let sources = [SourceKind::Repository];
    planner.plan_followup(&["implementations of Scheduler".to_string()], &[], &sources);

    let repeat =
        planner.plan_followup(&["implementations of Scheduler".to_string()], &[], &sources);

    assert!(repeat.is_empty(), "a repeated query must be suppressed");
}

#[test]
fn a_symbol_lookup_is_routed_only_to_code_bearing_sources() {
    let mut planner = QueryPlanner::new();

    let planned = planner.plan_followup(
        &["implementations of Scheduler".to_string()],
        &[],
        &[
            SourceKind::Repository,
            SourceKind::Test,
            SourceKind::Documentation,
        ],
    );

    assert!(!planned[0].sources.contains(&SourceKind::Documentation));
    assert!(planned[0].sources.contains(&SourceKind::Repository));
}

#[test]
fn a_trace_records_every_iteration_in_order() {
    let mut trace = RetrievalTrace::new("rag_1", "AS-482", "planner", "scheduler capacity");
    trace.begin_iteration(1);
    trace.record(TraceEvent::Query {
        query: "scheduler capacity".to_string(),
        mode: SearchMode::Semantic,
        source: SourceKind::Specification,
        results: 2,
        truncated: false,
    });
    trace.begin_iteration(2);
    trace.record(TraceEvent::GraphTraversal {
        origin: "Scheduler".to_string(),
        depth: 2,
        reached: 3,
    });
    trace.finish(StopReason::SufficientEvidence);

    let rendered = trace.render();

    assert!(rendered.contains("Iteration 1"));
    assert!(rendered.contains("Iteration 2"));
    assert!(rendered.contains("traverse Scheduler depth 2 -> 3 node(s)"));
    assert!(rendered.contains("stopped: evidence sufficiency threshold reached"));
}

#[test]
fn an_event_before_any_iteration_still_lands_in_the_trace() {
    let mut trace = RetrievalTrace::new("rag_1", "AS-1", "planner", "q");

    trace.record(TraceEvent::CacheLookup {
        key: "query/repository/autospec@9a223af/x".to_string(),
        hit: true,
    });

    assert_eq!(trace.event_count(), 1);
    assert_eq!(trace.iterations()[0].number, 1);
}

#[test]
fn an_unfinished_trace_says_so_rather_than_claiming_a_reason() {
    let trace = RetrievalTrace::new("rag_1", "AS-1", "planner", "q");

    assert!(trace.stop_reason().is_none());
    assert!(trace.render().contains("stopped: still running"));
}

#[test]
fn metrics_are_derived_from_the_trace_and_the_package() {
    let mut registry = SourceRegistry::new();
    let source: Box<dyn KnowledgeSource> = Box::new(StaticSource::semantic(
        SourceKind::Specification,
        vec![evidence(
            "ev_1",
            SourceKind::Specification,
            "docs/specs/routing.md",
            "the scheduler picks a node",
            SourceAuthority::AcceptedSpecification,
            900,
        )],
    ));
    registry.register(source).expect("registers");
    let mut coordinator = RetrievalCoordinator::new(&registry, AgentRole::Planner, NOW, REVISION);
    let request = RetrievalRequest::new(
        "AS-1",
        AgentRole::Planner,
        "how does the scheduler work?",
        scope(),
    )
    .requiring(["scheduler".to_string()]);
    let outcome = coordinator.retrieve("rag_1", &request).expect("loop runs");

    let metrics =
        RetrievalMetrics::from_execution(&outcome.trace, &outcome.ledger, outcome.package.as_ref());

    assert_eq!(metrics.iterations, 1);
    assert_eq!(metrics.source_queries, 1);
    assert_eq!(metrics.evidence_items, 1);
    assert!(metrics.context_tokens > 0);
    assert!(metrics.context_tokens_saved > 0);
}

#[test]
fn metrics_render_as_sorted_prometheus_style_lines() {
    let trace = RetrievalTrace::new("rag_1", "AS-1", "planner", "q");
    let ledger = BudgetLedger::new(RetrievalBudget::default()).expect("valid budget");

    let rendered = RetrievalMetrics::from_execution(&trace, &ledger, None).render();

    let lines = rendered.lines().collect::<Vec<_>>();
    let mut sorted = lines.clone();
    sorted.sort();
    assert_eq!(lines, sorted, "metric lines are emitted in a stable order");
    assert!(rendered.contains("rag_iterations_total 0"));
}

#[test]
fn insufficient_evaluations_are_counted() {
    let mut trace = RetrievalTrace::new("rag_1", "AS-1", "planner", "q");
    trace.record(TraceEvent::Evaluation {
        decision: SufficiencyDecision::Insufficient,
        reason: "missing callers".to_string(),
        mean_relevance: Score::from_permille(500),
    });
    trace.record(TraceEvent::Evaluation {
        decision: SufficiencyDecision::Sufficient,
        reason: "covered".to_string(),
        mean_relevance: Score::from_permille(900),
    });
    let ledger = BudgetLedger::new(RetrievalBudget::default()).expect("valid budget");

    let metrics = RetrievalMetrics::from_execution(&trace, &ledger, None);

    assert_eq!(metrics.insufficient_evidence, 1);
}

#[test]
fn merged_metrics_accumulate_across_retrievals() {
    let mut trace = RetrievalTrace::new("rag_1", "AS-1", "planner", "q");
    trace.record(TraceEvent::CacheLookup {
        key: "k".to_string(),
        hit: true,
    });
    let ledger = BudgetLedger::new(RetrievalBudget::default()).expect("valid budget");
    let single = RetrievalMetrics::from_execution(&trace, &ledger, None);
    let mut total = single;

    total.merge(&single);

    assert_eq!(total.cache_hits, 2);
    assert_eq!(total.cache_hit_ratio_permille(), 1_000);
}

#[test]
fn the_outcome_label_distinguishes_satisfaction_from_every_other_exit() {
    assert_eq!(outcome_label(&StopReason::SufficientEvidence), "satisfied");
    assert_eq!(
        outcome_label(&StopReason::AuthoritativeAnswerFound),
        "satisfied"
    );
    assert_eq!(outcome_label(&StopReason::Cancelled), "cancelled");
    assert_eq!(
        outcome_label(&StopReason::NoNewEvidence {
            unproductive_iterations: 2
        }),
        "no_new_evidence"
    );
}
