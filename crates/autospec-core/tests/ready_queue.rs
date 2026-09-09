use std::collections::BTreeMap;

use autospec_core::coordination::{
    parse_remote_pull_request_page_json, plan_ready_queue, CapabilityState, PullRequestEvidence,
    QueuePolicy, ReadyQueueInput, RemoteIssue, RemotePullRequest, RemotePullRequestCheck,
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
    ready_input_with(candidates, BTreeMap::new(), BTreeMap::new())
}

fn ready_input_with(
    candidates: Vec<RemoteIssue>,
    capabilities: BTreeMap<String, CapabilityState>,
    no_output_streaks: BTreeMap<u64, usize>,
) -> ReadyQueueInput {
    ReadyQueueInput {
        candidates,
        active: Vec::new(),
        dependencies: BTreeMap::new(),
        pull_requests: PullRequestEvidence::Available(Vec::new()),
        policy: QueuePolicy::new(3, 0),
        capabilities,
        no_output_streaks,
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
        Some("waits_on_foundation")
    );
    assert_eq!(plan.batch_numbers(), vec![500, 502]);
}

#[test]
fn holds_sibling_issues_that_share_a_new_foundation_module_declared_in_files_touched() {
    // Reproduces the five-siblings incident: each issue is self-contained and
    // declares the shared foundation module under `## Files touched` (the
    // issue-quality-contract section), in mixed backtick/dash forms. The
    // dispatch backstop must serialize them on the foundation path.
    let input = ready_input(vec![
        issue(
            900,
            "## Files touched\n\ncrates/autospec-core/src/evaluation/mod.rs\ncrates/autospec-core/src/evaluation/digest.rs\n",
            &["auto-implement", "safety:reviewed"],
        ),
        issue(
            901,
            "## Files touched\n\n- `crates/autospec-core/src/evaluation/mod.rs`\n- crates/autospec-core/src/evaluation/statistics.rs\n",
            &["auto-implement", "safety:reviewed"],
        ),
        issue(
            902,
            "## Files touched\n\ncrates/autospec-core/src/spec/parser.rs\n",
            &["auto-implement", "safety:reviewed"],
        ),
    ]);

    let plan = plan_ready_queue(&input);

    assert_eq!(plan.ready_numbers(), vec![900, 902]);
    assert_eq!(plan.conflicts.len(), 1);
    assert_eq!(plan.conflicts[0].issue.number, 901);
    assert_eq!(
        plan.conflicts[0].reason.as_deref(),
        Some("waits_on_foundation")
    );
    assert_eq!(plan.conflicts[0].conflicts_with, Some(900));
    assert_eq!(
        plan.conflicts[0].path.as_deref(),
        Some("crates/autospec-core/src/evaluation/mod.rs")
    );
    assert_eq!(plan.batch_numbers(), vec![900, 902]);
}

#[test]
fn holds_a_candidate_when_an_active_worker_already_claims_a_files_touched_path() {
    let mut input = ready_input(vec![issue(
        910,
        "## Files touched\n\n- `crates/autospec-core/src/evaluation/mod.rs`\n",
        &["auto-implement", "safety:reviewed"],
    )]);
    input.active.push(issue(
        909,
        "## Files touched\n\ncrates/autospec-core/src/evaluation/mod.rs\n",
        &["in-progress-by-bot"],
    ));

    let plan = plan_ready_queue(&input);

    assert!(plan.ready.is_empty());
    assert_eq!(plan.conflicts.len(), 1);
    assert_eq!(plan.conflicts[0].issue.number, 910);
    assert_eq!(plan.conflicts[0].reason.as_deref(), Some("path_conflict"));
    assert_eq!(plan.conflicts[0].conflicts_with, Some(909));
    assert!(plan.batch.is_empty());
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
    assert_eq!(plan.gate_counts.duplicates, 0);
    assert_eq!(plan.gate_counts.dependency_blocked, 1);
    assert_eq!(plan.gate_counts.linked_pr_blocked, 1);
    assert_eq!(plan.gate_counts.path_conflicted, 1);
    assert_eq!(plan.gate_counts.ready, 1);
    assert_eq!(plan.gate_counts.claimed, 1);
    assert_eq!(plan.gate_counts.selected, 1);
}

const DUP_SPEC: &str = "docs/specs/2026-08-12-language-selection-axis-design.md";

fn dup_body(goal: &str, anchors: &str, path: &str) -> String {
    format!(
        "## Goal\n\n{goal}\n\n## Source spec\n\n`{DUP_SPEC}` {anchors}\n\n## Implementation outline\n\n- edit `{path}`\n"
    )
}

#[test]
fn blocks_the_later_issue_when_spec_citation_and_goal_match() {
    let input = ready_input(vec![
        issue(
            3404,
            dup_body(
                "Add  the\nlanguage selection axis to the define skill.",
                "L209-224 and L240-241",
                "src/b.rs",
            ),
            &["auto-implement", "safety:reviewed"],
        ),
        issue(
            3402,
            dup_body(
                "Add the language selection axis to the define skill.",
                "L209-224 and L240-241",
                "src/a.rs",
            ),
            &["auto-implement", "safety:reviewed"],
        ),
    ]);

    let plan = plan_ready_queue(&input);

    assert_eq!(plan.ready_numbers(), vec![3402]);
    assert_eq!(plan.batch_numbers(), vec![3402]);
    assert_eq!(plan.blocked.len(), 1);
    assert_eq!(plan.blocked[0].issue.number, 3404);
    assert_eq!(plan.blocked[0].reason.as_deref(), Some("duplicate_issue"));
    assert_eq!(plan.blocked[0].duplicate_of, Some(3402));
    assert_eq!(plan.gate_counts.duplicates, 1);
}

#[test]
fn keeps_distinct_goals_that_cite_the_same_spec_section() {
    let input = ready_input(vec![
        issue(
            3402,
            dup_body(
                "Add the language selection axis to the define skill.",
                "L209-224 and L240-241",
                "src/a.rs",
            ),
            &["auto-implement", "safety:reviewed"],
        ),
        issue(
            3404,
            dup_body(
                "Render the selected language axis in the issue body.",
                "L209-224 and L240-241",
                "src/b.rs",
            ),
            &["auto-implement", "safety:reviewed"],
        ),
    ]);

    let plan = plan_ready_queue(&input);

    assert_eq!(plan.ready_numbers(), vec![3402, 3404]);
    assert_eq!(plan.batch_numbers(), vec![3402, 3404]);
    assert!(plan.blocked.is_empty());
    assert_eq!(plan.gate_counts.duplicates, 0);
}

#[test]
fn keeps_distinct_spec_sections_that_share_a_goal() {
    let input = ready_input(vec![
        issue(
            3402,
            dup_body(
                "Add the language selection axis to the define skill.",
                "L209-224 and L240-241",
                "src/a.rs",
            ),
            &["auto-implement", "safety:reviewed"],
        ),
        issue(
            3404,
            dup_body(
                "Add the language selection axis to the define skill.",
                "L300-310",
                "src/b.rs",
            ),
            &["auto-implement", "safety:reviewed"],
        ),
    ]);

    let plan = plan_ready_queue(&input);

    assert_eq!(plan.ready_numbers(), vec![3402, 3404]);
    assert!(plan.blocked.is_empty());
    assert_eq!(plan.gate_counts.duplicates, 0);
}

#[test]
fn leaves_issues_without_a_spec_citation_undeduplicated() {
    let body = |path: &str| {
        format!(
            "## Goal\n\nAdd the language selection axis to the define skill.\n\n## Implementation outline\n\n- edit `{path}`\n"
        )
    };
    let input = ready_input(vec![
        issue(
            3402,
            body("src/a.rs"),
            &["auto-implement", "safety:reviewed"],
        ),
        issue(
            3404,
            body("src/b.rs"),
            &["auto-implement", "safety:reviewed"],
        ),
    ]);

    let plan = plan_ready_queue(&input);

    assert_eq!(plan.ready_numbers(), vec![3402, 3404]);
    assert!(plan.blocked.is_empty());
    assert_eq!(plan.gate_counts.duplicates, 0);
}

#[test]
fn blocks_the_twin_of_an_already_active_issue() {
    let mut input = ready_input(vec![issue(
        3404,
        dup_body(
            "Add the language selection axis to the define skill.",
            "L209-224 and L240-241",
            "src/b.rs",
        ),
        &["auto-implement", "safety:reviewed"],
    )]);
    input.active.push(issue(
        3402,
        dup_body(
            "Add the language selection axis to the define skill.",
            "L209-224 and L240-241",
            "src/a.rs",
        ),
        &["in-progress-by-bot"],
    ));

    let plan = plan_ready_queue(&input);

    assert!(plan.ready.is_empty());
    assert!(plan.batch.is_empty());
    assert_eq!(plan.blocked.len(), 1);
    assert_eq!(plan.blocked[0].issue.number, 3404);
    assert_eq!(plan.blocked[0].reason.as_deref(), Some("duplicate_issue"));
    assert_eq!(plan.blocked[0].duplicate_of, Some(3402));
    assert_eq!(plan.gate_counts.duplicates, 1);
}

#[test]
fn keeps_both_candidates_ready_when_they_share_a_conflict_domain() {
    // Spec §27.6: a conflict domain is scheduling metadata, not a readiness
    // edge. Two issues that name the same conflict domain must both stay
    // ready (and both make the batch) when their write paths differ.
    let body = |path: &str| {
        format!(
            "## Concurrency\n\nParallel safe: yes\n\n### Conflict domains\n- `router-http`\n\n## Implementation outline\n\n- edit `{path}`\n"
        )
    };
    let input = ready_input(vec![
        issue(
            910,
            body("crates/router/src/http.rs"),
            &["auto-implement", "safety:reviewed"],
        ),
        issue(
            911,
            body("crates/router/src/grpc.rs"),
            &["auto-implement", "safety:reviewed"],
        ),
    ]);

    let plan = plan_ready_queue(&input);

    assert_eq!(plan.ready_numbers(), vec![910, 911]);
    assert_eq!(plan.batch_numbers(), vec![910, 911]);
    assert!(plan.blocked.is_empty(), "no issue may be blocked");
    assert!(
        plan.conflicts.is_empty(),
        "a shared conflict domain is not a path conflict"
    );
    for view in &plan.ready {
        assert!(view.unmet_dependencies.is_empty());
        assert!(view.conflicts_with.is_none());
    }
}

#[test]
fn keeps_both_candidates_ready_when_they_share_a_write_surface() {
    // Spec §27.6: shared write surfaces declared in the Concurrency section
    // are metadata; they only become a conflict when the same file also
    // appears in both Implementation outlines.
    let body = |path: &str| {
        format!(
            "## Concurrency\n\nParallel safe: yes\n\n### Exclusive write ownership\n- `{path}`\n\n### Shared write surfaces\n- `crates/core/src/lib.rs`\n\n## Implementation outline\n\n- edit `{path}`\n"
        )
    };
    let input = ready_input(vec![
        issue(
            912,
            body("crates/core/src/a.rs"),
            &["auto-implement", "safety:reviewed"],
        ),
        issue(
            913,
            body("crates/core/src/b.rs"),
            &["auto-implement", "safety:reviewed"],
        ),
    ]);

    let plan = plan_ready_queue(&input);

    assert_eq!(plan.ready_numbers(), vec![912, 913]);
    assert_eq!(plan.batch_numbers(), vec![912, 913]);
    assert!(plan.blocked.is_empty());
    assert!(plan.conflicts.is_empty());
}

#[test]
fn withholds_the_dependent_of_a_hard_dependency_edge() {
    // Spec §27.6: only the `## Dependencies` heading creates readiness edges.
    // `Depends on issue #A` against an open issue must withhold the
    // dependent while the dependency itself stays ready.
    let input = ready_input(vec![
        issue(
            920,
            "## Implementation outline\n\n- edit `src/parent.rs`\n",
            &["auto-implement", "safety:reviewed"],
        ),
        issue(
            921,
            "## Dependencies\n\nDepends on issue #920\n\n## Implementation outline\n\n- edit `src/child.rs`\n",
            &["auto-implement", "safety:reviewed"],
        ),
    ]);

    let plan = plan_ready_queue(&input);

    assert_eq!(plan.ready_numbers(), vec![920]);
    assert_eq!(plan.batch_numbers(), vec![920]);
    assert_eq!(plan.blocked.len(), 1);
    assert_eq!(plan.blocked[0].issue.number, 921);
    assert_eq!(
        plan.blocked[0].reason.as_deref(),
        Some("blocked_dependencies")
    );
    assert_eq!(plan.blocked[0].unmet_dependencies, vec![920]);
    assert!(plan.blocked[0].non_blocking_refs.is_empty());
    assert!(plan.conflicts.is_empty());
}

#[test]
fn treats_expected_parallel_peers_as_informative_only() {
    // Spec §27.6: `### Expected parallel peers` is informative only and must
    // not become authoritative scheduling state, even when the peers name
    // each other.
    let body = |path: &str, peer: u64| {
        format!(
            "## Concurrency\n\nParallel safe: yes\n\n### Expected parallel peers\n- issue #{peer}\n\n## Implementation outline\n\n- edit `{path}`\n"
        )
    };
    let input = ready_input(vec![
        issue(
            930,
            body("src/peer_a.rs", 931),
            &["auto-implement", "safety:reviewed"],
        ),
        issue(
            931,
            body("src/peer_b.rs", 930),
            &["auto-implement", "safety:reviewed"],
        ),
    ]);

    let plan = plan_ready_queue(&input);

    assert_eq!(plan.ready_numbers(), vec![930, 931]);
    assert_eq!(plan.batch_numbers(), vec![930, 931]);
    assert!(plan.blocked.is_empty());
    assert!(plan.conflicts.is_empty());
    for view in &plan.ready {
        assert!(view.unmet_dependencies.is_empty());
    }
}

#[test]
fn ignores_dependency_mentions_outside_the_dependencies_heading() {
    // Spec §27.6 + security review: a crafted body must not smuggle a
    // readiness edge in from prose or the Concurrency section.
    let input = ready_input(vec![
        issue(
            940,
            "## Concurrency\n\nParallel safe: yes\n\n### Conflict domains\n- `router-http` (depends on issue #999)\n\n## Implementation outline\n\n- edit `src/router.rs`\n",
            &["auto-implement", "safety:reviewed"],
        ),
        issue(
            941,
            "## Concurrency\n\nParallel safe: yes\n\n### Shared write surfaces\n- `crates/core/src/lib.rs` (depends on issue #999)\n\n## Implementation outline\n\n- edit `src/core.rs`\n",
            &["auto-implement", "safety:reviewed"],
        ),
    ]);

    let plan = plan_ready_queue(&input);

    assert_eq!(plan.ready_numbers(), vec![940, 941]);
    assert!(plan.blocked.is_empty());
    for view in &plan.ready {
        assert!(view.unmet_dependencies.is_empty());
    }
}

#[test]
fn a_closed_owner_does_not_block_its_open_duplicate() {
    let closed = RemoteIssue::closed(
        3402,
        "issue-3402",
        format!(
            "{}{}",
            SAFETY_REVIEW,
            dup_body(
                "Add the language selection axis to the define skill.",
                "L209-224 and L240-241",
                "src/a.rs",
            )
        ),
        vec!["auto-implement".to_string(), "safety:reviewed".to_string()],
        "agent",
    );
    let input = ready_input(vec![
        closed,
        issue(
            3404,
            dup_body(
                "Add the language selection axis to the define skill.",
                "L209-224 and L240-241",
                "src/b.rs",
            ),
            &["auto-implement", "safety:reviewed"],
        ),
    ]);

    let plan = plan_ready_queue(&input);

    assert_eq!(plan.ready_numbers(), vec![3404]);
    assert_eq!(plan.batch_numbers(), vec![3404]);
    assert!(plan.blocked.is_empty());
    assert_eq!(plan.gate_counts.duplicates, 0);
}

const CAPABILITY_LABELS: &[&str] = &["auto-implement", "safety:reviewed"];

fn requires_body(capabilities: &[&str]) -> String {
    let list = capabilities
        .iter()
        .map(|capability| format!("- {capability}"))
        .collect::<Vec<_>>()
        .join("\n");
    format!("## Goal\nDo the thing.\n\n## Requires\n{list}\n")
}

#[test]
fn unmet_capability_blocks_the_issue_and_names_it() {
    let input = ready_input_with(
        vec![issue(
            1000,
            requires_body(&["gateway:up"]),
            CAPABILITY_LABELS,
        )],
        BTreeMap::from([("gateway:up".to_string(), CapabilityState::Unmet)]),
        BTreeMap::new(),
    );

    let plan = plan_ready_queue(&input);

    assert!(plan.ready.is_empty());
    assert_eq!(plan.blocked.len(), 1);
    let blocked = &plan.blocked[0];
    assert_eq!(blocked.issue.number, 1000);
    assert_eq!(blocked.reason.as_deref(), Some("blocked_capabilities"));
    assert_eq!(blocked.blocked_capabilities, vec!["gateway:up".to_string()]);
    assert_eq!(plan.gate_counts.capability_blocked, 1);
}

#[test]
fn satisfied_capability_re_admits_the_same_issue() {
    let issue = issue(1000, requires_body(&["gateway:up"]), CAPABILITY_LABELS);
    let blocked_input = ready_input_with(
        vec![issue.clone()],
        BTreeMap::from([("gateway:up".to_string(), CapabilityState::Unmet)]),
        BTreeMap::new(),
    );
    let admitted_input = ready_input_with(
        vec![issue],
        BTreeMap::from([("gateway:up".to_string(), CapabilityState::Satisfied)]),
        BTreeMap::new(),
    );

    assert!(plan_ready_queue(&blocked_input).ready.is_empty());
    assert_eq!(
        plan_ready_queue(&admitted_input).ready_numbers(),
        vec![1000]
    );
}

#[test]
fn capability_with_no_observation_fails_closed() {
    let input = ready_input_with(
        vec![issue(
            1000,
            requires_body(&["db:populated"]),
            CAPABILITY_LABELS,
        )],
        BTreeMap::new(),
        BTreeMap::new(),
    );

    let plan = plan_ready_queue(&input);

    assert!(plan.ready.is_empty());
    assert_eq!(
        plan.blocked[0].reason.as_deref(),
        Some("blocked_capabilities")
    );
    assert_eq!(
        plan.blocked[0].blocked_capabilities,
        vec!["db:populated".to_string()]
    );
}

#[test]
fn unmet_capabilities_are_sorted_for_deterministic_holds() {
    let input = ready_input_with(
        vec![issue(
            1000,
            requires_body(&["zeta:up", "alpha:up"]),
            CAPABILITY_LABELS,
        )],
        BTreeMap::new(),
        BTreeMap::new(),
    );

    let plan = plan_ready_queue(&input);

    assert_eq!(
        plan.blocked[0].blocked_capabilities,
        vec!["alpha:up".to_string(), "zeta:up".to_string()]
    );
}

#[test]
fn single_zero_output_run_still_redispatches() {
    let input = ready_input_with(
        vec![issue(2000, "## Goal\nDo the thing.\n", CAPABILITY_LABELS)],
        BTreeMap::new(),
        BTreeMap::from([(2000, 1)]),
    );

    let plan = plan_ready_queue(&input);

    assert_eq!(plan.ready_numbers(), vec![2000]);
    assert!(plan.blocked.is_empty());
    assert!(plan.ready[0].zero_output_streak.is_none());
    assert_eq!(plan.gate_counts.zero_output_review, 0);
}

#[test]
fn two_consecutive_zero_output_runs_route_the_issue_to_review() {
    let input = ready_input_with(
        vec![issue(2000, "## Goal\nDo the thing.\n", CAPABILITY_LABELS)],
        BTreeMap::new(),
        BTreeMap::from([(2000, 2)]),
    );

    let plan = plan_ready_queue(&input);

    assert!(plan.ready.is_empty());
    assert_eq!(plan.blocked.len(), 1);
    let blocked = &plan.blocked[0];
    assert_eq!(blocked.issue.number, 2000);
    assert_eq!(blocked.reason.as_deref(), Some("zero_output_review"));
    assert_eq!(blocked.zero_output_streak, Some(2));
    assert_eq!(plan.gate_counts.zero_output_review, 1);
}

#[test]
fn longer_zero_output_streaks_stay_routed_to_review() {
    let input = ready_input_with(
        vec![issue(2000, "## Goal\nDo the thing.\n", CAPABILITY_LABELS)],
        BTreeMap::new(),
        BTreeMap::from([(2000, 5)]),
    );

    let plan = plan_ready_queue(&input);

    assert_eq!(
        plan.blocked[0].reason.as_deref(),
        Some("zero_output_review")
    );
    assert_eq!(plan.blocked[0].zero_output_streak, Some(5));
}

#[test]
fn populated_frontier_offers_only_work_whose_prerequisites_hold() {
    // #3793-shaped frontier: several open auto-implement issues whose issue
    // dependencies are all closed, but which declare world-state
    // prerequisites. Only the task whose capability is actually satisfied
    // may be offered.
    let gateway_issue = issue(
        3793,
        "## Goal\nDo the thing.\n\n## Dependencies\n\nDepends on #3790\n\n## Requires\n\n- gateway:up\n",
        CAPABILITY_LABELS,
    );
    let database_issue = issue(3794, requires_body(&["db:populated"]), CAPABILITY_LABELS);
    let plain_issue = issue(3795, "## Goal\nDo the thing.\n", CAPABILITY_LABELS);
    let zeroed_issue = issue(3796, "## Goal\nDo the thing.\n", CAPABILITY_LABELS);

    let mut input = ready_input_with(
        vec![gateway_issue, database_issue, plain_issue, zeroed_issue],
        BTreeMap::from([
            ("gateway:up".to_string(), CapabilityState::Satisfied),
            ("db:populated".to_string(), CapabilityState::Unmet),
        ]),
        BTreeMap::from([(3796, 2)]),
    );
    let mut closed_dependency = RemoteIssue::open(3790, "upstream", "", Vec::new(), "agent");
    closed_dependency.closed = true;
    input.dependencies.insert(3790, closed_dependency);

    let plan = plan_ready_queue(&input);

    // The satisfied-capability issue (with a closed dep) and the plain
    // issue are the only offers.
    assert_eq!(plan.ready_numbers(), vec![3793, 3795]);
    // The unmet-capability issue and the twice-zero-output issue are held.
    let blocked_by_reason: BTreeMap<u64, String> = plan
        .blocked
        .iter()
        .map(|view| {
            (
                view.issue.number,
                view.reason.clone().unwrap_or_else(|| "<none>".to_string()),
            )
        })
        .collect();
    assert_eq!(
        blocked_by_reason.get(&3794).map(String::as_str),
        Some("blocked_capabilities")
    );
    assert_eq!(
        blocked_by_reason.get(&3796).map(String::as_str),
        Some("zero_output_review")
    );
    assert_eq!(plan.gate_counts.capability_blocked, 1);
    assert_eq!(plan.gate_counts.zero_output_review, 1);
    assert_eq!(plan.gate_counts.ready, 2);
}

#[test]
fn hold_view_names_the_missing_capability_for_the_hold_message() {
    let input = ready_input_with(
        vec![issue(
            4000,
            requires_body(&["gateway:up", "db:populated"]),
            CAPABILITY_LABELS,
        )],
        BTreeMap::from([("gateway:up".to_string(), CapabilityState::Satisfied)]),
        BTreeMap::new(),
    );

    let plan = plan_ready_queue(&input);

    // gateway:up is satisfied; only db:populated remains, and it is the
    // name a hold message must surface.
    assert_eq!(
        plan.blocked[0].blocked_capabilities,
        vec!["db:populated".to_string()]
    );
    assert_eq!(
        plan.blocked[0].reason.as_deref(),
        Some("blocked_capabilities")
    );
}
