#[path = "../src/commands/autonomous/accountability.rs"]
#[allow(dead_code)]
mod accountability;

use accountability::github::{
    bind_epic, EpicBindingRequest, GithubCommand, GithubFailure, GithubTransport, ResumePolicy,
};
use accountability::{
    AccountabilityEvent, AccountabilityStore, EventKind, Evidence, LaunchDescriptor,
    LeaseGeneration, ProjectionDisposition, RepositoryIdentity, RunIdentity, RunNonce,
};
use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

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
    responses: VecDeque<Result<String, GithubFailure>>,
    calls: Vec<GithubCommand>,
    last_edit: Option<String>,
}

impl StubGithub {
    fn with(responses: impl IntoIterator<Item = Result<String, GithubFailure>>) -> Self {
        Self {
            responses: responses.into_iter().collect(),
            calls: Vec::new(),
            last_edit: None,
        }
    }
}

impl GithubTransport for StubGithub {
    fn execute(&mut self, command: GithubCommand) -> Result<String, GithubFailure> {
        if let GithubCommand::EditIssue { body, .. } = &command {
            self.last_edit = Some(body.clone());
        }
        let dynamic_number = match &command {
            GithubCommand::ViewIssue { number, .. } => Some(*number),
            _ => None,
        };
        self.calls.push(command);
        let response = self.responses.pop_front().unwrap_or_else(|| {
            Err(GithubFailure::Definitive(
                "unexpected GitHub call".to_string(),
            ))
        });
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
        adopted_lease_generation: None,
    }
}

#[path = "autonomous_accountability_github/binding.rs"]
mod binding;
#[path = "autonomous_accountability_github/contracts.rs"]
mod contracts;
#[path = "autonomous_accountability_github/projection.rs"]
mod projection;
#[path = "autonomous_accountability_github/resume.rs"]
mod resume;
