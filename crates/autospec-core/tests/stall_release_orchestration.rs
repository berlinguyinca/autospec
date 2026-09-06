//! Orchestration: what `StallRelease::finish` actually does with a tracker.
//!
//! The decision function is pure; this file pins the side-effecting wrapper
//! around it, because that is where the original bug lived — evidence was
//! thrown away before anything could see it. Four properties matter:
//!
//! * capture happens before any tracker call, so a tracker outage cannot race
//!   the worktree out of existence;
//! * captured work is stored locally *and* attached, and an attachment failure
//!   degrades to an error record rather than a loss;
//! * a stall re-queues with the next model, an exhausted issue escalates;
//! * everything the release path does goes through `IssueTracker`, so a
//!   non-GitHub tracker receives the same information.

use std::cell::RefCell;
use std::fs;
use std::path::PathBuf;
use std::rc::Rc;

use autospec_core::stall::{
    Artifact, ArtifactStore, AttemptHistory, AttemptOutcome, AttemptRecord, CommitRecord,
    FinishedAttempt, IssueRef, IssueTracker, LeasePolicy, Liveness, ModelChoice, ModelRoster,
    ReleaseDecision, SpecRepairReport, StallPolicy, StallReason, StallRelease, TrackerError,
    WorkProduced, WorktreeEvidence,
};

/// One ordered log shared by the evidence source and the tracker, so test
/// assertions are about *ordering*, not just occurrence.
type Log = Rc<RefCell<Vec<String>>>;

struct FakeEvidence {
    log: Log,
    commits: usize,
    tree_dirty: bool,
    transcript: String,
}

impl FakeEvidence {
    fn new(log: &Log, commits: usize, tree_dirty: bool) -> Self {
        Self {
            log: log.clone(),
            commits,
            tree_dirty,
            transcript: "assistant: reading the fixture table\n".to_string(),
        }
    }
}

impl WorktreeEvidence for FakeEvidence {
    fn commits_ahead_of_base(&self, base: &str) -> Result<Vec<CommitRecord>, String> {
        self.log
            .borrow_mut()
            .push(format!("capture:commits:{base}"));
        Ok((0..self.commits)
            .map(|index| CommitRecord {
                id: format!("deadbeef{index}"),
                subject: format!("implement part {index}"),
            })
            .collect())
    }

    fn commit_patch(&self, base: &str) -> Result<String, String> {
        self.log.borrow_mut().push(format!("capture:patch:{base}"));
        if self.commits == 0 {
            return Ok(String::new());
        }
        Ok("From deadbeef0 implement part 0\n".to_string())
    }

    fn working_tree_patch(&self) -> Result<String, String> {
        self.log.borrow_mut().push("capture:worktree".to_string());
        if self.tree_dirty {
            return Ok("diff --git a/src/plan.rs b/src/plan.rs\n".to_string());
        }
        Ok(String::new())
    }

    fn transcript_tail(&self, max_bytes: usize) -> Result<(String, u64), String> {
        self.log.borrow_mut().push("capture:transcript".to_string());
        let total = self.transcript.len() as u64;
        Ok((
            self.transcript[..self.transcript.len().min(max_bytes)].to_string(),
            total,
        ))
    }
}

#[derive(Clone)]
struct RecordingTracker {
    log: Log,
    counter: u32,
    fail_attach: bool,
    released: Rc<RefCell<Vec<(IssueRef, String)>>>,
    labels: Rc<RefCell<Vec<(IssueRef, String)>>>,
    escalations: Rc<RefCell<Vec<(IssueRef, SpecRepairReport)>>>,
}

impl RecordingTracker {
    fn new(log: &Log) -> Self {
        Self {
            log: log.clone(),
            counter: 0,
            fail_attach: false,
            released: Rc::new(RefCell::new(Vec::new())),
            labels: Rc::new(RefCell::new(Vec::new())),
            escalations: Rc::new(RefCell::new(Vec::new())),
        }
    }
}

impl IssueTracker for RecordingTracker {
    fn release_to_queue(&mut self, issue: &IssueRef, note: &str) -> Result<(), TrackerError> {
        self.log.borrow_mut().push("tracker:release".to_string());
        self.released
            .borrow_mut()
            .push((issue.clone(), note.to_string()));
        Ok(())
    }

    fn bump_attempt_counter(&mut self, issue: &IssueRef) -> Result<u32, TrackerError> {
        let _ = issue;
        self.log.borrow_mut().push("tracker:bump".to_string());
        self.counter += 1;
        Ok(self.counter)
    }

    fn attach(&mut self, issue: &IssueRef, artifact: &Artifact) -> Result<(), TrackerError> {
        let _ = issue;
        self.log
            .borrow_mut()
            .push(format!("tracker:attach:{}", artifact.name));
        if self.fail_attach {
            return Err(TrackerError::new("attachment endpoint is down"));
        }
        Ok(())
    }

    fn add_label(&mut self, issue: &IssueRef, label: &str) -> Result<(), TrackerError> {
        self.log.borrow_mut().push(format!("tracker:label:{label}"));
        self.labels
            .borrow_mut()
            .push((issue.clone(), label.to_string()));
        Ok(())
    }

    fn escalate_to_spec_repair(
        &mut self,
        issue: &IssueRef,
        report: &SpecRepairReport,
    ) -> Result<(), TrackerError> {
        self.log.borrow_mut().push("tracker:escalate".to_string());
        self.escalations
            .borrow_mut()
            .push((issue.clone(), report.clone()));
        Ok(())
    }
}

fn temp_dir(tag: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "autospec-stall-{}-{}-{}",
        tag,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or(0)
    ));
    fs::create_dir_all(&path).expect("temp dir");
    path
}

fn policy(max_attempts: u32) -> StallPolicy {
    StallPolicy {
        lease: LeasePolicy::default(),
        stall_secs: 1_800,
        max_attempts,
        transcript_tail_bytes: 4_096,
    }
}

fn roster() -> ModelRoster {
    ModelRoster::from_ids(["Qwen3.8-27B Q8", "DeepSeek-V4-Flash"])
}

fn attempt(model: &str, duration_secs: u64) -> AttemptRecord {
    AttemptRecord {
        attempt: 0,
        worker_id: "worker-7".to_string(),
        model: ModelChoice::from_id(model),
        configuration: "reasoning_effort=high".to_string(),
        duration_secs,
        produced: WorkProduced::None,
        outcome: AttemptOutcome::Stalled,
        last_activity: "reading crates/autospec-core/src/plan.rs".to_string(),
    }
}

struct Harness {
    log: Log,
    store_root: PathBuf,
    issue: IssueRef,
}

impl Harness {
    fn new(project: &str) -> Self {
        Self {
            log: Rc::new(RefCell::new(Vec::new())),
            store_root: temp_dir("release"),
            issue: IssueRef::new(project, 3289),
        }
    }

    fn ops(&self) -> Vec<String> {
        self.log.borrow().clone()
    }
}

#[test]
fn evidence_is_captured_before_the_tracker_is_contacted() {
    let harness = Harness::new("owner/name");
    let mut tracker = RecordingTracker::new(&harness.log);
    let evidence = FakeEvidence::new(&harness.log, 2, true);

    let mut release = StallRelease::new(
        &mut tracker,
        ArtifactStore::new(&harness.store_root),
        policy(5),
        roster(),
    );
    let mut history = AttemptHistory::new();
    let result = release
        .finish(
            &harness.issue,
            &mut history,
            FinishedAttempt {
                record: attempt("Qwen3.8-27B Q8", 903),
                reason: StallReason::LeaseExpired,
                liveness: Liveness::Hung,
                base: "base-rev".to_string(),
                evidence: &evidence,
            },
        )
        .expect("finish");

    let ops = harness.ops();
    let first_capture = ops
        .iter()
        .position(|op| op.starts_with("capture:"))
        .expect("capture ran");
    let first_tracker = ops
        .iter()
        .position(|op| op.starts_with("tracker:"))
        .expect("tracker ran");
    assert!(
        first_capture < first_tracker,
        "tracker was contacted before capture: {ops:?}"
    );
    assert_eq!(
        result.plan.work,
        WorkProduced::CommitsAndWorkingTree { count: 2 },
        "the captured work, not the record's stale field, is what gets reported"
    );
}

#[test]
fn a_stalled_attempt_is_stored_then_requeued_onto_the_next_model() {
    let harness = Harness::new("owner/name");
    let mut tracker = RecordingTracker::new(&harness.log);
    let evidence = FakeEvidence::new(&harness.log, 1, false);

    let mut release = StallRelease::new(
        &mut tracker,
        ArtifactStore::new(&harness.store_root),
        policy(5),
        roster(),
    );
    let mut history = AttemptHistory::new();
    let result = release
        .finish(
            &harness.issue,
            &mut history,
            FinishedAttempt {
                record: attempt("Qwen3.8-27B Q8", 903),
                reason: StallReason::NoLiveness,
                liveness: Liveness::Hung,
                base: "base-rev".to_string(),
                evidence: &evidence,
            },
        )
        .expect("finish");

    let ReleaseDecision::Requeue { next_model, .. } = &result.plan.decision else {
        panic!("expected a requeue, got {:?}", result.plan.decision);
    };
    assert_eq!(next_model.id, "DeepSeek-V4-Flash");
    assert_eq!(result.attempt_counter, 1);

    // Everything captured is on disk for the next attempt, including the note.
    let stored = release.store().read_latest(3289).expect("read store");
    let names: Vec<&str> = stored
        .iter()
        .map(|artifact| artifact.name.as_str())
        .collect();
    assert!(
        names.contains(&"attempt-1-commits.patch"),
        "commits patch missing from {names:?}"
    );
    assert!(
        names.contains(&"attempt-1-stall-note.md"),
        "stall note missing from {names:?}"
    );
    assert_eq!(
        stored.len(),
        result.stored_artifacts.len(),
        "every stored artifact is reported with a path"
    );
    for path in &result.stored_artifacts {
        assert!(path.exists(), "reported artifact is missing: {path:?}");
    }

    // And the queue comment says what happened and what comes next.
    let released = tracker.released.borrow();
    assert_eq!(released.len(), 1);
    let (issue, note) = &released[0];
    assert_eq!(issue, &harness.issue);
    for expected in [
        "Stalled attempt 1 of 5",
        "reason: no liveness signal",
        "work produced: 1 commit",
        "model: Qwen3.8-27B Q8",
        "next attempt: DeepSeek-V4-Flash",
    ] {
        assert!(
            note.contains(expected),
            "note missing {expected:?}:\n{note}"
        );
    }

    // Attachments went out before the counter bump and the release.
    let ops = harness.ops();
    let last_attach = ops
        .iter()
        .rposition(|op| op.starts_with("tracker:attach"))
        .expect("attachments attempted");
    let bump = ops
        .iter()
        .position(|op| op == "tracker:bump")
        .expect("counter bumped");
    assert!(
        last_attach < bump,
        "evidence must be on the issue first: {ops:?}"
    );
}

#[test]
fn attachment_failures_are_recorded_and_never_lose_the_work() {
    let harness = Harness::new("owner/name");
    let mut tracker = RecordingTracker::new(&harness.log);
    tracker.fail_attach = true;
    let evidence = FakeEvidence::new(&harness.log, 1, true);

    let mut release = StallRelease::new(
        &mut tracker,
        ArtifactStore::new(&harness.store_root),
        policy(5),
        roster(),
    );
    let mut history = AttemptHistory::new();
    let result = release
        .finish(
            &harness.issue,
            &mut history,
            FinishedAttempt {
                record: attempt("Qwen3.8-27B Q8", 600),
                reason: StallReason::WorkerExited,
                liveness: Liveness::Hung,
                base: "base-rev".to_string(),
                evidence: &evidence,
            },
        )
        .expect("a dead attachment endpoint must not fail the release");

    assert!(
        !result.attachment_errors.is_empty(),
        "the failure must be visible to the caller"
    );
    assert_eq!(
        result.stored_artifacts.len(),
        result.plan.artifacts.len(),
        "every artifact is still stored locally despite the attachment failure"
    );
    assert_eq!(tracker.released.borrow().len(), 1, "the issue is requeued");
}

#[test]
fn the_last_attempt_escalates_with_labels_history_and_artifact_paths() {
    let harness = Harness::new("owner/name");
    let mut tracker = RecordingTracker::new(&harness.log);
    let evidence = FakeEvidence::new(&harness.log, 0, false);

    let mut release = StallRelease::new(
        &mut tracker,
        ArtifactStore::new(&harness.store_root),
        policy(2),
        roster(),
    );
    let mut history = AttemptHistory::new();
    history.push(attempt("Qwen3.8-27B Q8", 900));

    let result = release
        .finish(
            &harness.issue,
            &mut history,
            FinishedAttempt {
                record: attempt("DeepSeek-V4-Flash", 750),
                reason: StallReason::NoLiveness,
                liveness: Liveness::Hung,
                base: "base-rev".to_string(),
                evidence: &evidence,
            },
        )
        .expect("finish");

    let ReleaseDecision::Escalate { report, reason, .. } = &result.plan.decision else {
        panic!("expected escalation, got {:?}", result.plan.decision);
    };
    assert_eq!(report.attempts.len(), 2, "both attempts are in the report");
    assert_eq!(
        report.attempts[1].produced,
        WorkProduced::None,
        "the second attempt's capture is recorded, not its stale default"
    );
    assert!(
        !report.artifact_paths.is_empty(),
        "spec repair must be pointed at the captured evidence"
    );
    for path in &report.artifact_paths {
        assert!(path.exists(), "reported artifact is missing: {path:?}");
    }
    assert!(report.markdown().contains("attempt 2"));

    let labels = tracker.labels.borrow();
    let mut labels: Vec<String> = labels.iter().map(|(_, label)| label.clone()).collect();
    labels.sort();
    assert_eq!(
        labels,
        vec![
            "spec-repair".to_string(),
            "stalled-attempts-exhausted".to_string()
        ],
        "reason: {reason:?}"
    );
    assert_eq!(tracker.escalations.borrow().len(), 1);
    assert!(
        tracker.released.borrow().is_empty(),
        "no requeue on escalation"
    );
}

#[test]
fn a_local_tracker_gets_the_same_handoff_as_github() {
    let harness = Harness::new("jira:OPS");
    let mut tracker = RecordingTracker::new(&harness.log);
    let evidence = FakeEvidence::new(&harness.log, 1, false);

    let mut release = StallRelease::new(
        &mut tracker,
        ArtifactStore::new(&harness.store_root),
        policy(5),
        roster(),
    );
    let mut history = AttemptHistory::new();
    release
        .finish(
            &harness.issue,
            &mut history,
            FinishedAttempt {
                record: attempt("Qwen3.8-27B Q8", 903),
                reason: StallReason::LeaseExpired,
                liveness: Liveness::Hung,
                base: "base-rev".to_string(),
                evidence: &evidence,
            },
        )
        .expect("finish");

    // The store keys on the issue number alone, so a non-GitHub project still
    // lands its evidence somewhere the next attempt can find it.
    let stored = release.store().latest_attempt(3289).expect("store");
    let released = tracker.released.borrow();
    let (issue, _) = &released[0];
    assert_eq!(issue.project, "jira:OPS");
    assert_eq!(issue.number, 3289);
    assert_eq!(released.len(), 1);
    assert_eq!(stored, Some(1));
}

#[test]
fn a_completed_attempt_touches_nothing() {
    let harness = Harness::new("owner/name");
    let mut tracker = RecordingTracker::new(&harness.log);
    let evidence = FakeEvidence::new(&harness.log, 3, false);

    let mut release = StallRelease::new(
        &mut tracker,
        ArtifactStore::new(&harness.store_root),
        policy(5),
        roster(),
    );
    let mut history = AttemptHistory::new();
    let mut record = attempt("Qwen3.8-27B Q8", 400);
    record.outcome = AttemptOutcome::Completed;

    let result = release
        .finish(
            &harness.issue,
            &mut history,
            FinishedAttempt {
                record,
                reason: StallReason::Cancelled,
                liveness: Liveness::Deliberating,
                base: "base-rev".to_string(),
                evidence: &evidence,
            },
        )
        .expect("finish");

    assert!(matches!(
        result.plan.decision,
        ReleaseDecision::Completed { .. }
    ));
    assert_eq!(result.attempt_counter, 0);
    assert!(
        harness.ops().iter().all(|op| !op.starts_with("tracker:")),
        "a completion must not touch the tracker: {:?}",
        harness.ops()
    );
    assert_eq!(history.len(), 1);
    assert_eq!(
        history.last().expect("record").produced,
        WorkProduced::Commits { count: 3 }
    );
}
