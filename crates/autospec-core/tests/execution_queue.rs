use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use autospec_core::execution::{
    ExecutionQueue, FailureKind, QueueStatus, QueueValidationResult, QueueValidationStatus,
};

static NEXT_TEMP_ROOT: AtomicU64 = AtomicU64::new(0);

struct TempProjectRoot {
    path: PathBuf,
}

impl TempProjectRoot {
    fn new() -> Self {
        let nonce = NEXT_TEMP_ROOT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "autospec-execution-queue-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("temporary project root is created");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempProjectRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[test]
fn execution_queue_resumes_first_incomplete_entry() {
    let mut queue = ExecutionQueue::new(
        "run-v66",
        vec![
            "v65-spec-state-validation".to_string(),
            "v66-autonomous-execution-queue".to_string(),
        ],
    );
    queue
        .mark_passed("v65-spec-state-validation")
        .expect("known spec");

    let next = queue.next_incomplete().expect("second entry remains");

    assert_eq!(next.spec_id, "v66-autonomous-execution-queue");
    assert_eq!(next.status, QueueStatus::Pending);
}

#[test]
fn execution_queue_enforces_retry_limit() {
    let mut queue = ExecutionQueue::new(
        "run-v66",
        vec!["v66-autonomous-execution-queue".to_string()],
    );

    queue
        .record_failure("v66-autonomous-execution-queue", FailureKind::Validation, 1)
        .expect("first failure records");
    let error = queue
        .record_failure("v66-autonomous-execution-queue", FailureKind::Validation, 1)
        .expect_err("second failure exceeds retry limit");

    assert!(error.contains("retry limit exceeded"));
    assert_eq!(
        queue
            .entry("v66-autonomous-execution-queue")
            .unwrap()
            .status,
        QueueStatus::Blocked
    );
}

#[test]
fn execution_queue_renders_handoff_and_report() {
    let mut queue = ExecutionQueue::new(
        "run-v66",
        vec![
            "v65-spec-state-validation".to_string(),
            "v66-autonomous-execution-queue".to_string(),
        ],
    );
    queue
        .mark_passed("v65-spec-state-validation")
        .expect("known spec");
    queue
        .block("v66-autonomous-execution-queue", "validation failed")
        .expect("known spec");

    let handoff = queue
        .handoff_markdown("v66-autonomous-execution-queue")
        .unwrap();
    let report = queue.final_report_markdown();

    assert!(handoff.contains("# Blocked Spec: v66-autonomous-execution-queue"));
    assert!(handoff.contains("validation failed"));
    assert!(report.contains("passed: 1"));
    assert!(report.contains("blocked: 1"));
}

#[test]
fn execution_queue_round_trips_timestamped_validation_metadata() {
    let root = TempProjectRoot::new();
    let mut queue = ExecutionQueue::new(
        "run-v66-persisted",
        vec!["v66-autonomous-execution-queue".to_string()],
    );

    queue
        .mark_started_at("v66-autonomous-execution-queue", 100)
        .expect("known spec starts");
    queue
        .record_validation_at(
            "v66-autonomous-execution-queue",
            QueueValidationResult::new(QueueValidationStatus::Passed, "cargo test --workspace"),
            101,
        )
        .expect("known spec records validation");
    queue
        .mark_passed_at("v66-autonomous-execution-queue", 102)
        .expect("known spec passes");
    queue.save(root.path()).expect("queue saves");

    let loaded = ExecutionQueue::load_named(root.path(), "run-v66-persisted")
        .expect("queue loads")
        .expect("named queue exists");
    let entry = loaded
        .entry("v66-autonomous-execution-queue")
        .expect("entry survives round trip");

    assert_eq!(entry.status, QueueStatus::Passed);
    assert_eq!(entry.started_at, Some(100));
    assert_eq!(entry.updated_at, 102);
    assert_eq!(
        entry
            .validation
            .as_ref()
            .map(|result| result.summary.as_str()),
        Some("cargo test --workspace")
    );
}

#[test]
fn execution_queue_load_latest_incomplete_skips_complete_runs() {
    let root = TempProjectRoot::new();
    let mut complete = ExecutionQueue::new(
        "run-complete",
        vec!["v65-spec-state-validation".to_string()],
    );
    complete
        .mark_passed_at("v65-spec-state-validation", 10)
        .expect("complete queue passes");
    complete.save(root.path()).expect("complete queue saves");

    let mut incomplete = ExecutionQueue::new(
        "run-incomplete",
        vec!["v66-autonomous-execution-queue".to_string()],
    );
    incomplete
        .mark_started_at("v66-autonomous-execution-queue", 20)
        .expect("incomplete queue starts");
    incomplete
        .save(root.path())
        .expect("incomplete queue saves");

    let resumed = ExecutionQueue::load_latest_incomplete(root.path())
        .expect("resume discovery succeeds")
        .expect("incomplete queue exists");

    assert_eq!(resumed.run_id, "run-incomplete");
}

#[test]
fn execution_queue_recovers_a_complete_temporary_file_after_primary_corruption() {
    let root = TempProjectRoot::new();
    let queue = ExecutionQueue::new_at(
        "run-recovery",
        vec!["v66-autonomous-execution-queue".to_string()],
        30,
    );
    queue.save(root.path()).expect("queue saves");

    let directory = root
        .path()
        .join(".autospec")
        .join("runs")
        .join("run-recovery");
    let primary = directory.join("queue.json");
    let temporary = directory.join("queue.json.tmp");
    let document = fs::read_to_string(&primary).expect("queue document is readable");
    fs::write(&temporary, &document).expect("complete recovery file is written");
    fs::write(&primary, "{not valid json").expect("primary is corrupted");

    let loaded = ExecutionQueue::load_named(root.path(), "run-recovery")
        .expect("temporary queue recovers")
        .expect("recovered queue exists");

    assert_eq!(loaded.run_id, "run-recovery");
    assert_eq!(
        fs::read_to_string(primary).expect("temporary file is promoted"),
        document
    );
    assert!(
        !temporary.exists(),
        "recovery file is consumed after promotion"
    );
}

#[test]
fn execution_queue_rejects_path_like_run_ids_before_touching_disk() {
    let root = TempProjectRoot::new();
    let queue = ExecutionQueue::new("..", vec!["v66-autonomous-execution-queue".to_string()]);

    assert!(queue.save(root.path()).is_err());
    assert!(ExecutionQueue::load_named(root.path(), "..").is_err());
}
