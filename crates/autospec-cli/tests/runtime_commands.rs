use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_RUNTIME_FIXTURE: AtomicUsize = AtomicUsize::new(0);

fn autospec() -> Command {
    Command::new(env!("CARGO_BIN_EXE_autospec"))
}

fn audit_fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/runtime-audit")
}

struct RuntimeFixture {
    root: PathBuf,
    state_root: PathBuf,
}

impl RuntimeFixture {
    fn new(command: &str, down: &str) -> Self {
        let suffix = NEXT_RUNTIME_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "autospec-runtime-cli-{}-{suffix}",
            std::process::id()
        ));
        let state_root = root.join("state");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".autospec")).expect("create runtime manifest directory");
        std::fs::write(
            root.join(".autospec/runtime.yml"),
            format!(
                "version: 1\nname: sample-app\ndefault_mode: local\nmodes:\n  local:\n    command: {command}\n    down: {down}\n"
            ),
        )
        .expect("write runtime manifest");
        Self { root, state_root }
    }

    fn command(&self) -> Command {
        let mut command = autospec();
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
fn runtime_env_up_preserves_manifest_command_exit_status() {
    let fixture = RuntimeFixture::new("sh -c 'exit 42'", "sh -c 'true'");

    let output = fixture
        .command()
        .args([
            "runtime",
            "env",
            "up",
            "--repo",
            fixture.root.to_str().expect("fixture path is UTF-8"),
        ])
        .output()
        .expect("runtime env up starts");

    assert_eq!(output.status.code(), Some(42));
}

#[test]
fn runtime_env_up_preserves_caller_overrides_in_state_output_and_child() {
    let fixture = RuntimeFixture::new(
        "sh -c 'printf \"%s|%s|%s|%s|%s\\n\" \"$AGENT_FRONTEND_PORT\" \"$AGENT_BACKEND_PORT\" \"$AGENT_PUBLIC_URL\" \"$AUTOSPEC_PUBLIC_URL\" \"$COMPOSE_PROJECT_NAME\" > overrides.txt'",
        "sh -c 'true'",
    );
    let overrides = [
        ("AGENT_FRONTEND_PORT", "45101"),
        ("AGENT_BACKEND_PORT", "45102"),
        ("AGENT_PUBLIC_URL", "http://override.test:45101"),
        ("AUTOSPEC_PUBLIC_URL", "https://autospec.example.test"),
        ("COMPOSE_PROJECT_NAME", "caller_compose"),
    ];

    let mut command = fixture.command();
    command.args([
        "runtime",
        "env",
        "up",
        "--repo",
        fixture.root.to_str().expect("fixture path is UTF-8"),
    ]);
    for (key, value) in overrides {
        command.env(key, value);
    }
    let output = command.output().expect("runtime env up starts");

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let protocol = String::from_utf8_lossy(&output.stdout);
    for (key, value) in overrides {
        assert!(protocol
            .lines()
            .any(|line| line == format!("{key}={value}")));
    }
    let state_file = std::fs::read_dir(&fixture.state_root)
        .expect("state root exists")
        .next()
        .expect("one environment exists")
        .expect("state directory entry")
        .path()
        .join("env");
    let state = std::fs::read_to_string(state_file).expect("read state file");
    for (key, value) in overrides {
        assert!(state.contains(&format!("export {key}='{value}'")));
    }
    assert_eq!(
        std::fs::read_to_string(fixture.root.join("overrides.txt")).expect("read child output"),
        "45101|45102|http://override.test:45101|https://autospec.example.test|caller_compose\n"
    );
}

#[test]
fn runtime_env_up_reports_a_mode_without_a_command() {
    let fixture = RuntimeFixture::new("", "sh -c 'true'");

    let output = fixture
        .command()
        .args([
            "runtime",
            "env",
            "up",
            "--repo",
            fixture.root.to_str().expect("fixture path is UTF-8"),
        ])
        .output()
        .expect("runtime env up starts");

    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("has no command"));
    assert!(std::fs::read_dir(&fixture.state_root)
        .expect("state directory was created before command validation")
        .next()
        .is_some());
}

#[test]
fn runtime_env_rejects_malformed_options_before_creating_state() {
    let fixture = RuntimeFixture::new("sh -c 'true'", "sh -c 'true'");

    let output = fixture
        .command()
        .args(["runtime", "env", "up", "--repo", "--mode"])
        .output()
        .expect("runtime env up starts");

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("--repo requires a path"));
    assert!(!fixture.state_root.exists());
}

#[test]
fn runtime_env_status_reports_inactive_environment_with_exit_three() {
    let fixture = RuntimeFixture::new("sh -c 'true'", "sh -c 'true'");

    let output = fixture
        .command()
        .args([
            "runtime",
            "env",
            "status",
            "--repo",
            fixture.root.to_str().expect("fixture path is UTF-8"),
        ])
        .output()
        .expect("runtime env status starts");

    assert_eq!(output.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&output.stderr).contains("no active environment"));
}

#[test]
fn runtime_env_up_prints_the_legacy_protocol_and_reuses_existing_state() {
    let fixture = RuntimeFixture::new(
        "sh -c 'test ! -e started.txt || exit 41; printf started > started.txt'",
        "sh -c 'true'",
    );

    let first = fixture
        .command()
        .args(["runtime", "env", "up"])
        .arg(format!(
            "--repo={}",
            fixture.root.to_str().expect("fixture path is UTF-8")
        ))
        .output()
        .expect("first runtime env up starts");
    assert!(
        first.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(fixture.root.join("started.txt")).unwrap(),
        "started"
    );
    let first_stdout = String::from_utf8_lossy(&first.stdout);
    let expected_prefixes = [
        "AGENT_ENV_ID=sample-app-",
        "AGENT_ENV_MODE=local",
        "AGENT_ENV_REPO=",
        "AGENT_ENV_FILE=",
        "AGENT_FRONTEND_PORT=",
        "AGENT_BACKEND_PORT=",
        "AGENT_PUBLIC_URL=http://127.0.0.1:",
        "AUTOSPEC_PUBLIC_URL=http://127.0.0.1:",
        "COMPOSE_PROJECT_NAME=agent_sample_app_",
    ];
    assert_eq!(first_stdout.lines().count(), expected_prefixes.len());
    for (line, prefix) in first_stdout.lines().zip(expected_prefixes) {
        assert!(
            line.starts_with(prefix),
            "expected {prefix:?}, got {line:?}"
        );
    }

    let second = fixture
        .command()
        .args([
            "runtime",
            "env",
            "up",
            "--repo",
            fixture.root.to_str().expect("fixture path is UTF-8"),
        ])
        .output()
        .expect("second runtime env up starts");
    assert!(
        second.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&second.stderr)
    );
    assert_eq!(first.stdout, second.stdout);
}

#[test]
fn runtime_env_down_is_idempotent_after_state_cleanup() {
    let fixture = RuntimeFixture::new("sh -c 'true'", "sh -c 'printf down > down.txt'");
    let up = fixture
        .command()
        .args([
            "runtime",
            "env",
            "up",
            "--repo",
            fixture.root.to_str().expect("fixture path is UTF-8"),
        ])
        .output()
        .expect("runtime env up starts");
    assert!(up.status.success());

    for _ in 0..2 {
        let down = fixture
            .command()
            .args([
                "runtime",
                "env",
                "down",
                "--repo",
                fixture.root.to_str().expect("fixture path is UTF-8"),
            ])
            .output()
            .expect("runtime env down starts");
        assert!(
            down.status.success(),
            "stderr={}",
            String::from_utf8_lossy(&down.stderr)
        );
    }
    assert_eq!(
        std::fs::read_to_string(fixture.root.join("down.txt")).unwrap(),
        "down"
    );
    assert!(std::fs::read_dir(&fixture.state_root)
        .map(|mut entries| entries.next().is_none())
        .unwrap_or(true));
}

#[test]
fn runtime_commands_json_reports_r1_for_stateful_shell_helpers() {
    let output = autospec()
        .args(["runtime", "classify", "scripts/lint-issue.sh", "--json"])
        .output()
        .expect("autospec runtime classify runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("\"path\":\"scripts/lint-issue.sh\""));
    assert!(stdout.contains("\"runtime\":\"shell\""));
    assert!(stdout.contains("\"class\":\"R1\""));
    assert!(stdout.contains("\"stateful platform behavior belongs in Rust core\""));
}

#[test]
fn runtime_commands_text_reports_one_line_with_class_and_path() {
    let output = autospec()
        .args(["runtime", "classify", "scripts/lint-issue.sh"])
        .output()
        .expect("autospec runtime classify runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert_eq!(stdout.lines().count(), 1);
    assert!(stdout.starts_with("R1 scripts/lint-issue.sh "));
}

#[test]
fn runtime_commands_unknown_paths_return_r2_json() {
    let output = autospec()
        .args(["runtime", "classify", "docs/specs/example.md", "--json"])
        .output()
        .expect("autospec runtime classify runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("\"path\":\"docs/specs/example.md\""));
    assert!(stdout.contains("\"runtime\":\"unknown\""));
    assert!(stdout.contains("\"class\":\"R2\""));
}

#[test]
fn runtime_audit_json_groups_platform_files_and_skips_build_output() {
    let output = autospec()
        .args([
            "runtime",
            "audit",
            "--root",
            audit_fixture().to_str().expect("fixture path is UTF-8"),
            "--json",
        ])
        .output()
        .expect("autospec runtime audit runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("\"command\":\"runtime audit\""));
    assert!(stdout.contains(
        "\"R1\":[\"scripts/lint-issue.sh\",\"skills/autospec-run/tests/watchdog_claim_timeout.bats\"]"
    ));
    assert!(stdout.contains("\"R2\":[\"packages/example/go.mod\"]"));
    assert!(stdout.contains("\"R4\":[\"skills/autospec-fab/scripts/mesh.py\"]"));
    assert!(!stdout.contains("target/ignored.rs"));
}

#[test]
fn runtime_audit_rejects_missing_root() {
    let output = autospec()
        .args(["runtime", "audit", "--root", "/missing/runtime-audit-root"])
        .output()
        .expect("autospec runtime audit starts");

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("does not exist"));
}

#[cfg(unix)]
#[test]
fn runtime_audit_does_not_follow_a_symlinked_platform_root() {
    let temporary = std::env::temp_dir().join(format!(
        "autospec-runtime-audit-{}-{}",
        std::process::id(),
        "symlink-root"
    ));
    let root = temporary.join("root");
    let outside = temporary.join("outside");
    std::fs::create_dir_all(&root).expect("create fixture root");
    std::fs::create_dir_all(&outside).expect("create outside directory");
    std::fs::write(outside.join("secret.py"), "print('secret')\n").expect("write outside file");
    std::os::unix::fs::symlink(&outside, root.join("scripts")).expect("link scripts root");

    let output = autospec()
        .args([
            "runtime",
            "audit",
            "--root",
            root.to_str().expect("temporary path is UTF-8"),
            "--json",
        ])
        .output()
        .expect("autospec runtime audit runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    std::fs::remove_dir_all(&temporary).expect("remove temporary fixture");
    assert!(output.status.success());
    assert!(!stdout.contains("secret.py"));
}
