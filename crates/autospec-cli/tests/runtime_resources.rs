use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use autospec_core::runtime_env::{
    read_json, ComposeIsolation, ComposePlan, EnvironmentIdentity, EnvironmentLifecycle,
    EnvironmentOwner, MavenIsolation, MavenPlan, ResolvedExport, ResourceInventory, ResourcePlan,
};

static NEXT_FIXTURE: AtomicUsize = AtomicUsize::new(0);

fn normalize_cli_fixture(name: &str) -> RuntimeFixture {
    let fixture = RuntimeFixture::empty();
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/fixtures/compose-normalize")
        .join(name);
    std::fs::create_dir_all(fixture.root.join(".autospec")).unwrap();
    std::fs::copy(
        source.join("runtime.yml"),
        fixture.root.join(".autospec/runtime.yml"),
    )
    .unwrap();
    std::fs::copy(
        source.join("compose.yaml"),
        fixture.root.join("compose.yaml"),
    )
    .unwrap();
    fixture
}

fn normalize_command(fixture: &RuntimeFixture, operation: &str) -> Command {
    let mut command = fixture.command();
    command.args(["runtime", "env", "normalize-compose", "--repo"]);
    command.arg(&fixture.root).arg(operation);
    command
}

#[test]
fn normalize_check_is_read_only_and_returns_a_stable_fingerprint() {
    let fixture = normalize_cli_fixture("fixed-port");
    let compose = fixture.root.join("compose.yaml");
    let before = std::fs::read(&compose).unwrap();

    let first = normalize_command(&fixture, "--check").output().unwrap();
    let second = normalize_command(&fixture, "--check").output().unwrap();

    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert_eq!(first.stdout, second.stdout);
    let report: serde_json::Value = serde_json::from_slice(&first.stdout).unwrap();
    assert_eq!(report["schema_version"], 1);
    assert_eq!(report["fingerprint"].as_str().unwrap().len(), 64);
    assert_eq!(std::fs::read(compose).unwrap(), before);
}

#[test]
#[cfg(unix)]
fn normalize_without_manifest_checks_then_creates_v2_manifest() {
    let fixture = RuntimeFixture::empty();
    let compose = fixture.root.join("compose.yaml");
    let manifest = fixture.root.join(".autospec/runtime.yml");
    std::fs::write(
        &compose,
        "services:\n  web:\n    image: nginx\n    ports:\n      - \"18080:8080\"\n",
    )
    .unwrap();
    let check = normalize_command(&fixture, "--check").output().unwrap();

    assert!(
        check.status.success(),
        "{}",
        String::from_utf8_lossy(&check.stderr)
    );
    assert!(!manifest.exists(), "check must remain read-only");
    let report: serde_json::Value = serde_json::from_slice(&check.stdout).unwrap();
    assert_eq!(
        report["input_paths"],
        serde_json::json!([".autospec/runtime.yml", "compose.yaml"])
    );
    assert_eq!(report["manifest_path"], ".autospec/runtime.yml");
    assert!(report["edits"]
        .as_array()
        .unwrap()
        .iter()
        .any(|edit| edit.get("UpsertRuntimeResources").is_some()));

    let apply = normalize_command(&fixture, "--apply")
        .args(["--fingerprint", report["fingerprint"].as_str().unwrap()])
        .output()
        .unwrap();

    assert!(
        apply.status.success(),
        "{}",
        String::from_utf8_lossy(&apply.stderr)
    );
    let rendered = std::fs::read_to_string(&manifest).unwrap();
    assert!(!rendered.contains('\\'), "manifest paths must be portable");
    let rendered = std::fs::read_to_string(manifest).unwrap();
    let parsed = autospec_core::runtime_env::RuntimeManifest::parse(&rendered).unwrap();
    assert_eq!(
        parsed.resources().compose.files,
        vec![PathBuf::from("compose.yaml")]
    );
}

#[test]
#[cfg(unix)]
fn normalize_without_manifest_rolls_back_created_destination() {
    let fixture = RuntimeFixture::empty();
    let compose = fixture.root.join("compose.yaml");
    let manifest = fixture.root.join(".autospec/runtime.yml");
    let original = b"services:\n  web:\n    image: nginx\n    ports:\n      - \"18080:8080\"\n";
    std::fs::write(&compose, original).unwrap();
    let (bin, model) = fixture.install_fake_docker(&serde_json::json!({
        "services":{"web":{"image":"nginx","ports":[{"target":8080,"published":"18080","protocol":"tcp"}]}}
    }));
    let inherited = std::env::var_os("PATH").unwrap_or_default();
    let path = std::env::join_paths(std::iter::once(bin).chain(std::env::split_paths(&inherited)))
        .unwrap();
    let log = fixture.root.join("docker.log");
    let count = fixture.root.join("docker-count");
    let check = normalize_command(&fixture, "--check")
        .env("PATH", &path)
        .env("FAKE_DOCKER_LOG", &log)
        .env("FAKE_DOCKER_MODEL", &model)
        .env("FAKE_DOCKER_COUNT", &count)
        .env("FAKE_DOCKER_FAIL_AFTER", "2")
        .output()
        .unwrap();
    assert!(check.status.success());
    let report: serde_json::Value = serde_json::from_slice(&check.stdout).unwrap();

    let apply = normalize_command(&fixture, "--apply")
        .args(["--fingerprint", report["fingerprint"].as_str().unwrap()])
        .env("PATH", path)
        .env("FAKE_DOCKER_LOG", log)
        .env("FAKE_DOCKER_MODEL", model)
        .env("FAKE_DOCKER_COUNT", count)
        .env("FAKE_DOCKER_FAIL_AFTER", "2")
        .output()
        .unwrap();

    assert!(!apply.status.success());
    assert!(String::from_utf8_lossy(&apply.stderr).contains("NORMALIZE_COMPOSE_CONFIG_FAILED"));
    assert!(
        !manifest.exists(),
        "rollback must remove a created manifest"
    );
    assert!(!fixture.root.join(".autospec").exists());
    assert_eq!(std::fs::read(compose).unwrap(), original);
}

#[test]
#[cfg(unix)]
fn normalize_without_manifest_rejects_stale_source_before_destination_write() {
    let fixture = RuntimeFixture::empty();
    let compose = fixture.root.join("compose.yaml");
    let manifest = fixture.root.join(".autospec/runtime.yml");
    std::fs::write(
        &compose,
        "services:\n  web:\n    image: nginx\n    ports:\n      - \"18080:8080\"\n",
    )
    .unwrap();
    let check = normalize_command(&fixture, "--check").output().unwrap();
    assert!(check.status.success());
    let report: serde_json::Value = serde_json::from_slice(&check.stdout).unwrap();
    let mutation = b"services: {}\n";
    std::fs::write(&compose, mutation).unwrap();

    let apply = normalize_command(&fixture, "--apply")
        .args(["--fingerprint", report["fingerprint"].as_str().unwrap()])
        .output()
        .unwrap();

    assert!(!apply.status.success());
    assert!(String::from_utf8_lossy(&apply.stderr).contains("NORMALIZE_STALE_FINGERPRINT"));
    assert!(!manifest.exists());
    assert_eq!(std::fs::read(compose).unwrap(), mutation);
}

#[test]
#[cfg(unix)]
fn normalize_without_manifest_rejects_symlinked_destination_parent() {
    use std::os::unix::fs::symlink;

    let fixture = RuntimeFixture::empty();
    let outside = fixture.root.join("outside");
    std::fs::create_dir(&outside).unwrap();
    symlink(&outside, fixture.root.join(".autospec")).unwrap();
    std::fs::write(
        fixture.root.join("compose.yaml"),
        "services:\n  web:\n    image: nginx\n",
    )
    .unwrap();

    let check = normalize_command(&fixture, "--check").output().unwrap();

    assert!(!check.status.success());
    assert!(String::from_utf8_lossy(&check.stderr)
        .contains("normalization destination parent is not a regular directory"));
    assert!(!outside.join("runtime.yml").exists());
}

#[test]
#[cfg(unix)]
fn normalize_check_uses_the_complete_compose_model_before_reporting() {
    let fixture = normalize_cli_fixture("fixed-port");
    let (bin, model) = fixture.install_fake_docker(&serde_json::json!({
        "services":{"web":{"image":"nginx","ports":[{"target":8080,"published":"8080","protocol":"tcp"}]}}
    }));
    let log = fixture.root.join("docker.log");
    let inherited = std::env::var_os("PATH").unwrap_or_default();
    let path = std::env::join_paths(std::iter::once(bin).chain(std::env::split_paths(&inherited)))
        .unwrap();

    let output = normalize_command(&fixture, "--check")
        .env("PATH", path)
        .env("FAKE_DOCKER_LOG", &log)
        .env("FAKE_DOCKER_MODEL", model)
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let arguments = std::fs::read_to_string(log).expect("check must invoke docker compose config");
    assert!(arguments.contains("compose\n--profile\n*\n--all-resources\n"));
    assert!(arguments.contains("config\n--format\njson\n"));
}

#[test]
fn normalize_help_and_digest_case_are_explicit() {
    let help = RuntimeFixture::empty()
        .command()
        .args(["runtime", "env", "--help"])
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&help.stdout)
        .contains("normalize-compose --repo PATH --check|--apply --fingerprint SHA256"));

    let fixture = normalize_cli_fixture("fixed-port");
    let uppercase = normalize_command(&fixture, "--apply")
        .args(["--fingerprint", &"A".repeat(64)])
        .output()
        .unwrap();
    assert_eq!(uppercase.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&uppercase.stderr).contains("lowercase SHA-256"));
}

#[test]
fn normalize_unsafe_check_is_json_and_apply_refuses_without_writes() {
    let fixture = normalize_cli_fixture("host-network");
    let (bin, model) = fixture.install_fake_docker(&serde_json::json!({
        "services":{"web":{"image":"nginx","network_mode":"host"}}
    }));
    let inherited = std::env::var_os("PATH").unwrap_or_default();
    let path = std::env::join_paths(std::iter::once(bin).chain(std::env::split_paths(&inherited)))
        .unwrap();
    let log = fixture.root.join("docker.log");
    let compose = fixture.root.join("compose.yaml");
    let before = std::fs::read(&compose).unwrap();
    let check = normalize_command(&fixture, "--check")
        .env("PATH", &path)
        .env("FAKE_DOCKER_LOG", &log)
        .env("FAKE_DOCKER_MODEL", &model)
        .output()
        .unwrap();
    assert!(
        check.status.success(),
        "{}",
        String::from_utf8_lossy(&check.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&check.stdout).unwrap();
    assert_eq!(
        report["remaining_diagnostics"][0]["code"],
        "COMPOSE_HOST_NETWORK"
    );

    let apply = normalize_command(&fixture, "--apply")
        .args(["--fingerprint", report["fingerprint"].as_str().unwrap()])
        .env("PATH", &path)
        .env("FAKE_DOCKER_LOG", &log)
        .env("FAKE_DOCKER_MODEL", &model)
        .output()
        .unwrap();
    assert_eq!(apply.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&apply.stderr).contains("NORMALIZE_UNRESOLVED_DIAGNOSTICS"));
    assert_eq!(std::fs::read(compose).unwrap(), before);
}

#[test]
#[cfg(unix)]
fn normalize_flow_manifest_check_and_apply_preserve_golden_bytes() {
    let fixture = RuntimeFixture::empty();
    std::fs::create_dir_all(fixture.root.join(".autospec")).unwrap();
    let compose = fixture.root.join("compose.yaml");
    let manifest = fixture.root.join(".autospec/runtime.yml");
    let compose_bytes =
        b"services:\n  web:\n    image: nginx\n    ports:\n      - \"18080:8080\"\n";
    let manifest_bytes = b"{version: 2, resources: {compose: {files: [compose.yaml]}}}\n";
    std::fs::write(&compose, compose_bytes).unwrap();
    std::fs::write(&manifest, manifest_bytes).unwrap();
    let (bin, model) = fixture.install_fake_docker(&serde_json::json!({
        "services":{"web":{"image":"nginx","ports":[{"target":8080,"published":"18080","protocol":"tcp"}]}}
    }));
    let inherited = std::env::var_os("PATH").unwrap_or_default();
    let path = std::env::join_paths(std::iter::once(bin).chain(std::env::split_paths(&inherited)))
        .unwrap();
    let log = fixture.root.join("docker.log");

    let check = normalize_command(&fixture, "--check")
        .env("PATH", &path)
        .env("FAKE_DOCKER_LOG", &log)
        .env("FAKE_DOCKER_MODEL", &model)
        .output()
        .unwrap();
    assert!(check.status.success());
    let report: serde_json::Value = serde_json::from_slice(&check.stdout).unwrap();
    assert_eq!(
        report["remaining_diagnostics"][0]["code"],
        "NORMALIZE_FLOW_MANIFEST_UNSUPPORTED"
    );
    assert_eq!(std::fs::read(&compose).unwrap(), compose_bytes);
    assert_eq!(std::fs::read(&manifest).unwrap(), manifest_bytes);

    let apply = normalize_command(&fixture, "--apply")
        .args(["--fingerprint", report["fingerprint"].as_str().unwrap()])
        .env("PATH", &path)
        .env("FAKE_DOCKER_LOG", &log)
        .env("FAKE_DOCKER_MODEL", &model)
        .output()
        .unwrap();
    assert_eq!(apply.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&apply.stderr).contains("NORMALIZE_UNRESOLVED_DIAGNOSTICS"));
    assert_eq!(std::fs::read(compose).unwrap(), compose_bytes);
    assert_eq!(std::fs::read(manifest).unwrap(), manifest_bytes);
}

#[test]
fn normalize_apply_rejects_a_stale_fingerprint_before_writes() {
    let fixture = normalize_cli_fixture("fixed-port");
    let manifest = fixture.root.join(".autospec/runtime.yml");
    let original = std::fs::read(&manifest).unwrap();
    let output = normalize_command(&fixture, "--check").output().unwrap();
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let fingerprint = report["fingerprint"].as_str().unwrap();
    let stale = normalize_command(&fixture, "--apply")
        .args(["--fingerprint", &"0".repeat(fingerprint.len())])
        .output()
        .unwrap();

    assert!(!stale.status.success());
    assert!(String::from_utf8_lossy(&stale.stderr).contains("NORMALIZE_STALE_FINGERPRINT"));
    assert_eq!(std::fs::read(manifest).unwrap(), original);
}

#[test]
#[cfg(unix)]
fn normalize_apply_is_idempotent_after_complete_model_verification() {
    if !current_compose_supports_complete_config() {
        eprintln!("SKIP: current Docker Compose with --all-resources is unavailable");
        return;
    }
    let fixture = normalize_cli_fixture("fixed-port");
    let check = normalize_command(&fixture, "--check").output().unwrap();
    let report: serde_json::Value = serde_json::from_slice(&check.stdout).unwrap();
    let fingerprint = report["fingerprint"].as_str().unwrap();

    let first = normalize_command(&fixture, "--apply")
        .args(["--fingerprint", fingerprint])
        .output()
        .unwrap();
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let compose = fixture.root.join("compose.yaml");
    let manifest = fixture.root.join(".autospec/runtime.yml");
    let after = (
        std::fs::read(&compose).unwrap(),
        std::fs::read(&manifest).unwrap(),
    );
    let next = normalize_command(&fixture, "--check").output().unwrap();
    let next_report: serde_json::Value = serde_json::from_slice(&next.stdout).unwrap();
    let next_fingerprint = next_report["fingerprint"].as_str().unwrap();
    let second = normalize_command(&fixture, "--apply")
        .args(["--fingerprint", next_fingerprint])
        .output()
        .unwrap();

    assert!(
        second.status.success(),
        "{}",
        String::from_utf8_lossy(&second.stderr)
    );
    assert_eq!(after.0, std::fs::read(compose).unwrap());
    assert_eq!(after.1, std::fs::read(manifest).unwrap());
}

#[test]
#[cfg(unix)]
fn normalize_apply_rolls_back_all_files_when_compose_verification_fails() {
    let fixture = normalize_cli_fixture("fixed-port");
    let (bin, model) = fixture.install_fake_docker(&serde_json::json!({}));
    let inherited = std::env::var_os("PATH").unwrap_or_default();
    let path = std::env::join_paths(std::iter::once(bin).chain(std::env::split_paths(&inherited)))
        .unwrap();
    let compose = fixture.root.join("compose.yaml");
    let manifest = fixture.root.join(".autospec/runtime.yml");
    let before = (
        std::fs::read(&compose).unwrap(),
        std::fs::read(&manifest).unwrap(),
    );
    let check = normalize_command(&fixture, "--check")
        .env("PATH", &path)
        .env("FAKE_DOCKER_LOG", fixture.root.join("docker.log"))
        .env("FAKE_DOCKER_MODEL", &model)
        .output()
        .unwrap();
    let report: serde_json::Value = serde_json::from_slice(&check.stdout).unwrap();

    let output = normalize_command(&fixture, "--apply")
        .args(["--fingerprint", report["fingerprint"].as_str().unwrap()])
        .env("PATH", path)
        .env("FAKE_DOCKER_LOG", fixture.root.join("docker.log"))
        .env("FAKE_DOCKER_MODEL", model)
        .env("FAKE_DOCKER_EXIT", "42")
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("NORMALIZE_COMPOSE_CONFIG_FAILED"));
    assert_eq!(std::fs::read(compose).unwrap(), before.0);
    assert_eq!(std::fs::read(manifest).unwrap(), before.1);
}

#[test]
#[cfg(unix)]
fn normalize_apply_passes_the_current_compose_complete_model() {
    if !current_compose_supports_complete_config() {
        eprintln!("SKIP: current Docker Compose with --all-resources is unavailable");
        return;
    }
    for name in [
        "fixed-port",
        "container-name",
        "project-name",
        "single-http",
    ] {
        let fixture = normalize_cli_fixture(name);
        let check = normalize_command(&fixture, "--check").output().unwrap();
        let report: serde_json::Value = serde_json::from_slice(&check.stdout).unwrap();
        let apply = normalize_command(&fixture, "--apply")
            .args(["--fingerprint", report["fingerprint"].as_str().unwrap()])
            .output()
            .unwrap();
        assert!(
            apply.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&apply.stderr)
        );
        let config = Command::new("docker")
            .args(["compose", "-f"])
            .arg(fixture.root.join("compose.yaml"))
            .arg("config")
            .current_dir(&fixture.root)
            .output()
            .unwrap();
        assert!(
            config.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&config.stderr)
        );
    }
}

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
        let source = "#!/bin/sh\nprintf '%s\\n' \"$@\" >> \"$FAKE_DOCKER_LOG\"\ncase \" $* \" in\n  *\" config @@format json \"*)\n    if [ -n \"${FAKE_DOCKER_COUNT:-}\" ]; then\n      count=$(cat \"$FAKE_DOCKER_COUNT\" 2>/dev/null || printf 0)\n      count=$((count + 1))\n      printf '%s' \"$count\" > \"$FAKE_DOCKER_COUNT\"\n      if [ \"$count\" -gt \"${FAKE_DOCKER_FAIL_AFTER:-999999}\" ]; then\n        printf 'compose config failed\\n' >&2\n        exit 42\n      fi\n    fi\n    if [ \"${FAKE_DOCKER_EXIT:-0}\" -ne 0 ]; then\n      printf 'compose config failed \\n\\n' >&2\n      exit \"$FAKE_DOCKER_EXIT\"\n    fi\n    cat \"$FAKE_DOCKER_MODEL\" ;;\n  *\" port @@protocol \"*) printf '%s\\n' '127.0.0.1:49152' ;;\n  *) : ;;\nesac\n";
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
state=${FAKE_DOCKER_STATE_DIR:-$(dirname "$FAKE_DOCKER_LOG")/fake-docker-state}
mkdir -p "$state"
inventory=$(find "$AGENT_ENV_STATE_ROOT" -name inventory.json -print -quit 2>/dev/null)
owner=$(find "$AGENT_ENV_STATE_ROOT" -name owner.json -print -quit 2>/dev/null)
plan=$(find "$AGENT_ENV_STATE_ROOT" -name plan.json -print -quit 2>/dev/null)
environment_id=${FAKE_ENVIRONMENT_ID:-$(sed -n 's/.*"environment_id":"\([^"]*\)".*/\1/p' "$inventory" 2>/dev/null)}
owner_key=${FAKE_OWNER_KEY:-$(sed -n 's/.*"owner_key":"\([^"]*\)".*/\1/p' "$owner" 2>/dev/null)}
plan_digest=${FAKE_PLAN_DIGEST:-$(sed -n 's/.*"digest":"\([^"]*\)".*/\1/p' "$plan" 2>/dev/null)}
[ "${FAKE_LABEL_MISMATCH:-0}" = 1 ] && owner_key=foreign-owner
case " $* " in
  *" config @@format json "*) printf '%s\n' '{"services":{"web":{"image":"nginx","ports":[{"target":8080,"protocol":"tcp"}]}},"networks":{"default":{}},"volumes":{"cache":{}}}' ;;
  *" compose "*" up -d @@remove-orphans "*)
    [ "${FAKE_UP_EXIT:-0}" -eq 0 ] || exit "$FAKE_UP_EXIT" ;;
  *" compose "*" port @@protocol tcp web 8080 "*) printf '%s\n' '127.0.0.1:49152' ;;
  *" ps -aq "*) [ -f "$state/container-removed" ] || printf '%s\n' "${FAKE_CONTAINER_ID:-container-a}" ;;
  *" network ls -q "*) [ -f "$state/network-removed" ] || printf '%s\n' "${FAKE_NETWORK_ID:-network-a}" ;;
  *" volume ls -q "*) [ -f "$state/volume-removed" ] || printf '%s\n' "${FAKE_VOLUME_ID:-volume-a}" ;;
  *"{{range .Mounts}}"*)
    if [ "${FAKE_ANONYMOUS_VOLUME:-}" ]; then
      printf '%s\n' "$FAKE_ANONYMOUS_VOLUME"
    elif [ -z "${FAKE_LOGICAL_VOLUME:-}" ]; then
      printf '%s\n' "${FAKE_VOLUME_ID:-volume-a}"
    fi ;;
  *"com.docker.compose.volume"*) printf '%s\n' "${FAKE_LOGICAL_VOLUME:-}" ;;
  *"{{json "*)
    [ "${FAKE_INSPECT_EXIT:-0}" -eq 0 ] || exit "$FAKE_INSPECT_EXIT"
    printf '{"com.autospec.environment-id":"%s","com.autospec.owner-key":"%s","com.autospec.plan-digest":"%s"}\n' "$environment_id" "$owner_key" "$plan_digest" ;;
  *" rm -f -v "*) touch "$state/container-removed" "$state/anonymous-removed" ;;
  *" network rm "*)
    if [ "${FAKE_NETWORK_RM_ONCE:-0}" = 1 ] && [ ! -f "$state/network-failed-once" ]; then
      touch "$state/network-failed-once"
      exit 41
    fi
    touch "$state/network-removed" ;;
  *" volume rm "*) touch "$state/volume-removed" ;;
  *) : ;;
esac
"##;
        std::fs::write(&docker, source.replace("@@", concat!("-", "-"))).unwrap();
        let mut permissions = std::fs::metadata(&docker).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&docker, permissions).unwrap();
        bin
    }

    #[cfg(unix)]
    fn install_fake_maven(&self) -> PathBuf {
        let bin = self.root.join("fake-maven-bin");
        std::fs::create_dir_all(&bin).unwrap();
        let mvn = bin.join("mvn");
        let repository = self.root.join("maven-repository");
        let source = format!(
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then printf '%s\\n' 'Apache Maven 4.0.0-rc-5'; exit 0; fi\nmkdir -p '{repository}'\nprintf '%s\\n' '{repository}'\n",
            repository = repository.display(),
        );
        std::fs::write(&mvn, source).unwrap();
        let mut permissions = std::fs::metadata(&mvn).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&mvn, permissions).unwrap();
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
fn whole_environment_disable_cannot_disable_down_for_owned_state() {
    let fixture = RuntimeFixture::empty();
    let pid = fixture.root.join("server.pid");
    std::fs::create_dir_all(fixture.root.join(".autospec")).unwrap();
    std::fs::write(
        fixture.root.join(".autospec/runtime.yml"),
        format!(
            "version: 1\nmodes:\n  local:\n    command: sh -c 'python3 -m http.server \"$AGENT_FRONTEND_PORT\" --bind 127.0.0.1 >/dev/null 2>&1 & echo $! > {pid}'\n    down: sh -c 'kill \"$(cat {pid})\"'\n",
            pid = pid.display()
        ),
    )
    .unwrap();
    let up = fixture
        .command()
        .args(["runtime", "env", "up", "--repo"])
        .arg(&fixture.root)
        .output()
        .unwrap();
    assert!(
        up.status.success(),
        "{}",
        String::from_utf8_lossy(&up.stderr)
    );

    let down = fixture
        .command()
        .env("AUTOSPEC_ENV_DISABLE", "1")
        .args(["runtime", "env", "down", "--repo"])
        .arg(&fixture.root)
        .output()
        .unwrap();

    assert!(
        down.status.success(),
        "{}",
        String::from_utf8_lossy(&down.stderr)
    );
    assert!(std::fs::read_dir(&fixture.state_root)
        .map(|mut entries| entries.all(|entry| !entry.unwrap().path().join("owner.json").exists()))
        .unwrap_or(true));
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
fn gc_removes_stale_empty_owned_state_even_when_isolation_is_disabled() {
    let fixture = RuntimeFixture::empty();
    let stale_repo = fixture.root.join("removed-worktree");
    write_gc_state(&fixture, &stale_repo, "env-stale", "owner-a", "env-stale");

    let output = fixture
        .command()
        .env("AUTOSPEC_ENV_DISABLE", "1")
        .args(["runtime", "env", "gc", "--repo"])
        .arg(&stale_repo)
        .output()
        .expect("runtime env gc starts");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!fixture.state_root.join("env-stale").exists());
}

#[test]
fn gc_preserves_ambiguous_owner_mismatch_with_recovery_command() {
    let fixture = RuntimeFixture::empty();
    let stale_repo = fixture.root.join("removed-worktree");
    write_gc_state(&fixture, &stale_repo, "env-stale", "owner-a", "env-foreign");

    let output = fixture
        .command()
        .args(["runtime", "env", "gc", "--repo"])
        .arg(&stale_repo)
        .output()
        .expect("runtime env gc starts");

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("RESOURCE_OWNER_MISMATCH"), "{stderr}");
    assert!(
        stderr.contains("autospec runtime env gc --repo"),
        "{stderr}"
    );
    assert!(fixture.state_root.join("env-stale").exists());
}

#[test]
fn provisioning_failure_releases_every_claimed_port() {
    let fixture = RuntimeFixture::with_manifest(
        "version: 1\nmodes:\n  local:\n    command: sh -c 'exit 23'\n",
    );

    let output = fixture
        .command()
        .args(["runtime", "env", "up", "--repo"])
        .arg(&fixture.root)
        .output()
        .unwrap();

    assert!(!output.status.success());
    let inventory_path = environment_directory(&fixture).join("inventory.json");
    let inventory: ResourceInventory = read_json(&inventory_path).unwrap();
    let registry: autospec_core::runtime_env::PortRegistry =
        read_json(&fixture.state_root.join("ports/registry.json")).unwrap();
    assert_eq!(registry.owner(inventory.frontend_port.unwrap()), None);
    assert_eq!(registry.owner(inventory.backend_port.unwrap()), None);
}

#[test]
fn direct_server_reallocates_after_a_real_bind_collision() {
    let fixture = RuntimeFixture::empty();
    std::fs::create_dir_all(fixture.root.join(".autospec")).unwrap();
    let command = format!(
        "if [ ! -f {root}/first-attempt ]; then touch {root}/first-attempt; python3 -m http.server \"$AGENT_FRONTEND_PORT\" --bind 127.0.0.1 >/dev/null 2>&1 & echo $! > {root}/first.pid; exit 23; fi; python3 -m http.server \"$AGENT_FRONTEND_PORT\" --bind 127.0.0.1 >/dev/null 2>&1 & echo $! > {root}/second.pid",
        root = fixture.root.display()
    );
    std::fs::write(
        fixture.root.join(".autospec/runtime.yml"),
        format!("version: 1\nmodes:\n  local:\n    command: sh -c '{command}'\n"),
    )
    .unwrap();

    let output = fixture
        .command()
        .args(["runtime", "env", "up", "--repo"])
        .arg(&fixture.root)
        .output()
        .unwrap();

    for name in ["first.pid", "second.pid"] {
        if let Ok(pid) = std::fs::read_to_string(fixture.root.join(name)) {
            let _ = Command::new("kill").arg(pid.trim()).status();
        }
    }
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let inventory: ResourceInventory =
        read_json(&environment_directory(&fixture).join("inventory.json")).unwrap();
    assert_ne!(inventory.frontend_port, inventory.backend_port);
}

#[test]
fn direct_server_requires_frontend_bind_before_active() {
    let fixture = RuntimeFixture::empty();
    std::fs::create_dir_all(fixture.root.join(".autospec")).unwrap();
    let attempts = fixture.root.join("attempts");
    std::fs::write(
        fixture.root.join(".autospec/runtime.yml"),
        format!(
            "version: 1\nmodes:\n  local:\n    command: sh -c 'printf x >> {}; exit 0'\n",
            attempts.display()
        ),
    )
    .unwrap();

    let output = fixture
        .command()
        .args(["runtime", "env", "up", "--repo"])
        .arg(&fixture.root)
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr).starts_with("PORT_BIND_HEALTH_RETRIES_EXHAUSTED")
    );
    assert_eq!(std::fs::read_to_string(attempts).unwrap(), "xxxxx");
    let directory = environment_directory(&fixture);
    let owner: EnvironmentOwner = read_json(&directory.join("owner.json")).unwrap();
    assert_eq!(owner.lifecycle, EnvironmentLifecycle::CleanupFailed);
    let inventory: ResourceInventory = read_json(&directory.join("inventory.json")).unwrap();
    let registry: autospec_core::runtime_env::PortRegistry =
        read_json(&fixture.state_root.join("ports/registry.json")).unwrap();
    assert_eq!(registry.owner(inventory.frontend_port.unwrap()), None);
    assert_eq!(registry.owner(inventory.backend_port.unwrap()), None);
}

#[test]
fn deferred_direct_runtime_activates_after_one_successful_setup() {
    let fixture = RuntimeFixture::with_manifest(
        "version: 1\nmodes:\n  local:\n    command: sh -c 'printf x >> attempts'\n    readiness: deferred\n",
    );

    let output = fixture
        .command()
        .args(["runtime", "env", "up", "--repo"])
        .arg(&fixture.root)
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(fixture.root.join("attempts")).unwrap(),
        "x"
    );
    let directory = environment_directory(&fixture);
    let owner: EnvironmentOwner = read_json(&directory.join("owner.json")).unwrap();
    assert_eq!(owner.lifecycle, EnvironmentLifecycle::Active);
    let inventory: ResourceInventory = read_json(&directory.join("inventory.json")).unwrap();
    let registry: autospec_core::runtime_env::PortRegistry =
        read_json(&fixture.state_root.join("ports/registry.json")).unwrap();
    assert_eq!(
        registry.owner(inventory.frontend_port.unwrap()),
        Some(owner.identity.environment_id.as_str())
    );
    assert_eq!(
        registry.owner(inventory.backend_port.unwrap()),
        Some(owner.identity.environment_id.as_str())
    );
}

#[test]
fn direct_server_becomes_active_after_real_frontend_bind() {
    let fixture = RuntimeFixture::empty();
    std::fs::create_dir_all(fixture.root.join(".autospec")).unwrap();
    let pid = fixture.root.join("server.pid");
    std::fs::write(
        fixture.root.join(".autospec/runtime.yml"),
        format!(
            "version: 1\nmodes:\n  local:\n    command: sh -c 'python3 -m http.server \"$AGENT_FRONTEND_PORT\" --bind 127.0.0.1 >/dev/null 2>&1 & echo $! > {}'\n",
            pid.display()
        ),
    )
    .unwrap();

    let output = fixture
        .command()
        .args(["runtime", "env", "up", "--repo"])
        .arg(&fixture.root)
        .output()
        .unwrap();

    if let Ok(server_pid) = std::fs::read_to_string(&pid) {
        let _ = Command::new("kill").arg(server_pid.trim()).status();
    }
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let owner: EnvironmentOwner =
        read_json(&environment_directory(&fixture).join("owner.json")).unwrap();
    assert_eq!(owner.lifecycle, EnvironmentLifecycle::Active);
}

#[test]
fn gc_refuses_inventory_shared_with_a_locked_foreign_environment() {
    let fixture = RuntimeFixture::empty();
    let candidate_repo = fixture.root.join("removed-candidate");
    let foreign_repo = fixture.root.join("foreign-repo");
    let shared = fixture.root.join("maven/autospec/shared");
    write_gc_state(
        &fixture,
        &candidate_repo,
        "env-candidate",
        "owner-a",
        "env-candidate",
    );
    write_gc_state(
        &fixture,
        &foreign_repo,
        "env-foreign",
        "owner-b",
        "env-foreign",
    );
    for environment in ["env-candidate", "env-foreign"] {
        let path = fixture.state_root.join(environment).join("inventory.json");
        let mut inventory: ResourceInventory = read_json(&path).unwrap();
        inventory.maven_local_prefix = Some(shared.clone());
        autospec_core::runtime_env::write_json_atomic(&path, &inventory).unwrap();
    }
    let ready = fixture.root.join("foreign-ready");
    let release = fixture.root.join("foreign-release");
    let mut holder = fixture
        .command()
        .args(["runtime", "env", "lease-probe"])
        .arg(fixture.state_root.join("env-foreign"))
        .arg(&ready)
        .arg(&release)
        .spawn()
        .unwrap();
    wait_for_file(&ready);

    let output = fixture
        .command()
        .args(["runtime", "env", "gc", "--repo"])
        .arg(&candidate_repo)
        .output()
        .unwrap();

    std::fs::write(release, "release\n").unwrap();
    assert!(holder.wait().unwrap().success());
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).starts_with("RESOURCE_OWNER_MISMATCH"));
    assert!(fixture.state_root.join("env-candidate").is_dir());
}

#[test]
fn gc_refuses_cross_role_and_export_ports_owned_by_a_locked_foreign_environment() {
    for (
        candidate_frontend,
        candidate_backend,
        candidate_export,
        foreign_frontend,
        foreign_backend,
        foreign_export,
    ) in [
        (Some(41001), None, None, None, Some(41001), None),
        (None, None, Some(41002), Some(41002), None, None),
        (None, Some(41003), None, None, None, Some(41003)),
    ] {
        let fixture = RuntimeFixture::empty();
        let candidate_repo = fixture.root.join("removed-candidate");
        let foreign_repo = fixture.root.join("foreign-repo");
        write_gc_state(
            &fixture,
            &candidate_repo,
            "env-candidate",
            "owner-a",
            "env-candidate",
        );
        write_gc_state(
            &fixture,
            &foreign_repo,
            "env-foreign",
            "owner-b",
            "env-foreign",
        );
        set_inventory_ports(
            &fixture,
            "env-candidate",
            candidate_frontend,
            candidate_backend,
            candidate_export,
        );
        set_inventory_ports(
            &fixture,
            "env-foreign",
            foreign_frontend,
            foreign_backend,
            foreign_export,
        );
        assert_gc_rejects_locked_foreign_port_owner(&fixture, &candidate_repo);
    }
}

fn assert_gc_rejects_locked_foreign_port_owner(fixture: &RuntimeFixture, candidate_repo: &Path) {
    let ready = fixture.root.join("foreign-ready");
    let release = fixture.root.join("foreign-release");
    let mut holder = fixture
        .command()
        .args(["runtime", "env", "lease-probe"])
        .arg(fixture.state_root.join("env-foreign"))
        .arg(&ready)
        .arg(&release)
        .spawn()
        .unwrap();
    wait_for_file(&ready);
    let output = fixture
        .command()
        .args(["runtime", "env", "gc", "--repo"])
        .arg(candidate_repo)
        .output()
        .unwrap();
    std::fs::write(release, "release\n").unwrap();
    assert!(holder.wait().unwrap().success());
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).starts_with("RESOURCE_OWNER_MISMATCH"));
    assert!(fixture.state_root.join("env-candidate").is_dir());
}

#[test]
fn gc_treats_an_already_absent_container_as_deleted() {
    let fixture = RuntimeFixture::empty();
    let stale_repo = fixture.root.join("removed-worktree");
    write_gc_state(&fixture, &stale_repo, "env-stale", "owner-a", "env-stale");
    let inventory_path = fixture.state_root.join("env-stale/inventory.json");
    let mut inventory: ResourceInventory = read_json(&inventory_path).unwrap();
    inventory.containers = vec![format!("autospec-absent-{}", std::process::id())];
    autospec_core::runtime_env::write_json_atomic(&inventory_path, &inventory).unwrap();

    let output = fixture
        .command()
        .args(["runtime", "env", "gc", "--repo"])
        .arg(&stale_repo)
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!fixture.state_root.join("env-stale").exists());
}

#[test]
fn gc_reuses_guarded_maven_purge_and_rejects_a_wrong_effective_root() {
    let fixture = RuntimeFixture::with_manifest(
        "version: 2\nresources:\n  maven:\n    isolation: split-local\n  compose:\n    isolation: off\nmodes:\n  local:\n    command: true\n",
    );
    std::fs::write(
        fixture.root.join("pom.xml"),
        "<project xmlns=\"http://maven.apache.org/POM/4.0.0\"><modelVersion>4.0.0</modelVersion><groupId>test</groupId><artifactId>gc</artifactId><version>1</version></project>\n",
    )
    .unwrap();
    let up = fixture
        .command()
        .args(["runtime", "env", "up", "--repo"])
        .arg(&fixture.root)
        .output()
        .unwrap();
    assert!(
        up.status.success(),
        "{}",
        String::from_utf8_lossy(&up.stderr)
    );
    let directory = environment_directory(&fixture);
    let plan_path = directory.join("plan.json");
    let owner_path = directory.join("owner.json");
    let inventory_path = directory.join("inventory.json");
    let plan: ResourcePlan = read_json(&plan_path).unwrap();
    let mut identity = plan.identity.clone();
    identity.generation = Some("stale-generation".into());
    let stale_plan =
        ResourcePlan::new(identity.clone(), plan.maven.clone(), plan.compose.clone()).unwrap();
    let mut owner: EnvironmentOwner = read_json(&owner_path).unwrap();
    owner.identity = identity;
    owner.manifest_digest = stale_plan.digest.clone();
    autospec_core::runtime_env::write_json_atomic(&plan_path, &stale_plan).unwrap();
    autospec_core::runtime_env::write_json_atomic(&owner_path, &owner).unwrap();
    let wrong_prefix = fixture
        .root
        .join("wrong-root/autospec")
        .join(&owner.identity.environment_id);
    std::fs::create_dir_all(&wrong_prefix).unwrap();
    std::fs::write(wrong_prefix.join("must-survive"), "owned elsewhere\n").unwrap();
    let mut inventory: ResourceInventory = read_json(&inventory_path).unwrap();
    inventory.maven_local_prefix = Some(wrong_prefix.clone());
    autospec_core::runtime_env::write_json_atomic(&inventory_path, &inventory).unwrap();

    let output = fixture
        .command()
        .args(["runtime", "env", "gc", "--repo"])
        .arg(&fixture.root)
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).starts_with("MAVEN_PURGE_IDENTITY_MISMATCH"));
    assert!(wrong_prefix.join("must-survive").is_file());
}

fn environment_directory(fixture: &RuntimeFixture) -> PathBuf {
    std::fs::read_dir(&fixture.state_root)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.join("owner.json").is_file())
        .unwrap()
}

fn wait_for_file(path: &Path) {
    for _ in 0..200 {
        if path.is_file() {
            return;
        }
        let _ = Command::new("sleep").arg("0.01").status();
    }
    panic!("timed out waiting for {}", path.display());
}

fn write_gc_state(
    fixture: &RuntimeFixture,
    stale_repo: &Path,
    environment_id: &str,
    owner_key: &str,
    inventory_environment_id: &str,
) {
    let directory = fixture.state_root.join(environment_id);
    std::fs::create_dir_all(&directory).unwrap();
    let identity = EnvironmentIdentity {
        canonical_repo: stale_repo.to_path_buf(),
        mode: "local".into(),
        generation: Some("stale-generation".into()),
        environment_id: environment_id.into(),
        owner_key: owner_key.into(),
    };
    let plan = ResourcePlan::new(identity.clone(), None, None).unwrap();
    let owner = EnvironmentOwner {
        schema_version: 1,
        identity,
        host: "test-host".into(),
        created_at_unix_ms: 1,
        manifest_digest: plan.digest.clone(),
        lifecycle: EnvironmentLifecycle::Active,
    };
    let inventory = ResourceInventory {
        schema_version: 1,
        environment_id: inventory_environment_id.into(),
        ..ResourceInventory::default()
    };
    autospec_core::runtime_env::write_json_atomic(&directory.join("owner.json"), &owner).unwrap();
    autospec_core::runtime_env::write_json_atomic(&directory.join("plan.json"), &plan).unwrap();
    autospec_core::runtime_env::write_json_atomic(&directory.join("inventory.json"), &inventory)
        .unwrap();
}

fn set_inventory_ports(
    fixture: &RuntimeFixture,
    environment_id: &str,
    frontend: Option<u16>,
    backend: Option<u16>,
    export: Option<u16>,
) {
    let path = fixture
        .state_root
        .join(environment_id)
        .join("inventory.json");
    let mut inventory: ResourceInventory = read_json(&path).unwrap();
    inventory.frontend_port = frontend;
    inventory.backend_port = backend;
    inventory.exports = export
        .map(|port| ResolvedExport {
            env: "TEST_URL".into(),
            host: "127.0.0.1".into(),
            port,
        })
        .into_iter()
        .collect();
    autospec_core::runtime_env::write_json_atomic(&path, &inventory).unwrap();
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
    let fixture = RuntimeFixture::empty();
    let pid = fixture.root.join("server.pid");
    std::fs::create_dir_all(fixture.root.join(".autospec")).unwrap();
    std::fs::write(
        fixture.root.join(".autospec/runtime.yml"),
        format!(
            "version: 1\ndefault_mode: local\nmodes:\n  local:\n    command: sh -c 'grep -q Provisioning \"$AGENT_ENV_STATE_ROOT/$AGENT_ENV_ID/owner.json\" || exit 1; touch provisioning-seen; python3 -m http.server \"$AGENT_FRONTEND_PORT\" --bind 127.0.0.1 >/dev/null 2>&1 & echo $! > {pid}'\n    down: sh -c 'grep -q TearingDown \"$AGENT_ENV_STATE_ROOT/$AGENT_ENV_ID/owner.json\" || exit 1; touch teardown-seen; kill \"$(cat {pid})\"'\n",
            pid = pid.display()
        ),
    )
    .unwrap();
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
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.join("owner.json").is_file())
        .unwrap();
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
        "version: 2\ndefault_mode: local\nmodes:\n  local:\n    command: {command}\nresources:\n  compose:\n    files: [compose.yaml, compose.override.yaml]\n    exports:\n      - service: web\n        target: 8080\n        protocol: http\n        env: WEB_URL\n        value: url\n"
    )
}

fn compose_manifest_with_preserved_cache(command: &str) -> String {
    format!(
        "{}    preserve_volumes: [cache]\n",
        compose_manifest(command)
    )
}

#[cfg(unix)]
fn replace_exported_value(path: &Path, key: &str, value: &str) {
    let source = std::fs::read_to_string(path).unwrap();
    let prefix = format!("export {key}=");
    let replaced = source
        .lines()
        .map(|line| {
            if line.starts_with(&prefix) {
                format!("{prefix}'{value}'")
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(path, format!("{replaced}\n")).unwrap();
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
fn compose_lifecycle_rejects_caller_project_before_active_reuse() {
    let fixture = RuntimeFixture::with_manifest(&compose_manifest("sh -c 'true'"));
    std::fs::write(fixture.root.join("compose.yaml"), "services: {}\n").unwrap();
    std::fs::write(fixture.root.join("compose.override.yaml"), "services: {}\n").unwrap();
    let bin = fixture.install_lifecycle_fake_docker();
    let log = fixture.root.join("lifecycle-docker.log");
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap());
    let up = fixture
        .command()
        .env("PATH", &path)
        .env("FAKE_DOCKER_LOG", &log)
        .args(["runtime", "env", "up", "--repo"])
        .arg(&fixture.root)
        .output()
        .unwrap();
    assert!(
        up.status.success(),
        "{}",
        String::from_utf8_lossy(&up.stderr)
    );
    let calls_before = std::fs::read_to_string(&log).unwrap().lines().count();

    let reused = fixture
        .command()
        .env("PATH", path)
        .env("FAKE_DOCKER_LOG", &log)
        .env("COMPOSE_PROJECT_NAME", "caller-owned")
        .args(["runtime", "env", "up", "--repo"])
        .arg(&fixture.root)
        .output()
        .unwrap();

    assert_eq!(reused.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&reused.stderr).contains("COMPOSE_PROJECT_NAME_CALLER_OVERRIDE")
    );
    assert_eq!(
        std::fs::read_to_string(log).unwrap().lines().count(),
        calls_before
    );
}

#[test]
#[cfg(unix)]
fn compose_lifecycle_records_actual_ids_and_exports_dynamic_state() {
    let fixture = RuntimeFixture::with_manifest(&compose_manifest(
        "sh -c 'test \"$WEB_URL\" = http://127.0.0.1:49152'",
    ));
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
fn compose_lifecycle_persists_partial_up_and_down_recovers() {
    let fixture = RuntimeFixture::with_manifest(&compose_manifest("sh -c 'true'"));
    std::fs::write(fixture.root.join("compose.yaml"), "services: {}\n").unwrap();
    std::fs::write(fixture.root.join("compose.override.yaml"), "services: {}\n").unwrap();
    let bin = fixture.install_lifecycle_fake_docker();
    let log = fixture.root.join("lifecycle-docker.log");
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap());

    let failed = fixture
        .command()
        .env("PATH", &path)
        .env("FAKE_DOCKER_LOG", &log)
        .env("FAKE_UP_EXIT", "37")
        .args(["runtime", "env", "up", "--repo"])
        .arg(&fixture.root)
        .output()
        .unwrap();
    assert_eq!(failed.status.code(), Some(37));
    assert!(String::from_utf8_lossy(&failed.stderr).contains("COMPOSE_UP_FAILED"));

    let identity = EnvironmentIdentity::resolve(&fixture.root, "local", None).unwrap();
    let environment = fixture.state_root.join(&identity.environment_id);
    let inventory: ResourceInventory = read_json(&environment.join("inventory.json")).unwrap();
    assert!(inventory.compose_project.is_some());
    assert_eq!(inventory.containers, vec!["container-a"]);
    assert_eq!(inventory.networks, vec!["network-a"]);
    assert_eq!(inventory.volumes[0].id, "volume-a");
    let owner: EnvironmentOwner = read_json(&environment.join("owner.json")).unwrap();
    assert_eq!(owner.lifecycle, EnvironmentLifecycle::CleanupFailed);

    let recovered = fixture
        .command()
        .env("PATH", path)
        .env("FAKE_DOCKER_LOG", &log)
        .args(["runtime", "env", "down", "--repo"])
        .arg(&fixture.root)
        .output()
        .unwrap();
    assert!(
        recovered.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&recovered.stderr),
        std::fs::read_to_string(&log).unwrap_or_default()
    );
    assert!(!environment.join("owner.json").exists());
}

#[test]
#[cfg(unix)]
fn compose_lifecycle_rejects_tampered_child_environment_before_spawn() {
    let fixture = RuntimeFixture::with_manifest(&compose_manifest("sh -c 'true'"));
    std::fs::write(fixture.root.join("compose.yaml"), "services: {}\n").unwrap();
    std::fs::write(fixture.root.join("compose.override.yaml"), "services: {}\n").unwrap();
    let bin = fixture.install_lifecycle_fake_docker();
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap());
    let up = fixture
        .command()
        .env("PATH", &path)
        .env("FAKE_DOCKER_LOG", fixture.root.join("lifecycle-docker.log"))
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
    let env_file = fixture.state_root.join(identity.environment_id).join("env");
    let mut source = std::fs::read_to_string(&env_file).unwrap();
    source.push_str("export LD_PRELOAD='/tmp/not-owned.so'\n");
    std::fs::write(&env_file, source).unwrap();
    let marker = fixture.root.join("tampered-child-started");

    let exec = fixture
        .command()
        .env("PATH", path)
        .env("FAKE_DOCKER_LOG", fixture.root.join("lifecycle-docker.log"))
        .args(["runtime", "env", "exec", "--repo"])
        .arg(&fixture.root)
        .args(["--", "sh", "-c"])
        .arg(format!("touch '{}'", marker.display()))
        .output()
        .unwrap();

    assert_eq!(exec.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&exec.stderr).contains("RUNTIME_CHILD_ENV_UNDECLARED: LD_PRELOAD"),
        "{}",
        String::from_utf8_lossy(&exec.stderr)
    );
    assert!(!marker.exists());
}

#[test]
#[cfg(unix)]
fn compose_lifecycle_rejects_tampered_authoritative_values_before_spawn() {
    for (key, value) in [
        ("AGENT_ENV_ID", "foreign-environment"),
        ("COMPOSE_PROJECT_NAME", "foreign-project"),
        ("MODE_SENTINEL", "foreign-mode-value"),
        ("WEB_URL", "http://127.0.0.1:1"),
    ] {
        let manifest = compose_manifest("sh -c 'true'").replace(
            "    command: sh -c 'true'\n",
            "    command: sh -c 'true'\n    env:\n      MODE_SENTINEL: declared\n",
        );
        let fixture = RuntimeFixture::with_manifest(&manifest);
        std::fs::write(fixture.root.join("compose.yaml"), "services: {}\n").unwrap();
        std::fs::write(fixture.root.join("compose.override.yaml"), "services: {}\n").unwrap();
        let bin = fixture.install_lifecycle_fake_docker();
        let log = fixture.root.join("lifecycle-docker.log");
        let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap());
        let up = fixture
            .command()
            .env("PATH", &path)
            .env("FAKE_DOCKER_LOG", &log)
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
        replace_exported_value(
            &fixture.state_root.join(identity.environment_id).join("env"),
            key,
            value,
        );
        let marker = fixture.root.join(format!("spawned-{key}"));

        let exec = fixture
            .command()
            .env("PATH", &path)
            .env("FAKE_DOCKER_LOG", &log)
            .args(["runtime", "env", "exec", "--repo"])
            .arg(&fixture.root)
            .args(["--", "sh", "-c"])
            .arg(format!("touch '{}'", marker.display()))
            .output()
            .unwrap();

        assert_eq!(exec.status.code(), Some(2), "{key}");
        assert!(
            String::from_utf8_lossy(&exec.stderr).contains("RUNTIME_CHILD_ENV_VALUE_MISMATCH"),
            "{key}: {}",
            String::from_utf8_lossy(&exec.stderr)
        );
        assert!(!marker.exists(), "{key} reached the child process");
    }
}

#[test]
#[cfg(unix)]
fn status_rejects_tampered_allowed_value_without_printing_it() {
    let fixture = RuntimeFixture::with_manifest(&compose_manifest("sh -c 'true'"));
    std::fs::write(fixture.root.join("compose.yaml"), "services: {}\n").unwrap();
    std::fs::write(fixture.root.join("compose.override.yaml"), "services: {}\n").unwrap();
    let bin = fixture.install_lifecycle_fake_docker();
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap());
    let log = fixture.root.join("status-docker.log");
    let up = fixture
        .command()
        .env("PATH", &path)
        .env("FAKE_DOCKER_LOG", &log)
        .args(["runtime", "env", "up", "--repo"])
        .arg(&fixture.root)
        .output()
        .unwrap();
    assert!(up.status.success());
    let identity = EnvironmentIdentity::resolve(&fixture.root, "local", None).unwrap();
    let tampered = "http://tampered.invalid:1";
    replace_exported_value(
        &fixture.state_root.join(identity.environment_id).join("env"),
        "WEB_URL",
        tampered,
    );

    let status = fixture
        .command()
        .env("PATH", path)
        .env("FAKE_DOCKER_LOG", log)
        .args(["runtime", "env", "status", "--repo"])
        .arg(&fixture.root)
        .output()
        .unwrap();

    assert_eq!(status.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&status.stderr);
    assert!(
        stderr.contains("RUNTIME_CHILD_ENV_VALUE_MISMATCH: WEB_URL"),
        "{stderr}"
    );
    assert!(!String::from_utf8_lossy(&status.stdout).contains(tampered));
}

#[test]
fn supported_initial_overrides_survive_reuse_exec_and_session_without_caller_env() {
    let fixture = RuntimeFixture::empty();
    let pid = fixture.root.join("server.pid");
    std::fs::create_dir_all(fixture.root.join(".autospec")).unwrap();
    std::fs::write(
        fixture.root.join(".autospec/runtime.yml"),
        format!(
            "version: 1\ndefault_mode: local\nmodes:\n  local:\n    command: sh -c 'python3 -m http.server \"$AGENT_FRONTEND_PORT\" --bind 127.0.0.1 >/dev/null 2>&1 & echo $! > {}'\n",
            pid.display()
        ),
    )
    .unwrap();
    let overrides = [
        ("AGENT_FRONTEND_PORT", "45101"),
        ("AGENT_BACKEND_PORT", "45102"),
        ("AGENT_PUBLIC_URL", "http://agent.override.test"),
        ("AUTOSPEC_PUBLIC_URL", "https://autospec.override.test"),
        ("COMPOSE_PROJECT_NAME", "caller_compose"),
    ];
    let mut first = fixture.command();
    for (key, value) in overrides {
        first.env(key, value);
    }
    first.env("UNDECLARED_OVERRIDE", "must-not-persist");
    let first = first
        .args(["runtime", "env", "up", "--repo"])
        .arg(&fixture.root)
        .output()
        .unwrap();
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let identity = EnvironmentIdentity::resolve(&fixture.root, "local", None).unwrap();
    let inventory: ResourceInventory = read_json(
        &fixture
            .state_root
            .join(identity.environment_id)
            .join("inventory.json"),
    )
    .unwrap();
    assert_eq!(
        inventory.initial_overrides,
        overrides.map(|(key, value)| (key.to_string(), value.to_string()))
    );

    let reused = fixture
        .command()
        .args(["runtime", "env", "up", "--repo"])
        .arg(&fixture.root)
        .output()
        .unwrap();
    assert!(
        reused.status.success(),
        "{}",
        String::from_utf8_lossy(&reused.stderr)
    );
    let protocol = String::from_utf8_lossy(&reused.stdout);
    for (key, value) in overrides {
        assert!(
            protocol.contains(&format!("{key}={value}")),
            "{key}: {protocol}"
        );
    }

    for (operation, marker) in [("exec", "exec-overrides"), ("session", "session-overrides")] {
        let output = fixture
            .command()
            .args(["runtime", "env", operation, "--repo"])
            .arg(&fixture.root)
            .args(["--", "sh", "-c"])
            .arg(format!(
                "printf '%s|%s|%s|%s|%s' \"$AGENT_FRONTEND_PORT\" \"$AGENT_BACKEND_PORT\" \"$AGENT_PUBLIC_URL\" \"$AUTOSPEC_PUBLIC_URL\" \"$COMPOSE_PROJECT_NAME\" > {marker}"
            ))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{operation}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            std::fs::read_to_string(fixture.root.join(marker)).unwrap(),
            "45101|45102|http://agent.override.test|https://autospec.override.test|caller_compose"
        );
    }
    if let Ok(server_pid) = std::fs::read_to_string(pid) {
        let _ = Command::new("kill").arg(server_pid.trim()).status();
    }
}

#[test]
#[cfg(unix)]
fn maven_lifecycle_rejects_tampered_repository_argument_before_spawn() {
    let fixture = RuntimeFixture::with_manifest(
        "version: 1\ndefault_mode: local\nmodes:\n  local:\n    command: sh -c 'true'\n",
    );
    std::fs::write(fixture.root.join("pom.xml"), "<project/>\n").unwrap();
    let bin = fixture.install_fake_maven();
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap());
    let up = fixture
        .command()
        .env("PATH", &path)
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
    replace_exported_value(
        &fixture.state_root.join(identity.environment_id).join("env"),
        "MAVEN_ARGS",
        "-Dmaven.repo.local=/tmp/foreign-repository",
    );
    let marker = fixture.root.join("tampered-maven-child-started");

    let exec = fixture
        .command()
        .env("PATH", path)
        .args(["runtime", "env", "exec", "--repo"])
        .arg(&fixture.root)
        .args(["--", "sh", "-c"])
        .arg(format!("touch '{}'", marker.display()))
        .output()
        .unwrap();

    assert_eq!(exec.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&exec.stderr)
        .contains("RUNTIME_CHILD_ENV_VALUE_MISMATCH: MAVEN_ARGS"));
    assert!(!marker.exists());
}

#[test]
#[cfg(unix)]
fn compose_canonical_ambiguity_fails_before_any_docker_side_effect() {
    let manifest = "version: 2\ndefault_mode: local\nmodes:\n  local:\n    command: sh -c 'true'\nresources:\n  compose:\n    files: [compose.yaml]\n    exports:\n      - { service: web, target: 8080, protocol: http, env: WEB_URL, value: url }\n      - { service: admin, target: 8081, protocol: https, env: ADMIN_URL, value: url }\n";
    let fixture = RuntimeFixture::with_manifest(manifest);
    std::fs::write(fixture.root.join("compose.yaml"), "services: {}\n").unwrap();
    let (bin, model) = fixture.install_fake_docker(&serde_json::json!({}));

    let output = compose_command(&fixture, &bin, &model)
        .args(["runtime", "env", "up", "--repo"])
        .arg(&fixture.root)
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("COMPOSE_CANONICAL_URL_AMBIGUOUS"));
    assert!(!fixture.root.join("docker-arguments.log").exists());
}

#[test]
#[cfg(unix)]
fn compose_without_http_exports_provisions_tcp_udp_and_http_host_port() {
    for (exports, protocol) in [
        ("", "tcp"),
        (
            "    exports:\n      - { service: web, target: 8080, protocol: tcp, env: WEB_PORT, value: port }\n",
            "tcp",
        ),
        (
            "    exports:\n      - { service: web, target: 8080, protocol: udp, env: WEB_UDP_PORT, value: port }\n",
            "udp",
        ),
    ] {
        let manifest = format!("version: 2\ndefault_mode: local\nmodes:\n  local:\n    command: sh -c 'true'\nresources:\n  compose:\n    files: [compose.yaml]\n{exports}");
        let fixture = RuntimeFixture::with_manifest(&manifest);
        std::fs::write(fixture.root.join("compose.yaml"), "services: {}\n").unwrap();
        let model = if exports.is_empty() {
            serde_json::json!({"services":{"web":{"image":"nginx"}}})
        } else {
            serde_json::json!({"services":{"web":{"ports":[{"target":8080,"protocol":protocol}]}}})
        };
        let (bin, model) = fixture.install_fake_docker(&model);
        let output = compose_command(&fixture, &bin, &model)
            .args(["runtime", "env", "up", "--repo"])
            .arg(&fixture.root)
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    }

    let manifest = "version: 2\ndefault_mode: local\nmodes:\n  local:\n    command: sh -c 'test \"$WEB_HOST\" = 127.0.0.1:49152 && test \"$AUTOSPEC_PUBLIC_URL\" = http://127.0.0.1:49152 && test \"$AGENT_PUBLIC_URL\" = http://127.0.0.1:49152'\nresources:\n  compose:\n    files: [compose.yaml]\n    exports:\n      - { service: web, target: 8080, protocol: http, env: WEB_HOST, value: host-port }\n";
    let fixture = RuntimeFixture::with_manifest(manifest);
    std::fs::write(fixture.root.join("compose.yaml"), "services: {}\n").unwrap();
    let bin = fixture.install_lifecycle_fake_docker();
    let output = fixture
        .command()
        .env(
            "PATH",
            format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
        )
        .env("FAKE_DOCKER_LOG", fixture.root.join("docker.log"))
        .args(["runtime", "env", "up", "--repo"])
        .arg(&fixture.root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
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
        .env("FAKE_LABEL_MISMATCH", "1")
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
            "COMPOSE_OWNERSHIP_MISMATCH: recovery: autospec runtime env down {}repo '{}' {}mode 'local'\n",
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
fn compose_lifecycle_inspect_failure_is_not_treated_as_absence() {
    let fixture = RuntimeFixture::with_manifest(&compose_manifest("sh -c 'true'"));
    std::fs::write(fixture.root.join("compose.yaml"), "services: {}\n").unwrap();
    std::fs::write(fixture.root.join("compose.override.yaml"), "services: {}\n").unwrap();
    let bin = fixture.install_lifecycle_fake_docker();
    let log = fixture.root.join("lifecycle-docker.log");
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap());
    let up = fixture
        .command()
        .env("PATH", &path)
        .env("FAKE_DOCKER_LOG", &log)
        .args(["runtime", "env", "up", "--repo"])
        .arg(&fixture.root)
        .output()
        .unwrap();
    assert!(
        up.status.success(),
        "{}",
        String::from_utf8_lossy(&up.stderr)
    );

    let down = fixture
        .command()
        .env("PATH", path)
        .env("FAKE_DOCKER_LOG", &log)
        .env("FAKE_INSPECT_EXIT", "42")
        .args(["runtime", "env", "down", "--repo"])
        .arg(&fixture.root)
        .output()
        .unwrap();

    assert_eq!(down.status.code(), Some(42));
    assert!(String::from_utf8_lossy(&down.stderr).contains("COMPOSE_OWNERSHIP_CHECK_FAILED"));
    assert!(!std::fs::read_to_string(log).unwrap().contains("rm -f -v"));
}

#[test]
#[cfg(unix)]
fn compose_lifecycle_partial_exact_delete_completes_on_retry() {
    let fixture = RuntimeFixture::with_manifest(&compose_manifest("sh -c 'true'"));
    std::fs::write(fixture.root.join("compose.yaml"), "services: {}\n").unwrap();
    std::fs::write(fixture.root.join("compose.override.yaml"), "services: {}\n").unwrap();
    let bin = fixture.install_lifecycle_fake_docker();
    let log = fixture.root.join("lifecycle-docker.log");
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap());
    let up = fixture
        .command()
        .env("PATH", &path)
        .env("FAKE_DOCKER_LOG", &log)
        .args(["runtime", "env", "up", "--repo"])
        .arg(&fixture.root)
        .output()
        .unwrap();
    assert!(
        up.status.success(),
        "{}",
        String::from_utf8_lossy(&up.stderr)
    );

    let first = fixture
        .command()
        .env("PATH", &path)
        .env("FAKE_DOCKER_LOG", &log)
        .env("FAKE_NETWORK_RM_ONCE", "1")
        .args(["runtime", "env", "down", "--repo"])
        .arg(&fixture.root)
        .output()
        .unwrap();
    assert_eq!(first.status.code(), Some(41));
    assert!(String::from_utf8_lossy(&first.stderr).contains("COMPOSE_NETWORK_DELETE_FAILED"));

    let second = fixture
        .command()
        .env("PATH", path)
        .env("FAKE_DOCKER_LOG", &log)
        .env("FAKE_NETWORK_RM_ONCE", "1")
        .args(["runtime", "env", "down", "--repo"])
        .arg(&fixture.root)
        .output()
        .unwrap();
    assert!(
        second.status.success(),
        "{}",
        String::from_utf8_lossy(&second.stderr)
    );
    let identity = EnvironmentIdentity::resolve(&fixture.root, "local", None).unwrap();
    assert!(!fixture
        .state_root
        .join(identity.environment_id)
        .join("owner.json")
        .exists());
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
        .env("FAKE_LOGICAL_VOLUME", "cache")
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
    let manifest = "version: 2\ndefault_mode: local\nmodes:\n  local:\n    command: sh -c 'true'\nresources:\n  compose:\n    files: [compose.yaml]\n    exports:\n      - service: web\n        target: 8080\n        protocol: http\n        env: WEB_URL\n        value: url\n";
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
        assert!(
            output.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
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
