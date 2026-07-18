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

    fn healthy(command: &str, down: &str) -> Self {
        let fixture = Self::with_manifest(
            "version: 1\nmodes:\n  local:\n    command: sh .autospec/runtime-up.sh\n    down: sh .autospec/runtime-down.sh\n",
        );
        std::fs::write(
            fixture.root.join(".autospec/runtime-up.sh"),
            format!(
                "set -eu\n{command}\npython3 -m http.server \"$AGENT_FRONTEND_PORT\" > runtime-server.log 2>&1 &\nprintf '%s\\n' \"$!\" > runtime-server.pid\n"
            ),
        )
        .unwrap();
        std::fs::write(
            fixture.root.join(".autospec/runtime-down.sh"),
            format!(
                "set -eu\npid=$(cat runtime-server.pid)\nif /bin/kill -0 \"$pid\" 2>/dev/null; then /bin/kill \"$pid\"; fi\nrm -f runtime-server.pid\n{down}\n"
            ),
        )
        .unwrap();
        fixture
    }
}

impl Drop for RuntimeFixture {
    fn drop(&mut self) {
        stop_runtime_server(&self.root);
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn session_worker_marker_is_absent_from_manifest_and_harness_commands() {
    let fixture = RuntimeFixture::healthy(
        "test -z \"${AUTOSPEC_RUNTIME_SESSION_WORKER-}${AUTOSPEC_RUNTIME_SESSION_HANDOFF-}${AUTOSPEC_RUNTIME_SESSION_TOKEN-}\"",
        "test -z \"${AUTOSPEC_RUNTIME_SESSION_WORKER-}${AUTOSPEC_RUNTIME_SESSION_HANDOFF-}${AUTOSPEC_RUNTIME_SESSION_TOKEN-}\"",
    );

    let output = fixture
        .command()
        .args(["runtime", "env", "session", "--repo"])
        .arg(&fixture.root)
        .args([
            "--",
            "sh",
            "-c",
            "test -z \"${AUTOSPEC_RUNTIME_SESSION_WORKER-}${AUTOSPEC_RUNTIME_SESSION_HANDOFF-}${AUTOSPEC_RUNTIME_SESSION_TOKEN-}\"",
        ])
        .output()
        .expect("session starts");

    assert!(output.status.success(), "{output:?}");
}

#[cfg(unix)]
#[test]
fn stale_worker_marker_cannot_bypass_the_supervisor() {
    let fixture = RuntimeFixture::healthy("true", "true");
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
fn heartbeat_signal_failure_or_reap_timeout_hold_ownership_until_release() {
    for (signal_exit, probe_exit) in [(42, 0), (0, 0), (42, 77)] {
        let fixture = RuntimeFixture::healthy("true", "echo down >> down-count.txt");
        let kill_log = fixture.root.join("kill-args.txt");
        let fake_bin = install_kill(&fixture.root, signal_exit, probe_exit);
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
            "echo $$ > harness.pid; sleep 1000 & child=$!; echo $child > grandchild.pid; touch harness.ready; while test ! -f harness.release; do sleep 0.02; done; kill $child; wait $child 2>/dev/null || true",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null());
        let mut outer = outer.spawn().expect("session supervisor starts");
        wait_for(&fixture.root.join("harness.ready"));
        let harness_pid = read_pid(&fixture.root.join("harness.pid"));
        let grandchild_pid = read_pid(&fixture.root.join("grandchild.pid"));
        let (record, worker_pid) = break_heartbeat_record_destination(&fixture);
        wait_for(&kill_log);
        std::thread::sleep(Duration::from_millis(700));

        let down = runtime_down(&fixture);
        let live_before_release = [worker_pid, harness_pid, grandchild_pid].map(process_alive);
        let cleanup_before_release = line_count(&fixture.root.join("down-count.txt"));
        restore_heartbeat_record(&fixture, &record);
        release_harness(&fixture.root);
        if probe_exit != 0 {
            assert!(wait_for_process_exit(harness_pid));
            assert!(wait_for_process_exit(grandchild_pid));
            std::thread::sleep(Duration::from_millis(700));
            let down_after = runtime_down(&fixture);
            let retained = process_alive(worker_pid);
            let cleanup_after = line_count(&fixture.root.join("down-count.txt"));
            if retained {
                send_signal(worker_pid, "-KILL");
            }
            let _ = wait_for_exit(&mut outer, Duration::from_secs(2));
            assert_eq!(down.status.code(), Some(2));
            assert_eq!(live_before_release, [true, true, true]);
            assert_eq!(cleanup_before_release, 0);
            assert!(retained);
            assert_eq!(down_after.status.code(), Some(2));
            assert_eq!(cleanup_after, 0);
            continue;
        }
        let status = wait_for_exit(&mut outer, Duration::from_secs(5));
        let stopped_after_release =
            [worker_pid, harness_pid, grandchild_pid].map(wait_for_process_exit);
        cleanup_live_processes(
            [worker_pid, harness_pid, grandchild_pid],
            stopped_after_release,
        );

        assert_eq!(down.status.code(), Some(2));
        assert_eq!(live_before_release, [true, true, true]);
        assert_eq!(cleanup_before_release, 0);
        assert_eq!(status.and_then(|value| value.code()), Some(2));
        assert_eq!(stopped_after_release, [true, true, true]);
        assert_eq!(line_count(&fixture.root.join("down-count.txt")), 1);
    }
}

#[cfg(unix)]
#[test]
fn supervisor_group_signal_failure_leaves_worker_and_harness_owned() {
    let fixture = RuntimeFixture::healthy("true", "echo down >> down-count.txt");
    let kill_log = fixture.root.join("kill-args.txt");
    let fake_bin = install_kill(&fixture.root, 42, 0);
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
            "echo $$ > harness.pid; sleep 1000 & child=$!; echo $child > grandchild.pid; touch harness.ready; while test ! -f harness.release; do sleep 0.02; done; kill $child; wait $child 2>/dev/null || true",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut outer = outer.spawn().expect("session supervisor starts");
    wait_for(&fixture.root.join("harness.ready"));

    send_signal(outer.id(), "-TERM");
    let status = wait_for_exit(&mut outer, Duration::from_secs(3));
    let args = std::fs::read_to_string(&kill_log).unwrap_or_default();
    let worker_pid = asserted_worker_pid(&args);
    let harness_pid = read_pid(&fixture.root.join("harness.pid"));
    let grandchild_pid = read_pid(&fixture.root.join("grandchild.pid"));
    let down = runtime_down(&fixture);
    let live_before_release = [worker_pid, harness_pid, grandchild_pid].map(process_alive);
    let cleanup_before_release = line_count(&fixture.root.join("down-count.txt"));
    release_harness(&fixture.root);
    let stopped_after_release =
        [worker_pid, harness_pid, grandchild_pid].map(wait_for_process_exit);
    cleanup_live_processes(
        [worker_pid, harness_pid, grandchild_pid],
        stopped_after_release,
    );

    assert_eq!(status.and_then(|value| value.code()), Some(2));
    assert_eq!(down.status.code(), Some(2));
    assert!(output_stderr_has(&down, "RUNTIME_LIVE_SESSIONS"));
    assert_eq!(live_before_release, [true, true, true]);
    assert_eq!(cleanup_before_release, 0);
    assert_eq!(stopped_after_release, [true, true, true]);
    wait_for(&fixture.root.join("down-count.txt"));
    assert_eq!(line_count(&fixture.root.join("down-count.txt")), 1);
}

#[cfg(unix)]
fn install_kill(root: &Path, signal_exit: i32, probe_exit: i32) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let fake_bin = root.join("fake-bin");
    std::fs::create_dir(&fake_bin).unwrap();
    let path = fake_bin.join("kill");
    std::fs::write(&path, format!(
        "#!/bin/sh\nif test \"$1\" = -0; then test {probe_exit} -eq 0 && exec /bin/kill \"$@\"; echo 'Operation not permitted' >&2; exit {probe_exit}; fi\nprintf '%s\\n' \"$@\" >> \"$KILL_ARGS_FILE\"\nexit {signal_exit}\n"
    ))
    .unwrap();
    let mut permissions = std::fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions).unwrap();
    fake_bin
}

#[cfg(unix)]
fn cleanup_live_processes(pids: [u32; 3], stopped: [bool; 3]) {
    for (pid, stopped) in pids.into_iter().zip(stopped) {
        if !stopped {
            send_signal(pid, "-KILL");
        }
    }
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
fn release_harness(root: &Path) {
    std::fs::write(root.join("harness.release"), "release\n").unwrap();
}

#[cfg(unix)]
fn break_heartbeat_record_destination(fixture: &RuntimeFixture) -> (String, u32) {
    let sessions = environment_dir(fixture).join("sessions");
    let record = std::fs::read_dir(&sessions)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.extension().and_then(|value| value.to_str()) == Some("json"))
        .unwrap();
    let contents = std::fs::read_to_string(&record).unwrap();
    let worker_pid = serde_json::from_str::<serde_json::Value>(&contents).unwrap()["pid"]
        .as_u64()
        .and_then(|value| u32::try_from(value).ok())
        .unwrap();
    std::fs::remove_file(&record).unwrap();
    std::fs::create_dir(&record).unwrap();
    (contents, worker_pid)
}

#[cfg(unix)]
fn restore_heartbeat_record(fixture: &RuntimeFixture, contents: &str) {
    let sessions = environment_dir(fixture).join("sessions");
    let record = std::fs::read_dir(&sessions)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.extension().and_then(|value| value.to_str()) == Some("json"))
        .unwrap();
    std::fs::remove_dir(&record).unwrap();
    std::fs::write(record, contents).unwrap();
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
fn environment_dir(fixture: &RuntimeFixture) -> PathBuf {
    std::fs::read_dir(&fixture.state_root)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.join("owner.json").is_file())
        .unwrap()
}

fn stop_runtime_server(root: &Path) {
    let Ok(pid) = std::fs::read_to_string(root.join("runtime-server.pid")) else {
        return;
    };
    let _ = Command::new("/bin/kill").arg(pid.trim()).output();
}

fn line_count(path: &Path) -> usize {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .count()
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
