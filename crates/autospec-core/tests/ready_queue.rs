use std::collections::BTreeMap;

use autospec_core::coordination::{
    parse_remote_pull_request_page_json, plan_ready_queue, PullRequestEvidence, QueuePolicy,
    ReadyQueueInput, RemoteIssue, RemotePullRequest, RemotePullRequestCheck,
};

const SAFETY_REVIEW: &str = "## Safety review\n\n<!-- autospec-safety:begin -->\n- **decision:** `SAFETY_PASS`\n<!-- autospec-safety:end -->\n\n";

fn issue(number: u64, body: &str, labels: &[&str]) -> RemoteIssue {
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
    }
}

#[test]
fn parses_cursor_paged_pull_request_evidence_and_rejects_a_missing_cursor() {
    let page = parse_remote_pull_request_page_json(
        r#"{"items":[{"number":900,"state":"OPEN","body":"Fixes #400","statusCheckRollup":[{"name":"tests","status":"COMPLETED","conclusion":"SUCCESS"}]}],"page_info":{"has_next_page":true,"end_cursor":"cursor-900"}}"#,
    )
    .expect("parse paged pull request evidence");

    assert!(page.has_next_page);
    assert_eq!(page.end_cursor.as_deref(), Some("cursor-900"));
    assert_eq!(page.pull_requests[0].number, 900);
    assert_eq!(
        page.pull_requests[0].checks[0].conclusion.as_deref(),
        Some("SUCCESS")
    );
    assert!(parse_remote_pull_request_page_json(
        r#"{"items":[],"page_info":{"has_next_page":true,"end_cursor":null}}"#
    )
    .is_err());
}

#[test]
fn scopes_dependency_edges_to_the_dependencies_heading() {
    let mut input = ready_input(vec![issue(
        100,
        "## Shared contracts\n\n#100 depends on #101.\n\n## Implementation outline\n\n- edit `src/a.rs`\n",
        &["auto-implement", "safety:reviewed"],
    )]);
    input.dependencies.insert(
        101,
        RemoteIssue::open(101, "upstream", "", Vec::new(), "agent"),
    );

    let plan = plan_ready_queue(&input);

    assert_eq!(plan.ready_numbers(), vec![100]);
    assert!(plan.blocked.is_empty());
}

#[test]
fn blocks_dependencies_and_reports_a_cycle_without_reordering_candidates() {
    let mut input = ready_input(vec![issue(
        200,
        "## Dependencies\n\nDepends on issue #201\n\n## Implementation outline\n\n- edit `src/a.rs`\n",
        &["auto-implement", "safety:reviewed"],
    )]);
    input.dependencies.insert(
        201,
        RemoteIssue::open(
            201,
            "upstream",
            "## Dependencies\n\nDepends on #200\n",
            Vec::new(),
            "agent",
        ),
    );

    let plan = plan_ready_queue(&input);
    let blocked = &plan.blocked[0];

    assert_eq!(blocked.issue.number, 200);
    assert_eq!(blocked.reason.as_deref(), Some("blocked_cycle"));
    assert_eq!(blocked.unmet_dependencies, vec![201]);
    assert_eq!(blocked.cycle_dependencies, vec![201]);
}

#[test]
fn treats_epic_and_children_back_edges_as_observable_non_blocking_references() {
    let mut input = ready_input(vec![issue(
        300,
        "## Dependencies\n\nDepends on #301\nDepends on #302\n\n## Implementation outline\n\n- edit `src/a.rs`\n",
        &["auto-implement", "safety:reviewed"],
    )]);
    input.dependencies.insert(
        301,
        RemoteIssue::open(301, "epic", "", vec!["epic".to_string()], "agent"),
    );
    input.dependencies.insert(
        302,
        RemoteIssue::open(
            302,
            "tracker",
            "## Children\n\n- [ ] #300 child\n",
            Vec::new(),
            "agent",
        ),
    );

    let plan = plan_ready_queue(&input);
    let ready = &plan.ready[0];

    assert_eq!(ready.issue.number, 300);
    assert_eq!(ready.non_blocking_refs.len(), 2);
    assert_eq!(ready.non_blocking_refs[0].reason, "epic_label");
    assert_eq!(ready.non_blocking_refs[1].reason, "children_back_edge");
    assert!(ready.non_blocking_refs[1].cycle);
}

#[test]
fn blocks_a_candidate_when_linked_pr_evidence_is_unavailable() {
    let mut input = ready_input(vec![issue(
        400,
        "## Implementation outline\n\n- edit `src/a.rs`\n",
        &["auto-implement", "safety:reviewed"],
    )]);
    input.pull_requests = PullRequestEvidence::Unavailable("gh pr list failed".to_string());

    let plan = plan_ready_queue(&input);

    assert_eq!(
        plan.blocked[0].reason.as_deref(),
        Some("linked_pr_evidence_unavailable")
    );
}

#[test]
fn blocks_open_linked_pull_requests_with_nonterminal_checks() {
    let mut input = ready_input(vec![issue(
        401,
        "## Implementation outline\n\n- edit `src/a.rs`\n",
        &["auto-implement", "safety:reviewed"],
    )]);
    input.pull_requests = PullRequestEvidence::Available(vec![RemotePullRequest::open(
        900,
        "Fixes #401",
        vec![RemotePullRequestCheck::in_progress("tests")],
    )]);

    let plan = plan_ready_queue(&input);
    let blocked = &plan.blocked[0];

    assert_eq!(blocked.reason.as_deref(), Some("linked_pr_open"));
    assert_eq!(blocked.linked_pr, Some(900));
}

#[test]
fn recognizes_linked_pr_closures_with_flexible_whitespace() {
    let mut input = ready_input(vec![issue(
        402,
        "## Implementation outline\n\n- edit `src/a.rs`\n",
        &["auto-implement", "safety:reviewed"],
    )]);
    input.pull_requests = PullRequestEvidence::Available(vec![RemotePullRequest::open(
        901,
        "Resolves    #402",
        vec![RemotePullRequestCheck::in_progress("tests")],
    )]);

    let plan = plan_ready_queue(&input);

    assert_eq!(plan.blocked[0].reason.as_deref(), Some("linked_pr_open"));
    assert_eq!(plan.blocked[0].linked_pr, Some(901));
}

#[test]
fn detects_active_and_same_batch_path_conflicts_before_selecting_a_batch() {
    let mut input = ready_input(vec![
        issue(
            500,
            "## Implementation outline\n\n- edit `src/shared.rs`\n",
            &["auto-implement", "safety:reviewed"],
        ),
        issue(
            501,
            "## Implementation outline\n\n- edit `src/shared.rs`\n",
            &["auto-implement", "safety:reviewed"],
        ),
        issue(
            502,
            "## Implementation outline\n\n- edit `docs/independent.md`\n",
            &["auto-implement", "safety:reviewed"],
        ),
    ]);
    input.active.push(issue(
        499,
        "## Implementation outline\n\n- edit `src/active.rs`\n",
        &["in-progress-by-bot"],
    ));

    let plan = plan_ready_queue(&input);

    assert_eq!(plan.ready_numbers(), vec![500, 502]);
    assert_eq!(plan.conflicts.len(), 1);
    assert_eq!(plan.conflicts[0].issue.number, 501);
    assert_eq!(
        plan.conflicts[0].reason.as_deref(),
        Some("batch_path_conflict")
    );
    assert_eq!(plan.batch_numbers(), vec![500, 502]);
}

#[test]
fn gives_the_first_serial_issue_an_exclusive_batch_and_respects_worker_capacity() {
    let mut input = ready_input(vec![
        issue(
            600,
            "## Implementation outline\n\n- edit `src/deep.rs`\n",
            &["auto-implement", "safety:reviewed", "reasoning:deep"],
        ),
        issue(
            601,
            "## Implementation outline\n\n- edit `src/safe.rs`\n",
            &["auto-implement", "safety:reviewed"],
        ),
    ]);
    input.policy = QueuePolicy::new(3, 2);
    input.active.push(issue(
        599,
        "## Implementation outline\n\n- edit `src/active.rs`\n",
        &["in-progress-by-bot"],
    ));

    let plan = plan_ready_queue(&input);

    assert_eq!(plan.worker_cap.remaining, 1);
    assert_eq!(plan.ready[0].serialization_reasons, vec!["reasoning:deep"]);
    assert_eq!(plan.batch_numbers(), vec![600]);
}

#[test]
fn blocks_unreviewed_and_needs_human_candidates_before_other_planning() {
    let input = ready_input(vec![
        issue(
            700,
            "## Implementation outline\n\n- edit `src/a.rs`\n",
            &["auto-implement"],
        ),
        issue(
            701,
            "## Implementation outline\n\n- edit `src/b.rs`\n",
            &["auto-implement", "safety:reviewed", "autospec:needs-human"],
        ),
    ]);

    let plan = plan_ready_queue(&input);

    assert_eq!(
        plan.blocked[0].reason.as_deref(),
        Some("safety_gate_failed")
    );
    assert_eq!(
        plan.blocked[1].reason.as_deref(),
        Some("autospec_needs_human")
    );
}

#[test]
fn blocks_classification_drafts_and_requires_the_implementation_label() {
    let input = ready_input(vec![
        issue(
            702,
            "## Implementation outline\n\n- edit `src/draft.rs`\n",
            &["auto-implement", "needs-classify", "safety:reviewed"],
        ),
        issue(
            703,
            "## Implementation outline\n\n- edit `src/unlabeled.rs`\n",
            &["safety:reviewed"],
        ),
        issue(
            704,
            "## Implementation outline\n\n- edit `src/promoted.rs`\n",
            &["auto-implement", "safety:reviewed"],
        ),
    ]);

    let plan = plan_ready_queue(&input);

    assert_eq!(plan.ready_numbers(), vec![704]);
    assert_eq!(plan.batch_numbers(), vec![704]);
    assert_eq!(plan.blocked[0].reason.as_deref(), Some("needs_classify"));
    assert_eq!(
        plan.blocked[0].blocked_label.as_deref(),
        Some("needs-classify")
    );
    assert_eq!(
        plan.blocked[1].reason.as_deref(),
        Some("missing_auto_implement")
    );
}

#[test]
fn blocks_groom_proposed_issues_until_admission() {
    let input = ready_input(vec![
        issue(
            710,
            "## Implementation outline\n\n- edit `src/proposed.rs`\n",
            &["auto-implement", "groom:proposed", "safety:reviewed"],
        ),
        issue(
            711,
            "## Implementation outline\n\n- edit `src/admitted.rs`\n",
            &["auto-implement", "safety:reviewed"],
        ),
    ]);

    let plan = plan_ready_queue(&input);

    assert_eq!(plan.ready_numbers(), vec![711]);
    assert_eq!(plan.batch_numbers(), vec![711]);
    assert_eq!(plan.blocked[0].reason.as_deref(), Some("groom_proposed"));
    assert_eq!(
        plan.blocked[0].blocked_label.as_deref(),
        Some("groom:proposed")
    );
}

#[test]
fn blocks_security_prerequisites_even_when_auto_implement_is_stale() {
    let input = ready_input(vec![
        issue(
            712,
            "## Prerequisites\n\n- blocking: replica unavailable\n",
            &[
                "auto-implement",
                "autospec:blocked-prerequisite",
                "safety:reviewed",
            ],
        ),
        issue(
            713,
            "## Prerequisites\n\n- verified: replica available\n",
            &["auto-implement", "safety:reviewed"],
        ),
    ]);

    let plan = plan_ready_queue(&input);

    assert_eq!(plan.ready_numbers(), vec![713]);
    assert_eq!(
        plan.blocked[0].reason.as_deref(),
        Some("security_prerequisite_blocked")
    );
    assert_eq!(
        plan.blocked[0].blocked_label.as_deref(),
        Some("autospec:blocked-prerequisite")
    );
}

#[test]
fn excludes_closed_auto_implement_issues_from_the_ready_queue() {
    let closed = RemoteIssue::closed(
        705,
        "closed candidate",
        format!("{SAFETY_REVIEW}## Implementation outline\n\n- edit `src/closed.rs`\n"),
        vec!["auto-implement".to_string(), "safety:reviewed".to_string()],
        "agent",
    );
    let input = ready_input(vec![
        closed,
        issue(
            706,
            "## Implementation outline\n\n- edit `src/open.rs`\n",
            &["auto-implement", "safety:reviewed"],
        ),
    ]);

    let plan = plan_ready_queue(&input);

    assert_eq!(plan.ready_numbers(), vec![706]);
    assert_eq!(plan.batch_numbers(), vec![706]);
    assert_eq!(plan.gate_counts.open, 1);
    assert_eq!(plan.gate_counts.candidate, 1);
}

#[test]
fn deduplicates_issue_numbers_before_planning_and_reports_gate_counts() {
    let mut input = ready_input(vec![
        issue(
            800,
            "## Implementation outline\n\n- edit `src/ready.rs`\n",
            &["auto-implement", "safety:reviewed"],
        ),
        issue(
            800,
            "## Implementation outline\n\n- edit `src/duplicate.rs`\n",
            &["auto-implement", "needs-classify", "safety:reviewed"],
        ),
        issue(
            801,
            "## Dependencies\n\nDepends on #802\n\n## Implementation outline\n\n- edit `src/dependent.rs`\n",
            &["auto-implement", "safety:reviewed"],
        ),
        issue(
            803,
            "## Implementation outline\n\n- edit `src/linked-pr.rs`\n",
            &["auto-implement", "safety:reviewed"],
        ),
        issue(
            804,
            "## Implementation outline\n\n- edit `src/active.rs`\n",
            &["auto-implement", "safety:reviewed"],
        ),
        issue(
            805,
            "## Implementation outline\n\n- edit `src/unreviewed.rs`\n",
            &["auto-implement"],
        ),
    ]);
    input.dependencies.insert(
        802,
        RemoteIssue::open(802, "unmerged dependency", "", Vec::new(), "agent"),
    );
    input.pull_requests = PullRequestEvidence::Available(vec![RemotePullRequest::open(
        900,
        "Fixes #803",
        vec![RemotePullRequestCheck::in_progress("tests")],
    )]);
    input.active = vec![
        issue(
            700,
            "## Implementation outline\n\n- edit `src/active.rs`\n",
            &["in-progress-by-bot"],
        ),
        issue(
            700,
            "## Implementation outline\n\n- edit `src/ignored-duplicate.rs`\n",
            &["in-progress-by-bot"],
        ),
    ];

    let plan = plan_ready_queue(&input);

    assert_eq!(plan.ready_numbers(), vec![800]);
    assert_eq!(
        plan.claimed
            .iter()
            .map(|issue| issue.number)
            .collect::<Vec<_>>(),
        vec![700]
    );
    assert_eq!(plan.gate_counts.open, 5);
    assert_eq!(plan.gate_counts.candidate, 5);
    assert_eq!(plan.gate_counts.reviewed, 4);
    assert_eq!(plan.gate_counts.blocked, 3);
    assert_eq!(plan.gate_counts.dependency_blocked, 1);
    assert_eq!(plan.gate_counts.linked_pr_blocked, 1);
    assert_eq!(plan.gate_counts.path_conflicted, 1);
    assert_eq!(plan.gate_counts.ready, 1);
    assert_eq!(plan.gate_counts.claimed, 1);
    assert_eq!(plan.gate_counts.selected, 1);
}
