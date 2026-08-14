use nix::errno::Errno;
use nix::sys::signal::{kill, Signal};
use nix::unistd::{getpgid, Pid};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, ExitStatus};
use std::thread;
use std::time::{Duration, Instant};

const TERMINATE_GRACE: Duration = Duration::from_millis(400);
const REAP_POLL: Duration = Duration::from_millis(10);

pub(super) struct UnixOwnedChild {
    child: ChildLifecycle,
    pid: u32,
    pgid: Pid,
    group_cleaned: bool,
}

enum ChildLifecycle {
    Live { child: Child, may_signal: bool },
    Reaped(ExitStatus),
}

struct GroupSignalError {
    message: String,
    ignorable_if_reaped: bool,
}

trait GroupOperations {
    fn signal(&mut self, pgid: Pid, signal: Signal) -> Result<(), GroupSignalError>;
    fn exists(&mut self, pgid: Pid) -> Result<bool, String>;
}

struct SystemGroupOperations;

impl GroupOperations for SystemGroupOperations {
    fn signal(&mut self, pgid: Pid, signal: Signal) -> Result<(), GroupSignalError> {
        signal_group(pgid, signal).map_err(|error| GroupSignalError {
            message: error.to_string(),
            ignorable_if_reaped: error == Errno::EPERM,
        })
    }

    fn exists(&mut self, pgid: Pid) -> Result<bool, String> {
        process_group_exists_pid(pgid)
    }
}

impl UnixOwnedChild {
    pub(super) fn spawn(command: &mut Command) -> Result<Self, String> {
        command.process_group(0);
        let mut child = command
            .spawn()
            .map_err(|error| format!("spawn owned process group: {error}"))?;
        let pgid = Pid::from_raw(
            i32::try_from(child.id()).map_err(|_| "owned process group ID is out of range")?,
        );
        let observed = match getpgid(Some(pgid)) {
            Ok(observed) => observed,
            Err(error) => {
                let cleanup = child.kill().and_then(|()| child.wait());
                return Err(format!(
                    "observe spawned process group: {error}; cleanup={cleanup:?}"
                ));
            }
        };
        if observed != pgid {
            let cleanup = child.kill().and_then(|()| child.wait());
            return Err(format!(
                "spawned process group leader {pgid} joined unexpected group {observed}; cleanup={cleanup:?}"
            ));
        }
        let pid = child.id();
        Ok(Self {
            child: ChildLifecycle::Live {
                child,
                may_signal: true,
            },
            pid,
            pgid,
            group_cleaned: false,
        })
    }

    pub(super) fn id(&self) -> u32 {
        self.pid
    }

    pub(super) fn try_wait(&mut self) -> Result<Option<ExitStatus>, String> {
        match &mut self.child {
            ChildLifecycle::Reaped(status) => Ok(Some(*status)),
            ChildLifecycle::Live { child, .. } => match child.try_wait() {
                Ok(Some(status)) => {
                    self.child = ChildLifecycle::Reaped(status);
                    Ok(Some(status))
                }
                Ok(None) => Ok(None),
                Err(error) => Err(format!("poll owned process group leader: {error}")),
            },
        }
    }

    pub(super) fn wait(&mut self) -> Result<ExitStatus, String> {
        match &mut self.child {
            ChildLifecycle::Reaped(status) => Ok(*status),
            ChildLifecycle::Live { child, .. } => {
                let result = child
                    .wait()
                    .map_err(|error| format!("wait for owned process group leader: {error}"));
                match result {
                    Ok(status) => {
                        self.child = ChildLifecycle::Reaped(status);
                        Ok(status)
                    }
                    Err(error) => Err(error),
                }
            }
        }
    }

    pub(super) fn terminate(&mut self) -> Result<ExitStatus, String> {
        self.terminate_with_operations(&mut SystemGroupOperations, TERMINATE_GRACE)
    }

    fn terminate_with_operations(
        &mut self,
        operations: &mut impl GroupOperations,
        grace: Duration,
    ) -> Result<ExitStatus, String> {
        if self.group_cleaned {
            return self.wait();
        }
        if let ChildLifecycle::Live { may_signal, .. } = &mut self.child {
            if !*may_signal {
                return self.wait();
            }
            *may_signal = false;
        }
        // The process-group authority remains valid after its leader is reaped. Mark cleanup
        // consumed before signalling so an error cannot cause a later PID/PGID reuse signal.
        self.group_cleaned = true;

        let mut errors = Vec::new();
        let mut child_fallback = false;
        match operations.signal(self.pgid, Signal::SIGTERM) {
            Ok(()) => {
                let deadline = Instant::now() + grace;
                let mut exists = true;
                while Instant::now() < deadline {
                    match operations.exists(self.pgid) {
                        Ok(false) => {
                            exists = false;
                            break;
                        }
                        Ok(true) => thread::sleep(REAP_POLL),
                        Err(error) => {
                            errors.push(format!(
                                "inspect owned process group {}: {error}",
                                self.pgid
                            ));
                            child_fallback = true;
                            exists = false;
                            break;
                        }
                    }
                }
                if exists && !child_fallback {
                    match operations.exists(self.pgid) {
                        Ok(false) => {}
                        Ok(true) => match operations.signal(self.pgid, Signal::SIGKILL) {
                            Ok(()) => {}
                            Err(error) => {
                                if error.ignorable_if_reaped
                                    && self.try_wait().ok().flatten().is_some()
                                {
                                    // A zombie-only group can report EPERM on macOS. The retained
                                    // child has already proven and cached its terminal state.
                                } else {
                                    errors.push(format!(
                                        "signal owned process group {}: {}",
                                        self.pgid, error.message
                                    ));
                                    child_fallback = true;
                                }
                            }
                        },
                        Err(error) => {
                            errors.push(format!(
                                "inspect owned process group {}: {error}",
                                self.pgid
                            ));
                            child_fallback = true;
                        }
                    }
                }
            }
            Err(error) => {
                errors.push(format!(
                    "signal owned process group {}: {}",
                    self.pgid, error.message
                ));
                child_fallback = true;
            }
        }

        if child_fallback {
            if let ChildLifecycle::Live { child, .. } = &mut self.child {
                if let Err(error) = child.kill() {
                    errors.push(format!("kill owned process group leader: {error}"));
                }
            }
        }

        let reaped = self.wait();
        match (errors.is_empty(), reaped) {
            (true, result) => result,
            (false, Ok(_)) => Err(errors.join("; ")),
            (false, Err(error)) => {
                errors.push(error);
                Err(errors.join("; "))
            }
        }
    }
}

fn signal_group(pgid: Pid, signal: Signal) -> Result<(), Errno> {
    match kill(Pid::from_raw(-pgid.as_raw()), signal) {
        Ok(()) | Err(Errno::ESRCH) => Ok(()),
        Err(error) => Err(error),
    }
}

fn process_group_exists_pid(pgid: Pid) -> Result<bool, String> {
    match kill(Pid::from_raw(-pgid.as_raw()), None) {
        Ok(()) | Err(Errno::EPERM) => Ok(true),
        Err(Errno::ESRCH) => Ok(false),
        Err(error) => Err(format!("inspect owned process group {pgid}: {error}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct InjectedGroupOperations {
        fail_term: bool,
        fail_probe: bool,
        fail_kill: bool,
        group_exists: bool,
        signals: Vec<Signal>,
        probes: usize,
    }

    impl GroupOperations for InjectedGroupOperations {
        fn signal(&mut self, _pgid: Pid, signal: Signal) -> Result<(), GroupSignalError> {
            self.signals.push(signal);
            if (signal == Signal::SIGTERM && self.fail_term)
                || (signal == Signal::SIGKILL && self.fail_kill)
            {
                Err(GroupSignalError {
                    message: format!("injected {signal:?} failure"),
                    ignorable_if_reaped: false,
                })
            } else {
                Ok(())
            }
        }

        fn exists(&mut self, _pgid: Pid) -> Result<bool, String> {
            self.probes += 1;
            if self.fail_probe {
                Err("injected probe failure".to_string())
            } else {
                Ok(self.group_exists)
            }
        }
    }

    fn sleeping_child() -> UnixOwnedChild {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "exec sleep 30"]);
        UnixOwnedChild::spawn(&mut command).expect("spawn owned test group")
    }

    fn assert_reaped_without_resignal(
        child: &mut UnixOwnedChild,
        expected_error: &str,
        mut operations: InjectedGroupOperations,
    ) {
        let error = child
            .terminate_with_operations(&mut operations, Duration::ZERO)
            .expect_err("injected cleanup failure must be reported");
        assert!(error.contains(expected_error), "unexpected error: {error}");
        assert!(
            child
                .try_wait()
                .expect("read cached terminal state")
                .is_some(),
            "leader was not reaped after the injected failure"
        );

        let mut forbidden = InjectedGroupOperations {
            fail_term: true,
            fail_probe: true,
            fail_kill: true,
            ..InjectedGroupOperations::default()
        };
        child
            .terminate_with_operations(&mut forbidden, Duration::ZERO)
            .expect("idempotent cleanup returns cached terminal status");
        assert!(forbidden.signals.is_empty());
        assert_eq!(
            forbidden.probes, 0,
            "reaped ownership must not probe a PGID"
        );
    }

    #[test]
    fn term_failure_falls_back_to_child_wait_and_never_resignals() {
        let mut child = sleeping_child();
        assert_reaped_without_resignal(
            &mut child,
            "injected SIGTERM failure",
            InjectedGroupOperations {
                fail_term: true,
                ..InjectedGroupOperations::default()
            },
        );
    }

    #[test]
    fn probe_failure_falls_back_to_child_wait_and_never_resignals() {
        let mut child = sleeping_child();
        assert_reaped_without_resignal(
            &mut child,
            "injected probe failure",
            InjectedGroupOperations {
                fail_probe: true,
                ..InjectedGroupOperations::default()
            },
        );
    }

    #[test]
    fn kill_failure_falls_back_to_child_wait_and_never_resignals() {
        let mut child = sleeping_child();
        assert_reaped_without_resignal(
            &mut child,
            "injected SIGKILL failure",
            InjectedGroupOperations {
                fail_kill: true,
                group_exists: true,
                ..InjectedGroupOperations::default()
            },
        );
    }
}
