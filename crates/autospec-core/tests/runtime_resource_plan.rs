use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use autospec_core::runtime_env::{
    ComposeIsolation, EnvironmentIdentity, MavenIsolation, RuntimeManifest,
};

static NEXT_TEMP_REPO: AtomicUsize = AtomicUsize::new(0);

struct TempRepo {
    root: PathBuf,
}

impl TempRepo {
    fn with_files(files: &[(&str, &str)]) -> Self {
        let suffix = NEXT_TEMP_REPO.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "autospec-runtime-resource-plan-{}-{suffix}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create temporary repository");
        let root = std::fs::canonicalize(root).expect("canonicalize temporary repository");
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

fn fixture_identity(repo: &Path) -> EnvironmentIdentity {
    EnvironmentIdentity::resolve(repo, "local", Some("fixture-generation"))
        .expect("fixture identity resolves")
}

#[test]
fn resource_detection_finds_pom_and_standard_compose_without_a_manifest() {
    let repo = TempRepo::with_files(&[
        ("pom.xml", "<project/>"),
        ("compose.yaml", "services: {}\n"),
    ]);

    let plan = RuntimeManifest::resource_plan_for_repo(repo.path(), &fixture_identity(repo.path()))
        .expect("detected resources produce a plan");

    assert!(plan.maven.is_some());
    assert_eq!(plan.maven.unwrap().isolation, MavenIsolation::SplitLocal);
    let compose = plan.compose.expect("Compose is detected");
    assert_eq!(compose.isolation, ComposeIsolation::Managed);
    assert_eq!(compose.files, vec![repo.path().join("compose.yaml")]);
}

#[test]
fn standard_compose_detection_stops_at_the_first_supported_filename() {
    let repo = TempRepo::with_files(&[
        ("compose.yml", "services: {}\n"),
        ("docker-compose.yaml", "services: {}\n"),
    ]);

    let plan = RuntimeManifest::resource_plan_for_repo(repo.path(), &fixture_identity(repo.path()))
        .expect("Compose plan builds");

    assert_eq!(
        plan.compose.expect("Compose is detected").files,
        vec![repo.path().join("compose.yml")]
    );
}

#[test]
fn v1_command_that_starts_compose_cannot_compete_with_the_broker() {
    let repo = TempRepo::with_files(&[
        (
            ".autospec/runtime.yml",
            "version: 1\nmodes:\n  local:\n    command: docker compose up\n",
        ),
        ("compose.yaml", "services: {}\n"),
    ]);

    let error =
        RuntimeManifest::resource_plan_for_repo(repo.path(), &fixture_identity(repo.path()))
            .expect_err("dual Compose authority is rejected");

    assert!(error.to_string().contains("RUNTIME_DUAL_COMPOSE_AUTHORITY"));
}

#[test]
fn compose_only_plan_does_not_require_a_mode_command() {
    let repo = TempRepo::with_files(&[("compose.yaml", "services: {}\n")]);

    assert!(
        RuntimeManifest::resource_plan_for_repo(repo.path(), &fixture_identity(repo.path()))
            .is_ok()
    );
}

#[test]
fn empty_plan_without_a_mode_command_keeps_the_existing_error() {
    let repo = TempRepo::with_files(&[]);

    let error =
        RuntimeManifest::resource_plan_for_repo(repo.path(), &fixture_identity(repo.path()))
            .expect_err("empty plan still requires a mode command");

    assert!(error.to_string().contains("command"));
}

#[test]
fn explicit_compose_files_are_resolved_in_declared_order() {
    let repo = TempRepo::with_files(&[
        (
            ".autospec/runtime.yml",
            "version: 2\nresources:\n  compose:\n    files:\n      - compose.yaml\n      - compose.override.yaml\n",
        ),
        ("compose.yaml", "services: {}\n"),
        ("compose.override.yaml", "services: {}\n"),
    ]);

    let plan = RuntimeManifest::resource_plan_for_repo(repo.path(), &fixture_identity(repo.path()))
        .expect("explicit Compose files build a plan");

    assert_eq!(
        plan.compose.expect("Compose plan").files,
        vec![
            repo.path().join("compose.yaml"),
            repo.path().join("compose.override.yaml")
        ]
    );
}

#[test]
fn shared_resources_are_carried_into_the_plan_digest_and_json() {
    let repo = TempRepo::with_files(&[
        (
            ".autospec/runtime.yml",
            "version: 2\nresources:\n  compose:\n    files: [compose.yaml]\n    shared_resources:\n      networks: [company-vpn]\n      volumes: [maven-cache]\n",
        ),
        ("compose.yaml", "services: {}\n"),
    ]);

    let plan = RuntimeManifest::resource_plan_for_repo(repo.path(), &fixture_identity(repo.path()))
        .expect("shared resources produce a plan");
    let compose = plan.compose.as_ref().expect("Compose plan exists");
    assert_eq!(compose.shared_networks, ["company-vpn"]);
    assert_eq!(compose.shared_volumes, ["maven-cache"]);
    let json = serde_json::to_string(&plan).unwrap();
    assert_eq!(
        serde_json::from_str::<autospec_core::runtime_env::ResourcePlan>(&json).unwrap(),
        plan
    );
    let mut changed_compose = plan.compose.clone().unwrap();
    changed_compose.shared_volumes.push("other-cache".into());
    let changed = autospec_core::runtime_env::ResourcePlan::new(
        plan.identity.clone(),
        plan.maven.clone(),
        Some(changed_compose),
    )
    .unwrap();
    assert_ne!(changed.digest, plan.digest);
}

#[test]
fn explicit_compose_files_must_be_regular_contained_and_canonically_unique() {
    let repo = TempRepo::with_files(&[
        ("compose.yaml", "services: {}\n"),
        ("configs/placeholder", "not compose\n"),
    ]);
    let outside = repo.path().with_extension("outside-compose.yaml");
    std::fs::write(&outside, "services: {}\n").expect("write outside Compose file");
    let parent_escape = format!("../{}", outside.file_name().unwrap().to_string_lossy());

    for (files, expected) in [
        (
            vec![outside.display().to_string()],
            "outside the repository",
        ),
        (vec![parent_escape], "outside the repository"),
        (vec!["configs".to_string()], "regular file"),
        (vec!["missing.yaml".to_string()], "does not exist"),
        (
            vec!["compose.yaml".to_string(), "./compose.yaml".to_string()],
            "duplicate Compose file",
        ),
    ] {
        let list = files
            .iter()
            .map(|file| format!("      - {file}\n"))
            .collect::<String>();
        let manifest = format!("version: 2\nresources:\n  compose:\n    files:\n{list}");
        std::fs::create_dir_all(repo.path().join(".autospec")).unwrap();
        std::fs::write(repo.path().join(".autospec/runtime.yml"), manifest).unwrap();
        let error =
            RuntimeManifest::resource_plan_for_repo(repo.path(), &fixture_identity(repo.path()))
                .expect_err("unsafe Compose path is rejected");
        assert!(
            error.to_string().contains(expected),
            "expected {expected:?} in {error:?}"
        );
    }
    let _ = std::fs::remove_file(outside);
}

#[cfg(unix)]
#[test]
fn explicit_compose_symlink_cannot_escape_the_repository() {
    use std::os::unix::fs::symlink;

    let repo = TempRepo::with_files(&[]);
    let outside = repo.path().with_extension("symlink-target.yaml");
    std::fs::write(&outside, "services: {}\n").unwrap();
    symlink(&outside, repo.path().join("compose.yaml")).unwrap();
    std::fs::create_dir_all(repo.path().join(".autospec")).unwrap();
    std::fs::write(
        repo.path().join(".autospec/runtime.yml"),
        "version: 2\nresources:\n  compose:\n    files: [compose.yaml]\n",
    )
    .unwrap();

    let error =
        RuntimeManifest::resource_plan_for_repo(repo.path(), &fixture_identity(repo.path()))
            .expect_err("symlink escape is rejected");
    assert!(error.to_string().contains("outside the repository"));
    let _ = std::fs::remove_file(outside);
}

#[test]
fn compose_command_detection_uses_shell_command_positions() {
    let repo = TempRepo::with_files(&[("compose.yaml", "services: {}\n")]);
    std::fs::create_dir_all(repo.path().join(".autospec")).unwrap();

    for command in [
        "printf 'docker compose up'",
        "printf docker compose up # docker compose up",
    ] {
        std::fs::write(
            repo.path().join(".autospec/runtime.yml"),
            format!("version: 1\nmodes:\n  local:\n    command: {command}\n"),
        )
        .unwrap();
        RuntimeManifest::resource_plan_for_repo(repo.path(), &fixture_identity(repo.path()))
            .expect("quoted and comment data do not claim Compose authority");
    }

    let mut missed_authority = Vec::new();
    for command in [
        "printf ready; docker compose up",
        "printf ready && /usr/bin/docker-compose up",
        "docker \\\n      compose up",
        "sh -c 'docker compose up'",
        "sh -ec 'docker compose up'",
        "exec docker compose up",
        "env FOO=bar docker compose up",
        "env -i docker compose up",
        "sudo docker compose up",
        "sudo -u root docker compose up",
        concat!("docker --con", "text local compose up"),
        "docker -c local compose up",
        ">log docker compose up",
        "2>log docker compose up",
        "if docker compose up; then printf ready; fi",
        "if false; then :; else docker compose up; fi",
    ] {
        std::fs::write(
            repo.path().join(".autospec/runtime.yml"),
            format!("version: 1\nmodes:\n  local:\n    command: {command}\n"),
        )
        .unwrap();
        match RuntimeManifest::resource_plan_for_repo(repo.path(), &fixture_identity(repo.path())) {
            Ok(_) => missed_authority.push(command),
            Err(error) => assert!(error.to_string().contains("RUNTIME_DUAL_COMPOSE_AUTHORITY")),
        }
    }
    assert!(
        missed_authority.is_empty(),
        "command-position Compose authority was missed for {missed_authority:?}"
    );

    std::fs::write(
        repo.path().join(".autospec/runtime.yml"),
        "version: 1\nmodes:\n  local:\n    command: $COMPOSE_RUNNER up\n",
    )
    .unwrap();
    let error =
        RuntimeManifest::resource_plan_for_repo(repo.path(), &fixture_identity(repo.path()))
            .expect_err("ambiguous command-position expansion fails closed");
    assert!(error.to_string().contains("AMBIGUOUS_COMPOSE_AUTHORITY"));
}

#[test]
fn off_only_resources_do_not_replace_a_missing_mode_command() {
    let repo = TempRepo::with_files(&[
        (
            ".autospec/runtime.yml",
            "version: 2\nresources:\n  maven:\n    isolation: off\n  compose:\n    isolation: off\n",
        ),
        ("pom.xml", "<project/>"),
        ("compose.yaml", "services: {}\n"),
    ]);

    let error =
        RuntimeManifest::resource_plan_for_repo(repo.path(), &fixture_identity(repo.path()))
            .expect_err("off-only plan still requires a command");
    assert!(error.to_string().contains("selected mode has no command"));
}

#[test]
fn maven_directory_detection_ignores_a_plain_dot_mvn_file() {
    let repo = TempRepo::with_files(&[(".mvn", "not a directory\n")]);

    let error =
        RuntimeManifest::resource_plan_for_repo(repo.path(), &fixture_identity(repo.path()))
            .expect_err("plain .mvn does not create a Maven plan");
    assert!(error.to_string().contains("resource plan is empty"));
}

#[test]
fn v2_resource_grammar_rejects_ambiguous_or_unknown_values() {
    for (source, expected) in [
        (
            "version: 2\nresources:\n  unknown: {}\n",
            "unknown runtime resource key",
        ),
        (
            "version: 2\nresources:\n  compose:\n    files: [compose.yaml, compose.yaml]\n",
            "duplicate Compose file",
        ),
        (
            "version: 2\nresources:\n  compose:\n    exports:\n      - service: one\n        target: 80\n        protocol: http\n        env: SERVICE_URL\n      - service: two\n        target: 81\n        protocol: http\n        env: SERVICE_URL\n",
            "duplicate Compose export environment name",
        ),
        (
            "version: 2\nresources:\n  compose:\n    exports:\n      - service: web\n        target: 80\n        protocol: smtp\n        env: SERVICE_URL\n",
            "unsupported Compose export protocol",
        ),
        (
            "version: 2\nresources:\n  compose:\n    preserve_volumes: [postgres/data]\n",
            "invalid preserved Compose volume key",
        ),
        (
            "version: 2\nresources:\n  maven:\n    isolation: managed\n",
            "unsupported Maven isolation",
        ),
        (
            "version: 2\nresources:\n  compose:\n    isolation: managed\n",
            "unsupported Compose isolation",
        ),
    ] {
        let error = RuntimeManifest::parse(source).expect_err("invalid v2 grammar is rejected");
        assert!(
            error.to_string().contains(expected),
            "expected {expected:?} in {error:?}"
        );
    }
}
