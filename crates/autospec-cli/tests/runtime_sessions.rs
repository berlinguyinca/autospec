use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use autospec_core::runtime_env::SessionRecord;

static NEXT_FIXTURE: AtomicUsize = AtomicUsize::new(0);

struct RuntimeFixture {
    root: PathBuf,
    state_root: PathBuf,
}
impl RuntimeFixture {
    fn new() -> Self {
        let suffix = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "autospec-runtime-sessions-{}-{suffix}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".autospec")).expect("create fixture directory");
        std::fs::write(
            root.join(".autospec/runtime-up.sh"),
            "set -eu\necho up >> up-count.txt\npython3 -m http.server \"$AGENT_FRONTEND_PORT\" > runtime-server.log 2>&1 &\nprintf '%s\\n' \"$!\" > runtime-server.pid\n",
        )
        .expect("write runtime up script");
        std::fs::write(
            root.join(".autospec/runtime-down.sh"),
            "set -eu\npid=$(cat runtime-server.pid)\nif kill -0 \"$pid\" 2>/dev/null; then kill \"$pid\"; fi\nrm -f runtime-server.pid\necho down >> down-count.txt\n",
        )
        .expect("write runtime down script");
        std::fs::write(
            root.join(".autospec/runtime.yml"),
            "version: 1\ndefault_mode: local\nmodes:\n  local:\n    command: sh .autospec/runtime-up.sh\n    down: sh .autospec/runtime-down.sh\n",
        )
        .expect("write manifest");
        let state_root = root.join("state");
        Self { root, state_root }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_autospec"));
        command.env("AGENT_ENV_STATE_ROOT", &self.state_root);
        command
    }

    fn session(&self, name: &str) -> ChildGuard {
        let ready = format!("{name}.ready");
        let release = format!("{name}.release");
        let child = self
            .command()
            .args(["runtime", "env", "session", "--repo"])
            .arg(&self.root)
            .args([
                "--",
                "sh",
                "-c",
                &format!("echo $$ > {name}.pid; touch {ready}; while test ! -f {release}; do sleep 0.02; done"),
            ])
            .spawn()
            .expect("session starts");
        ChildGuard {
            child: Some(child),
            release: self.root.join(release),
        }
    }
}
impl Drop for RuntimeFixture {
    fn drop(&mut self) {
        stop_runtime_server(&self.root);
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

struct ChildGuard {
    child: Option<Child>,
    release: PathBuf,
}
impl ChildGuard {
    fn pid(&self) -> u32 {
        self.child.as_ref().expect("child present").id()
    }

    fn wait(mut self) -> std::process::Output {
        self.child
            .take()
            .expect("child present")
            .wait_with_output()
            .expect("session exits")
    }
}
impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(child) = &mut self.child {
            let _ = std::fs::write(&self.release, "release\n");
            let deadline = Instant::now() + Duration::from_secs(2);
            while child.try_wait().ok().flatten().is_none() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[test]
fn runtime_env_two_sessions_teardown_only_after_the_final_release() {
    let fixture = RuntimeFixture::new();
    let first = fixture.session("first");
    wait_for(&fixture.root.join("first.ready"));
    let second = fixture.session("second");
    wait_for(&fixture.root.join("second.ready"));

    std::fs::write(fixture.root.join("first.release"), "release\n").unwrap();
    let first_output = first.wait();
    assert!(
        first_output.status.success(),
        "{}",
        String::from_utf8_lossy(&first_output.stderr)
    );
    assert_eq!(line_count(&fixture.root.join("down-count.txt")), 0);
    assert!(runtime_status(&fixture).status.success());

    std::fs::write(fixture.root.join("second.release"), "release\n").unwrap();
    let second_output = second.wait();
    assert!(
        second_output.status.success(),
        "{}",
        String::from_utf8_lossy(&second_output.stderr)
    );
    assert_eq!(line_count(&fixture.root.join("down-count.txt")), 1);
}

#[test]
fn live_lock_blocks_down_and_heartbeat_updates_schema_one_record() {
    let fixture = RuntimeFixture::new();
    let session = fixture.session("live");
    wait_for(&fixture.root.join("live.ready"));
    let record_path = only_session_record(&fixture);
    let first: SessionRecord = read_record(&record_path);

    let down = runtime_down(&fixture);
    assert_eq!(down.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&down.stderr).contains("RUNTIME_LIVE_SESSIONS"));
    assert_eq!(first.schema_version, 1);
    assert!(record_path.with_extension("lock").is_file());

    let second = wait_for_heartbeat(&record_path, first.heartbeat_at_unix_ms);
    assert!(second.heartbeat_at_unix_ms > first.heartbeat_at_unix_ms);
    std::fs::write(fixture.root.join("live.release"), "release\n").unwrap();
    assert!(session.wait().status.success());
}

#[test]
fn keep_alive_releases_all_records_and_down_prunes_an_unlocked_stale_record() {
    let fixture = RuntimeFixture::new();
    let output = fixture
        .command()
        .args(["runtime", "env", "session", "--keep-alive", "--repo"])
        .arg(&fixture.root)
        .args(["--", "sh", "-c", "true"])
        .output()
        .expect("keep-alive session starts");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let environment = environment_dir(&fixture);
    let sessions = environment.join("sessions");
    assert_eq!(session_records(&sessions).count(), 0);
    std::fs::write(sessions.join("stale.json"), "not-json\n").unwrap();
    std::fs::write(sessions.join("stale.lock"), "").unwrap();

    let down = runtime_down(&fixture);
    assert!(
        down.status.success(),
        "{}",
        String::from_utf8_lossy(&down.stderr)
    );
    assert_eq!(line_count(&fixture.root.join("down-count.txt")), 1);
    assert_eq!(directory_names(&environment), ["lease.lock"]);
}

#[test]
fn teardown_retains_one_stable_environment_lease_inode() {
    let fixture = RuntimeFixture::new();
    let up = runtime_up(&fixture);
    assert!(up.status.success());
    let environment = environment_dir(&fixture);
    let lease = environment.join("lease.lock");
    #[cfg(unix)]
    let original_inode = inode(&lease);

    let down = runtime_down(&fixture);

    assert!(
        down.status.success(),
        "{}",
        String::from_utf8_lossy(&down.stderr)
    );
    assert_eq!(directory_names(&environment), ["lease.lock"]);
    #[cfg(unix)]
    assert_eq!(inode(&lease), original_inode);
    assert_probes_serialize(&fixture, &environment);
}

#[cfg(unix)]
#[test]
fn killing_outer_session_keeps_teardown_blocked_while_harness_lives() {
    let fixture = RuntimeFixture::new();
    let outer = fixture.session("orphan");
    wait_for(&fixture.root.join("orphan.ready"));
    let harness_pid = child_pid(&fixture, "orphan");
    send_signal(outer.pid(), 9);
    let outer_output = outer.wait();
    assert_eq!(outer_output.status.code(), None);

    let down = runtime_down(&fixture);
    let child_alive = process_alive(harness_pid);
    let teardown_count = line_count(&fixture.root.join("down-count.txt"));
    std::fs::write(fixture.root.join("orphan.release"), "release\n").unwrap();
    wait_for_process_exit(harness_pid);
    wait_for(&fixture.root.join("down-count.txt"));

    assert!(child_alive, "orphan_child_alive=0");
    assert_eq!(down.status.code(), Some(2), "teardown_ran=1");
    assert!(String::from_utf8_lossy(&down.stderr).contains("RUNTIME_LIVE_SESSIONS"));
    assert_eq!(teardown_count, 0);
}

#[cfg(unix)]
#[test]
fn session_preserves_direct_child_sigterm_exit_code() {
    let fixture = RuntimeFixture::new();
    let output = fixture
        .command()
        .args(["runtime", "env", "session", "--repo"])
        .arg(&fixture.root)
        .args(["--", "sh", "-c", "kill -TERM $$"])
        .output()
        .expect("signal session starts");

    assert_eq!(output.status.code(), Some(143));
}

fn wait_for(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !path.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(path.exists(), "timed out waiting for {}", path.display());
}

fn wait_for_heartbeat(path: &Path, initial: u64) -> SessionRecord {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let record = read_record(path);
        if record.heartbeat_at_unix_ms > initial {
            return record;
        }
        assert!(Instant::now() < deadline, "heartbeat did not advance");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn line_count(path: &Path) -> usize {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .count()
}

fn runtime_status(fixture: &RuntimeFixture) -> std::process::Output {
    fixture
        .command()
        .args(["runtime", "env", "status", "--repo"])
        .arg(&fixture.root)
        .output()
        .expect("status starts")
}

fn runtime_down(fixture: &RuntimeFixture) -> std::process::Output {
    fixture
        .command()
        .args(["runtime", "env", "down", "--repo"])
        .arg(&fixture.root)
        .output()
        .expect("down starts")
}

fn runtime_up(fixture: &RuntimeFixture) -> std::process::Output {
    fixture
        .command()
        .args(["runtime", "env", "up", "--repo"])
        .arg(&fixture.root)
        .output()
        .expect("up starts")
}

fn environment_dir(fixture: &RuntimeFixture) -> PathBuf {
    std::fs::read_dir(&fixture.state_root)
        .expect("state root exists")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.join("owner.json").is_file())
        .expect("environment exists")
}

fn stop_runtime_server(root: &Path) {
    let Ok(pid) = std::fs::read_to_string(root.join("runtime-server.pid")) else {
        return;
    };
    let _ = Command::new("kill").arg(pid.trim()).output();
}

fn only_session_record(fixture: &RuntimeFixture) -> PathBuf {
    session_records(&environment_dir(fixture).join("sessions"))
        .next()
        .expect("session record exists")
}

fn session_records(sessions: &Path) -> impl Iterator<Item = PathBuf> {
    std::fs::read_dir(sessions)
        .expect("sessions directory exists")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("json"))
}

fn read_record(path: &Path) -> SessionRecord {
    serde_json::from_slice(&std::fs::read(path).expect("read session record"))
        .expect("parse session record")
}

fn directory_names(path: &Path) -> Vec<String> {
    let mut names = std::fs::read_dir(path)
        .expect("environment tombstone exists")
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    names.sort();
    names
}

fn assert_probes_serialize(fixture: &RuntimeFixture, environment: &Path) {
    let first = lease_probe(fixture, environment, "first-probe");
    wait_for(&fixture.root.join("first-probe.ready"));
    let second = lease_probe(fixture, environment, "second-probe");
    std::thread::sleep(Duration::from_millis(100));
    assert!(!fixture.root.join("second-probe.ready").exists());
    std::fs::write(fixture.root.join("first-probe.release"), "release\n").unwrap();
    assert!(first.wait().status.success());
    wait_for(&fixture.root.join("second-probe.ready"));
    std::fs::write(fixture.root.join("second-probe.release"), "release\n").unwrap();
    assert!(second.wait().status.success());
}

fn lease_probe(fixture: &RuntimeFixture, environment: &Path, name: &str) -> ChildGuard {
    let ready = fixture.root.join(format!("{name}.ready"));
    let release = fixture.root.join(format!("{name}.release"));
    let child = fixture
        .command()
        .args(["runtime", "env", "lease-probe"])
        .arg(environment)
        .arg(&ready)
        .arg(&release)
        .spawn()
        .expect("lease probe starts");
    ChildGuard {
        child: Some(child),
        release,
    }
}

#[cfg(unix)]
fn inode(path: &Path) -> u64 {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(path).unwrap().ino()
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
fn child_pid(fixture: &RuntimeFixture, name: &str) -> u32 {
    std::fs::read_to_string(fixture.root.join(format!("{name}.pid")))
        .expect("harness pid exists")
        .trim()
        .parse()
        .unwrap()
}

#[cfg(unix)]
fn process_alive(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .output()
        .is_ok_and(|output| output.status.success())
}

#[cfg(unix)]
fn wait_for_process_exit(pid: u32) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while process_alive(pid) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!process_alive(pid), "process {pid} did not exit");
}
