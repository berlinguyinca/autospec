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
        Err(GithubFailure::Ambiguous(
            "connection reset after request body".to_string(),
        )),
        Err(GithubFailure::RetryAfter {
            message: "Retry-After: 0".to_string(),
            delay: Duration::from_millis(1),
        }),
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
        Err(GithubFailure::Ambiguous("timeout".to_string())),
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
    let projection = "Existing run overview";
    let manifest = accountability::RecoveryManifest::new(
        run(),
        77,
        "https://github.com/acme/widgets/issues/77",
        4,
        autospec_core::autonomous::waterfall::sha256_hex(format!("{projection}\n").as_bytes()),
        12,
        2,
    )
    .unwrap()
    .with_recovery_state(accountability::RecoveryState::Parked, vec![], vec![])
    .unwrap();
    let marker = format!(
        "<!-- autospec:run-epic repo=acme/widgets run_id={} -->",
        run().run_id()
    );
    let body = accountability::github::compose_managed_body(
        &marker,
        projection,
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
fn resume_policy_rejects_parked_open_and_active_closed_epics() {
    for (name, state, recovery_state, policy) in [
        (
            "parked-open",
            "OPEN",
            accountability::RecoveryState::Parked,
            ResumePolicy::ActiveOnly,
        ),
        (
            "active-closed",
            "CLOSED",
            accountability::RecoveryState::Active,
            ResumePolicy::ReopenClosed,
        ),
        (
            "active-open-unowned",
            "OPEN",
            accountability::RecoveryState::Active,
            ResumePolicy::ActiveOnly,
        ),
    ] {
        let fixture = Fixture::new(name);
        let mut store = AccountabilityStore::open(fixture.path()).unwrap();
        let projection = "Existing run overview";
        let manifest = accountability::RecoveryManifest::new(
            run(),
            79,
            "https://github.com/acme/widgets/issues/79",
            4,
            autospec_core::autonomous::waterfall::sha256_hex(format!("{projection}\n").as_bytes()),
            12,
            2,
        )
        .unwrap()
        .with_recovery_state(recovery_state, vec![], vec![])
        .unwrap();
        let marker = format!(
            "<!-- autospec:run-epic repo=acme/widgets run_id={} -->",
            run().run_id()
        );
        let body = accountability::github::compose_managed_body(&marker, projection, &manifest, "");
        let mut github = StubGithub::with([Ok(issue(79, state, &body))]);
        let mut explicit = request();
        explicit.explicit_epic = Some(79);
        explicit.resume_policy = policy;

        let error = bind_epic(&mut store, &mut github, explicit, || Ok(())).unwrap_err();
        assert!(error.to_string().contains("policy"));
    }
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
        Err(GithubFailure::Definitive(
            "missing project scope".to_string(),
        )),
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
fn definitive_create_failure_is_not_reclassified_as_unknown_or_retried() {
    let fixture = Fixture::new("definitive-create");
    let mut store = store(&fixture);
    let mut github = StubGithub::with([
        Ok("[[]]".to_string()),
        Ok(String::new()),
        Ok(String::new()),
        Ok(String::new()),
        Ok(String::new()),
        Err(GithubFailure::Definitive("validation failed".to_string())),
    ]);

    let error = bind_epic(&mut store, &mut github, request(), || Ok(())).unwrap_err();
    assert!(error.to_string().contains("validation failed"));
    assert_eq!(
        github
            .calls
            .iter()
            .filter(|call| matches!(call, GithubCommand::CreateIssue { .. }))
            .count(),
        1
    );
}

#[test]
fn crash_after_local_binding_projects_missing_manifest_without_creating_again() {
    let fixture = Fixture::new("bound-before-manifest");
    let mut store = store(&fixture);
    let projection = store.render().unwrap();
    let marker = format!(
        "<!-- autospec:run-epic repo=acme/widgets run_id={} -->",
        run().run_id()
    );
    store
        .bind_epic(91, "https://github.com/acme/widgets/issues/91")
        .unwrap();
    assert_eq!(store.status().pending_projection_count, 1);
    let remote_without_manifest = issue(91, "OPEN", &format!("{marker}\n{}", projection.markdown));
    let mut github = StubGithub::with([
        Ok(pages(&[remote_without_manifest])),
        Ok(String::new()),
        Ok("__return_last_edit__".to_string()),
    ]);

    let binding = bind_epic(&mut store, &mut github, request(), || Ok(())).unwrap();
    assert_eq!(binding.number, 91);
    assert_eq!(store.status().pending_projection_count, 0);
    assert!(github
        .calls
        .iter()
        .all(|call| !matches!(call, GithubCommand::CreateIssue { .. })));
}

#[test]
fn managed_projection_rejects_duplicate_blocks_and_digest_mismatch() {
    let marker = format!(
        "<!-- autospec:run-epic repo=acme/widgets run_id={} -->",
        run().run_id()
    );
    let manifest = accountability::RecoveryManifest::new(
        run(),
        92,
        "https://github.com/acme/widgets/issues/92",
        3,
        "a".repeat(64),
        0,
        1,
    )
    .unwrap();
    let body = accountability::github::compose_managed_body(&marker, "projection", &manifest, "");
    let duplicate = body.replace(
        "<!-- autospec:accountability:end -->",
        "<!-- autospec:accountability:end -->\n<!-- autospec:accountability:start -->\nextra\n<!-- autospec:accountability:end -->",
    );
    for invalid in [
        duplicate,
        body.replacen("\nprojection\n", "\ntampered\n", 1),
    ] {
        let fixture = Fixture::new("managed-integrity");
        let mut store = store(&fixture);
        let mut github = StubGithub::with([Ok(issue(92, "OPEN", &invalid))]);
        let mut explicit = request();
        explicit.explicit_epic = Some(92);
        let error = bind_epic(&mut store, &mut github, explicit, || Ok(())).unwrap_err();
        assert!(
            error.to_string().contains("managed") || error.to_string().contains("digest"),
            "unexpected error: {error}"
        );
    }
}

#[test]
fn retryable_issue_failure_keeps_the_spawned_run_active() {
    let fixture = Fixture::new("retryable-failure");
    let mut store = store(&fixture);
    store
        .bind_epic(93, "https://github.com/acme/widgets/issues/93")
        .unwrap();
    store.mark_spawned().unwrap();
    store
        .append_event(
            AccountabilityEvent::new(
                EventKind::Failed,
                "Issue attempt failed",
                "The issue remains eligible for bounded retry",
                vec![Evidence::outcome("retry scheduled")],
            )
            .unwrap(),
        )
        .unwrap();

    assert_eq!(store.status().lifecycle_phase, "spawned");
}

#[test]
fn pending_projection_coalesces_later_events_without_losing_high_watermark() {
    let fixture = Fixture::new("outbox-coalesce");
    let mut store = store(&fixture);
    store
        .append_event(
            AccountabilityEvent::new(
                EventKind::RunStarted,
                "Run started",
                "The launch is durable",
                vec![Evidence::outcome("started")],
            )
            .unwrap(),
        )
        .unwrap();
    let first = store.render().unwrap();
    store
        .append_event(
            AccountabilityEvent::new(
                EventKind::IssueClaimed { issue: 41 },
                "Issue claimed",
                "The next unit of work was selected",
                vec![Evidence::github_url("https://github.com/acme/widgets/issues/41").unwrap()],
            )
            .unwrap(),
        )
        .unwrap();

    let retry = store.projection_for_delivery().unwrap();
    assert_eq!(retry.revision, first.revision + 1);
    assert_eq!(retry.desired_high_watermark, 2);
    assert!(retry.markdown.contains("#41"));
}

#[test]
fn parked_projection_carries_links_and_closes_the_epic() {
    let fixture = Fixture::new("park-close");
    let mut store = store(&fixture);
    let marker = format!(
        "<!-- autospec:run-epic repo=acme/widgets run_id={} -->",
        run().run_id()
    );
    let remote = issue(94, "OPEN", &marker);
    let mut initial = StubGithub::with([
        Ok(pages(&[remote])),
        Ok(String::new()),
        Ok("__return_last_edit__".to_string()),
    ]);
    bind_epic(&mut store, &mut initial, request(), || Ok(())).unwrap();

    store
        .append_event(
            AccountabilityEvent::new(
                EventKind::IssueClaimed { issue: 41 },
                "Issue claimed",
                "The run owns this issue",
                vec![Evidence::github_url("https://github.com/acme/widgets/issues/41").unwrap()],
            )
            .unwrap(),
        )
        .unwrap();
    store
        .append_event(
            AccountabilityEvent::new(
                EventKind::PullRequestOpened { pull_request: 52 },
                "Pull request opened",
                "The implementation is reviewable",
                vec![Evidence::github_url("https://github.com/acme/widgets/pull/52").unwrap()],
            )
            .unwrap(),
        )
        .unwrap();
    store
        .append_event(
            AccountabilityEvent::new(
                EventKind::Parked,
                "Run parked",
                "The operator requested a resumable stop",
                vec![Evidence::outcome("parked")],
            )
            .unwrap(),
        )
        .unwrap();
    store.render().unwrap();
    let current = initial.last_edit.unwrap();
    let mut github = StubGithub::with([
        Ok(pages(&[issue(94, "OPEN", &current)])),
        Ok(String::new()),
        Ok("__return_last_edit__".to_string()),
        Ok(String::new()),
    ]);

    bind_epic(&mut store, &mut github, request(), || Ok(())).unwrap();

    let projected = github.last_edit.unwrap();
    assert!(projected.contains("\"recovery_state\":\"parked\""));
    assert!(projected.contains("\"linked_issues\":[41]"));
    assert!(projected.contains("\"linked_pull_requests\":[52]"));
    assert!(github
        .calls
        .iter()
        .any(|call| matches!(call, GithubCommand::CloseIssue { number: 94, .. })));
    assert!(store.status().last_projected_at.is_some());
}

#[test]
fn marker_integrity_failure_is_typed_separately_from_transport_degradation() {
    let fixture = Fixture::new("typed-integrity");
    let mut store = store(&fixture);
    let marker = format!(
        "<!-- autospec:run-epic repo=acme/widgets run_id={} -->",
        run().run_id()
    );
    let mut github = StubGithub::with([Ok(pages(&[
        issue(95, "OPEN", &marker),
        issue(96, "CLOSED", &marker),
    ]))]);

    let error = bind_epic(&mut store, &mut github, request(), || Ok(())).unwrap_err();
    assert_eq!(
        error.projection_disposition(),
        Some(ProjectionDisposition::IntegrityBlock)
    );
}

#[test]
fn retryable_transport_failure_is_typed_as_degradable() {
    let fixture = Fixture::new("typed-transport");
    let mut store = store(&fixture);
    let mut github = StubGithub::with([Err(GithubFailure::Retryable(
        "temporary network outage".to_string(),
    ))]);

    let error = bind_epic(&mut store, &mut github, request(), || Ok(())).unwrap_err();
    assert_eq!(
        error.projection_disposition(),
        Some(ProjectionDisposition::DegradableTransport)
    );
}

#[test]
fn sanitizer_redacts_pem_payloads_and_embedded_absolute_paths() {
    let fixture = Fixture::new("sanitizer-followup");
    let mut store = store(&fixture);
    store
        .append_event(
            AccountabilityEvent::new(
                EventKind::Verified,
                "Checked path=/Users/alice/private and home=~/secret",
                "-----BEGIN PRIVATE KEY----- c2VjcmV0LXBheWxvYWQ= -----END PRIVATE KEY-----",
                vec![Evidence::outcome(
                    "config=C:\\Users\\alice\\secret share=\\\\server\\private",
                )],
            )
            .unwrap(),
        )
        .unwrap();

    let markdown = store.render().unwrap().markdown;
    for forbidden in [
        "/Users/alice/private",
        "~/secret",
        "c2VjcmV0LXBheWxvYWQ=",
        "C:\\Users\\alice\\secret",
        "\\\\server\\private",
    ] {
        assert!(
            !markdown.contains(forbidden),
            "leaked {forbidden}: {markdown}"
        );
    }
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
