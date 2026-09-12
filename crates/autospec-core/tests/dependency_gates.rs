//! Prerequisites block dispatch; gates block release (issue #4015).
//!
//! The regression tests run in the configuration the incident required: a
//! 61-issue-style roadmap whose depth of 28 rested on five phase-approval
//! checkpoints, each listed in the next phase's `## Dependencies`, with one
//! independent-review checkpoint (#53, no enrolled reviewer) transitively
//! blocking the bulk of the open issues — and a dispatcher that reported
//! "nothing new is ready" for days.

use autospec_core::dependency_gates::{
    classify, depth_verdict, emit, frontier_verdict, is_checkpoint, measure, ready_set_change,
    redundant_edges, DependencyClass, DependencySpec, DepthPolicy, FrontierIssue, FrontierVerdict,
};

fn spec(id: &str, title: &str, body: &str) -> DependencySpec {
    DependencySpec {
        source_id: id.to_string(),
        source_title: title.to_string(),
        source_body: body.to_string(),
        produced_interface: None,
    }
}

fn frontier(id: &str, assignee: Option<&str>, agent_completable: bool) -> FrontierIssue {
    FrontierIssue {
        id: id.to_string(),
        assignee: assignee.map(|a| a.to_string()),
        agent_completable,
    }
}

// --- AC5a: a review checkpoint emits as a gate; the work stays dispatchable

#[test]
fn a_review_checkpoint_is_emitted_as_a_gate_and_the_next_phase_stays_dispatchable() {
    // The incident's checkpoint titles, verbatim in shape: each phase's
    // work listed the previous phase's review as a dependency.
    let gate = spec(
        "53",
        "Phase-1 soak, failure injection and independent review",
        "Requires an enrolled independent reviewer. No reviewer is enrolled.",
    );
    let technical = spec(
        "52",
        "Define the ingest API schema",
        "Produces the schema the ingest worker consumes.",
    );

    let emission = emit(&[gate.clone(), technical]);
    assert_eq!(emission.dependencies, vec!["52".to_string()]);
    assert_eq!(emission.gate_labels, vec!["gate:53".to_string()]);

    // With the gate edge out of `## Dependencies`, the next phase's work
    // is held behind the technical prerequisite only — and when that
    // closes it is dispatchable, with no human in the dispatch path.
    let issues = [
        frontier("52", None, true),
        frontier("53", None, false),
        frontier("54", None, true),
    ];
    let edges: &[(&str, &str)] = &[("52", "54")]; // the gate edge is absent
                                                  // The technical work is dispatchable; the gate is open work no agent
                                                  // may take, not a dispatch.
    assert_eq!(
        frontier_verdict(&issues, edges),
        FrontierVerdict::Eligible { count: 1 }
    );
    // Once the technical prerequisite closes, the next phase's work is
    // dispatchable even with the gate still open: the gate is a label on
    // the release, not a block on the dispatch.
    assert_eq!(
        frontier_verdict(
            &[frontier("53", None, false), frontier("54", None, true)],
            &[]
        ),
        FrontierVerdict::Eligible { count: 1 }
    );
}

// --- AC5b: a genuine interface prerequisite still blocks

#[test]
fn a_genuine_interface_prerequisite_still_blocks_dispatch() {
    let schema = spec(
        "7",
        "Define the config schema",
        "The loader, the validator and the CLI all consume this schema.",
    );
    assert_eq!(
        classify(&schema),
        DependencyClass::Prerequisite { interface: None }
    );
    assert_eq!(
        emit(std::slice::from_ref(&schema)).dependencies,
        vec!["7".to_string()]
    );

    // Both open: only the producer is eligible — the consumer is held
    // behind it, which is the point of the dispatch block.
    let issues = [frontier("7", None, true), frontier("9", None, true)];
    let edges: &[(&str, &str)] = &[("7", "9")];
    assert_eq!(
        frontier_verdict(&issues, edges),
        FrontierVerdict::Eligible { count: 1 }
    );
    // Once the producer closes, the consumer is dispatchable.
    assert_eq!(
        frontier_verdict(&[frontier("9", None, true)], &[("7", "9")]),
        FrontierVerdict::Eligible { count: 1 }
    );
}

// --- AC2: a checkpoint is a gate by default; a prerequisite needs the interface

#[test]
fn a_checkpoint_is_a_gate_unless_it_states_the_interface_it_produces() {
    let gate_title = "Phase-2 private-alpha review and qualification";

    // No interface stated: gate by default.
    assert_eq!(classify(&spec("14", gate_title, "")), DependencyClass::Gate);
    // Whitespace-only interface is not a statement.
    let blank = DependencySpec {
        produced_interface: Some("   ".to_string()),
        ..spec("14", gate_title, "")
    };
    assert_eq!(classify(&blank), DependencyClass::Gate);

    // Interface stated: the dependent consumes it, so the dispatch block
    // is legitimate.
    let with_interface = DependencySpec {
        produced_interface: Some(
            "the signed qualification report at docs/qualification.md".to_string(),
        ),
        ..spec("14", gate_title, "")
    };
    assert_eq!(
        classify(&with_interface),
        DependencyClass::Prerequisite {
            interface: Some("the signed qualification report at docs/qualification.md".to_string())
        }
    );
}

#[test]
fn checkpoint_detection_matches_the_marked_words_on_boundaries() {
    // The incident's titles and the AC's four marks.
    assert!(is_checkpoint(
        "Phase-1 soak, failure injection and independent review",
        ""
    ));
    assert!(is_checkpoint(
        "Phase-2 private-alpha review and qualification",
        ""
    ));
    assert!(is_checkpoint("Phase-3 sign-off", ""));
    assert!(is_checkpoint("Phase-3 SIGNOFF", ""));
    assert!(is_checkpoint("Phase-3 sign_off", ""));
    assert!(is_checkpoint("phase 3 approval", ""));
    assert!(is_checkpoint("", "This is a release checkpoint."));
    assert!(is_checkpoint(
        "Ship it",
        "Awaiting operator approval before merge."
    ));

    // Not checkpoints: the words must stand on boundaries.
    assert!(!is_checkpoint("Implement the preview pane", ""));
    assert!(!is_checkpoint("Add a reviewer-facing log line", ""));
    assert!(!is_checkpoint(
        "Approvals queue UI",
        "renders the queue state"
    ));
    assert!(!is_checkpoint(
        "Implement the ingest pipeline",
        "ships without ceremony"
    ));
}

// --- AC5c: a graph deeper than the threshold fails, with blockers named

#[test]
fn a_graph_exceeding_the_depth_threshold_fails_with_the_blocking_issues_named() {
    // A 10-deep chain: a=1 b=2 ... j=10.
    let ids: &[&str] = &["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"];
    let edges: Vec<(&str, &str)> = (0..9).map(|k| (ids[k], ids[k + 1])).collect();
    let m = measure(ids, &edges);
    assert_eq!(m.max_depth, 10);
    assert_eq!(m.frontier_width, 1);
    assert_eq!(
        m.top_blockers,
        vec![
            ("a".to_string(), 9),
            ("b".to_string(), 8),
            ("c".to_string(), 7),
        ]
    );

    let policy = DepthPolicy::default(); // threshold 7
    let verdict = depth_verdict(&m, &policy);
    assert!(matches!(
        verdict,
        autospec_core::dependency_gates::DepthVerdict::Reject { .. }
    ));
    let line = verdict.line();
    assert!(line.contains("depth 10 exceeds threshold 7"), "{line}");
    assert!(line.contains("a (blocks 9)"), "{line}");

    // Within the threshold: file. A custom threshold admits the chain.
    let shallow = measure(ids, &[]);
    assert_eq!(shallow.max_depth, 1);
    let file = depth_verdict(&shallow, &policy);
    assert!(matches!(
        file,
        autospec_core::dependency_gates::DepthVerdict::File { .. }
    ));
    assert_eq!(
        depth_verdict(&m, &DepthPolicy { max_depth: 10 }),
        autospec_core::dependency_gates::DepthVerdict::File {
            depth: 10,
            limit: 10
        }
    );
}

#[test]
fn the_top_three_blockers_are_reported_by_downstream_count() {
    // x blocks 4, y blocks 3, z blocks 2, w blocks 1: only the top 3.
    // Fan-outs, so no intermediate issue shares a blocker's count.
    let ids = [
        "x", "y", "z", "w", "a1", "a2", "a3", "a4", "b1", "b2", "b3", "c1", "c2", "d1",
    ];
    let edges: &[(&str, &str)] = &[
        ("x", "a1"),
        ("x", "a2"),
        ("x", "a3"),
        ("x", "a4"),
        ("y", "b1"),
        ("y", "b2"),
        ("y", "b3"),
        ("z", "c1"),
        ("z", "c2"),
        ("w", "d1"),
    ];
    let m = measure(&ids, edges);
    assert_eq!(
        m.top_blockers,
        vec![
            ("x".to_string(), 4),
            ("y".to_string(), 3),
            ("z".to_string(), 2),
        ]
    );
    assert_eq!(m.frontier_width, 4);
    assert_eq!(
        m.line(),
        "graph: depth 2, frontier width 4, top blockers: x (blocks 4), y (blocks 3), z (blocks 2)"
    );
}

#[test]
fn the_incident_shape_gate_edges_account_for_the_excess_depth() {
    // Five phase checkpoints (53, 14, 21, 35, 47), each reviewing the
    // previous phase's work and listed in the next phase's dependencies;
    // within-phase work is a technical chain.
    // 53 -> p1 (7) -> 14 -> p2 (5) -> 21 -> p3 (4) -> 35 -> p4 (4) -> 47
    // -> p5 (3) = depth 28 with the gate edges, depth 7 without them.
    let phases: [(u64, usize); 5] = [(1, 7), (2, 5), (3, 4), (4, 4), (5, 3)];
    let gate_ids: [&str; 5] = ["53", "14", "21", "35", "47"];
    let all: Vec<String> = gate_ids
        .iter()
        .copied()
        .map(|g| g.to_string())
        .chain(
            phases
                .iter()
                .flat_map(|(phase, len)| (1..=*len).map(move |k| format!("p{phase}-{k}"))),
        )
        .collect();
    let find = |s: &str| all.iter().position(|x| x.as_str() == s).unwrap();
    let mut edges: Vec<(usize, usize)> = vec![];
    // The first checkpoint gates the first phase's head; each phase's tail
    // feeds the next checkpoint — the incident's five gate edges.
    edges.push((find(gate_ids[0]), find("p1-1")));
    for (i, &(phase, len)) in phases.iter().enumerate() {
        for k in 1..len {
            edges.push((
                find(&format!("p{phase}-{k}")),
                find(&format!("p{phase}-{}", k + 1)),
            ));
        }
        if i < 4 {
            // The checkpoint reviews this phase's tail, and gates the next
            // phase's head — both directions of the incident's edges.
            edges.push((find(&format!("p{phase}-{len}")), find(gate_ids[i + 1])));
            edges.push((find(gate_ids[i + 1]), find(&format!("p{}-1", phase + 1))));
        }
    }
    let ids: Vec<&str> = all.iter().map(|s| s.as_str()).collect();
    let edge_strs: Vec<(&str, &str)> = edges.iter().map(|(a, b)| (ids[*a], ids[*b])).collect();

    let with_gates = measure(&ids, &edge_strs);
    assert_eq!(with_gates.max_depth, 28);
    assert_eq!(with_gates.frontier_width, 1);
    // 53 transitively blocks every issue after it: 23 work + 4 later gates.
    assert_eq!(with_gates.top_blockers[0], ("53".to_string(), 27));
    assert!(matches!(
        depth_verdict(&with_gates, &DepthPolicy::default()),
        autospec_core::dependency_gates::DepthVerdict::Reject { .. }
    ));

    // Remove only the five checkpoints' edges — no technical dependency
    // touched.
    let gate_set: std::collections::BTreeSet<&str> = gate_ids.iter().copied().collect();
    let technical_only: Vec<(&str, &str)> = edge_strs
        .iter()
        .copied()
        .filter(|(pre, succ)| !gate_set.contains(pre) && !gate_set.contains(succ))
        .collect();
    let without_gates = measure(&ids, &technical_only);
    assert_eq!(without_gates.max_depth, 7); // the longest within-phase chain
                                            // One head per phase plus the five gates, all without prerequisites:
                                            // the incident's `ready: 5 -> 10`.
    assert_eq!(without_gates.frontier_width, 10);
    assert_eq!(
        depth_verdict(&without_gates, &DepthPolicy::default()),
        autospec_core::dependency_gates::DepthVerdict::File { depth: 7, limit: 7 }
    );
}

// --- AC4: a zero-eligible frontier distinguishes exhausted from structurally blocked

#[test]
fn a_frontier_blocked_on_unassignable_gates_is_reported_distinctly() {
    // The incident state, in miniature: two checkpoints nobody can
    // complete hold the work; the work is held behind them.
    let issues = [
        frontier("53", None, false), // independent review, no reviewer
        frontier("14", None, false), // qualification
        frontier("100", None, true),
        frontier("101", Some("agent-1"), true),
        frontier("102", None, true),
    ];
    let edges: &[(&str, &str)] = &[("53", "100"), ("100", "101"), ("14", "101"), ("53", "102")];

    let verdict = frontier_verdict(&issues, edges);
    assert!(!matches!(verdict, FrontierVerdict::Eligible { .. }));
    assert!(verdict.blocked_on_unassignable());
    let line = verdict.line();
    assert!(
        line.starts_with("frontier blocked on 2 unassignable issue(s) an agent cannot complete:"),
        "{line}"
    );
    assert!(line.contains("53 (holds 3)"), "{line}");
    assert!(line.contains("14 (holds 1)"), "{line}");
    // The completable hold is reported, but not as unassignable.
    assert!(line.contains("100 (holds 1)"), "{line}");
    assert!(!line.contains("101 ("));
}

#[test]
fn an_exhausted_frontier_is_reported_as_exhausted() {
    assert_eq!(frontier_verdict(&[], &[]), FrontierVerdict::Exhausted);
    assert_eq!(
        frontier_verdict(&[], &[]).line(),
        "frontier exhausted: no open issues remain"
    );
    assert!(!frontier_verdict(&[], &[]).blocked_on_unassignable());
}

#[test]
fn an_ordinary_hold_is_not_reported_as_unassignable() {
    // The hold rests on a checkpoint a human is assigned to: visible, in
    // hands, and not the unassignable structural block.
    let issues = [
        frontier("53", Some("reviewer-1"), false),
        frontier("100", None, true),
        frontier("101", None, true),
    ];
    let edges: &[(&str, &str)] = &[("53", "100"), ("100", "101")];
    let verdict = frontier_verdict(&issues, edges);
    assert!(!verdict.blocked_on_unassignable());
    assert_eq!(
        verdict.line(),
        "frontier held behind 2 open issue(s): 53 (holds 2), 100 (holds 1)"
    );
}

#[test]
fn a_lone_gate_holds_the_frontier_with_nothing_downstream() {
    let issues = [frontier("53", None, false)];
    let verdict = frontier_verdict(&issues, &[]);
    assert!(verdict.blocked_on_unassignable());
    assert_eq!(
        verdict.line(),
        "frontier blocked on 1 unassignable issue(s) an agent cannot complete: 53 (holds 0)"
    );
}

// --- measurement edge cases

#[test]
fn measurement_of_an_empty_graph_reports_zero() {
    let m = measure(&[], &[]);
    assert_eq!(m.max_depth, 0);
    assert_eq!(m.frontier_width, 0);
    assert!(m.top_blockers.is_empty());
    assert_eq!(
        m.line(),
        "graph: depth 0, frontier width 0, top blockers: none"
    );
}

#[test]
fn measurement_ignores_edges_outside_the_graph_and_self_edges() {
    let ids = ["a", "b"];
    let edges: &[(&str, &str)] = &[("a", "b"), ("a", "ghost"), ("zz", "b"), ("a", "a")];
    let m = measure(&ids, edges);
    assert_eq!(m.max_depth, 2);
    assert_eq!(m.frontier_width, 1);
    assert_eq!(m.top_blockers, vec![("a".to_string(), 1)]);
}

// --- Invariant 5: a metric over the artifact vs the outcome it cashes into

#[test]
fn a_transitively_redundant_edge_is_the_one_already_implied_by_another_path() {
    // a -> b -> c, plus the shortcut a -> c. The shortcut is redundant: the
    // ordering of a before c already holds through b.
    let ids = ["a", "b", "c"];
    let edges: &[(&str, &str)] = &[("a", "b"), ("a", "c"), ("b", "c")];
    let redundant = redundant_edges(&ids, edges);
    assert_eq!(redundant, [("a", "c")].into_iter().collect());
}

#[test]
fn a_redundant_edge_removed_frees_nothing() {
    // The invariant in its sharpest form: the artifact metric is non-zero
    // (one redundant edge), but acting on it cashes into zero issues
    // startable. The percentage describes the graph; the outcome is the
    // decision input, and here it is inert.
    let ids = ["a", "b", "c"];
    let edges: &[(&str, &str)] = &[("a", "b"), ("a", "c"), ("b", "c")];
    let redundant = redundant_edges(&ids, edges);
    assert_eq!(redundant.len(), 1);
    let change = ready_set_change(&ids, edges, &redundant);
    assert_eq!(change.before, 1);
    assert_eq!(change.after, 1);
    assert_eq!(change.gained(), 0);
}

#[test]
fn a_dense_graph_reports_many_redundant_edges_but_gains_nothing() {
    // The pitfall at scale: a 5-node chain with every forward shortcut. Six
    // of the ten edges are transitively redundant — a 60% artifact metric
    // that reads as waste to be pruned. Removing all six moves the ready set
    // by exactly nothing, because each shortcut's successor is already
    // ordered by the chain.
    let ids = ["a", "b", "c", "d", "e"];
    let edges: &[(&str, &str)] = &[
        ("a", "b"),
        ("b", "c"),
        ("c", "d"),
        ("d", "e"), // the chain
        ("a", "c"),
        ("a", "d"),
        ("a", "e"),
        ("b", "d"),
        ("b", "e"),
        ("c", "e"), // the shortcuts
    ];
    let redundant = redundant_edges(&ids, edges);
    assert_eq!(redundant.len(), 6);
    let change = ready_set_change(&ids, edges, &redundant);
    assert_eq!(change.before, 1);
    assert_eq!(change.after, 1);
    assert_eq!(change.gained(), 0);
    // The report names the outcome, not the six edges.
    assert_eq!(change.line(), "ready set 1 -> 1 (gains 0)");
}

#[test]
fn removing_a_genuine_prerequisite_does_widen_the_ready_set() {
    // Contrast: an edge that is not redundant, because it is the only path
    // that orders its successor. Removing it frees the successor — the
    // simulation reports the one-issue gain that the redundant-edge case
    // could not.
    let ids = ["a", "b", "c"];
    let edges: &[(&str, &str)] = &[("a", "b"), ("b", "c")];
    let removed: std::collections::BTreeSet<(&str, &str)> = [("a", "b")].into_iter().collect();
    assert!(redundant_edges(&ids, edges).is_empty());
    let change = ready_set_change(&ids, edges, &removed);
    assert_eq!(change.before, 1);
    assert_eq!(change.after, 2);
    assert_eq!(change.gained(), 1);
}
