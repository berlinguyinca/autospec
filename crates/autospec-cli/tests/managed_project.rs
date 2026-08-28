#[path = "../src/commands/mod.rs"]
mod commands;

use autospec_core::managed_project::ManagedProjectPolicy;
use autospec_core::managed_project::{
    ProductKey, RelationshipEdge, RelationshipEvidence, RelationshipKind, RelationshipState,
    RepositoryRecord,
};
use commands::autonomous::accountability::github::{GithubCommand, GithubFailure, GithubTransport};
use commands::managed_project::{
    reconcile_issue, resolve_or_create_project, retry_pending_projections, verify_managed_marker,
    ManagedProjectStore,
};
use std::collections::VecDeque;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1);

struct Fixture(PathBuf);

impl Fixture {
    fn new(name: &str) -> Self {
        let serial = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "autospec-managed-project-{name}-{}-{serial}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn state_path(&self, name: &str) -> PathBuf {
        self.0.join("projects").join("autospec").join(name)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn key(value: &str) -> ProductKey {
    ProductKey::new(value).unwrap()
}

fn repository(repository: &str, entry_kind: &str) -> RepositoryRecord {
    RepositoryRecord {
        repository: repository.to_owned(),
        entry_kind: entry_kind.to_owned(),
    }
}

fn edge() -> RelationshipEdge {
    RelationshipEdge {
        product_key: key("autospec"),
        kind: RelationshipKind::DependsOn,
        source: "berlinguyinca/autospec".to_owned(),
        target: "berlinguyinca/autospec-node".to_owned(),
        evidence: RelationshipEvidence {
            kind: "manifest-dependency".to_owned(),
            location: "Cargo.toml".to_owned(),
            discovered_at: "2026-08-27T00:00:00Z".to_owned(),
            confidence: 100,
        },
        state: RelationshipState::Active,
    }
}

fn add_item(issue_url: &str) -> String {
    format!("project:item-add:PV_123:{issue_url}")
}

#[derive(Default)]
struct ScriptedGithub {
    responses: VecDeque<Result<String, GithubFailure>>,
    calls: Vec<GithubCommand>,
}

impl ScriptedGithub {
    fn with(responses: impl IntoIterator<Item = Result<String, GithubFailure>>) -> Self {
        Self {
            responses: responses.into_iter().collect(),
            calls: Vec::new(),
        }
    }
}

impl GithubTransport for ScriptedGithub {
    fn execute(&mut self, command: GithubCommand) -> Result<String, GithubFailure> {
        self.calls.push(command);
        self.responses.pop_front().unwrap_or_else(|| {
            Err(GithubFailure::Definitive(
                "unexpected GitHub call".to_owned(),
            ))
        })
    }
}

fn policy(owner: &str) -> ManagedProjectPolicy {
    ManagedProjectPolicy {
        product_key: key("autospec"),
        owner: owner.to_owned(),
        repository_seeds: vec![format!("{owner}/autospec")],
        repo_allowlist: vec![format!("{owner}/autospec")],
        discovery_max_repos: 25,
    }
}

fn marker(owner: &str) -> String {
    format!(
        "<!-- autospec-managed-project:begin -->\nschema: 1\nproduct-key: autospec\nowner: {owner}\n<!-- autospec-managed-project:end -->"
    )
}

fn project(number: u64, owner: &str, title: &str, readme: &str) -> String {
    serde_json::json!({
        "id": format!("PVT_{number}"),
        "number": number,
        "url": format!("https://github.com/orgs/{owner}/projects/{number}"),
        "title": title,
        "owner": { "login": owner },
        "readme": readme,
    })
    .to_string()
}

fn project_list(projects: serde_json::Value) -> String {
    serde_json::json!({ "projects": projects }).to_string()
}

fn item_list(urls: &[&str]) -> String {
    serde_json::json!({
        "items": urls.iter().map(|url| serde_json::json!({"content": {"type": "Issue", "url": url}})).collect::<Vec<_>>()
    })
    .to_string()
}

#[test]
fn github_marker_parser_requires_one_complete_exact_marker() {
    assert!(verify_managed_marker(&marker("berlinguyinca"), &policy("berlinguyinca")).unwrap());
    assert!(!verify_managed_marker("# Human project", &policy("berlinguyinca")).unwrap());
    assert!(verify_managed_marker(
        &format!("{}\n{}", marker("berlinguyinca"), marker("berlinguyinca")),
        &policy("berlinguyinca")
    )
    .is_err());
}

#[test]
fn github_resolve_creates_marks_verifies_and_persists_when_no_marker_matches() {
    let fixture = Fixture::new("github-create");
    let mut store = ManagedProjectStore::open(fixture.path(), &key("autospec")).unwrap();
    let human_readme = "# Human notes\n\nKeep this text.\n \n\t";
    let marked_readme = format!("{human_readme}\n\n{}", marker("berlinguyinca"));
    let mut github = ScriptedGithub::with([
        Ok(project_list(serde_json::json!([{
            "number": 3,
            "title": "Autospec"
        }]))),
        Ok(project(3, "berlinguyinca", "Autospec", "# Title only")),
        Ok(project(7, "berlinguyinca", "Autospec", human_readme)),
        Ok(project(7, "berlinguyinca", "Autospec", human_readme)),
        Ok(String::new()),
        Ok(project(7, "berlinguyinca", "Autospec", &marked_readme)),
    ]);

    let resolved = resolve_or_create_project(
        &mut store,
        &mut github,
        &policy("berlinguyinca"),
        "Autospec",
    )
    .unwrap();

    assert_eq!(resolved.number, 7);
    assert_eq!(store.snapshot().project_node_id.as_deref(), Some("PVT_7"));
    assert_eq!(store.snapshot().project_number, Some(7));
    assert!(store.snapshot().pending_projections.is_empty());
    assert_eq!(
        github
            .calls
            .iter()
            .filter(|call| matches!(call, GithubCommand::CreateProject { .. }))
            .count(),
        1
    );
    let edited = github.calls.iter().find_map(|call| match call {
        GithubCommand::EditProjectMarker { readme, .. } => Some(readme),
        _ => None,
    });
    assert_eq!(edited.map(String::as_str), Some(marked_readme.as_str()));
}

#[test]
fn github_create_failure_persists_identity_and_resumes_marker_edit_without_duplicate_create() {
    let fixture = Fixture::new("github-create-resume");
    let policy = policy("berlinguyinca");
    let mut store = ManagedProjectStore::open(fixture.path(), &key("autospec")).unwrap();
    let human_readme = "human bytes\n \n";
    let marked = format!("{human_readme}\n\n{}", marker("berlinguyinca"));
    let mut first = ScriptedGithub::with([
        Ok(project_list(serde_json::json!([]))),
        Ok(project(7, "berlinguyinca", "Autospec", human_readme)),
        Err(GithubFailure::Retryable("view interrupted".to_owned())),
    ]);

    assert!(resolve_or_create_project(&mut store, &mut first, &policy, "Autospec").is_err());
    assert_eq!(store.snapshot().project_node_id, None);
    assert_eq!(store.snapshot().project_number, None);
    assert_eq!(store.snapshot().pending_projections.len(), 1);
    drop(store);

    let mut reopened = ManagedProjectStore::open(fixture.path(), &key("autospec")).unwrap();
    let mut retry = ScriptedGithub::with([
        Ok(project(7, "berlinguyinca", "Autospec", human_readme)),
        Ok(String::new()),
        Ok(project(7, "berlinguyinca", "Autospec", &marked)),
    ]);
    resolve_or_create_project(&mut reopened, &mut retry, &policy, "Autospec").unwrap();

    assert_eq!(
        reopened.snapshot().project_node_id.as_deref(),
        Some("PVT_7")
    );
    assert_eq!(reopened.snapshot().project_number, Some(7));
    assert!(reopened.snapshot().pending_projections.is_empty());
    assert!(retry.calls.iter().all(|call| !matches!(
        call,
        GithubCommand::ListProjects { .. } | GithubCommand::CreateProject { .. }
    )));
    let edited = retry.calls.iter().find_map(|call| match call {
        GithubCommand::EditProjectMarker { readme, .. } => Some(readme.as_str()),
        _ => None,
    });
    assert_eq!(edited, Some(marked.as_str()));
}

#[test]
fn github_provisional_creation_cannot_authorize_item_mutation() {
    let fixture = Fixture::new("github-provisional-create");
    let policy = policy("berlinguyinca");
    let mut store = ManagedProjectStore::open(fixture.path(), &key("autospec")).unwrap();
    let mut create = ScriptedGithub::with([
        Ok(project_list(serde_json::json!([]))),
        Ok(project(7, "berlinguyinca", "Autospec", "human")),
        Err(GithubFailure::Retryable("view interrupted".to_owned())),
    ]);

    assert!(resolve_or_create_project(&mut store, &mut create, &policy, "Autospec").is_err());
    assert_eq!(store.snapshot().project_node_id, None);
    assert_eq!(store.snapshot().project_number, None);
    let journal = fs::read_to_string(fixture.state_path("events.jsonl")).unwrap();
    assert!(journal.contains("\"kind\":\"project-created\""));
    assert!(!journal.contains("\"kind\":\"project-bound\""));
    drop(store);

    let mut reopened = ManagedProjectStore::open(fixture.path(), &key("autospec")).unwrap();
    let mut reconcile = ScriptedGithub::default();
    assert!(reconcile_issue(
        &mut reopened,
        &mut reconcile,
        &policy,
        "https://github.com/berlinguyinca/autospec/issues/47",
    )
    .is_err());
    assert!(reconcile.calls.is_empty());
    assert_eq!(reopened.snapshot().pending_projections.len(), 1);
}

#[test]
fn github_ambiguous_marker_edit_resumes_from_verified_bound_project() {
    let fixture = Fixture::new("github-edit-resume");
    let policy = policy("berlinguyinca");
    let mut store = ManagedProjectStore::open(fixture.path(), &key("autospec")).unwrap();
    let unmarked = project(7, "berlinguyinca", "Autospec", "human");
    let marked = project(
        7,
        "berlinguyinca",
        "Autospec",
        &format!("human\n\n{}", marker("berlinguyinca")),
    );
    let mut first = ScriptedGithub::with([
        Ok(project_list(serde_json::json!([]))),
        Ok(unmarked.clone()),
        Ok(unmarked),
        Err(GithubFailure::Retryable("edit response lost".to_owned())),
    ]);
    assert!(resolve_or_create_project(&mut store, &mut first, &policy, "Autospec").is_err());
    assert_eq!(store.snapshot().pending_projections.len(), 1);

    let mut retry = ScriptedGithub::with([Ok(marked)]);
    resolve_or_create_project(&mut store, &mut retry, &policy, "Autospec").unwrap();

    assert!(store.snapshot().pending_projections.is_empty());
    assert_eq!(retry.calls.len(), 1);
    assert!(matches!(retry.calls[0], GithubCommand::ViewProject { .. }));
}

#[test]
fn github_pending_create_without_identity_never_creates_a_second_project() {
    let fixture = Fixture::new("github-create-ambiguous");
    let policy = policy("berlinguyinca");
    let mut store = ManagedProjectStore::open(fixture.path(), &key("autospec")).unwrap();
    store.enqueue_projection("project:create:autospec").unwrap();
    let mut github = ScriptedGithub::with([Ok(project_list(serde_json::json!([])))]);

    let error = resolve_or_create_project(&mut store, &mut github, &policy, "Autospec")
        .expect_err("an unbound pending create is ambiguous");

    assert!(error.to_string().contains("pending project creation"));
    assert_eq!(store.snapshot().pending_projections.len(), 1);
    assert!(github
        .calls
        .iter()
        .all(|call| !matches!(call, GithubCommand::CreateProject { .. })));
}

#[test]
fn github_verified_adoption_acknowledges_a_pending_create_projection() {
    let fixture = Fixture::new("github-create-adopt");
    let policy = policy("berlinguyinca");
    let mut store = ManagedProjectStore::open(fixture.path(), &key("autospec")).unwrap();
    store.enqueue_projection("project:create:autospec").unwrap();
    let mut github = ScriptedGithub::with([
        Ok(project_list(serde_json::json!([{"number": 7}]))),
        Ok(project(
            7,
            "berlinguyinca",
            "Autospec",
            &marker("berlinguyinca"),
        )),
    ]);

    resolve_or_create_project(&mut store, &mut github, &policy, "Autospec").unwrap();

    assert!(store.snapshot().pending_projections.is_empty());
    assert!(github
        .calls
        .iter()
        .all(|call| !matches!(call, GithubCommand::CreateProject { .. })));
}

#[test]
fn github_project_readme_must_be_a_string_before_any_mutation() {
    for (name, readme) in [
        ("missing", None),
        ("null", Some(serde_json::Value::Null)),
        ("number", Some(serde_json::json!(7))),
    ] {
        let fixture = Fixture::new(&format!("github-readme-{name}"));
        let mut store = ManagedProjectStore::open(fixture.path(), &key("autospec")).unwrap();
        let mut value: serde_json::Value =
            serde_json::from_str(&project(7, "berlinguyinca", "Autospec", "human")).unwrap();
        match readme {
            Some(readme) => value["readme"] = readme,
            None => {
                value.as_object_mut().unwrap().remove("readme");
            }
        }
        let mut github = ScriptedGithub::with([
            Ok(project_list(serde_json::json!([{"number": 7}]))),
            Ok(value.to_string()),
        ]);

        assert!(resolve_or_create_project(
            &mut store,
            &mut github,
            &policy("berlinguyinca"),
            "Autospec"
        )
        .is_err());
        assert!(store.snapshot().pending_projections.is_empty());
        assert!(github.calls.iter().all(|call| !matches!(
            call,
            GithubCommand::CreateProject { .. } | GithubCommand::EditProjectMarker { .. }
        )));
    }
}

#[test]
fn github_resolve_adopts_one_exact_marker_and_ignores_title_only_matches() {
    let fixture = Fixture::new("github-adopt");
    let mut store = ManagedProjectStore::open(fixture.path(), &key("autospec")).unwrap();
    let mut github = ScriptedGithub::with([
        Ok(project_list(serde_json::json!([
            {"number": 3, "title": "Autospec"},
            {"number": 7, "title": "Renamed by a human"}
        ]))),
        Ok(project(3, "berlinguyinca", "Autospec", "# No marker")),
        Ok(project(
            7,
            "berlinguyinca",
            "Renamed by a human",
            &marker("berlinguyinca"),
        )),
    ]);

    let resolved = resolve_or_create_project(
        &mut store,
        &mut github,
        &policy("berlinguyinca"),
        "Autospec",
    )
    .unwrap();

    assert_eq!(resolved.number, 7);
    assert_eq!(
        store.snapshot().project_title.as_deref(),
        Some("Renamed by a human")
    );
    assert!(github.calls.iter().all(|call| !matches!(
        call,
        GithubCommand::CreateProject { .. } | GithubCommand::EditProjectMarker { .. }
    )));
}

#[test]
fn github_resolve_rejects_ambiguous_markers_without_mutation() {
    let fixture = Fixture::new("github-ambiguous");
    let mut store = ManagedProjectStore::open(fixture.path(), &key("autospec")).unwrap();
    let mut github = ScriptedGithub::with([
        Ok(project_list(
            serde_json::json!([{"number": 7}, {"number": 8}]),
        )),
        Ok(project(7, "berlinguyinca", "One", &marker("berlinguyinca"))),
        Ok(project(8, "berlinguyinca", "Two", &marker("berlinguyinca"))),
    ]);

    assert!(resolve_or_create_project(
        &mut store,
        &mut github,
        &policy("berlinguyinca"),
        "Autospec"
    )
    .is_err());
    assert!(store.snapshot().project_node_id.is_none());
    assert!(store.snapshot().pending_projections.is_empty());
    assert!(github.calls.iter().all(|call| !matches!(
        call,
        GithubCommand::CreateProject { .. } | GithubCommand::EditProjectMarker { .. }
    )));
}

#[test]
fn github_resolve_fails_closed_when_project_discovery_may_be_truncated() {
    let fixture = Fixture::new("github-truncated-projects");
    let mut store = ManagedProjectStore::open(fixture.path(), &key("autospec")).unwrap();
    let projects = (1..=500)
        .map(|number| serde_json::json!({"number": number}))
        .collect::<Vec<_>>();
    let mut github = ScriptedGithub::with([Ok(project_list(serde_json::json!(projects)))]);

    let error = resolve_or_create_project(
        &mut store,
        &mut github,
        &policy("berlinguyinca"),
        "Autospec",
    )
    .unwrap_err();

    assert!(error.to_string().contains("truncated"));
    assert_eq!(github.calls.len(), 1);
    assert!(store.snapshot().pending_projections.is_empty());
}

#[test]
fn github_resolve_fails_closed_on_marker_owner_mismatch() {
    let fixture = Fixture::new("github-owner-mismatch");
    let mut store = ManagedProjectStore::open(fixture.path(), &key("autospec")).unwrap();
    let mut github = ScriptedGithub::with([
        Ok(project_list(serde_json::json!([{"number": 7}]))),
        Ok(project(
            7,
            "berlinguyinca",
            "Autospec",
            &marker("someone-else"),
        )),
    ]);

    let error = resolve_or_create_project(
        &mut store,
        &mut github,
        &policy("berlinguyinca"),
        "Autospec",
    )
    .unwrap_err();
    assert!(error.to_string().contains("owner"));
    assert!(store.snapshot().pending_projections.is_empty());
    assert!(github
        .calls
        .iter()
        .all(|call| !matches!(call, GithubCommand::CreateProject { .. })));
}

#[test]
fn github_reconcile_is_idempotent_and_journals_failures_before_item_add() {
    let fixture = Fixture::new("github-reconcile");
    let policy = policy("berlinguyinca");
    let issue_url = "https://github.com/berlinguyinca/autospec/issues/42";
    let mut store = ManagedProjectStore::open(fixture.path(), &key("autospec")).unwrap();
    let mut resolver = ScriptedGithub::with([
        Ok(project_list(serde_json::json!([{"number": 7}]))),
        Ok(project(
            7,
            "berlinguyinca",
            "Autospec",
            &marker("berlinguyinca"),
        )),
    ]);
    resolve_or_create_project(&mut store, &mut resolver, &policy, "Autospec").unwrap();

    let mut already_present = ScriptedGithub::with([Ok(item_list(&[
        "HTTPS://GITHUB.COM/berlinguyinca/autospec/issues/00042/?view=1",
    ]))]);
    reconcile_issue(&mut store, &mut already_present, &policy, issue_url).unwrap();
    assert!(already_present
        .calls
        .iter()
        .all(|call| !matches!(call, GithubCommand::AddToProject { .. })));

    let mut failing = ScriptedGithub::with([
        Ok(item_list(&[])),
        Err(GithubFailure::Retryable("rate limited".to_owned())),
    ]);
    assert!(reconcile_issue(
        &mut store,
        &mut failing,
        &policy,
        "https://github.com/berlinguyinca/autospec/issues/43"
    )
    .is_err());
    assert_eq!(store.snapshot().pending_projections.len(), 1);

    let mut retry = ScriptedGithub::with([Ok(item_list(&[
        "https://github.com/berlinguyinca/autospec/issues/43",
    ]))]);
    retry_pending_projections(&mut store, &mut retry, &policy).unwrap();
    assert!(store.snapshot().pending_projections.is_empty());
    assert!(retry.calls.iter().all(|call| !matches!(
        call,
        GithubCommand::CreateIssue { .. } | GithubCommand::AddToProject { .. }
    )));
}

#[test]
fn github_item_reconciliation_ignores_known_nonissues_but_rejects_unknown_items() {
    let fixture = Fixture::new("github-item-shapes");
    let policy = policy("berlinguyinca");
    let mut store = ManagedProjectStore::open(fixture.path(), &key("autospec")).unwrap();
    store
        .record_project(
            "berlinguyinca",
            "PVT_7",
            7,
            "https://github.com/orgs/berlinguyinca/projects/7",
            "Autospec",
        )
        .unwrap();
    let pull_request_items = serde_json::json!({
        "items": [{"content": {"type": "PullRequest", "url": "https://github.com/berlinguyinca/autospec/pull/9"}}]
    })
    .to_string();
    let mut known = ScriptedGithub::with([Ok(pull_request_items), Ok(String::new())]);
    reconcile_issue(
        &mut store,
        &mut known,
        &policy,
        "https://github.com/berlinguyinca/autospec/issues/44",
    )
    .unwrap();
    assert!(known
        .calls
        .iter()
        .any(|call| matches!(call, GithubCommand::AddToProject { .. })));

    for invalid_item in [
        serde_json::json!({
            "content": {"type": "Mystery", "url": "https://github.com/berlinguyinca/autospec/issues/45"}
        }),
        serde_json::json!({
            "content": {"url": "https://github.com/berlinguyinca/autospec/issues/45"}
        }),
    ] {
        let invalid_items = serde_json::json!({"items": [invalid_item]}).to_string();
        let mut invalid = ScriptedGithub::with([Ok(invalid_items)]);
        assert!(reconcile_issue(
            &mut store,
            &mut invalid,
            &policy,
            "https://github.com/berlinguyinca/autospec/issues/45",
        )
        .is_err());
        assert!(store.snapshot().pending_projections.is_empty());
        assert!(invalid
            .calls
            .iter()
            .all(|call| !matches!(call, GithubCommand::AddToProject { .. })));
    }
}

#[test]
fn github_issue_urls_require_nonempty_identity_and_positive_canonical_number() {
    let fixture = Fixture::new("github-canonical-url");
    let policy = policy("berlinguyinca");
    let mut store = ManagedProjectStore::open(fixture.path(), &key("autospec")).unwrap();
    store
        .record_project(
            "berlinguyinca",
            "PVT_7",
            7,
            "https://github.com/orgs/berlinguyinca/projects/7",
            "Autospec",
        )
        .unwrap();

    for invalid in [
        "https://github.com///issues/1",
        "https://github.com/owner//issues/1",
        "https://github.com/owner/repo/issues/0",
    ] {
        let mut github = ScriptedGithub::default();
        assert!(reconcile_issue(&mut store, &mut github, &policy, invalid).is_err());
        assert!(github.calls.is_empty());
    }

    let mut github = ScriptedGithub::with([Ok(item_list(&[])), Ok(String::new())]);
    reconcile_issue(
        &mut store,
        &mut github,
        &policy,
        "HTTPS://GITHUB.COM/BerlinGuyInCA/AutoSpec/issues/00046/?tab=1",
    )
    .unwrap();
    let added = github.calls.iter().find_map(|call| match call {
        GithubCommand::AddToProject { issue_url, .. } => Some(issue_url.as_str()),
        _ => None,
    });
    assert_eq!(
        added,
        Some("https://github.com/berlinguyinca/autospec/issues/46")
    );
}

#[test]
fn store_reopens_repository_edge_and_pending_projection_from_journal() {
    let fixture = Fixture::new("reopen");
    let mut store = ManagedProjectStore::open(fixture.path(), &key("autospec")).unwrap();
    store
        .record_repository(repository("berlinguyinca/autospec", "explicit-seed"))
        .unwrap();
    store.record_edge(edge()).unwrap();
    store
        .enqueue_projection(add_item(
            "https://github.com/berlinguyinca/autospec/issues/42",
        ))
        .unwrap();
    drop(store);

    let reopened = ManagedProjectStore::open(fixture.path(), &key("autospec")).unwrap();
    assert_eq!(reopened.snapshot().repositories.len(), 1);
    assert_eq!(reopened.snapshot().relationships.len(), 1);
    assert_eq!(reopened.snapshot().pending_projections.len(), 1);
}

#[test]
fn store_duplicate_event_keys_are_no_ops() {
    let fixture = Fixture::new("dedupe");
    let mut store = ManagedProjectStore::open(fixture.path(), &key("autospec")).unwrap();
    let repository = repository("berlinguyinca/autospec", "explicit-seed");
    let edge = edge();
    let projection = add_item("https://github.com/berlinguyinca/autospec/issues/42");

    store.record_repository(repository.clone()).unwrap();
    store.record_repository(repository).unwrap();
    store.record_edge(edge.clone()).unwrap();
    store.record_edge(edge).unwrap();
    store.enqueue_projection(projection.clone()).unwrap();
    store.enqueue_projection(projection).unwrap();

    assert_eq!(store.snapshot().repositories.len(), 1);
    assert_eq!(store.snapshot().relationships.len(), 1);
    assert_eq!(store.snapshot().pending_projections.len(), 1);
    assert_eq!(
        fs::read_to_string(fixture.state_path("events.jsonl"))
            .unwrap()
            .lines()
            .count(),
        3
    );
}

#[test]
fn store_two_writers_refresh_under_lock_without_losing_events() {
    let fixture = Fixture::new("two-writers");
    let mut first = ManagedProjectStore::open(fixture.path(), &key("autospec")).unwrap();
    let mut second = ManagedProjectStore::open(fixture.path(), &key("autospec")).unwrap();

    first
        .record_repository(repository("berlinguyinca/autospec", "explicit-seed"))
        .unwrap();
    second
        .record_repository(repository("berlinguyinca/autospec-node", "explicit-seed"))
        .unwrap();
    drop((first, second));

    let reopened = ManagedProjectStore::open(fixture.path(), &key("autospec")).unwrap();
    assert_eq!(reopened.snapshot().repositories.len(), 2);
    assert_eq!(
        fs::read_to_string(fixture.state_path("events.jsonl"))
            .unwrap()
            .lines()
            .count(),
        2
    );
}

#[test]
fn store_partial_append_failure_rolls_back_before_same_instance_retry() {
    let fixture = Fixture::new("append-rollback");
    let mut store = ManagedProjectStore::open(fixture.path(), &key("autospec")).unwrap();
    store
        .record_repository(repository("berlinguyinca/autospec", "explicit-seed"))
        .unwrap();
    let journal_path = fixture.state_path("events.jsonl");
    let length_before = fs::metadata(&journal_path).unwrap().len();
    store.fail_next_append_after(17);

    assert!(store
        .record_repository(repository("berlinguyinca/autospec-node", "explicit-seed"))
        .is_err());
    assert_eq!(fs::metadata(&journal_path).unwrap().len(), length_before);
    store
        .record_repository(repository("berlinguyinca/autospec-node", "explicit-seed"))
        .unwrap();
    drop(store);

    let reopened = ManagedProjectStore::open(fixture.path(), &key("autospec")).unwrap();
    assert_eq!(reopened.snapshot().repositories.len(), 2);
    assert_eq!(fs::read_to_string(journal_path).unwrap().lines().count(), 2);
}

#[test]
fn store_ack_projection_is_retryable_but_unknown_keys_fail_closed() {
    let fixture = Fixture::new("ack");
    let mut store = ManagedProjectStore::open(fixture.path(), &key("autospec")).unwrap();
    let projection = add_item("https://github.com/berlinguyinca/autospec/issues/42");
    store.enqueue_projection(projection.clone()).unwrap();

    store.ack_projection(&projection).unwrap();
    store.ack_projection(&projection).unwrap();
    assert!(store
        .ack_projection("project:item-add:PV_123:missing")
        .is_err());
    assert!(store.snapshot().pending_projections.is_empty());
}

#[test]
fn store_discards_only_a_truncated_jsonl_tail_and_rebuilds_the_snapshot() {
    let fixture = Fixture::new("truncated-tail");
    let mut store = ManagedProjectStore::open(fixture.path(), &key("autospec")).unwrap();
    store
        .record_repository(repository("berlinguyinca/autospec", "explicit-seed"))
        .unwrap();
    drop(store);
    fs::OpenOptions::new()
        .append(true)
        .open(fixture.state_path("events.jsonl"))
        .unwrap()
        .write_all(br#"{"sequence":2,"kind":"projection-enqueued""#)
        .unwrap();
    fs::remove_file(fixture.state_path("binding.json")).unwrap();

    let reopened = ManagedProjectStore::open(fixture.path(), &key("autospec")).unwrap();
    assert_eq!(reopened.snapshot().repositories.len(), 1);
    assert!(fs::read_to_string(fixture.state_path("events.jsonl"))
        .unwrap()
        .ends_with('\n'));
    assert!(fixture.state_path("binding.json").is_file());
}

#[test]
fn store_rejects_empty_interior_journal_lines() {
    let fixture = Fixture::new("empty-journal-line");
    let mut store = ManagedProjectStore::open(fixture.path(), &key("autospec")).unwrap();
    store
        .record_repository(repository("berlinguyinca/autospec", "explicit-seed"))
        .unwrap();
    drop(store);
    fs::OpenOptions::new()
        .append(true)
        .open(fixture.state_path("events.jsonl"))
        .unwrap()
        .write_all(b"\n")
        .unwrap();

    assert!(ManagedProjectStore::open(fixture.path(), &key("autospec")).is_err());
}

#[test]
fn store_rebuilds_a_stale_snapshot_from_the_journal() {
    let fixture = Fixture::new("stale-snapshot");
    let mut store = ManagedProjectStore::open(fixture.path(), &key("autospec")).unwrap();
    store
        .record_repository(repository("berlinguyinca/autospec", "explicit-seed"))
        .unwrap();
    drop(store);
    let binding_path = fixture.state_path("binding.json");
    let mut binding: serde_json::Value =
        serde_json::from_slice(&fs::read(&binding_path).unwrap()).unwrap();
    binding["repositories"] = serde_json::json!([]);
    fs::write(&binding_path, serde_json::to_vec(&binding).unwrap()).unwrap();
    #[cfg(unix)]
    fs::set_permissions(&binding_path, fs::Permissions::from_mode(0o600)).unwrap();

    let reopened = ManagedProjectStore::open(fixture.path(), &key("autospec")).unwrap();
    assert_eq!(reopened.snapshot().repositories.len(), 1);
}

#[test]
fn store_missing_journal_fails_closed_without_overwriting_a_nonempty_binding() {
    let fixture = Fixture::new("missing-journal");
    let mut store = ManagedProjectStore::open(fixture.path(), &key("autospec")).unwrap();
    store
        .record_repository(repository("berlinguyinca/autospec", "explicit-seed"))
        .unwrap();
    drop(store);
    let binding_path = fixture.state_path("binding.json");
    let binding_before = fs::read(&binding_path).unwrap();
    fs::remove_file(fixture.state_path("events.jsonl")).unwrap();

    assert!(ManagedProjectStore::open(fixture.path(), &key("autospec")).is_err());
    assert_eq!(fs::read(binding_path).unwrap(), binding_before);
    assert!(!fixture.state_path("events.jsonl").exists());
}

#[test]
fn store_zero_length_journal_fails_closed_without_modifying_state() {
    let fixture = Fixture::new("empty-journal");
    let mut store = ManagedProjectStore::open(fixture.path(), &key("autospec")).unwrap();
    store
        .record_repository(repository("berlinguyinca/autospec", "explicit-seed"))
        .unwrap();
    drop(store);
    let binding_path = fixture.state_path("binding.json");
    let journal_path = fixture.state_path("events.jsonl");
    let binding_before = fs::read(&binding_path).unwrap();
    fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(&journal_path)
        .unwrap();

    assert!(ManagedProjectStore::open(fixture.path(), &key("autospec")).is_err());
    assert_eq!(fs::read(binding_path).unwrap(), binding_before);
    assert!(fs::read(journal_path).unwrap().is_empty());
}

#[test]
fn store_valid_journal_prefix_behind_snapshot_fails_closed_without_modifying_state() {
    let fixture = Fixture::new("journal-prefix");
    let mut store = ManagedProjectStore::open(fixture.path(), &key("autospec")).unwrap();
    store
        .record_repository(repository("berlinguyinca/autospec", "explicit-seed"))
        .unwrap();
    store
        .record_repository(repository("berlinguyinca/autospec-node", "explicit-seed"))
        .unwrap();
    drop(store);
    let binding_path = fixture.state_path("binding.json");
    let journal_path = fixture.state_path("events.jsonl");
    let binding_before = fs::read(&binding_path).unwrap();
    let journal = fs::read_to_string(&journal_path).unwrap();
    let first_line = format!("{}\n", journal.lines().next().unwrap());
    fs::write(&journal_path, first_line.as_bytes()).unwrap();
    #[cfg(unix)]
    fs::set_permissions(&journal_path, fs::Permissions::from_mode(0o600)).unwrap();

    assert!(ManagedProjectStore::open(fixture.path(), &key("autospec")).is_err());
    assert_eq!(fs::read(binding_path).unwrap(), binding_before);
    assert_eq!(fs::read(journal_path).unwrap(), first_line.as_bytes());
}

#[test]
fn store_rejects_mismatched_binding_and_edge_product_keys() {
    let fixture = Fixture::new("mismatched-key");
    let store = ManagedProjectStore::open(fixture.path(), &key("autospec")).unwrap();
    drop(store);
    let binding_path = fixture.state_path("binding.json");
    let mut binding: serde_json::Value =
        serde_json::from_slice(&fs::read(&binding_path).unwrap()).unwrap();
    binding["product_key"] = serde_json::json!("other");
    fs::write(&binding_path, serde_json::to_vec(&binding).unwrap()).unwrap();
    #[cfg(unix)]
    fs::set_permissions(&binding_path, fs::Permissions::from_mode(0o600)).unwrap();
    assert!(ManagedProjectStore::open(fixture.path(), &key("autospec")).is_err());

    let other = Fixture::new("mismatched-edge");
    let mut store = ManagedProjectStore::open(other.path(), &key("autospec")).unwrap();
    let mut wrong_edge = edge();
    wrong_edge.product_key = key("other");
    assert!(store.record_edge(wrong_edge).is_err());
}

#[test]
#[cfg(unix)]
fn store_uses_private_state_and_rejects_public_binding_files() {
    let fixture = Fixture::new("private-state");
    let store = ManagedProjectStore::open(fixture.path(), &key("autospec")).unwrap();
    drop(store);
    let project_dir = fixture
        .state_path("binding.json")
        .parent()
        .unwrap()
        .to_path_buf();
    assert_eq!(
        fs::metadata(&project_dir).unwrap().permissions().mode() & 0o777,
        0o700
    );
    for name in ["binding.json", "events.jsonl", "binding.lock"] {
        assert_eq!(
            fs::metadata(fixture.state_path(name))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }

    fs::set_permissions(
        fixture.state_path("binding.json"),
        fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    assert!(ManagedProjectStore::open(fixture.path(), &key("autospec")).is_err());
}

#[test]
#[cfg(unix)]
fn store_rejects_a_public_product_lock_file() {
    let fixture = Fixture::new("public-lock");
    let store = ManagedProjectStore::open(fixture.path(), &key("autospec")).unwrap();
    drop(store);
    fs::set_permissions(
        fixture.state_path("binding.lock"),
        fs::Permissions::from_mode(0o644),
    )
    .unwrap();

    assert!(ManagedProjectStore::open(fixture.path(), &key("autospec")).is_err());
}

#[test]
#[cfg(unix)]
fn store_rejects_symlinked_product_state_directories() {
    let fixture = Fixture::new("symlink-state");
    fs::create_dir(&fixture.0).unwrap();
    fs::set_permissions(&fixture.0, fs::Permissions::from_mode(0o700)).unwrap();
    let projects = fixture.0.join("projects");
    fs::create_dir(&projects).unwrap();
    fs::set_permissions(&projects, fs::Permissions::from_mode(0o700)).unwrap();
    let outside = fixture.0.join("outside");
    fs::create_dir(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, projects.join("autospec")).unwrap();

    assert!(ManagedProjectStore::open(fixture.path(), &key("autospec")).is_err());
}
