//! Graph RAG traversal (spec sections 14, 15, 55.2).

use autospec_core::rag::graph::{GraphNode, KnowledgeGraph, NodeKind, Relation};

/// The section 14 example graph: an implementation, its interface, its caller,
/// its test, its configuration surface, and the dashboard that exposes it.
fn scheduler_graph() -> KnowledgeGraph {
    let mut graph = KnowledgeGraph::new("9a223af");
    for (id, kind) in [
        ("FairShareScheduler", NodeKind::Symbol),
        ("Scheduler", NodeKind::Symbol),
        ("GatewayRouter", NodeKind::Symbol),
        ("SchedulerFairnessTest", NodeKind::Test),
        ("AdminSchedulerConfig", NodeKind::Api),
        ("DashboardSchedulerAPI", NodeKind::Dashboard),
        ("NodeRegistry", NodeKind::Symbol),
    ] {
        graph.add_node(GraphNode::new(id, kind));
    }
    graph
        .add_edge("FairShareScheduler", Relation::Implements, "Scheduler")
        .unwrap();
    graph
        .add_edge("GatewayRouter", Relation::Calls, "FairShareScheduler")
        .unwrap();
    graph
        .add_edge(
            "FairShareScheduler",
            Relation::TestedBy,
            "SchedulerFairnessTest",
        )
        .unwrap();
    graph
        .add_edge(
            "FairShareScheduler",
            Relation::ConfiguredBy,
            "AdminSchedulerConfig",
        )
        .unwrap();
    graph
        .add_edge(
            "FairShareScheduler",
            Relation::ExposedBy,
            "DashboardSchedulerAPI",
        )
        .unwrap();
    graph
        .add_edge("FairShareScheduler", Relation::Calls, "NodeRegistry")
        .unwrap();
    graph
}

#[test]
fn implementations_are_reachable_from_the_interface_side() {
    let graph = scheduler_graph();

    let implemented = graph.neighbors("FairShareScheduler", Relation::Implements);

    assert_eq!(implemented.len(), 1);
    assert_eq!(implemented[0].id, "Scheduler");
}

#[test]
fn callers_are_reachable_and_typed() {
    let graph = scheduler_graph();

    let called = graph.neighbors("GatewayRouter", Relation::Calls);

    assert_eq!(called.len(), 1);
    assert_eq!(called[0].id, "FairShareScheduler");
}

#[test]
fn tests_covering_a_symbol_are_reachable() {
    let graph = scheduler_graph();

    let tests = graph.neighbors("FairShareScheduler", Relation::TestedBy);

    assert_eq!(tests.len(), 1);
    assert_eq!(tests[0].id, "SchedulerFairnessTest");
    assert_eq!(tests[0].kind, NodeKind::Test);
}

#[test]
fn traversal_respects_the_edge_filter() {
    let graph = scheduler_graph();

    let reached = graph
        .traverse("FairShareScheduler", &[Relation::TestedBy], 3)
        .expect("origin exists");

    assert_eq!(reached.len(), 1);
    assert_eq!(reached[0].node.id, "SchedulerFairnessTest");
}

#[test]
fn traversal_respects_the_depth_limit() {
    let graph = scheduler_graph();

    let depth_one = graph
        .traverse("GatewayRouter", &[], 1)
        .expect("origin exists");
    let depth_two = graph
        .traverse("GatewayRouter", &[], 2)
        .expect("origin exists");

    assert_eq!(depth_one.len(), 1, "one hop reaches only the scheduler");
    assert!(
        depth_two.len() > depth_one.len(),
        "two hops reach the scheduler's own neighbours"
    );
}

#[test]
fn traversal_returns_the_path_it_took() {
    let graph = scheduler_graph();

    let reached = graph
        .traverse("GatewayRouter", &[], 2)
        .expect("origin exists");
    let test_node = reached
        .iter()
        .find(|node| node.node.id == "SchedulerFairnessTest")
        .expect("the test is reachable in two hops");

    assert_eq!(test_node.depth, 2);
    assert_eq!(test_node.path.len(), 2);
    assert_eq!(test_node.path[0].from, "GatewayRouter");
    assert_eq!(test_node.path[1].relation, Relation::TestedBy);
}

#[test]
fn a_cycle_does_not_make_traversal_diverge() {
    let mut graph = scheduler_graph();
    graph
        .add_edge("NodeRegistry", Relation::Calls, "GatewayRouter")
        .unwrap();
    graph
        .add_edge("Scheduler", Relation::DependsOn, "FairShareScheduler")
        .unwrap();

    let reached = graph
        .traverse("FairShareScheduler", &[], 10)
        .expect("origin exists");

    assert!(reached.len() < graph.node_count() + 1);
    let mut ids = reached.iter().map(|node| node.node.id.clone()).collect::<Vec<_>>();
    ids.sort();
    let unique = ids.len();
    ids.dedup();
    assert_eq!(unique, ids.len(), "each node is reached at most once");
}

#[test]
fn a_dangling_edge_is_rejected_at_insert() {
    let mut graph = KnowledgeGraph::new("9a223af");
    graph.add_node(GraphNode::new("Scheduler", NodeKind::Symbol));

    let error = graph
        .add_edge("Scheduler", Relation::Calls, "GhostSymbol")
        .expect_err("an edge to an unindexed node must be refused");

    assert!(error.contains("GhostSymbol"), "{error}");
}

#[test]
fn traversal_from_an_unknown_origin_is_an_error_not_an_empty_result() {
    let graph = scheduler_graph();

    let error = graph
        .traverse("GhostSymbol", &[], 2)
        .expect_err("an unknown origin must be reported");

    assert!(error.contains("GhostSymbol"), "{error}");
}

#[test]
fn existence_checks_let_a_planner_avoid_a_hallucinated_symbol() {
    let graph = scheduler_graph();

    assert!(graph.exists("FairShareScheduler"));
    assert!(!graph.exists("FairShareSchedulerImpl"));
}

#[test]
fn duplicate_edges_are_stored_once() {
    let mut graph = scheduler_graph();
    let before = graph.edge_count();

    graph
        .add_edge("FairShareScheduler", Relation::Implements, "Scheduler")
        .unwrap();

    assert_eq!(graph.edge_count(), before);
}

#[test]
fn the_graph_reports_the_revision_it_indexes() {
    assert_eq!(scheduler_graph().revision(), "9a223af");
}
