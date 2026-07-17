use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use autospec_core::runtime_env::{
    read_json, ComposeIsolation, ComposePlan, EnvironmentIdentity, EnvironmentLifecycle,
    EnvironmentOwner, MavenIsolation, MavenPlan, ResourceInventory, ResourcePlan,
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
        let source = "#!/bin/sh\nprintf '%s\\n' \"$@\" >> \"$FAKE_DOCKER_LOG\"\ncase \" $* \" in\n  *\" config @@format json \"*)\n    if [ \"${FAKE_DOCKER_EXIT:-0}\" -ne 0 ]; then\n      printf 'compose config failed \\n\\n' >&2\n      exit \"$FAKE_DOCKER_EXIT\"\n    fi\n    cat \"$FAKE_DOCKER_MODEL\" ;;\n  *\" port @@protocol \"*) printf '%s\\n' '127.0.0.1:49152' ;;\n  *) : ;;\nesac\n";
        std::fs::write(&docker, source.replace("@@", concat!("-", "-")))
            .expect("write fake Docker");
        let mut permissions = std::fs::metadata(&docker).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&docker, permissions).unwrap();
        let model_path = self.root.join("resolved-model.json");
        std::fs::write(&model_path, serde_json::to_vec(model).unwrap()).unwrap();
        (bin, model_path)
    }

    #[cfg(unix)]
    fn install_real_config_docker(&self) -> PathBuf {
        let bin = self.root.join("real-config-bin");
        std::fs::create_dir_all(&bin).unwrap();
        let docker = bin.join("docker");
        let source = "#!/bin/sh\ncase \" $* \" in\n  *\" config @@format json \"*) exec \"$REAL_DOCKER\" \"$@\" ;;\n  *\" port @@protocol \"*) printf '%s\\n' '127.0.0.1:49152' ;;\n  *) : ;;\nesac\n";
        std::fs::write(&docker, source.replace("@@", concat!("-", "-"))).unwrap();
        let mut permissions = std::fs::metadata(&docker).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&docker, permissions).unwrap();
        bin
    }

    #[cfg(unix)]
    fn install_lifecycle_fake_docker(&self) -> PathBuf {
        let bin = self.root.join("fake-lifecycle-bin");
        std::fs::create_dir_all(&bin).unwrap();
        let docker = bin.join("docker");
        let source = r##"#!/bin/sh
printf '%s\n' "$*" >> "$FAKE_DOCKER_LOG"
case " $* " in
  *" config @@format json "*) printf '%s\n' '{"services":{"web":{"image":"nginx","ports":[{"target":8080,"protocol":"tcp"}]}},"networks":{"default":{}},"volumes":{"cache":{}}}' ;;
  *" compose "*" up -d @@remove-orphans "*) : ;;
  *" compose "*" port @@protocol tcp web 8080 "*) printf '%s\n' '127.0.0.1:49152' ;;
  *" ps -aq "*) printf '%s\n' 'container-a' ;;
  *" network ls -q "*) printf '%s\n' 'network-a' ;;
  *" volume ls -q "*) printf '%s\n' 'volume-a' ;;
  *"com.docker.compose.volume"*) printf '%s\n' "${FAKE_LOGICAL_VOLUME:-}" ;;
  *"{{json "*) printf '{"com.autospec.environment-id":"%s","com.autospec.owner-key":"%s","com.autospec.plan-digest":"%s"}\n' "${FAKE_ENVIRONMENT_ID:-}" "${FAKE_OWNER_KEY:-}" "${FAKE_PLAN_DIGEST:-}" ;;
  *" compose "*" down @@remove-orphans "*) touch "$FAKE_DOCKER_DOWN" ;;
  *" inspect "*) [ ! -f "$FAKE_DOCKER_DOWN" ] ;;
  *) : ;;
esac
"##;
        std::fs::write(&docker, source.replace("@@", concat!("-", "-"))).unwrap();
        let mut permissions = std::fs::metadata(&docker).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&docker, permissions).unwrap();
        bin
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

fn compose_manifest_with_preserved_cache(command: &str) -> String {
    format!(
        "{}    preserve_volumes: [cache]\n",
        compose_manifest(command)
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
    let arguments = arguments.lines().collect::<Vec<_>>();
    let compose_file = fixture.root.join("compose.yaml");
    let compose_override = fixture.root.join("compose.override.yaml");
    let expected = vec![
        "compose",
        "--profile",
        "*",
        "--all-resources",
        "-f",
        compose_file.to_str().unwrap(),
        "-f",
        compose_override.to_str().unwrap(),
        "--project-name",
        project_name.as_str(),
        "config",
        "--format",
        "json",
    ];
    assert_eq!(&arguments[..expected.len()], expected);
    assert!(arguments
        .iter()
        .position(|argument| *argument == "up")
        .is_some_and(|up| up > expected.len()));
}

#[test]
#[cfg(unix)]
fn compose_config_preserves_nonzero_exit_and_stderr_trailing_text() {
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
        "compose config failed \n\n\n"
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
fn compose_lifecycle_rejects_caller_project_before_docker_side_effect() {
    let fixture = RuntimeFixture::with_manifest(&compose_manifest("sh -c 'true'"));
    std::fs::write(fixture.root.join("compose.yaml"), "services: {}\n").unwrap();
    std::fs::write(fixture.root.join("compose.override.yaml"), "services: {}\n").unwrap();
    let (bin, model) = fixture.install_fake_docker(&serde_json::json!({}));

    let output = compose_command(&fixture, &bin, &model)
        .env("COMPOSE_PROJECT_NAME", "caller-owned")
        .args(["runtime", "env", "up", "--repo"])
        .arg(&fixture.root)
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "COMPOSE_PROJECT_NAME_CALLER_OVERRIDE: the broker owns the Compose project name\n"
    );
    assert!(!fixture.root.join("docker-arguments.log").exists());
}

#[test]
#[cfg(unix)]
fn compose_lifecycle_records_actual_ids_and_exports_dynamic_state() {
    let fixture =
        RuntimeFixture::with_manifest(&compose_manifest("sh -c 'test \"$WEB_PORT\" = 49152'"));
    std::fs::write(fixture.root.join("compose.yaml"), "services: {}\n").unwrap();
    std::fs::write(fixture.root.join("compose.override.yaml"), "services: {}\n").unwrap();
    let bin = fixture.install_lifecycle_fake_docker();
    let log = fixture.root.join("lifecycle-docker.log");

    let output = fixture
        .command()
        .env(
            "PATH",
            format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
        )
        .env("FAKE_DOCKER_LOG", &log)
        .args(["runtime", "env", "up", "--repo"])
        .arg(&fixture.root)
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let identity = EnvironmentIdentity::resolve(&fixture.root, "local", None).unwrap();
    let inventory: ResourceInventory = read_json(
        &fixture
            .state_root
            .join(identity.environment_id)
            .join("inventory.json"),
    )
    .unwrap();
    assert_eq!(inventory.containers, vec!["container-a"]);
    assert_eq!(inventory.networks, vec!["network-a"]);
    assert_eq!(inventory.volumes[0].id, "volume-a");
    assert_eq!(inventory.exports[0].port, 49152);
    let commands = std::fs::read_to_string(log).unwrap();
    let config_command = format!("config {}format json", concat!("-", "-"));
    let up_command = format!("up -d {}remove-orphans", concat!("-", "-"));
    let config = commands
        .lines()
        .position(|line| line.ends_with(&config_command))
        .unwrap();
    let up = commands
        .lines()
        .position(|line| line.ends_with(&up_command))
        .unwrap();
    assert!(config < up);
}

#[test]
#[cfg(unix)]
fn compose_lifecycle_ownership_mismatch_records_cleanup_failed_recovery() {
    let fixture = RuntimeFixture::with_manifest(&compose_manifest("sh -c 'true'"));
    std::fs::write(fixture.root.join("compose.yaml"), "services: {}\n").unwrap();
    std::fs::write(fixture.root.join("compose.override.yaml"), "services: {}\n").unwrap();
    let bin = fixture.install_lifecycle_fake_docker();
    let log = fixture.root.join("lifecycle-docker.log");
    let mut command = fixture.command();
    command
        .env(
            "PATH",
            format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
        )
        .env("FAKE_DOCKER_LOG", &log);
    let up = command
        .args(["runtime", "env", "up", "--repo"])
        .arg(&fixture.root)
        .output()
        .unwrap();
    assert!(up.status.success());

    let down = fixture
        .command()
        .env(
            "PATH",
            format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
        )
        .env("FAKE_DOCKER_LOG", &log)
        .args(["runtime", "env", "down", "--repo"])
        .arg(&fixture.root)
        .output()
        .unwrap();

    assert_eq!(down.status.code(), Some(2));
    let identity = EnvironmentIdentity::resolve(&fixture.root, "local", None).unwrap();
    let owner: EnvironmentOwner = read_json(
        &fixture
            .state_root
            .join(&identity.environment_id)
            .join("owner.json"),
    )
    .unwrap();
    assert_eq!(owner.lifecycle, EnvironmentLifecycle::CleanupFailed);
    assert_eq!(
        String::from_utf8(down.stderr).unwrap(),
        format!(
            "COMPOSE_OWNERSHIP_MISMATCH: recovery: autospec runtime env down {}repo '{}' {}mode local\n",
            concat!("-", "-"), fixture.root.display(), concat!("-", "-")
        )
    );
    let down_command = format!("down {}remove-orphans", concat!("-", "-"));
    assert_eq!(
        std::fs::read_to_string(log)
            .unwrap()
            .lines()
            .filter(|line| line.ends_with(&down_command))
            .count(),
        0
    );
}

#[test]
#[cfg(unix)]
fn compose_lifecycle_preserves_declared_volume_and_tears_down_exact_owned_stack() {
    let fixture =
        RuntimeFixture::with_manifest(&compose_manifest_with_preserved_cache("sh -c 'true'"));
    std::fs::write(fixture.root.join("compose.yaml"), "services: {}\n").unwrap();
    std::fs::write(fixture.root.join("compose.override.yaml"), "services: {}\n").unwrap();
    let bin = fixture.install_lifecycle_fake_docker();
    let log = fixture.root.join("lifecycle-docker.log");
    let down_marker = fixture.root.join("docker-down");
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap());
    let up = fixture
        .command()
        .env("PATH", &path)
        .env("FAKE_DOCKER_LOG", &log)
        .env("FAKE_LOGICAL_VOLUME", "cache")
        .args(["runtime", "env", "up", "--repo"])
        .arg(&fixture.root)
        .output()
        .unwrap();
    assert!(
        up.status.success(),
        "{}",
        String::from_utf8_lossy(&up.stderr)
    );
    let identity = EnvironmentIdentity::resolve(&fixture.root, "local", None).unwrap();
    let environment = fixture.state_root.join(&identity.environment_id);
    let owner: EnvironmentOwner = read_json(&environment.join("owner.json")).unwrap();
    let plan: ResourcePlan = read_json(&environment.join("plan.json")).unwrap();

    let down = fixture
        .command()
        .env("PATH", path)
        .env("FAKE_DOCKER_LOG", &log)
        .env("FAKE_DOCKER_DOWN", &down_marker)
        .env("FAKE_ENVIRONMENT_ID", &identity.environment_id)
        .env("FAKE_OWNER_KEY", &owner.identity.owner_key)
        .env("FAKE_PLAN_DIGEST", &plan.digest)
        .args(["runtime", "env", "down", "--repo"])
        .arg(&fixture.root)
        .output()
        .unwrap();

    assert!(
        down.status.success(),
        "{}",
        String::from_utf8_lossy(&down.stderr)
    );
    assert!(!environment.join("owner.json").exists());
    assert_eq!(
        std::fs::read_to_string(log)
            .unwrap()
            .lines()
            .filter(|line| line.starts_with("volume rm "))
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
            "COMPOSE_CONTAINER_NAME: services.web.container_name=web (environment {}; recovery: autospec runtime env normalize-compose --repo '{}' --check)\n",
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

#[cfg(unix)]
fn current_compose_supports_complete_config() -> bool {
    Command::new("docker")
        .args([
            "compose",
            "--profile",
            "*",
            "--all-resources",
            "config",
            "--help",
        ])
        .output()
        .is_ok_and(|output| output.status.success())
}

#[cfg(unix)]
fn runtime_fixture_source(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/runtime-resources/compose")
        .join(name)
}

#[cfg(unix)]
fn real_compose_fixture(name: &str) -> (RuntimeFixture, EnvironmentIdentity) {
    let manifest = "version: 2\ndefault_mode: local\nmodes:\n  local:\n    command: sh -c 'true'\nresources:\n  compose:\n    files: [compose.yaml]\n    exports:\n      - service: web\n        target: 8080\n        protocol: tcp\n        env: WEB_PORT\n        value: port\n";
    let fixture = RuntimeFixture::with_manifest(manifest);
    std::fs::copy(
        runtime_fixture_source(name),
        fixture.root.join("compose.yaml"),
    )
    .unwrap();
    let identity = EnvironmentIdentity::resolve(&fixture.root, "local", None).unwrap();
    (fixture, identity)
}

#[cfg(unix)]
fn assert_real_compose_fixture(name: &str, expected: Option<(&str, &str, &str)>) {
    let (fixture, identity) = real_compose_fixture(name);
    let bin = fixture.install_real_config_docker();
    let output = fixture
        .command()
        .env(
            "PATH",
            format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
        )
        .env("REAL_DOCKER", "/usr/bin/docker")
        .args(["runtime", "env", "up", "--repo"])
        .arg(&fixture.root)
        .output()
        .unwrap();
    if let Some((code, resource, evidence)) = expected {
        assert_eq!(output.status.code(), Some(2), "{name}");
        let recovery = format!(
            "autospec runtime env normalize-compose --repo '{}' --check",
            fixture.root.display()
        );
        assert_eq!(
            String::from_utf8(output.stderr).unwrap(),
            format!(
                "{code}: {resource}={evidence} (environment {}; recovery: {recovery})\n",
                identity.environment_id
            )
        );
    } else {
        assert!(output.status.success(), "{name}");
    }
}

#[test]
#[cfg(unix)]
fn compose_current_plugin_drives_the_committed_fixture_matrix() {
    if !current_compose_supports_complete_config() {
        eprintln!("SKIP: current Docker Compose with --all-resources is unavailable");
        return;
    }
    assert_real_compose_fixture("safe.yaml", None);
    for (name, code, resource, evidence) in COMPOSE_UNSAFE_FIXTURES {
        assert_real_compose_fixture(name, Some((code, resource, evidence)));
    }
}

#[test]
#[cfg(unix)]
fn compose_current_plugin_includes_inactive_services_and_unused_resources() {
    if !current_compose_supports_complete_config() {
        eprintln!("SKIP: current Docker Compose with --all-resources is unavailable");
        return;
    }
    for name in ["container-name.yaml", "global-name.yaml"] {
        let (fixture, _) = real_compose_fixture(name);
        let output = fixture
            .command()
            .args(["runtime", "env", "up", "--repo"])
            .arg(&fixture.root)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2), "{name}");
    }
}

#[cfg(unix)]
const COMPOSE_UNSAFE_FIXTURES: [(&str, &str, &str, &str); 7] = [
    (
        "fixed-port.yaml",
        "COMPOSE_FIXED_PORT",
        "services.web.ports[0].published",
        "8080",
    ),
    (
        "container-name.yaml",
        "COMPOSE_CONTAINER_NAME",
        "services.web.container_name",
        "fixed-web",
    ),
    (
        "host-network.yaml",
        "COMPOSE_HOST_NETWORK",
        "services.web.network_mode",
        "host",
    ),
    (
        "global-name.yaml",
        "COMPOSE_GLOBAL_NAME",
        "networks.private.name",
        "global-network",
    ),
    (
        "fixed-ip.yaml",
        "COMPOSE_FIXED_ADDRESS",
        "services.web.networks.private.ipv4_address",
        "10.0.0.8",
    ),
    (
        "external.yaml",
        "COMPOSE_EXTERNAL_UNDECLARED",
        "networks.company-vpn.external",
        "company-vpn",
    ),
    (
        "writable-bind.yaml",
        "COMPOSE_WRITABLE_BIND_OUTSIDE_WORKTREE",
        "services.web.volumes[0].source",
        "/tmp/shared-data",
    ),
];
