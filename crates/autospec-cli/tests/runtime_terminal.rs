#![cfg(target_os = "linux")]

use std::collections::BTreeSet;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

const WAIT_TIMEOUT: Duration = Duration::from_secs(5);
const TERMINATION_TIMEOUT: Duration = Duration::from_millis(500);

#[test]
fn runtime_session_interactive_child_reads_from_terminal() {
    let fixture = Fixture::new();
    let root = fixture.path();
    let state_root = root.join("state");
    std::fs::create_dir_all(root.join(".autospec")).expect("create fixture");
    std::fs::write(
        root.join(".autospec/runtime.yml"),
        "version: 2\ndefault_mode: local\nmodes:\n  local:\n    command: \"true\"\n    down: \"true\"\n    readiness: deferred\n",
    )
    .expect("write runtime manifest");
    let harness_read = root.join("harness-read.txt");
    let caller_read = root.join("caller-read.txt");
    let session_log = root.join("session.log");
    let log = File::create(&session_log).expect("create session log");
    let mut command = Command::new("setsid");
    command
        .args([
            "script",
            "-qfec",
            r#""$AUTOSPEC_TEST_BIN" runtime env session --repo "$AUTOSPEC_TEST_REPO" -- sh -c 'IFS= read -r line && printf "%s\n" "$line" > "$AUTOSPEC_TEST_HARNESS_READ"'
IFS= read -r line && printf "%s\n" "$line" > "$AUTOSPEC_TEST_CALLER_READ""#,
            "/dev/null",
        ])
        .env("AUTOSPEC_TEST_BIN", env!("CARGO_BIN_EXE_autospec"))
        .env("AUTOSPEC_TEST_REPO", root)
        .env("AUTOSPEC_TEST_HARNESS_READ", &harness_read)
        .env("AUTOSPEC_TEST_CALLER_READ", &caller_read)
        .env("AGENT_ENV_STATE_ROOT", &state_root)
        .stdin(Stdio::piped())
        .stdout(Stdio::from(log.try_clone().expect("clone session log")))
        .stderr(Stdio::from(log));
    let mut session = SessionGuard::spawn(command);
    session
        .stdin()
        .write_all(b"harness-input\ncaller-input\n")
        .expect("write PTY input");
    session.close_stdin();

    let status = session.wait_until(Instant::now() + WAIT_TIMEOUT);
    session.cleanup();

    assert!(
        status.is_some_and(|status| status.success()),
        "interactive runtime session did not exit: {}",
        std::fs::read_to_string(session_log).unwrap_or_default()
    );
    assert_eq!(
        std::fs::read_to_string(harness_read).unwrap(),
        "harness-input\n"
    );
    assert_eq!(
        std::fs::read_to_string(caller_read).unwrap(),
        "caller-input\n"
    );
}

#[test]
fn session_cleanup_reaps_nested_process_groups_after_timeout() {
    let mut command = Command::new("setsid");
    command
        .args([
            "sh",
            "-c",
            "trap '' TERM; python3 -c 'import os, signal, time; os.setpgid(0, 0); signal.signal(signal.SIGTERM, signal.SIG_IGN); time.sleep(30)' & while :; do sleep 1; done",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut session = SessionGuard::spawn(command);
    let discovery_deadline = Instant::now() + Duration::from_secs(2);
    while session_process_groups(session.session_id).len() < 2
        && Instant::now() < discovery_deadline
    {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        session_process_groups(session.session_id).len() >= 2,
        "fixture did not create nested process groups"
    );

    let cleanup_started = Instant::now();
    session.cleanup();

    assert!(
        session_process_groups(session.session_id).is_empty(),
        "session cleanup left a nested process group"
    );
    assert!(
        cleanup_started.elapsed() < Duration::from_secs(2),
        "session cleanup exceeded its bounded TERM-to-KILL budget"
    );
}

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("autospec-runtime-terminal-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        Self { root }
    }

    fn path(&self) -> &Path {
        &self.root
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

struct SessionGuard {
    child: Child,
    session_id: i32,
    cleaned: bool,
}

impl SessionGuard {
    fn spawn(mut command: Command) -> Self {
        let child = command.spawn().expect("spawn shell on PTY");
        Self {
            session_id: child.id() as i32,
            child,
            cleaned: false,
        }
    }

    fn stdin(&mut self) -> &mut impl Write {
        self.child.stdin.as_mut().expect("session stdin")
    }

    fn close_stdin(&mut self) {
        self.child.stdin.take();
    }

    fn wait_until(&mut self, deadline: Instant) -> Option<ExitStatus> {
        loop {
            if let Some(status) = self.child.try_wait().expect("wait for PTY shell") {
                return Some(status);
            }
            if Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn cleanup(&mut self) {
        if self.cleaned {
            return;
        }
        terminate_session_groups(self.session_id, nix::sys::signal::Signal::SIGTERM);
        wait_until_session_gone(self.session_id, Instant::now() + TERMINATION_TIMEOUT);
        terminate_session_groups(self.session_id, nix::sys::signal::Signal::SIGKILL);
        wait_until_session_gone(self.session_id, Instant::now() + TERMINATION_TIMEOUT);
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
        self.cleaned = true;
    }
}

impl Drop for SessionGuard {
    fn drop(&mut self) {
        self.cleanup();
    }
}

fn wait_until_session_gone(session_id: i32, deadline: Instant) {
    while Instant::now() < deadline {
        if session_process_groups(session_id).is_empty() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn terminate_session_groups(session_id: i32, signal: nix::sys::signal::Signal) {
    for process_group in session_process_groups(session_id) {
        let _ = nix::sys::signal::kill(nix::unistd::Pid::from_raw(-process_group), signal);
    }
}

fn session_process_groups(session_id: i32) -> BTreeSet<i32> {
    let mut groups = BTreeSet::new();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return groups;
    };
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
            continue;
        };
        let Some(fields) = stat.rsplit_once(") ").map(|(_, fields)| fields) else {
            continue;
        };
        let mut fields = fields.split_whitespace();
        let (Some(_state), Some(_parent), Some(process_group), Some(session)) = (
            fields.next(),
            fields.next(),
            fields.next().and_then(|value| value.parse::<i32>().ok()),
            fields.next().and_then(|value| value.parse::<i32>().ok()),
        ) else {
            continue;
        };
        if session == session_id && process_group > 0 {
            groups.insert(process_group);
        }
    }
    groups
}
