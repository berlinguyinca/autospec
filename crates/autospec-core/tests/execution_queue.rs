use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use autospec_core::agent::AgentResult;
use autospec_core::execution::{
    AgentOutcome, ExecutionQueue, FailureKind, IngestedAgentResult, OneShotIssueSelector,
    QueueResultApplication, QueueStatus, QueueValidationResult, QueueValidationStatus,
};

#[test]
fn one_shot_scope_consumed_once_after_terminal_outcome() {
    let mut selector = OneShotIssueSelector::new(42).expect("positive issue selector");
    assert!(selector.matches(42));
    assert!(!selector.observe_status(42, &QueueStatus::Running).unwrap());
    assert!(selector.observe_status(42, &QueueStatus::Passed).unwrap());
    assert!(!selector.matches(42));
    assert!(!selector.observe_status(42, &QueueStatus::Blocked).unwrap());
    assert_eq!(
        selector.status_json(),
        r#"{"issue":42,"consumed":true,"scope":"unscoped"}"#
    );
}

#[test]
fn one_shot_scope_does_not_consume_other_issue_or_retryable_failure() {
    let mut selector = OneShotIssueSelector::new(42).expect("positive issue selector");
    assert!(!selector.observe_status(7, &QueueStatus::Blocked).unwrap());
    assert!(!selector.observe_status(42, &QueueStatus::Failed).unwrap());
    assert!(selector.matches(42));
}

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
    let mut queue = ExecutionQueue::new_at(
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
    let mut queue = ExecutionQueue::new("..", vec!["v66-autonomous-execution-queue".to_string()]);

    assert!(queue.save(root.path()).is_err());
    assert!(ExecutionQueue::load_named(root.path(), "..").is_err());
}

#[test]
fn execution_queue_rejects_a_document_stored_under_the_wrong_run_id() {
    let root = TempProjectRoot::new();
    let mut queue = ExecutionQueue::new_at(
        "run-original",
        vec!["v66-autonomous-execution-queue".to_string()],
        40,
    );
    queue.save(root.path()).expect("queue saves");

    let original = root.path().join(".autospec/runs/run-original/queue.json");
    let misplaced = root.path().join(".autospec/runs/run-misplaced/queue.json");
    fs::create_dir_all(misplaced.parent().expect("misplaced queue has parent"))
        .expect("misplaced run directory is created");
    fs::copy(original, misplaced).expect("queue is deliberately misplaced");

    assert!(ExecutionQueue::load_named(root.path(), "run-misplaced").is_err());
}

#[test]
fn execution_queue_does_not_promote_a_misbound_recovery_document() {
    let root = TempProjectRoot::new();
    let mut queue = ExecutionQueue::new_at(
        "run-original-recovery",
        vec!["v66-autonomous-execution-queue".to_string()],
        45,
    );
    queue.save(root.path()).expect("source queue saves");

    let source = root
        .path()
        .join(".autospec/runs/run-original-recovery/queue.json");
    let directory = root.path().join(".autospec/runs/run-misbound-recovery");
    fs::create_dir_all(&directory).expect("target queue directory is created");
    let primary = directory.join("queue.json");
    let temporary = directory.join("queue.json.tmp");
    fs::copy(&source, &temporary).expect("misbound recovery document is written");

    assert!(ExecutionQueue::load_named(root.path(), "run-misbound-recovery").is_err());
    assert!(!primary.exists(), "misbound recovery file is not promoted");
    assert!(
        temporary.exists(),
        "misbound recovery file remains for diagnosis"
    );
}

#[test]
fn execution_queue_cannot_restart_a_terminal_entry() {
    let mut queue = ExecutionQueue::new_at(
        "run-terminal",
        vec!["v66-autonomous-execution-queue".to_string()],
        50,
    );
    queue
        .mark_passed_at("v66-autonomous-execution-queue", 51)
        .expect("entry passes");

    assert!(queue
        .mark_started_at("v66-autonomous-execution-queue", 52)
        .is_err());
}

#[test]
fn execution_queue_persists_agent_results_and_replays_each_id_once() {
    let root = TempProjectRoot::new();
    ExecutionQueue::create_if_absent_at(
        root.path(),
        "run-agent-results",
        vec!["v67-agent-integration-contracts".to_string()],
        100,
    )
    .expect("queue is created");
    let result = IngestedAgentResult::new_at(
        "run-agent-results",
        "v67-agent-integration-contracts",
        "result-1",
        AgentOutcome::Failed {
            failure_kind: FailureKind::Validation,
        },
        AgentResult::new(
            "cargo test failed",
            Vec::new(),
            "cargo test --workspace: exit 1",
            Vec::new(),
            "fix the failing test",
        ),
        101,
    )
    .expect("ingested result is valid");

    assert_eq!(
        ExecutionQueue::ingest_agent_result_at(root.path(), &result, 3, 102)
            .expect("result persists and applies")
            .application,
        QueueResultApplication::Applied
    );
    assert!(root
        .path()
        .join(".autospec/runs/run-agent-results/agent-results/v67-agent-integration-contracts/result-1.json")
        .exists());

    let replayed = IngestedAgentResult::new_at(
        "run-agent-results",
        "v67-agent-integration-contracts",
        "result-1",
        AgentOutcome::Failed {
            failure_kind: FailureKind::Validation,
        },
        AgentResult::new(
            "cargo test failed",
            Vec::new(),
            "cargo test --workspace: exit 1",
            Vec::new(),
            "fix the failing test",
        ),
        999,
    )
    .expect("same result id is valid");
    assert_eq!(
        ExecutionQueue::ingest_agent_result_at(root.path(), &replayed, 3, 103)
            .expect("same result replay is safe")
            .application,
        QueueResultApplication::AlreadyApplied
    );
    let loaded_queue = ExecutionQueue::load_named(root.path(), "run-agent-results")
        .expect("queue reloads")
        .expect("queue remains");
    let entry = loaded_queue
        .entry("v67-agent-integration-contracts")
        .expect("queue entry exists");
    assert_eq!(entry.attempts, 1);
    assert_eq!(entry.status, QueueStatus::Failed);
    assert_eq!(entry.agent_result_ids, ["result-1"]);
}

#[test]
fn execution_queue_recovers_agent_result_temporary_file_and_rejects_a_misbound_path() {
    let root = TempProjectRoot::new();
    ExecutionQueue::create_if_absent_at(
        root.path(),
        "run-agent-recovery",
        vec![
            "v67-agent-integration-contracts".to_string(),
            "v67-other-contract".to_string(),
        ],
        199,
    )
    .expect("queue is created");
    let result = IngestedAgentResult::new_at(
        "run-agent-recovery",
        "v67-agent-integration-contracts",
        "result-1",
        AgentOutcome::Blocked,
        AgentResult::new(
            "external dependency unavailable",
            Vec::new(),
            "",
            vec!["registry outage".to_string()],
            "resume when the registry recovers",
        ),
        200,
    )
    .expect("blocked result is valid");
    ExecutionQueue::ingest_agent_result_at(root.path(), &result, 3, 201).expect("result persists");

    let directory = root
        .path()
        .join(".autospec/runs/run-agent-recovery/agent-results/v67-agent-integration-contracts");
    let primary = directory.join("result-1.json");
    let temporary = directory.join("result-1.json.tmp");
    let document = fs::read_to_string(&primary).expect("result document is readable");
    fs::rename(&primary, &temporary).expect("simulated interrupted promotion");

    assert_eq!(
        ExecutionQueue::ingest_agent_result_at(root.path(), &result, 3, 202)
            .expect("temporary result recovers")
            .application,
        QueueResultApplication::AlreadyApplied
    );
    assert_eq!(
        fs::read_to_string(&primary).expect("temporary is promoted"),
        document
    );
    assert!(!temporary.exists(), "recovery file is consumed");

    let misplaced = root
        .path()
        .join(".autospec/runs/run-agent-recovery/agent-results/v67-other-contract/result-1.json");
    fs::create_dir_all(misplaced.parent().expect("misplaced result parent"))
        .expect("misplaced parent is created");
    fs::copy(&primary, &misplaced).expect("result is deliberately misplaced");

    let misbound = IngestedAgentResult::new_at(
        "run-agent-recovery",
        "v67-other-contract",
        "result-1",
        AgentOutcome::Blocked,
        AgentResult::new(
            "blocked",
            Vec::new(),
            "",
            vec!["dependency".to_string()],
            "",
        ),
        203,
    )
    .expect("misbound input is otherwise valid");
    assert!(ExecutionQueue::ingest_agent_result_at(root.path(), &misbound, 3, 204).is_err());
}

#[test]
fn execution_queue_discards_an_interrupted_first_agent_result_write_before_replay() {
    let root = TempProjectRoot::new();
    ExecutionQueue::create_if_absent_at(
        root.path(),
        "run-agent-truncated",
        vec!["v67-agent-integration-contracts".to_string()],
        300,
    )
    .expect("queue is created");
    let result = IngestedAgentResult::new_at(
        "run-agent-truncated",
        "v67-agent-integration-contracts",
        "result-1",
        AgentOutcome::Passed,
        AgentResult::new(
            "implemented",
            Vec::new(),
            "cargo test: exit 0",
            Vec::new(),
            "",
        ),
        301,
    )
    .expect("result is valid");
    let directory = root
        .path()
        .join(".autospec/runs/run-agent-truncated/agent-results/v67-agent-integration-contracts");
    fs::create_dir_all(&directory).expect("result directory is created");
    fs::write(directory.join("result-1.json.tmp"), "{\"schema\":")
        .expect("truncated temporary result is written");

    assert_eq!(
        ExecutionQueue::ingest_agent_result_at(root.path(), &result, 3, 302)
            .expect("retry replaces interrupted first write")
            .application,
        QueueResultApplication::Applied
    );
    assert!(directory.join("result-1.json").exists());
    assert!(!directory.join("result-1.json.tmp").exists());
}

#[test]
fn execution_queue_serializes_concurrent_distinct_result_ingestions() {
    let root = TempProjectRoot::new();
    ExecutionQueue::create_if_absent_at(
        root.path(),
        "run-agent-concurrent",
        vec!["v67-agent-integration-contracts".to_string()],
        400,
    )
    .expect("queue is created");
    let results = ["result-1", "result-2"].map(|result_id| {
        IngestedAgentResult::new_at(
            "run-agent-concurrent",
            "v67-agent-integration-contracts",
            result_id,
            AgentOutcome::Failed {
                failure_kind: FailureKind::Validation,
            },
            AgentResult::new(
                "test failed",
                Vec::new(),
                "cargo test: exit 1",
                Vec::new(),
                "",
            ),
            401,
        )
        .expect("result is valid")
    });
    let path = root.path().to_path_buf();

    std::thread::scope(|scope| {
        for result in &results {
            let path = path.clone();
            scope.spawn(move || {
                ExecutionQueue::ingest_agent_result_at(&path, result, 3, 402)
                    .expect("concurrent result is serialized");
            });
        }
    });

    let queue = ExecutionQueue::load_named(root.path(), "run-agent-concurrent")
        .expect("queue loads")
        .expect("queue exists");
    let entry = queue
        .entry("v67-agent-integration-contracts")
        .expect("queue entry exists");
    assert_eq!(entry.attempts, 2);
    let mut result_ids = entry.agent_result_ids.clone();
    result_ids.sort();
    assert_eq!(result_ids, ["result-1", "result-2"]);
}

#[test]
fn execution_queue_rejects_a_stale_save_after_result_ingestion() {
    let root = TempProjectRoot::new();
    let mut initial = ExecutionQueue::new_at(
        "run-agent-stale-save",
        vec!["v67-agent-integration-contracts".to_string()],
        450,
    );
    initial.save(root.path()).expect("initial queue saves");
    let mut stale = ExecutionQueue::load_named(root.path(), "run-agent-stale-save")
        .expect("stale queue loads")
        .expect("stale queue exists");
    let result = IngestedAgentResult::new_at(
        "run-agent-stale-save",
        "v67-agent-integration-contracts",
        "result-1",
        AgentOutcome::Passed,
        AgentResult::new(
            "implemented",
            Vec::new(),
            "cargo test: exit 0",
            Vec::new(),
            "",
        ),
        451,
    )
    .expect("result is valid");
    ExecutionQueue::ingest_agent_result_at(root.path(), &result, 3, 452).expect("result ingests");
    stale
        .mark_started_at("v67-agent-integration-contracts", 453)
        .expect("stale queue mutates in memory");

    let error = stale
        .save(root.path())
        .expect_err("stale queue cannot overwrite a newer result");
    assert!(error.contains("revision conflict"));
    let current = ExecutionQueue::load_named(root.path(), "run-agent-stale-save")
        .expect("current queue loads")
        .expect("current queue exists");
    assert_eq!(
        current
            .entry("v67-agent-integration-contracts")
            .expect("current entry exists")
            .status,
        QueueStatus::Passed
    );
}

#[test]
fn execution_queue_rejects_mutated_agent_results_before_writing_or_transitioning() {
    let root = TempProjectRoot::new();
    ExecutionQueue::create_if_absent_at(
        root.path(),
        "run-agent-invalid",
        vec!["v67-agent-integration-contracts".to_string()],
        500,
    )
    .expect("queue is created");
    let mut result = IngestedAgentResult::new_at(
        "run-agent-invalid",
        "v67-agent-integration-contracts",
        "result-1",
        AgentOutcome::Passed,
        AgentResult::new(
            "implemented",
            Vec::new(),
            "cargo test: exit 0",
            Vec::new(),
            "",
        ),
        501,
    )
    .expect("result is initially valid");
    result.agent_result.validation.clear();

    assert!(ExecutionQueue::ingest_agent_result_at(root.path(), &result, 3, 502).is_err());
    let queue = ExecutionQueue::load_named(root.path(), "run-agent-invalid")
        .expect("queue loads")
        .expect("queue exists");
    assert_eq!(
        queue
            .entry("v67-agent-integration-contracts")
            .expect("queue entry exists")
            .status,
        QueueStatus::Pending
    );
    assert!(!root
        .path()
        .join(".autospec/runs/run-agent-invalid/agent-results/v67-agent-integration-contracts/result-1.json")
        .exists());
}

#[test]
fn execution_queue_reads_legacy_documents_and_upgrades_them_on_save() {
    let root = TempProjectRoot::new();
    let directory = root.path().join(".autospec/runs/run-legacy");
    fs::create_dir_all(&directory).expect("legacy queue directory is created");
    fs::write(
        directory.join("queue.json"),
        "{\"schema\":1,\"run_id\":\"run-legacy\",\"updated_at\":1,\"entries\":[{\"spec_id\":\"v67-agent-integration-contracts\",\"status\":\"pending\",\"attempts\":0,\"failure_kind\":null,\"blocker\":null,\"started_at\":null,\"updated_at\":1,\"validation\":null}]}",
    )
    .expect("legacy queue is written");

    let mut queue = ExecutionQueue::load_named(root.path(), "run-legacy")
        .expect("legacy queue loads")
        .expect("legacy queue exists");
    assert!(queue
        .entry("v67-agent-integration-contracts")
        .expect("legacy entry exists")
        .agent_result_ids
        .is_empty());

    queue.save(root.path()).expect("legacy queue upgrades");
    let upgraded = fs::read_to_string(directory.join("queue.json")).expect("upgraded queue");
    assert!(upgraded.contains("\"schema\":3"));
    assert!(upgraded.contains("\"revision\":1"));
    assert!(upgraded.contains("\"agent_result_ids\":[]"));
}
