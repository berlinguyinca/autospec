use std::fs;
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
    fn empty() -> Self {
        let suffix = NEXT_RUNTIME_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "autospec-runtime-cli-{}-{suffix}",
            std::process::id()
        ));
        let state_root = root.join("state");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create runtime fixture root");
        Self { root, state_root }
    }

    fn new(command: &str, down: &str) -> Self {
        let fixture = Self::empty();
        std::fs::create_dir_all(fixture.root.join(".autospec"))
            .expect("create runtime manifest directory");
        std::fs::write(
            fixture.root.join(".autospec/runtime.yml"),
            format!(
                "version: 1\nname: sample-app\ndefault_mode: local\nmodes:\n  local:\n    command: {command}\n    down: {down}\n"
            ),
        )
        .expect("write runtime manifest");
        fixture
    }

    fn command(&self) -> Command {
        let mut command = autospec();
        command.env("AGENT_ENV_STATE_ROOT", &self.state_root);
        command
    }

    fn has_session_record(&self) -> bool {
        std::fs::read_dir(&self.state_root)
            .ok()
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .any(|entry| {
                std::fs::read_dir(entry.path().join("sessions"))
                    .ok()
                    .into_iter()
                    .flatten()
                    .any(|record| record.is_ok())
            })
    }
}

impl Drop for RuntimeFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn runtime_env_init_writes_and_protects_the_conservative_manifest() {
    let fixture = RuntimeFixture::empty();
    let output = fixture
        .command()
        .args([
            "runtime",
            "env",
            "init",
            "--repo",
            fixture.root.to_str().expect("fixture path is UTF-8"),
        ])
        .output()
        .expect("runtime env init starts");

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let manifest = fixture.root.join(".agent-runtime.yml");
    let name = fixture
        .root
        .file_name()
        .and_then(|value| value.to_str())
        .expect("fixture basename");
    assert_eq!(
        std::fs::read_to_string(&manifest).expect("agent manifest exists"),
        format!(
            "version: 1\nname: {name}\ndefault_mode: local\nmodes:\n  local:\n    command: sh -c 'true'\n    down: sh -c 'true'\nports:\n  frontend:\n    env: AGENT_FRONTEND_PORT\n    default: dynamic\npublic_url_env:\n  - AUTOSPEC_PUBLIC_URL\n  - AGENT_PUBLIC_URL\n"
        )
    );
    let protected = fixture
        .command()
        .args([
            "runtime",
            "env",
            "init",
            "--repo",
            fixture.root.to_str().expect("fixture path is UTF-8"),
        ])
        .output()
        .expect("second runtime env init starts");
    assert_eq!(protected.status.code(), Some(4));
    assert!(String::from_utf8_lossy(&protected.stderr).contains("already exists"));
}

#[test]
fn runtime_parent_help_lists_the_complete_environment_command_family() {
    let output = autospec()
        .args(["runtime", "--help"])
        .output()
        .expect("runtime help starts");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout
        .contains("autospec runtime env init [--repo PATH] [--manifest agent|autospec] [--force]"));
    assert!(stdout.contains("autospec runtime env up [--repo PATH] [--mode MODE]"));
    assert!(stdout.contains("autospec runtime env status [--repo PATH] [--mode MODE]"));
    assert!(stdout.contains("autospec runtime env down [--repo PATH] [--mode MODE]"));
    assert!(stdout
        .contains("autospec runtime env exec [--repo PATH] [--mode MODE] -- COMMAND [ARGS...]"));
    assert!(stdout.contains("autospec runtime env session [--repo PATH] [--mode MODE] [--keep-alive] -- COMMAND [ARGS...]"));
}

#[test]
fn runtime_env_unquotes_quoted_manifest_command_and_down_scalars() {
    let fixture = RuntimeFixture::empty();
    std::fs::create_dir_all(fixture.root.join(".autospec"))
        .expect("create runtime manifest directory");
    std::fs::write(
        fixture.root.join(".autospec/runtime.yml"),
        "version: 1\nname: quoted-app\ndefault_mode: local\nmodes:\n  local:\n    command: \"sh -c 'printf up > quoted-up.txt'\"\n    down: 'sh -c \"printf down > quoted-down.txt\"'\n",
    )
    .expect("write quoted command manifest");

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
    assert!(
        up.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&up.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(fixture.root.join("quoted-up.txt")).expect("quoted command ran"),
        "up"
    );

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
    assert_eq!(
        std::fs::read_to_string(fixture.root.join("quoted-down.txt")).expect("quoted teardown ran"),
        "down"
    );
}

#[test]
fn runtime_env_treats_an_empty_state_root_as_unset() {
    let fixture = RuntimeFixture::new("sh -c 'true'", "sh -c 'true'");
    let home = fixture.root.join("home");
    std::fs::create_dir_all(&home).expect("create fixture home");
    let args = [
        "runtime",
        "env",
        "up",
        "--repo",
        fixture.root.to_str().expect("fixture path is UTF-8"),
    ];

    let unset = autospec()
        .current_dir(&fixture.root)
        .env("HOME", &home)
        .env_remove("AGENT_ENV_STATE_ROOT")
        .args(args)
        .output()
        .expect("runtime env with unset state root starts");
    assert!(
        unset.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&unset.stderr)
    );

    let empty = autospec()
        .current_dir(&fixture.root)
        .env("HOME", &home)
        .env("AGENT_ENV_STATE_ROOT", "")
        .args(args)
        .output()
        .expect("runtime env with empty state root starts");
    assert!(
        empty.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&empty.stderr)
    );
    assert_eq!(empty.stdout, unset.stdout);
}

#[test]
fn runtime_env_exec_runs_a_direct_child_in_the_provisioned_environment() {
    let fixture = RuntimeFixture::new("sh -c 'true'", "sh -c 'true'");
    let output = fixture
        .command()
        .args([
            "runtime",
            "env",
            "exec",
            "--repo",
            fixture.root.to_str().expect("fixture path is UTF-8"),
            "--",
            "sh",
            "-c",
            "printf '%s|%s' \"$AGENT_ENV_ID\" \"$AUTOSPEC_PUBLIC_URL\" > exec.txt",
        ])
        .output()
        .expect("runtime env exec starts");

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let child_output =
        std::fs::read_to_string(fixture.root.join("exec.txt")).expect("direct child output");
    let (environment_id, public_url) = child_output
        .split_once('|')
        .expect("direct child output has both values");
    assert!(environment_id.starts_with("sample-app-"));
    assert!(public_url.starts_with("http://127.0.0.1:"));
    let status = fixture
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
    assert!(status.status.success());
}

#[test]
fn runtime_env_session_cleans_state_after_child_completion() {
    let fixture = RuntimeFixture::new("sh -c 'true'", "sh -c 'printf down > down.txt'");
    let output = fixture
        .command()
        .args([
            "runtime",
            "env",
            "session",
            "--repo",
            fixture.root.to_str().expect("fixture path is UTF-8"),
            "--",
            "sh",
            "-c",
            "exit 0",
        ])
        .output()
        .expect("runtime env session starts");

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(fixture.root.join("down.txt")).expect("teardown ran"),
        "down"
    );
    let status = fixture
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
    assert_eq!(status.status.code(), Some(3));
}

#[test]
fn runtime_env_session_writes_its_record_before_starting_the_child() {
    let fixture = RuntimeFixture::new("sh -c 'true'", "sh -c 'true'");
    let output = fixture
        .command()
        .args([
            "runtime",
            "env",
            "session",
            "--repo",
            fixture.root.to_str().expect("fixture path is UTF-8"),
            "--",
            "sh",
            "-c",
            "if find \"$AGENT_ENV_STATE_ROOT\" -path '*/sessions/*' -type f -print -quit | grep -q .; then printf present > record-order.txt; else printf missing > record-order.txt; fi",
        ])
        .output()
        .expect("runtime env session starts");

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(fixture.root.join("record-order.txt")).expect("child order marker"),
        "present"
    );
}

#[test]
fn runtime_env_session_cleans_up_when_its_child_cannot_start() {
    let fixture = RuntimeFixture::new("sh -c 'true'", "sh -c 'printf down > down.txt'");
    let output = fixture
        .command()
        .args([
            "runtime",
            "env",
            "session",
            "--repo",
            fixture.root.to_str().expect("fixture path is UTF-8"),
            "--",
            "autospec-runtime-command-that-does-not-exist",
        ])
        .output()
        .expect("runtime env session reports child spawn failure");

    assert_eq!(output.status.code(), Some(2));
    assert!(!fixture.has_session_record());
    assert!(std::fs::read_dir(&fixture.state_root)
        .map(|mut entries| entries.next().is_none())
        .unwrap_or(true));
    assert_eq!(
        std::fs::read_to_string(fixture.root.join("down.txt")).expect("teardown ran"),
        "down"
    );
}

#[test]
fn runtime_env_session_bypasses_a_manifest_when_disabled() {
    let fixture = RuntimeFixture::new("sh -c 'exit 41'", "sh -c 'exit 42'");
    let output = fixture
        .command()
        .env("AUTOSPEC_ENV_DISABLE", "1")
        .args([
            "runtime",
            "env",
            "session",
            "--repo",
            fixture.root.to_str().expect("fixture path is UTF-8"),
            "--",
            "sh",
            "-c",
            "pwd > bypass.txt",
        ])
        .output()
        .expect("disabled runtime env session starts");

    assert!(output.status.success());
    assert_eq!(
        std::fs::read_to_string(fixture.root.join("bypass.txt"))
            .expect("bypass child output")
            .trim(),
        std::fs::canonicalize(&fixture.root)
            .expect("canonical fixture root")
            .to_str()
            .expect("fixture path is UTF-8")
    );
    assert!(!fixture.state_root.exists());
}

#[test]
fn runtime_env_session_passes_through_without_a_manifest() {
    let fixture = RuntimeFixture::empty();
    let output = fixture
        .command()
        .args([
            "runtime",
            "env",
            "session",
            "--repo",
            fixture.root.to_str().expect("fixture path is UTF-8"),
            "--",
            "sh",
            "-c",
            "printf pass-through > direct.txt",
        ])
        .output()
        .expect("no-manifest runtime env session starts");

    assert!(output.status.success());
    assert_eq!(
        std::fs::read_to_string(fixture.root.join("direct.txt")).expect("direct child output"),
        "pass-through"
    );
    assert!(!fixture.root.join(".agent-runtime.yml").exists());
    assert!(!fixture.state_root.exists());
}

#[test]
fn runtime_env_session_auto_initializes_and_keeps_state_when_requested() {
    let fixture = RuntimeFixture::empty();
    let output = fixture
        .command()
        .env("AUTOSPEC_ENV_AUTO_INIT", "1")
        .env("AUTOSPEC_ENV_KEEP_ALIVE", "1")
        .args([
            "runtime",
            "env",
            "session",
            "--repo",
            fixture.root.to_str().expect("fixture path is UTF-8"),
            "--",
            "sh",
            "-c",
            "test -f .agent-runtime.yml && printf auto > auto-init.txt",
        ])
        .output()
        .expect("auto-init runtime env session starts");

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(fixture.root.join(".agent-runtime.yml").is_file());
    assert_eq!(
        std::fs::read_to_string(fixture.root.join("auto-init.txt"))
            .expect("auto-init child output"),
        "auto"
    );
    let status = fixture
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
    assert!(status.status.success());
    assert!(!fixture.has_session_record());
}

#[cfg(unix)]
fn send_signal(pid: u32, signal: i32) {
    let status = Command::new("kill")
        .arg(format!("-{signal}"))
        .arg(pid.to_string())
        .status()
        .expect("kill command starts");
    assert!(status.success(), "kill command succeeds");
}

#[cfg(unix)]
#[test]
fn runtime_env_session_tears_down_after_sigterm() {
    use std::time::{Duration, Instant};

    let fixture = RuntimeFixture::new("sh -c 'true'", "sh -c 'printf down > down.txt'");
    let session = fixture
        .command()
        .args([
            "runtime",
            "env",
            "session",
            "--repo",
            fixture.root.to_str().expect("fixture path is UTF-8"),
            "--",
            "sleep",
            "30",
        ])
        .spawn()
        .expect("runtime env session starts");
    let deadline = Instant::now() + Duration::from_secs(5);
    while !fixture.has_session_record() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(fixture.has_session_record(), "session record was created");
    send_signal(session.id(), 15);
    let output = session.wait_with_output().expect("session process exits");

    assert_eq!(output.status.code(), Some(143));
    assert!(!fixture.has_session_record());
    assert!(std::fs::read_dir(&fixture.state_root)
        .map(|mut entries| entries.next().is_none())
        .unwrap_or(true));
    assert_eq!(
        std::fs::read_to_string(fixture.root.join("down.txt")).expect("teardown ran"),
        "down"
    );
}

#[cfg(unix)]
#[test]
fn runtime_env_session_signal_exit_takes_precedence_over_failed_teardown() {
    use std::time::{Duration, Instant};

    for (signal, expected_exit) in [(2, 130), (15, 143)] {
        let fixture = RuntimeFixture::new("sh -c 'true'", "sh -c 'exit 42'");
        let session = fixture
            .command()
            .args([
                "runtime",
                "env",
                "session",
                "--repo",
                fixture.root.to_str().expect("fixture path is UTF-8"),
                "--",
                "sleep",
                "30",
            ])
            .spawn()
            .expect("runtime env session starts");
        let deadline = Instant::now() + Duration::from_secs(5);
        while !fixture.has_session_record() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(fixture.has_session_record(), "session record was created");
        send_signal(session.id(), signal);
        let output = session.wait_with_output().expect("session process exits");

        assert_eq!(output.status.code(), Some(expected_exit));
        assert!(!fixture.has_session_record());
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

    let status = fixture
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
    assert!(
        status.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&status.stderr)
    );
    assert_eq!(second.stdout, status.stdout);
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

#[test]
fn legacy_agent_env_authority_is_absent() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("workspace root")
        .to_path_buf();
    let tracked = Command::new("git")
        .args([
            "-C",
            root.to_str().expect("workspace path is UTF-8"),
            "ls-files",
        ])
        .output()
        .expect("git lists tracked source paths");
    assert!(
        tracked.status.success(),
        "git ls-files failed: {}",
        String::from_utf8_lossy(&tracked.stderr)
    );

    let legacy_filename = ["agent-env", ".sh"].concat();
    let legacy_path = format!("scripts/{legacy_filename}");
    let mut live_references = Vec::new();
    for path in String::from_utf8_lossy(&tracked.stdout).lines() {
        if path == legacy_path {
            continue;
        }
        let source = fs::read(root.join(path))
            .unwrap_or_else(|error| panic!("read tracked source {path}: {error}"));
        let source = String::from_utf8_lossy(&source);
        for (line_number, line) in source.lines().enumerate() {
            let trimmed = line.trim_start();
            let is_negative_test_assertion =
                path.starts_with("tests/") && trimmed.starts_with("! grep");
            let is_approved_history_reference = match path {
                "docs/superpowers/specs/2026-07-14-rust-control-plane-completion-design.md" => {
                    line.starts_with("Implement a typed `.autospec/runtime.yml` model with ")
                }
                "docs/superpowers/plans/2026-07-14-rust-runtime-broker.md" => {
                    line.starts_with("**Goal:** Replace `scripts/")
                        || line.starts_with("- Do not retain `scripts/")
                        || line.starts_with("- Delete: `scripts/")
                        || line.starts_with("Make the three autospec-run bodies require ")
                        || line.starts_with("Expected: failures because installer wrappers ")
                        || line.starts_with(
                            "Add a Rust integration test that reads tracked source paths ",
                        )
                        || line.starts_with("Expected: failure naming `scripts/")
                        || line.starts_with("git add -A scripts/")
                }
                _ => false,
            };
            if line.contains(&legacy_filename)
                && !trimmed.starts_with('#')
                && !is_negative_test_assertion
                && !is_approved_history_reference
            {
                live_references.push(format!("{path}:{}", line_number + 1));
            }
        }
    }

    assert!(
        !root.join(&legacy_path).exists(),
        "legacy authority remains at {legacy_path}"
    );
    assert!(
        live_references.is_empty(),
        "live references to {legacy_filename}: {}",
        live_references.join(", ")
    );
}
