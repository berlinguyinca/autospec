use std::fs;
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use autospec_core::runtime_env::{random_session_token, RuntimeContext, RuntimeState};

use crate::commands::CommandFailure;

const CHILD_TERMINATION_TIMEOUT: Duration = Duration::from_millis(500);
const LEGACY_SESSION_WORKER_ENV: &str = "AUTOSPEC_RUNTIME_SESSION_WORKER";
const SESSION_HANDOFF_ENV: &str = "AUTOSPEC_RUNTIME_SESSION_HANDOFF";
const SESSION_TOKEN_ENV: &str = "AUTOSPEC_RUNTIME_SESSION_TOKEN";
const SESSION_HANDOFF_PREFIX: &str = "autospec-runtime-session-handoff-";
struct SessionWorkerHandoff {
    path: PathBuf,
    token: String,
}

impl SessionWorkerHandoff {
    fn create() -> Result<Self, CommandFailure> {
        let token = random_session_token()
            .map_err(|error| CommandFailure::diagnostic(error.to_string()))?;
        let path = std::env::temp_dir().join(format!("{SESSION_HANDOFF_PREFIX}{token}"));
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&path).map_err(|error| {
            CommandFailure::diagnostic(format!("could not create session handoff: {error}"))
        })?;
        if let Err(error) = file.write_all(token.as_bytes()) {
            let _ = fs::remove_file(&path);
            return Err(CommandFailure::diagnostic(format!(
                "could not write session handoff: {error}"
            )));
        }
        Ok(Self { path, token })
    }

    fn configure(&self, command: &mut Command) {
        command
            .env_remove(LEGACY_SESSION_WORKER_ENV)
            .env(SESSION_HANDOFF_ENV, &self.path)
            .env(SESSION_TOKEN_ENV, &self.token);
    }
}

impl Drop for SessionWorkerHandoff {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

pub(super) fn consume_session_worker_handoff() -> bool {
    let (Some(path), Ok(token)) = (
        std::env::var_os(SESSION_HANDOFF_ENV).map(PathBuf::from),
        std::env::var(SESSION_TOKEN_ENV),
    ) else {
        return false;
    };
    let expected = std::env::temp_dir().join(format!("{SESSION_HANDOFF_PREFIX}{token}"));
    if path != expected
        || !fs::read_to_string(&path).is_ok_and(|stored_token| stored_token == token)
    {
        return false;
    }
    fs::remove_file(path).is_ok()
}

pub(super) fn supervise_session_worker(
    command: &[String],
    repo: &Path,
    mode: &str,
    keep_alive: bool,
) -> Result<(), CommandFailure> {
    let handoff = SessionWorkerHandoff::create()?;
    let executable = std::env::current_exe().map_err(|error| {
        CommandFailure::diagnostic(format!("could not locate runtime session worker: {error}"))
    })?;
    let mut worker = Command::new(executable);
    worker
        .args(["runtime", "env", "session", "--repo"])
        .arg(repo)
        .args(["--mode", mode]);
    if keep_alive {
        worker.arg("--keep-alive");
    }
    worker.arg("--").args(command);
    handoff.configure(&mut worker);
    run_session_supervisor(&mut worker)
}

pub(super) fn spawn_direct_command(
    command: &[String],
    repo: &Path,
    runtime: Option<(&RuntimeContext, &RuntimeState)>,
    bypassed: bool,
    process_group: bool,
) -> Result<SpawnedCommand, CommandFailure> {
    let (program, arguments) = command.split_first().ok_or_else(|| {
        CommandFailure::diagnostic("autospec runtime env requires a child command")
    })?;
    let mut child = Command::new(program);
    child.args(arguments).current_dir(repo);
    scrub_external_environment(&mut child);
    if let Some((context, state)) = runtime {
        configure_runtime_environment(&mut child, context, state, bypassed)?;
    } else if bypassed {
        child.env("AUTOSPEC_ISOLATION_BYPASSED", "1");
    }
    let foreground = configure_process_group(&mut child, process_group)?;
    let child = child.spawn().map_err(|error| {
        CommandFailure::diagnostic(format!(
            "could not run runtime child command {program}: {error}"
        ))
    })?;
    Ok(SpawnedCommand { child, foreground })
}

pub(super) fn configure_runtime_environment(
    process: &mut Command,
    context: &RuntimeContext,
    state: &RuntimeState,
    bypassed: bool,
) -> Result<(), CommandFailure> {
    state
        .validate_child_environment(context)
        .map_err(|error| CommandFailure::diagnostic(error.to_string()))?;
    for (key, value) in state.values() {
        process.env(key, value);
    }
    if bypassed {
        process.env("AUTOSPEC_ISOLATION_BYPASSED", "1");
    }
    Ok(())
}

pub(super) fn scrub_external_environment(command: &mut Command) {
    command
        .env_remove("COMPOSE_PROJECT_NAME")
        .env_remove(LEGACY_SESSION_WORKER_ENV)
        .env_remove(SESSION_HANDOFF_ENV)
        .env_remove(SESSION_TOKEN_ENV);
}

pub(super) fn run_session_supervisor(command: &mut Command) -> Result<(), CommandFailure> {
    super::reset_session_signal_handlers();
    let foreground = configure_process_group(command, true)?;
    let mut worker = ChildGuard::new(
        command.spawn().map_err(|error| {
            diagnostic(format!("could not start runtime session worker: {error}"))
        })?,
        cfg!(unix),
    );
    let mut forwarded = false;
    loop {
        #[cfg(unix)]
        {
            let received = super::received_session_signal();
            if !forwarded && (received == 2 || received == 15) {
                if let Err(error) = signal_process_group(worker.child().id(), received) {
                    worker.disarm();
                    return Err(error);
                }
                forwarded = true;
            }
        }
        if let Some(status) = worker.child().try_wait().map_err(wait_error)? {
            worker.disarm();
            foreground.restore()?;
            return super::child_status(status);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

pub(in crate::commands::runtime::env) struct TerminalForeground {
    #[cfg(unix)]
    original_process_group: Option<nix::libc::pid_t>,
}

impl TerminalForeground {
    fn inactive() -> Self {
        Self {
            #[cfg(unix)]
            original_process_group: None,
        }
    }

    fn capture() -> Result<Self, CommandFailure> {
        #[cfg(unix)]
        {
            if !std::io::stdin().is_terminal() {
                return Ok(Self::inactive());
            }
            // SAFETY: tcgetpgrp reads process metadata from the fixed stdin descriptor.
            let foreground = unsafe { nix::libc::tcgetpgrp(nix::libc::STDIN_FILENO) };
            if foreground < 0 {
                return Err(diagnostic(format!(
                    "could not inspect runtime terminal foreground: {}",
                    std::io::Error::last_os_error()
                )));
            }
            // A background caller must not steal terminal ownership.
            let current = nix::unistd::getpgrp().as_raw();
            Ok(Self {
                original_process_group: (foreground == current).then_some(foreground),
            })
        }
        #[cfg(not(unix))]
        Ok(Self {})
    }

    fn is_active(&self) -> bool {
        #[cfg(unix)]
        return self.original_process_group.is_some();
        #[cfg(not(unix))]
        false
    }

    pub(in crate::commands::runtime::env) fn restore(mut self) -> Result<(), CommandFailure> {
        self.restore_inner()
    }

    fn restore_inner(&mut self) -> Result<(), CommandFailure> {
        #[cfg(unix)]
        if let Some(process_group) = self.original_process_group {
            set_terminal_foreground(process_group).map_err(|error| {
                diagnostic(format!(
                    "could not restore runtime terminal foreground: {error}"
                ))
            })?;
            self.original_process_group = None;
        }
        Ok(())
    }
}

impl Drop for TerminalForeground {
    fn drop(&mut self) {
        let _ = self.restore_inner();
    }
}

fn configure_process_group(
    command: &mut Command,
    process_group: bool,
) -> Result<TerminalForeground, CommandFailure> {
    let foreground = if process_group {
        TerminalForeground::capture()?
    } else {
        TerminalForeground::inactive()
    };
    #[cfg(unix)]
    if process_group {
        use std::os::unix::process::CommandExt;
        if foreground.is_active() {
            // SAFETY: the child-only hook contains async-signal-safe process and
            // terminal syscalls and returns before any Rust allocation in the child.
            unsafe {
                command.pre_exec(setup_foreground_child);
            }
        } else {
            command.process_group(0);
        }
    }
    Ok(foreground)
}

#[cfg(unix)]
fn setup_foreground_child() -> std::io::Result<()> {
    let child = nix::unistd::Pid::from_raw(0);
    nix::unistd::setpgid(child, child).map_err(errno_to_io)?;
    set_terminal_foreground(nix::unistd::getpgrp().as_raw())
}

#[cfg(unix)]
fn set_terminal_foreground(process_group: nix::libc::pid_t) -> std::io::Result<()> {
    // Blocking SIGTTOU lets a background group perform tcsetpgrp. The original
    // mask is restored before exec or before the parent resumes normal work.
    use nix::sys::signal::{pthread_sigmask, SigSet, SigmaskHow, Signal};

    let mut blocked = SigSet::empty();
    blocked.add(Signal::SIGTTOU);
    let mut original = SigSet::empty();
    pthread_sigmask(SigmaskHow::SIG_BLOCK, Some(&blocked), Some(&mut original))
        .map_err(errno_to_io)?;
    // SAFETY: stdin is the verified controlling terminal and the process group
    // belongs to the same session.
    let transfer = unsafe { nix::libc::tcsetpgrp(nix::libc::STDIN_FILENO, process_group) };
    let transfer_error = (transfer < 0).then(std::io::Error::last_os_error);
    let restore = pthread_sigmask(SigmaskHow::SIG_SETMASK, Some(&original), None);
    if let Some(error) = transfer_error {
        let _ = restore;
        return Err(error);
    }
    restore.map_err(errno_to_io)
}

#[cfg(unix)]
fn errno_to_io(error: nix::errno::Errno) -> std::io::Error {
    std::io::Error::from_raw_os_error(error as i32)
}

pub(super) struct SpawnedCommand {
    pub(super) child: Child,
    pub(super) foreground: TerminalForeground,
}

impl std::ops::Deref for SpawnedCommand {
    type Target = Child;

    fn deref(&self) -> &Self::Target {
        &self.child
    }
}

impl std::ops::DerefMut for SpawnedCommand {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.child
    }
}

pub(super) struct ChildGuard {
    child: Option<Child>,
    process_group: bool,
}

impl ChildGuard {
    pub(super) fn new(child: Child, process_group: bool) -> Self {
        Self {
            child: Some(child),
            process_group,
        }
    }

    pub(super) fn child(&mut self) -> &mut Child {
        self.child.as_mut().expect("child is armed")
    }

    pub(super) fn disarm(&mut self) {
        self.child.take();
    }

    pub(super) fn terminate(&mut self) -> Result<(), CommandFailure> {
        if let Some(child) = &mut self.child {
            #[cfg(unix)]
            if self.process_group {
                signal_process_group(child.id(), 9)?;
            } else {
                let _ = child.kill();
            }
            #[cfg(not(unix))]
            let _ = child.kill();
            if !reap_child_bounded(child)? {
                return Err(diagnostic("runtime child did not exit after termination"));
            }
        }
        self.disarm();
        Ok(())
    }

    pub(super) fn wait_for_natural_group_exit(&mut self) {
        let Some(child) = &mut self.child else {
            return;
        };
        let process_group = child.id();
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) | Err(_) => std::thread::sleep(Duration::from_millis(10)),
            }
        }
        #[cfg(unix)]
        if self.process_group {
            wait_for_process_group_exit(process_group);
        }
        self.disarm();
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.terminate();
    }
}

#[cfg(unix)]
fn signal_process_group(pid: u32, signal: i32) -> Result<(), CommandFailure> {
    let signal = match signal {
        2 => "-INT",
        9 => "-KILL",
        15 => "-TERM",
        _ => return Err(diagnostic("unsupported runtime process-group signal")),
    };
    let process_group = process_group_argument(pid)?;
    let status = Command::new("kill")
        .args([signal, "--", &process_group])
        .status()
        .map_err(|error| {
            diagnostic(format!(
                "RUNTIME_PROCESS_GROUP_SIGNAL_FAILED: could not execute kill: {error}"
            ))
        })?;
    if status.success() {
        Ok(())
    } else {
        Err(diagnostic(format!(
            "RUNTIME_PROCESS_GROUP_SIGNAL_FAILED: kill {signal} exited with {status}"
        )))
    }
}

#[cfg(unix)]
fn process_group_is_alive(pid: u32) -> Result<bool, CommandFailure> {
    let process_group = process_group_argument(pid)?;
    let output = Command::new("kill")
        .args(["-0", "--", &process_group])
        .env("LC_ALL", "C")
        .output()
        .map_err(|error| diagnostic(format!("could not inspect runtime process group: {error}")))?;
    if output.status.success() {
        return Ok(true);
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.lines().any(|line| line.ends_with("No such process")) {
        return Ok(false);
    }
    Err(diagnostic(format!(
        "RUNTIME_PROCESS_GROUP_PROBE_FAILED: kill -0 exited with {}: {}",
        output.status,
        stderr.trim()
    )))
}

#[cfg(unix)]
fn wait_for_process_group_exit(process_group: u32) {
    let mut diagnostic_emitted = false;
    loop {
        let alive = match process_group_is_alive(process_group) {
            Ok(alive) => alive,
            Err(error) => {
                emit_probe_diagnostic_once(&mut diagnostic_emitted, &error);
                true
            }
        };
        if !alive {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(unix)]
fn emit_probe_diagnostic_once(emitted: &mut bool, error: &CommandFailure) {
    if !*emitted {
        eprintln!("{error}");
        *emitted = true;
    }
}

#[cfg(unix)]
fn process_group_argument(pid: u32) -> Result<String, CommandFailure> {
    let pid = i32::try_from(pid)
        .map_err(|_| diagnostic("runtime process-group ID exceeds signed integer range"))?;
    if pid <= 0 {
        return Err(diagnostic("runtime process-group ID must be positive"));
    }
    Ok(format!("-{pid}"))
}

fn reap_child_bounded(child: &mut Child) -> Result<bool, CommandFailure> {
    let deadline = Instant::now() + CHILD_TERMINATION_TIMEOUT;
    while Instant::now() < deadline {
        if child.try_wait().map_err(wait_error)?.is_some() {
            return Ok(true);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Ok(child.try_wait().map_err(wait_error)?.is_some())
}

fn wait_error(error: std::io::Error) -> CommandFailure {
    diagnostic(format!("could not wait for runtime child command: {error}"))
}

fn diagnostic(message: impl Into<String>) -> CommandFailure {
    CommandFailure::diagnostic(message)
}
