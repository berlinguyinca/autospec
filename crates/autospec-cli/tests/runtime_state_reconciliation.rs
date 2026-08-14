#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::os::unix::fs::{symlink, PermissionsExt};

use autospec_core::runtime_env::{
    write_json_atomic, EnvironmentLifecycle, EnvironmentOwner, ResourceInventory, ResourcePlan,
};

static NEXT_FIXTURE: AtomicUsize = AtomicUsize::new(0);

struct RuntimeFixture {
    root: PathBuf,
    state_root: PathBuf,
}

struct SessionGuard(Option<std::process::Child>);

impl SessionGuard {
    fn new(child: std::process::Child) -> Self {
        Self(Some(child))
    }

    fn id(&self) -> u32 {
        self.0.as_ref().expect("session child exists").id()
    }

    fn wait_with_output(mut self) -> std::io::Result<std::process::Output> {
        self.0
            .take()
            .expect("session child exists")
            .wait_with_output()
    }
}

impl Drop for SessionGuard {
    fn drop(&mut self) {
        let Some(mut child) = self.0.take() else {
            return;
        };
        #[cfg(unix)]
        let _ = Command::new("kill")
            .args(["-TERM", &child.id().to_string()])
            .output();
        #[cfg(not(unix))]
        let _ = child.kill();
        let _ = child.wait();
    }
}

impl RuntimeFixture {
    fn counted() -> Self {
        Self::with_manifest(
            "version: 1\nmodes:\n  local:\n    command: sh -c 'python3 -m http.server \"$AGENT_FRONTEND_PORT\" > server.log 2>&1 & echo $! > server.pid; echo up >> up-count.txt'\n    down: sh -c 'kill \"$(cat server.pid)\"; echo down >> down-count.txt'\n",
        )
    }

    fn deferred() -> Self {
        Self::with_manifest(
            "version: 1\nmodes:\n  local:\n    command: sh -c 'echo up >> up-count.txt'\n    down: sh -c 'echo down >> down-count.txt'\n    readiness: deferred\n",
        )
    }

    fn with_manifest(manifest: &str) -> Self {
        let suffix = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "autospec-runtime-state-{}-{suffix}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".autospec")).unwrap();
        std::fs::write(root.join(".autospec/runtime.yml"), manifest).unwrap();
        let state_root = root.join("state");
        Self { root, state_root }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_autospec"));
        command.env("AGENT_ENV_STATE_ROOT", &self.state_root);
        command
    }
}

impl Drop for RuntimeFixture {
    fn drop(&mut self) {
        stop_fixture_server(&self.root);
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn up_reconciles_same_plan_empty_inventory_without_running_down() {
    let fixture = RuntimeFixture::counted();
    assert!(runtime(&fixture, "up").status.success());
    let environment = environment_dir(&fixture);
    let mut owner: EnvironmentOwner =
        autospec_core::runtime_env::read_json(&environment.join("owner.json")).unwrap();
    owner.lifecycle = EnvironmentLifecycle::Provisioning;
    write_json_atomic(&environment.join("owner.json"), &owner).unwrap();

    let second = runtime(&fixture, "up");

    assert!(
        second.status.success(),
        "{}",
        String::from_utf8_lossy(&second.stderr)
    );
    assert_eq!(line_count(&fixture.root.join("up-count.txt")), 2);
    assert_eq!(line_count(&fixture.root.join("down-count.txt")), 0);
}

#[test]
fn up_rejects_resource_empty_cleanup_failed_state_without_rerunning_setup() {
    let fixture = RuntimeFixture::deferred();
    assert!(runtime(&fixture, "up").status.success());
    let environment = environment_dir(&fixture);
    let mut owner: EnvironmentOwner =
        autospec_core::runtime_env::read_json(&environment.join("owner.json")).unwrap();
    owner.lifecycle = EnvironmentLifecycle::CleanupFailed;
    write_json_atomic(&environment.join("owner.json"), &owner).unwrap();

    let second = runtime(&fixture, "up");

    assert_eq!(second.status.code(), Some(2));
    assert!(stderr_has(&second, "RUNTIME_LIFECYCLE_MISMATCH"));
    assert_eq!(line_count(&fixture.root.join("up-count.txt")), 1);
    assert_eq!(line_count(&fixture.root.join("down-count.txt")), 0);
    assert_authoritative_state_retained(&fixture, &environment);
}

#[test]
fn partial_authoritative_state_fails_closed_for_status_and_down() {
    let fixture = RuntimeFixture::counted();
    assert!(runtime(&fixture, "up").status.success());
    let environment = environment_dir(&fixture);
    std::fs::remove_file(environment.join("plan.json")).unwrap();

    for operation in ["status", "down"] {
        let output = runtime(&fixture, operation);
        assert_eq!(output.status.code(), Some(2));
        assert!(stderr_has(&output, "RUNTIME_PARTIAL_STATE"));
    }
    assert!(environment.join("owner.json").is_file());
    assert_eq!(line_count(&fixture.root.join("down-count.txt")), 0);
}

#[test]
fn provisioning_reconciliation_rejects_plan_or_inventory_mismatch() {
    for mutation in ["plan", "inventory"] {
        let fixture = RuntimeFixture::counted();
        assert!(runtime(&fixture, "up").status.success());
        let environment = environment_dir(&fixture);
        let mut owner: EnvironmentOwner =
            autospec_core::runtime_env::read_json(&environment.join("owner.json")).unwrap();
        owner.lifecycle = EnvironmentLifecycle::Provisioning;
        write_json_atomic(&environment.join("owner.json"), &owner).unwrap();
        if mutation == "plan" {
            let mut plan: ResourcePlan =
                autospec_core::runtime_env::read_json(&environment.join("plan.json")).unwrap();
            plan.digest = "different-plan".to_string();
            write_json_atomic(&environment.join("plan.json"), &plan).unwrap();
        } else {
            let mut inventory: ResourceInventory =
                autospec_core::runtime_env::read_json(&environment.join("inventory.json")).unwrap();
            inventory.containers.push("owned-container".to_string());
            write_json_atomic(&environment.join("inventory.json"), &inventory).unwrap();
        }
        let output = runtime(&fixture, "up");
        assert_eq!(output.status.code(), Some(2), "{mutation}");
        assert_eq!(line_count(&fixture.root.join("down-count.txt")), 0);
    }
}

#[test]
fn identity_mismatch_fails_closed_without_cleanup() {
    for mutation in ["owner", "inventory"] {
        let fixture = RuntimeFixture::counted();
        assert!(runtime(&fixture, "up").status.success());
        let environment = environment_dir(&fixture);
        if mutation == "owner" {
            let mut owner: EnvironmentOwner =
                autospec_core::runtime_env::read_json(&environment.join("owner.json")).unwrap();
            owner.identity.environment_id = "other-environment".to_string();
            write_json_atomic(&environment.join("owner.json"), &owner).unwrap();
        } else {
            let mut inventory: ResourceInventory =
                autospec_core::runtime_env::read_json(&environment.join("inventory.json")).unwrap();
            inventory.environment_id = "other-environment".to_string();
            write_json_atomic(&environment.join("inventory.json"), &inventory).unwrap();
        }
        let output = runtime(&fixture, "down");
        assert_eq!(output.status.code(), Some(2), "{mutation}");
        assert!(stderr_has(&output, "RUNTIME_OWNER_MISMATCH"));
        assert_eq!(line_count(&fixture.root.join("down-count.txt")), 0);
    }
}

#[test]
fn cleanup_command_failure_persists_cleanup_failed_owner() {
    let fixture = RuntimeFixture::with_manifest(
        "version: 1\nmodes:\n  local:\n    command: sh -c 'python3 -m http.server \"$AGENT_FRONTEND_PORT\" > server.log 2>&1 & echo $! > server.pid'\n    down: sh -c 'kill \"$(cat server.pid)\"; exit 42'\n",
    );
    assert!(runtime(&fixture, "up").status.success());
    assert_eq!(runtime(&fixture, "down").status.code(), Some(42));
    let owner: EnvironmentOwner =
        autospec_core::runtime_env::read_json(&environment_dir(&fixture).join("owner.json"))
            .unwrap();
    assert_eq!(owner.lifecycle, EnvironmentLifecycle::CleanupFailed);
}

#[test]
fn explicit_down_rejects_nonempty_inventory_without_cleanup() {
    let fixture = RuntimeFixture::counted();
    assert!(runtime(&fixture, "up").status.success());
    let environment = environment_dir(&fixture);
    seed_owned_container(&environment);

    let down = runtime(&fixture, "down");

    assert_inventory_cleanup_blocked(&fixture, &environment, &down);
}

#[test]
fn final_session_release_rejects_nonempty_inventory_without_cleanup() {
    let fixture = RuntimeFixture::counted();
    let ready = fixture.root.join("session.ready");
    let release = fixture.root.join("session.release");
    let session = SessionGuard::new(
        fixture
            .command()
            .args(["runtime", "env", "session", "--repo"])
            .arg(&fixture.root)
            .args([
                "--",
                "sh",
                "-c",
                "touch session.ready; while test ! -f session.release; do sleep 0.02; done",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("session starts"),
    );
    wait_for(&ready);
    let environment = environment_dir(&fixture);
    seed_owned_container(&environment);
    std::fs::write(&release, "release\n").unwrap();

    let output = session.wait_with_output().expect("session exits");

    assert_inventory_cleanup_blocked(&fixture, &environment, &output);
}

#[test]
fn persisted_plan_identity_mismatch_fails_closed_without_cleanup() {
    let fixture = RuntimeFixture::counted();
    assert!(runtime(&fixture, "up").status.success());
    let environment = environment_dir(&fixture);
    let plan_path = environment.join("plan.json");
    let mut plan: ResourcePlan = autospec_core::runtime_env::read_json(&plan_path).unwrap();
    plan.identity.environment_id = "forged-environment".to_string();
    write_json_atomic(&plan_path, &plan).unwrap();

    for operation in ["status", "down"] {
        let output = runtime(&fixture, operation);
        assert_eq!(output.status.code(), Some(2));
        assert!(stderr_has(&output, "RUNTIME_PLAN_MISMATCH"));
    }
    assert_authoritative_state_retained(&fixture, &environment);
}

#[test]
fn authoritative_schema_mismatch_fails_closed_without_cleanup() {
    for document in ["owner", "plan", "inventory"] {
        let fixture = RuntimeFixture::counted();
        assert!(runtime(&fixture, "up").status.success());
        let environment = environment_dir(&fixture);
        set_schema_version(&environment, document, 2);

        for operation in ["status", "down"] {
            let output = runtime(&fixture, operation);
            assert_eq!(output.status.code(), Some(2), "{document} {operation}");
            assert!(stderr_has(&output, "RUNTIME_SCHEMA_MISMATCH"));
        }
        assert_authoritative_state_retained(&fixture, &environment);
    }
}

#[cfg(unix)]
#[test]
fn runtime_state_directories_and_files_are_private() {
    let fixture = RuntimeFixture::counted();
    let up = runtime(&fixture, "up");
    assert!(
        up.status.success(),
        "{}",
        String::from_utf8_lossy(&up.stderr)
    );
    let environment = environment_dir(&fixture);
    let session = fixture
        .command()
        .args(["runtime", "env", "session", "--keep-alive", "--repo"])
        .arg(&fixture.root)
        .args(["--", "sh", "-c", "true"])
        .output()
        .unwrap();
    assert!(session.status.success());

    for directory in [
        &fixture.state_root,
        &environment,
        &environment.join("sessions"),
    ] {
        assert_eq!(mode(directory), 0o700, "{}", directory.display());
    }
    for file in [
        "owner.json",
        "plan.json",
        "inventory.json",
        "env",
        "lease.lock",
    ] {
        let path = environment.join(file);
        assert_eq!(mode(&path), 0o600, "{}", path.display());
    }
}

#[cfg(unix)]
#[test]
fn down_rejects_a_symlinked_environment_root_before_cleanup() {
    let fixture = RuntimeFixture::counted();
    assert!(runtime(&fixture, "up").status.success());
    let environment = environment_dir(&fixture);
    let actual = fixture.root.join("relocated-environment");
    std::fs::rename(&environment, &actual).unwrap();
    symlink(&actual, &environment).unwrap();
    let down_count = line_count(&fixture.root.join("down-count.txt"));

    let output = runtime(&fixture, "down");

    assert_eq!(output.status.code(), Some(2));
    assert!(stderr_has(&output, "RUNTIME_STATE_SYMLINK_REJECTED"));
    assert_eq!(line_count(&fixture.root.join("down-count.txt")), down_count);
    assert!(actual.join("owner.json").is_file());
}

#[cfg(unix)]
#[test]
fn down_rejects_a_symlinked_session_root_before_cleanup() {
    let fixture = RuntimeFixture::counted();
    assert!(runtime(&fixture, "up").status.success());
    let environment = environment_dir(&fixture);
    let external = fixture.root.join("external-sessions");
    std::fs::create_dir(&external).unwrap();
    std::fs::remove_dir(environment.join("sessions")).unwrap();
    symlink(&external, environment.join("sessions")).unwrap();
    let down_count = line_count(&fixture.root.join("down-count.txt"));

    let output = runtime(&fixture, "down");

    assert_eq!(output.status.code(), Some(2));
    assert!(stderr_has(&output, "RUNTIME_STATE_SYMLINK_REJECTED"));
    assert_eq!(line_count(&fixture.root.join("down-count.txt")), down_count);
    assert!(environment.join("owner.json").is_file());
}

#[cfg(unix)]
#[test]
fn outer_sigterm_stops_the_provisioning_process_tree() {
    let fixture = RuntimeFixture::with_manifest(
        "version: 1\nmodes:\n  local:\n    command: sh -c 'echo $$ > provision.pid; touch provision.ready; while :; do sleep 1; done'\n    down: sh -c 'echo down >> down-count.txt'\n",
    );
    let outer = SessionGuard::new(
        fixture
            .command()
            .args(["runtime", "env", "session", "--repo"])
            .arg(&fixture.root)
            .args(["--", "sh", "-c", "true"])
            .spawn()
            .expect("session wrapper starts"),
    );
    wait_for(&fixture.root.join("provision.ready"));
    let provisioning_pid: u32 = std::fs::read_to_string(fixture.root.join("provision.pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    send_signal(outer.id(), 15);

    let output = outer.wait_with_output().expect("session wrapper exits");
    let descendant_stopped = wait_for_process_exit(provisioning_pid);
    if !descendant_stopped {
        send_signal(provisioning_pid, 9);
    }

    assert_eq!(output.status.code(), Some(143));
    assert!(descendant_stopped, "provisioning descendant remained alive");
}

fn runtime(fixture: &RuntimeFixture, operation: &str) -> std::process::Output {
    fixture
        .command()
        .args(["runtime", "env", operation, "--repo"])
        .arg(&fixture.root)
        .output()
        .expect("runtime command starts")
}

fn environment_dir(fixture: &RuntimeFixture) -> PathBuf {
    std::fs::read_dir(&fixture.state_root)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.join("owner.json").is_file())
        .unwrap()
}

fn line_count(path: &Path) -> usize {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .count()
}

fn seed_owned_container(environment: &Path) {
    let path = environment.join("inventory.json");
    let mut inventory: ResourceInventory = autospec_core::runtime_env::read_json(&path).unwrap();
    inventory.containers.push("owned-container".to_string());
    write_json_atomic(&path, &inventory).unwrap();
}

fn assert_inventory_cleanup_blocked(
    fixture: &RuntimeFixture,
    environment: &Path,
    output: &std::process::Output,
) {
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr_has(output, "RUNTIME_INVENTORY_NOT_EMPTY"));
    let owner: EnvironmentOwner =
        autospec_core::runtime_env::read_json(&environment.join("owner.json")).unwrap();
    let inventory: ResourceInventory =
        autospec_core::runtime_env::read_json(&environment.join("inventory.json")).unwrap();
    assert_eq!(owner.lifecycle, EnvironmentLifecycle::CleanupFailed);
    assert_eq!(inventory.containers, ["owned-container"]);
    assert!(environment.join("plan.json").is_file());
    assert_eq!(line_count(&fixture.root.join("down-count.txt")), 0);
}

fn wait_for(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !path.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(path.exists(), "timed out waiting for {}", path.display());
}

fn set_schema_version(environment: &Path, document: &str, version: u32) {
    let path = environment.join(format!("{document}.json"));
    let mut value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    value["schema_version"] = serde_json::Value::from(version);
    std::fs::write(path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
}

fn assert_authoritative_state_retained(fixture: &RuntimeFixture, environment: &Path) {
    for name in ["owner.json", "plan.json", "inventory.json"] {
        assert!(environment.join(name).is_file(), "missing {name}");
    }
    assert_eq!(line_count(&fixture.root.join("down-count.txt")), 0);
}

#[cfg(unix)]
fn send_signal(pid: u32, signal: i32) {
    assert!(Command::new("kill")
        .arg(format!("-{signal}"))
        .arg(pid.to_string())
        .status()
        .unwrap()
        .success());
}

#[cfg(unix)]
fn wait_for_process_exit(pid: u32) -> bool {
    let deadline = Instant::now() + Duration::from_secs(5);
    while process_alive(pid) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    !process_alive(pid)
}

#[cfg(unix)]
fn process_alive(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .output()
        .is_ok_and(|output| output.status.success())
}

fn stderr_has(output: &std::process::Output, marker: &str) -> bool {
    String::from_utf8_lossy(&output.stderr)
        .lines()
        .any(|line| line.starts_with(marker))
}

fn stop_fixture_server(root: &Path) {
    let Ok(pid) = std::fs::read_to_string(root.join("server.pid")) else {
        return;
    };
    let _ = Command::new("kill").arg(pid.trim()).output();
}

#[cfg(unix)]
fn mode(path: &Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}
