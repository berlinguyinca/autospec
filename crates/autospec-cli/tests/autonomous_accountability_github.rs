#[path = "../src/commands/autonomous/accountability.rs"]
#[allow(dead_code)]
mod accountability;

use accountability::github::{
    bind_epic, EpicBindingRequest, GithubCommand, GithubTransport, ResumePolicy,
};
use accountability::{
    AccountabilityStore, LaunchDescriptor, LeaseGeneration, RepositoryIdentity, RunIdentity,
    RunNonce,
};
use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1);

struct Fixture(PathBuf);

impl Fixture {
    fn new(name: &str) -> Self {
        let serial = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "autospec-accountability-github-{name}-{}-{serial}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[derive(Default)]
struct StubGithub {
    responses: VecDeque<Result<String, String>>,
    calls: Vec<GithubCommand>,
    last_edit: Option<String>,
}

impl StubGithub {
    fn with(responses: impl IntoIterator<Item = Result<String, String>>) -> Self {
        Self {
            responses: responses.into_iter().collect(),
            calls: Vec::new(),
            last_edit: None,
        }
    }
}

impl GithubTransport for StubGithub {
    fn execute(&mut self, command: GithubCommand) -> Result<String, String> {
        if let GithubCommand::EditIssue { body, .. } = &command {
            self.last_edit = Some(body.clone());
        }
        let dynamic_number = match &command {
            GithubCommand::ViewIssue { number, .. } => Some(*number),
            _ => None,
        };
        self.calls.push(command);
        let response = self
            .responses
            .pop_front()
            .unwrap_or_else(|| Err("unexpected GitHub call".to_string()));
        match (response, dynamic_number) {
            (Ok(value), Some(number)) if value == "__return_last_edit__" => Ok(issue(
                number,
                "OPEN",
                self.last_edit.as_deref().expect("edit body recorded"),
            )),
            (response, _) => response,
        }
    }
}

fn run() -> RunIdentity {
    RunIdentity::derive(
        RepositoryIdentity::parse("acme/widgets").unwrap(),
        RunNonce::parse("00112233445566778899aabbccddeeff").unwrap(),
        LeaseGeneration::new(7).unwrap(),
    )
}

fn store(fixture: &Fixture) -> AccountabilityStore {
    let mut store = AccountabilityStore::open(fixture.path()).unwrap();
    store
        .begin_launch(
            LaunchDescriptor::new(
                run(),
                "Build the requested autonomous work",
                "The operator needs one understandable and resumable run record",
            )
            .unwrap(),
        )
        .unwrap();
    store
}

fn labels() -> &'static str {
    r#"[{"name":"epic"},{"name":"type:tracker"},{"name":"no-auto"},{"name":"autospec:run-accountability"}]"#
}

fn issue(number: u64, state: &str, body: &str) -> String {
    format!(
        r#"{{"number":{number},"url":"https://api.github.com/repos/acme/widgets/issues/{number}","html_url":"https://github.com/acme/widgets/issues/{number}","state":"{state}","body":{},"labels":{}}}"#,
        serde_json::to_string(body).unwrap(),
        labels()
    )
}

fn pages(issues: &[String]) -> String {
    format!("[[{}]]", issues.join(","))
}

fn request() -> EpicBindingRequest {
    EpicBindingRequest {
        repository: RepositoryIdentity::parse("acme/widgets").unwrap(),
        explicit_epic: None,
        resume_policy: ResumePolicy::ActiveOnly,
        project_number: None,
    }
}

#[test]
fn zero_matches_creates_once_then_reconciles_exact_marker() {
    let fixture = Fixture::new("create");
    let mut store = store(&fixture);
    let projection = store.render().unwrap();
    let marker = format!(
        "<!-- autospec:run-epic repo=acme/widgets run_id={} -->",
        run().run_id()
    );
    let remote = issue(42, "OPEN", &format!("{marker}\n{}", projection.markdown));
    let mut github = StubGithub::with([
        Ok("[[]]".to_string()),
        Ok(String::new()),
        Ok(String::new()),
        Ok(String::new()),
        Ok(String::new()),
        Ok("https://github.com/acme/widgets/issues/42\n".to_string()),
        Ok(pages(&[remote])),
        Ok(String::new()),
        Ok("__return_last_edit__".to_string()),
    ]);
    let mut renewals = 0;

    let binding = bind_epic(&mut store, &mut github, request(), || {
        renewals += 1;
        Ok(())
    })
    .unwrap();

    assert_eq!(binding.number, 42);
    assert!(
        renewals >= 3,
        "lease must be renewed across remote boundaries"
    );
    assert_eq!(
        github
            .calls
            .iter()
            .filter(|call| matches!(call, GithubCommand::CreateIssue { .. }))
            .count(),
        1
    );
    assert_eq!(store.status().epic_number, Some(42));
    assert_eq!(store.status().pending_projection_count, 0);
}

#[test]
fn ambiguous_create_response_never_permits_a_second_create() {
    let fixture = Fixture::new("ambiguous");
    let mut store = store(&fixture);
    let marker = format!(
        "<!-- autospec:run-epic repo=acme/widgets run_id={} -->",
        run().run_id()
    );
    let remote = issue(43, "OPEN", &marker);
    let mut github = StubGithub::with([
        Ok("[[]]".to_string()),
        Ok(String::new()),
        Ok(String::new()),
        Ok(String::new()),
        Ok(String::new()),
        Err("connection reset after request body".to_string()),
        Err("temporary list failure".to_string()),
        Ok("[[]]".to_string()),
        Ok(pages(&[remote])),
        Ok(String::new()),
        Ok("__return_last_edit__".to_string()),
    ]);

    bind_epic(&mut store, &mut github, request(), || Ok(())).unwrap();
    let projected = github.last_edit.clone().unwrap();
    assert_eq!(
        github
            .calls
            .iter()
            .filter(|call| matches!(call, GithubCommand::CreateIssue { .. }))
            .count(),
        1
    );

    let mut second = StubGithub::with([Ok(pages(&[issue(43, "OPEN", &projected)]))]);
    bind_epic(&mut store, &mut second, request(), || Ok(())).unwrap();
    assert!(second
        .calls
        .iter()
        .all(|call| !matches!(call, GithubCommand::CreateIssue { .. })));
}

#[test]
fn multiple_exact_markers_fail_closed() {
    let fixture = Fixture::new("duplicates");
    let mut store = store(&fixture);
    let marker = format!(
        "<!-- autospec:run-epic repo=acme/widgets run_id={} -->",
        run().run_id()
    );
    let mut github = StubGithub::with([Ok(pages(&[
        issue(42, "OPEN", &marker),
        issue(43, "CLOSED", &marker),
    ]))]);

    let error = bind_epic(&mut store, &mut github, request(), || Ok(())).unwrap_err();
    assert!(error.to_string().contains("multiple"));
    assert!(github
        .calls
        .iter()
        .all(|call| !matches!(call, GithubCommand::CreateIssue { .. })));
}

#[test]
fn lease_loss_during_reconciliation_fails_before_spawn_binding() {
    let fixture = Fixture::new("lease-loss");
    let mut store = store(&fixture);
    let mut github = StubGithub::with([
        Ok("[[]]".to_string()),
        Ok(String::new()),
        Ok(String::new()),
        Ok(String::new()),
        Ok(String::new()),
        Err("timeout".to_string()),
        Ok("[[]]".to_string()),
    ]);

    let error = bind_epic(&mut store, &mut github, request(), || {
        Err("lifecycle lease token mismatch".to_string())
    })
    .unwrap_err();
    assert!(error.to_string().contains("lease"));
    assert_eq!(store.status().epic_number, None);
}

#[test]
fn explicit_resume_reconstructs_manifest_reopens_and_records_resume() {
    let fixture = Fixture::new("resume");
    let empty = AccountabilityStore::open(fixture.path()).unwrap();
    drop(empty);
    let manifest = accountability::RecoveryManifest::new(
        run(),
        77,
        "https://github.com/acme/widgets/issues/77",
        4,
        "a".repeat(64),
        12,
        2,
    )
    .unwrap();
    let marker = format!(
        "<!-- autospec:run-epic repo=acme/widgets run_id={} -->",
        run().run_id()
    );
    let body = accountability::github::compose_managed_body(
        &marker,
        "Existing run overview",
        &manifest,
        "human-authored tail",
    );
    let mut github = StubGithub::with([
        Ok(issue(77, "CLOSED", &body)),
        Ok(String::new()),
        Ok(issue(77, "OPEN", &body)),
        Ok(String::new()),
        Ok("__return_last_edit__".to_string()),
    ]);
    let mut store = AccountabilityStore::open(fixture.path()).unwrap();
    let mut resume_request = request();
    resume_request.explicit_epic = Some(77);
    resume_request.resume_policy = ResumePolicy::ReopenClosed;

    let binding = bind_epic(&mut store, &mut github, resume_request, || Ok(())).unwrap();

    assert_eq!(binding.number, 77);
    assert_eq!(store.status().journal_segment, 3);
    assert_eq!(store.status().event_count, 1);
    assert!(github
        .calls
        .iter()
        .any(|call| matches!(call, GithubCommand::ReopenIssue { number: 77, .. })));
    let edited = github.calls.iter().find_map(|call| match call {
        GithubCommand::EditIssue { body, .. } => Some(body),
        _ => None,
    });
    assert!(edited.unwrap().contains("human-authored tail"));
}

#[test]
fn explicit_epic_rejects_wrong_labels_or_duplicate_markers() {
    let fixture = Fixture::new("reject-explicit");
    let mut store = AccountabilityStore::open(fixture.path()).unwrap();
    let marker = format!(
        "<!-- autospec:run-epic repo=acme/widgets run_id={} -->",
        run().run_id()
    );
    let invalid_labels = format!(
        r#"{{"number":55,"url":"https://github.com/acme/widgets/issues/55","state":"OPEN","body":{},"labels":[{{"name":"epic"}}]}}"#,
        serde_json::to_string(&marker).unwrap()
    );
    let mut github = StubGithub::with([Ok(invalid_labels)]);
    let mut explicit = request();
    explicit.explicit_epic = Some(55);
    let error = bind_epic(&mut store, &mut github, explicit, || Ok(())).unwrap_err();
    assert!(error.to_string().contains("labels"));

    let duplicate = format!("{marker}\n{marker}");
    let mut github = StubGithub::with([Ok(issue(56, "OPEN", &duplicate))]);
    let mut explicit = request();
    explicit.explicit_epic = Some(56);
    let error = bind_epic(&mut store, &mut github, explicit, || Ok(())).unwrap_err();
    assert!(error.to_string().contains("exactly one"));
}

#[test]
fn optional_project_failure_does_not_unbind_verified_epic() {
    let fixture = Fixture::new("project-warning");
    let mut store = store(&fixture);
    let marker = format!(
        "<!-- autospec:run-epic repo=acme/widgets run_id={} -->",
        run().run_id()
    );
    let remote = issue(88, "OPEN", &marker);
    let mut project_request = request();
    project_request.project_number = Some(9);
    let mut github = StubGithub::with([
        Ok(pages(std::slice::from_ref(&remote))),
        Ok(String::new()),
        Ok("__return_last_edit__".to_string()),
        Err("missing project scope".to_string()),
    ]);

    let binding = bind_epic(&mut store, &mut github, project_request, || Ok(())).unwrap();
    assert_eq!(binding.number, 88);
    assert_eq!(store.status().epic_number, Some(88));
    assert_eq!(
        binding.project_warning.as_deref(),
        Some("missing project scope")
    );
}

#[test]
fn autonomous_cli_exposes_explicit_epic_start_and_resume_contract() {
    let help = Command::new(env!("CARGO_BIN_EXE_autospec"))
        .args(["autonomous", "--help"])
        .output()
        .unwrap();
    assert!(help.status.success());
    let help = String::from_utf8_lossy(&help.stdout);
    assert!(help.contains("resume"));
    assert!(help.contains("--epic N"));

    let missing = Command::new(env!("CARGO_BIN_EXE_autospec"))
        .args(["autonomous", "resume", "--repo", "acme/widgets"])
        .output()
        .unwrap();
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("resume requires --epic N"));
}

#[test]
fn launcher_binds_verified_epic_before_any_conductor_spawn_and_supervisor_reuses_it() {
    let source = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands/autonomous.rs"),
    )
    .unwrap();
    let start = source.find("fn start(options:").unwrap();
    let after_start = &source[start..];
    let binding = after_start.find("bind_accountability_epic").unwrap();
    let launch = after_start.find("start_after_lease").unwrap();
    assert!(binding < launch, "epic binding must precede launch");

    let repair = source.find("fn repair_stopped_conductor").unwrap();
    let repair_body = &source[repair..];
    let verification = repair_body.find("verify_existing_accountability").unwrap();
    let spawn = repair_body.find("spawn_unit(").unwrap();
    assert!(
        verification < spawn,
        "supervisor must verify the inherited epic before respawn"
    );
    assert!(source.contains("\\\"accountability\\\":{}"));
    let foreground = source.find("fn run_foreground(options:").unwrap();
    let foreground_body = &source[foreground..];
    let foreground_binding = foreground_body
        .find("verify_existing_accountability")
        .unwrap();
    let foreground_cycles = foreground_body.find("run_foreground_cycles").unwrap();
    assert!(
        foreground_binding < foreground_cycles,
        "inherited foreground workers must verify accountability before work"
    );
}
