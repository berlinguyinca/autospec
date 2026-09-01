#[path = "../src/commands/mod.rs"]
mod commands;

use autospec_core::managed_project::{
    ItemKey, ManagedProjectIdentity, ManagedProjectNamespace, ManagedProjectPolicy, PortfolioId,
    SourceSpecIdentity, SpecPortfolioIdentity,
};
use autospec_core::managed_project::{
    ProductKey, RelationshipEdge, RelationshipEvidence, RelationshipKind, RelationshipState,
    RepositoryRecord,
};
use commands::autonomous::accountability::github::{GithubCommand, GithubFailure, GithubTransport};
use commands::managed_project::{
    active_dependency_graph, journal_issue_projection, onboard_repositories, reconcile_issue,
    resolve_or_create_project, retry_pending_projections, run_with_transport, tracked_issue_urls,
    verify_managed_marker, ManagedProjectStore, OnboardingOptions,
};
use std::collections::VecDeque;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
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

    fn portfolio_state_path(&self, name: &str) -> PathBuf {
        self.0
            .join("projects")
            .join(portfolio_store_identity().namespace().to_string())
            .join(name)
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

fn portfolio_store_snapshot() -> serde_json::Value {
    let source = portfolio_source_identity();
    let portfolio_id = source.portfolio_id().to_string();
    serde_json::json!({
        "schema": "autospec.portfolio-snapshot.v1",
        "portfolio_id": portfolio_id,
        "owner": "berlinguyinca",
        "project_number": 42,
        "project_node_id": "PVT_42",
        "project_url": "https://github.com/orgs/berlinguyinca/projects/42",
        "source_spec": source,
        "plan_digest": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "lease_generation": 3,
        "state": "applying",
        "projection_high_watermark": 0,
        "recovery_capsule": {
            "schema": "autospec.portfolio-recovery.v1",
            "portfolio_id": portfolio_id,
            "plan_digest": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "create_nonce": "00112233445566778899aabbccddeeff",
            "items": [
                {
                    "item_key": "source-tracker",
                    "repository": "berlinguyinca/autospec",
                    "role": "source-tracker",
                    "completion_policy": "closed-tracker",
                    "local_parents": [],
                    "dependencies": []
                },
                {
                    "item_key": "issue:portfolio-store",
                    "repository": "berlinguyinca/autospec",
                    "role": "implementation",
                    "completion_policy": "merged-pr",
                    "local_parents": ["source-tracker"],
                    "dependencies": ["source-tracker"]
                },
                {
                    "item_key": "audit:phase-5.5",
                    "repository": "berlinguyinca/autospec",
                    "role": "audit",
                    "completion_policy": "audit-receipt",
                    "local_parents": ["source-tracker"],
                    "dependencies": ["issue:portfolio-store"]
                }
            ]
        }
    })
}

fn portfolio_source_identity() -> SourceSpecIdentity {
    SourceSpecIdentity::new(
        "berlinguyinca/autospec",
        "docs/specs/automatic-projects.md",
        "0123456789abcdef0123456789abcdef01234567",
    )
    .unwrap()
}

fn portfolio_store_identity() -> ManagedProjectIdentity {
    ManagedProjectIdentity::SpecPortfolio(SpecPortfolioIdentity::new(portfolio_source_identity()))
}

fn open_portfolio_store(root: &Path) -> ManagedProjectStore {
    ManagedProjectStore::open(root, &portfolio_store_identity()).unwrap()
}

fn portfolio_item_binding(item_key: &str, issue_number: u64) -> serde_json::Value {
    let dependencies = if item_key == "source-tracker" {
        serde_json::json!([])
    } else {
        serde_json::json!(["source-tracker"])
    };
    serde_json::json!({
        "item_key": item_key,
        "repository": "berlinguyinca/autospec",
        "issue_url": format!("https://github.com/berlinguyinca/autospec/issues/{issue_number}"),
        "role": if item_key == "source-tracker" { "source-tracker" } else { "implementation" },
        "completion_policy": if item_key == "source-tracker" {
            "closed-tracker"
        } else {
            "merged-pr"
        },
        "dependencies": dependencies,
        "terminal_state": null
    })
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
        "<!-- autospec-managed-project:begin -->\nschema: 2\nkind: product\nproduct-key: autospec\nowner: {owner}\n<!-- autospec-managed-project:end -->"
    )
}

fn legacy_product_marker(owner: &str) -> String {
    format!(
        "<!-- autospec-managed-project:begin -->\nschema: 1\nproduct-key: autospec\nowner: {owner}\n<!-- autospec-managed-project:end -->"
    )
}

fn portfolio_marker(owner: &str) -> String {
    let identity = match portfolio_store_identity() {
        ManagedProjectIdentity::SpecPortfolio(identity) => identity,
        ManagedProjectIdentity::Product { .. } => unreachable!(),
    };
    let snapshot = portfolio_store_snapshot();
    let capsule = serde_json::to_string(&snapshot["recovery_capsule"]).unwrap();
    format!(
        "<!-- autospec-managed-project:begin -->\nschema: 2\nkind: spec_portfolio\nportfolio-id: {}\nsource: {}\nowner: {owner}\nrecovery-capsule: {capsule}\n<!-- autospec-managed-project:end -->",
        identity.portfolio_id(),
        identity.source(),
    )
}

#[test]
fn managed_project_portfolio_identity_is_stable_and_changes_with_source_blob() {
    let source = SourceSpecIdentity::new(
        "BerlinGuyInCA/AutoSpec",
        "docs/specs/automatic-projects.md",
        "0123456789abcdef0123456789abcdef01234567",
    )
    .unwrap();
    let same_source = SourceSpecIdentity::new(
        "berlinguyinca/autospec",
        "docs/specs/automatic-projects.md",
        "0123456789abcdef0123456789abcdef01234567",
    )
    .unwrap();
    let changed_source = SourceSpecIdentity::new(
        "berlinguyinca/autospec",
        "docs/specs/automatic-projects.md",
        "1123456789abcdef0123456789abcdef01234567",
    )
    .unwrap();

    assert_eq!(source.portfolio_id(), same_source.portfolio_id());
    assert_ne!(source.portfolio_id(), changed_source.portfolio_id());
    assert_eq!(
        source.portfolio_id().as_str(),
        "6d50c9f07ca522115a08e5d94f402135cc226619fc669ac3329d8eff186a10cd"
    );
}

#[test]
fn managed_project_portfolio_identity_preserves_component_boundaries() {
    let left = PortfolioId::from_source("a", "bc", "d").unwrap();
    let right = PortfolioId::from_source("ab", "c", "d").unwrap();

    assert_ne!(left, right);
}

#[test]
fn managed_project_binding_evolves_legacy_products_to_schema_two() {
    let legacy = serde_json::json!({
        "schema_version": 1,
        "product_key": "autospec",
        "owner": "berlinguyinca",
        "project_node_id": null,
        "project_number": null,
        "project_url": null,
        "project_title": null,
        "repositories": [],
        "last_reconciled_at": null,
        "pending_projections": [],
        "relationships": []
    });

    let binding: autospec_core::managed_project::ManagedProjectBinding =
        serde_json::from_value(legacy).unwrap();
    assert_eq!(binding.schema_version, 2);
    assert_eq!(
        binding.identity(),
        &ManagedProjectIdentity::Product {
            product_key: key("autospec")
        }
    );
    let serialized = serde_json::to_value(binding).unwrap();
    assert_eq!(serialized["schema_version"], 2);
    assert_eq!(serialized["identity"]["kind"], "product");
    assert!(serialized.get("product_key").is_none());
}

#[test]
fn managed_project_binding_rejects_incomplete_or_mixed_identity_schemas() {
    let legacy_spec = serde_json::json!({
        "schema_version": 1,
        "kind": "spec_portfolio",
        "product_key": "autospec"
    });
    let schema_two_legacy_shape = serde_json::json!({
        "schema_version": 2,
        "kind": "product",
        "product_key": "autospec"
    });
    let schema_two_missing_identity = serde_json::json!({"schema_version": 2});
    let mismatched_portfolio = serde_json::json!({
        "schema_version": 2,
        "identity": {
            "kind": "spec_portfolio",
            "portfolio_id": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "source": "berlinguyinca/autospec:docs/specs/automatic-projects.md@0123456789abcdef0123456789abcdef01234567"
        }
    });

    for malformed in [
        legacy_spec,
        schema_two_legacy_shape,
        schema_two_missing_identity,
        mismatched_portfolio,
    ] {
        assert!(
            serde_json::from_value::<autospec_core::managed_project::ManagedProjectBinding>(
                malformed
            )
            .is_err()
        );
    }
}

#[test]
fn managed_project_binding_round_trips_closed_spec_portfolio_identity() {
    let source = SourceSpecIdentity::new(
        "berlinguyinca/autospec",
        "docs/specs/automatic-projects.md",
        "0123456789abcdef0123456789abcdef01234567",
    )
    .unwrap();
    let identity = ManagedProjectIdentity::SpecPortfolio(SpecPortfolioIdentity::new(source));
    let binding =
        autospec_core::managed_project::ManagedProjectBinding::new_identity(identity).unwrap();

    let encoded = serde_json::to_value(&binding).unwrap();
    assert_eq!(encoded["identity"]["kind"], "spec_portfolio");
    assert_eq!(
        encoded["identity"]["portfolio_id"],
        "6d50c9f07ca522115a08e5d94f402135cc226619fc669ac3329d8eff186a10cd"
    );
    assert!(encoded.get("product_key").is_none());
    let decoded = serde_json::from_value(encoded).unwrap();
    assert_eq!(binding, decoded);
}

#[test]
fn managed_project_namespaces_are_collision_safe_and_round_trip() {
    let product = ManagedProjectIdentity::Product {
        product_key: key("repo.owner__name"),
    }
    .namespace();
    let source = SourceSpecIdentity::new(
        "berlinguyinca/autospec",
        "docs/specs/automatic-projects.md",
        "0123456789abcdef0123456789abcdef01234567",
    )
    .unwrap();
    let portfolio =
        ManagedProjectIdentity::SpecPortfolio(SpecPortfolioIdentity::new(source)).namespace();

    assert_eq!(product.to_string(), "product.repo.owner__name");
    assert_eq!(
        portfolio.to_string(),
        "portfolio.6d50c9f07ca522115a08e5d94f402135cc226619fc669ac3329d8eff186a10cd"
    );

    for namespace in [&product, &portfolio] {
        let encoded = namespace.to_string();
        let decoded: ManagedProjectNamespace = encoded.parse().unwrap();
        assert_eq!(&decoded, namespace);
        assert_eq!(decoded.to_string(), encoded);
    }
    assert_ne!(product.to_string(), portfolio.to_string());
}

#[test]
fn managed_project_item_keys_preserve_stable_logical_identity() {
    for value in [
        "source-tracker",
        "repo:berlinguyinca/autospec:tracker",
        "issue:portfolio-identity",
        "audit:phase-5.5",
    ] {
        let key = ItemKey::new(value).unwrap();
        assert_eq!(key.as_str(), value);
        assert_eq!(key.to_string(), value);
        assert_eq!(value.parse::<ItemKey>().unwrap(), key);
        assert_eq!(
            serde_json::from_value::<ItemKey>(serde_json::json!(value)).unwrap(),
            key
        );
    }
}

#[test]
fn managed_project_item_key_serialization_has_a_durable_golden() {
    let item_key = ItemKey::new("repo:berlinguyinca/autospec:tracker").unwrap();

    assert_eq!(
        serde_json::to_string(&item_key).unwrap(),
        "\"repo:berlinguyinca/autospec:tracker\""
    );
}

#[test]
fn managed_project_spec_portfolio_identity_derives_id_and_rejects_mismatched_serde() {
    let source = SourceSpecIdentity::new(
        "berlinguyinca/autospec",
        "docs/specs/automatic-projects.md",
        "0123456789abcdef0123456789abcdef01234567",
    )
    .unwrap();
    let identity = SpecPortfolioIdentity::new(source.clone());
    assert_eq!(identity.portfolio_id(), &source.portfolio_id());
    assert_eq!(identity.source(), &source);

    let mismatched = serde_json::json!({
        "portfolio_id": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "source": source.to_string()
    });
    assert!(serde_json::from_value::<SpecPortfolioIdentity>(mismatched).is_err());
}

#[test]
#[should_panic(expected = "spec portfolio bindings do not expose product compatibility")]
fn managed_project_spec_portfolio_binding_hides_product_compatibility() {
    let source = SourceSpecIdentity::new(
        "berlinguyinca/autospec",
        "docs/specs/automatic-projects.md",
        "0123456789abcdef0123456789abcdef01234567",
    )
    .unwrap();
    let binding = autospec_core::managed_project::ManagedProjectBinding::new_identity(
        ManagedProjectIdentity::SpecPortfolio(SpecPortfolioIdentity::new(source)),
    )
    .unwrap();

    let _ = binding.product_key.as_str();
}

#[test]
fn managed_project_source_spec_identity_round_trips_as_a_validated_scalar() {
    let identity = SourceSpecIdentity::new(
        "BerlinGuyInCA/AutoSpec",
        "docs/specs/automatic-projects.md",
        "0123456789ABCDEF0123456789ABCDEF01234567",
    )
    .unwrap();
    let encoded = identity.to_string();

    assert_eq!(
        encoded,
        "berlinguyinca/autospec:docs/specs/automatic-projects.md@0123456789abcdef0123456789abcdef01234567"
    );
    assert_eq!(encoded.parse::<SourceSpecIdentity>().unwrap(), identity);
    assert_eq!(
        serde_json::from_value::<SourceSpecIdentity>(serde_json::json!(encoded)).unwrap(),
        identity
    );
}

#[test]
fn managed_project_identity_types_reject_unsafe_keys() {
    for unsafe_namespace in [
        "product.../autospec",
        "portfolio.aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\\",
        "C:portfolio.aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    ] {
        assert!(unsafe_namespace.parse::<ManagedProjectNamespace>().is_err());
    }
    for unsafe_item_key in [
        "../source-tracker",
        "repo:owner\\repo:tracker",
        "issue:nul\0key",
    ] {
        assert!(ItemKey::new(unsafe_item_key).is_err());
    }
}

fn initialize_repository(path: &Path, remote: &str) {
    fs::create_dir_all(path).unwrap();
    assert!(Command::new("git")
        .args(["init", "-q"])
        .current_dir(path)
        .status()
        .unwrap()
        .success());
    assert!(Command::new("git")
        .args(["remote", "add", "origin", remote])
        .current_dir(path)
        .status()
        .unwrap()
        .success());
}

fn initialize_managed_repository(fixture: &Fixture, name: &str) -> PathBuf {
    let repository_path = fixture.path().join(name);
    initialize_repository(
        &repository_path,
        "https://github.com/berlinguyinca/autospec.git",
    );
    fs::create_dir_all(repository_path.join(".autospec")).unwrap();
    fs::write(
        repository_path.join(".autospec/autonomous.yml"),
        "project_board:\n  mode: managed\n  product_key: autospec\n  owner: berlinguyinca\n  repo_allowlist: [\"berlinguyinca/*\"]\n  repository_seeds: [\"berlinguyinca/autospec\"]\n  discovery_max_repos: 25\n",
    )
    .unwrap();
    repository_path
}

fn run_real_onboard(repository_path: &Path, gh_program: &Path) -> std::process::Output {
    let autospec_home = repository_path.join("test-autospec-home");
    fs::create_dir_all(&autospec_home).unwrap();
    #[cfg(unix)]
    fs::set_permissions(&autospec_home, fs::Permissions::from_mode(0o700)).unwrap();
    Command::new(env!("CARGO_BIN_EXE_autospec"))
        .args([
            "project",
            "onboard",
            "--repo-dir",
            repository_path.to_str().unwrap(),
            "--repo",
            "berlinguyinca/created",
            "--spawned-from",
            "spec:managed-project-onboarding",
        ])
        .env("AUTOSPEC_HOME", autospec_home)
        .env("AUTOSPEC_GH_PROGRAM", gh_program)
        .output()
        .unwrap()
}

#[test]
fn onboard_admits_explicit_and_allowlisted_evidence_without_executing_manifests() {
    let fixture = Fixture::new("onboard-evidence");
    let repository_path = fixture.path().join("checkout");
    initialize_repository(
        &repository_path,
        "git@github.com:berlinguyinca/autospec.git",
    );
    fs::write(
        repository_path.join(".gitmodules"),
        "[submodule \"node\"]\n  path = node\n  url = https://github.com/berlinguyinca/autospec-node.git\n",
    )
    .unwrap();
    fs::write(
        repository_path.join("Cargo.toml"),
        "[workspace]\nmembers = [\n  \"member\",\n]\n[dependencies]\nallowed = { git = \"ssh://git@github.com/berlinguyinca/autospec-tools.git\" }\noutside = { git = \"https://github.com/other/private.git\" }\n",
    )
    .unwrap();
    let member_path = repository_path.join("member");
    initialize_repository(
        &member_path,
        "https://github.com/berlinguyinca/autospec-member.git",
    );
    fs::write(
        member_path.join("package.json"),
        r#"{"repository":"https://github.com/berlinguyinca/autospec-member-dep"}"#,
    )
    .unwrap();
    fs::write(
        repository_path.join("package.json"),
        r#"{"workspaces":["packages/*"],"dependencies":{"web":"github:berlinguyinca/autospec-web"}}"#,
    )
    .unwrap();
    fs::write(
        repository_path.join("go.mod"),
        "module github.com/berlinguyinca/autospec\nreplace example.invalid/tool => github.com/berlinguyinca/autospec-go v1.0.0\n",
    )
    .unwrap();
    fs::write(
        repository_path.join("autospec-fleet.yml"),
        "repositories:\n  - https://github.com/berlinguyinca/autospec-fleet-worker\n",
    )
    .unwrap();
    fs::create_dir_all(repository_path.join(".autospec/issues")).unwrap();
    fs::write(
        repository_path.join(".autospec/issues/42.md"),
        "## Autospec relationships\nSource spec: https://github.com/berlinguyinca/autospec-spec/issues/7\nTracker: berlinguyinca/autospec-tracker#9\nDepends on https://github.com/berlinguyinca/autospec-node/pull/3\n",
    )
    .unwrap();
    let mut policy = policy("berlinguyinca");
    policy.repo_allowlist = vec!["berlinguyinca/autospec*".to_owned()];
    let state = fixture.path().join("state");
    let mut store = ManagedProjectStore::open_product(&state, &key("autospec")).unwrap();

    let report = onboard_repositories(
        &mut store,
        &policy,
        &OnboardingOptions {
            repo_dir: repository_path.clone(),
            repositories: vec!["https://github.com/berlinguyinca/autospec".to_owned()],
            workspaces: vec![repository_path],
            dry_run: false,
        },
    )
    .unwrap();

    let repositories = store
        .snapshot()
        .repositories
        .iter()
        .map(|record| record.repository.as_str())
        .collect::<Vec<_>>();
    assert_eq!(repositories.first(), Some(&"berlinguyinca/autospec"));
    for expected in [
        "berlinguyinca/autospec-node",
        "berlinguyinca/autospec-tools",
        "berlinguyinca/autospec-web",
        "berlinguyinca/autospec-go",
        "berlinguyinca/autospec-fleet-worker",
        "berlinguyinca/autospec-spec",
        "berlinguyinca/autospec-tracker",
        "berlinguyinca/autospec-member",
        "berlinguyinca/autospec-member-dep",
    ] {
        assert!(repositories.contains(&expected), "missing {expected}");
    }
    assert_eq!(report.out_of_bound, 1);
    assert!(store.snapshot().relationships.iter().all(|edge| {
        edge.state == RelationshipState::Active
            && matches!(
                edge.evidence.kind.as_str(),
                "submodule"
                    | "manifest-dependency"
                    | "fleet"
                    | "issue-reference"
                    | "source-spec"
                    | "tracker"
            )
    }));
    let issue_edges = store
        .snapshot()
        .relationships
        .iter()
        .filter(|edge| edge.evidence.kind == "issue-reference")
        .collect::<Vec<_>>();
    assert_eq!(issue_edges.len(), 1);
    assert_eq!(issue_edges[0].kind, RelationshipKind::DependsOn);
    assert_eq!(
        issue_edges[0].target,
        "https://github.com/berlinguyinca/autospec-node/pull/3"
    );

    let repository_count = store.snapshot().repositories.len();
    let edge_count = store.snapshot().relationships.len();
    let repeated = onboard_repositories(
        &mut store,
        &policy,
        &OnboardingOptions {
            repo_dir: fixture.path().join("checkout"),
            repositories: vec!["berlinguyinca/autospec".to_owned()],
            workspaces: Vec::new(),
            dry_run: false,
        },
    )
    .unwrap();
    assert_eq!(store.snapshot().repositories.len(), repository_count);
    assert_eq!(store.snapshot().relationships.len(), edge_count);
    assert!(repeated.unchanged > 0);
}

#[test]
fn onboard_issue_scanner_keeps_blocks_kind_and_canonical_issue_identity() {
    let fixture = Fixture::new("onboard-blocks-identity");
    let repository_path = fixture.path().join("checkout");
    initialize_repository(
        &repository_path,
        "https://github.com/berlinguyinca/autospec.git",
    );
    fs::create_dir_all(repository_path.join(".autospec/issues")).unwrap();
    fs::write(
        repository_path.join(".autospec/issues/42.md"),
        "## Autospec relationships\nBlocks HTTPS://GITHUB.COM/BerlinGuyInCA/AutoSpec-Node/issues/0007/?x=1\n",
    )
    .unwrap();
    let mut policy = policy("berlinguyinca");
    policy.repo_allowlist = vec!["berlinguyinca/*".to_owned()];
    let mut store =
        ManagedProjectStore::open_product(&fixture.path().join("state"), &key("autospec")).unwrap();

    onboard_repositories(
        &mut store,
        &policy,
        &OnboardingOptions {
            repo_dir: repository_path,
            repositories: Vec::new(),
            workspaces: Vec::new(),
            dry_run: false,
        },
    )
    .unwrap();

    let edge = store
        .snapshot()
        .relationships
        .iter()
        .find(|edge| edge.evidence.kind == "issue-reference")
        .unwrap();
    assert_eq!(edge.kind, RelationshipKind::Blocks);
    assert_eq!(
        edge.source,
        "https://github.com/berlinguyinca/autospec/issues/42"
    );
    assert_eq!(
        edge.target,
        "https://github.com/berlinguyinca/autospec-node/issues/7"
    );
}

#[test]
fn onboard_issue_scanner_keeps_same_repository_issue_edges() {
    let fixture = Fixture::new("onboard-same-repo-issue-edge");
    let repository_path = fixture.path().join("checkout");
    initialize_repository(
        &repository_path,
        "https://github.com/berlinguyinca/autospec.git",
    );
    fs::create_dir_all(repository_path.join(".autospec/issues")).unwrap();
    fs::write(
        repository_path.join(".autospec/issues/42.md"),
        "## Autospec relationships\nDepends on berlinguyinca/autospec#7\n",
    )
    .unwrap();
    let mut store =
        ManagedProjectStore::open_product(&fixture.path().join("state"), &key("autospec")).unwrap();

    onboard_repositories(
        &mut store,
        &policy("berlinguyinca"),
        &OnboardingOptions {
            repo_dir: repository_path,
            repositories: Vec::new(),
            workspaces: Vec::new(),
            dry_run: false,
        },
    )
    .unwrap();

    let edge = store.snapshot().relationships.first().unwrap();
    assert_eq!(edge.kind, RelationshipKind::DependsOn);
    assert_eq!(
        edge.source,
        "https://github.com/berlinguyinca/autospec/issues/42"
    );
    assert_eq!(
        edge.target,
        "https://github.com/berlinguyinca/autospec/issues/7"
    );
}

#[test]
fn onboard_bounds_queue_and_excludes_proposed_edges_from_active_graph() {
    let fixture = Fixture::new("onboard-bounds");
    let repository_path = fixture.path().join("checkout");
    initialize_repository(
        &repository_path,
        "https://github.com/berlinguyinca/autospec.git",
    );
    fs::write(
        repository_path.join(".gitmodules"),
        "[submodule \"one\"]\nurl = https://github.com/berlinguyinca/one\n[submodule \"two\"]\nurl = https://github.com/berlinguyinca/two\n",
    )
    .unwrap();
    fs::create_dir_all(repository_path.join(".autospec/issues")).unwrap();
    fs::write(
        repository_path.join(".autospec/issues/ambiguous.md"),
        "## Autospec relationships\nThe autospec-node repository is related.\n",
    )
    .unwrap();
    let mut policy = policy("berlinguyinca");
    policy.repo_allowlist = vec!["berlinguyinca/*".to_owned()];
    policy.discovery_max_repos = 1;
    let mut store =
        ManagedProjectStore::open_product(&fixture.path().join("state"), &key("autospec")).unwrap();

    let report = onboard_repositories(
        &mut store,
        &policy,
        &OnboardingOptions {
            repo_dir: repository_path,
            repositories: Vec::new(),
            workspaces: Vec::new(),
            dry_run: false,
        },
    )
    .unwrap();

    assert_eq!(store.snapshot().repositories.len(), 2);
    let admitted = store
        .snapshot()
        .repositories
        .iter()
        .map(|record| record.repository.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    assert!(store
        .snapshot()
        .relationships
        .iter()
        .filter(|edge| edge.state == RelationshipState::Active)
        .all(|edge| admitted.contains(edge.source.as_str())
            && commands::managed_project::normalize_github_repository(&edge.target)
                .is_some_and(|repository| admitted.contains(repository.as_str()))));
    assert_eq!(report.proposed, 0);
    assert!(active_dependency_graph(store.snapshot())
        .iter()
        .all(|edge| edge.state == RelationshipState::Active));
}

#[test]
fn onboard_applies_discovery_cap_only_to_expansion_not_explicit_seeds() {
    let fixture = Fixture::new("onboard-expansion-cap");
    let repository_path = fixture.path().join("checkout");
    initialize_repository(
        &repository_path,
        "https://github.com/berlinguyinca/autospec.git",
    );
    fs::write(
        repository_path.join(".gitmodules"),
        "[submodule \"one\"]\nurl = https://github.com/berlinguyinca/discovered-one\n[submodule \"two\"]\nurl = https://github.com/berlinguyinca/discovered-two\n",
    )
    .unwrap();
    let mut policy = policy("berlinguyinca");
    policy.repository_seeds = vec![
        "berlinguyinca/autospec".to_owned(),
        "berlinguyinca/explicit-one".to_owned(),
        "berlinguyinca/explicit-two".to_owned(),
    ];
    policy.repo_allowlist = vec!["berlinguyinca/*".to_owned()];
    policy.discovery_max_repos = 1;
    let mut store =
        ManagedProjectStore::open_product(&fixture.path().join("state"), &key("autospec")).unwrap();

    let report = onboard_repositories(
        &mut store,
        &policy,
        &OnboardingOptions {
            repo_dir: repository_path,
            repositories: Vec::new(),
            workspaces: Vec::new(),
            dry_run: false,
        },
    )
    .unwrap();

    let repositories = report
        .repositories
        .iter()
        .map(|record| record.repository.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    for explicit in [
        "berlinguyinca/autospec",
        "berlinguyinca/explicit-one",
        "berlinguyinca/explicit-two",
    ] {
        assert!(repositories.contains(explicit), "missing {explicit}");
    }
    assert_eq!(
        report
            .repositories
            .iter()
            .filter(|record| record.entry_kind == "submodule")
            .count(),
        1
    );
    assert_eq!(report.repositories.len(), 4);
    assert_eq!(report.edges.len(), 1);
}

#[test]
fn onboard_retains_under_cap_name_reference_as_proposed_only() {
    let fixture = Fixture::new("onboard-proposed");
    let repository_path = fixture.path().join("checkout");
    initialize_repository(
        &repository_path,
        "https://github.com/berlinguyinca/autospec.git",
    );
    fs::create_dir_all(repository_path.join(".autospec/issues")).unwrap();
    fs::write(
        repository_path.join(".autospec/issues/ambiguous.md"),
        "## Autospec relationships\nThe autospec-node repository is related.\n",
    )
    .unwrap();
    let mut policy = policy("berlinguyinca");
    policy.repo_allowlist = vec!["berlinguyinca/*".to_owned()];
    policy.discovery_max_repos = 1;
    let mut store =
        ManagedProjectStore::open_product(&fixture.path().join("state"), &key("autospec")).unwrap();

    let report = onboard_repositories(
        &mut store,
        &policy,
        &OnboardingOptions {
            repo_dir: repository_path,
            repositories: Vec::new(),
            workspaces: Vec::new(),
            dry_run: false,
        },
    )
    .unwrap();

    assert_eq!(report.proposed, 1);
    let proposed = report
        .edges
        .iter()
        .find(|edge| edge.target == "berlinguyinca/autospec-node")
        .unwrap();
    assert_eq!(proposed.state, RelationshipState::Proposed);
    assert!(!active_dependency_graph(store.snapshot())
        .iter()
        .any(|edge| edge.dedupe_key() == proposed.dedupe_key()));
}

#[test]
fn onboard_cli_dry_run_emits_stable_sorted_json() {
    let fixture = Fixture::new("onboard-cli");
    let repository_path = fixture.path().join("checkout");
    initialize_repository(
        &repository_path,
        "https://github.com/berlinguyinca/autospec.git",
    );
    fs::create_dir_all(repository_path.join(".autospec")).unwrap();
    fs::write(
        repository_path.join(".autospec/autonomous.yml"),
        "project_board:\n  mode: managed\n  product_key: autospec\n  owner: berlinguyinca\n  repo_allowlist: [\"berlinguyinca/autospec*\"]\n  repository_seeds: [\"berlinguyinca/autospec\"]\n  discovery_max_repos: 10\n",
    )
    .unwrap();
    fs::write(
        repository_path.join(".gitmodules"),
        "[submodule \"z\"]\nurl = https://github.com/berlinguyinca/zeta\n[submodule \"a\"]\nurl = https://github.com/berlinguyinca/alpha\n",
    )
    .unwrap();
    let state_root = repository_path.join(".autospec/state");
    let mut persisted = ManagedProjectStore::open_product(&state_root, &key("autospec")).unwrap();
    persisted
        .record_repository(repository("berlinguyinca/autospec", "explicit-seed"))
        .unwrap();
    persisted
        .enqueue_projection(
            "project:item-add:PVT_1:https://github.com/berlinguyinca/autospec/issues/1",
        )
        .unwrap();
    drop(persisted);
    let binding_path = state_root.join("projects/autospec/binding.json");
    let journal_path = state_root.join("projects/autospec/events.jsonl");
    let binding_before = fs::read(&binding_path).unwrap();
    let journal_before = fs::read(&journal_path).unwrap();

    let run = || {
        Command::new(env!("CARGO_BIN_EXE_autospec"))
            .args([
                "project",
                "onboard",
                "--repo-dir",
                repository_path.to_str().unwrap(),
                "--dry-run",
            ])
            .env("AUTOSPEC_HOME", &state_root)
            .output()
            .unwrap()
    };
    let first = run();
    let second = run();

    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert_eq!(first.stdout, second.stdout);
    assert_eq!(fs::read(binding_path).unwrap(), binding_before);
    assert_eq!(fs::read(journal_path).unwrap(), journal_before);
    let report: serde_json::Value = serde_json::from_slice(&first.stdout).unwrap();
    assert_eq!(report["adopted"], 0);
    assert_eq!(report["created"], 0);
    assert_eq!(report["unchanged"], 1);
    assert_eq!(report["pending_projection"], 1);
    assert_eq!(
        report["repositories"][0]["repository"],
        "berlinguyinca/autospec"
    );
    assert!(report["edges"].as_array().unwrap().windows(2).all(|pair| {
        pair[0]["target"].as_str().unwrap() <= pair[1]["target"].as_str().unwrap()
    }));
}

#[test]
fn onboard_cli_journals_and_reconciles_every_selected_open_or_closed_issue() {
    let fixture = Fixture::new("onboard-selected-issues");
    let repository_path = initialize_managed_repository(&fixture, "checkout");
    let state_root = repository_path.join(".autospec/state");
    let mut store = ManagedProjectStore::open_product(&state_root, &key("autospec")).unwrap();
    store
        .record_project(
            "berlinguyinca",
            "PVT_7",
            7,
            "https://github.com/orgs/berlinguyinca/projects/7",
            "Autospec",
        )
        .unwrap();
    drop(store);
    let args = vec![
        "onboard".to_owned(),
        "--repo-dir".to_owned(),
        repository_path.display().to_string(),
        "--issue-url".to_owned(),
        "https://github.com/berlinguyinca/autospec/issues/41".to_owned(),
        "--issue-url".to_owned(),
        "https://github.com/berlinguyinca/autospec/issues/42".to_owned(),
    ];
    let mut github = ScriptedGithub::with([
        Ok(issue(
            "https://github.com/berlinguyinca/autospec/issues/41",
            "## AutoSpec relationships\nDepends on: https://github.com/berlinguyinca/autospec-node/issues/9",
        )),
        Ok(issue(
            "https://github.com/berlinguyinca/autospec/issues/42",
            "No managed relationships.",
        )),
        Ok(project(
            7,
            "berlinguyinca",
            "Autospec",
            &marker("berlinguyinca"),
        )),
        Ok(item_list(&[])),
        Ok(String::new()),
        Ok(String::new()),
    ]);

    let outcome = run_with_transport(&args, &mut github).unwrap();

    assert_eq!(outcome["selected_issues"], 2);
    assert_eq!(outcome["reconciled_issues"], 2);
    assert_eq!(outcome["pending_projection"], 0);
    assert!(outcome["edges"].as_array().unwrap().iter().any(|edge| {
        edge["source"] == "https://github.com/berlinguyinca/autospec/issues/41"
            && edge["target"] == "https://github.com/berlinguyinca/autospec-node/issues/9"
    }));
    assert!(outcome["repositories"]
        .as_array()
        .unwrap()
        .iter()
        .any(|record| { record["repository"] == "berlinguyinca/autospec-node" }));
    assert_eq!(
        github
            .calls
            .iter()
            .filter(|call| matches!(call, GithubCommand::AddToProject { .. }))
            .count(),
        2
    );
    let reopened = ManagedProjectStore::open_product(&state_root, &key("autospec")).unwrap();
    assert!(reopened.snapshot().pending_projections.is_empty());
}

#[test]
fn sync_without_a_new_url_restores_every_durably_tracked_issue() {
    let fixture = Fixture::new("sync-restores-tracked-issue");
    let repository_path = initialize_managed_repository(&fixture, "checkout");
    let state_root = repository_path.join(".autospec/state");
    let issue_url = "https://github.com/berlinguyinca/autospec/issues/45";
    let second_issue_url = "https://github.com/berlinguyinca/autospec/issues/46";
    let mut store = ManagedProjectStore::open_product(&state_root, &key("autospec")).unwrap();
    store
        .record_project(
            "berlinguyinca",
            "PVT_7",
            7,
            "https://github.com/orgs/berlinguyinca/projects/7",
            "Autospec",
        )
        .unwrap();
    journal_issue_projection(&mut store, issue_url).unwrap();
    journal_issue_projection(&mut store, second_issue_url).unwrap();
    let first_bound = format!("project:item-add:PVT_7:{issue_url}");
    let second_bound = format!("project:item-add:PVT_7:{second_issue_url}");
    store.enqueue_projection(first_bound.clone()).unwrap();
    store.enqueue_projection(second_bound.clone()).unwrap();
    store
        .ack_projection(&format!("project:item-add:unresolved:{issue_url}"))
        .unwrap();
    store
        .ack_projection(&format!("project:item-add:unresolved:{second_issue_url}"))
        .unwrap();
    store.ack_projection(&first_bound).unwrap();
    store.ack_projection(&second_bound).unwrap();
    assert_eq!(
        tracked_issue_urls(&store),
        vec![issue_url.to_owned(), second_issue_url.to_owned()]
    );
    drop(store);
    let reopened = ManagedProjectStore::open_product(&state_root, &key("autospec")).unwrap();
    assert_eq!(
        tracked_issue_urls(&reopened),
        vec![issue_url.to_owned(), second_issue_url.to_owned()]
    );
    drop(reopened);
    let args = vec![
        "sync".to_owned(),
        "--repo-dir".to_owned(),
        repository_path.display().to_string(),
    ];
    let mut github = ScriptedGithub::with([
        Ok(project(
            7,
            "berlinguyinca",
            "Autospec",
            &marker("berlinguyinca"),
        )),
        Ok(item_list(&[])),
        Ok(String::new()),
        Ok(String::new()),
    ]);

    run_with_transport(&args, &mut github).unwrap();

    assert!(
        github.calls.iter().any(|call| matches!(
            call,
            GithubCommand::AddToProject { issue_url: added, .. } if added == issue_url
        )),
        "calls: {:?}",
        github.calls
    );
    assert!(github.calls.iter().any(|call| matches!(
        call,
        GithubCommand::AddToProject { issue_url: added, .. } if added == second_issue_url
    )));
    assert_eq!(
        github
            .calls
            .iter()
            .filter(|call| matches!(call, GithubCommand::ListProjectItems { .. }))
            .count(),
        1
    );
    let reopened = ManagedProjectStore::open_product(&state_root, &key("autospec")).unwrap();
    assert!(reopened.snapshot().pending_projections.is_empty());
}

#[test]
fn onboard_cli_journals_selected_issue_before_relationship_fetch_failure() {
    let fixture = Fixture::new("onboard-selected-issue-fetch-failure");
    let repository_path = initialize_managed_repository(&fixture, "checkout");
    let state_root = repository_path.join(".autospec/state");
    let mut store = ManagedProjectStore::open_product(&state_root, &key("autospec")).unwrap();
    store
        .record_project(
            "berlinguyinca",
            "PVT_7",
            7,
            "https://github.com/orgs/berlinguyinca/projects/7",
            "Autospec",
        )
        .unwrap();
    drop(store);
    let issue_url = "https://github.com/berlinguyinca/autospec/issues/43";
    let args = vec![
        "onboard".to_owned(),
        "--repo-dir".to_owned(),
        repository_path.display().to_string(),
        "--issue-url".to_owned(),
        issue_url.to_owned(),
    ];
    let mut github = ScriptedGithub::with([
        Err(GithubFailure::Definitive("missing issue scope".to_owned())),
        Ok(project(
            7,
            "berlinguyinca",
            "Autospec",
            &marker("berlinguyinca"),
        )),
        Ok(item_list(&[])),
        Ok(String::new()),
    ]);

    let outcome = run_with_transport(&args, &mut github).unwrap();
    assert_eq!(outcome["inaccessible"], 1);
    assert_eq!(outcome["reconciled_issues"], 1);

    let reopened = ManagedProjectStore::open_product(&state_root, &key("autospec")).unwrap();
    assert!(reopened.snapshot().pending_projections.is_empty());
}

#[test]
fn onboard_cli_journals_selected_issue_before_owner_enumeration_failure() {
    let fixture = Fixture::new("onboard-selected-issue-owner-failure");
    let repository_path = initialize_managed_repository(&fixture, "checkout");
    let issue_url = "https://github.com/berlinguyinca/autospec/issues/44";
    let args = vec![
        "onboard".to_owned(),
        "--repo-dir".to_owned(),
        repository_path.display().to_string(),
        "--owner".to_owned(),
        "berlinguyinca".to_owned(),
        "--allow".to_owned(),
        "berlinguyinca/*".to_owned(),
        "--issue-url".to_owned(),
        issue_url.to_owned(),
    ];
    let mut github = ScriptedGithub::with([Err(GithubFailure::Definitive(
        "missing repository scope".to_owned(),
    ))]);

    assert!(run_with_transport(&args, &mut github).is_err());

    let reopened = ManagedProjectStore::open_product(
        &repository_path.join(".autospec/state"),
        &key("autospec"),
    )
    .unwrap();
    assert_eq!(
        reopened.snapshot().pending_projections,
        [format!("project:item-add:unresolved:{issue_url}")]
    );
}

#[test]
fn selected_issue_discovery_shares_one_repository_cap_across_all_issues() {
    let fixture = Fixture::new("selected-issue-shared-cap");
    let repository_path = initialize_managed_repository(&fixture, "checkout");
    fs::write(
        repository_path.join(".autospec/autonomous.yml"),
        "project_board:\n  mode: managed\n  product_key: autospec\n  owner: berlinguyinca\n  repo_allowlist: [\"berlinguyinca/*\"]\n  repository_seeds: [\"https://github.com/BerlinGuyInCA/autospec.git\"]\n  discovery_max_repos: 1\n",
    )
    .unwrap();
    let args = vec![
        "onboard".to_owned(),
        "--repo-dir".to_owned(),
        repository_path.display().to_string(),
        "--issue-url".to_owned(),
        "https://github.com/berlinguyinca/autospec/issues/50".to_owned(),
        "--issue-url".to_owned(),
        "https://github.com/berlinguyinca/autospec/issues/51".to_owned(),
        "--dry-run".to_owned(),
    ];
    let mut github = ScriptedGithub::with([
        Ok(issue(
            "https://github.com/berlinguyinca/autospec/issues/50",
            "## AutoSpec relationships\nDepends on: https://github.com/berlinguyinca/autospec/issues/99",
        )),
        Ok(issue(
            "https://github.com/berlinguyinca/autospec/issues/51",
            "## AutoSpec relationships\nDepends on: https://github.com/berlinguyinca/zeta/issues/1",
        )),
    ]);

    let outcome = run_with_transport(&args, &mut github).unwrap();

    let repositories = outcome["repositories"].as_array().unwrap();
    assert_eq!(repositories.len(), 2);
    assert_eq!(
        repositories
            .iter()
            .filter(|record| record["entry_kind"] == "issue-reference")
            .count(),
        1
    );
    assert_eq!(outcome["out_of_bound"], 0);
}

#[test]
fn active_edges_returns_empty_for_an_external_project_board() {
    let fixture = Fixture::new("external-active-edges");
    let repository_path = fixture.path().join("checkout");
    initialize_repository(
        &repository_path,
        "https://github.com/berlinguyinca/autospec.git",
    );
    fs::create_dir_all(repository_path.join(".autospec")).unwrap();
    fs::write(
        repository_path.join(".autospec/autonomous.yml"),
        "project_board:\n  mode: external\n  url: https://github.com/orgs/berlinguyinca/projects/7\n  repo_allowlist: [\"berlinguyinca/*\"]\n",
    )
    .unwrap();
    let args = vec![
        "active-edges".to_owned(),
        "--repo-dir".to_owned(),
        repository_path.display().to_string(),
        "--board-url".to_owned(),
        "https://github.com/orgs/berlinguyinca/projects/7".to_owned(),
    ];
    let mut github = ScriptedGithub::default();

    let outcome = run_with_transport(&args, &mut github).unwrap();

    assert_eq!(outcome, serde_json::json!([]));
    assert!(github.calls.is_empty());
    assert!(!repository_path.join(".autospec/state").exists());
}

#[test]
fn active_edges_rejects_a_board_other_than_the_managed_binding() {
    let fixture = Fixture::new("active-edges-board-mismatch");
    let repository_path = initialize_managed_repository(&fixture, "checkout");
    let mut store = ManagedProjectStore::open_product(
        &repository_path.join(".autospec/state"),
        &key("autospec"),
    )
    .unwrap();
    store
        .record_project(
            "berlinguyinca",
            "PVT_7",
            7,
            "https://github.com/orgs/berlinguyinca/projects/7",
            "Autospec",
        )
        .unwrap();
    drop(store);
    let args = vec![
        "active-edges".to_owned(),
        "--repo-dir".to_owned(),
        repository_path.display().to_string(),
        "--board-url".to_owned(),
        "https://github.com/orgs/berlinguyinca/projects/8".to_owned(),
    ];
    let mut github = ScriptedGithub::default();

    let error = run_with_transport(&args, &mut github).unwrap_err();

    assert!(error.to_string().contains("does not match"));
    assert!(github.calls.is_empty());
}

#[test]
fn onboard_cli_rejects_selected_issue_outside_the_admitted_repository_boundary() {
    let fixture = Fixture::new("onboard-selected-issue-boundary");
    let repository_path = initialize_managed_repository(&fixture, "checkout");
    let args = vec![
        "onboard".to_owned(),
        "--repo-dir".to_owned(),
        repository_path.display().to_string(),
        "--issue-url".to_owned(),
        "https://github.com/other/private/issues/9".to_owned(),
    ];
    let mut github = ScriptedGithub::default();

    assert!(run_with_transport(&args, &mut github).is_err());
    assert!(github.calls.is_empty());
}

#[test]
fn sync_cli_journals_normalized_issue_before_project_resolution_failure() {
    let fixture = Fixture::new("sync-pre-resolution-journal");
    let repository_path = initialize_managed_repository(&fixture, "checkout");
    let args = vec![
        "sync".to_owned(),
        "--repo-dir".to_owned(),
        repository_path.display().to_string(),
        "--issue-url".to_owned(),
        "HTTPS://GITHUB.COM/BerlinGuyInCA/AutoSpec/issues/00051/?tab=1".to_owned(),
    ];
    let mut github = ScriptedGithub::with([Err(GithubFailure::Definitive(
        "missing project scope".to_owned(),
    ))]);

    assert!(run_with_transport(&args, &mut github).is_err());

    let reopened = ManagedProjectStore::open_product(
        &repository_path.join(".autospec/state"),
        &key("autospec"),
    )
    .unwrap();
    assert_eq!(
        reopened.snapshot().pending_projections,
        ["project:item-add:unresolved:https://github.com/berlinguyinca/autospec/issues/51"]
    );
}

#[test]
fn onboard_cli_bounds_owner_enumeration_and_filters_before_scanning() {
    let fixture = Fixture::new("onboard-owner");
    let repository_path = initialize_managed_repository(&fixture, "checkout");
    let args = vec![
        "onboard".to_owned(),
        "--repo-dir".to_owned(),
        repository_path.display().to_string(),
        "--owner".to_owned(),
        "berlinguyinca".to_owned(),
        "--allow".to_owned(),
        "berlinguyinca/kept-*".to_owned(),
        "--dry-run".to_owned(),
    ];
    let mut github = ScriptedGithub::with([Ok(
        r#"[{"nameWithOwner":"berlinguyinca/kept-one"},{"nameWithOwner":"berlinguyinca/rejected"},{"nameWithOwner":"other/kept-two"}]"#
            .to_owned(),
    )]);

    let outcome = run_with_transport(&args, &mut github).unwrap();

    assert_eq!(
        github.calls,
        vec![GithubCommand::ListOwnerRepositories {
            owner: "berlinguyinca".to_owned(),
            limit: 25,
        }]
    );
    let repositories = outcome["repositories"]
        .as_array()
        .unwrap()
        .iter()
        .map(|repository| repository["repository"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(repositories.contains(&"berlinguyinca/kept-one"));
    assert!(!repositories.contains(&"berlinguyinca/rejected"));
    assert!(!repositories.contains(&"other/kept-two"));
}

#[test]
fn onboard_cli_rejects_an_invalid_issue_before_owner_enumeration() {
    let fixture = Fixture::new("onboard-invalid-issue-before-owner");
    let repository_path = initialize_managed_repository(&fixture, "checkout");
    let args = vec![
        "onboard".to_owned(),
        "--repo-dir".to_owned(),
        repository_path.display().to_string(),
        "--owner".to_owned(),
        "berlinguyinca".to_owned(),
        "--allow".to_owned(),
        "berlinguyinca/*".to_owned(),
        "--issue-url".to_owned(),
        "https://github.com/other/out-of-bound/issues/7".to_owned(),
        "--dry-run".to_owned(),
    ];
    let mut github = ScriptedGithub::with([Ok("[]".to_owned())]);

    let error = run_with_transport(&args, &mut github).unwrap_err();

    assert!(
        error
            .to_string()
            .contains("outside the managed repository boundary"),
        "{error}"
    );
    assert!(github.calls.is_empty());
}

#[test]
fn onboard_cli_requires_an_allowlist_for_owner_enumeration() {
    let fixture = Fixture::new("onboard-owner-no-allow");
    let repository_path = initialize_managed_repository(&fixture, "checkout");
    let args = vec![
        "onboard".to_owned(),
        "--repo-dir".to_owned(),
        repository_path.display().to_string(),
        "--owner".to_owned(),
        "berlinguyinca".to_owned(),
    ];
    let mut github = ScriptedGithub::default();

    let error = run_with_transport(&args, &mut github).unwrap_err();

    assert!(error.to_string().contains("--owner requires --allow"));
    assert!(github.calls.is_empty());
    assert!(!repository_path.join(".autospec/state").exists());
}

#[test]
fn onboard_cli_rejects_owner_responses_above_the_requested_bound() {
    let fixture = Fixture::new("onboard-owner-oversized");
    let repository_path = initialize_managed_repository(&fixture, "checkout");
    fs::write(
        repository_path.join(".autospec/autonomous.yml"),
        "project_board:\n  mode: managed\n  product_key: autospec\n  owner: berlinguyinca\n  repo_allowlist: [\"berlinguyinca/*\"]\n  repository_seeds: [\"berlinguyinca/autospec\"]\n  discovery_max_repos: 2\n",
    )
    .unwrap();
    let args = vec![
        "onboard".to_owned(),
        "--repo-dir".to_owned(),
        repository_path.display().to_string(),
        "--owner".to_owned(),
        "berlinguyinca".to_owned(),
        "--allow".to_owned(),
        "berlinguyinca/*".to_owned(),
        "--dry-run".to_owned(),
    ];
    let mut github = ScriptedGithub::with([Ok(
        r#"[{"nameWithOwner":"berlinguyinca/one"},{"nameWithOwner":"berlinguyinca/two"},{"nameWithOwner":"berlinguyinca/three"}]"#
            .to_owned(),
    )]);

    let error = run_with_transport(&args, &mut github).unwrap_err();

    assert!(error.to_string().contains("exceeds discovery_max_repos 2"));
    assert!(!repository_path.join(".autospec/state").exists());
}

#[test]
fn onboard_cli_rejects_spawned_from_with_owner_enumeration() {
    let fixture = Fixture::new("onboard-owner-spawned-from");
    let repository_path = initialize_managed_repository(&fixture, "checkout");
    let args = vec![
        "onboard".to_owned(),
        "--repo-dir".to_owned(),
        repository_path.display().to_string(),
        "--owner".to_owned(),
        "berlinguyinca".to_owned(),
        "--allow".to_owned(),
        "berlinguyinca/*".to_owned(),
        "--spawned-from".to_owned(),
        "run:managed-project-onboarding".to_owned(),
    ];
    let mut github = ScriptedGithub::default();

    let error = run_with_transport(&args, &mut github).unwrap_err();

    assert!(error
        .to_string()
        .contains("--spawned-from cannot be combined with --owner"));
    assert!(github.calls.is_empty());
    assert!(!repository_path.join(".autospec/state").exists());
}

#[test]
fn onboard_cli_accepts_owner_wide_allow_under_the_result_cap() {
    let fixture = Fixture::new("onboard-owner-wide");
    let repository_path = initialize_managed_repository(&fixture, "checkout");
    let args = vec![
        "onboard".to_owned(),
        "--repo-dir".to_owned(),
        repository_path.display().to_string(),
        "--owner".to_owned(),
        "berlinguyinca".to_owned(),
        "--allow".to_owned(),
        "berlinguyinca/*".to_owned(),
        "--dry-run".to_owned(),
    ];
    let mut github = ScriptedGithub::with([Ok(
        r#"[{"nameWithOwner":"berlinguyinca/one"},{"nameWithOwner":"berlinguyinca/two"},{"nameWithOwner":"other/three"}]"#
            .to_owned(),
    )]);

    let outcome = run_with_transport(&args, &mut github).unwrap();

    let repositories = outcome["repositories"]
        .as_array()
        .unwrap()
        .iter()
        .map(|repository| repository["repository"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(repositories.contains(&"berlinguyinca/one"));
    assert!(repositories.contains(&"berlinguyinca/two"));
    assert!(!repositories.contains(&"other/three"));
}

#[test]
fn onboard_scans_only_structured_manifest_and_fleet_fields() {
    let fixture = Fixture::new("onboard-structured-fields");
    let repository_path = fixture.path().join("checkout");
    initialize_repository(
        &repository_path,
        "https://github.com/berlinguyinca/autospec.git",
    );
    fs::write(
        repository_path.join("Cargo.toml"),
        "# https://github.com/berlinguyinca/comment-only\n[package]\ndescription = \"https://github.com/berlinguyinca/description-only\"\n[dependencies]\nreal = { git = \"https://github.com/berlinguyinca/real-cargo\" }\n",
    )
    .unwrap();
    fs::write(
        repository_path.join("package.json"),
        r#"{"scripts":{"postinstall":"echo https://github.com/berlinguyinca/script-only"},"homepage":"https://github.com/berlinguyinca/homepage-only","repository":"https://github.com/berlinguyinca/real-package","dependencies":{"real":"github:berlinguyinca/real-npm"}}"#,
    )
    .unwrap();
    fs::write(
        repository_path.join("autospec-fleet.yml"),
        "note: https://github.com/berlinguyinca/note-only\nrepositories:\n  - https://github.com/berlinguyinca/real-fleet\n",
    )
    .unwrap();
    let mut policy = policy("berlinguyinca");
    policy.repo_allowlist = vec!["berlinguyinca/*".to_owned()];
    let mut store =
        ManagedProjectStore::open_product(&fixture.path().join("state"), &key("autospec")).unwrap();

    onboard_repositories(
        &mut store,
        &policy,
        &OnboardingOptions {
            repo_dir: repository_path,
            repositories: Vec::new(),
            workspaces: Vec::new(),
            dry_run: false,
        },
    )
    .unwrap();

    let repositories = store
        .snapshot()
        .repositories
        .iter()
        .map(|record| record.repository.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    for expected in [
        "berlinguyinca/real-cargo",
        "berlinguyinca/real-package",
        "berlinguyinca/real-npm",
        "berlinguyinca/real-fleet",
    ] {
        assert!(repositories.contains(expected), "missing {expected}");
    }
    for false_positive in [
        "berlinguyinca/comment-only",
        "berlinguyinca/description-only",
        "berlinguyinca/script-only",
        "berlinguyinca/homepage-only",
        "berlinguyinca/note-only",
    ] {
        assert!(
            !repositories.contains(false_positive),
            "retained {false_positive}"
        );
    }
}

#[test]
fn onboard_ignores_non_repository_npm_dependency_protocols() {
    let fixture = Fixture::new("onboard-npm-protocols");
    let repository_path = fixture.path().join("checkout");
    initialize_repository(
        &repository_path,
        "https://github.com/berlinguyinca/autospec.git",
    );
    fs::write(
        repository_path.join("package.json"),
        r#"{"dependencies":{"semver":"^1.2.3","registry_alias":"npm:real-package@^2","local":"file:../local","workspace":"workspace:*","github":"github:berlinguyinca/real-npm"}}"#,
    )
    .unwrap();
    let mut policy = policy("berlinguyinca");
    policy.repo_allowlist = vec!["berlinguyinca/*".to_owned()];
    let mut store =
        ManagedProjectStore::open_product(&fixture.path().join("state"), &key("autospec")).unwrap();

    let report = onboard_repositories(
        &mut store,
        &policy,
        &OnboardingOptions {
            repo_dir: repository_path,
            repositories: Vec::new(),
            workspaces: Vec::new(),
            dry_run: false,
        },
    )
    .unwrap();

    assert_eq!(report.inaccessible, 0);
    assert!(report
        .repositories
        .iter()
        .any(|record| record.repository == "berlinguyinca/real-npm"));
    assert_eq!(
        report
            .repositories
            .iter()
            .filter(|record| record.entry_kind == "manifest-dependency")
            .count(),
        1
    );
}

#[test]
fn onboard_accepts_npm_git_plus_github_repository_forms() {
    let fixture = Fixture::new("onboard-npm-git-plus");
    let repository_path = fixture.path().join("checkout");
    initialize_repository(
        &repository_path,
        "https://github.com/berlinguyinca/autospec.git",
    );
    fs::write(
        repository_path.join("package.json"),
        r#"{"repository":{"type":"git","url":"git+https://github.com/berlinguyinca/repository-form.git"},"dependencies":{"ssh":"git+ssh://git@github.com/berlinguyinca/dependency-form.git#main","gitlab":"git+https://gitlab.com/berlinguyinca/not-github.git","malformed":"not a repository"}}"#,
    )
    .unwrap();
    let mut policy = policy("berlinguyinca");
    policy.repo_allowlist = vec!["berlinguyinca/*".to_owned()];
    let mut store =
        ManagedProjectStore::open_product(&fixture.path().join("state"), &key("autospec")).unwrap();

    let report = onboard_repositories(
        &mut store,
        &policy,
        &OnboardingOptions {
            repo_dir: repository_path,
            repositories: Vec::new(),
            workspaces: Vec::new(),
            dry_run: false,
        },
    )
    .unwrap();

    assert_eq!(report.inaccessible, 0);
    let repositories = report
        .repositories
        .iter()
        .map(|record| record.repository.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    assert!(repositories.contains("berlinguyinca/repository-form"));
    assert!(repositories.contains("berlinguyinca/dependency-form"));
    assert_eq!(repositories.len(), 3);
}

#[test]
fn onboard_ignores_non_github_npm_repository_fields() {
    for (name, repository) in [
        (
            "gitlab-repository",
            "git+https://gitlab.com/berlinguyinca/not-github.git",
        ),
        ("malformed-repository", "not a repository"),
    ] {
        let fixture = Fixture::new(name);
        let repository_path = fixture.path().join("checkout");
        initialize_repository(
            &repository_path,
            "https://github.com/berlinguyinca/autospec.git",
        );
        fs::write(
            repository_path.join("package.json"),
            serde_json::json!({ "repository": repository }).to_string(),
        )
        .unwrap();
        let mut policy = policy("berlinguyinca");
        policy.repo_allowlist = vec!["berlinguyinca/*".to_owned()];
        let mut store =
            ManagedProjectStore::open_product(&fixture.path().join("state"), &key("autospec"))
                .unwrap();

        let report = onboard_repositories(
            &mut store,
            &policy,
            &OnboardingOptions {
                repo_dir: repository_path,
                repositories: Vec::new(),
                workspaces: Vec::new(),
                dry_run: false,
            },
        )
        .unwrap();

        assert_eq!(report.inaccessible, 0, "case {name}");
        assert_eq!(report.repositories.len(), 1, "case {name}");
    }
}

#[test]
fn onboard_resolves_cargo_paths_and_pnpm_members_with_typed_failures() {
    let fixture = Fixture::new("onboard-local-workspaces");
    let repository_path = fixture.path().join("checkout");
    initialize_repository(
        &repository_path,
        "https://github.com/berlinguyinca/autospec.git",
    );
    fs::write(
        repository_path.join("Cargo.toml"),
        "[dependencies]\nlocal = { path = \"../cargo-local\" }\n",
    )
    .unwrap();
    fs::write(
        repository_path.join("pnpm-workspace.yaml"),
        "packages:\n  - packages/*\n",
    )
    .unwrap();
    initialize_repository(
        &fixture.path().join("cargo-local"),
        "https://github.com/berlinguyinca/cargo-local.git",
    );
    initialize_repository(
        &repository_path.join("packages/allowed"),
        "https://github.com/berlinguyinca/pnpm-allowed.git",
    );
    initialize_repository(
        &repository_path.join("packages/outside"),
        "https://github.com/other/pnpm-outside.git",
    );
    fs::create_dir_all(repository_path.join("packages/inaccessible")).unwrap();
    let mut policy = policy("berlinguyinca");
    policy.repo_allowlist = vec!["berlinguyinca/*".to_owned()];
    let mut store =
        ManagedProjectStore::open_product(&fixture.path().join("state"), &key("autospec")).unwrap();

    let report = onboard_repositories(
        &mut store,
        &policy,
        &OnboardingOptions {
            repo_dir: repository_path,
            repositories: Vec::new(),
            workspaces: Vec::new(),
            dry_run: false,
        },
    )
    .unwrap();

    let repositories = store
        .snapshot()
        .repositories
        .iter()
        .map(|record| record.repository.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    assert!(repositories.contains("berlinguyinca/cargo-local"));
    assert!(repositories.contains("berlinguyinca/pnpm-allowed"));
    assert_eq!(report.out_of_bound, 1);
    assert_eq!(report.inaccessible, 1);
}

#[test]
fn onboard_rejects_malformed_explicit_repository_seeds() {
    let fixture = Fixture::new("onboard-malformed-seed");
    let repository_path = fixture.path().join("checkout");
    initialize_repository(
        &repository_path,
        "https://github.com/berlinguyinca/autospec.git",
    );
    let mut policy = policy("berlinguyinca");
    policy.repo_allowlist = vec!["berlinguyinca/*".to_owned()];
    let mut store =
        ManagedProjectStore::open_product(&fixture.path().join("state"), &key("autospec")).unwrap();

    let result = onboard_repositories(
        &mut store,
        &policy,
        &OnboardingOptions {
            repo_dir: repository_path,
            repositories: vec!["not a repository".to_owned()],
            workspaces: Vec::new(),
            dry_run: false,
        },
    );

    assert!(result.is_err());
    assert!(store.snapshot().repositories.is_empty());
}

#[test]
fn onboard_cli_validates_every_explicit_seed_before_state_or_github() {
    for (name, policy_seeds, cli_seeds) in [
        (
            "policy-seed",
            "[\"berlinguyinca/autospec\", \"not a repository\"]",
            Vec::new(),
        ),
        (
            "cli-seed",
            "[\"berlinguyinca/autospec\"]",
            vec![
                "--repo",
                "berlinguyinca/allowed",
                "--repo",
                "not a repository",
            ],
        ),
    ] {
        let fixture = Fixture::new(name);
        let repository_path = fixture.path().join("checkout");
        initialize_repository(
            &repository_path,
            "https://github.com/berlinguyinca/autospec.git",
        );
        fs::create_dir_all(repository_path.join(".autospec")).unwrap();
        fs::write(
            repository_path.join(".autospec/autonomous.yml"),
            format!(
                "project_board:\n  mode: managed\n  product_key: autospec\n  owner: berlinguyinca\n  repo_allowlist: [\"berlinguyinca/*\"]\n  repository_seeds: {policy_seeds}\n  discovery_max_repos: 1\n"
            ),
        )
        .unwrap();
        let mut args = vec![
            "onboard".to_owned(),
            "--repo-dir".to_owned(),
            repository_path.display().to_string(),
        ];
        args.extend(cli_seeds.into_iter().map(str::to_owned));
        let mut github = ScriptedGithub::default();

        assert!(run_with_transport(&args, &mut github).is_err());
        assert!(github.calls.is_empty());
        assert!(!repository_path.join(".autospec/state").exists());
    }
}

#[test]
fn onboard_cli_records_contains_and_only_explicit_creation_records_spawned_from() {
    for (name, repository_name, spawned_from) in [
        ("adopted-repository", "berlinguyinca/adopted", None),
        (
            "created-repository",
            "berlinguyinca/created",
            Some("run:managed-project-onboarding"),
        ),
    ] {
        let fixture = Fixture::new(name);
        let repository_path = fixture.path().join("checkout");
        initialize_repository(
            &repository_path,
            "https://github.com/berlinguyinca/autospec.git",
        );
        fs::create_dir_all(repository_path.join(".autospec")).unwrap();
        fs::write(
            repository_path.join(".autospec/autonomous.yml"),
            "project_board:\n  mode: managed\n  product_key: autospec\n  owner: berlinguyinca\n  repo_allowlist: [\"berlinguyinca/*\"]\n  repository_seeds: [\"berlinguyinca/autospec\"]\n  discovery_max_repos: 25\n",
        )
        .unwrap();
        let mut store = ManagedProjectStore::open_product(
            &repository_path.join(".autospec/state"),
            &key("autospec"),
        )
        .unwrap();
        store
            .record_project(
                "berlinguyinca",
                "PVT_7",
                7,
                "https://github.com/orgs/berlinguyinca/projects/7",
                "Autospec",
            )
            .unwrap();
        let mut args = vec![
            "onboard".to_owned(),
            "--repo-dir".to_owned(),
            repository_path.display().to_string(),
            "--repo".to_owned(),
            repository_name.to_owned(),
        ];
        if let Some(identity) = spawned_from {
            args.extend(["--spawned-from".to_owned(), identity.to_owned()]);
        }
        let mut github = ScriptedGithub::with([Ok(project(
            7,
            "berlinguyinca",
            "Autospec",
            &marker("berlinguyinca"),
        ))]);

        run_with_transport(&args, &mut github).unwrap();

        let store = ManagedProjectStore::open_product(
            &repository_path.join(".autospec/state"),
            &key("autospec"),
        )
        .unwrap();
        assert!(store.snapshot().relationships.iter().any(|edge| {
            edge.kind == RelationshipKind::Contains
                && edge.source == "product:autospec"
                && edge.target == repository_name
        }));
        let spawned = store
            .snapshot()
            .relationships
            .iter()
            .filter(|edge| edge.kind == RelationshipKind::SpawnedFrom)
            .collect::<Vec<_>>();
        match spawned_from {
            Some(identity) => {
                assert_eq!(spawned.len(), 1);
                assert_eq!(spawned[0].source, repository_name);
                assert_eq!(spawned[0].target, identity);
            }
            None => assert!(spawned.is_empty()),
        }
    }
}

#[test]
fn onboard_cli_journals_repository_projection_before_remote_reconciliation_failure() {
    let fixture = Fixture::new("onboard-pending-projection");
    let repository_path = fixture.path().join("checkout");
    initialize_repository(
        &repository_path,
        "https://github.com/berlinguyinca/autospec.git",
    );
    fs::create_dir_all(repository_path.join(".autospec")).unwrap();
    fs::write(
        repository_path.join(".autospec/autonomous.yml"),
        "project_board:\n  mode: managed\n  product_key: autospec\n  owner: berlinguyinca\n  repo_allowlist: [\"berlinguyinca/*\"]\n  repository_seeds: [\"berlinguyinca/autospec\"]\n  discovery_max_repos: 25\n",
    )
    .unwrap();
    let args = vec![
        "onboard".to_owned(),
        "--repo-dir".to_owned(),
        repository_path.display().to_string(),
        "--repo".to_owned(),
        "https://github.com/berlinguyinca/created.git".to_owned(),
        "--spawned-from".to_owned(),
        "spec:managed-project-onboarding".to_owned(),
    ];
    let mut github = ScriptedGithub::with([
        Err(GithubFailure::Retryable(
            "project lookup unavailable".to_owned(),
        )),
        Err(GithubFailure::Retryable(
            "project lookup still unavailable".to_owned(),
        )),
    ]);

    let outcome = run_with_transport(&args, &mut github).unwrap();
    let repeated = run_with_transport(&args, &mut github).unwrap();

    assert_eq!(outcome["outcome"], "journaled_projection_pending");
    assert_eq!(outcome["pending_projection"], 2);
    assert_eq!(repeated["outcome"], "journaled_projection_pending");
    assert_eq!(repeated["pending_projection"], 2);
    assert_eq!(
        outcome["repositories"][1]["repository"],
        "berlinguyinca/created"
    );
    assert_eq!(github.calls.len(), 2);

    let state_root = repository_path.join(".autospec/state");
    let reopened = ManagedProjectStore::open_product(&state_root, &key("autospec")).unwrap();
    let snapshot = reopened.snapshot();
    assert!(snapshot
        .repositories
        .iter()
        .any(|record| record.repository == "berlinguyinca/created"));
    assert!(snapshot.relationships.iter().any(|edge| {
        edge.kind == RelationshipKind::Contains
            && edge.source == "product:autospec"
            && edge.target == "berlinguyinca/created"
    }));
    assert!(snapshot.relationships.iter().any(|edge| {
        edge.kind == RelationshipKind::SpawnedFrom
            && edge.source == "berlinguyinca/created"
            && edge.target == "spec:managed-project-onboarding"
    }));
    assert!(snapshot
        .pending_projections
        .contains(&"repository:register:autospec:berlinguyinca/autospec".to_owned()));
    assert!(snapshot
        .pending_projections
        .contains(&"repository:register:autospec:berlinguyinca/created".to_owned()));

    let journal = fs::read_to_string(state_root.join("projects/autospec/events.jsonl")).unwrap();
    let repository_event = journal.find("repository-recorded").unwrap();
    let relationship_event = journal.find("relationship-recorded").unwrap();
    let projection_event = journal.find("projection-enqueued").unwrap();
    assert_eq!(journal.matches("projection-enqueued").count(), 2);
    assert!(repository_event < relationship_event);
    assert!(relationship_event < projection_event);

    drop(reopened);
    let mut bound = ManagedProjectStore::open_product(&state_root, &key("autospec")).unwrap();
    bound
        .record_project(
            "berlinguyinca",
            "PVT_7",
            7,
            "https://github.com/orgs/berlinguyinca/projects/7",
            "Autospec",
        )
        .unwrap();
    drop(bound);
    let sync_args = vec![
        "sync".to_owned(),
        "--repo-dir".to_owned(),
        repository_path.display().to_string(),
    ];
    let mut sync_github = ScriptedGithub::with([Ok(project(
        7,
        "berlinguyinca",
        "Autospec",
        &marker("berlinguyinca"),
    ))]);
    let synced = run_with_transport(&sync_args, &mut sync_github).unwrap();
    assert_eq!(synced["outcome"], "reconciled");
    assert_eq!(synced["pending_projection"], 0);
    let synced_store = ManagedProjectStore::open_product(&state_root, &key("autospec")).unwrap();
    assert!(synced_store.snapshot().pending_projections.is_empty());
}

#[test]
fn onboard_cli_propagates_hard_remote_validation_failure_after_journaling() {
    let fixture = Fixture::new("onboard-hard-remote-validation");
    let repository_path = fixture.path().join("checkout");
    initialize_repository(
        &repository_path,
        "https://github.com/berlinguyinca/autospec.git",
    );
    fs::create_dir_all(repository_path.join(".autospec")).unwrap();
    fs::write(
        repository_path.join(".autospec/autonomous.yml"),
        "project_board:\n  mode: managed\n  product_key: autospec\n  owner: berlinguyinca\n  repo_allowlist: [\"berlinguyinca/*\"]\n  repository_seeds: [\"berlinguyinca/autospec\"]\n  discovery_max_repos: 25\n",
    )
    .unwrap();
    let args = vec![
        "onboard".to_owned(),
        "--repo-dir".to_owned(),
        repository_path.display().to_string(),
        "--repo".to_owned(),
        "berlinguyinca/created".to_owned(),
    ];
    let mut github = ScriptedGithub::with([Ok("not project json".to_owned())]);

    let error = run_with_transport(&args, &mut github).unwrap_err();

    assert!(!error.to_string().is_empty());
    let reopened = ManagedProjectStore::open_product(
        &repository_path.join(".autospec/state"),
        &key("autospec"),
    )
    .unwrap();
    assert!(reopened
        .snapshot()
        .pending_projections
        .contains(&"repository:register:autospec:berlinguyinca/created".to_owned()));
}

#[test]
fn gh_cli_missing_executable_is_a_hard_nonzero_onboarding_failure() {
    let fixture = Fixture::new("gh-cli-missing");
    let repository_path = initialize_managed_repository(&fixture, "checkout");

    let output = run_real_onboard(&repository_path, &fixture.path().join("missing-gh"));

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("cannot execute gh"));
    let store = ManagedProjectStore::open_product(
        &repository_path.join("test-autospec-home"),
        &key("autospec"),
    )
    .unwrap();
    assert!(store
        .snapshot()
        .pending_projections
        .contains(&"repository:register:autospec:berlinguyinca/created".to_owned()));
}

#[cfg(unix)]
#[test]
fn gh_cli_read_auth_403_is_a_hard_nonzero_onboarding_failure() {
    let fixture = Fixture::new("gh-cli-auth-403");
    let repository_path = initialize_managed_repository(&fixture, "checkout");
    let gh_program = fixture.path().join("gh-auth-403");
    fs::write(
        &gh_program,
        "#!/bin/sh\nprintf '%s\\n' 'GraphQL: Resource not accessible by integration (HTTP 403)' >&2\nexit 1\n",
    )
    .unwrap();
    fs::set_permissions(&gh_program, fs::Permissions::from_mode(0o755)).unwrap();

    let output = run_real_onboard(&repository_path, &gh_program);

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("HTTP 403"));
}

#[cfg(unix)]
#[test]
fn gh_cli_owner_enumeration_auth_failure_is_hard_before_onboarding_state() {
    let fixture = Fixture::new("gh-cli-owner-auth-403");
    let repository_path = initialize_managed_repository(&fixture, "checkout");
    let gh_program = fixture.path().join("gh-owner-auth-403");
    fs::write(
        &gh_program,
        "#!/bin/sh\nprintf '%s\\n' 'authentication required (HTTP 401)' >&2\nexit 1\n",
    )
    .unwrap();
    fs::set_permissions(&gh_program, fs::Permissions::from_mode(0o755)).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_autospec"))
        .args([
            "project",
            "onboard",
            "--repo-dir",
            repository_path.to_str().unwrap(),
            "--owner",
            "berlinguyinca",
            "--allow",
            "berlinguyinca/autospec-*",
        ])
        .env("AUTOSPEC_GH_PROGRAM", gh_program)
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("HTTP 401"));
    assert!(!repository_path.join(".autospec/state").exists());
}

#[cfg(unix)]
#[test]
fn gh_cli_transient_read_failure_keeps_the_typed_pending_outcome() {
    let fixture = Fixture::new("gh-cli-transient");
    let repository_path = initialize_managed_repository(&fixture, "checkout");
    let gh_program = fixture.path().join("gh-transient");
    fs::write(
        &gh_program,
        "#!/bin/sh\nprintf '%s\\n' 'failed to connect: HTTP 503 service unavailable' >&2\nexit 1\n",
    )
    .unwrap();
    fs::set_permissions(&gh_program, fs::Permissions::from_mode(0o755)).unwrap();

    let output = run_real_onboard(&repository_path, &gh_program);

    assert!(output.status.success());
    let summary: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(summary["outcome"], "journaled_projection_pending");
    assert_eq!(summary["pending_projection"], 2);
}

#[test]
fn onboard_cli_does_not_project_or_link_an_out_of_bound_spawned_repository() {
    let fixture = Fixture::new("onboard-out-of-bound-spawned");
    let repository_path = fixture.path().join("checkout");
    initialize_repository(
        &repository_path,
        "https://github.com/berlinguyinca/autospec.git",
    );
    fs::create_dir_all(repository_path.join(".autospec")).unwrap();
    fs::write(
        repository_path.join(".autospec/autonomous.yml"),
        "project_board:\n  mode: managed\n  product_key: autospec\n  owner: berlinguyinca\n  repo_allowlist: [\"berlinguyinca/autospec\"]\n  repository_seeds: [\"berlinguyinca/autospec\"]\n  discovery_max_repos: 25\n",
    )
    .unwrap();
    let mut store = ManagedProjectStore::open_product(
        &repository_path.join(".autospec/state"),
        &key("autospec"),
    )
    .unwrap();
    store
        .record_project(
            "berlinguyinca",
            "PVT_7",
            7,
            "https://github.com/orgs/berlinguyinca/projects/7",
            "Autospec",
        )
        .unwrap();
    let args = vec![
        "onboard".to_owned(),
        "--repo-dir".to_owned(),
        repository_path.display().to_string(),
        "--repo".to_owned(),
        "other/created".to_owned(),
        "--spawned-from".to_owned(),
        "spec:managed-project-onboarding".to_owned(),
    ];
    let mut github = ScriptedGithub::with([Ok(project(
        7,
        "berlinguyinca",
        "Autospec",
        &marker("berlinguyinca"),
    ))]);

    let outcome = run_with_transport(&args, &mut github).unwrap();

    assert_eq!(outcome["outcome"], "reconciled");
    assert_eq!(outcome["out_of_bound"], 1);
    let reopened = ManagedProjectStore::open_product(
        &repository_path.join(".autospec/state"),
        &key("autospec"),
    )
    .unwrap();
    assert!(!reopened.snapshot().relationships.iter().any(|edge| {
        edge.kind == RelationshipKind::SpawnedFrom && edge.source == "other/created"
    }));
    assert!(!reopened
        .snapshot()
        .pending_projections
        .iter()
        .any(|projection| projection.ends_with(":other/created")));
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

fn issue(url: &str, body: &str) -> String {
    serde_json::json!({ "url": url, "body": body }).to_string()
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
    assert!(verify_managed_marker(
        &legacy_product_marker("berlinguyinca"),
        &policy("berlinguyinca")
    )
    .unwrap());
    assert!(!verify_managed_marker("# Human project", &policy("berlinguyinca")).unwrap());
    assert!(verify_managed_marker(
        &format!("{}\n{}", marker("berlinguyinca"), marker("berlinguyinca")),
        &policy("berlinguyinca")
    )
    .is_err());
}

#[test]
fn github_bound_spec_portfolio_rejects_product_kind_without_mutation() {
    let fixture = Fixture::new("github-spec-portfolio-kind-mismatch");
    let mut store = open_portfolio_store(fixture.path());
    store
        .record_portfolio_snapshot(portfolio_store_snapshot())
        .unwrap();
    store
        .record_project(
            "berlinguyinca",
            "PVT_7",
            7,
            "https://github.com/orgs/berlinguyinca/projects/7",
            "Delivery",
        )
        .unwrap();
    let mut github = ScriptedGithub::with([Ok(project(
        7,
        "berlinguyinca",
        "Delivery",
        &marker("berlinguyinca"),
    ))]);

    let error = resolve_or_create_project(
        &mut store,
        &mut github,
        &policy("berlinguyinca"),
        "Delivery",
    )
    .unwrap_err();

    assert!(error.to_string().contains("different managed marker"));
    assert!(github
        .calls
        .iter()
        .all(|call| !matches!(call, GithubCommand::EditProjectMarker { .. })));
}

#[test]
fn github_legacy_product_adoption_migrates_the_existing_marker_block() {
    let fixture = Fixture::new("github-legacy-marker-migration");
    let mut store = ManagedProjectStore::open_product(fixture.path(), &key("autospec")).unwrap();
    let legacy = format!(
        "# Human notes\n\n{}\n\nKeep this too.",
        legacy_product_marker("berlinguyinca")
    );
    let migrated = format!(
        "# Human notes\n\n{}\n\nKeep this too.",
        marker("berlinguyinca")
    );
    let mut github = ScriptedGithub::with([
        Ok(project_list(serde_json::json!([{"number": 7}]))),
        Ok(project(7, "berlinguyinca", "Human title", &legacy)),
        Ok(String::new()),
        Ok(project(7, "berlinguyinca", "Human title", &migrated)),
    ]);

    let resolved = resolve_or_create_project(
        &mut store,
        &mut github,
        &policy("berlinguyinca"),
        "Autospec",
    )
    .unwrap();

    assert_eq!(resolved.number, 7);
    let edited = github.calls.iter().find_map(|call| match call {
        GithubCommand::EditProjectMarker { readme, .. } => Some(readme.as_str()),
        _ => None,
    });
    assert_eq!(edited, Some(migrated.as_str()));
}

#[test]
fn github_legacy_product_migration_requires_the_schema_two_marker_on_requery() {
    let fixture = Fixture::new("github-legacy-marker-stale-requery");
    let mut store = ManagedProjectStore::open_product(fixture.path(), &key("autospec")).unwrap();
    let legacy = legacy_product_marker("berlinguyinca");
    let mut github = ScriptedGithub::with([
        Ok(project_list(serde_json::json!([{"number": 7}]))),
        Ok(project(7, "berlinguyinca", "Human title", &legacy)),
        Ok(String::new()),
        Ok(project(7, "berlinguyinca", "Human title", &legacy)),
    ]);

    let error = resolve_or_create_project(
        &mut store,
        &mut github,
        &policy("berlinguyinca"),
        "Autospec",
    )
    .unwrap_err();

    assert!(error.to_string().contains("expected managed marker"));
    assert!(store.snapshot().project_node_id.is_none());
}

#[test]
fn github_spec_portfolio_adopts_exactly_one_marker_bearing_project() {
    let fixture = Fixture::new("github-spec-portfolio-adopt");
    let mut store = open_portfolio_store(fixture.path());
    store
        .record_portfolio_snapshot(portfolio_store_snapshot())
        .unwrap();
    let marker = portfolio_marker("berlinguyinca");
    let mut github = ScriptedGithub::with([
        Ok(project_list(serde_json::json!([{"number": 7}]))),
        Ok(project(7, "berlinguyinca", "Delivery", &marker)),
    ]);

    let resolved = resolve_or_create_project(
        &mut store,
        &mut github,
        &policy("berlinguyinca"),
        "Delivery",
    )
    .unwrap();

    assert_eq!(resolved.number, 7);
    assert!(github.calls.iter().all(|call| !matches!(
        call,
        GithubCommand::CreateProject { .. } | GithubCommand::EditProjectMarker { .. }
    )));
}

#[test]
fn github_spec_portfolio_create_unknown_waits_when_nonce_title_is_not_visible() {
    let fixture = Fixture::new("github-spec-portfolio-create-unknown-zero");
    let mut store = open_portfolio_store(fixture.path());
    store
        .record_portfolio_snapshot(portfolio_store_snapshot())
        .unwrap();
    let mut first = ScriptedGithub::with([
        Ok(project_list(serde_json::json!([]))),
        Err(GithubFailure::Ambiguous("create response lost".to_owned())),
    ]);
    assert!(resolve_or_create_project(
        &mut store,
        &mut first,
        &policy("berlinguyinca"),
        "Delivery"
    )
    .is_err());

    let mut retry = ScriptedGithub::with([Ok(project_list(serde_json::json!([])))]);
    let error =
        resolve_or_create_project(&mut store, &mut retry, &policy("berlinguyinca"), "Delivery")
            .unwrap_err();
    assert!(error.to_string().contains("create_unknown"));
    assert!(retry
        .calls
        .iter()
        .all(|call| !matches!(call, GithubCommand::CreateProject { .. })));
}

#[test]
fn github_spec_portfolio_create_unknown_recovers_one_nonce_title_candidate() {
    let fixture = Fixture::new("github-spec-portfolio-create-unknown-one");
    let mut store = open_portfolio_store(fixture.path());
    store
        .record_portfolio_snapshot(portfolio_store_snapshot())
        .unwrap();
    let exact_title = "Delivery [autospec:00112233445566778899aabbccddeeff]";
    let mut first = ScriptedGithub::with([
        Ok(project_list(serde_json::json!([]))),
        Err(GithubFailure::Ambiguous("create response lost".to_owned())),
    ]);
    assert!(resolve_or_create_project(
        &mut store,
        &mut first,
        &policy("berlinguyinca"),
        "Delivery"
    )
    .is_err());

    let marked = format!("human\n\n{}", portfolio_marker("berlinguyinca"));
    let unmarked = project(7, "berlinguyinca", exact_title, "human");
    let mut retry = ScriptedGithub::with([
        Ok(project_list(serde_json::json!([{
            "number": 7,
            "title": exact_title
        }]))),
        Ok(unmarked.clone()),
        Ok(unmarked),
        Ok(String::new()),
        Ok(project(7, "berlinguyinca", exact_title, &marked)),
    ]);

    let resolved =
        resolve_or_create_project(&mut store, &mut retry, &policy("berlinguyinca"), "Delivery")
            .unwrap();

    assert_eq!(resolved.number, 7);
    assert!(store.snapshot().pending_projections.is_empty());
    assert!(retry
        .calls
        .iter()
        .all(|call| !matches!(call, GithubCommand::CreateProject { .. })));
}

#[test]
fn github_spec_portfolio_create_unknown_refuses_two_nonce_title_candidates() {
    let fixture = Fixture::new("github-spec-portfolio-create-unknown-two");
    let mut store = open_portfolio_store(fixture.path());
    store
        .record_portfolio_snapshot(portfolio_store_snapshot())
        .unwrap();
    let exact_title = "Delivery [autospec:00112233445566778899aabbccddeeff]";
    let mut first = ScriptedGithub::with([
        Ok(project_list(serde_json::json!([]))),
        Err(GithubFailure::Ambiguous("create response lost".to_owned())),
    ]);
    assert!(resolve_or_create_project(
        &mut store,
        &mut first,
        &policy("berlinguyinca"),
        "Delivery"
    )
    .is_err());

    let mut retry = ScriptedGithub::with([
        Ok(project_list(serde_json::json!([
            {"number": 7, "title": exact_title},
            {"number": 8, "title": exact_title}
        ]))),
        Ok(project(7, "berlinguyinca", exact_title, "human one")),
        Ok(project(8, "berlinguyinca", exact_title, "human two")),
    ]);

    let error =
        resolve_or_create_project(&mut store, &mut retry, &policy("berlinguyinca"), "Delivery")
            .unwrap_err();

    assert!(error.to_string().contains("create_unknown: 2"));
    assert!(retry.calls.iter().all(|call| !matches!(
        call,
        GithubCommand::CreateProject { .. } | GithubCommand::EditProjectMarker { .. }
    )));
}

#[test]
fn github_resolve_creates_marks_verifies_and_persists_when_no_marker_matches() {
    let fixture = Fixture::new("github-create");
    let mut store = ManagedProjectStore::open_product(fixture.path(), &key("autospec")).unwrap();
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
    let mut store = ManagedProjectStore::open_product(fixture.path(), &key("autospec")).unwrap();
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

    let mut reopened = ManagedProjectStore::open_product(fixture.path(), &key("autospec")).unwrap();
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
    let mut store = ManagedProjectStore::open_product(fixture.path(), &key("autospec")).unwrap();
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

    let mut reopened = ManagedProjectStore::open_product(fixture.path(), &key("autospec")).unwrap();
    let mut reconcile = ScriptedGithub::default();
    assert!(reconcile_issue(
        &mut reopened,
        &mut reconcile,
        &policy,
        "https://github.com/berlinguyinca/autospec/issues/47",
    )
    .is_err());
    assert!(reconcile.calls.is_empty());
    assert_eq!(reopened.snapshot().pending_projections.len(), 2);
    assert!(reopened
        .snapshot()
        .pending_projections
        .iter()
        .any(|projection| {
            projection
                == "project:item-add:unresolved:https://github.com/berlinguyinca/autospec/issues/47"
        }));
}

#[test]
fn github_provisional_recovery_accepts_renamed_project_and_persists_verified_metadata() {
    let fixture = Fixture::new("github-provisional-rename");
    let policy = policy("berlinguyinca");
    let mut store = ManagedProjectStore::open_product(fixture.path(), &key("autospec")).unwrap();
    let mut create = ScriptedGithub::with([
        Ok(project_list(serde_json::json!([]))),
        Ok(project(7, "berlinguyinca", "Original title", "human")),
        Err(GithubFailure::Retryable("view interrupted".to_owned())),
    ]);
    assert!(resolve_or_create_project(&mut store, &mut create, &policy, "Original title").is_err());
    drop(store);

    let mut reopened = ManagedProjectStore::open_product(fixture.path(), &key("autospec")).unwrap();
    let marked = format!("human\n\n{}", marker("berlinguyinca"));
    let mut renamed: serde_json::Value =
        serde_json::from_str(&project(7, "berlinguyinca", "Human rename", &marked)).unwrap();
    renamed["url"] =
        serde_json::json!("https://github.com/orgs/berlinguyinca/projects/7?view=roadmap");
    let mut github = ScriptedGithub::with([Ok(renamed.to_string())]);

    resolve_or_create_project(&mut reopened, &mut github, &policy, "Original title").unwrap();

    assert_eq!(
        reopened.snapshot().project_title.as_deref(),
        Some("Human rename")
    );
    assert_eq!(
        reopened.snapshot().project_url.as_deref(),
        Some("https://github.com/orgs/berlinguyinca/projects/7?view=roadmap")
    );
    assert!(reopened.snapshot().pending_projections.is_empty());
    assert_eq!(github.calls.len(), 1);
}

#[test]
fn github_ambiguous_marker_edit_resumes_from_verified_bound_project() {
    let fixture = Fixture::new("github-edit-resume");
    let policy = policy("berlinguyinca");
    let mut store = ManagedProjectStore::open_product(fixture.path(), &key("autospec")).unwrap();
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

    let mut refreshed: serde_json::Value = serde_json::from_str(&marked).unwrap();
    refreshed["title"] = serde_json::json!("Autospec delivery");
    refreshed["url"] =
        serde_json::json!("https://github.com/orgs/berlinguyinca/projects/7?view=delivery");
    let mut retry = ScriptedGithub::with([Ok(refreshed.to_string())]);
    resolve_or_create_project(&mut store, &mut retry, &policy, "Autospec").unwrap();

    assert!(store.snapshot().pending_projections.is_empty());
    assert_eq!(
        store.snapshot().project_title.as_deref(),
        Some("Autospec delivery")
    );
    assert_eq!(
        store.snapshot().project_url.as_deref(),
        Some("https://github.com/orgs/berlinguyinca/projects/7?view=delivery")
    );
    assert_eq!(retry.calls.len(), 1);
    assert!(matches!(retry.calls[0], GithubCommand::ViewProject { .. }));
}

#[test]
fn github_pending_create_without_identity_never_creates_a_second_project() {
    let fixture = Fixture::new("github-create-ambiguous");
    let policy = policy("berlinguyinca");
    let mut store = ManagedProjectStore::open_product(fixture.path(), &key("autospec")).unwrap();
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
    let mut store = ManagedProjectStore::open_product(fixture.path(), &key("autospec")).unwrap();
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
        let mut store =
            ManagedProjectStore::open_product(fixture.path(), &key("autospec")).unwrap();
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
    let mut store = ManagedProjectStore::open_product(fixture.path(), &key("autospec")).unwrap();
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
    let mut store = ManagedProjectStore::open_product(fixture.path(), &key("autospec")).unwrap();
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
    let mut store = ManagedProjectStore::open_product(fixture.path(), &key("autospec")).unwrap();
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
    let mut store = ManagedProjectStore::open_product(fixture.path(), &key("autospec")).unwrap();
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
    let mut store = ManagedProjectStore::open_product(fixture.path(), &key("autospec")).unwrap();
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
    assert!(store.snapshot().pending_projections.is_empty());
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
fn github_reconcile_journals_before_listing_remote_items() {
    let fixture = Fixture::new("github-reconcile-list-failure");
    let policy = policy("berlinguyinca");
    let issue_url = "HTTPS://GITHUB.COM/berlinguyinca/autospec/issues/00046/?view=1";
    let mut store = ManagedProjectStore::open_product(fixture.path(), &key("autospec")).unwrap();
    store
        .record_project(
            "berlinguyinca",
            "PVT_7",
            7,
            "https://github.com/orgs/berlinguyinca/projects/7",
            "Autospec",
        )
        .unwrap();
    let mut github = ScriptedGithub::with([Err(GithubFailure::Retryable(
        "item list unavailable".to_owned(),
    ))]);

    assert!(reconcile_issue(&mut store, &mut github, &policy, issue_url).is_err());
    assert_eq!(store.snapshot().pending_projections.len(), 1);
    assert_eq!(
        store.snapshot().pending_projections[0],
        "project:item-add:PVT_7:https://github.com/berlinguyinca/autospec/issues/46"
    );
}

#[test]
fn issue_projection_is_durable_before_project_identity_exists_and_promotes_after_binding() {
    let fixture = Fixture::new("github-unresolved-issue-outbox");
    let policy = policy("berlinguyinca");
    let issue_url = "HTTPS://GITHUB.COM/BerlinGuyInCA/AutoSpec/issues/00046/?view=1";
    let mut store = ManagedProjectStore::open_product(fixture.path(), &key("autospec")).unwrap();

    journal_issue_projection(&mut store, issue_url).unwrap();
    assert_eq!(
        store.snapshot().pending_projections,
        ["project:item-add:unresolved:https://github.com/berlinguyinca/autospec/issues/46"]
    );

    store
        .record_project(
            "berlinguyinca",
            "PVT_7",
            7,
            "https://github.com/orgs/berlinguyinca/projects/7",
            "Autospec",
        )
        .unwrap();
    let mut github = ScriptedGithub::with([Ok(item_list(&[])), Ok(String::new())]);
    retry_pending_projections(&mut store, &mut github, &policy).unwrap();

    assert!(store.snapshot().pending_projections.is_empty());
    assert!(github.calls.iter().any(|call| matches!(
        call,
        GithubCommand::AddToProject { issue_url, .. }
            if issue_url == "https://github.com/berlinguyinca/autospec/issues/46"
    )));
}

#[test]
fn project_binding_refreshes_mutable_title_and_url_for_the_same_project_identity() {
    let fixture = Fixture::new("project-binding-metadata-refresh");
    let mut store = ManagedProjectStore::open_product(fixture.path(), &key("autospec")).unwrap();
    store
        .record_project(
            "berlinguyinca",
            "PVT_7",
            7,
            "https://github.com/orgs/berlinguyinca/projects/7",
            "Autospec",
        )
        .unwrap();

    store
        .record_project(
            "berlinguyinca",
            "PVT_7",
            7,
            "https://github.com/users/berlinguyinca/projects/7",
            "Autospec delivery",
        )
        .unwrap();
    drop(store);

    let reopened = ManagedProjectStore::open_product(fixture.path(), &key("autospec")).unwrap();
    assert_eq!(
        reopened.snapshot().project_url.as_deref(),
        Some("https://github.com/users/berlinguyinca/projects/7")
    );
    assert_eq!(
        reopened.snapshot().project_title.as_deref(),
        Some("Autospec delivery")
    );
}

#[test]
fn github_item_reconciliation_ignores_known_nonissues_but_rejects_unknown_items() {
    let fixture = Fixture::new("github-item-shapes");
    let policy = policy("berlinguyinca");
    let mut store = ManagedProjectStore::open_product(fixture.path(), &key("autospec")).unwrap();
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
        assert_eq!(store.snapshot().pending_projections.len(), 1);
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
    let mut store = ManagedProjectStore::open_product(fixture.path(), &key("autospec")).unwrap();
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
    let mut store = ManagedProjectStore::open_product(fixture.path(), &key("autospec")).unwrap();
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

    let reopened = ManagedProjectStore::open_product(fixture.path(), &key("autospec")).unwrap();
    assert_eq!(reopened.snapshot().repositories.len(), 1);
    assert_eq!(reopened.snapshot().relationships.len(), 1);
    assert_eq!(reopened.snapshot().pending_projections.len(), 1);
}

#[test]
fn store_read_only_replays_newest_valid_events_without_repairing_files() {
    let fixture = Fixture::new("read-only-replay");
    let mut store = ManagedProjectStore::open_product(fixture.path(), &key("autospec")).unwrap();
    store
        .record_repository(repository("berlinguyinca/first", "explicit-seed"))
        .unwrap();
    let stale_binding = fs::read(fixture.state_path("binding.json")).unwrap();
    store
        .record_repository(repository("berlinguyinca/second", "manifest-dependency"))
        .unwrap();
    drop(store);
    fs::write(fixture.state_path("binding.json"), stale_binding).unwrap();
    fs::OpenOptions::new()
        .append(true)
        .open(fixture.state_path("events.jsonl"))
        .unwrap()
        .write_all(br#"{"partial""#)
        .unwrap();
    let binding_before = fs::read(fixture.state_path("binding.json")).unwrap();
    let journal_before = fs::read(fixture.state_path("events.jsonl")).unwrap();

    let read_only =
        ManagedProjectStore::open_product_read_only(fixture.path(), &key("autospec")).unwrap();

    assert_eq!(read_only.snapshot().repositories.len(), 2);
    assert_eq!(
        fs::read(fixture.state_path("binding.json")).unwrap(),
        binding_before
    );
    assert_eq!(
        fs::read(fixture.state_path("events.jsonl")).unwrap(),
        journal_before
    );
}

#[test]
fn store_read_only_rejects_nonempty_binding_without_valid_journal() {
    let fixture = Fixture::new("read-only-missing-journal");
    let mut store = ManagedProjectStore::open_product(fixture.path(), &key("autospec")).unwrap();
    store
        .record_repository(repository("berlinguyinca/autospec", "explicit-seed"))
        .unwrap();
    drop(store);
    fs::remove_file(fixture.state_path("events.jsonl")).unwrap();
    let binding_before = fs::read(fixture.state_path("binding.json")).unwrap();

    assert!(ManagedProjectStore::open_product_read_only(fixture.path(), &key("autospec")).is_err());
    assert_eq!(
        fs::read(fixture.state_path("binding.json")).unwrap(),
        binding_before
    );
    assert!(!fixture.state_path("events.jsonl").exists());
}

#[test]
fn store_global_open_imports_one_legacy_repo_state_and_reuses_it_across_repositories() {
    let fixture = Fixture::new("global-import");
    let global = fixture.path().join("global");
    let legacy_one = fixture.path().join("repo-one/.autospec/state");
    let legacy_two = fixture.path().join("repo-two/.autospec/state");
    fs::create_dir_all(legacy_one.parent().unwrap()).unwrap();
    fs::create_dir_all(legacy_two.parent().unwrap()).unwrap();
    let mut first = ManagedProjectStore::open_product(&legacy_one, &key("autospec")).unwrap();
    first
        .record_repository(repository("berlinguyinca/one", "explicit-seed"))
        .unwrap();
    drop(first);
    let mut second = ManagedProjectStore::open_product(&legacy_two, &key("autospec")).unwrap();
    second
        .record_repository(repository("berlinguyinca/two", "explicit-seed"))
        .unwrap();
    drop(second);

    let imported =
        ManagedProjectStore::open_product_global(&global, Some(&legacy_one), &key("autospec"))
            .unwrap();
    assert_eq!(
        imported.snapshot().repositories[0].repository,
        "berlinguyinca/one"
    );
    drop(imported);
    let reused =
        ManagedProjectStore::open_product_global(&global, Some(&legacy_two), &key("autospec"))
            .unwrap();
    assert_eq!(reused.snapshot().repositories.len(), 1);
    assert_eq!(
        reused.snapshot().repositories[0].repository,
        "berlinguyinca/one"
    );
    assert!(global.join("projects/autospec/binding.lock").exists());
}

#[test]
fn store_global_open_ignores_corrupt_legacy_after_global_state_exists() {
    let fixture = Fixture::new("global-precedes-corrupt-legacy");
    let global = fixture.path().join("global");
    let legacy = fixture.path().join("repo/.autospec/state");
    fs::create_dir_all(fixture.path()).unwrap();
    let mut existing = ManagedProjectStore::open_product(&global, &key("autospec")).unwrap();
    existing
        .record_repository(repository("berlinguyinca/global", "explicit-seed"))
        .unwrap();
    drop(existing);
    let legacy_project = legacy.join("projects/autospec");
    fs::create_dir_all(&legacy_project).unwrap();
    fs::write(legacy_project.join("binding.json"), "not-json").unwrap();
    fs::write(legacy_project.join("events.jsonl"), "not-json\n").unwrap();

    let reopened =
        ManagedProjectStore::open_product_global(&global, Some(&legacy), &key("autospec")).unwrap();

    assert_eq!(reopened.snapshot().repositories.len(), 1);
    assert_eq!(
        reopened.snapshot().repositories[0].repository,
        "berlinguyinca/global"
    );
}

#[cfg(unix)]
#[test]
fn store_read_only_rejects_a_symlinked_or_public_ancestor_before_reading_state() {
    let fixture = Fixture::new("read-only-ancestor");
    let state = fixture.path().join("state");
    fs::create_dir_all(fixture.path()).unwrap();
    let mut store = ManagedProjectStore::open_product(&state, &key("autospec")).unwrap();
    store
        .record_repository(repository("berlinguyinca/autospec", "explicit-seed"))
        .unwrap();
    drop(store);

    fs::set_permissions(&state, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(ManagedProjectStore::open_product_read_only(&state, &key("autospec")).is_err());
    fs::set_permissions(&state, fs::Permissions::from_mode(0o700)).unwrap();

    let linked = fixture.path().join("linked-state");
    std::os::unix::fs::symlink(&state, &linked).unwrap();
    assert!(ManagedProjectStore::open_product_read_only(&linked, &key("autospec")).is_err());
}

#[test]
fn store_duplicate_event_keys_are_no_ops() {
    let fixture = Fixture::new("dedupe");
    let mut store = ManagedProjectStore::open_product(fixture.path(), &key("autospec")).unwrap();
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
    let mut first = ManagedProjectStore::open_product(fixture.path(), &key("autospec")).unwrap();
    let mut second = ManagedProjectStore::open_product(fixture.path(), &key("autospec")).unwrap();

    first
        .record_repository(repository("berlinguyinca/autospec", "explicit-seed"))
        .unwrap();
    second
        .record_repository(repository("berlinguyinca/autospec-node", "explicit-seed"))
        .unwrap();
    drop((first, second));

    let reopened = ManagedProjectStore::open_product(fixture.path(), &key("autospec")).unwrap();
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
    let mut store = ManagedProjectStore::open_product(fixture.path(), &key("autospec")).unwrap();
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

    let reopened = ManagedProjectStore::open_product(fixture.path(), &key("autospec")).unwrap();
    assert_eq!(reopened.snapshot().repositories.len(), 2);
    assert_eq!(fs::read_to_string(journal_path).unwrap().lines().count(), 2);
}

#[test]
fn store_ack_projection_is_retryable_but_unknown_keys_fail_closed() {
    let fixture = Fixture::new("ack");
    let mut store = ManagedProjectStore::open_product(fixture.path(), &key("autospec")).unwrap();
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
    let mut store = ManagedProjectStore::open_product(fixture.path(), &key("autospec")).unwrap();
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

    let reopened = ManagedProjectStore::open_product(fixture.path(), &key("autospec")).unwrap();
    assert_eq!(reopened.snapshot().repositories.len(), 1);
    assert!(fs::read_to_string(fixture.state_path("events.jsonl"))
        .unwrap()
        .ends_with('\n'));
    assert!(fixture.state_path("binding.json").is_file());
}

#[test]
fn store_rejects_empty_interior_journal_lines() {
    let fixture = Fixture::new("empty-journal-line");
    let mut store = ManagedProjectStore::open_product(fixture.path(), &key("autospec")).unwrap();
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

    assert!(ManagedProjectStore::open_product(fixture.path(), &key("autospec")).is_err());
}

#[test]
fn store_rebuilds_a_stale_snapshot_from_the_journal() {
    let fixture = Fixture::new("stale-snapshot");
    let mut store = ManagedProjectStore::open_product(fixture.path(), &key("autospec")).unwrap();
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

    let reopened = ManagedProjectStore::open_product(fixture.path(), &key("autospec")).unwrap();
    assert_eq!(reopened.snapshot().repositories.len(), 1);
}

#[test]
fn store_missing_journal_fails_closed_without_overwriting_a_nonempty_binding() {
    let fixture = Fixture::new("missing-journal");
    let mut store = ManagedProjectStore::open_product(fixture.path(), &key("autospec")).unwrap();
    store
        .record_repository(repository("berlinguyinca/autospec", "explicit-seed"))
        .unwrap();
    drop(store);
    let binding_path = fixture.state_path("binding.json");
    let binding_before = fs::read(&binding_path).unwrap();
    fs::remove_file(fixture.state_path("events.jsonl")).unwrap();

    assert!(ManagedProjectStore::open_product(fixture.path(), &key("autospec")).is_err());
    assert_eq!(fs::read(binding_path).unwrap(), binding_before);
    assert!(!fixture.state_path("events.jsonl").exists());
}

#[test]
fn store_zero_length_journal_fails_closed_without_modifying_state() {
    let fixture = Fixture::new("empty-journal");
    let mut store = ManagedProjectStore::open_product(fixture.path(), &key("autospec")).unwrap();
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

    assert!(ManagedProjectStore::open_product(fixture.path(), &key("autospec")).is_err());
    assert_eq!(fs::read(binding_path).unwrap(), binding_before);
    assert!(fs::read(journal_path).unwrap().is_empty());
}

#[test]
fn store_valid_journal_prefix_behind_snapshot_fails_closed_without_modifying_state() {
    let fixture = Fixture::new("journal-prefix");
    let mut store = ManagedProjectStore::open_product(fixture.path(), &key("autospec")).unwrap();
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

    assert!(ManagedProjectStore::open_product(fixture.path(), &key("autospec")).is_err());
    assert_eq!(fs::read(binding_path).unwrap(), binding_before);
    assert_eq!(fs::read(journal_path).unwrap(), first_line.as_bytes());
}

#[test]
fn store_rejects_mismatched_binding_and_edge_product_keys() {
    let fixture = Fixture::new("mismatched-key");
    let store = ManagedProjectStore::open_product(fixture.path(), &key("autospec")).unwrap();
    drop(store);
    let binding_path = fixture.state_path("binding.json");
    let mut binding: serde_json::Value =
        serde_json::from_slice(&fs::read(&binding_path).unwrap()).unwrap();
    binding["product_key"] = serde_json::json!("other");
    fs::write(&binding_path, serde_json::to_vec(&binding).unwrap()).unwrap();
    #[cfg(unix)]
    fs::set_permissions(&binding_path, fs::Permissions::from_mode(0o600)).unwrap();
    assert!(ManagedProjectStore::open_product(fixture.path(), &key("autospec")).is_err());

    let other = Fixture::new("mismatched-edge");
    let mut store = ManagedProjectStore::open_product(other.path(), &key("autospec")).unwrap();
    let mut wrong_edge = edge();
    wrong_edge.product_key = key("other");
    assert!(store.record_edge(wrong_edge).is_err());
}

#[test]
#[cfg(unix)]
fn store_uses_private_state_and_rejects_public_binding_files() {
    let fixture = Fixture::new("private-state");
    let store = ManagedProjectStore::open_product(fixture.path(), &key("autospec")).unwrap();
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
    assert!(ManagedProjectStore::open_product(fixture.path(), &key("autospec")).is_err());
}

#[test]
#[cfg(unix)]
fn store_rejects_a_public_product_lock_file() {
    let fixture = Fixture::new("public-lock");
    let store = ManagedProjectStore::open_product(fixture.path(), &key("autospec")).unwrap();
    drop(store);
    fs::set_permissions(
        fixture.state_path("binding.lock"),
        fs::Permissions::from_mode(0o644),
    )
    .unwrap();

    assert!(ManagedProjectStore::open_product(fixture.path(), &key("autospec")).is_err());
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

    assert!(ManagedProjectStore::open_product(fixture.path(), &key("autospec")).is_err());
}

#[test]
fn managed_project_store_persists_ordered_portfolio_bindings_and_operation_states() {
    let fixture = Fixture::new("portfolio-state");
    let mut store = open_portfolio_store(fixture.path());
    store
        .record_portfolio_snapshot(portfolio_store_snapshot())
        .unwrap();
    assert!(store
        .record_portfolio_item_binding(portfolio_item_binding("issue:portfolio-store", 101))
        .is_err());
    store
        .record_portfolio_item_binding(portfolio_item_binding("source-tracker", 100))
        .unwrap();
    store
        .record_portfolio_item_binding(portfolio_item_binding("issue:portfolio-store", 101))
        .unwrap();
    assert!(store
        .transition_portfolio_operation(
            "item:add:issue:portfolio-store",
            "sent",
            serde_json::json!({"field": "Status"}),
        )
        .is_err());
    for state in ["intent", "sent", "acknowledged"] {
        store
            .transition_portfolio_operation(
                "item:add:issue:portfolio-store",
                state,
                serde_json::json!({"field": "Status"}),
            )
            .unwrap();
    }
    store
        .record_portfolio_snapshot(portfolio_store_snapshot())
        .unwrap();
    drop(store);

    let reopened = open_portfolio_store(fixture.path());
    assert_eq!(reopened.portfolio_item_bindings().len(), 2);
    assert_eq!(
        reopened.portfolio_operation_states(),
        vec![
            (
                "item:add:issue:portfolio-store".to_owned(),
                "intent".to_owned()
            ),
            (
                "item:add:issue:portfolio-store".to_owned(),
                "sent".to_owned()
            ),
            (
                "item:add:issue:portfolio-store".to_owned(),
                "acknowledged".to_owned()
            ),
        ]
    );
    assert!(reopened.portfolio_snapshot().is_some());
    assert!(
        reopened.portfolio_snapshot().unwrap()["projection_high_watermark"]
            .as_u64()
            .is_some_and(|value| value > 0)
    );
}

#[test]
fn managed_project_store_rejects_incomplete_or_mismatched_recovery_capsules() {
    let mut cases = Vec::new();

    let mut missing_capsule = portfolio_store_snapshot();
    missing_capsule
        .as_object_mut()
        .unwrap()
        .remove("recovery_capsule");
    cases.push(missing_capsule);

    let mut mismatched_identity = portfolio_store_snapshot();
    mismatched_identity["recovery_capsule"]["portfolio_id"] =
        serde_json::json!("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
    cases.push(mismatched_identity);

    let mut duplicate_item = portfolio_store_snapshot();
    duplicate_item["recovery_capsule"]["items"][1]["item_key"] =
        serde_json::json!("source-tracker");
    cases.push(duplicate_item);

    let mut dangling_dependency = portfolio_store_snapshot();
    dangling_dependency["recovery_capsule"]["items"][1]["dependencies"] =
        serde_json::json!(["issue:missing"]);
    cases.push(dangling_dependency);

    for (index, snapshot) in cases.into_iter().enumerate() {
        let fixture = Fixture::new(&format!("incomplete-capsule-{index}"));
        let mut store = open_portfolio_store(fixture.path());
        assert!(store.record_portfolio_snapshot(snapshot).is_err());
    }
}

#[test]
fn managed_project_store_repairs_partial_portfolio_journal_tail_from_complete_capsule() {
    let fixture = Fixture::new("portfolio-truncated-tail");
    let mut store = open_portfolio_store(fixture.path());
    store
        .record_portfolio_snapshot(portfolio_store_snapshot())
        .unwrap();
    store
        .record_portfolio_item_binding(portfolio_item_binding("source-tracker", 100))
        .unwrap();
    drop(store);
    fs::OpenOptions::new()
        .append(true)
        .open(fixture.portfolio_state_path("events.jsonl"))
        .unwrap()
        .write_all(br#"{"schema":1,"sequence":3,"kind":"portfolio-operation""#)
        .unwrap();
    fs::remove_file(fixture.portfolio_state_path("portfolio.json")).unwrap();

    let reopened = open_portfolio_store(fixture.path());
    assert_eq!(reopened.portfolio_item_bindings().len(), 1);
    assert!(fixture.portfolio_state_path("portfolio.json").is_file());
    assert!(
        fs::read_to_string(fixture.portfolio_state_path("events.jsonl"))
            .unwrap()
            .ends_with('\n')
    );
}

#[test]
#[cfg(unix)]
fn managed_project_store_keeps_portfolio_snapshot_private_and_rejects_unsafe_files() {
    let fixture = Fixture::new("private-portfolio");
    let mut store = open_portfolio_store(fixture.path());
    store
        .record_portfolio_snapshot(portfolio_store_snapshot())
        .unwrap();
    drop(store);
    let portfolio_path = fixture.portfolio_state_path("portfolio.json");
    assert_eq!(
        fs::metadata(&portfolio_path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    fs::set_permissions(&portfolio_path, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(ManagedProjectStore::open(fixture.path(), &portfolio_store_identity()).is_err());

    let symlink_fixture = Fixture::new("symlinked-portfolio");
    let store = open_portfolio_store(symlink_fixture.path());
    drop(store);
    let target = symlink_fixture.path().join("outside.json");
    fs::write(&target, b"{}").unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();
    std::os::unix::fs::symlink(
        &target,
        symlink_fixture.portfolio_state_path("portfolio.json"),
    )
    .unwrap();
    assert!(
        ManagedProjectStore::open(symlink_fixture.path(), &portfolio_store_identity()).is_err()
    );
}

#[test]
fn managed_project_store_binds_portfolio_identity_and_rejects_product_events() {
    let fixture = Fixture::new("portfolio-identity");
    let mut portfolio = open_portfolio_store(fixture.path());
    portfolio
        .record_portfolio_snapshot(portfolio_store_snapshot())
        .unwrap();
    assert!(fixture.portfolio_state_path("portfolio.json").starts_with(
        fixture.path().join("projects").join(format!(
            "portfolio.{}",
            portfolio_source_identity().portfolio_id()
        ))
    ));

    let changed = ManagedProjectIdentity::SpecPortfolio(SpecPortfolioIdentity::new(
        SourceSpecIdentity::new(
            "berlinguyinca/autospec",
            "docs/specs/automatic-projects.md",
            "1123456789abcdef0123456789abcdef01234567",
        )
        .unwrap(),
    ));
    let mut wrong_identity = ManagedProjectStore::open(fixture.path(), &changed).unwrap();
    assert!(wrong_identity
        .record_portfolio_snapshot(portfolio_store_snapshot())
        .is_err());

    let mut product = ManagedProjectStore::open_product(fixture.path(), &key("autospec")).unwrap();
    assert!(product
        .record_portfolio_snapshot(portfolio_store_snapshot())
        .is_err());
}

#[test]
fn managed_project_store_retries_durable_events_before_derived_validation() {
    let fixture = Fixture::new("portfolio-retry-classifier");
    let mut store = open_portfolio_store(fixture.path());
    store
        .record_portfolio_snapshot(portfolio_store_snapshot())
        .unwrap();

    let binding = portfolio_item_binding("source-tracker", 100);
    store.fail_next_portfolio_persist();
    assert!(store
        .record_portfolio_item_binding(binding.clone())
        .is_err());
    store
        .record_portfolio_item_binding(binding.clone())
        .unwrap();
    let mut conflicting_binding = binding;
    conflicting_binding["issue_url"] =
        serde_json::json!("https://github.com/berlinguyinca/autospec/issues/999");
    assert!(store
        .record_portfolio_item_binding(conflicting_binding)
        .is_err());

    for state in ["intent", "sent", "acknowledged"] {
        let payload = serde_json::json!({"field": "Status", "boundary": state});
        store.fail_next_portfolio_persist();
        assert!(store
            .transition_portfolio_operation("item:add:source-tracker", state, payload.clone(),)
            .is_err());
        store
            .transition_portfolio_operation("item:add:source-tracker", state, payload.clone())
            .unwrap();
        assert!(store
            .transition_portfolio_operation(
                "item:add:source-tracker",
                state,
                serde_json::json!({"field": "Different", "boundary": state}),
            )
            .is_err());
    }
}

#[test]
fn managed_project_store_high_watermark_stops_before_earliest_unresolved_operation() {
    let fixture = Fixture::new("portfolio-high-watermark");
    let mut store = open_portfolio_store(fixture.path());
    store
        .record_portfolio_snapshot(portfolio_store_snapshot())
        .unwrap();
    let payload = serde_json::json!({"field": "Status"});
    store
        .transition_portfolio_operation("item:add:a", "intent", payload.clone())
        .unwrap();
    for state in ["intent", "sent", "acknowledged"] {
        store
            .transition_portfolio_operation("item:add:b", state, payload.clone())
            .unwrap();
    }
    assert_eq!(
        store.portfolio_snapshot().unwrap()["projection_high_watermark"],
        1
    );
    for state in ["sent", "acknowledged"] {
        store
            .transition_portfolio_operation("item:add:a", state, payload.clone())
            .unwrap();
    }
    assert_eq!(
        store.portfolio_snapshot().unwrap()["projection_high_watermark"],
        7
    );
}

#[test]
fn managed_project_store_high_watermark_includes_pending_generic_projections() {
    let fixture = Fixture::new("portfolio-projection-high-watermark");
    let mut store = open_portfolio_store(fixture.path());
    store
        .record_portfolio_snapshot(portfolio_store_snapshot())
        .unwrap();
    store.enqueue_projection("portfolio:field:update").unwrap();
    let payload = serde_json::json!({"field": "Status"});
    for state in ["intent", "sent", "acknowledged"] {
        store
            .transition_portfolio_operation("item:add:a", state, payload.clone())
            .unwrap();
    }
    assert_eq!(
        store.portfolio_snapshot().unwrap()["projection_high_watermark"],
        1
    );
    store.ack_projection("portfolio:field:update").unwrap();
    assert_eq!(
        store.portfolio_snapshot().unwrap()["projection_high_watermark"],
        6
    );
}

#[test]
fn managed_project_store_mutable_open_rejects_snapshot_without_journal() {
    let fixture = Fixture::new("portfolio-missing-journal");
    let mut store = open_portfolio_store(fixture.path());
    store
        .record_portfolio_snapshot(portfolio_store_snapshot())
        .unwrap();
    drop(store);
    let events = fixture.portfolio_state_path("events.jsonl");
    fs::remove_file(&events).unwrap();

    assert!(ManagedProjectStore::open(fixture.path(), &portfolio_store_identity()).is_err());
    assert!(!events.exists());
}

#[test]
fn managed_project_store_rejects_cycles_duplicate_edges_and_forward_references() {
    let mut cycle = portfolio_store_snapshot();
    cycle["recovery_capsule"]["items"][0]["dependencies"] =
        serde_json::json!(["issue:portfolio-store"]);
    let mut duplicate = portfolio_store_snapshot();
    duplicate["recovery_capsule"]["items"][1]["dependencies"] =
        serde_json::json!(["source-tracker", "source-tracker"]);
    let mut forward_parent = portfolio_store_snapshot();
    forward_parent["recovery_capsule"]["items"][0]["local_parents"] =
        serde_json::json!(["issue:portfolio-store"]);
    let mut self_cycle = portfolio_store_snapshot();
    self_cycle["recovery_capsule"]["items"][0]["dependencies"] =
        serde_json::json!(["source-tracker"]);

    for (index, snapshot) in [cycle, duplicate, forward_parent, self_cycle]
        .into_iter()
        .enumerate()
    {
        let fixture = Fixture::new(&format!("portfolio-graph-{index}"));
        let mut store = open_portfolio_store(fixture.path());
        assert!(store.record_portfolio_snapshot(snapshot).is_err());
    }
}

#[test]
fn managed_project_store_freezes_item_role_and_completion_policy() {
    let fixture = Fixture::new("portfolio-item-policy");
    let mut store = open_portfolio_store(fixture.path());
    store
        .record_portfolio_snapshot(portfolio_store_snapshot())
        .unwrap();
    let mut wrong_role = portfolio_item_binding("source-tracker", 100);
    wrong_role["role"] = serde_json::json!("implementation");
    assert!(store.record_portfolio_item_binding(wrong_role).is_err());
    let mut wrong_policy = portfolio_item_binding("source-tracker", 100);
    wrong_policy["completion_policy"] = serde_json::json!("manual-close");
    assert!(store.record_portfolio_item_binding(wrong_policy).is_err());
}

#[test]
fn managed_project_store_rejects_invalid_role_policy_order_and_cardinality() {
    let mut invalid_role = portfolio_store_snapshot();
    invalid_role["recovery_capsule"]["items"][1]["role"] = serde_json::json!("manual");

    let mut invalid_policy = portfolio_store_snapshot();
    invalid_policy["recovery_capsule"]["items"][1]["completion_policy"] =
        serde_json::json!("manual-close");

    let mut incompatible = portfolio_store_snapshot();
    incompatible["recovery_capsule"]["items"][0]["completion_policy"] =
        serde_json::json!("merged-pr");

    let mut no_source = portfolio_store_snapshot();
    no_source["recovery_capsule"]["items"][0]["role"] = serde_json::json!("repo-tracker");

    let mut duplicate_source = portfolio_store_snapshot();
    duplicate_source["recovery_capsule"]["items"][1]["role"] = serde_json::json!("source-tracker");
    duplicate_source["recovery_capsule"]["items"][1]["completion_policy"] =
        serde_json::json!("closed-tracker");

    let mut no_audit = portfolio_store_snapshot();
    no_audit["recovery_capsule"]["items"][2]["role"] = serde_json::json!("prerequisite");
    no_audit["recovery_capsule"]["items"][2]["completion_policy"] =
        serde_json::json!("external-prerequisite");

    let mut duplicate_audit = portfolio_store_snapshot();
    duplicate_audit["recovery_capsule"]["items"][1]["role"] = serde_json::json!("audit");
    duplicate_audit["recovery_capsule"]["items"][1]["completion_policy"] =
        serde_json::json!("audit-receipt");

    let mut audit_not_last = portfolio_store_snapshot();
    audit_not_last["recovery_capsule"]["items"]
        .as_array_mut()
        .unwrap()
        .swap(1, 2);

    let mut rank_regression = portfolio_store_snapshot();
    let mut repo_tracker = rank_regression["recovery_capsule"]["items"][1].clone();
    repo_tracker["item_key"] = serde_json::json!("repo:autospec:tracker");
    repo_tracker["role"] = serde_json::json!("repo-tracker");
    repo_tracker["completion_policy"] = serde_json::json!("closed-tracker");
    rank_regression["recovery_capsule"]["items"]
        .as_array_mut()
        .unwrap()
        .insert(2, repo_tracker);

    let cases = [
        invalid_role,
        invalid_policy,
        incompatible,
        no_source,
        duplicate_source,
        no_audit,
        duplicate_audit,
        audit_not_last,
        rank_regression,
    ];
    for (index, snapshot) in cases.into_iter().enumerate() {
        let fixture = Fixture::new(&format!("portfolio-role-policy-{index}"));
        let mut store = open_portfolio_store(fixture.path());
        assert!(store.record_portfolio_snapshot(snapshot).is_err());
    }
}
