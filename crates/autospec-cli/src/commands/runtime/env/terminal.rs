use std::process::Command;

use crate::commands::CommandFailure;

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
            // SAFETY: these terminal/process queries use the fixed stdin descriptor.
            if unsafe { nix::libc::isatty(nix::libc::STDIN_FILENO) } == 0 {
                return Ok(Self::inactive());
            }
            // SAFETY: tcgetpgrp and getpgrp have no memory-safety preconditions.
            let foreground = unsafe { nix::libc::tcgetpgrp(nix::libc::STDIN_FILENO) };
            if foreground < 0 {
                return Err(diagnostic(format!(
                    "could not inspect runtime terminal foreground: {}",
                    std::io::Error::last_os_error()
                )));
            }
            // A background caller must not steal terminal ownership.
            let current = unsafe { nix::libc::getpgrp() };
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

pub(super) fn configure_process_group(
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
    // SAFETY: this runs after fork and uses only async-signal-safe process calls.
    unsafe {
        if nix::libc::setpgid(0, 0) < 0 {
            return Err(std::io::Error::last_os_error());
        }
        set_terminal_foreground(nix::libc::getpgrp())
    }
}

#[cfg(unix)]
fn set_terminal_foreground(process_group: nix::libc::pid_t) -> std::io::Result<()> {
    // Blocking SIGTTOU lets a background group perform tcsetpgrp. The original
    // mask is restored before exec or before the parent resumes normal work.
    let mut blocked = unsafe { std::mem::zeroed::<nix::libc::sigset_t>() };
    let mut original = unsafe { std::mem::zeroed::<nix::libc::sigset_t>() };
    // SAFETY: both signal sets are initialized storage owned by this function.
    unsafe {
        nix::libc::sigemptyset(&mut blocked);
        nix::libc::sigaddset(&mut blocked, nix::libc::SIGTTOU);
        if nix::libc::sigprocmask(nix::libc::SIG_BLOCK, &blocked, &mut original) < 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    // SAFETY: stdin is the verified controlling terminal and the process group
    // belongs to the same session.
    let transfer = unsafe { nix::libc::tcsetpgrp(nix::libc::STDIN_FILENO, process_group) };
    let transfer_error = (transfer < 0).then(std::io::Error::last_os_error);
    // SAFETY: original was filled by the successful SIG_BLOCK call above.
    let restore =
        unsafe { nix::libc::sigprocmask(nix::libc::SIG_SETMASK, &original, std::ptr::null_mut()) };
    if let Some(error) = transfer_error {
        return Err(error);
    }
    if restore < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

fn diagnostic(message: impl Into<String>) -> CommandFailure {
    CommandFailure::diagnostic(message)
}
