use std::collections::BTreeMap;

use autospec_core::autonomous::drain::{
    decide, DrainDecision, DrainExecutorInput, DrainObservation, DrainProgress,
};
use autospec_core::coordination::{
    plan_ready_queue, PullRequestEvidence, QueuePolicy, ReadyQueueInput, RemoteIssue,
};

const SAFETY_REVIEW: &str = "## Safety review\n\n<!-- autospec-safety:begin -->\n- **decision:** `SAFETY_PASS`\n<!-- autospec-safety:end -->\n\n";

#[test]
fn completed_child_precedes_a_stall_timeout() {
    let observation = DrainObservation::completed(124, 30, 30);

    assert_eq!(
        decide(&observation),
        DrainDecision::Complete { exit_code: 124 }
    );
}

#[test]
fn output_or_artifact_progress_keeps_a_live_child_running() {
    for progress in [DrainProgress::ChildOutput, DrainProgress::Artifact] {
        let observation = DrainObservation::live(30, 30, progress);

        assert_eq!(decide(&observation), DrainDecision::Wait, "{progress:?}");
    }
}

#[test]
fn external_progress_warns_instead_of_terminating_a_quiet_child() {
    for progress in [DrainProgress::Heartbeat, DrainProgress::Github] {
        let observation = DrainObservation::live(30, 30, progress);

        assert_eq!(
            decide(&observation),
            DrainDecision::WarnExternalProgress,
            "{progress:?}"
        );
    }
}

#[test]
fn silent_live_child_is_terminated_only_after_the_stall_window() {
    let waiting = DrainObservation::live(29, 30, DrainProgress::None);
    let stalled = DrainObservation::live(30, 30, DrainProgress::None);

    assert_eq!(decide(&waiting), DrainDecision::Wait);
    assert_eq!(decide(&stalled), DrainDecision::TerminateStalled);
}

#[test]
fn drain_executor_input_preserves_autospec_run_as_literal_typed_data() {
    let input = DrainExecutorInput::omx_autospec_run("/repo").expect("typed drain executor input");

    assert_eq!(input.program(), "omx");
    assert_eq!(input.skill_identifier(), "$autospec-run");
    assert_eq!(
        input.arguments(),
        [
            "exec",
            "--cd",
            "/repo",
            "--dangerously-bypass-approvals-and-sandbox",
            "$autospec-run"
        ]
    );
}

#[test]
fn tier1_queue_admission_accepts_literal_autospec_run_token_in_issue_body() {
    let issue = RemoteIssue::open(
        1600,
        "Tier-1 literal token regression",
        format!(
            "{SAFETY_REVIEW}## Goal\n\nPreserve `$autospec-run` as typed data.\n\n## Implementation outline\n\n- edit `crates/autospec-core/src/autonomous/drain.rs`\n"
        ),
        vec!["auto-implement".to_string(), "safety:reviewed".to_string()],
        "berlinguyinca",
    );
    let input = ReadyQueueInput {
        candidates: vec![issue],
        active: Vec::new(),
        dependencies: BTreeMap::new(),
        pull_requests: PullRequestEvidence::Available(Vec::new()),
        policy: QueuePolicy::new(1, 0),
    };

    let plan = plan_ready_queue(&input);

    assert_eq!(plan.ready_numbers(), vec![1600]);
    assert_eq!(plan.batch_numbers(), vec![1600]);
    assert!(plan.blocked.is_empty());
}
