#![cfg(target_os = "linux")]

use std::collections::{BTreeMap, BTreeSet};
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
        .env("SHELL", "/bin/sh")
        .stdin(Stdio::piped())
        .stdout(Stdio::from(log.try_clone().expect("clone session log")))
        .stderr(Stdio::from(log));
    let mut session = SessionGuard::spawn(command);
    assert!(
        session.wait_for_owned_session_count(2, Instant::now() + Duration::from_secs(2)),
        "script did not publish its nested PTY session"
    );
    session
        .stdin()
        .write_all(b"harness-input\ncaller-input\n")
        .expect("write PTY input");
    session.close_stdin();

    let status = session.wait_until(Instant::now() + WAIT_TIMEOUT);
    session.cleanup().expect("clean PTY session");

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
            "script",
            "-qfec",
            "trap '' TERM; python3 -c 'import os, signal, time; os.setpgid(0, 0); signal.signal(signal.SIGTERM, signal.SIG_IGN); time.sleep(30)' & while :; do sleep 1; done",
            "/dev/null",
        ])
        .env("SHELL", "/bin/sh")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut session = SessionGuard::spawn(command);
    let discovery_deadline = Instant::now() + Duration::from_secs(2);
    assert!(
        session.wait_for_owned_session_count(2, discovery_deadline),
        "fixture did not create the nested PTY session"
    );
    assert!(
        session.wait_for_owned_process_group_count(3, discovery_deadline),
        "fixture did not create nested process groups across both sessions"
    );

    let cleanup_started = Instant::now();
    session.cleanup().expect("clean nested PTY sessions");

    assert!(
        session.owned_process_groups().is_empty(),
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
    root_pid: i32,
    root_start_time: u64,
    owned_sessions: BTreeMap<i32, u64>,
    cleaned: bool,
}

impl SessionGuard {
    fn spawn(mut command: Command) -> Self {
        let child = command.spawn().expect("spawn shell on PTY");
        let root_pid = child.id() as i32;
        let root_start_time = process_table()
            .into_iter()
            .find(|process| process.pid == root_pid)
            .map(|process| process.start_time)
            .expect("read PTY wrapper identity");
        Self {
            root_pid,
            root_start_time,
            owned_sessions: BTreeMap::new(),
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

    fn wait_for_owned_session_count(&mut self, count: usize, deadline: Instant) -> bool {
        while Instant::now() < deadline {
            self.refresh_owned_sessions();
            if self.owned_sessions.len() >= count {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }

    fn wait_for_owned_process_group_count(&mut self, count: usize, deadline: Instant) -> bool {
        while Instant::now() < deadline {
            self.refresh_owned_sessions();
            if self.owned_process_groups().len() >= count {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }

    fn wait_until(&mut self, deadline: Instant) -> Option<ExitStatus> {
        loop {
            self.refresh_owned_sessions();
            if let Some(status) = self.child.try_wait().expect("wait for PTY shell") {
                return Some(status);
            }
            if Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn cleanup(&mut self) -> Result<(), String> {
        if self.cleaned {
            return Ok(());
        }
        let gone = self.terminate_until(
            nix::sys::signal::Signal::SIGTERM,
            Instant::now() + TERMINATION_TIMEOUT,
        );
        let gone = gone
            || self.terminate_until(
                nix::sys::signal::Signal::SIGKILL,
                Instant::now() + TERMINATION_TIMEOUT,
            );
        if !gone {
            return Err(format!(
                "owned PTY sessions survived SIGKILL: {:?}",
                self.owned_process_groups()
            ));
        }
        if self
            .child
            .try_wait()
            .map_err(|error| error.to_string())?
            .is_none()
        {
            let _ = self.child.kill();
        }
        let reap_deadline = Instant::now() + TERMINATION_TIMEOUT;
        while self
            .child
            .try_wait()
            .map_err(|error| error.to_string())?
            .is_none()
        {
            if Instant::now() >= reap_deadline {
                return Err("PTY wrapper was not reaped before the deadline".to_string());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        self.cleaned = true;
        Ok(())
    }

    fn terminate_until(&mut self, signal: nix::sys::signal::Signal, deadline: Instant) -> bool {
        while Instant::now() < deadline {
            self.refresh_owned_sessions();
            let groups = self.owned_process_groups();
            if groups.is_empty() {
                return true;
            }
            terminate_process_groups(groups, signal);
            std::thread::sleep(Duration::from_millis(10));
        }
        self.owned_process_groups().is_empty()
    }

    fn refresh_owned_sessions(&mut self) {
        let processes = process_table();
        let valid_sessions = self.valid_owned_sessions(&processes);
        let mut owned_processes = processes
            .iter()
            .filter(|process| {
                process.pid == self.root_pid && process.start_time == self.root_start_time
            })
            .map(|process| process.pid)
            .collect::<BTreeSet<_>>();
        loop {
            let mut changed = false;
            for process in &processes {
                if (owned_processes.contains(&process.parent)
                    || valid_sessions.contains(&process.session))
                    && owned_processes.insert(process.pid)
                {
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        for session in processes
            .iter()
            .filter(|process| {
                owned_processes.contains(&process.pid)
                    && owned_processes.contains(&process.session)
            })
            .map(|process| process.session)
        {
            if self.owned_sessions.contains_key(&session) {
                continue;
            }
            if let Some(leader) = processes.iter().find(|process| process.pid == session) {
                self.owned_sessions.insert(session, leader.start_time);
            }
        }
    }

    fn owned_process_groups(&self) -> BTreeSet<i32> {
        let processes = process_table();
        let valid_sessions = self.valid_owned_sessions(&processes);
        processes
            .into_iter()
            .filter(|process| process.state != 'Z' && valid_sessions.contains(&process.session))
            .map(|process| process.group)
            .filter(|group| *group > 0)
            .collect()
    }

    fn valid_owned_sessions(&self, processes: &[Process]) -> BTreeSet<i32> {
        self.owned_sessions
            .iter()
            .filter_map(|(session, start_time)| {
                match processes.iter().find(|process| process.pid == *session) {
                    Some(leader) if leader.start_time == *start_time => Some(*session),
                    Some(_) => None,
                    None if processes.iter().any(|process| process.session == *session) => {
                        Some(*session)
                    }
                    None => None,
                }
            })
            .collect()
    }
}

impl Drop for SessionGuard {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

fn terminate_process_groups(process_groups: BTreeSet<i32>, signal: nix::sys::signal::Signal) {
    for process_group in process_groups {
        let _ = nix::sys::signal::kill(nix::unistd::Pid::from_raw(-process_group), signal);
    }
}

#[derive(Clone, Copy)]
struct Process {
    pid: i32,
    parent: i32,
    group: i32,
    session: i32,
    start_time: u64,
    state: char,
}

fn process_table() -> Vec<Process> {
    let mut processes = Vec::new();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return processes;
    };
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<i32>().ok())
        else {
            continue;
        };
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
            continue;
        };
        let Some(fields) = stat.rsplit_once(") ").map(|(_, fields)| fields) else {
            continue;
        };
        let fields = fields.split_whitespace().collect::<Vec<_>>();
        if fields.len() <= 19 {
            continue;
        }
        let (Some(state), Ok(parent), Ok(group), Ok(session), Ok(start_time)) = (
            fields[0].chars().next(),
            fields[1].parse::<i32>(),
            fields[2].parse::<i32>(),
            fields[3].parse::<i32>(),
            fields[19].parse::<u64>(),
        ) else {
            continue;
        };
        processes.push(Process {
            pid,
            parent,
            group,
            session,
            start_time,
            state,
        });
    }
    processes
}
