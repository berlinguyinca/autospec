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
    child: Child,
    pgid: Pid,
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
        Ok(Self { child, pgid })
    }

    pub(super) fn id(&self) -> u32 {
        self.child.id()
    }

    pub(super) fn try_wait(&mut self) -> Result<Option<ExitStatus>, String> {
        self.child
            .try_wait()
            .map_err(|error| format!("poll owned process group leader: {error}"))
    }

    pub(super) fn wait(&mut self) -> Result<ExitStatus, String> {
        self.child
            .wait()
            .map_err(|error| format!("wait for owned process group leader: {error}"))
    }

    pub(super) fn terminate(&mut self) -> Result<ExitStatus, String> {
        signal_group(self.pgid, Signal::SIGTERM)
            .map_err(|error| format!("signal owned process group {}: {error}", self.pgid))?;
        let deadline = Instant::now() + TERMINATE_GRACE;
        while Instant::now() < deadline && process_group_exists_pid(self.pgid)? {
            thread::sleep(REAP_POLL);
        }
        if process_group_exists_pid(self.pgid)? {
            match signal_group(self.pgid, Signal::SIGKILL) {
                Ok(()) => {}
                Err(Errno::EPERM) if self.child.try_wait().is_ok_and(|status| status.is_some()) => {
                }
                Err(error) => {
                    return Err(format!("signal owned process group {}: {error}", self.pgid))
                }
            }
        }
        self.wait()
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
