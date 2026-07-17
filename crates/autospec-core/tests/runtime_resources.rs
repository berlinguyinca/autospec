use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use autospec_core::runtime_env::{
    load_generation_token, read_json, write_json_atomic, EnvironmentIdentity, EnvironmentLifecycle,
    EnvironmentOwner, IsolationDiagnostic, OwnedVolume, ResolvedExport, ResourceInventory,
    SessionRecord,
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
