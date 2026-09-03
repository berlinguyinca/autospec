//! End-to-end coverage for `autospec initiative`.
//!
//! The fixture is the Initiative from the specification: three repositories
//! across two GitHub owners, a cross-repository dependency chain expressed in
//! AutoSpec task ids, and a completion gate that refuses to pass while a
//! requirement is unverified.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

use autospec_core::initiative::dag::TaskGraph;
use autospec_core::initiative::definition::{
    AcceptanceCriterion, Definition, Provenance, Requirement, RequirementKind,
};
use autospec_core::initiative::ids::{
    CriterionId, EvidenceId, GraphId, InitiativeId, PlanId, RequirementId, TaskId,
};
use autospec_core::initiative::plan::{ArchitecturePlan, WorkStream};
use autospec_core::initiative::projection::ProjectTarget;
use autospec_core::initiative::repository::{
    Capability, RepositoryId, RepositoryRecord, Workspace,
};
use autospec_core::initiative::store::{InitiativeArtifact, InitiativeStore};
use autospec_core::initiative::task::{Task, TaskState};
use autospec_core::initiative::traceability::{
    EvidenceKind, EvidenceOutcome, EvidenceRecord, Waiver,
};
use autospec_core::initiative::Initiative;

static NEXT_FIXTURE: AtomicUsize = AtomicUsize::new(0);

const INITIATIVE: &str = "INIT-2026-0042";

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let suffix = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "autospec-initiative-cli-{name}-{}-{suffix}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("create fixture root");
        Self { root }
    }

    fn run(&self, args: &[&str]) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_autospec"));
        command.arg("initiative");
        command.args(args);
        command.arg("--root");
        command.arg(&self.root);
        command.output().expect("autospec initiative runs")
    }

    fn store(&self) -> InitiativeStore {
        InitiativeStore::new(&self.root, initiative_id())
    }

    /// Seed a complete, valid Initiative registry.
    fn seed(&self, workspace: &Workspace, graph: &TaskGraph) {
        let output = self.run(&[
            "init",
            "--id",
            INITIATIVE,
            "--slug",
            "planning-orchestration-v2",
            "--spec",
            "spec/spec.md",
            "--now",
            "1772000000",
        ]);
        assert!(output.status.success(), "{}", stderr(&output));

        let store = self.store();
        let mut record: Initiative = store
            .read_json(&InitiativeArtifact::Record)
            .expect("initiative record");
        record.definition_version = 1;
        record.repositories = workspace.repositories.keys().cloned().collect();
        record.architecture_plan = Some(plan_id());
        record.task_graph = Some(graph_id());
        record.github_projects = vec![ProjectTarget {
            owner: "InferWeave".to_string(),
            project_number: 12,
        }];
        write(&store, &InitiativeArtifact::Record, &record);

        write(
            &store,
            &InitiativeArtifact::Definition { version: 1 },
            &definition(),
        );
        write(
            &store,
            &InitiativeArtifact::WorkspaceRepositories,
            workspace,
        );
        write(
            &store,
            &InitiativeArtifact::ArchitecturePlan { version: 1 },
            &plan(workspace),
        );
        write(&store, &InitiativeArtifact::TaskGraph { version: 1 }, graph);
    }

    fn write_evidence(&self, evidence: &[EvidenceRecord]) {
        let path = self.store().path(&InitiativeArtifact::Evidence);
        write_path(&path, evidence);
    }

    fn write_waivers(&self, waivers: &[Waiver]) {
        let path = self.store().path(&InitiativeArtifact::Waivers);
        write_path(&path, waivers);
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// Overwrite an artifact, bypassing the immutability guard the fixture owns.
fn write<T: serde::Serialize>(store: &InitiativeStore, artifact: &InitiativeArtifact, value: &T) {
    write_path(&store.path(artifact), value);
}

fn write_path<T: serde::Serialize + ?Sized>(path: &Path, value: &T) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("artifact directory");
    }
    let rendered = serde_json::to_string_pretty(value).expect("serializable artifact");
    fs::write(path, format!("{rendered}\n")).expect("artifact written");
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

fn json(output: &Output) -> serde_json::Value {
    serde_json::from_str(stdout(output).trim()).expect("json output")
}

fn initiative_id() -> InitiativeId {
    InitiativeId::parse(INITIATIVE).expect("valid initiative id")
}

fn plan_id() -> PlanId {
    PlanId::parse("PLAN-ARCH-0003").expect("valid plan id")
}

fn graph_id() -> GraphId {
    GraphId::parse("DAG-0007").expect("valid graph id")
}

fn repository(text: &str) -> RepositoryId {
    RepositoryId::parse(text).expect("valid repository id")
}

fn requirement_id(text: &str) -> RequirementId {
    RequirementId::parse(text).expect("valid requirement id")
}

fn task_id(text: &str) -> TaskId {
    TaskId::parse(text).expect("valid task id")
}

fn full_capabilities() -> BTreeSet<Capability> {
    BTreeSet::from([
        Capability::Read,
        Capability::Issues,
        Capability::Branches,
        Capability::Push,
        Capability::PullRequests,
        Capability::Workflows,
        Capability::ProjectMutation,
    ])
}

fn record(text: &str, capabilities: BTreeSet<Capability>) -> RepositoryRecord {
    RepositoryRecord {
        id: repository(text),
        revision: Some("aaa1111".to_string()),
        default_branch: Some("main".to_string()),
        credential_reference: Some("app-installation/example".to_string()),
        capabilities,
        languages: vec!["rust".to_string()],
        build_systems: vec!["cargo".to_string()],
        validation_commands: vec!["cargo test --workspace --no-fail-fast".to_string()],
    }
}

/// Three repositories across two owners.
fn workspace() -> Workspace {
    let mut workspace = Workspace::new();
    workspace.insert(record(
        "github.com/InferWeave/autospec",
        full_capabilities(),
    ));
    workspace.insert(record(
        "github.com/InferWeave/autospec-orchestrator",
        full_capabilities(),
    ));
    workspace.insert(record(
        "github.com/OtherOrg/frontend",
        BTreeSet::from([
            Capability::Read,
            Capability::Issues,
            Capability::Branches,
            Capability::Push,
            Capability::PullRequests,
        ]),
    ));
    workspace
}

fn definition() -> Definition {
    let mut definition = Definition::new(initiative_id(), 1, "sha256:specification");
    definition.requirements = ["REQ-001", "REQ-002"]
        .into_iter()
        .enumerate()
        .map(|(index, id)| Requirement {
            id: requirement_id(id),
            statement: format!("{id} holds across the Initiative"),
            kind: RequirementKind::Functional,
            acceptance: vec![AcceptanceCriterion {
                id: CriterionId::from_sequence(index as u32 + 1, 3),
                statement: "verified by an independent session".to_string(),
                objectively_verifiable: true,
                provenance: Provenance::section("Acceptance Criteria"),
            }],
            provenance: Provenance::section("Goals"),
            candidate_repositories: Vec::new(),
            open_questions: Vec::new(),
        })
        .collect();
    definition
}

fn plan(workspace: &Workspace) -> ArchitecturePlan {
    let mut plan = ArchitecturePlan::new(plan_id(), initiative_id(), 1, &definition(), workspace);
    plan.work_streams = vec![WorkStream {
        id: "WS-01".to_string(),
        summary: "orchestration core and its consumers".to_string(),
        satisfies: vec![requirement_id("REQ-001"), requirement_id("REQ-002")],
        repositories: workspace.repositories.keys().cloned().collect(),
    }];
    plan
}

/// The dependency chain from the specification: autospec -> orchestrator -> frontend.
fn graph() -> TaskGraph {
    let mut graph = TaskGraph::new(graph_id(), 1, plan_id());
    graph.insert(Task::implementation(
        task_id("TASK-001"),
        repository("github.com/InferWeave/autospec"),
        vec![requirement_id("REQ-001")],
        1,
    ));
    graph.insert(
        Task::implementation(
            task_id("TASK-002"),
            repository("github.com/InferWeave/autospec-orchestrator"),
            vec![requirement_id("REQ-001"), requirement_id("REQ-002")],
            1,
        )
        .depending_on(vec![task_id("TASK-001")]),
    );
    graph.insert(
        Task::implementation(
            task_id("TASK-003"),
            repository("github.com/OtherOrg/frontend"),
            vec![requirement_id("REQ-002")],
            1,
        )
        .depending_on(vec![task_id("TASK-002")]),
    );
    graph
}

fn evidence_chain(requirement: &str, task: &str, offset: u32) -> Vec<EvidenceRecord> {
    [
        (EvidenceKind::Implementation, EvidenceOutcome::Pass, "impl"),
        (EvidenceKind::Test, EvidenceOutcome::Pass, "test"),
        (EvidenceKind::Review, EvidenceOutcome::Approved, "rev"),
        (
            EvidenceKind::Verification,
            EvidenceOutcome::Verified,
            "verify",
        ),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, (kind, outcome, token))| EvidenceRecord {
        id: EvidenceId::from_sequence(offset + index as u32, 3),
        kind,
        outcome,
        task: task_id(task),
        requirements: vec![requirement_id(requirement)],
        session_name: format!("aspec-INIT-0042-{task}-{token}-a1"),
        reference: format!("artifact://{task}/{token}"),
    })
    .collect()
}

#[test]
fn init_creates_the_registry_layout_and_an_audit_event() {
    let fixture = Fixture::new("init");

    let output = fixture.run(&[
        "init",
        "--id",
        INITIATIVE,
        "--slug",
        "planning-orchestration-v2",
        "--now",
        "1772000000",
    ]);

    assert!(output.status.success(), "{}", stderr(&output));
    let root = fixture.root.join(".autospec/initiatives/INIT-2026-0042");
    assert!(root.join("initiative.json").is_file());
    assert!(root.join("plans").is_dir());
    assert!(root.join("verification").is_dir());
    let events = fs::read_to_string(root.join("audit/events.jsonl")).expect("audit log");
    assert!(events.contains("initiative.created"), "{events}");
}

#[test]
fn init_refuses_to_overwrite_an_existing_initiative() {
    let fixture = Fixture::new("init-twice");
    let arguments = [
        "init",
        "--id",
        INITIATIVE,
        "--slug",
        "planning-orchestration-v2",
    ];
    assert!(fixture.run(&arguments).status.success());

    let output = fixture.run(&arguments);

    assert!(!output.status.success());
    assert!(stderr(&output).contains("already exists"), "{}", stderr(&output));
}

#[test]
fn init_rejects_an_identifier_that_is_not_an_initiative_id() {
    let fixture = Fixture::new("bad-id");

    let output = fixture.run(&["init", "--id", "autospec#412", "--slug", "x"]);

    assert!(!output.status.success());
    assert!(stderr(&output).contains("INIT-"), "{}", stderr(&output));
}

#[test]
fn validate_accepts_a_three_repository_two_owner_initiative() {
    let fixture = Fixture::new("validate-ok");
    fixture.seed(&workspace(), &graph());

    let output = fixture.run(&["validate", "--id", INITIATIVE, "--json"]);

    assert!(output.status.success(), "{}", stdout(&output));
    let report = json(&output);
    assert_eq!(report["valid"], serde_json::Value::Bool(true));
    assert_eq!(report["problems"].as_array().map(Vec::len), Some(0));
}

#[test]
fn validate_reports_a_task_whose_repository_was_never_discovered() {
    let fixture = Fixture::new("validate-unknown-repo");
    let mut graph = graph();
    graph.insert(Task::implementation(
        task_id("TASK-004"),
        repository("github.com/ThirdOrg/mobile"),
        vec![requirement_id("REQ-002")],
        1,
    ));
    fixture.seed(&workspace(), &graph);

    let output = fixture.run(&["validate", "--id", INITIATIVE, "--json"]);

    assert!(!output.status.success());
    let problems = stdout(&output);
    assert!(problems.contains("undiscovered repository"), "{problems}");
}

#[test]
fn ready_releases_the_root_task_and_explains_every_block() {
    let fixture = Fixture::new("ready");
    fixture.seed(&workspace(), &graph());

    let output = fixture.run(&["ready", "--id", INITIATIVE, "--json", "--now", "1772000000"]);

    assert!(output.status.success(), "{}", stderr(&output));
    let schedule = json(&output);
    assert_eq!(schedule["ready"], serde_json::json!(["TASK-001"]));
    assert_eq!(
        schedule["blocked"]["TASK-002"]["reason"],
        serde_json::json!("dependency_unverified")
    );
    assert_eq!(
        schedule["blocked"]["TASK-003"]["reason"],
        serde_json::json!("ancestor_blocked")
    );
}

#[test]
fn a_missing_permission_blocks_only_the_branch_that_needs_it() {
    let fixture = Fixture::new("permission");
    let mut workspace = workspace();
    // The other organization revoked write access.
    workspace.insert(RepositoryRecord::read_only(repository(
        "github.com/OtherOrg/frontend",
    )));
    let mut graph = graph();
    for id in ["TASK-001", "TASK-002"] {
        graph.get_mut(&task_id(id)).expect("task exists").state = TaskState::Verified;
    }
    // An independent branch in a repository that still grants write access.
    graph.insert(Task::implementation(
        task_id("TASK-004"),
        repository("github.com/InferWeave/autospec"),
        vec![requirement_id("REQ-002")],
        1,
    ));
    fixture.seed(&workspace, &graph);

    let output = fixture.run(&["ready", "--id", INITIATIVE, "--json", "--now", "1772000000"]);

    assert!(output.status.success(), "{}", stderr(&output));
    let schedule = json(&output);
    assert_eq!(schedule["ready"], serde_json::json!(["TASK-004"]));
    assert_eq!(
        schedule["blocked"]["TASK-003"]["reason"],
        serde_json::json!("missing_capability")
    );
}

#[test]
fn verify_refuses_completion_while_a_requirement_is_unverified() {
    let fixture = Fixture::new("verify-incomplete");
    fixture.seed(&workspace(), &graph());
    fixture.write_evidence(&evidence_chain("REQ-001", "TASK-001", 10));

    let output = fixture.run(&["verify", "--id", INITIATIVE]);

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(1));
    assert!(stdout(&output).contains("REQ-002"), "{}", stdout(&output));
}

#[test]
fn verify_passes_once_every_requirement_has_independent_verification() {
    let fixture = Fixture::new("verify-complete");
    fixture.seed(&workspace(), &graph());
    let mut evidence = evidence_chain("REQ-001", "TASK-001", 10);
    evidence.extend(evidence_chain("REQ-002", "TASK-003", 20));
    fixture.write_evidence(&evidence);

    let output = fixture.run(&["verify", "--id", INITIATIVE, "--json"]);

    assert!(output.status.success(), "{}", stdout(&output));
    assert_eq!(json(&output)["complete"], serde_json::Value::Bool(true));
}

#[test]
fn verification_from_the_implementation_session_does_not_close_the_gate() {
    let fixture = Fixture::new("verify-not-independent");
    fixture.seed(&workspace(), &graph());
    let mut evidence = evidence_chain("REQ-001", "TASK-001", 10);
    evidence.extend(evidence_chain("REQ-002", "TASK-003", 20));
    // The implementer signs off on its own work.
    evidence[3].session_name = evidence[0].session_name.clone();
    fixture.write_evidence(&evidence);

    let output = fixture.run(&["verify", "--id", INITIATIVE]);

    assert!(!output.status.success());
    let report = stdout(&output);
    assert!(report.contains("REQ-001"), "{report}");
    assert!(report.contains("implementation session"), "{report}");
}

#[test]
fn an_approved_waiver_closes_the_gate_for_a_requirement_that_stays_unverified() {
    let fixture = Fixture::new("waiver");
    fixture.seed(&workspace(), &graph());
    fixture.write_evidence(&evidence_chain("REQ-001", "TASK-001", 10));
    fixture.write_waivers(&[Waiver {
        requirement: requirement_id("REQ-002"),
        reason: "the frontend ships in the next Initiative".to_string(),
        approved_by: "product-owner".to_string(),
        approved_at: 1_772_000_000,
    }]);

    let output = fixture.run(&["verify", "--id", INITIATIVE, "--json"]);

    assert!(output.status.success(), "{}", stdout(&output));
    assert_eq!(json(&output)["waived"], serde_json::json!(["REQ-002"]));
}

#[test]
fn coverage_traces_each_requirement_through_its_evidence() {
    let fixture = Fixture::new("coverage");
    fixture.seed(&workspace(), &graph());
    fixture.write_evidence(&evidence_chain("REQ-001", "TASK-001", 10));

    let output = fixture.run(&["coverage", "--id", INITIATIVE, "--json"]);

    assert!(output.status.success(), "{}", stderr(&output));
    let matrix = json(&output);
    assert_eq!(matrix["statuses"]["REQ-001"]["state"], "verified");
    assert_eq!(matrix["statuses"]["REQ-002"]["state"], "in_progress");
    assert_eq!(
        matrix["statuses"]["REQ-001"]["evidence"].as_array().map(Vec::len),
        Some(4)
    );
}

#[test]
fn project_renders_issues_across_two_owners_and_is_rebuilt_from_canonical_state() {
    let fixture = Fixture::new("project");
    fixture.seed(&workspace(), &graph());

    let first = fixture.run(&["project", "--id", INITIATIVE, "--json", "--now", "1772000000"]);
    assert!(first.status.success(), "{}", stderr(&first));
    let projection = json(&first);
    assert_eq!(projection["issues"].as_object().map(serde_json::Map::len), Some(3));
    assert_eq!(
        projection["issues"]["TASK-003"]["repository"],
        "github.com/OtherOrg/frontend"
    );
    // The second organization does not grant Project mutation.
    assert_eq!(projection["items"].as_array().map(Vec::len), Some(2));

    // Deleting the rendered projection loses nothing: it rebuilds identically.
    let rendered = fixture.store().path(&InitiativeArtifact::GithubProjection);
    fs::remove_file(&rendered).expect("projection removed");
    let second = fixture.run(&["project", "--id", INITIATIVE, "--json", "--now", "1772000000"]);

    assert!(second.status.success(), "{}", stderr(&second));
    assert_eq!(json(&second), projection);
    assert!(rendered.is_file());
}

#[test]
fn status_summarizes_stage_scheduling_and_coverage() {
    let fixture = Fixture::new("status");
    fixture.seed(&workspace(), &graph());

    let output = fixture.run(&["status", "--id", INITIATIVE, "--json", "--now", "1772000000"]);

    assert!(output.status.success(), "{}", stderr(&output));
    let snapshot = json(&output);
    assert_eq!(snapshot["stage"], "scheduled");
    assert_eq!(snapshot["repository_count"], 3);
    assert_eq!(snapshot["owner_scope_count"], 2);
    assert_eq!(snapshot["ready_tasks"], 1);
    assert_eq!(snapshot["completion"]["complete"], false);
}

#[test]
fn commands_report_a_missing_registry_instead_of_panicking() {
    let fixture = Fixture::new("missing");

    let output = fixture.run(&["status", "--id", INITIATIVE]);

    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("no initiative registry"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn subcommands_require_an_initiative_id() {
    let fixture = Fixture::new("no-id");

    let output = fixture.run(&["status"]);

    assert!(!output.status.success());
    assert!(stderr(&output).contains("--id"), "{}", stderr(&output));
}
