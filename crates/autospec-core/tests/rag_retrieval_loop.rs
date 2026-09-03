//! The agentic retrieval loop, budgets and stopping rules
//! (spec sections 6, 12, 39, 40, 55.1, 55.4).

mod rag_support;

use autospec_core::rag::authority::SourceAuthority;
use autospec_core::rag::budget::{BudgetLimit, RetrievalBudget, StopReason};
use autospec_core::rag::coordinator::{RetrievalCoordinator, RetrievalRequest};
use autospec_core::rag::policy::AgentRole;
use autospec_core::rag::source::{SearchMode, SourceKind, SourceRegistry};
use rag_support::{
    CountingSource, FailingSource, NOW, REVISION, StaticSource, evidence, scope,
};

fn registry_with(sources: Vec<Box<dyn autospec_core::rag::source::KnowledgeSource>>) -> SourceRegistry {
    let mut registry = SourceRegistry::new();
    for source in sources {
        registry.register(source).expect("distinct source kinds");
    }
    registry
}

fn spec_evidence(id: &str, content: &str, relevance: u16) -> autospec_core::rag::evidence::Evidence {
    evidence(
        id,
        SourceKind::Specification,
        "docs/specs/routing.md",
        content,
        SourceAuthority::AcceptedSpecification,
        relevance,
    )
}

#[test]
fn retrieval_stops_once_the_evidence_covers_every_required_aspect() {
    let registry = registry_with(vec![Box::new(StaticSource::semantic(
        SourceKind::Specification,
        vec![
            spec_evidence("ev_1", "the scheduler selects a node by capacity", 900),
            spec_evidence("ev_2", "the registry reports available seats", 880),
        ],
    ))]);
    let mut coordinator =
        RetrievalCoordinator::new(&registry, AgentRole::Planner, NOW, REVISION);
    let request = RetrievalRequest::new(
        "AS-482",
        AgentRole::Planner,
        "how does the scheduler pick a node?",
        scope(),
    )
    .requiring(["scheduler".to_string(), "registry".to_string()]);

    let outcome = coordinator.retrieve("rag_1", &request).expect("loop runs");

    assert_eq!(outcome.stop_reason, StopReason::SufficientEvidence);
    assert_eq!(outcome.status(), "sufficient");
    assert!(outcome.package.is_some());
}

#[test]
fn insufficient_evidence_reformulates_rather_than_stopping() {
    // Only the scheduler aspect is covered, so the first evaluation must come
    // back insufficient and the loop must issue a second round.
    let registry = registry_with(vec![Box::new(StaticSource::semantic(
        SourceKind::Specification,
        vec![spec_evidence("ev_1", "the scheduler selects a node", 900)],
    ))]);
    let mut coordinator =
        RetrievalCoordinator::new(&registry, AgentRole::Planner, NOW, REVISION);
    let request = RetrievalRequest::new(
        "AS-482",
        AgentRole::Planner,
        "how does the scheduler pick a node?",
        scope(),
    )
    .requiring(["scheduler".to_string(), "telemetry".to_string()]);

    let outcome = coordinator.retrieve("rag_2", &request).expect("loop runs");

    assert!(!outcome.stop_reason.is_satisfied(), "{:?}", outcome.stop_reason);
    let assessment = outcome.assessment.expect("an assessment was produced");
    assert_eq!(assessment.uncovered_aspects, vec!["telemetry".to_string()]);
    assert!(outcome.trace.count_events("evaluation") >= 1);
}

#[test]
fn the_result_states_why_retrieval_stopped() {
    let registry = registry_with(vec![Box::new(StaticSource::semantic(
        SourceKind::Specification,
        vec![spec_evidence("ev_1", "unrelated content", 100)],
    ))]);
    let mut coordinator =
        RetrievalCoordinator::new(&registry, AgentRole::Planner, NOW, REVISION);
    let request = RetrievalRequest::new("AS-1", AgentRole::Planner, "anything", scope())
        .requiring(["absent".to_string()]);

    let outcome = coordinator.retrieve("rag_3", &request).expect("loop runs");

    assert!(!outcome.stop_reason.describe().is_empty());
    assert_eq!(
        outcome.trace.stop_reason(),
        Some(&outcome.stop_reason),
        "the trace records the same stop reason as the outcome"
    );
    let package = outcome.package.expect("a package is still returned");
    assert!(package.render().contains("stopped:"));
}

#[test]
fn the_iteration_budget_stops_the_loop() {
    let registry = registry_with(vec![Box::new(StaticSource::semantic(
        SourceKind::Specification,
        vec![spec_evidence("ev_1", "partial answer", 900)],
    ))]);
    let mut coordinator =
        RetrievalCoordinator::new(&registry, AgentRole::Planner, NOW, REVISION);
    let budget = RetrievalBudget {
        max_iterations: 1,
        max_unproductive_iterations: 9,
        ..RetrievalBudget::default()
    };
    let request = RetrievalRequest::new("AS-1", AgentRole::Planner, "unanswerable", scope())
        .requiring(["never_present".to_string()])
        .with_budget(budget);

    let outcome = coordinator.retrieve("rag_4", &request).expect("loop runs");

    assert_eq!(
        outcome.stop_reason,
        StopReason::BudgetExhausted(BudgetLimit::Iterations)
    );
    assert_eq!(outcome.ledger.iterations(), 1);
}

#[test]
fn the_query_budget_stops_the_loop() {
    let registry = registry_with(vec![Box::new(StaticSource::semantic(
        SourceKind::Specification,
        vec![spec_evidence("ev_1", "partial", 900)],
    ))]);
    let mut coordinator =
        RetrievalCoordinator::new(&registry, AgentRole::Planner, NOW, REVISION);
    let budget = RetrievalBudget {
        max_queries: 1,
        max_external_queries: 1,
        max_unproductive_iterations: 9,
        ..RetrievalBudget::default()
    };
    let request = RetrievalRequest::new("AS-1", AgentRole::Planner, "unanswerable", scope())
        .requiring(["never_present".to_string()])
        .with_budget(budget);

    let outcome = coordinator.retrieve("rag_5", &request).expect("loop runs");

    assert_eq!(
        outcome.stop_reason,
        StopReason::BudgetExhausted(BudgetLimit::Queries)
    );
    assert_eq!(outcome.ledger.queries(), 1);
}

#[test]
fn an_authoritative_answer_ends_the_loop_immediately() {
    let user_requirement = evidence(
        "ev_user",
        SourceKind::Specification,
        "task/requirement",
        "the gateway timeout must be 30 seconds",
        SourceAuthority::ExplicitUserRequirement,
        980,
    );
    let registry = registry_with(vec![Box::new(StaticSource::semantic(
        SourceKind::Specification,
        vec![user_requirement],
    ))]);
    let mut coordinator =
        RetrievalCoordinator::new(&registry, AgentRole::Planner, NOW, REVISION);
    let request = RetrievalRequest::new(
        "AS-1",
        AgentRole::Planner,
        "what is the gateway timeout?",
        scope(),
    )
    .requiring(["timeout".to_string()]);

    let outcome = coordinator.retrieve("rag_6", &request).expect("loop runs");

    assert_eq!(outcome.stop_reason, StopReason::AuthoritativeAnswerFound);
    assert_eq!(outcome.ledger.iterations(), 1);
}

#[test]
fn a_repeated_query_is_never_issued_twice() {
    let counting = Box::new(CountingSource::new(
        SourceKind::Specification,
        vec![spec_evidence("ev_1", "partial", 900)],
    ));
    let registry = registry_with(vec![counting]);
    let mut coordinator =
        RetrievalCoordinator::new(&registry, AgentRole::Planner, NOW, REVISION);
    let request = RetrievalRequest::new(
        "AS-1",
        AgentRole::Planner,
        "how does capacity work?",
        scope(),
    )
    .requiring(["never_present".to_string()]);

    let outcome = coordinator.retrieve("rag_7", &request).expect("loop runs");

    // Every distinct query is issued at most once, so total queries can never
    // exceed the planner's distinct-query count.
    let queries = outcome.trace.count_events("query");
    assert!(queries <= 4, "issued {queries} queries for one question");
    assert!(matches!(
        outcome.stop_reason,
        StopReason::NoNewEvidence { .. }
    ));
}

#[test]
fn a_failing_source_degrades_the_retrieval_without_ending_it() {
    let registry = registry_with(vec![
        Box::new(FailingSource::new(SourceKind::Repository)),
        Box::new(StaticSource::semantic(
            SourceKind::Specification,
            vec![spec_evidence("ev_1", "the scheduler picks a node", 900)],
        )),
    ]);
    let mut coordinator =
        RetrievalCoordinator::new(&registry, AgentRole::Planner, NOW, REVISION);
    let request = RetrievalRequest::new(
        "AS-1",
        AgentRole::Planner,
        "how does the scheduler work?",
        scope(),
    )
    .requiring(["scheduler".to_string()]);

    let outcome = coordinator.retrieve("rag_8", &request).expect("loop runs");

    assert_eq!(outcome.trace.count_events("source_failure"), 1);
    assert!(outcome.stop_reason.is_satisfied());
}

#[test]
fn the_evidence_item_budget_caps_what_the_loop_retains() {
    let many = (0..20)
        .map(|index| spec_evidence(&format!("ev_{index}"), &format!("fact {index}"), 900))
        .collect::<Vec<_>>();
    let registry = registry_with(vec![Box::new(StaticSource::semantic(
        SourceKind::Specification,
        many,
    ))]);
    let mut coordinator =
        RetrievalCoordinator::new(&registry, AgentRole::Planner, NOW, REVISION);
    let budget = RetrievalBudget {
        max_evidence_items: 3,
        ..RetrievalBudget::default()
    };
    let request = RetrievalRequest::new("AS-1", AgentRole::Planner, "facts", scope())
        .with_budget(budget);

    let outcome = coordinator.retrieve("rag_9", &request).expect("loop runs");

    assert!(outcome.ledger.evidence_items() <= 3);
    let package = outcome.package.expect("package assembled");
    assert!(package.all_evidence().len() <= 3);
}

#[test]
fn a_source_that_cannot_answer_a_mode_is_rejected_before_it_runs() {
    let registry = registry_with(vec![Box::new(StaticSource::new(
        SourceKind::Specification,
        vec![SearchMode::Semantic],
        vec![spec_evidence("ev_1", "content", 900)],
    ))]);
    let request = autospec_core::rag::source::SearchRequest::new(
        "Scheduler",
        SearchMode::Implementations,
        scope(),
        5,
    );

    let error = registry
        .search(SourceKind::Specification, &request)
        .expect_err("an unsupported mode must be refused");

    assert!(error.contains("does not support mode"), "{error}");
}

#[test]
fn a_budget_with_a_zero_limit_is_refused_rather_than_silently_returning_nothing() {
    let registry = registry_with(vec![Box::new(StaticSource::semantic(
        SourceKind::Specification,
        vec![spec_evidence("ev_1", "content", 900)],
    ))]);
    let mut coordinator =
        RetrievalCoordinator::new(&registry, AgentRole::Planner, NOW, REVISION);
    let budget = RetrievalBudget {
        max_iterations: 0,
        ..RetrievalBudget::default()
    };
    let request =
        RetrievalRequest::new("AS-1", AgentRole::Planner, "q", scope()).with_budget(budget);

    let error = coordinator
        .retrieve("rag_10", &request)
        .expect_err("a zero budget must be refused");

    assert!(error.contains("max_iterations"), "{error}");
}

#[test]
fn two_sources_quoting_the_same_lines_reach_the_agent_once() {
    // The same file and lines, surfaced by two adapters with different wording.
    // Both pass the content-hash admission check; only one should be paid for
    // in context.
    let location_uri = "docs/specs/routing.md";
    let registry = registry_with(vec![Box::new(StaticSource::semantic(
        SourceKind::Specification,
        vec![
            evidence(
                "ev_dup",
                SourceKind::Specification,
                location_uri,
                "the scheduler picks a node",
                SourceAuthority::AcceptedSpecification,
                900,
            ),
            evidence(
                "ev_dup",
                SourceKind::Documentation,
                location_uri,
                "the scheduler picks a node (restated)",
                SourceAuthority::OfficialDocumentation,
                880,
            ),
        ],
    ))]);
    let mut coordinator =
        RetrievalCoordinator::new(&registry, AgentRole::Planner, NOW, REVISION);
    let request = RetrievalRequest::new(
        "AS-1",
        AgentRole::Planner,
        "how does the scheduler work?",
        scope(),
    )
    .requiring(["scheduler".to_string()]);

    let outcome = coordinator.retrieve("rag_dup", &request).expect("loop runs");

    let package = outcome.package.expect("package assembled");
    assert_eq!(
        package.all_evidence().len(),
        1,
        "the same lines quoted twice are one fact"
    );
    assert_eq!(
        package.all_evidence()[0].authority(),
        SourceAuthority::AcceptedSpecification,
        "the higher-authority citation is the one kept"
    );
}

#[test]
fn the_wall_clock_budget_stops_the_loop_when_the_caller_supplies_a_clock() {
    let registry = registry_with(vec![Box::new(StaticSource::semantic(
        SourceKind::Specification,
        vec![spec_evidence("ev_1", "partial", 900)],
    ))]);
    let budget = RetrievalBudget {
        max_wall_clock_seconds: 5,
        max_unproductive_iterations: 9,
        ..RetrievalBudget::default()
    };
    let request = RetrievalRequest::new("AS-1", AgentRole::Planner, "unanswerable", scope())
        .requiring(["never_present".to_string()])
        .with_budget(budget);

    // A clock that has already passed the limit on the first reading.
    let mut coordinator =
        RetrievalCoordinator::new(&registry, AgentRole::Planner, NOW, REVISION)
            .with_elapsed(|| 9);
    let outcome = coordinator.retrieve("rag_clock", &request).expect("loop runs");

    assert_eq!(
        outcome.stop_reason,
        StopReason::BudgetExhausted(BudgetLimit::WallClock)
    );
    assert_eq!(outcome.ledger.iterations(), 0);
}

#[test]
fn without_a_clock_the_wall_clock_budget_simply_does_not_fire() {
    // The core reads no clock. A caller that supplies none gets the structural
    // budgets and nothing else; this pins that as intended rather than a bug.
    let registry = registry_with(vec![Box::new(StaticSource::semantic(
        SourceKind::Specification,
        vec![spec_evidence("ev_1", "the scheduler picks a node", 900)],
    ))]);
    let budget = RetrievalBudget {
        max_wall_clock_seconds: 1,
        ..RetrievalBudget::default()
    };
    let request = RetrievalRequest::new(
        "AS-1",
        AgentRole::Planner,
        "how does the scheduler work?",
        scope(),
    )
    .requiring(["scheduler".to_string()])
    .with_budget(budget);

    let mut coordinator =
        RetrievalCoordinator::new(&registry, AgentRole::Planner, NOW, REVISION);
    let outcome = coordinator.retrieve("rag_noclock", &request).expect("loop runs");

    assert_eq!(outcome.stop_reason, StopReason::SufficientEvidence);
    assert_eq!(outcome.ledger.elapsed_seconds(), 0);
}
