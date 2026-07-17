use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use autospec_core::runtime_env::{
    write_json_atomic, EnvironmentLifecycle, EnvironmentOwner, ResourceInventory, ResourcePlan,
};

static NEXT_FIXTURE: AtomicUsize = AtomicUsize::new(0);

struct RuntimeFixture {
    root: PathBuf,
    state_root: PathBuf,
}

impl RuntimeFixture {
    fn counted() -> Self {
        Self::with_manifest(
            "version: 1\nmodes:\n  local:\n    command: sh -c 'echo up >> up-count.txt'\n    down: sh -c 'echo down >> down-count.txt'\n",
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
fn partial_authoritative_state_fails_closed_for_status_and_down() {
    let fixture = RuntimeFixture::counted();
    assert!(runtime(&fixture, "up").status.success());
    let environment = environment_dir(&fixture);
    std::fs::remove_file(environment.join("plan.json")).unwrap();

    for operation in ["status", "down"] {
        let output = runtime(&fixture, operation);
        assert_eq!(output.status.code(), Some(2));
        assert!(String::from_utf8_lossy(&output.stderr).contains("RUNTIME_PARTIAL_STATE"));
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
        assert!(String::from_utf8_lossy(&output.stderr).contains("RUNTIME_OWNER_MISMATCH"));
        assert_eq!(line_count(&fixture.root.join("down-count.txt")), 0);
    }
}

#[test]
fn cleanup_command_failure_persists_cleanup_failed_owner() {
    let fixture = RuntimeFixture::with_manifest(
        "version: 1\nmodes:\n  local:\n    command: sh -c 'true'\n    down: sh -c 'exit 42'\n",
    );
    assert!(runtime(&fixture, "up").status.success());
    assert_eq!(runtime(&fixture, "down").status.code(), Some(42));
    let owner: EnvironmentOwner =
        autospec_core::runtime_env::read_json(&environment_dir(&fixture).join("owner.json"))
            .unwrap();
    assert_eq!(owner.lifecycle, EnvironmentLifecycle::CleanupFailed);
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
        .next()
        .unwrap()
        .unwrap()
        .path()
}

fn line_count(path: &Path) -> usize {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .count()
}
