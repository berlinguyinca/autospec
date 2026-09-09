//! Regression tests for the project-board readiness projection.
//!
//! Spec: `docs/specs/2026-09-08-parallel-decomposition-fleet-saturation.md`
//! §18, §26, §27, §35. The board projection shares the ready-queue planner
//! as its readiness authority, so these fixtures drive `plan_ready_queue`
//! directly with real `RemoteIssue` values (no mocks, no stub stores) and
//! pin the invariants:
//!
//! * a conflict domain never blocks readiness (§18, §26);
//! * a shared write surface never blocks readiness (§27.6);
//! * only the `## Dependencies` heading creates readiness edges (§27.6);
//! * parent links — epic references and children back-edges — are
//!   observable, non-blocking references, never blockers (§27.6, §35).

use std::collections::BTreeMap;

use autospec_core::coordination::{
    plan_ready_queue, PullRequestEvidence, QueuePolicy, ReadyQueueInput, RemoteIssue,
};

const SAFETY_REVIEW: &str = "## Safety review\n\n<!-- autospec-safety:begin -->\n- **decision:** `SAFETY_PASS`\n<!-- autospec-safety:end -->\n\n";

fn issue(number: u64, body: impl Into<String>, labels: &[&str]) -> RemoteIssue {
    let body = body.into();
    RemoteIssue::open(
        number,
        format!("issue-{number}"),
        format!("{SAFETY_REVIEW}{body}"),
        labels.iter().map(|label| (*label).to_string()).collect(),
        "agent",
    )
}

fn ready_input(candidates: Vec<RemoteIssue>) -> ReadyQueueInput {
    ReadyQueueInput {
        candidates,
        active: Vec::new(),
        dependencies: BTreeMap::new(),
        pull_requests: PullRequestEvidence::Available(Vec::new()),
        policy: QueuePolicy::new(3, 0),
        capabilities: BTreeMap::new(),
        no_output_streaks: BTreeMap::new(),
    }
}

#[test]
fn parent_link_never_blocks_readiness_in_the_board_projection() {
    // The parent tracks its child in `## Children` and the child declares
    // `Depends on issue #<parent>`: the back-edge is observable but never a
    // readiness blocker, so both issues stay ready in the projection.
    let input = ready_input(vec![
        issue(
            950,
            "## Children\n\n- [ ] #951 child\n\n## Implementation outline\n\n- edit `src/parent.rs`\n",
            &["auto-implement", "safety:reviewed"],
        ),
        issue(
            951,
            "## Dependencies\n\nDepends on issue #950\n\n## Implementation outline\n\n- edit `src/child.rs`\n",
            &["auto-implement", "safety:reviewed"],
        ),
    ]);

    let plan = plan_ready_queue(&input);

    assert_eq!(plan.ready_numbers(), vec![950, 951]);
    assert_eq!(plan.batch_numbers(), vec![950, 951]);
    assert!(plan.blocked.is_empty());
    assert!(plan.conflicts.is_empty());
    let child = plan
        .ready
        .iter()
        .find(|view| view.issue.number == 951)
        .expect("child ready");
    assert_eq!(child.non_blocking_refs.len(), 1);
    assert_eq!(child.non_blocking_refs[0].issue, 950);
    assert_eq!(child.non_blocking_refs[0].reason, "children_back_edge");
    assert!(child.non_blocking_refs[0].cycle);
}

#[test]
fn epic_parent_link_never_blocks_readiness_in_the_board_projection() {
    // A child that depends on an open epic parent stays ready: the epic
    // reference is a non-blocking reference, not a readiness edge.
    let mut input = ready_input(vec![issue(
        961,
        "## Dependencies\n\nDepends on issue #960\n\n## Implementation outline\n\n- edit `src/child.rs`\n",
        &["auto-implement", "safety:reviewed"],
    )]);
    input.dependencies.insert(
        960,
        RemoteIssue::open(960, "epic parent", "", vec!["epic".to_string()], "agent"),
    );

    let plan = plan_ready_queue(&input);

    assert_eq!(plan.ready_numbers(), vec![961]);
    assert!(plan.blocked.is_empty());
    assert!(plan.conflicts.is_empty());
    assert_eq!(plan.ready[0].non_blocking_refs.len(), 1);
    assert_eq!(plan.ready[0].non_blocking_refs[0].issue, 960);
    assert_eq!(plan.ready[0].non_blocking_refs[0].reason, "epic_label");
}

#[test]
fn shared_conflict_domains_and_write_surfaces_never_block_readiness_in_the_board_projection() {
    // Two board candidates that share a conflict domain and a shared write
    // surface are still independent readiness-wise: the Concurrency section
    // carries no readiness edges.
    let body = |path: &str| {
        format!(
            "## Concurrency\n\nParallel safe: yes\n\n### Exclusive write ownership\n- `{path}`\n\n### Shared write surfaces\n- `crates/core/src/lib.rs`\n\n### Conflict domains\n- `core-registry`\n\n## Implementation outline\n\n- edit `{path}`\n"
        )
    };
    let input = ready_input(vec![
        issue(
            970,
            body("crates/core/src/alpha.rs"),
            &["auto-implement", "safety:reviewed"],
        ),
        issue(
            971,
            body("crates/core/src/beta.rs"),
            &["auto-implement", "safety:reviewed"],
        ),
    ]);

    let plan = plan_ready_queue(&input);

    assert_eq!(plan.ready_numbers(), vec![970, 971]);
    assert_eq!(plan.batch_numbers(), vec![970, 971]);
    assert!(plan.blocked.is_empty());
    assert!(plan.conflicts.is_empty());
    for view in &plan.ready {
        assert!(view.unmet_dependencies.is_empty());
        assert!(view.conflicts_with.is_none());
    }
}
