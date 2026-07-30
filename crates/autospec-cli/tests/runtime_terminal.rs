#![cfg(unix)]

use std::fs::File;
use std::io::Write;
use std::os::fd::FromRawFd;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
fn runtime_session_interactive_child_reads_from_terminal() {
    let root = fixture_root();
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
    let (master, slave) = open_pseudo_terminal();
    let log = File::create(&session_log).expect("create session log");
    let mut shell = Command::new("sh");
    shell
        .args([
            "-c",
            r#""$AUTOSPEC_TEST_BIN" runtime env session --repo "$AUTOSPEC_TEST_REPO" -- sh -c 'IFS= read -r line && printf "%s\n" "$line" > "$AUTOSPEC_TEST_HARNESS_READ"'
IFS= read -r line && printf "%s\n" "$line" > "$AUTOSPEC_TEST_CALLER_READ""#,
        ])
        .env("AUTOSPEC_TEST_BIN", env!("CARGO_BIN_EXE_autospec"))
        .env("AUTOSPEC_TEST_REPO", &root)
        .env("AUTOSPEC_TEST_HARNESS_READ", &harness_read)
        .env("AUTOSPEC_TEST_CALLER_READ", &caller_read)
        .env("AGENT_ENV_STATE_ROOT", &state_root)
        .stdin(Stdio::from(slave))
        .stdout(Stdio::from(log.try_clone().expect("clone session log")))
        .stderr(Stdio::from(log));
    // SAFETY: the child-only hook calls async-signal-safe process/session syscalls.
    unsafe {
        shell.pre_exec(|| {
            // linter:allow-SECURITY pre_exec only establishes the test child's PTY session
            if nix::libc::setsid() < 0
                || nix::libc::ioctl(nix::libc::STDIN_FILENO, nix::libc::TIOCSCTTY as _, 0) < 0
            {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut shell = shell.spawn().expect("spawn shell on PTY");
    (&master)
        .write_all(b"harness-input\ncaller-input\n")
        .expect("write PTY input");

    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        if let Some(status) = shell.try_wait().expect("wait for PTY shell") {
            break Some(status);
        }
        if Instant::now() >= deadline {
            break None;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    if status.is_none() {
        // The shell and outer supervisor share this group; SIGTERM lets the
        // supervisor forward termination and clean up its nested groups.
        let _ = unsafe { nix::libc::kill(-(shell.id() as i32), nix::libc::SIGTERM) };
        let _ = shell.wait();
    }

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
    let _ = std::fs::remove_dir_all(root);
}

fn fixture_root() -> PathBuf {
    std::env::temp_dir().join(format!("autospec-runtime-terminal-{}", std::process::id()))
}

fn open_pseudo_terminal() -> (File, File) {
    let mut master = -1;
    let mut slave = -1;
    // SAFETY: openpty initializes both descriptors; optional configuration is null.
    let result = unsafe {
        nix::libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null(),
            std::ptr::null(),
        )
    };
    assert_eq!(result, 0, "open PTY: {}", std::io::Error::last_os_error());
    // SAFETY: openpty returned fresh owned descriptors on success.
    unsafe { (File::from_raw_fd(master), File::from_raw_fd(slave)) }
}
