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
