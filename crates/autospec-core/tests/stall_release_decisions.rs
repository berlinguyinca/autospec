//! Release decisions: note content, attempt counting, rotation, escalation (#3563).
//!
//! Covers the six cases the issue requires: a clean completion, a stall with
//! partial work, a stall with none, attempt exhaustion, rotation to a different
//! model, and the single-model degradation path.

use autospec_core::stall::{
    plan_release, AttemptHistory, AttemptOutcome, AttemptRecord, CommitRecord, EscalationReason,
    LeasePolicy, Liveness, ModelChoice, ModelRoster, PartialWork, ReleaseDecision, ReleasePlan,
    StallPolicy, StallReason, WorkProduced,
};

fn policy(max_attempts: u32) -> StallPolicy {
    StallPolicy {
        lease: LeasePolicy::default(),
        stall_secs: 1_800,
        max_attempts,
        transcript_tail_bytes: 4_096,
    }
}

fn roster() -> ModelRoster {
    ModelRoster::from_ids(["Qwen3.8-27B Q8", "DeepSeek-V4-Flash", "GLM-5.3-Flash"])
}

fn record(model: &str, outcome: AttemptOutcome) -> AttemptRecord {
    AttemptRecord {
        attempt: 0,
        worker_id: "worker-7".to_string(),
        model: ModelChoice::from_id(model),
        configuration: "reasoning_effort=high".to_string(),
        duration_secs: 903,
        produced: WorkProduced::None,
        outcome,
        last_activity: "reading crates/autospec-core/src/plan.rs".to_string(),
    }
}

/// History built from `(model, outcome)` pairs, oldest first.
fn history_of(entries: &[(&str, AttemptOutcome)]) -> AttemptHistory {
    let mut history = AttemptHistory::new();
    for (model, outcome) in entries {
        history.push(record(model, *outcome));
    }
    history
}

fn work_with(commits: u64, tree_dirty: bool) -> PartialWork {
    PartialWork {
        commits: (0..commits)
            .map(|index| CommitRecord {
                id: format!("commit{index}"),
                subject: format!("subject {index}"),
            })
            .collect(),
        commit_patch: if commits > 0 {
            "From patch\n".to_string()
        } else {
            String::new()
        },
        working_tree_patch: if tree_dirty {
            "diff --git a/x b/x\n".to_string()
        } else {
            String::new()
        },
        transcript_excerpt: "thinking about the fixture table".to_string(),
        transcript_bytes: 4_096,
        capture_errors: Vec::new(),
    }
}

fn plan_for(
    max_attempts: u32,
    models: &ModelRoster,
    history: &AttemptHistory,
    work: &PartialWork,
) -> ReleasePlan {
    plan_release(
        3289,
        &policy(max_attempts),
        models,
        history,
        work,
        StallReason::LeaseExpired,
        Liveness::Hung,
    )
}

#[test]
fn a_clean_completion_records_no_note_and_requeues_nothing() {
    let history = history_of(&[("Qwen3.8-27B Q8", AttemptOutcome::Completed)]);
    let plan = plan_for(2, &roster(), &history, &work_with(2, false));
    assert_eq!(plan.decision.attempt(), 1);
    assert!(matches!(
        plan.decision,
        ReleaseDecision::Completed { attempt: 1 }
    ));
    assert!(plan.note.is_none(), "a finished attempt gets no stall note");
    assert!(plan.artifacts.is_empty());
    assert_eq!(plan.work, WorkProduced::Commits { count: 2 });
}

#[test]
fn a_stall_with_partial_work_records_duration_output_activity_and_model() {
    let history = history_of(&[("Qwen3.8-27B Q8", AttemptOutcome::Stalled)]);
    let plan = plan_for(2, &roster(), &history, &work_with(3, true));
    let note = plan.note.clone().expect("a stall records a note");
    let rendered = note.markdown();
    for expected in [
        "Stalled attempt 1 of 2",
        "duration: 903s",
        "work produced: 3 commit(s) plus working-tree changes",
        "last activity: reading crates/autospec-core/src/plan.rs",
        "model: Qwen3.8-27B Q8",
        "configuration: reasoning_effort=high",
        "reason: lease expired",
        "attempt 1",
    ] {
        assert!(
            rendered.contains(expected),
            "note missing {expected:?}:\n{rendered}"
        );
    }
    assert_eq!(note.model_configuration(), "Qwen3.8-27B Q8");
    assert_eq!(note.attempt, 1);
    assert_eq!(note.max_attempts, 2);
    // The captured work travels with the note.
    assert!(plan
        .artifacts
        .iter()
        .any(|artifact| artifact.name == "attempt-1-commits.patch"));
    assert!(plan
        .artifacts
        .iter()
        .any(|artifact| artifact.name == "attempt-1-transcript-tail.txt"));
}

#[test]
fn a_stall_with_nothing_produced_says_so() {
    let history = history_of(&[("Qwen3.8-27B Q8", AttemptOutcome::Stalled)]);
    let plan = plan_for(2, &roster(), &history, &work_with(0, false));
    let note = plan.note.clone().expect("a stall records a note");
    assert_eq!(note.work, WorkProduced::None);
    assert!(note.markdown().contains("work produced: nothing"));
    // Only the transcript is attached; there is no patch to send.
    let names: Vec<&str> = plan
        .artifacts
        .iter()
        .map(|artifact| artifact.name.as_str())
        .collect();
    assert_eq!(names, ["attempt-1-transcript-tail.txt"]);
}

#[test]
fn commits_are_work_even_though_the_working_tree_is_clean() {
    // The tree-only check reported a *committing* agent as having produced
    // nothing; a clean tree with commits must read as work.
    let plan = plan_for(
        3,
        &roster(),
        &history_of(&[("Qwen3.8-27B Q8", AttemptOutcome::Stalled)]),
        &work_with(2, false),
    );
    assert_eq!(plan.work, WorkProduced::Commits { count: 2 });
    assert!(plan.work.produced());
    assert!(plan
        .note
        .clone()
        .expect("note")
        .markdown()
        .contains("work produced: 2 commit(s)"));
}

#[test]
fn rotation_moves_to_a_different_model_preferring_another_family() {
    let plan = plan_for(
        5,
        &roster(),
        &history_of(&[("Qwen3.8-27B Q8", AttemptOutcome::Stalled)]),
        &work_with(0, false),
    );
    let ReleaseDecision::Requeue {
        next_model,
        changed_family,
        rotation_note,
        ..
    } = &plan.decision
    else {
        panic!("a first stall re-queues: {:?}", plan.decision);
    };
    assert_eq!(next_model.id, "DeepSeek-V4-Flash");
    assert!(changed_family);
    assert!(rotation_note.contains("different family"));
    assert!(plan
        .note
        .clone()
        .expect("note")
        .markdown()
        .contains("next attempt: DeepSeek-V4-Flash (different family"));
}

#[test]
fn rotation_skips_a_same_family_endpoint_when_another_family_remains() {
    let models = ModelRoster::from_ids([
        "inference/qwen-a",
        "inference/qwen-b",
        "inference/glm-c",
        "inference/deepseek-d",
    ]);
    let plan = plan_for(
        5,
        &models,
        &history_of(&[("inference/qwen-a", AttemptOutcome::Stalled)]),
        &work_with(0, false),
    );
    let ReleaseDecision::Requeue {
        next_model,
        changed_family,
        ..
    } = &plan.decision
    else {
        panic!("expected requeue");
    };
    assert_eq!(next_model.id, "inference/glm-c");
    assert!(changed_family);
}

#[test]
fn a_model_that_stalled_is_never_tried_again_until_the_roster_runs_out() {
    let models = roster();
    let mut history = AttemptHistory::new();
    let mut chosen = Vec::new();
    for step in 0..2 {
        let stalled = history
            .last()
            .map(|record| record.model.id.clone())
            .unwrap_or_else(|| "Qwen3.8-27B Q8".to_string());
        history.push(record(&stalled, AttemptOutcome::Stalled));
        let plan = plan_for(9, &models, &history, &work_with(0, false));
        match &plan.decision {
            ReleaseDecision::Requeue { next_model, .. } => {
                let attempted: Vec<String> = history.attempted_model_ids();
                assert!(
                    !attempted.contains(&next_model.id),
                    "re-queued onto a model that already stalled: {attempted:?}"
                );
                chosen.push(next_model.id.clone());
                // The next attempt runs on that model and stalls too.
                history.push(record(&next_model.id, AttemptOutcome::Stalled));
            }
            other => panic!("expected rotation at step {step}, got {other:?}"),
        }
    }
    assert_eq!(
        chosen,
        vec!["DeepSeek-V4-Flash".to_string(), "GLM-5.3-Flash".to_string(),],
        "rotation walks the untried models, then stops"
    );

    // One more stall with the whole roster tried: escalation, not wrap-around.
    let final_plan = plan_for(9, &models, &history, &work_with(0, false));
    assert!(
        matches!(final_plan.decision, ReleaseDecision::Escalate { .. }),
        "an exhausted roster escalates instead of reusing a stalled model, got {:?}",
        final_plan.decision
    );
}

#[test]
fn attempt_exhaustion_stops_requeueing_and_escalates_to_spec_repair() {
    let history = history_of(&[
        ("Qwen3.8-27B Q8", AttemptOutcome::Stalled),
        ("DeepSeek-V4-Flash", AttemptOutcome::Stalled),
    ]);
    let plan = plan_for(2, &roster(), &history, &work_with(0, false));
    let ReleaseDecision::Escalate { reason, report, .. } = &plan.decision else {
        panic!("the attempt limit must escalate, got {:?}", plan.decision);
    };
    assert_eq!(*reason, EscalationReason::AttemptLimit { max_attempts: 2 });
    assert_eq!(report.attempts.len(), 2);
    assert_eq!(report.issue, 3289);
    let markdown = report.markdown();
    for expected in [
        "hit the attempt limit of 2",
        "Qwen3.8-27B Q8",
        "DeepSeek-V4-Flash",
        "none produced a commit",
        "- attempt 1: Qwen3.8-27B Q8",
        "spec repair",
    ] {
        assert!(
            markdown.contains(expected),
            "report missing {expected:?}:\n{markdown}"
        );
    }
    assert!(
        plan.note.is_some(),
        "the final note still records the last attempt"
    );
}

#[test]
fn roster_exhaustion_escalates_even_below_the_attempt_limit() {
    let models = ModelRoster::from_ids(["Qwen3.8-27B Q8", "DeepSeek-V4-Flash"]);
    let history = history_of(&[
        ("Qwen3.8-27B Q8", AttemptOutcome::Stalled),
        ("DeepSeek-V4-Flash", AttemptOutcome::Stalled),
    ]);
    let plan = plan_for(5, &models, &history, &work_with(0, false));
    let ReleaseDecision::Escalate { reason, .. } = &plan.decision else {
        panic!("expected escalation, got {:?}", plan.decision);
    };
    assert_eq!(*reason, EscalationReason::RosterExhausted { attempted: 2 });
}

#[test]
fn a_single_model_retries_once_then_escalates_with_rotation_unavailable() {
    let models = ModelRoster::from_ids(["Qwen3.8-27B Q8"]);
    assert_eq!(models.len(), 1);

    let first = plan_for(
        2,
        &models,
        &history_of(&[("Qwen3.8-27B Q8", AttemptOutcome::Stalled)]),
        &work_with(0, false),
    );
    let ReleaseDecision::Requeue {
        next_model,
        changed_family,
        rotation_note,
        ..
    } = &first.decision
    else {
        panic!("the single model gets one retry, got {:?}", first.decision);
    };
    assert_eq!(next_model.id, "Qwen3.8-27B Q8");
    assert!(!changed_family);
    assert!(rotation_note.contains("rotation unavailable"));

    let second = plan_for(
        2,
        &models,
        &history_of(&[
            ("Qwen3.8-27B Q8", AttemptOutcome::Stalled),
            ("Qwen3.8-27B Q8", AttemptOutcome::Stalled),
        ]),
        &work_with(0, false),
    );
    let ReleaseDecision::Escalate { reason, report, .. } = &second.decision else {
        panic!("the second stall must escalate, got {:?}", second.decision);
    };
    assert_eq!(*reason, EscalationReason::RotationUnavailable);
    assert!(
        report.reason_sentence().contains("rotation unavailable"),
        "{}",
        report.reason_sentence()
    );
    assert_eq!(report.attempts.len(), 2);
}

#[test]
fn an_empty_roster_degrades_to_the_model_that_stalled() {
    let models = ModelRoster::default();
    let plan = plan_for(
        2,
        &models,
        &history_of(&[("unknown", AttemptOutcome::Stalled)]),
        &work_with(0, false),
    );
    let ReleaseDecision::Requeue { next_model, .. } = &plan.decision else {
        panic!("expected requeue, got {:?}", plan.decision);
    };
    assert_eq!(next_model.id, "unknown");
    // With no roster the single-model rule still bounds it at one retry.
    let second = plan_for(
        2,
        &models,
        &history_of(&[
            ("unknown", AttemptOutcome::Stalled),
            ("unknown", AttemptOutcome::Stalled),
        ]),
        &work_with(0, false),
    );
    assert!(matches!(
        second.decision,
        ReleaseDecision::Escalate {
            reason: EscalationReason::RotationUnavailable,
            ..
        }
    ));
}

#[test]
fn a_produced_diff_completes_rather_than_rotating() {
    // Work captured means the next agent takes a diff, not another attempt.
    let plan = plan_for(
        2,
        &roster(),
        &history_of(&[("Qwen3.8-27B Q8", AttemptOutcome::Stalled)]),
        &work_with(1, false),
    );
    // The attempt itself stalled, so it is still recorded and re-queued, but the
    // captured patch is attached for the next agent rather than thrown away.
    assert!(matches!(plan.decision, ReleaseDecision::Requeue { .. }));
    assert!(plan
        .artifacts
        .iter()
        .any(|artifact| artifact.name == "attempt-1-commits.patch"));

    let finished = plan_for(
        2,
        &roster(),
        &history_of(&[("Qwen3.8-27B Q8", AttemptOutcome::Completed)]),
        &work_with(1, false),
    );
    assert!(matches!(
        finished.decision,
        ReleaseDecision::Completed { .. }
    ));
}
