use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use autospec_core::runtime_env::{
    load_generation_token, read_json, write_json_atomic, ComposeExport, ComposeIsolation,
    ComposeOverride, ComposeOwnership, ComposePlan, ComposePolicy, EnvironmentIdentity,
    EnvironmentLifecycle, EnvironmentOwner, ExportProtocol, ExportValue, IsolationDiagnostic,
    OwnedVolume, ResolvedExport, ResourceInventory, RuntimeContext, RuntimeManifest, SessionRecord,
};

static NEXT_TEMP_REPO: AtomicUsize = AtomicUsize::new(0);

struct TempRepo {
    root: PathBuf,
}

impl TempRepo {
    fn with_files(files: &[(&str, &str)]) -> Self {
        let suffix = NEXT_TEMP_REPO.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "autospec-runtime-resources-{}-{suffix}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create temporary repository");
        for (relative, content) in files {
            let path = root.join(relative);
            std::fs::create_dir_all(path.parent().expect("fixture file has parent"))
                .expect("create fixture parent");
            std::fs::write(path, content).expect("write fixture file");
        }
        Self { root }
    }

    fn path(&self) -> &Path {
        &self.root
    }
}

impl Drop for TempRepo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn generation_token_prevents_path_reuse_from_adopting_state() {
    let repo = TempRepo::with_files(&[]);
    let first = EnvironmentIdentity::resolve(repo.path(), "local", Some("gen-a")).unwrap();
    let second = EnvironmentIdentity::resolve(repo.path(), "local", Some("gen-b")).unwrap();

    assert_ne!(first.environment_id, second.environment_id);
    assert_ne!(first.owner_key, second.owner_key);
}

#[test]
fn generation_identity_selects_the_authoritative_state_directory() {
    let repo = TempRepo::with_files(&[(
        ".autospec/runtime.yml",
        "version: 1\ndefault_mode: local\nmodes:\n  local:\n    command: true\n",
    )]);
    let manifest = RuntimeManifest::read_from_repo(repo.path()).unwrap();
    let first = EnvironmentIdentity::resolve(repo.path(), "local", Some("gen-a")).unwrap();
    let second = EnvironmentIdentity::resolve(repo.path(), "local", Some("gen-b")).unwrap();
    let state_root = repo.path().join("state");

    let first_context = RuntimeContext::new_with_identity(
        manifest.clone(),
        repo.path(),
        "local",
        &state_root,
        &first,
    )
    .unwrap();
    let second_context =
        RuntimeContext::new_with_identity(manifest, repo.path(), "local", &state_root, &second)
            .unwrap();

    assert_eq!(
        first_context.environment_dir.file_name().unwrap(),
        first.environment_id.as_str()
    );
    assert_eq!(
        second_context.environment_dir.file_name().unwrap(),
        second.environment_id.as_str()
    );
    assert_ne!(
        first_context.environment_dir,
        second_context.environment_dir
    );
}

#[test]
fn inventory_json_preserves_resource_ids_and_ports() {
    let inventory = ResourceInventory {
        schema_version: 1,
        environment_id: "env-a".to_string(),
        compose_project: Some("compose-a".to_string()),
        containers: vec!["container-a".to_string()],
        networks: vec!["network-a".to_string()],
        volumes: vec![OwnedVolume {
            logical_key: Some("database".to_string()),
            id: "volume-a".to_string(),
        }],
        exports: vec![ResolvedExport {
            env: "AUTOSPEC_PUBLIC_URL".to_string(),
            host: "127.0.0.1".to_string(),
            port: 49152,
        }],
        maven_local_prefix: Some(PathBuf::from("autospec/env-a")),
    };

    let encoded = serde_json::to_string(&inventory).unwrap();

    assert_eq!(
        serde_json::from_str::<ResourceInventory>(&encoded).unwrap(),
        inventory
    );
}

#[test]
fn legacy_compose_plan_json_defaults_new_shared_resource_fields() {
    let legacy = r#"{
        "isolation":"Managed",
        "files":["compose.yaml"],
        "project_name":"agent_legacy",
        "exports":[],
        "preserve_volumes":[]
    }"#;

    let plan =
        serde_json::from_str::<ComposePlan>(legacy).expect("schema-v1 plan remains readable");

    assert!(plan.shared_networks.is_empty());
    assert!(plan.shared_volumes.is_empty());
}

#[test]
fn compose_override_publishes_only_declared_loopback_targets_with_ownership() {
    let repo = TempRepo::with_files(&[("compose.yaml", "services: {}\n")]);
    let plan = compose_policy_plan(repo.path());
    let model = serde_json::json!({
        "services": {"web": {"image": "example.invalid/web"}},
        "networks": {"private": {}},
        "volumes": {"cache": {}}
    });
    let ownership = ComposeOwnership {
        environment_id: "env-a".to_string(),
        owner_key: "owner-a".to_string(),
        plan_digest: "digest-a".to_string(),
    };

    let rendered = ComposeOverride::render(&plan, &model, &ownership).unwrap();

    assert_eq!(
        rendered,
        "services:\n  web:\n    labels:\n      com.autospec.environment-id: 'env-a'\n      com.autospec.owner-key: 'owner-a'\n      com.autospec.plan-digest: 'digest-a'\n    ports:\n      - target: 8080\n        published: '0'\n        host_ip: '127.0.0.1'\n        protocol: tcp\nnetworks:\n  private:\n    labels:\n      com.autospec.environment-id: 'env-a'\n      com.autospec.owner-key: 'owner-a'\n      com.autospec.plan-digest: 'digest-a'\nvolumes:\n  cache:\n    labels:\n      com.autospec.environment-id: 'env-a'\n      com.autospec.owner-key: 'owner-a'\n      com.autospec.plan-digest: 'digest-a'\n"
    );
}

#[test]
fn compose_inventory_excludes_preserved_logical_volumes_from_deletion() {
    let inventory = ResourceInventory {
        volumes: vec![
            OwnedVolume {
                logical_key: Some("db-data".to_string()),
                id: "volume-db".to_string(),
            },
            OwnedVolume {
                logical_key: Some("cache-data".to_string()),
                id: "volume-cache".to_string(),
            },
            OwnedVolume {
                logical_key: None,
                id: "volume-anonymous".to_string(),
            },
        ],
        ..ResourceInventory::default()
    };

    assert_eq!(
        inventory.deletable_volumes(&["db-data".to_string()]),
        vec!["volume-cache".to_string(), "volume-anonymous".to_string()]
    );
}

#[test]
fn resolved_compose_exports_render_by_declared_value_type() {
    let resolved = ResolvedExport {
        env: "SERVICE".to_string(),
        host: "127.0.0.1".to_string(),
        port: 49152,
    };

    for (protocol, value, expected) in [
        (
            ExportProtocol::Http,
            ExportValue::Url,
            "http://127.0.0.1:49152",
        ),
        (
            ExportProtocol::Https,
            ExportValue::Url,
            "https://127.0.0.1:49152",
        ),
        (ExportProtocol::Tcp, ExportValue::Port, "49152"),
        (ExportProtocol::Udp, ExportValue::Port, "49152"),
        (
            ExportProtocol::Tcp,
            ExportValue::HostPort,
            "127.0.0.1:49152",
        ),
    ] {
        let export = ComposeExport {
            service: "web".to_string(),
            target: 8080,
            protocol,
            env: "SERVICE".to_string(),
            value,
        };
        assert_eq!(resolved.render(&export).unwrap(), expected);
    }
}

#[test]
fn compose_canonical_url_requires_explicit_or_unique_http_export() {
    let mut plan = compose_policy_plan(Path::new("/repo"));
    assert!(plan.canonical_url_export_index().is_err());

    plan.exports = vec![ComposeExport {
        service: "web".to_string(),
        target: 8080,
        protocol: ExportProtocol::Http,
        env: "WEB_URL".to_string(),
        value: ExportValue::Url,
    }];
    assert_eq!(plan.canonical_url_export_index().unwrap(), 0);

    plan.exports.push(ComposeExport {
        service: "admin".to_string(),
        target: 8081,
        protocol: ExportProtocol::Https,
        env: "ADMIN_URL".to_string(),
        value: ExportValue::Url,
    });
    assert!(plan.canonical_url_export_index().is_err());

    plan.exports[1].env = "AUTOSPEC_PUBLIC_URL".to_string();
    assert_eq!(plan.canonical_url_export_index().unwrap(), 1);
}

#[test]
fn atomic_json_state_round_trips_without_evaluating_shell_text() {
    let repo = TempRepo::with_files(&[]);
    let marker = repo.path().join("shell-evaluated");
    let path = repo.path().join("state/inventory.json");
    let inventory = ResourceInventory {
        schema_version: 1,
        environment_id: "env-a".to_string(),
        compose_project: Some(format!("$(touch {})", marker.display())),
        ..ResourceInventory::default()
    };

    write_json_atomic(&path, &inventory).expect("inventory persists");

    assert_eq!(read_json::<ResourceInventory>(&path).unwrap(), inventory);
    assert!(!marker.exists(), "JSON state was evaluated as shell source");
}

#[test]
fn owner_and_session_contracts_round_trip_with_schema_one() {
    let repo = TempRepo::with_files(&[]);
    let identity = EnvironmentIdentity::resolve(repo.path(), "local", Some("generation")).unwrap();
    let owner = EnvironmentOwner {
        schema_version: 1,
        identity,
        host: "host-a".to_string(),
        created_at_unix_ms: 10,
        manifest_digest: "digest-a".to_string(),
        lifecycle: EnvironmentLifecycle::Planned,
    };
    let session = SessionRecord {
        schema_version: 1,
        session_id: "session-a".to_string(),
        pid: 42,
        process_start: "start-a".to_string(),
        harness: "codex".to_string(),
        host: "host-a".to_string(),
        started_at_unix_ms: 11,
        heartbeat_at_unix_ms: 12,
    };

    assert_eq!(
        serde_json::from_str::<EnvironmentOwner>(&serde_json::to_string(&owner).unwrap()).unwrap(),
        owner
    );
    assert_eq!(
        serde_json::from_str::<SessionRecord>(&serde_json::to_string(&session).unwrap()).unwrap(),
        session
    );
}

#[test]
fn isolation_diagnostic_has_the_exact_schema_one_json_shape() {
    let diagnostic = IsolationDiagnostic {
        schema_version: 1,
        code: "RUNTIME_COLLISION".to_string(),
        environment_id: "env-a".to_string(),
        resource: "compose.web.ports[0]".to_string(),
        evidence: "fixed host port 8080".to_string(),
        recovery_command: "autospec runtime env normalize".to_string(),
    };
    let expected = serde_json::json!({
        "schema_version": 1,
        "code": "RUNTIME_COLLISION",
        "environment_id": "env-a",
        "resource": "compose.web.ports[0]",
        "evidence": "fixed host port 8080",
        "recovery_command": "autospec runtime env normalize",
    });

    let encoded = serde_json::to_value(&diagnostic).unwrap();

    assert_eq!(encoded, expected);
    assert_eq!(
        serde_json::from_value::<IsolationDiagnostic>(encoded).unwrap(),
        diagnostic
    );
}

fn compose_policy_plan(repo: &Path) -> ComposePlan {
    ComposePlan {
        isolation: ComposeIsolation::Managed,
        files: vec![repo.join("compose.yaml")],
        project_name: "agent_env-a".to_string(),
        exports: vec![ComposeExport {
            service: "web".to_string(),
            target: 8080,
            protocol: ExportProtocol::Tcp,
            env: "WEB_PORT".to_string(),
            value: ExportValue::Port,
        }],
        preserve_volumes: Vec::new(),
        shared_networks: vec!["company-vpn".to_string()],
        shared_volumes: vec!["shared-cache".to_string()],
    }
}

fn compose_policy_diagnostics(
    repo: &TempRepo,
    model: serde_json::Value,
) -> Vec<IsolationDiagnostic> {
    ComposePolicy::evaluate(&model, &compose_policy_plan(repo.path()))
}

#[test]
fn compose_policy_fixed_port_reports_exact_path_and_value() {
    let repo = TempRepo::with_files(&[("compose.yaml", "services: {}\n")]);
    let diagnostics = compose_policy_diagnostics(
        &repo,
        serde_json::json!({"services":{"web":{"ports":[{"target":8080,"published":"49152","protocol":"tcp"}]}}}),
    );

    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code, "COMPOSE_FIXED_PORT");
    assert_eq!(diagnostics[0].resource, "services.web.ports[0].published");
    assert_eq!(diagnostics[0].evidence, "49152");
    assert_eq!(diagnostics[0].environment_id, "env-a");
    assert_eq!(
        diagnostics[0].recovery_command,
        format!(
            "autospec runtime env normalize-compose --repo '{}' --check",
            repo.path().display()
        )
    );
}

#[test]
fn compose_policy_explicit_context_keeps_nested_files_at_the_worktree_boundary() {
    let repo = TempRepo::with_files(&[("deploy/compose.yaml", "services: {}\n")]);
    let mut plan = compose_policy_plan(repo.path());
    plan.files = vec![repo.path().join("deploy/compose.yaml")];
    let diagnostics = ComposePolicy::evaluate_in_context(
        &serde_json::json!({"services":{"web":{"container_name":"web"}}}),
        &plan,
        "exact-environment",
        repo.path(),
    );

    assert_eq!(diagnostics[0].environment_id, "exact-environment");
    assert_eq!(
        diagnostics[0].recovery_command,
        format!(
            "autospec runtime env normalize-compose --repo '{}' --check",
            repo.path().display()
        )
    );
}

#[test]
fn compose_policy_rejects_undeclared_targets_and_protocols() {
    let repo = TempRepo::with_files(&[("compose.yaml", "services: {}\n")]);
    let diagnostics = compose_policy_diagnostics(
        &repo,
        serde_json::json!({"services":{"web":{"ports":[
            {"target":8080,"protocol":"sctp"},
            {"target":9090,"protocol":"tcp"}
        ]}}}),
    );

    assert_eq!(
        diagnostics
            .iter()
            .map(|item| (item.code.as_str(), item.resource.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("COMPOSE_UNDECLARED_PORT", "services.web.ports[0].protocol"),
            ("COMPOSE_UNDECLARED_PORT", "services.web.ports[1].target")
        ]
    );
}

#[test]
fn compose_policy_rejects_global_service_identity() {
    let repo = TempRepo::with_files(&[("compose.yaml", "services: {}\n")]);
    let diagnostics = compose_policy_diagnostics(
        &repo,
        serde_json::json!({"services":{"web":{"container_name":"fixed-web","network_mode":"host"}}}),
    );

    assert_eq!(
        diagnostics
            .iter()
            .map(|item| (item.code.as_str(), item.resource.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("COMPOSE_CONTAINER_NAME", "services.web.container_name"),
            ("COMPOSE_HOST_NETWORK", "services.web.network_mode")
        ]
    );
}

#[test]
fn compose_policy_rejects_global_names_and_fixed_addresses() {
    let repo = TempRepo::with_files(&[("compose.yaml", "services: {}\n")]);
    let diagnostics = compose_policy_diagnostics(
        &repo,
        serde_json::json!({
            "services":{"web":{"networks":{"private":{"ipv4_address":"10.0.0.8"}}}},
            "networks":{"private":{"name":"global-network"}},
            "volumes":{"data":{"name":"global-volume"}}
        }),
    );

    assert_eq!(
        diagnostics
            .iter()
            .map(|item| (item.code.as_str(), item.resource.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("COMPOSE_GLOBAL_NAME", "networks.private.name"),
            (
                "COMPOSE_FIXED_ADDRESS",
                "services.web.networks.private.ipv4_address"
            ),
            ("COMPOSE_GLOBAL_NAME", "volumes.data.name")
        ]
    );
}

#[test]
fn compose_policy_allows_only_exact_declared_external_keys() {
    let repo = TempRepo::with_files(&[("compose.yaml", "services: {}\n")]);
    let diagnostics = compose_policy_diagnostics(
        &repo,
        serde_json::json!({
            "networks":{
                "company-vpn":{"external":true,"name":"corp-vpn"},
                "company":{"external":true}
            },
            "volumes":{
                "shared-cache":{"external":true,"name":"corp-cache"},
                "cache":{"external":true}
            }
        }),
    );

    assert_eq!(
        diagnostics
            .iter()
            .map(|item| (item.code.as_str(), item.resource.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("COMPOSE_EXTERNAL_UNDECLARED", "networks.company.external"),
            ("COMPOSE_EXTERNAL_UNDECLARED", "volumes.cache.external")
        ]
    );
}

#[test]
fn compose_policy_undeclared_external_reports_only_the_external_rule() {
    let repo = TempRepo::with_files(&[("compose.yaml", "services: {}\n")]);
    let diagnostics = compose_policy_diagnostics(
        &repo,
        serde_json::json!({
            "networks":{"company":{"external":true,"name":"shared-company-network"}}
        }),
    );

    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code, "COMPOSE_EXTERNAL_UNDECLARED");
    assert_eq!(diagnostics[0].resource, "networks.company.external");
}

#[test]
fn compose_policy_shell_quotes_recovery_repository() {
    let repo = TempRepo::with_files(&[("space's dir/compose.yaml", "services: {}\n")]);
    let nested_repo = repo.path().join("space's dir");
    let mut plan = compose_policy_plan(&nested_repo);
    plan.files = vec![nested_repo.join("compose.yaml")];
    let diagnostics = ComposePolicy::evaluate_in_context(
        &serde_json::json!({"services":{"web":{"container_name":"web"}}}),
        &plan,
        "env-a",
        &nested_repo,
    );

    assert_eq!(
        diagnostics[0].recovery_command,
        format!(
            "autospec runtime env normalize-compose --repo '{}' --check",
            nested_repo.display().to_string().replace('\'', "'\\''")
        )
    );
}

#[test]
fn compose_policy_allows_read_only_and_contained_writable_binds() {
    let repo = TempRepo::with_files(&[
        ("compose.yaml", "services: {}\n"),
        ("inside/data.txt", "data\n"),
    ]);
    let outside = repo.path().parent().unwrap().join("outside-bind");
    std::fs::create_dir_all(&outside).unwrap();
    let model = serde_json::json!({
        "services":{"web":{"volumes":[
            {"type":"bind","source":outside,"target":"/readonly","read_only":true},
            {"type":"bind","source":repo.path().join("inside"),"target":"/inside"}
        ]}},
        "networks":{"private":{"name":"agent_env-a_private"}},
        "volumes":{"data":{"name":"agent_env-a_data"}}
    });

    assert!(compose_policy_diagnostics(&repo, model).is_empty());
    std::fs::remove_dir_all(outside).unwrap();
}

#[test]
fn compose_policy_rejects_writable_bind_outside_worktree() {
    let repo = TempRepo::with_files(&[("compose.yaml", "services: {}\n")]);
    let outside = repo.path().parent().unwrap().join("outside-writable-bind");
    std::fs::create_dir_all(&outside).unwrap();
    let model = serde_json::json!({"services":{"web":{"volumes":[
        {"type":"bind","source":outside,"target":"/outside"}
    ]}}});
    let diagnostics = compose_policy_diagnostics(&repo, model);

    assert_eq!(diagnostics.len(), 1);
    assert_eq!(
        diagnostics[0].code,
        "COMPOSE_WRITABLE_BIND_OUTSIDE_WORKTREE"
    );
    assert_eq!(diagnostics[0].resource, "services.web.volumes[0].source");
    std::fs::remove_dir_all(outside).unwrap();
}

#[test]
fn compose_policy_returns_all_rule_ids_in_resource_path_order() {
    let repo = TempRepo::with_files(&[("compose.yaml", "services: {}\n")]);
    let outside = repo.path().parent().unwrap().join("outside-all-rules");
    std::fs::create_dir_all(&outside).unwrap();
    let diagnostics = compose_policy_diagnostics(
        &repo,
        serde_json::json!({
            "services":{"web":{
                "container_name":"web",
                "network_mode":"host",
                "networks":{"private":{"ipv6_address":"fd00::8"}},
                "ports":[{"target":9090,"published":8080,"protocol":"tcp"}],
                "volumes":[{"type":"bind","source":outside,"target":"/outside"}]
            }},
            "networks":{"private":{"external":true,"name":"private-global"}},
            "volumes":{"data":{"name":"global-data"}}
        }),
    );
    let resources = diagnostics
        .iter()
        .map(|item| item.resource.as_str())
        .collect::<Vec<_>>();
    let mut sorted = resources.clone();
    sorted.sort_unstable();

    assert_eq!(resources, sorted);
    assert_eq!(
        diagnostics
            .iter()
            .map(|item| item.code.as_str())
            .collect::<std::collections::BTreeSet<_>>(),
        std::collections::BTreeSet::from([
            "COMPOSE_CONTAINER_NAME",
            "COMPOSE_EXTERNAL_UNDECLARED",
            "COMPOSE_FIXED_ADDRESS",
            "COMPOSE_FIXED_PORT",
            "COMPOSE_GLOBAL_NAME",
            "COMPOSE_HOST_NETWORK",
            "COMPOSE_UNDECLARED_PORT",
            "COMPOSE_WRITABLE_BIND_OUTSIDE_WORKTREE",
        ])
    );
    std::fs::remove_dir_all(outside).unwrap();
}

#[test]
fn linked_worktree_identity_uses_git_root_and_worktree_specific_generation() {
    let repository = TempRepo::with_files(&[("tracked.txt", "tracked\n")]);
    run_git(repository.path(), &["init"]);
    run_git(
        repository.path(),
        &["config", "user.email", "autospec@example.test"],
    );
    run_git(repository.path(), &["config", "user.name", "Autospec Test"]);
    run_git(repository.path(), &["add", "tracked.txt"]);
    run_git(repository.path(), &["commit", "-m", "fixture"]);

    let worktree = repository.path().join("linked");
    run_git(
        repository.path(),
        &[
            "worktree",
            "add",
            "--detach",
            worktree.to_str().expect("worktree path is UTF-8"),
        ],
    );
    let nested = worktree.join("nested/directory");
    std::fs::create_dir_all(&nested).expect("create nested worktree directory");

    let generation = load_generation_token(&nested)
        .expect("generation resolves")
        .expect("linked worktree has a generation token");
    let identity = EnvironmentIdentity::resolve(&nested, "local", Some(&generation))
        .expect("identity resolves");
    let git_dir = git_output(
        &nested,
        &["rev-parse", "--path-format=absolute", "--git-dir"],
    );

    assert_eq!(
        identity.canonical_repo,
        std::fs::canonicalize(&worktree).unwrap()
    );
    assert_eq!(
        std::fs::read_to_string(PathBuf::from(git_dir).join("autospec-runtime-generation"))
            .unwrap(),
        generation
    );
}

#[test]
fn generation_loader_recovers_an_interrupted_empty_initialization() {
    let repository = TempRepo::with_files(&[("tracked.txt", "tracked\n")]);
    run_git(repository.path(), &["init"]);
    let git_dir = git_output(
        repository.path(),
        &["rev-parse", "--path-format=absolute", "--git-dir"],
    );
    let generation_path = PathBuf::from(git_dir).join("autospec-runtime-generation");
    std::fs::write(&generation_path, "").expect("seed interrupted generation creation");

    let token = load_generation_token(repository.path())
        .expect("generation loader recovers")
        .expect("Git repository gets a generation");

    assert_eq!(token.len(), 32);
    assert_eq!(std::fs::read_to_string(generation_path).unwrap(), token);
}

#[test]
fn generation_loader_replaces_a_partial_non_empty_authoritative_token() {
    let repository = TempRepo::with_files(&[("tracked.txt", "tracked\n")]);
    run_git(repository.path(), &["init"]);
    let git_dir = PathBuf::from(git_output(
        repository.path(),
        &["rev-parse", "--path-format=absolute", "--git-dir"],
    ));
    let generation_path = git_dir.join("autospec-runtime-generation");
    std::fs::write(&generation_path, "partial-token").expect("seed partial generation token");

    let token = load_generation_token(repository.path())
        .expect("generation loader recovers")
        .expect("Git repository gets a generation");

    assert_eq!(token.len(), 32);
    assert_eq!(std::fs::read_to_string(generation_path).unwrap(), token);
    assert!(git_dir.join("autospec-runtime-generation.lock").is_file());
}

#[test]
fn git_rev_parse_operational_failure_is_not_treated_as_non_git() {
    let repository = TempRepo::with_files(&[("not-a-directory", "file\n")]);
    let invalid_working_directory = repository.path().join("not-a-directory");

    let error = EnvironmentIdentity::resolve(&invalid_working_directory, "local", None)
        .expect_err("git operational failure is surfaced");

    let message = error.to_string();
    assert!(message.contains("git rev-parse failed for"));
    assert!(message.contains(&invalid_working_directory.display().to_string()));
    assert!(message.contains("(--show-toplevel)"));
}

#[test]
fn git_operational_error_with_non_repository_words_later_is_not_non_git() {
    let repository = TempRepo::with_files(&[("not a git repository", "file\n")]);
    let invalid_working_directory = repository.path().join("not a git repository");

    let error = EnvironmentIdentity::resolve(&invalid_working_directory, "local", None)
        .expect_err("later stderr words do not trigger non-Git fallback");

    let message = error.to_string();
    assert!(message.contains("git rev-parse failed for"));
    assert!(message.contains(&invalid_working_directory.display().to_string()));
    assert!(message.contains("(--show-toplevel)"));
}

fn run_git(directory: &Path, arguments: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(directory)
        .args(arguments)
        .output()
        .expect("git starts");
    assert!(
        output.status.success(),
        "git {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_output(directory: &Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(directory)
        .args(arguments)
        .output()
        .expect("git starts");
    assert!(output.status.success());
    String::from_utf8(output.stdout)
        .expect("git output is UTF-8")
        .trim()
        .to_string()
}
