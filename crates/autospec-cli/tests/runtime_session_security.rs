use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

static NEXT_FIXTURE: AtomicUsize = AtomicUsize::new(0);

struct RuntimeFixture {
    root: PathBuf,
    state_root: PathBuf,
}

impl RuntimeFixture {
    fn with_manifest(manifest: &str) -> Self {
        let suffix = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "autospec-runtime-session-security-{}-{suffix}",
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
fn session_worker_marker_is_absent_from_manifest_and_harness_commands() {
    let fixture = RuntimeFixture::with_manifest(
        "version: 1\nmodes:\n  local:\n    command: sh -c 'printf \"%s:%s:%s\\n\" \"${AUTOSPEC_RUNTIME_SESSION_WORKER-unset}\" \"${AUTOSPEC_RUNTIME_SESSION_HANDOFF-unset}\" \"${AUTOSPEC_RUNTIME_SESSION_TOKEN-unset}\" > up-marker.txt'\n    down: sh -c 'printf \"%s:%s:%s\\n\" \"${AUTOSPEC_RUNTIME_SESSION_WORKER-unset}\" \"${AUTOSPEC_RUNTIME_SESSION_HANDOFF-unset}\" \"${AUTOSPEC_RUNTIME_SESSION_TOKEN-unset}\" > down-marker.txt'\n",
    );

    let output = fixture
        .command()
        .args(["runtime", "env", "session", "--repo"])
        .arg(&fixture.root)
        .args([
            "--",
            "sh",
            "-c",
            "printf '%s:%s:%s\\n' \"${AUTOSPEC_RUNTIME_SESSION_WORKER-unset}\" \"${AUTOSPEC_RUNTIME_SESSION_HANDOFF-unset}\" \"${AUTOSPEC_RUNTIME_SESSION_TOKEN-unset}\" > harness-marker.txt",
        ])
        .output()
        .expect("session starts");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    for name in ["up-marker.txt", "harness-marker.txt", "down-marker.txt"] {
        assert_eq!(
            std::fs::read_to_string(fixture.root.join(name)).unwrap(),
            "unset:unset:unset\n"
        );
    }
}

#[cfg(unix)]
#[test]
fn stale_worker_marker_cannot_bypass_the_supervisor() {
    let fixture = RuntimeFixture::with_manifest(
        "version: 1\nmodes:\n  local:\n    command: sh -c 'true'\n    down: sh -c 'true'\n",
    );
    let mut outer = fixture.command();
    outer
        .env("AUTOSPEC_RUNTIME_SESSION_WORKER", "1")
        .args(["runtime", "env", "session", "--repo"])
        .arg(&fixture.root)
        .args([
            "--",
            "sh",
            "-c",
            "echo $$ > harness.pid; touch harness.ready; while test ! -f harness.release; do sleep 0.02; done",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut outer = outer.spawn().expect("session supervisor starts");
    wait_for(&fixture.root.join("harness.ready"));
    let harness_pid = read_pid(&fixture.root.join("harness.pid"));

    send_signal(outer.id(), "-KILL");
    assert!(wait_for_exit(&mut outer, Duration::from_secs(2)).is_some());
    let down = runtime_down(&fixture);
    std::fs::write(fixture.root.join("harness.release"), "release\n").unwrap();
    let harness_stopped = wait_for_process_exit(harness_pid);
    if !harness_stopped {
        send_signal(harness_pid, "-KILL");
    }

    assert_eq!(down.status.code(), Some(2));
    assert!(output_stderr_has(&down, "RUNTIME_LIVE_SESSIONS"));
    assert!(harness_stopped, "harness remained alive after release");
}

#[cfg(unix)]
#[test]
fn heartbeat_failure_kills_the_harness_group_and_reports_cleanup_failure() {
    let fixture = RuntimeFixture::with_manifest(
        "version: 1\nmodes:\n  local:\n    command: sh -c 'true'\n    down: sh -c 'true'\n",
    );
    let stderr_path = fixture.root.join("heartbeat-stderr.txt");
    let stderr = File::create(&stderr_path).unwrap();
    let mut outer = fixture.command();
    outer
        .args(["runtime", "env", "session", "--repo"])
        .arg(&fixture.root)
        .args([
            "--",
            "sh",
            "-c",
            "echo $$ > harness.pid; sleep 1000 & echo $! > grandchild.pid; touch harness.ready; wait",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::from(stderr));
    let mut outer = outer.spawn().expect("session supervisor starts");
    wait_for(&fixture.root.join("harness.ready"));
    let harness_pid = read_pid(&fixture.root.join("harness.pid"));
    let grandchild_pid = read_pid(&fixture.root.join("grandchild.pid"));
    replace_sessions_directory_with_file(&fixture);

    let status = wait_for_exit(&mut outer, Duration::from_secs(4));
    let harness_stopped = wait_for_process_exit(harness_pid);
    let grandchild_stopped = wait_for_process_exit(grandchild_pid);
    let stderr = std::fs::read_to_string(stderr_path).unwrap();
    for (pid, stopped) in [
        (harness_pid, harness_stopped),
        (grandchild_pid, grandchild_stopped),
    ] {
        if !stopped {
            send_signal(pid, "-KILL");
        }
    }

    assert_eq!(status.and_then(|value| value.code()), Some(2), "{stderr}");
    assert!(harness_stopped, "harness leader remained alive");
    assert!(grandchild_stopped, "harness grandchild remained alive");
    assert!(
        text_line_has(&stderr, "runtime session cleanup also failed"),
        "{stderr}"
    );
}

#[cfg(unix)]
#[test]
fn group_signal_failure_returns_diagnostic_and_reaps_worker() {
    let fixture = RuntimeFixture::with_manifest(
        "version: 1\nmodes:\n  local:\n    command: sh -c 'true'\n    down: sh -c 'true'\n",
    );
    let kill_log = fixture.root.join("kill-args.txt");
    let fake_bin = install_failing_kill(&fixture.root);
    let stderr_path = fixture.root.join("outer-stderr.txt");
    let stderr = File::create(&stderr_path).unwrap();
    let inherited_path = std::env::var_os("PATH").unwrap_or_default();
    let mut outer = fixture.command();
    outer
        .env("PATH", joined_path(&fake_bin, &inherited_path))
        .env("KILL_ARGS_FILE", &kill_log)
        .args(["runtime", "env", "session", "--repo"])
        .arg(&fixture.root)
        .args([
            "--",
            "sh",
            "-c",
            "echo $$ > harness.pid; touch harness.ready; while :; do sleep 1; done",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::from(stderr));
    let mut outer = outer.spawn().expect("session supervisor starts");
    wait_for(&fixture.root.join("harness.ready"));

    send_signal(outer.id(), "-TERM");
    let started = Instant::now();
    let status = wait_for_exit(&mut outer, Duration::from_secs(3));
    let stderr = std::fs::read_to_string(stderr_path).unwrap();
    let args = std::fs::read_to_string(&kill_log).unwrap_or_default();
    let worker_pid = asserted_worker_pid(&args);
    cleanup_harness(&fixture.root);

    assert_eq!(status.and_then(|value| value.code()), Some(2), "{stderr}");
    assert!(
        text_line_has(&stderr, "RUNTIME_PROCESS_GROUP_SIGNAL_FAILED"),
        "{stderr}"
    );
    assert!(started.elapsed() < Duration::from_secs(3));
    assert!(
        !process_alive(worker_pid),
        "worker {worker_pid} remained alive"
    );
}

#[cfg(unix)]
fn install_failing_kill(root: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let fake_bin = root.join("fake-bin");
    std::fs::create_dir(&fake_bin).unwrap();
    let path = fake_bin.join("kill");
    std::fs::write(
        &path,
        "#!/bin/sh\nprintf '%s\\n' \"$@\" >> \"$KILL_ARGS_FILE\"\nexit 42\n",
    )
    .unwrap();
    let mut permissions = std::fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions).unwrap();
    fake_bin
}

#[cfg(unix)]
fn joined_path(fake_bin: &Path, inherited: &std::ffi::OsStr) -> std::ffi::OsString {
    let mut paths = vec![fake_bin.to_path_buf()];
    paths.extend(std::env::split_paths(inherited));
    std::env::join_paths(paths).unwrap()
}

#[cfg(unix)]
fn asserted_worker_pid(args: &str) -> u32 {
    let lines = args.lines().collect::<Vec<_>>();
    assert!(
        lines.len() >= 3,
        "structured kill arguments missing: {args:?}"
    );
    assert_eq!(&lines[..2], ["-TERM", "--"]);
    let group = lines[2].parse::<i32>().expect("negative process group");
    assert!(group < 0, "process group must be negative: {group}");
    group.unsigned_abs()
}

#[cfg(unix)]
fn wait_for_exit(child: &mut Child, timeout: Duration) -> Option<ExitStatus> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Some(status) = child.try_wait().unwrap() {
            return Some(status);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let _ = child.kill();
    let _ = child.wait();
    None
}

#[cfg(unix)]
fn cleanup_harness(root: &Path) {
    let pid = read_pid(&root.join("harness.pid"));
    if process_alive(pid) {
        send_signal(pid, "-KILL");
    }
}

#[cfg(unix)]
fn replace_sessions_directory_with_file(fixture: &RuntimeFixture) {
    let environment = std::fs::read_dir(&fixture.state_root)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let sessions = environment.join("sessions");
    std::fs::remove_dir_all(&sessions).unwrap();
    std::fs::write(sessions, "not a directory\n").unwrap();
}

#[cfg(unix)]
fn runtime_down(fixture: &RuntimeFixture) -> std::process::Output {
    fixture
        .command()
        .args(["runtime", "env", "down", "--repo"])
        .arg(&fixture.root)
        .output()
        .unwrap()
}

#[cfg(unix)]
fn read_pid(path: &Path) -> u32 {
    std::fs::read_to_string(path)
        .unwrap()
        .trim()
        .parse()
        .unwrap()
}

fn wait_for(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !path.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(path.exists(), "timed out waiting for {}", path.display());
}

#[cfg(unix)]
fn send_signal(pid: u32, signal: &str) {
    assert!(Command::new("/bin/kill")
        .args([signal, "--", &pid.to_string()])
        .status()
        .unwrap()
        .success());
}

#[cfg(unix)]
fn process_alive(pid: u32) -> bool {
    Command::new("/bin/kill")
        .args(["-0", "--", &pid.to_string()])
        .output()
        .is_ok_and(|output| output.status.success())
}

fn output_stderr_has(output: &std::process::Output, marker: &str) -> bool {
    text_line_has(&String::from_utf8_lossy(&output.stderr), marker)
}

fn text_line_has(text: &str, marker: &str) -> bool {
    text.lines().any(|line| line.find(marker).is_some())
}

#[cfg(unix)]
fn wait_for_process_exit(pid: u32) -> bool {
    let deadline = Instant::now() + Duration::from_secs(3);
    while process_alive(pid) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    !process_alive(pid)
}
