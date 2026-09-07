use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::Path;

use autospec_core::coordination::RemoteIssue;
use autospec_core::spec_repair::{
    acceptance_criteria_state, append_repair_event, check_spec_repair, classify_ambiguity_stakes,
    classify_maintainer_reply, load_repair_events, proposal_count, propose_spec_repair,
    render_pr_assumption_section, should_escalate_to_human, should_trigger_spec_review,
    summarize_by_origin_template, summarize_by_shape, AmbiguityStakes, CheckOutcome,
    IssueCommentSnapshot, IssueRepairTracker, MaintainerReply, ProposeOutcome, ProposedCriterion,
    SpecDefectShape, SpecRepairEvent, SpecRepairProposalInput, StatedAssumption, APPROVAL_MARKER,
    ASSUMPTION_HEADING, ESCALATION_MARKER, NEEDS_SPEC_CLARIFICATION_LABEL, PROPOSAL_MARKER,
    SPEC_REVIEW_STALL_THRESHOLD,
};

/// An in-memory tracker so tests exercise exactly the surface the real adapter has —
/// including the absence of any way to write an issue body.
struct InMemoryTracker {
    issue: RefCell<RemoteIssue>,
    comments: RefCell<Vec<IssueCommentSnapshot>>,
}

impl InMemoryTracker {
    fn new(body: &str, labels: &[&str]) -> Self {
        Self {
            issue: RefCell::new(RemoteIssue::open(
                3541,
                "Repair loop for unusable issues",
                body,
                labels.iter().map(|label| label.to_string()).collect(),
                "octo-author",
            )),
            comments: RefCell::new(Vec::new()),
        }
    }

    fn body(&self) -> String {
        self.issue.borrow().body.clone()
    }

    fn labels(&self) -> Vec<String> {
        self.issue.borrow().labels.clone()
    }
}

impl IssueRepairTracker for InMemoryTracker {
    fn read_issue(&self, _repo: &str, _number: u64) -> std::io::Result<RemoteIssue> {
        Ok(self.issue.borrow().clone())
    }

    fn post_comment(&self, _repo: &str, _number: u64, body: &str) -> std::io::Result<()> {
        self.comments.borrow_mut().push(IssueCommentSnapshot {
            author: "autospec-agent".to_string(),
            body: body.to_string(),
        });
        Ok(())
    }

    fn add_label(&self, _repo: &str, _number: u64, label: &str) -> std::io::Result<()> {
        let mut issue = self.issue.borrow_mut();
        if !issue.labels.iter().any(|existing| existing == label) {
            issue.labels.push(label.to_string());
        }
        Ok(())
    }

    fn remove_label(&self, _repo: &str, _number: u64, label: &str) -> std::io::Result<()> {
        self.issue
            .borrow_mut()
            .labels
            .retain(|existing| existing != label);
        Ok(())
    }

    fn list_comments(
        &self,
        _repo: &str,
        _number: u64,
    ) -> std::io::Result<Vec<IssueCommentSnapshot>> {
        Ok(self.comments.borrow().clone())
    }
}

fn dead_end_body() -> String {
    "## Goal\nShip the thing.\n\n## Acceptance criteria\n- [x] existing behavior kept\n- [x] docs updated\n".to_string()
}

fn open_body() -> String {
    "## Goal\nShip the thing.\n\n## Acceptance criteria\n- [ ] not done yet\n".to_string()
}

fn mixed_body() -> String {
    "## Goal\nShip the thing.\n\n## Acceptance criteria\n- [x] done\n- [ ] not done\n".to_string()
}

fn proposal_input() -> SpecRepairProposalInput {
    SpecRepairProposalInput {
        shape: SpecDefectShape::Ambiguous,
        reading: "The criterion means the CLI prints one JSON line on success.".to_string(),
        criteria: vec![ProposedCriterion {
            requirement:
                "`cargo build --workspace && cargo fmt --check && echo SMOKE_OK` prints SMOKE_OK"
                    .to_string(),
            check_command: "cargo build --workspace && cargo fmt --check && echo SMOKE_OK"
                .to_string(),
            current_exit_status: 1,
        }],
        question: "Should the smoke gate run fmt, or only build?".to_string(),
        checked: vec![
            "read `docs/cli-reference.md` — no smoke gate is defined".to_string(),
            "grep for `SMOKE_OK` across `crates/` — only referenced in issue text".to_string(),
        ],
    }
}

fn event(repo: &str, issue: u64, shape: &str, template: &str) -> SpecRepairEvent {
    SpecRepairEvent {
        recorded_at: 1_700_000_000,
        repo: repo.to_string(),
        issue,
        shape: shape.to_string(),
        origin_template: template.to_string(),
        origin_command: "autospec decompose".to_string(),
        origin_author: "octo-author".to_string(),
    }
}

#[test]
fn criteria_states_cover_the_four_dispatch_cases() {
    assert_eq!(acceptance_criteria_state("").id(), "no_criteria");
    assert_eq!(
        acceptance_criteria_state("no section here").id(),
        "no_criteria"
    );
    assert_eq!(
        acceptance_criteria_state("## Acceptance criteria\n- [ ] open\n").id(),
        "open_criteria"
    );
    assert_eq!(
        acceptance_criteria_state(&dead_end_body()).id(),
        "all_satisfied"
    );
    assert_eq!(acceptance_criteria_state(&mixed_body()).id(), "mixed");
    assert_eq!(
        acceptance_criteria_state(&open_body()).id(),
        "open_criteria"
    );
}

#[test]
fn criterion_fails_today_only_on_nonzero_exit_status() {
    let failing = ProposedCriterion {
        requirement: "r".to_string(),
        check_command: "cmd".to_string(),
        current_exit_status: 2,
    };
    let passing = ProposedCriterion {
        current_exit_status: 0,
        ..failing.clone()
    };
    assert!(failing.fails_today());
    assert!(!passing.fails_today());
}

#[test]
fn rendered_comment_has_all_five_parts_and_states_it_is_not_a_decision() {
    let proposal = proposal_input().into_proposal(3541);
    let comment = proposal.render_comment();
    assert!(comment.contains(PROPOSAL_MARKER));
    assert!(comment.contains("a proposal, not a decision"));
    assert!(comment.contains("The defect"));
    assert!(comment.contains("What we think it means"));
    assert!(comment.contains("Proposed acceptance criteria"));
    assert!(comment.contains("The question"));
    assert!(comment.contains("What was checked"));
    assert!(comment.contains("`cargo build --workspace && cargo fmt --check && echo SMOKE_OK`"));
    assert!(comment.contains("current exit status: 1 (fails today)"));
    assert!(comment.contains(APPROVAL_MARKER));
    assert!(comment.contains("the issue body was not modified"));
    assert!(!comment.contains("please clarify"));
}

#[test]
fn validation_rejects_proposals_that_are_not_repairs() {
    let mut input = proposal_input();
    input.criteria[0].current_exit_status = 0;
    let errors = input.validate().unwrap_err();
    assert!(
        !errors.is_empty(),
        "a criterion that passes today is not a repair"
    );

    let mut input = proposal_input();
    input.question = "please clarify the smoke gate".to_string();
    assert!(
        input.validate().is_err(),
        "hedged question without named answers"
    );

    let mut input = proposal_input();
    input.question = "no question mark here".to_string();
    assert!(input.validate().is_err());

    let mut input = proposal_input();
    input.reading = "  ".to_string();
    assert!(input.validate().is_err());

    let mut input = proposal_input();
    input.checked = Vec::new();
    assert!(input.validate().is_err());

    let mut input = proposal_input();
    input.criteria = Vec::new();
    assert!(input.validate().is_err());

    proposal_input().validate().expect("valid input passes");
}

#[test]
fn defect_shapes_round_trip_through_ids() {
    for shape in SpecDefectShape::all() {
        assert_eq!(SpecDefectShape::from_id(shape.id()), Some(*shape));
        assert!(!shape.headline().is_empty());
    }
    assert_eq!(SpecDefectShape::from_id("nonsense"), None);
}

#[test]
fn propose_posts_comment_and_label_without_touching_the_body() {
    let tracker = InMemoryTracker::new(&dead_end_body(), &[]);
    let outcome = propose_spec_repair(&tracker, "test/repo", 3541, proposal_input(), 0, false)
        .expect("propose on a dead-end issue");
    let ProposeOutcome::Posted { comment } = outcome else {
        panic!("expected a posted proposal, got {outcome:?}");
    };
    assert!(comment.contains(PROPOSAL_MARKER));
    assert_eq!(
        tracker.labels(),
        vec![NEEDS_SPEC_CLARIFICATION_LABEL.to_string()]
    );
    assert_eq!(
        tracker.body(),
        dead_end_body(),
        "the issue body must never change"
    );
}

#[test]
fn propose_refuses_issues_that_are_not_judged_unusable() {
    for body in [
        open_body(),
        mixed_body(),
        "## Goal\nno criteria at all\n".to_string(),
    ] {
        let tracker = InMemoryTracker::new(&body, &[]);
        let error = propose_spec_repair(&tracker, "test/repo", 3541, proposal_input(), 0, false)
            .expect_err("propose without judged-unusable on a workable issue must refuse");
        assert!(error
            .to_string()
            .contains("not in the mechanical dead-end state"));
        assert!(
            tracker.comments.borrow().is_empty(),
            "no comment may be posted"
        );
    }
    // With judged-unusable the same bodies go through: the pipeline read the issue
    // closely and found a defect the criteria check cannot see.
    let tracker = InMemoryTracker::new(&open_body(), &[]);
    let outcome = propose_spec_repair(&tracker, "test/repo", 3541, proposal_input(), 0, true)
        .expect("judged-unusable proposes anyway");
    assert!(matches!(outcome, ProposeOutcome::Posted { .. }));
}

#[test]
fn propose_is_idempotent_while_a_proposal_is_open() {
    let tracker = InMemoryTracker::new(&dead_end_body(), &[]);
    propose_spec_repair(&tracker, "test/repo", 3541, proposal_input(), 0, false).unwrap();
    let second =
        propose_spec_repair(&tracker, "test/repo", 3541, proposal_input(), 1, false).unwrap();
    assert_eq!(second, ProposeOutcome::AlreadyProposed);
    assert_eq!(tracker.comments.borrow().len(), 1, "no duplicate proposal");
    assert_eq!(
        tracker.labels(),
        vec![NEEDS_SPEC_CLARIFICATION_LABEL.to_string()]
    );
}

#[test]
fn two_prior_proposals_escalate_instead_of_looping() {
    let tracker = InMemoryTracker::new(&dead_end_body(), &[NEEDS_SPEC_CLARIFICATION_LABEL]);
    let outcome =
        propose_spec_repair(&tracker, "test/repo", 3541, proposal_input(), 2, false).unwrap();
    assert_eq!(outcome, ProposeOutcome::EscalatedToHuman);
    let comments = tracker.comments.borrow();
    assert_eq!(comments.len(), 1, "only the escalation notice is posted");
    assert!(comments[0].body.contains(ESCALATION_MARKER));
    assert!(comments[0]
        .body
        .contains("a maintainer must rewrite the issue"));
    drop(comments);
    // The escalation is idempotent: a repeat run must not spam a second notice.
    let outcome =
        propose_spec_repair(&tracker, "test/repo", 3541, proposal_input(), 2, false).unwrap();
    assert_eq!(outcome, ProposeOutcome::EscalatedToHuman);
    assert_eq!(tracker.comments.borrow().len(), 1);
    assert_eq!(
        tracker.body(),
        dead_end_body(),
        "body untouched even on escalation"
    );
}

#[test]
fn escalation_threshold_is_two_prior_proposals() {
    assert!(!should_escalate_to_human(0));
    assert!(!should_escalate_to_human(1));
    assert!(should_escalate_to_human(2));
    assert!(should_escalate_to_human(3));
}

#[test]
fn maintainer_replies_classify_only_on_their_own_lines() {
    assert_eq!(
        classify_maintainer_reply("spec-repair: approved"),
        MaintainerReply::Approval
    );
    assert_eq!(
        classify_maintainer_reply("  SPEC-REPAIR: APPROVED  \n"),
        MaintainerReply::Approval,
        "decision markers are case-insensitive and whitespace-tolerant"
    );
    assert_eq!(
        classify_maintainer_reply("spec-repair: rejected: the reading misses the salt form"),
        MaintainerReply::Rejection {
            reason: Some("the reading misses the salt form".to_string())
        }
    );
    assert_eq!(
        classify_maintainer_reply("LGTM, ship it"),
        MaintainerReply::Other
    );
    assert_eq!(
        classify_maintainer_reply("quoting the banner inline: spec-repair: approved"),
        MaintainerReply::Other,
        "an inline mention is not a decision"
    );
}

#[test]
fn check_classifies_replies_after_the_latest_proposal_only() {
    let tracker = InMemoryTracker::new(&dead_end_body(), &[]);
    assert_eq!(
        check_spec_repair(&tracker, "test/repo", 3541).unwrap(),
        CheckOutcome::NoProposal
    );

    propose_spec_repair(&tracker, "test/repo", 3541, proposal_input(), 0, false).unwrap();
    assert_eq!(
        check_spec_repair(&tracker, "test/repo", 3541).unwrap(),
        CheckOutcome::AwaitingMaintainer
    );

    // An unrelated comment does not resolve the proposal.
    tracker.comments.borrow_mut().push(IssueCommentSnapshot {
        author: "octo-bystander".to_string(),
        body: "watching this one".to_string(),
    });
    assert_eq!(
        check_spec_repair(&tracker, "test/repo", 3541).unwrap(),
        CheckOutcome::AwaitingMaintainer
    );

    // Approval removes the label; the body still carries no decision.
    tracker.comments.borrow_mut().push(IssueCommentSnapshot {
        author: "octo-maintainer".to_string(),
        body: "spec-repair: approved\n".to_string(),
    });
    assert_eq!(
        check_spec_repair(&tracker, "test/repo", 3541).unwrap(),
        CheckOutcome::Approved {
            label_removed: true
        }
    );
    assert!(tracker.labels().is_empty());
    assert_eq!(
        tracker.body(),
        dead_end_body(),
        "approval must not rewrite the body"
    );

    // Approval is idempotent: nothing to remove the second time.
    assert_eq!(
        check_spec_repair(&tracker, "test/repo", 3541).unwrap(),
        CheckOutcome::Approved {
            label_removed: false
        }
    );
}

#[test]
fn rejection_keeps_the_label_on() {
    let tracker = InMemoryTracker::new(&dead_end_body(), &[]);
    propose_spec_repair(&tracker, "test/repo", 3541, proposal_input(), 0, false).unwrap();
    tracker.comments.borrow_mut().push(IssueCommentSnapshot {
        author: "octo-maintainer".to_string(),
        body: "spec-repair: rejected: the proposed criteria test the wrong command\n".to_string(),
    });
    assert_eq!(
        check_spec_repair(&tracker, "test/repo", 3541).unwrap(),
        CheckOutcome::Rejected
    );
    assert_eq!(
        tracker.labels(),
        vec![NEEDS_SPEC_CLARIFICATION_LABEL.to_string()],
        "a rejected proposal leaves the issue needing clarification"
    );
}

#[test]
fn ledger_round_trips_and_counts_by_issue() {
    let dir = std::env::temp_dir().join(format!("autospec-repair-ledger-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = Path::new(&dir).join("ledger.jsonl");
    let _ = std::fs::remove_file(&path);

    assert!(
        load_repair_events(&path).unwrap().is_empty(),
        "missing ledger is empty"
    );

    append_repair_event(&path, &event("test/repo", 3541, "ambiguous", "tpl-a")).unwrap();
    append_repair_event(&path, &event("test/repo", 3541, "ambiguous", "tpl-a")).unwrap();
    append_repair_event(&path, &event("test/repo", 4000, "over_scoped", "tpl-b")).unwrap();
    append_repair_event(&path, &event("other/repo", 3541, "contradictory", "tpl-a")).unwrap();

    let events = load_repair_events(&path).unwrap();
    assert_eq!(events.len(), 4);
    assert_eq!(proposal_count(&events, "test/repo", 3541), 2);
    assert_eq!(proposal_count(&events, "test/repo", 4000), 1);
    assert_eq!(proposal_count(&events, "other/repo", 3541), 1);
    assert_eq!(proposal_count(&events, "test/repo", 9999), 0);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn ledger_summaries_surface_systemic_defect_sources() {
    let events = vec![
        event("test/repo", 3541, "ambiguous", "tpl-a"),
        event("test/repo", 4000, "ambiguous", "tpl-a"),
        event("test/repo", 4001, "over_scoped", "tpl-b"),
    ];
    let by_shape: BTreeMap<String, usize> = summarize_by_shape(&events);
    assert_eq!(by_shape.get("ambiguous"), Some(&2));
    assert_eq!(by_shape.get("over_scoped"), Some(&1));
    let by_template: BTreeMap<String, usize> = summarize_by_origin_template(&events);
    assert_eq!(by_template.get("tpl-a"), Some(&2));
    assert_eq!(by_template.get("tpl-b"), Some(&1));

    let unknown = vec![{
        let mut event = event("test/repo", 1, "ambiguous", "");
        event.origin_command = String::new();
        event
    }];
    let by_template: BTreeMap<String, usize> = summarize_by_origin_template(&unknown);
    assert_eq!(by_template.get("(unknown)"), Some(&1));
}

#[test]
fn corrupt_ledger_lines_fail_loudly() {
    let dir = std::env::temp_dir().join(format!("autospec-repair-corrupt-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = Path::new(&dir).join("ledger.jsonl");
    std::fs::write(&path, "{not json}\n").unwrap();
    assert!(load_repair_events(&path).is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn stakes_rule_blocks_high_stakes_and_allows_low_stakes() {
    let high = [
        "Rotate the credential used by the deploy pipeline",
        "Migration of the events table to add the tenant column",
        "Make `parse_config` a public interface for downstream crates",
        "Drop the staging table — the delete is destructive and irreversible",
    ];
    for subject in high {
        assert_eq!(
            classify_ambiguity_stakes(subject),
            AmbiguityStakes::High,
            "high-stakes subject: {subject}"
        );
    }
    assert_eq!(
        classify_ambiguity_stakes("Pick between JSON and JSONL for the report"),
        AmbiguityStakes::Low
    );
    assert_eq!(
        classify_ambiguity_stakes("Choose the log level for the new module"),
        AmbiguityStakes::Low
    );
}

#[test]
fn assumption_sections_render_for_low_stakes_and_refuse_high_stakes() {
    let assumption = StatedAssumption {
        statement: "The report format stays JSON; JSONL is only a display concern.".to_string(),
        alternatives_considered: vec!["JSONL as the stored format".to_string()],
        rollback: "Revert the PR; the format lives in one serializer module.".to_string(),
    };
    let section = render_pr_assumption_section(&assumption, AmbiguityStakes::Low).unwrap();
    assert!(section.contains(ASSUMPTION_HEADING));
    assert!(section.contains("The report format stays JSON"));
    assert!(section.contains("JSONL as the stored format"));
    assert!(section.contains("Revert the PR"));

    let error = render_pr_assumption_section(&assumption, AmbiguityStakes::High).unwrap_err();
    assert!(error.contains("high-stakes"));
}

#[test]
fn stall_threshold_triggers_spec_review_at_two_runs() {
    assert_eq!(SPEC_REVIEW_STALL_THRESHOLD, 2);
    assert!(!should_trigger_spec_review(0));
    assert!(!should_trigger_spec_review(1));
    assert!(should_trigger_spec_review(2));
    assert!(should_trigger_spec_review(5));
}
