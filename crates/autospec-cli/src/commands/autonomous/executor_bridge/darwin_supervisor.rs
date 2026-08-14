use super::{
    argv_digest, codex_host_auth_environment_key, prepare_output_pump, read_executor_exit_status,
    read_live_executor_exit_status, sensitive_executor_environment_key, OutputSinkPaths,
    ProcessIdentity, ValidatedInvocation,
};
use crate::commands::autonomous::platform_process::{self, ProcessObservation};
use nix::errno::Errno;
use nix::sys::signal::{killpg, Signal};
use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
use nix::unistd::{fork, write as fd_write, ForkResult, Pid};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::fs::{File, OpenOptions};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
#[cfg(test)]
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::thread;
use std::time::{Duration, Instant};

const TERM_GRACE: Duration = Duration::from_millis(500);
const KILL_GRACE: Duration = Duration::from_secs(5);
const OBSERVATION_INTERVAL: Duration = Duration::from_millis(10);
#[cfg(test)]
static FORCE_NEXT_TERMINATION_UNCERTAINTY: AtomicBool = AtomicBool::new(false);

#[cfg(test)]
pub(super) struct TerminationUncertaintyGuard;

#[cfg(test)]
impl Drop for TerminationUncertaintyGuard {
    fn drop(&mut self) {
        FORCE_NEXT_TERMINATION_UNCERTAINTY.store(false, AtomicOrdering::SeqCst);
    }
}

#[cfg(test)]
pub(super) fn force_next_termination_uncertainty_for_test() -> TerminationUncertaintyGuard {
    FORCE_NEXT_TERMINATION_UNCERTAINTY.store(true, AtomicOrdering::SeqCst);
    TerminationUncertaintyGuard
}

fn cloexec_pipe(label: &str) -> Result<(OwnedFd, OwnedFd), String> {
    let mut descriptors = [-1_i32; 2];
    // SAFETY: descriptors points to exactly two writable integers.
    if unsafe { nix::libc::pipe(descriptors.as_mut_ptr()) } != 0 {
        return Err(format!(
            "create {label}: {}",
            std::io::Error::last_os_error()
        ));
    }
    for descriptor in descriptors {
        // SAFETY: pipe returned both descriptors and F_SETFD changes descriptor flags only.
        if unsafe { nix::libc::fcntl(descriptor, nix::libc::F_SETFD, nix::libc::FD_CLOEXEC) } != 0 {
            // SAFETY: both descriptors are still owned by this function.
            unsafe {
                nix::libc::close(descriptors[0]);
                nix::libc::close(descriptors[1]);
            }
            return Err(format!(
                "set {label} close-on-exec: {}",
                std::io::Error::last_os_error()
            ));
        }
    }
    // SAFETY: the successful pipe returned two new descriptors uniquely owned here.
    Ok(unsafe {
        (
            OwnedFd::from_raw_fd(descriptors[0]),
            OwnedFd::from_raw_fd(descriptors[1]),
        )
    })
}

#[derive(Debug)]
pub(super) struct SpawnFailure {
    pub(super) reason: String,
}

impl From<String> for SpawnFailure {
    fn from(reason: String) -> Self {
        Self { reason }
    }
}

pub(super) struct DarwinOwnedGroup {
    leader: ProcessIdentity,
    supervisor_pid: Option<Pid>,
    barrier: Option<OwnedFd>,
    exec_status: Option<File>,
    sinks: OutputSinkPaths,
}

impl DarwinOwnedGroup {
    pub(super) fn spawn(
        harness: &ValidatedInvocation,
        sinks: &OutputSinkPaths,
    ) -> Result<Self, SpawnFailure> {
        Self::spawn_with_policy(harness, sinks, true)
    }

    fn spawn_with_policy(
        harness: &ValidatedInvocation,
        sinks: &OutputSinkPaths,
        strip_credentials: bool,
    ) -> Result<Self, SpawnFailure> {
        let pump = prepare_output_pump(sinks)?;
        let executable = CString::new(harness.program.as_os_str().as_bytes())
            .map_err(|_| "executor program contains a NUL byte".to_string())?;
        let mut argv = Vec::with_capacity(harness.args.len() + 1);
        argv.push(
            CString::new(
                harness
                    .argv_zero
                    .as_deref()
                    .unwrap_or(harness.program.as_os_str())
                    .as_bytes(),
            )
            .map_err(|_| "executor argv zero contains a NUL byte".to_string())?,
        );
        for arg in &harness.args {
            argv.push(
                CString::new(arg.as_bytes())
                    .map_err(|_| "executor argument contains a NUL byte".to_string())?,
            );
        }
        let worktree = CString::new(harness.current_dir.as_os_str().as_bytes())
            .map_err(|_| "executor worktree contains a NUL byte".to_string())?;
        let preserve_codex_host_auth = harness.args.first().is_some_and(|arg| arg == "exec")
            && harness
                .args
                .iter()
                .any(|arg| arg == "--output-last-message");
        let mut environment = std::env::vars_os()
            .filter(|(key, _)| {
                !strip_credentials
                    || ((!sensitive_executor_environment_key(key)
                        || (preserve_codex_host_auth && codex_host_auth_environment_key(key)))
                        && key != "COMPOSE_PROJECT_NAME")
            })
            .collect::<BTreeMap<_, _>>();
        for (key, value) in &harness.environment_overrides {
            if strip_credentials && sensitive_executor_environment_key(key) {
                return Err(format!(
                    "executor harness override may not restore credential authority: {}",
                    key.to_string_lossy()
                )
                .into());
            }
            environment.insert(key.clone(), value.clone());
        }
        if strip_credentials {
            let credentialless_config = sinks
                .stdout
                .parent()
                .ok_or_else(|| "executor output sink has no parent".to_string())?
                .join("credentialless-config");
            super::ensure_private_directory(&credentialless_config)?;
            environment.insert(
                "GH_CONFIG_DIR".into(),
                credentialless_config.into_os_string(),
            );
            environment.insert("GIT_CONFIG_NOSYSTEM".into(), "1".into());
            environment.insert("GIT_CONFIG_GLOBAL".into(), "/dev/null".into());
            environment.insert("GIT_TERMINAL_PROMPT".into(), "0".into());
            environment.insert(
                "GIT_SSH_COMMAND".into(),
                "/usr/bin/ssh -F /dev/null -o IdentityAgent=none -o IdentitiesOnly=yes -o IdentityFile=/dev/null -o BatchMode=yes".into(),
            );
        }
        let environment = environment
            .into_iter()
            .map(|(key, value)| {
                let mut entry = key.as_os_str().as_bytes().to_vec();
                entry.push(b'=');
                entry.extend_from_slice(value.as_os_str().as_bytes());
                CString::new(entry)
                    .map_err(|_| "executor environment contains a NUL byte".to_string())
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut argv_pointers = argv.iter().map(|value| value.as_ptr()).collect::<Vec<_>>();
        argv_pointers.push(std::ptr::null());
        let mut environment_pointers = environment
            .iter()
            .map(|value| value.as_ptr())
            .collect::<Vec<_>>();
        environment_pointers.push(std::ptr::null());
        let (barrier_read, barrier_write) = cloexec_pipe("Darwin launch barrier")?;
        let (ready_read, ready_write) = cloexec_pipe("Darwin launch readiness pipe")?;
        let (status_read, status_write) = cloexec_pipe("Darwin exec-status pipe")?;
        let (stdout_read, stdout_write) = cloexec_pipe("Darwin stdout pipe")?;
        let (stderr_read, stderr_write) = cloexec_pipe("Darwin stderr pipe")?;
        let null = OpenOptions::new()
            .read(true)
            .open("/dev/null")
            .map_err(|error| format!("open Darwin executor null input: {error}"))?;
        let fds = DarwinForkDescriptors {
            barrier_read: barrier_read.as_raw_fd(),
            barrier_write: barrier_write.as_raw_fd(),
            ready_read: ready_read.as_raw_fd(),
            ready_write: ready_write.as_raw_fd(),
            status_read: status_read.as_raw_fd(),
            status_write: status_write.as_raw_fd(),
            stdout_read: stdout_read.as_raw_fd(),
            stdout_write: stdout_write.as_raw_fd(),
            stderr_read: stderr_read.as_raw_fd(),
            stderr_write: stderr_write.as_raw_fd(),
            null: null.as_raw_fd(),
            stdout_ring: pump.stdout.as_raw_fd(),
            stderr_ring: pump.stderr.as_raw_fd(),
            stdout_cursor: pump.stdout_cursor.as_raw_fd(),
            stderr_cursor: pump.stderr_cursor.as_raw_fd(),
            exit_status: pump.exit_status.as_raw_fd(),
        };
        // SAFETY: all strings, pointer arrays, files, and pipes are prepared before fork. The
        // child path uses only async-signal-safe libc calls and never returns to Rust.
        let supervisor = match unsafe { fork() }
            .map_err(|error| format!("fork Darwin executor supervisor: {error}"))?
        {
            ForkResult::Child => unsafe {
                run_blocked_supervisor(
                    fds,
                    executable.as_ptr(),
                    argv_pointers.as_ptr(),
                    environment_pointers.as_ptr(),
                    worktree.as_ptr(),
                )
            },
            ForkResult::Parent { child } => child,
        };
        drop(barrier_read);
        drop(ready_write);
        drop(status_write);
        drop(stdout_read);
        drop(stdout_write);
        drop(stderr_read);
        drop(stderr_write);
        drop(null);
        drop(pump);
        let pid = supervisor.as_raw() as u32;
        let capture = (|| -> Result<ProcessIdentity, String> {
            read_exact_ready(ready_read.as_raw_fd())?;
            let before = platform_process::observe_birth(pid)?
                .ok_or_else(|| "Darwin supervisor exited before ownership capture".to_string())?;
            let observed_group = platform_process::observe_process_group(pid)?
                .ok_or_else(|| "Darwin supervisor exited before group verification".to_string())?;
            let after = platform_process::observe_birth(pid)?.ok_or_else(|| {
                "Darwin supervisor exited before ownership persistence".to_string()
            })?;
            if before != after || observed_group != pid || after.process_group != pid {
                return Err(
                    "spawned Darwin executor process-group identity is unstable".to_string()
                );
            }
            Ok(ProcessIdentity {
                pid,
                process_group: observed_group,
                executable: std::env::current_exe()
                    .map_err(|error| format!("resolve Darwin supervisor executable: {error}"))?,
                argv_digest: argv_digest(&std::env::args().skip(1).collect::<Vec<_>>()),
                boot_id: after.boot_id,
                start_identity: after.start_identity,
            })
        })();
        drop(ready_read);
        let leader = match capture {
            Ok(leader) => leader,
            Err(error) => {
                drop(barrier_write);
                reap_exact_child(supervisor)?;
                if !group_is_empty(pid)? {
                    return Err("cancelled Darwin launch retained process-group membership"
                        .to_string()
                        .into());
                }
                return Err(error.into());
            }
        };
        Ok(Self {
            leader,
            supervisor_pid: Some(supervisor),
            barrier: Some(barrier_write),
            exec_status: Some(File::from(status_read)),
            sinks: sinks.clone(),
        })
    }

    pub(super) fn adopt(
        expected: &ProcessIdentity,
        sinks: &OutputSinkPaths,
    ) -> Result<Self, String> {
        let supervisor_pid = reapable_child(expected.pid)?;
        match platform_process::observe_expected(
            expected.pid,
            &expected.boot_id,
            &expected.start_identity,
        ) {
            ProcessObservation::Exact(birth) if birth.process_group == expected.process_group => {}
            ProcessObservation::Dead => {
                if let Some(pid) = supervisor_pid {
                    reap_exact_child(pid)?;
                }
                if read_executor_exit_status(&sinks.exit_status)?.is_none()
                    || !group_is_empty(expected.process_group)?
                {
                    return Err(
                        "Darwin executor leader is dead without durable whole-group completion"
                            .to_string(),
                    );
                }
            }
            ProcessObservation::Exact(_)
            | ProcessObservation::Mismatch
            | ProcessObservation::Unknown(_) => {
                return Err("executor process group ownership is unverified".to_string())
            }
        }
        Ok(Self {
            leader: expected.clone(),
            supervisor_pid,
            barrier: None,
            exec_status: None,
            sinks: sinks.clone(),
        })
    }

    pub(super) fn release(&mut self) -> Result<(), String> {
        let barrier = self
            .barrier
            .as_ref()
            .ok_or_else(|| "Darwin launch barrier was already released".to_string())?;
        fd_write(barrier, b"\n")
            .map_err(|error| format!("release Darwin launch barrier: {error}"))?;
        drop(self.barrier.take());
        let status = self
            .exec_status
            .take()
            .ok_or_else(|| "Darwin exec-status pipe is missing".to_string())?;
        read_exec_status(status.as_raw_fd())
    }

    pub(super) fn cancel_unreleased(mut self) -> Result<(), String> {
        let barrier = self
            .barrier
            .take()
            .ok_or_else(|| "Darwin launch was already released".to_string())?;
        drop(barrier);
        self.exec_status.take();
        let pid = self
            .supervisor_pid
            .take()
            .ok_or_else(|| "blocked Darwin launch has no reapable child".to_string())?;
        reap_exact_child(pid)?;
        if !group_is_empty(self.leader.process_group)? {
            return Err("cancelled Darwin launch retained process-group membership".to_string());
        }
        Ok(())
    }

    pub(super) fn abort_failed_release(self) -> Result<(), String> {
        if self.barrier.is_some() {
            self.cancel_unreleased()
        } else {
            self.terminate()
        }
    }

    pub(super) fn poll(&mut self) -> Result<Option<i32>, String> {
        let exit = read_live_executor_exit_status(&self.sinks.exit_status)?;
        match platform_process::observe_expected(
            self.leader.pid,
            &self.leader.boot_id,
            &self.leader.start_identity,
        ) {
            ProcessObservation::Exact(birth)
                if birth.process_group == self.leader.process_group =>
            {
                Ok(None)
            }
            ProcessObservation::Dead => {
                self.reap_if_child()?;
                if !group_is_empty(self.leader.process_group)? {
                    return Err(
                        "Darwin executor leader exited while process-group membership remains uncertain"
                            .to_string(),
                    );
                }
                exit.map(Some).ok_or_else(|| {
                    "Darwin executor group exited without a durable terminal record".to_string()
                })
            }
            ProcessObservation::Exact(_)
            | ProcessObservation::Mismatch
            | ProcessObservation::Unknown(_) => {
                Err("executor process group ownership is unverified".to_string())
            }
        }
    }

    pub(super) fn terminate(mut self) -> Result<(), String> {
        if self.barrier.is_some() {
            return self.cancel_unreleased();
        }
        match platform_process::observe_expected(
            self.leader.pid,
            &self.leader.boot_id,
            &self.leader.start_identity,
        ) {
            ProcessObservation::Exact(birth)
                if birth.process_group == self.leader.process_group =>
            {
                signal_exact_group(&self.leader, Signal::SIGTERM)?;
            }
            ProcessObservation::Dead => {
                self.reap_if_child()?;
                return if group_is_empty(self.leader.process_group)? {
                    Ok(())
                } else {
                    Err(
                        "Darwin executor leader is dead while process-group membership remains"
                            .to_string(),
                    )
                };
            }
            ProcessObservation::Exact(_)
            | ProcessObservation::Mismatch
            | ProcessObservation::Unknown(_) => {
                return Err("executor process group ownership is unverified".to_string());
            }
        }
        if wait_for_empty_group(self.leader.process_group, self.supervisor_pid, TERM_GRACE)? {
            return Ok(());
        }
        signal_exact_group(&self.leader, Signal::SIGKILL)?;
        if !wait_for_empty_group(self.leader.process_group, self.supervisor_pid, KILL_GRACE)? {
            return Err("Darwin executor process group survived SIGKILL".to_string());
        }
        Ok(())
    }

    pub(super) fn identity(&self) -> &ProcessIdentity {
        &self.leader
    }

    fn reap_if_child(&mut self) -> Result<(), String> {
        let Some(pid) = self.supervisor_pid.take() else {
            return Ok(());
        };
        reap_exact_child(pid)
    }
}

impl Drop for DarwinOwnedGroup {
    fn drop(&mut self) {
        // A still-owned barrier means the harness never received launch authority. Closing the
        // barrier makes the blocked supervisor exit without executing user code; reap it so a
        // persistence failure cannot leak a zombie or a runnable child. Released/adopted groups
        // deliberately survive a parent crash for durable recovery.
        if self.barrier.take().is_some() {
            if let Some(pid) = self.supervisor_pid.take() {
                let _ = reap_exact_child(pid);
            }
        }
    }
}

#[derive(Clone, Copy)]
struct DarwinForkDescriptors {
    barrier_read: i32,
    barrier_write: i32,
    ready_read: i32,
    ready_write: i32,
    status_read: i32,
    status_write: i32,
    stdout_read: i32,
    stdout_write: i32,
    stderr_read: i32,
    stderr_write: i32,
    null: i32,
    stdout_ring: i32,
    stderr_ring: i32,
    stdout_cursor: i32,
    stderr_cursor: i32,
    exit_status: i32,
}

unsafe fn run_blocked_supervisor(
    fds: DarwinForkDescriptors,
    executable: *const nix::libc::c_char,
    argv: *const *const nix::libc::c_char,
    environment: *const *const nix::libc::c_char,
    worktree: *const nix::libc::c_char,
) -> ! {
    // SAFETY: this is the isolated post-fork child and every descriptor was prepared by parent.
    unsafe {
        nix::libc::close(fds.barrier_write);
        nix::libc::close(fds.ready_read);
        nix::libc::close(fds.status_read);
        if nix::libc::setpgid(0, 0) != 0 || !super::raw_write_all(fds.ready_write, b"R") {
            super::terminate_post_fork(127);
        }
        nix::libc::close(fds.ready_write);
        let mut release = 0_u8;
        let released = loop {
            let count =
                nix::libc::read(fds.barrier_read, std::ptr::addr_of_mut!(release).cast(), 1);
            if count < 0 && *nix::libc::__error() == nix::libc::EINTR {
                continue;
            }
            break count == 1 && release == b'\n';
        };
        nix::libc::close(fds.barrier_read);
        if !released {
            super::terminate_post_fork(125);
        }
        let harness = nix::libc::fork();
        if harness < 0 {
            let _ = super::raw_write_all(fds.status_write, b"!");
            super::terminate_post_fork(127);
        }
        if harness == 0 {
            nix::libc::close(fds.stdout_read);
            nix::libc::close(fds.stderr_read);
            if nix::libc::chdir(worktree) != 0
                || nix::libc::dup2(fds.null, nix::libc::STDIN_FILENO) < 0
                || nix::libc::dup2(fds.stdout_write, nix::libc::STDOUT_FILENO) < 0
                || nix::libc::dup2(fds.stderr_write, nix::libc::STDERR_FILENO) < 0
            {
                let _ = super::raw_write_all(fds.status_write, b"!");
                super::terminate_post_fork(126);
            }
            nix::libc::execve(executable, argv, environment);
            let _ = super::raw_write_all(fds.status_write, b"!");
            super::terminate_post_fork(127);
        }
        nix::libc::close(fds.status_write);
        nix::libc::close(fds.stdout_write);
        nix::libc::close(fds.stderr_write);
        nix::libc::close(fds.null);
        super::raw_supervisor_loop(
            harness,
            fds.stdout_read,
            fds.stderr_read,
            fds.stdout_ring,
            fds.stderr_ring,
            fds.stdout_cursor,
            fds.stderr_cursor,
            fds.exit_status,
            -1,
        );
    }
}

fn read_exact_ready(descriptor: i32) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut marker = 0_u8;
    loop {
        let mut pollfd = nix::libc::pollfd {
            fd: descriptor,
            events: nix::libc::POLLIN | nix::libc::POLLHUP,
            revents: 0,
        };
        // SAFETY: pollfd is initialized and points to one live descriptor.
        let polled = unsafe { nix::libc::poll(&mut pollfd, 1, 10) };
        if polled > 0 {
            // SAFETY: marker is one writable byte.
            let count =
                unsafe { nix::libc::read(descriptor, std::ptr::addr_of_mut!(marker).cast(), 1) };
            return if count == 1 && marker == b'R' {
                Ok(())
            } else {
                Err("Darwin supervisor failed before launch readiness".to_string())
            };
        }
        if Instant::now() >= deadline {
            return Err("Darwin supervisor launch readiness timed out".to_string());
        }
    }
}

fn read_exec_status(descriptor: i32) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let mut pollfd = nix::libc::pollfd {
            fd: descriptor,
            events: nix::libc::POLLIN | nix::libc::POLLHUP,
            revents: 0,
        };
        // SAFETY: pollfd is initialized and points to one live descriptor.
        let polled = unsafe { nix::libc::poll(&mut pollfd, 1, 10) };
        if polled > 0 {
            let mut marker = 0_u8;
            // SAFETY: marker is one writable byte.
            let count =
                unsafe { nix::libc::read(descriptor, std::ptr::addr_of_mut!(marker).cast(), 1) };
            return if count == 0 {
                Ok(())
            } else {
                Err("Darwin harness failed before exact exec".to_string())
            };
        }
        if Instant::now() >= deadline {
            return Err("Darwin harness exec-status timed out".to_string());
        }
    }
}

fn reap_exact_child(pid: Pid) -> Result<(), String> {
    loop {
        match waitpid(pid, Some(WaitPidFlag::WNOHANG)) {
            Ok(WaitStatus::Exited(..) | WaitStatus::Signaled(..))
            | Err(Errno::ECHILD)
            | Err(Errno::ESRCH) => return Ok(()),
            Ok(WaitStatus::StillAlive) => {
                thread::sleep(OBSERVATION_INTERVAL);
            }
            Err(Errno::EINTR) => continue,
            Ok(status) => return Err(format!("reap Darwin supervisor: {status:?}")),
            Err(error) => return Err(format!("reap Darwin supervisor: {error}")),
        }
    }
}

fn reapable_child(pid: u32) -> Result<Option<Pid>, String> {
    let pid = Pid::from_raw(
        i32::try_from(pid).map_err(|_| "Darwin supervisor PID is out of range".to_string())?,
    );
    loop {
        match waitpid(pid, Some(WaitPidFlag::WNOHANG)) {
            Ok(WaitStatus::StillAlive) => return Ok(Some(pid)),
            Ok(WaitStatus::Exited(..) | WaitStatus::Signaled(..))
            | Err(Errno::ECHILD)
            | Err(Errno::ESRCH) => return Ok(None),
            Err(Errno::EINTR) => continue,
            Ok(status) => return Err(format!("inspect Darwin supervisor child: {status:?}")),
            Err(error) => return Err(format!("inspect Darwin supervisor child: {error}")),
        }
    }
}

fn prove_exact_group(expected: &ProcessIdentity) -> Result<(), String> {
    match platform_process::observe_expected(
        expected.pid,
        &expected.boot_id,
        &expected.start_identity,
    ) {
        ProcessObservation::Exact(birth) if birth.process_group == expected.process_group => Ok(()),
        ProcessObservation::Dead => Err("executor process group leader is dead".to_string()),
        ProcessObservation::Exact(_)
        | ProcessObservation::Mismatch
        | ProcessObservation::Unknown(_) => {
            Err("executor process group ownership is unverified".to_string())
        }
    }
}

fn signal_exact_group(expected: &ProcessIdentity, signal: Signal) -> Result<(), String> {
    prove_exact_group(expected)?;
    #[cfg(test)]
    if FORCE_NEXT_TERMINATION_UNCERTAINTY.swap(false, AtomicOrdering::SeqCst) {
        return Err("injected Darwin termination uncertainty after exact proof".to_string());
    }
    let group = Pid::from_raw(
        i32::try_from(expected.process_group)
            .map_err(|_| "executor process group is out of range".to_string())?,
    );
    killpg(group, signal).map_err(|error| format!("signal Darwin executor process group: {error}"))
}

pub(super) fn group_is_empty(process_group: u32) -> Result<bool, String> {
    let group = Pid::from_raw(
        i32::try_from(process_group)
            .map_err(|_| "executor process group is out of range".to_string())?,
    );
    classify_group_probe(killpg(group, None))
}

fn classify_group_probe(observation: Result<(), Errno>) -> Result<bool, String> {
    match observation {
        Ok(()) => Ok(false),
        Err(Errno::ESRCH) => Ok(true),
        Err(Errno::EPERM) => {
            Err("Darwin executor process-group membership is permission-denied".to_string())
        }
        Err(error) => Err(format!(
            "observe Darwin executor process-group membership: {error}"
        )),
    }
}

fn wait_for_empty_group(
    process_group: u32,
    child: Option<Pid>,
    grace: Duration,
) -> Result<bool, String> {
    let deadline = Instant::now() + grace;
    loop {
        if let Some(child) = child {
            match waitpid(child, Some(WaitPidFlag::WNOHANG)) {
                Ok(_) | Err(Errno::ECHILD) | Err(Errno::ESRCH) => {}
                Err(Errno::EINTR) => continue,
                Err(error) => return Err(format!("reap Darwin executor leader: {error}")),
            }
        }
        match group_is_empty(process_group) {
            Ok(true) => return Ok(true),
            Ok(false) => {}
            Err(error) if Instant::now() >= deadline => return Err(error),
            Err(_) => {}
        }
        if Instant::now() >= deadline {
            return Ok(false);
        }
        thread::sleep(OBSERVATION_INTERVAL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    static FIXTURE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn blocked_fixture(script: &str) -> (DarwinOwnedGroup, PathBuf) {
        let root = std::env::current_dir()
            .expect("current Darwin fixture directory")
            .join("target/executor-bridge-tests")
            .join(format!(
                "autospec-darwin-owned-{}-{}",
                std::process::id(),
                FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
        std::fs::create_dir_all(&root).expect("create Darwin group fixture");
        let sinks = OutputSinkPaths {
            stdout: root.join("stdout"),
            stderr: root.join("stderr"),
            stdout_writer_cursor: root.join("stdout.writer"),
            stderr_writer_cursor: root.join("stderr.writer"),
            stdout_reader_cursor: root.join("stdout.reader"),
            stderr_reader_cursor: root.join("stderr.reader"),
            exit_status: root.join("exit"),
            supervisor_identity: root.join("supervisor.json"),
        };
        let invocation = ValidatedInvocation {
            program: Path::new("/bin/sh").to_path_buf(),
            argv_zero: None::<OsString>,
            args: vec!["-c".into(), script.into()],
            current_dir: root.clone(),
            environment_overrides: Vec::new(),
        };
        let group =
            DarwinOwnedGroup::spawn(&invocation, &sinks).expect("spawn Darwin group fixture");
        (group, root)
    }

    fn fixture(script: &str) -> (DarwinOwnedGroup, PathBuf) {
        let (mut group, root) = blocked_fixture(script);
        group.release().expect("release Darwin group fixture");
        (group, root)
    }

    #[test]
    fn darwin_launch_barrier_prevents_user_code_before_release() {
        let marker = "user-code-ran";
        let (mut group, root) = blocked_fixture(&format!("printf ran > {marker}"));
        thread::sleep(Duration::from_millis(50));
        assert!(
            !root.join(marker).exists(),
            "blocked harness executed user code"
        );
        group.release().expect("release persisted Darwin launch");
        let deadline = Instant::now() + Duration::from_secs(2);
        while group.poll().expect("poll released Darwin group").is_none() {
            assert!(
                Instant::now() < deadline,
                "released Darwin group did not exit"
            );
            thread::sleep(OBSERVATION_INTERVAL);
        }
        assert!(
            root.join(marker).is_file(),
            "released harness did not execute"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn darwin_unreleased_cancellation_reaps_blocked_child_without_signaling() {
        let marker = "cancelled-user-code-ran";
        let (group, root) = blocked_fixture(&format!("printf ran > {marker}"));
        let pid = Pid::from_raw(group.identity().pid as i32);
        let process_group = group.identity().process_group;

        group
            .cancel_unreleased()
            .expect("cancel blocked Darwin launch");

        assert!(!root.join(marker).exists(), "cancelled user code executed");
        assert_eq!(
            waitpid(pid, Some(WaitPidFlag::WNOHANG)),
            Err(Errno::ECHILD),
            "blocked child must already be reaped"
        );
        assert!(group_is_empty(process_group).expect("prove cancelled group ESRCH"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn darwin_unreleased_cancellation_race_never_signals_reused_pgid() {
        use std::os::unix::process::CommandExt;
        use std::process::Command;

        let marker = "raced-cancel-user-code-ran";
        let (mut group, root) = blocked_fixture(&format!("printf ran > {marker}"));
        let owned_pid = Pid::from_raw(group.identity().pid as i32);
        let owned_group = group.identity().process_group;
        let mut unrelated = Command::new("/bin/sh")
            .args(["-c", "trap '' TERM; while :; do sleep 1; done"])
            .process_group(0)
            .spawn()
            .expect("spawn reused-PGID cancellation target");
        group.leader.process_group = unrelated.id();

        let error = group
            .cancel_unreleased()
            .expect_err("reused PGID must remain uncertain after blocked-child reap");

        assert!(
            error.contains("retained process-group membership"),
            "{error}"
        );
        assert!(!root.join(marker).exists(), "cancelled user code executed");
        assert_eq!(
            waitpid(owned_pid, Some(WaitPidFlag::WNOHANG)),
            Err(Errno::ECHILD),
            "blocked child must already be reaped"
        );
        assert!(group_is_empty(owned_group).expect("prove actual blocked group ESRCH"));
        assert!(
            unrelated
                .try_wait()
                .expect("observe reused-PGID cancellation target")
                .is_none(),
            "blocked cancellation signaled unrelated process group"
        );
        unrelated
            .kill()
            .expect("stop reused-PGID cancellation target");
        unrelated
            .wait()
            .expect("reap reused-PGID cancellation target");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn darwin_output_ring_is_bounded_and_restart_preserves_overflow_evidence() {
        let (group, root) = fixture("yes x | head -c 1100000");
        let identity = group.identity().clone();
        let sinks = group.sinks.clone();
        drop(group);
        let mut adopted = DarwinOwnedGroup::adopt(&identity, &sinks).expect("adopt output group");
        let deadline = Instant::now() + Duration::from_secs(5);
        while adopted.poll().expect("poll adopted output group").is_none() {
            assert!(Instant::now() < deadline, "output group did not exit");
            thread::sleep(OBSERVATION_INTERVAL);
        }
        let cursor = super::super::read_output_cursor(
            &OpenOptions::new()
                .read(true)
                .open(&sinks.stdout_writer_cursor)
                .expect("cursor"),
        )
        .expect("read cursor");
        assert!(cursor.total > super::super::MAX_DIRECT_OUTPUT_BYTES);
        assert!(cursor.dropped > 0);
        assert_eq!(
            std::fs::metadata(&sinks.stdout)
                .expect("ring metadata")
                .len(),
            super::super::MAX_DIRECT_OUTPUT_BYTES
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn darwin_adoption_cleanup_requires_exact_boot_start_and_process_group() {
        let (group, root) = fixture("trap '' TERM; while :; do sleep 1; done");
        DarwinOwnedGroup::adopt(group.identity(), &group.sinks).expect("adopt exact group");
        for field in ["boot", "start", "group"] {
            let mut mismatched = group.identity().clone();
            match field {
                "boot" => mismatched.boot_id.push_str("-wrong"),
                "start" => mismatched.start_identity.push_str("-wrong"),
                "group" => mismatched.process_group = mismatched.process_group.saturating_add(1),
                _ => unreachable!(),
            }
            assert!(
                DarwinOwnedGroup::adopt(&mismatched, &group.sinks).is_err(),
                "{field} mismatch was accepted"
            );
        }
        group.terminate().expect("clean exact group");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn darwin_sidecar_launch_identity_mismatch_refuses_signal() {
        let (group, root) = fixture("trap '' TERM; while :; do sleep 1; done");
        let mut mismatched = group.identity().clone();
        mismatched.start_identity.push_str("-wrong");
        let forged = DarwinOwnedGroup {
            leader: mismatched,
            supervisor_pid: None,
            barrier: None,
            exec_status: None,
            sinks: group.sinks.clone(),
        };
        assert_eq!(
            forged.terminate().unwrap_err(),
            "executor process group ownership is unverified"
        );
        assert!(matches!(
            platform_process::observe_expected(
                group.identity().pid,
                &group.identity().boot_id,
                &group.identity().start_identity
            ),
            ProcessObservation::Exact(_)
        ));
        group.terminate().expect("clean untouched exact group");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn darwin_forged_reused_pgid_never_signals_unrelated_group() {
        use std::os::unix::process::CommandExt;
        use std::process::Command;

        let (owned, owned_root) = fixture("trap '' TERM; while :; do sleep 1; done");
        let mut unrelated = Command::new("/bin/sh")
            .args(["-c", "trap '' TERM; while :; do sleep 1; done"])
            .process_group(0)
            .spawn()
            .expect("spawn unrelated reused-PGID target");
        let mut forged = owned.identity().clone();
        forged.process_group = unrelated.id();
        let forged = DarwinOwnedGroup {
            leader: forged,
            supervisor_pid: None,
            barrier: None,
            exec_status: None,
            sinks: owned.sinks.clone(),
        };

        assert_eq!(
            forged.terminate().unwrap_err(),
            "executor process group ownership is unverified"
        );
        assert!(
            unrelated
                .try_wait()
                .expect("observe unrelated reused-PGID target")
                .is_none(),
            "forged ownership signaled unrelated process group"
        );
        unrelated.kill().expect("stop unrelated reused-PGID target");
        unrelated.wait().expect("reap unrelated reused-PGID target");
        owned.terminate().expect("clean owned group");
        let _ = std::fs::remove_dir_all(owned_root);
    }

    #[test]
    fn darwin_cleanup_never_signals_an_unrelated_exact_group() {
        use std::os::unix::process::CommandExt;
        use std::process::Command;

        let (owned, owned_root) = fixture("trap '' TERM; while :; do sleep 1; done");
        let mut unrelated = Command::new("/bin/sh")
            .args(["-c", "trap '' TERM; while :; do sleep 1; done"])
            .process_group(0)
            .spawn()
            .expect("spawn unrelated process group");
        owned.terminate().expect("clean owned group");
        assert!(unrelated
            .try_wait()
            .expect("observe unrelated group")
            .is_none());
        unrelated.kill().expect("stop unrelated group");
        unrelated.wait().expect("reap unrelated group");
        let _ = std::fs::remove_dir_all(owned_root);
    }

    #[test]
    fn darwin_group_probe_fails_closed_on_permission_and_unknown_errors() {
        assert_eq!(
            classify_group_probe(Err(Errno::EPERM)).unwrap_err(),
            "Darwin executor process-group membership is permission-denied"
        );
        assert!(classify_group_probe(Err(Errno::EIO))
            .unwrap_err()
            .contains("observe Darwin executor process-group membership"));
        assert!(classify_group_probe(Err(Errno::ESRCH)).expect("ESRCH is empty"));
    }

    #[test]
    fn darwin_restart_direct_completes_only_after_leader_and_group_exit() {
        let (mut group, root) = fixture("sleep 0.05 & exit 0");
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            match group.poll().expect("poll Darwin group") {
                Some(0) => break,
                Some(code) => panic!("unexpected Darwin exit code {code}"),
                None if Instant::now() < deadline => thread::sleep(OBSERVATION_INTERVAL),
                None => panic!("Darwin group did not reach empty completion"),
            }
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn darwin_restart_adopts_durable_exit_after_original_parent_crash() {
        const CHILD_ENV: &str = "AUTOSPEC_TEST_DARWIN_CRASH_PARENT_ROOT";
        let child_root = std::env::var_os(CHILD_ENV).map(PathBuf::from);
        if let Some(root) = child_root {
            let sinks = OutputSinkPaths {
                stdout: root.join("stdout"),
                stderr: root.join("stderr"),
                stdout_writer_cursor: root.join("stdout.writer"),
                stderr_writer_cursor: root.join("stderr.writer"),
                stdout_reader_cursor: root.join("stdout.reader"),
                stderr_reader_cursor: root.join("stderr.reader"),
                exit_status: root.join("exit"),
                supervisor_identity: root.join("supervisor.json"),
            };
            let invocation = ValidatedInvocation {
                program: Path::new("/bin/sh").to_path_buf(),
                argv_zero: None::<OsString>,
                args: vec!["-c".into(), "yes x | head -c 1100000; exit 7".into()],
                current_dir: root.clone(),
                environment_overrides: Vec::new(),
            };
            let mut group = DarwinOwnedGroup::spawn(&invocation, &sinks)
                .expect("spawn crash-parent Darwin group");
            let identity = super::super::process_identity_value(group.identity()).to_string();
            super::super::write_private_create_once(
                &root.join("identity.json"),
                identity.as_bytes(),
                "crash-parent Darwin identity",
            )
            .expect("persist crash-parent identity");
            group.release().expect("release crash-parent Darwin group");
            std::process::exit(0);
        }

        let root = std::env::current_dir()
            .expect("current Darwin fixture directory")
            .join("target/executor-bridge-tests")
            .join(format!(
                "autospec-darwin-crash-parent-{}-{}",
                std::process::id(),
                FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
        std::fs::create_dir_all(&root).expect("create crash-parent root");
        let test_name = "commands::autonomous::executor_bridge::darwin_supervisor::tests::darwin_restart_adopts_durable_exit_after_original_parent_crash";
        let status = std::process::Command::new(std::env::current_exe().expect("test executable"))
            .args(["--exact", test_name, "--test-threads=1"])
            .env(CHILD_ENV, &root)
            .status()
            .expect("run crash-parent subprocess");
        assert!(status.success(), "crash-parent subprocess failed: {status}");
        let identity = super::super::parse_process_identity(
            serde_json::from_str(
                &std::fs::read_to_string(root.join("identity.json"))
                    .expect("read crash-parent identity"),
            )
            .expect("parse crash-parent identity JSON"),
            "crash-parent Darwin identity",
        )
        .expect("validate crash-parent identity");
        let sinks = OutputSinkPaths {
            stdout: root.join("stdout"),
            stderr: root.join("stderr"),
            stdout_writer_cursor: root.join("stdout.writer"),
            stderr_writer_cursor: root.join("stderr.writer"),
            stdout_reader_cursor: root.join("stdout.reader"),
            stderr_reader_cursor: root.join("stderr.reader"),
            exit_status: root.join("exit"),
            supervisor_identity: root.join("supervisor.json"),
        };
        let mut adopted = DarwinOwnedGroup::adopt(&identity, &sinks).expect("adopt exact group");
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            match adopted.poll().expect("poll adopted Darwin group") {
                Some(7) => break,
                Some(code) => panic!("unexpected adopted Darwin exit {code}"),
                None if Instant::now() < deadline => thread::sleep(OBSERVATION_INTERVAL),
                None => panic!("adopted Darwin group never recovered its durable exit"),
            }
        }
        let cursor = super::super::read_output_cursor(
            &OpenOptions::new()
                .read(true)
                .open(&sinks.stdout_writer_cursor)
                .expect("crash-parent output cursor"),
        )
        .expect("read crash-parent output cursor");
        assert!(cursor.total > super::super::MAX_DIRECT_OUTPUT_BYTES);
        assert!(cursor.dropped > 0);
        assert_eq!(
            std::fs::metadata(&sinks.stdout)
                .expect("crash-parent bounded ring")
                .len(),
            super::super::MAX_DIRECT_OUTPUT_BYTES
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
