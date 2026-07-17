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
            root.join(".autospec/runtime.yml"),
            "version: 1\ndefault_mode: local\nmodes:\n  local:\n    command: sh -c 'echo up >> up-count.txt'\n    down: sh -c 'echo down >> down-count.txt'\n",
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
                &format!("touch {ready}; while test ! -f {release}; do sleep 0.02; done"),
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
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

struct ChildGuard {
    child: Option<Child>,
    release: PathBuf,
}

impl ChildGuard {
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
        let _ = std::fs::write(&self.release, "release\n");
        if let Some(child) = &mut self.child {
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

    std::thread::sleep(Duration::from_millis(1_100));
    let second: SessionRecord = read_record(&record_path);
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
    assert!(!environment.exists());
}

fn wait_for(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !path.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(path.exists(), "timed out waiting for {}", path.display());
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

fn environment_dir(fixture: &RuntimeFixture) -> PathBuf {
    std::fs::read_dir(&fixture.state_root)
        .expect("state root exists")
        .next()
        .expect("environment exists")
        .expect("environment entry")
        .path()
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
