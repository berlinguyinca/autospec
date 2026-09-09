use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use autospec_core::agent::AgentResult;
use autospec_core::execution::{
    AgentOutcome, ExecutionQueue, FailureKind, IngestedAgentResult, OneShotIssueSelector,
    QueueResultApplication, QueueStatus, QueueValidationResult, QueueValidationStatus, SpecDigest,
    StagedSpecCheck,
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
    assert!(upgraded.contains("\"schema\":4"));
    assert!(upgraded.contains("\"revision\":1"));
    assert!(upgraded.contains("\"agent_result_ids\":[]"));
    assert!(upgraded.contains("\"spec_digest\":null"));
}

#[test]
fn execution_queue_no_spec_parks_terminal_without_attempt_and_never_redispatches() {
    let root = TempProjectRoot::new();
    ExecutionQueue::create_if_absent_at(
        root.path(),
        "run-no-spec",
        vec![
            "v67-agent-integration-contracts".to_string(),
            "v68-second-spec".to_string(),
        ],
        100,
    )
    .expect("queue is created");
    let result = IngestedAgentResult::new_at(
        "run-no-spec",
        "v67-agent-integration-contracts",
        "result-1",
        AgentOutcome::NoSpec,
        AgentResult::new(
            "dispatched but never started",
            Vec::new(),
            "n/a — runner could not read the spec input",
            vec![
                "cat: specs/v67-agent-integration-contracts.md: No such file or directory"
                    .to_string(),
            ],
            "stage the spec input as part of dispatch",
        ),
        101,
    )
    .expect("no-spec result carries its blocker");

    let receipt = ExecutionQueue::ingest_agent_result_at(root.path(), &result, 3, 102)
        .expect("no-spec result applies");
    assert_eq!(receipt.application, QueueResultApplication::Applied);
    assert_eq!(receipt.status, QueueStatus::NoSpec);

    let mut queue = ExecutionQueue::load_named(root.path(), "run-no-spec")
        .expect("queue reloads")
        .expect("queue remains");
    let entry = queue
        .entry("v67-agent-integration-contracts")
        .expect("queue entry exists");
    assert_eq!(entry.status, QueueStatus::NoSpec);
    // Nothing was attempted: attempts stays 0 and no failure kind is recorded.
    assert_eq!(entry.attempts, 0);
    assert_eq!(entry.failure_kind, None);
    assert!(entry
        .blocker
        .as_deref()
        .is_some_and(|blocker| blocker.contains("No such file or directory")));
    assert!(entry.started_at.is_some());

    // The re-dispatch selector moves on to the sibling spec instead of
    // spinning on the one that never started.
    assert_eq!(
        queue.next_incomplete().map(|entry| entry.spec_id.as_str()),
        Some("v68-second-spec")
    );

    // Terminal: cannot restart, cannot receive a new result.
    assert!(queue
        .mark_started_at("v67-agent-integration-contracts", 103)
        .is_err());
    let replacement = IngestedAgentResult::new_at(
        "run-no-spec",
        "v67-agent-integration-contracts",
        "result-2",
        AgentOutcome::Passed,
        AgentResult::new(
            "implemented",
            Vec::new(),
            "cargo test --workspace: exit 0",
            Vec::new(),
            "ready",
        ),
        104,
    )
    .expect("passed result is valid");
    assert!(ExecutionQueue::ingest_agent_result_at(root.path(), &replacement, 3, 105).is_err());

    // A no-spec observation consumes a one-shot selector exactly like the
    // other terminal states.
    let mut selector = OneShotIssueSelector::new(42).expect("positive issue selector");
    assert!(selector
        .observe_status(42, &QueueStatus::NoSpec)
        .expect("terminal observation"));
    assert!(selector.consumed());

    // The report counts no-spec separately from failed.
    let report = queue.final_report_markdown();
    assert!(report.contains("no-spec: 1"));
    assert!(report.contains("failed: 0"));
}

#[test]
fn execution_queue_no_spec_result_requires_a_blocker() {
    let without_blocker = IngestedAgentResult::new_at(
        "run-no-spec-blocker",
        "v67-agent-integration-contracts",
        "result-1",
        AgentOutcome::NoSpec,
        AgentResult::new("dispatched", Vec::new(), "n/a", Vec::new(), "handoff"),
        100,
    );
    assert!(without_blocker.is_err());

    let root = TempProjectRoot::new();
    ExecutionQueue::create_if_absent_at(
        root.path(),
        "run-no-spec-blocker",
        vec!["v67-agent-integration-contracts".to_string()],
        100,
    )
    .expect("queue is created");
    let result = IngestedAgentResult::new_at(
        "run-no-spec-blocker",
        "v67-agent-integration-contracts",
        "result-1",
        AgentOutcome::NoSpec,
        AgentResult::new(
            "dispatched but never started",
            Vec::new(),
            "n/a",
            vec!["spec input missing".to_string()],
            "stage the spec input",
        ),
        101,
    )
    .expect("no-spec result carries its blocker");
    assert_eq!(
        ExecutionQueue::ingest_agent_result_at(root.path(), &result, 3, 102)
            .expect("no-spec result applies")
            .status,
        QueueStatus::NoSpec
    );
    // The status round-trips through the queue JSON without a schema bump.
    let queue = ExecutionQueue::load_named(root.path(), "run-no-spec-blocker")
        .expect("queue reloads")
        .expect("queue remains");
    assert_eq!(
        queue
            .entry("v67-agent-integration-contracts")
            .expect("queue entry exists")
            .status,
        QueueStatus::NoSpec
    );
}

#[test]
fn agent_outcome_no_spec_round_trips_json_with_null_failure_kind() {
    let result = IngestedAgentResult::new_at(
        "run-no-spec-json",
        "v67-agent-integration-contracts",
        "result-1",
        AgentOutcome::NoSpec,
        AgentResult::new(
            "dispatched but never started",
            Vec::new(),
            "n/a",
            vec!["spec input missing".to_string()],
            "stage the spec input",
        ),
        100,
    )
    .expect("no-spec result is valid");
    let json = result.to_json();
    assert!(json.contains("\"status\":\"no-spec\""));
    assert!(json.contains("\"failure_kind\":null"));
    let round_tripped = IngestedAgentResult::from_json(&json).expect("no-spec result round-trips");
    assert_eq!(round_tripped, result);
    let with_failure_kind = json.replace("\"failure_kind\":null", "\"failure_kind\":\"agent\"");
    assert!(IngestedAgentResult::from_json(&with_failure_kind).is_err());
}

#[test]
fn create_if_absent_staged_writes_inputs_in_the_same_action_that_schedules() {
    let root = TempProjectRoot::new();
    let input = root.path().join("input");
    fs::create_dir_all(&input).expect("input directory is created");
    fs::write(input.join("v99-first.md"), "# spec one\n").expect("first spec source is written");
    fs::write(input.join("v99-second.md"), "# spec two\n").expect("second spec source is written");

    let queue = ExecutionQueue::create_if_absent_staged(
        root.path(),
        "run-staged",
        &[
            ("v99-first".to_string(), input.join("v99-first.md")),
            ("v99-second".to_string(), input.join("v99-second.md")),
        ],
    )
    .expect("staged queue is created");

    // The scheduled run exists and every input landed at its stable path.
    let run_directory = root.path().join(".autospec/runs/run-staged");
    assert!(run_directory.join("queue.json").exists());
    assert_eq!(
        fs::read_to_string(run_directory.join("specs/v99-first.md")).expect("staged input"),
        "# spec one\n"
    );
    assert_eq!(
        fs::read_to_string(run_directory.join("specs/v99-second.md")).expect("staged input"),
        "# spec two\n"
    );
    assert_eq!(queue.run_id, "run-staged");

    // Re-creating the same run still refuses.
    assert!(ExecutionQueue::create_if_absent_staged(
        root.path(),
        "run-staged",
        &[("v99-first".to_string(), input.join("v99-first.md"),)],
    )
    .is_err());
}

#[test]
fn create_if_absent_staged_fails_closed_when_an_input_is_unreadable() {
    let root = TempProjectRoot::new();
    let input = root.path().join("input");
    fs::create_dir_all(&input).expect("input directory is created");
    fs::write(input.join("v99-first.md"), "# spec one\n").expect("first spec source is written");

    let missing = input.join("v99-missing.md");
    let error = ExecutionQueue::create_if_absent_staged(
        root.path(),
        "run-staged-missing",
        &[
            ("v99-first".to_string(), input.join("v99-first.md")),
            ("v99-missing".to_string(), missing),
        ],
    )
    .expect_err("unreadable spec input aborts the create");
    assert!(
        error.contains("v99-missing"),
        "error names the unreadable spec: {error}"
    );
    assert!(
        error.contains("run not scheduled"),
        "error states nothing was scheduled: {error}"
    );

    // Fail-closed: no queue, no partial staging — nothing to clean up.
    assert!(!root.path().join(".autospec").exists());
}

#[test]
fn spec_digest_survives_post_run_spec_edits() {
    // Acceptance, exercised against a populated case (#3939): a run record
    // that names a spec must report the size and content hash captured at
    // dispatch, and that report must survive the spec file being edited
    // after the run — the record, not the file, is the source of truth for
    // the guidance the agent was dispatched with.
    let root = TempProjectRoot::new();
    let input = root.path().join("input");
    fs::create_dir_all(&input).expect("input directory is created");
    fs::write(input.join("v99-first.md"), "# spec one\n").expect("spec source is written");

    let queue = ExecutionQueue::create_if_absent_staged(
        root.path(),
        "run-digest",
        &[("v99-first".to_string(), input.join("v99-first.md"))],
    )
    .expect("staged queue is created");

    // Dispatch captured the exact staged bytes: size and hash.
    let original = queue
        .spec_digest("v99-first")
        .cloned()
        .expect("dispatch captured the spec digest");
    assert_eq!(original, SpecDigest::from_bytes(b"# spec one\n"));
    assert_eq!(original.bytes, 11);
    assert_eq!(original.sha256.len(), 64);

    // The run record on disk carries the digest, not just the in-memory one.
    let record = fs::read_to_string(root.path().join(".autospec/runs/run-digest/queue.json"))
        .expect("run record is read");
    assert!(
        record.contains(&format!(
            "\"spec_digest\":{{\"bytes\":11,\"sha256\":\"{}\"}}",
            original.sha256
        )),
        "record names the digest: {record}"
    );

    // Post-run, the spec source — the mutable file the record's spec id
    // points back at — is edited.
    fs::write(
        input.join("v99-first.md"),
        "# spec one\n## later guidance\n",
    )
    .expect("spec source is edited after the run");

    // The record still reports the dispatch-time size and hash.
    let reloaded = ExecutionQueue::load_named(root.path(), "run-digest")
        .expect("queue loads")
        .expect("queue exists");
    assert_eq!(
        reloaded.spec_digest("v99-first").cloned(),
        Some(original.clone())
    );

    // The per-run staged file still verifies against the recorded hash.
    assert_eq!(
        ExecutionQueue::check_staged_spec(root.path(), "run-digest", "v99-first").expect("check"),
        StagedSpecCheck::Matches
    );

    // A staged file that drifts is reported Drifted with both digests, so a
    // reader can say exactly what changed.
    let staged = root
        .path()
        .join(".autospec/runs/run-digest/specs/v99-first.md");
    fs::write(&staged, "# spec one\n## edited after the run\n")
        .expect("staged file is tampered with");
    assert_eq!(
        ExecutionQueue::check_staged_spec(root.path(), "run-digest", "v99-first").expect("check"),
        StagedSpecCheck::Drifted {
            recorded: original,
            actual: SpecDigest::from_bytes(b"# spec one\n## edited after the run\n"),
        }
    );
}

#[test]
fn spec_digest_is_content_addressed_and_optional_when_not_staged() {
    let root = TempProjectRoot::new();
    let input = root.path().join("input");
    fs::create_dir_all(&input).expect("input directory is created");
    let content = "# shared guidance block\n";
    fs::write(input.join("v99-first.md"), content).expect("first spec is written");
    fs::write(input.join("v99-second.md"), content).expect("second spec is written");

    let first = ExecutionQueue::create_if_absent_staged(
        root.path(),
        "run-a",
        &[("v99-first".to_string(), input.join("v99-first.md"))],
    )
    .expect("first run is created");
    let second = ExecutionQueue::create_if_absent_staged(
        root.path(),
        "run-b",
        &[("v99-second".to_string(), input.join("v99-second.md"))],
    )
    .expect("second run is created");

    // Same guidance content in two runs is the same version.
    let digest_a = first
        .spec_digest("v99-first")
        .cloned()
        .expect("digest captured");
    let digest_b = second
        .spec_digest("v99-second")
        .cloned()
        .expect("digest captured");
    assert_eq!(digest_a, digest_b);

    // Different content is a different version.
    fs::write(
        input.join("v99-third.md"),
        "# shared guidance block\nextra\n",
    )
    .expect("third spec is written");
    let third = ExecutionQueue::create_if_absent_staged(
        root.path(),
        "run-c",
        &[("v99-third".to_string(), input.join("v99-third.md"))],
    )
    .expect("third run is created");
    assert_ne!(
        third
            .spec_digest("v99-third")
            .cloned()
            .expect("digest captured"),
        digest_a
    );

    // A run scheduled without staged inputs records no digest, and the check
    // says exactly that rather than pretending.
    let bare =
        ExecutionQueue::create_if_absent(root.path(), "run-bare", vec!["v99-first".to_string()])
            .expect("bare queue is created");
    assert_eq!(bare.spec_digest("v99-first"), None);
    assert_eq!(
        ExecutionQueue::check_staged_spec(root.path(), "run-bare", "v99-first").expect("check"),
        StagedSpecCheck::NotRecorded
    );

    // A recorded digest whose staged file was deleted is Missing, not a
    // silent match.
    let staged = root.path().join(".autospec/runs/run-a/specs/v99-first.md");
    fs::remove_file(&staged).expect("staged file is removed");
    assert_eq!(
        ExecutionQueue::check_staged_spec(root.path(), "run-a", "v99-first").expect("check"),
        StagedSpecCheck::Missing
    );
}

#[test]
fn pre_digest_queue_documents_load_without_a_spec_digest() {
    // A queue written before the digest existed (schema 3, no spec_digest
    // key) still loads: entries parse with no recorded digest, and a later
    // save rewrites the document at the current schema with an explicit null.
    let root = TempProjectRoot::new();
    let run_directory = root.path().join(".autospec/runs/run-legacy");
    fs::create_dir_all(&run_directory).expect("run directory is created");
    fs::write(
        run_directory.join("queue.json"),
        r#"{"schema":3,"run_id":"run-legacy","updated_at":100,"revision":2,"entries":[{"spec_id":"v99-first","status":"pending","attempts":0,"failure_kind":null,"blocker":null,"started_at":null,"updated_at":100,"validation":null,"agent_result_ids":[]}]}"#,
    )
    .expect("legacy queue document is written");

    let mut queue = ExecutionQueue::load_named(root.path(), "run-legacy")
        .expect("legacy queue loads")
        .expect("queue exists");
    assert_eq!(queue.spec_digest("v99-first"), None);

    queue
        .save(root.path())
        .expect("queue is saved at the current schema");
    let record = fs::read_to_string(run_directory.join("queue.json")).expect("record is read");
    assert!(
        record.contains("\"schema\":4"),
        "rewritten at schema 4: {record}"
    );
    assert!(
        record.contains("\"spec_digest\":null"),
        "absent digest is explicit, not guessed: {record}"
    );
}
