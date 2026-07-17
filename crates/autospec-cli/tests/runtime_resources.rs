use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use autospec_core::runtime_env::{
    ComposeIsolation, ComposePlan, EnvironmentIdentity, EnvironmentLifecycle, EnvironmentOwner,
    MavenIsolation, MavenPlan, ResourcePlan,
};

static NEXT_FIXTURE: AtomicUsize = AtomicUsize::new(0);

struct RuntimeFixture {
    root: PathBuf,
    state_root: PathBuf,
}

impl RuntimeFixture {
    fn empty() -> Self {
        let suffix = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "autospec-runtime-resource-cli-{}-{suffix}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create fixture directory");
        let state_root = root.join("state");
        Self { root, state_root }
    }

    fn with_manifest(manifest: &str) -> Self {
        let fixture = Self::empty();
        let root = fixture.root.clone();
        std::fs::create_dir_all(root.join(".autospec")).expect("create fixture directory");
        std::fs::write(root.join(".autospec/runtime.yml"), manifest).expect("write manifest");
        fixture
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_autospec"));
        command.env("AGENT_ENV_STATE_ROOT", &self.state_root);
        command
    }

    #[cfg(unix)]
    fn install_fake_docker(&self, model: &serde_json::Value) -> (PathBuf, PathBuf) {
        let bin = self.root.join("fake-bin");
        std::fs::create_dir_all(&bin).expect("create fake Docker directory");
        let docker = bin.join("docker");
        std::fs::write(
            &docker,
            "#!/bin/sh\nprintf '%s\\n' \"$@\" >> \"$FAKE_DOCKER_LOG\"\nif [ \"${FAKE_DOCKER_EXIT:-0}\" -ne 0 ]; then\n  printf 'compose config failed\\n' >&2\n  exit \"$FAKE_DOCKER_EXIT\"\nfi\ncat \"$FAKE_DOCKER_MODEL\"\n",
        )
        .expect("write fake Docker");
        let mut permissions = std::fs::metadata(&docker).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&docker, permissions).unwrap();
        let model_path = self.root.join("resolved-model.json");
        std::fs::write(&model_path, serde_json::to_vec(model).unwrap()).unwrap();
        (bin, model_path)
    }
}

impl Drop for RuntimeFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn fixture_plan(repo: &Path) -> ResourcePlan {
    let identity = EnvironmentIdentity::resolve(repo, "local", Some("generation")).unwrap();
    ResourcePlan::new(
        identity,
        Some(MavenPlan {
            isolation: MavenIsolation::SplitLocal,
            local_prefix: "autospec/test-environment".to_string(),
        }),
        Some(ComposePlan {
            isolation: ComposeIsolation::Managed,
            files: vec![repo.join("compose.yaml")],
            project_name: "autospec-test-environment".to_string(),
            exports: Vec::new(),
            preserve_volumes: Vec::new(),
            shared_networks: Vec::new(),
            shared_volumes: Vec::new(),
        }),
    )
    .unwrap()
}

#[test]
fn invocation_overrides_accept_only_off_and_report_bypass() {
    let fixture =
        RuntimeFixture::with_manifest("version: 1\nmodes:\n  local:\n    command: sh -c 'true'\n");
    let mut plan = fixture_plan(&fixture.root);

    let bypassed = plan
        .apply_invocation_overrides(Some("off"), Some("off"), false)
        .expect("documented overrides apply");

    assert!(bypassed);
    let overridden_digest = plan.digest.clone();
    assert_eq!(plan.maven.unwrap().isolation, MavenIsolation::Off);
    assert_eq!(plan.compose.unwrap().isolation, ComposeIsolation::Off);
    let mut expected = fixture_plan(&fixture.root);
    expected.maven.as_mut().unwrap().isolation = MavenIsolation::Off;
    expected.compose.as_mut().unwrap().isolation = ComposeIsolation::Off;
    let expected = ResourcePlan::new(expected.identity, expected.maven, expected.compose).unwrap();
    assert_eq!(overridden_digest, expected.digest);

    for (maven, compose, expected) in [
        (Some("split-local"), None, "AUTOSPEC_MAVEN_ISOLATION"),
        (None, Some("managed"), "AUTOSPEC_COMPOSE_ISOLATION"),
        (Some(""), None, "AUTOSPEC_MAVEN_ISOLATION"),
    ] {
        let mut plan = fixture_plan(&fixture.root);
        let error = plan
            .apply_invocation_overrides(maven, compose, false)
            .expect_err("unsupported invocation override fails closed");
        assert!(error.to_string().contains(expected));
    }
}

#[test]
fn compose_override_applies_before_legacy_dual_authority_validation() {
    let fixture = RuntimeFixture::with_manifest(
        "version: 1\nmodes:\n  local:\n    command: docker compose version >/dev/null\n",
    );
    std::fs::write(fixture.root.join("compose.yaml"), "services: {}\n").unwrap();

    let output = fixture
        .command()
        .env("AUTOSPEC_COMPOSE_ISOLATION", "off")
        .args(["runtime", "env", "up", "--repo"])
        .arg(&fixture.root)
        .output()
        .expect("runtime command starts");

    assert!(
        !String::from_utf8_lossy(&output.stderr).contains("RUNTIME_DUAL_COMPOSE_AUTHORITY"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn per_resource_bypass_is_exported_without_disabling_the_mode_command() {
    let fixture = RuntimeFixture::with_manifest(
        "version: 1\nmodes:\n  local:\n    command: sh -c 'test \"$AUTOSPEC_ISOLATION_BYPASSED\" = 1'\n",
    );
    std::fs::write(fixture.root.join("pom.xml"), "<project/>\n").unwrap();

    let output = fixture
        .command()
        .env("AUTOSPEC_MAVEN_ISOLATION", "off")
        .args(["runtime", "env", "up", "--repo"])
        .arg(&fixture.root)
        .output()
        .expect("runtime command starts");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("AUTOSPEC_ISOLATION_BYPASSED=1"));
}

#[test]
fn whole_environment_disable_marks_every_planned_resource_off() {
    let fixture =
        RuntimeFixture::with_manifest("version: 1\nmodes:\n  local:\n    command: sh -c 'true'\n");
    let mut plan = fixture_plan(&fixture.root);

    assert!(plan
        .apply_invocation_overrides(None, None, true)
        .expect("whole environment bypass applies"));
    assert_eq!(plan.maven.unwrap().isolation, MavenIsolation::Off);
    assert_eq!(plan.compose.unwrap().isolation, ComposeIsolation::Off);
}

#[test]
fn disabled_up_exec_and_session_skip_provisioning_and_export_the_marker() {
    let fixture = RuntimeFixture::with_manifest(
        "version: 1\nmodes:\n  local:\n    command: sh -c 'exit 99'\n",
    );

    let up = fixture
        .command()
        .env("AUTOSPEC_ENV_DISABLE", "1")
        .args(["runtime", "env", "up", "--repo"])
        .arg(&fixture.root)
        .output()
        .expect("disabled up starts");
    assert!(
        up.status.success(),
        "{}",
        String::from_utf8_lossy(&up.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&up.stdout),
        "AUTOSPEC_ISOLATION_BYPASSED=1\n"
    );
    assert!(!fixture.state_root.exists());

    for operation in ["exec", "session"] {
        let output = fixture
            .command()
            .env("AUTOSPEC_ENV_DISABLE", "1")
            .args(["runtime", "env", operation, "--repo"])
            .arg(&fixture.root)
            .args([
                "--",
                "sh",
                "-c",
                "test \"$AUTOSPEC_ISOLATION_BYPASSED\" = 1",
            ])
            .output()
            .expect("disabled child command starts");
        assert!(
            output.status.success(),
            "{operation}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn whole_environment_disable_does_not_require_a_manifest() {
    let fixture = RuntimeFixture::empty();

    let output = fixture
        .command()
        .env("AUTOSPEC_ENV_DISABLE", "1")
        .args(["runtime", "env", "up", "--repo"])
        .arg(&fixture.root)
        .output()
        .expect("disabled up starts");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "AUTOSPEC_ISOLATION_BYPASSED=1\n"
    );
}

#[test]
fn down_without_owned_state_never_runs_manifest_cleanup() {
    let fixture = RuntimeFixture::with_manifest(
        "version: 1\nmodes:\n  local:\n    command: sh -c 'true'\n    down: sh -c 'printf unsafe > down.txt'\n",
    );

    for _ in 0..2 {
        let output = fixture
            .command()
            .args(["runtime", "env", "down", "--repo"])
            .arg(&fixture.root)
            .output()
            .expect("runtime env down starts");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    assert!(!fixture.root.join("down.txt").exists());
    assert!(!fixture.state_root.exists());
}

#[test]
fn status_is_read_only_without_owned_state() {
    let fixture = RuntimeFixture::with_manifest(
        "version: 1\nmodes:\n  local:\n    command: sh -c 'true'\n    down: sh -c 'printf unsafe > down.txt'\n",
    );

    let output = fixture
        .command()
        .args(["runtime", "env", "status", "--repo"])
        .arg(&fixture.root)
        .output()
        .expect("runtime env status starts");

    assert_eq!(output.status.code(), Some(3));
    assert!(!fixture.root.join("down.txt").exists());
    assert!(!fixture.state_root.exists());
}

#[test]
fn invalid_resource_override_fails_before_a_disabled_command_can_start() {
    let fixture =
        RuntimeFixture::with_manifest("version: 1\nmodes:\n  local:\n    command: sh -c 'true'\n");

    let output = fixture
        .command()
        .env("AUTOSPEC_ENV_DISABLE", "1")
        .env("AUTOSPEC_MAVEN_ISOLATION", "split-local")
        .args(["runtime", "env", "up", "--repo"])
        .arg(&fixture.root)
        .output()
        .expect("invalid override command starts");

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("AUTOSPEC_MAVEN_ISOLATION"));
}

#[test]
fn owner_lifecycle_is_persisted_before_provision_and_teardown_effects() {
    let fixture = RuntimeFixture::with_manifest(
        "version: 1\ndefault_mode: local\nmodes:\n  local:\n    command: sh -c 'grep -q Provisioning \"$AGENT_ENV_STATE_ROOT/$AGENT_ENV_ID/owner.json\" && touch provisioning-seen'\n    down: sh -c 'grep -q TearingDown \"$AGENT_ENV_STATE_ROOT/$AGENT_ENV_ID/owner.json\" && touch teardown-seen'\n",
    );
    let up = fixture
        .command()
        .args(["runtime", "env", "up", "--repo"])
        .arg(&fixture.root)
        .output()
        .expect("up starts");
    assert!(
        up.status.success(),
        "{}",
        String::from_utf8_lossy(&up.stderr)
    );
    assert!(fixture.root.join("provisioning-seen").is_file());

    let environment = std::fs::read_dir(&fixture.state_root)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let active: EnvironmentOwner =
        autospec_core::runtime_env::read_json(&environment.join("owner.json")).unwrap();
    assert_eq!(active.lifecycle, EnvironmentLifecycle::Active);

    let down = fixture
        .command()
        .args(["runtime", "env", "down", "--repo"])
        .arg(&fixture.root)
        .output()
        .expect("down starts");
    assert!(
        down.status.success(),
        "{}",
        String::from_utf8_lossy(&down.stderr)
    );
    assert!(fixture.root.join("teardown-seen").is_file());
}

#[cfg(unix)]
fn compose_manifest(command: &str) -> String {
    format!(
        "version: 2\ndefault_mode: local\nmodes:\n  local:\n    command: {command}\nresources:\n  compose:\n    files: [compose.yaml, compose.override.yaml]\n    exports:\n      - service: web\n        target: 8080\n        protocol: tcp\n        env: WEB_PORT\n        value: port\n"
    )
}

#[cfg(unix)]
fn compose_command(fixture: &RuntimeFixture, bin: &Path, model: &Path) -> Command {
    let log = fixture.root.join("docker-arguments.log");
    let inherited_path = std::env::var_os("PATH").unwrap_or_default();
    let path = std::env::join_paths(
        std::iter::once(bin.to_path_buf()).chain(std::env::split_paths(&inherited_path)),
    )
    .unwrap();
    let mut command = fixture.command();
    command
        .env("PATH", path)
        .env("FAKE_DOCKER_LOG", log)
        .env("FAKE_DOCKER_MODEL", model);
    command
}

#[test]
#[cfg(unix)]
fn compose_config_orders_files_and_project_before_the_config_subcommand() {
    let fixture = RuntimeFixture::with_manifest(&compose_manifest("sh -c 'true'"));
    std::fs::write(fixture.root.join("compose.yaml"), "services: {}\n").unwrap();
    std::fs::write(fixture.root.join("compose.override.yaml"), "services: {}\n").unwrap();
    let (bin, model) = fixture.install_fake_docker(&serde_json::json!({
        "services":{"web":{"ports":[{"target":8080,"protocol":"tcp"}]}}
    }));
    let identity = EnvironmentIdentity::resolve(&fixture.root, "local", None).unwrap();
    let plan = autospec_core::runtime_env::RuntimeManifest::resource_plan_for_repo(
        &fixture.root,
        &identity,
    )
    .unwrap();
    let project_name = plan.compose.unwrap().project_name;

    let output = compose_command(&fixture, &bin, &model)
        .args(["runtime", "env", "up", "--repo"])
        .arg(&fixture.root)
        .output()
        .expect("runtime Compose validation starts");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let arguments = std::fs::read_to_string(fixture.root.join("docker-arguments.log")).unwrap();
    assert_eq!(
        arguments.lines().collect::<Vec<_>>(),
        vec![
            "compose",
            "-f",
            fixture.root.join("compose.yaml").to_str().unwrap(),
            "-f",
            fixture.root.join("compose.override.yaml").to_str().unwrap(),
            "--project-name",
            project_name.as_str(),
            "config",
            "--format",
            "json"
        ]
    );
}

#[test]
#[cfg(unix)]
fn compose_config_preserves_the_nonzero_docker_exit() {
    let fixture = RuntimeFixture::with_manifest(&compose_manifest("sh -c 'true'"));
    std::fs::write(fixture.root.join("compose.yaml"), "services: {}\n").unwrap();
    std::fs::write(fixture.root.join("compose.override.yaml"), "services: {}\n").unwrap();
    let (bin, model) = fixture.install_fake_docker(&serde_json::json!({}));

    let output = compose_command(&fixture, &bin, &model)
        .env("FAKE_DOCKER_EXIT", "37")
        .args(["runtime", "env", "up", "--repo"])
        .arg(&fixture.root)
        .output()
        .expect("runtime Compose validation starts");

    assert_eq!(output.status.code(), Some(37));
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "compose config failed\n"
    );
}

#[test]
#[cfg(unix)]
fn compose_config_validation_finishes_before_any_startup_effect() {
    let fixture =
        RuntimeFixture::with_manifest(&compose_manifest("sh -c 'printf started > startup-effect'"));
    std::fs::write(fixture.root.join("compose.yaml"), "services: {}\n").unwrap();
    std::fs::write(fixture.root.join("compose.override.yaml"), "services: {}\n").unwrap();
    let (bin, model) = fixture.install_fake_docker(&serde_json::json!({
        "services":{"web":{"ports":[{"target":8080,"published":8080,"protocol":"tcp"}]}}
    }));

    let output = compose_command(&fixture, &bin, &model)
        .args(["runtime", "env", "up", "--repo"])
        .arg(&fixture.root)
        .output()
        .expect("runtime Compose validation starts");

    assert_eq!(output.status.code(), Some(2));
    assert!(!fixture.root.join("startup-effect").exists());
    let arguments = std::fs::read_to_string(fixture.root.join("docker-arguments.log")).unwrap();
    assert_eq!(
        arguments
            .lines()
            .filter(|argument| *argument == "up")
            .count(),
        0
    );
}

#[test]
#[cfg(unix)]
fn compose_config_nested_file_diagnostic_keeps_repo_and_environment_context() {
    let fixture = RuntimeFixture::with_manifest(
        "version: 2\ndefault_mode: local\nmodes:\n  local:\n    command: sh -c 'true'\nresources:\n  compose:\n    files: [subdir/compose.yaml]\n",
    );
    std::fs::create_dir_all(fixture.root.join("subdir")).unwrap();
    std::fs::write(fixture.root.join("subdir/compose.yaml"), "services: {}\n").unwrap();
    let (bin, model) = fixture
        .install_fake_docker(&serde_json::json!({"services":{"web":{"container_name":"web"}}}));
    let identity = EnvironmentIdentity::resolve(&fixture.root, "local", None).unwrap();

    let output = compose_command(&fixture, &bin, &model)
        .args(["runtime", "env", "up", "--repo"])
        .arg(&fixture.root)
        .output()
        .expect("runtime Compose validation starts");

    assert_eq!(output.status.code(), Some(2));
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        format!(
            "COMPOSE_CONTAINER_NAME: services.web.container_name=web (environment {}; recovery: autospec runtime env normalize-compose --repo {} --check)\n",
            identity.environment_id,
            fixture.root.display()
        )
    );
}

#[test]
#[cfg(unix)]
fn compose_config_policy_failure_can_be_corrected_and_retried() {
    let fixture = RuntimeFixture::with_manifest(&compose_manifest("sh -c 'true'"));
    std::fs::write(fixture.root.join("compose.yaml"), "services: {}\n").unwrap();
    std::fs::write(fixture.root.join("compose.override.yaml"), "services: {}\n").unwrap();
    let (bin, model) = fixture.install_fake_docker(&serde_json::json!({
        "services":{"web":{"container_name":"web"}}
    }));

    let rejected = compose_command(&fixture, &bin, &model)
        .args(["runtime", "env", "up", "--repo"])
        .arg(&fixture.root)
        .output()
        .unwrap();
    assert_eq!(rejected.status.code(), Some(2));
    std::fs::write(
        &model,
        serde_json::to_vec(&serde_json::json!({
            "services":{"web":{"ports":[{"target":8080,"protocol":"tcp"}]}}
        }))
        .unwrap(),
    )
    .unwrap();

    let corrected = compose_command(&fixture, &bin, &model)
        .args(["runtime", "env", "up", "--repo"])
        .arg(&fixture.root)
        .output()
        .unwrap();

    assert!(
        corrected.status.success(),
        "{}",
        String::from_utf8_lossy(&corrected.stderr)
    );
}
