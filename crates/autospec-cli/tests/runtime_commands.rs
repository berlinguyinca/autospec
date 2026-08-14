#![cfg(unix)]

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use autospec_core::runtime_policy::{Runtime, RuntimeClass};
use serde_json::{json, Value};

static NEXT_RUNTIME_FIXTURE: AtomicUsize = AtomicUsize::new(0);

fn autospec() -> Command {
    Command::new(env!("CARGO_BIN_EXE_autospec"))
}

#[test]
fn runtime_env_lease_blocks_a_second_process_until_release() {
    use std::time::{Duration, Instant};

    let fixture = RuntimeFixture::empty();
    let environment = fixture.state_root.join("lease-environment");
    let first_ready = fixture.root.join("first.ready");
    let first_release = fixture.root.join("first.release");
    let second_ready = fixture.root.join("second.ready");
    let second_release = fixture.root.join("second.release");

    let first = lease_probe(&environment, &first_ready, &first_release);
    wait_for_file(&first_ready, Duration::from_secs(5));
    let second = lease_probe(&environment, &second_ready, &second_release);
    std::thread::sleep(Duration::from_millis(150));
    assert!(
        !second_ready.exists(),
        "second lease acquired while first was held"
    );

    std::fs::write(&first_release, "release\n").expect("release first probe");
    assert!(first.wait().expect("first probe exits").success());
    wait_for_file(&second_ready, Duration::from_secs(5));
    std::fs::write(&second_release, "release\n").expect("release second probe");
    assert!(second.wait().expect("second probe exits").success());

    fn lease_probe(
        environment: &std::path::Path,
        ready: &std::path::Path,
        release: &std::path::Path,
    ) -> ChildGuard {
        let mut command = autospec();
        command
            .args(["runtime", "env", "lease-probe"])
            .arg(environment)
            .arg(ready)
            .arg(release);
        ChildGuard::spawn(&mut command)
    }

    fn wait_for_file(path: &std::path::Path, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        while !path.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(path.exists(), "timed out waiting for {}", path.display());
    }
}

struct ChildGuard(Option<std::process::Child>);

impl ChildGuard {
    fn spawn(command: &mut Command) -> Self {
        Self(Some(command.spawn().expect("lease probe starts")))
    }

    fn wait(mut self) -> std::io::Result<std::process::ExitStatus> {
        self.0.take().expect("lease probe child is present").wait()
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn audit_fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/runtime-audit")
}

#[derive(Debug, PartialEq, Eq)]
struct RuntimeClassification {
    command: String,
    path: PathBuf,
    runtime: Runtime,
    class: RuntimeClass,
    reasons: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
struct RuntimeClassificationLine {
    class: RuntimeClass,
    path: PathBuf,
    reasons: String,
}

#[derive(Debug, PartialEq, Eq)]
struct RuntimeAudit {
    command: String,
    root: PathBuf,
    r0: Vec<PathBuf>,
    r1: Vec<PathBuf>,
    r2: Vec<PathBuf>,
    r3: Vec<PathBuf>,
    r4: Vec<PathBuf>,
}

impl RuntimeAudit {
    fn add(&mut self, class: RuntimeClass, paths: Vec<PathBuf>) {
        match class {
            RuntimeClass::R0 => self.r0 = paths,
            RuntimeClass::R1 => self.r1 = paths,
            RuntimeClass::R2 => self.r2 = paths,
            RuntimeClass::R3 => self.r3 = paths,
            RuntimeClass::R4 => self.r4 = paths,
        }
    }

    fn paths(&self, class: RuntimeClass) -> &[PathBuf] {
        match class {
            RuntimeClass::R0 => &self.r0,
            RuntimeClass::R1 => &self.r1,
            RuntimeClass::R2 => &self.r2,
            RuntimeClass::R3 => &self.r3,
            RuntimeClass::R4 => &self.r4,
        }
    }

    fn all_paths(&self) -> impl Iterator<Item = &PathBuf> {
        self.r0
            .iter()
            .chain(self.r1.iter())
            .chain(self.r2.iter())
            .chain(self.r3.iter())
            .chain(self.r4.iter())
    }
}

fn parse_runtime_classification(stdout: &[u8]) -> RuntimeClassification {
    let value: Value = serde_json::from_slice(stdout).expect("runtime classify emits JSON");
    RuntimeClassification {
        command: json_string(&value, "command").to_owned(),
        path: PathBuf::from(json_string(&value, "path")),
        runtime: parse_runtime(json_string(&value, "runtime")),
        class: parse_runtime_class(json_string(&value, "class")),
        reasons: value
            .get("reasons")
            .and_then(Value::as_array)
            .expect("runtime classify reasons are an array")
            .iter()
            .map(|reason| {
                reason
                    .as_str()
                    .expect("runtime classify reason is a string")
                    .to_owned()
            })
            .collect(),
    }
}

fn parse_runtime_classification_line(stdout: &[u8]) -> RuntimeClassificationLine {
    let stdout = std::str::from_utf8(stdout).expect("runtime classify text is UTF-8");
    let mut lines = stdout.lines();
    let line = lines.next().expect("runtime classify text has one line");
    assert!(lines.next().is_none(), "runtime classify text has one line");

    let mut fields = line.splitn(3, ' ');
    RuntimeClassificationLine {
        class: parse_runtime_class(fields.next().expect("classification class")),
        path: PathBuf::from(fields.next().expect("classification path")),
        reasons: fields.next().expect("classification reasons").to_owned(),
    }
}

fn parse_runtime_audit(stdout: &[u8]) -> RuntimeAudit {
    let value: Value = serde_json::from_slice(stdout).expect("runtime audit emits JSON");
    let mut audit = RuntimeAudit {
        command: json_string(&value, "command").to_owned(),
        root: PathBuf::from(json_string(&value, "root")),
        r0: Vec::new(),
        r1: Vec::new(),
        r2: Vec::new(),
        r3: Vec::new(),
        r4: Vec::new(),
    };

    for (class, paths) in value
        .get("classes")
        .and_then(Value::as_object)
        .expect("runtime audit classes are an object")
        .iter()
    {
        let paths = paths
            .as_array()
            .expect("runtime audit class entry is an array")
            .iter()
            .map(|path| PathBuf::from(path.as_str().expect("runtime audit path entry is a string")))
            .collect();
        audit.add(parse_runtime_class(class), paths);
    }
    audit
}

fn json_string<'a>(value: &'a Value, key: &str) -> &'a str {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("JSON field {key} is a string"))
}

fn parse_runtime(value: &str) -> Runtime {
    match value {
        "rust" => Runtime::Rust,
        "shell" => Runtime::Shell,
        "python" => Runtime::Python,
        "node" => Runtime::Node,
        "go" => Runtime::Go,
        "unknown" => Runtime::Unknown,
        other => panic!("unknown runtime kind: {other}"),
    }
}

fn parse_runtime_class(value: &str) -> RuntimeClass {
    match value {
        "R0" => RuntimeClass::R0,
        "R1" => RuntimeClass::R1,
        "R2" => RuntimeClass::R2,
        "R3" => RuntimeClass::R3,
        "R4" => RuntimeClass::R4,
        other => panic!("unknown runtime migration class: {other}"),
    }
}

#[test]
fn runtime_command_json_helpers_match_exact_paths() {
    let output = json!({
        "command": "runtime classify",
        "path": "scripts/lint-issue.sh",
        "runtime": "shell",
        "class": "R1",
        "reasons": ["stateful platform behavior belongs in Rust core"],
    })
    .to_string();

    let classification = parse_runtime_classification(output.as_bytes());

    assert_eq!(
        classification,
        RuntimeClassification {
            command: "runtime classify".to_string(),
            path: PathBuf::from("scripts/lint-issue.sh"),
            runtime: Runtime::Shell,
            class: RuntimeClass::R1,
            reasons: vec!["stateful platform behavior belongs in Rust core".to_string()],
        }
    );
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
            fixture.root.join(".autospec/runtime-up.sh"),
            format!(
                "set -eu\n{command}\npython3 -m http.server \"$AGENT_FRONTEND_PORT\" > runtime-server.log 2>&1 &\nprintf '%s\\n' \"$!\" > runtime-server.pid\n"
            ),
        )
        .expect("write runtime up script");
        std::fs::write(
            fixture.root.join(".autospec/runtime-down.sh"),
            format!(
                "set -eu\nif test -f runtime-server.pid; then\n  pid=$(cat runtime-server.pid)\n  if kill -0 \"$pid\" 2>/dev/null; then kill \"$pid\"; fi\n  rm -f runtime-server.pid\nfi\n{down}\n"
            ),
        )
        .expect("write runtime down script");
        std::fs::write(
            fixture.root.join(".autospec/runtime.yml"),
            "version: 1\nname: sample-app\ndefault_mode: local\nmodes:\n  local:\n    command: sh .autospec/runtime-up.sh\n    down: sh .autospec/runtime-down.sh\n",
        )
        .expect("write runtime manifest");
        fixture
    }

    fn raw(command: &str, down: &str) -> Self {
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

    fn has_only_lease_tombstone(&self) -> bool {
        let Ok(environments) = std::fs::read_dir(&self.state_root) else {
            return false;
        };
        let mut environments = environments
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name() != "ports");
        let Some(environment) = environments.next() else {
            return false;
        };
        if environments.next().is_some() {
            return false;
        }
        let Ok(entries) = std::fs::read_dir(environment.path()) else {
            return false;
        };
        let mut lease_found = false;
        for entry in entries.filter_map(Result::ok) {
            if entry.file_name() == "lease.lock" {
                lease_found = true;
            } else if entry.file_name() == "sessions"
                && std::fs::read_dir(entry.path()).is_ok_and(|mut records| records.next().is_none())
            {
                continue;
            } else {
                return false;
            }
        }
        lease_found
    }
}

impl Drop for RuntimeFixture {
    fn drop(&mut self) {
        stop_runtime_server(&self.root);
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn runtime_environment(fixture: &RuntimeFixture) -> PathBuf {
    std::fs::read_dir(&fixture.state_root)
        .expect("state root exists")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.join("owner.json").is_file())
        .expect("runtime environment exists")
}

fn stop_runtime_server(root: &std::path::Path) {
    let Ok(pid) = std::fs::read_to_string(root.join("runtime-server.pid")) else {
        return;
    };
    let _ = Command::new("kill").arg(pid.trim()).output();
}

enum RuntimeEnvCommand {
    Init,
    Up,
    Status,
    Down,
    Exec,
    Session,
}

impl RuntimeEnvCommand {
    const ALL: [Self; 6] = [
        Self::Init,
        Self::Up,
        Self::Status,
        Self::Down,
        Self::Exec,
        Self::Session,
    ];

    fn usage(&self) -> &'static str {
        match self {
            Self::Init => {
                "autospec runtime env init [--repo PATH] [--manifest agent|autospec] [--force]"
            }
            Self::Up => "autospec runtime env up [--repo PATH] [--mode MODE]",
            Self::Status => "autospec runtime env status [--repo PATH] [--mode MODE]",
            Self::Down => "autospec runtime env down [--repo PATH] [--mode MODE]",
            Self::Exec => {
                "autospec runtime env exec [--repo PATH] [--mode MODE] -- COMMAND [ARGS...]"
            }
            Self::Session => {
                "autospec runtime env session [--repo PATH] [--mode MODE] [--keep-alive] -- COMMAND [ARGS...]"
            }
        }
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
    let usage_lines: Vec<&str> = stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    for command in RuntimeEnvCommand::ALL {
        assert!(
            usage_lines.iter().any(|line| *line == command.usage()),
            "missing runtime env usage line {:?} in:\n{stdout}",
            command.usage()
        );
    }
}

#[test]
fn runtime_env_unquotes_quoted_manifest_command_and_down_scalars() {
    let fixture = RuntimeFixture::new("printf up > quoted-up.txt", "printf down > quoted-down.txt");
    std::fs::write(
        fixture.root.join(".autospec/runtime.yml"),
        "version: 1\nname: quoted-app\ndefault_mode: local\nmodes:\n  local:\n    command: \"sh .autospec/runtime-up.sh\"\n    down: 'sh .autospec/runtime-down.sh'\n",
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
    let state_environment = runtime_environment(&fixture);
    let state_environment_id = state_environment
        .file_name()
        .expect("runtime environment has an identifier");
    assert_eq!(state_environment_id.to_string_lossy(), environment_id);
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
    assert!(fixture.has_only_lease_tombstone());
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
fn runtime_env_session_auto_initialized_placeholder_fails_health_until_configured() {
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

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("PORT_BIND_HEALTH_RETRIES_EXHAUSTED"));
    assert!(fixture.root.join(".agent-runtime.yml").is_file());
    assert!(!fixture.root.join("auto-init.txt").exists());
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
    assert!(fixture.has_only_lease_tombstone());
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
    let state_file = runtime_environment(&fixture).join("env");
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
    let fixture = RuntimeFixture::raw("", "sh -c 'true'");

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
        "AGENT_ENV_ID=autospec-runtime-cli-",
        "AGENT_ENV_MODE=local",
        "AGENT_ENV_REPO=",
        "AGENT_ENV_FILE=",
        "AGENT_FRONTEND_PORT=",
        "AGENT_BACKEND_PORT=",
        "AGENT_PUBLIC_URL=http://127.0.0.1:",
        "AUTOSPEC_PUBLIC_URL=http://127.0.0.1:",
        "COMPOSE_PROJECT_NAME=agent_autospec_runtime_cli_",
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
    assert!(fixture.has_only_lease_tombstone());
}

#[test]
fn runtime_commands_json_reports_r1_for_stateful_shell_helpers() {
    let output = autospec()
        .args(["runtime", "classify", "scripts/lint-issue.sh", "--json"])
        .output()
        .expect("autospec runtime classify runs");

    assert!(output.status.success());
    assert_eq!(
        parse_runtime_classification(&output.stdout),
        RuntimeClassification {
            command: "runtime classify".to_string(),
            path: PathBuf::from("scripts/lint-issue.sh"),
            runtime: Runtime::Shell,
            class: RuntimeClass::R1,
            reasons: vec!["stateful platform behavior belongs in Rust core".to_string()],
        }
    );
}

#[test]
fn runtime_commands_text_reports_one_line_with_class_and_path() {
    let output = autospec()
        .args(["runtime", "classify", "scripts/lint-issue.sh"])
        .output()
        .expect("autospec runtime classify runs");

    assert!(output.status.success());
    assert_eq!(
        parse_runtime_classification_line(&output.stdout),
        RuntimeClassificationLine {
            class: RuntimeClass::R1,
            path: PathBuf::from("scripts/lint-issue.sh"),
            reasons: "stateful platform behavior belongs in Rust core".to_string(),
        }
    );
}

#[test]
fn runtime_commands_unknown_paths_return_r2_json() {
    let output = autospec()
        .args(["runtime", "classify", "docs/specs/example.md", "--json"])
        .output()
        .expect("autospec runtime classify runs");

    assert!(output.status.success());
    assert_eq!(
        parse_runtime_classification(&output.stdout),
        RuntimeClassification {
            command: "runtime classify".to_string(),
            path: PathBuf::from("docs/specs/example.md"),
            runtime: Runtime::Unknown,
            class: RuntimeClass::R2,
            reasons: vec!["stable helper; add parity fixture before porting".to_string()],
        }
    );
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

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let audit = parse_runtime_audit(&output.stdout);
    assert_eq!(audit.command, "runtime audit");
    assert_eq!(audit.root, audit_fixture());
    assert_eq!(
        audit.paths(RuntimeClass::R1),
        [
            PathBuf::from("scripts/lint-issue.sh"),
            PathBuf::from("skills/autospec-run/tests/watchdog_claim_timeout.bats"),
        ]
    );
    assert_eq!(
        audit.paths(RuntimeClass::R2),
        [PathBuf::from("packages/example/go.mod")]
    );
    assert_eq!(
        audit.paths(RuntimeClass::R4),
        [PathBuf::from("skills/autospec-fab/scripts/mesh.py")]
    );
    assert!(!audit
        .all_paths()
        .any(|path| path == &PathBuf::from("target/ignored.rs")));
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

    std::fs::remove_dir_all(&temporary).expect("remove temporary fixture");
    assert!(output.status.success());
    assert!(!parse_runtime_audit(&output.stdout)
        .all_paths()
        .any(|path| path == &PathBuf::from("scripts/secret.py")));
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
        let source_path = root.join(path);
        if path == legacy_path || !source_path.is_file() {
            continue;
        }
        let source = fs::read(&source_path)
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
