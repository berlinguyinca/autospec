use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use autospec_core::runtime_env::{
    ComposeIsolation, ComposePlan, EnvironmentIdentity, MavenIsolation, MavenPlan, ResourcePlan,
};

static NEXT_FIXTURE: AtomicUsize = AtomicUsize::new(0);

struct RuntimeFixture {
    root: PathBuf,
    state_root: PathBuf,
}

impl RuntimeFixture {
    fn with_manifest(manifest: &str) -> Self {
        let suffix = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "autospec-runtime-resource-cli-{}-{suffix}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".autospec")).expect("create fixture directory");
        std::fs::write(root.join(".autospec/runtime.yml"), manifest).expect("write manifest");
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
    assert_eq!(plan.maven.unwrap().isolation, MavenIsolation::Off);
    assert_eq!(plan.compose.unwrap().isolation, ComposeIsolation::Off);

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
